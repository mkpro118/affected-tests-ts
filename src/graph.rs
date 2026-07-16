//! Deterministic dependency graph contracts and traversal views.

use std::collections::BTreeMap;

use rayon::prelude::{IntoParallelIterator, ParallelIterator};

use crate::failure;
use crate::modules;
use crate::parser;
use crate::roots;
use crate::settings;

/// Reverse dependency access consumed by affected selection and tracing.
pub trait GraphView {
    /// Returns direct reverse dependents for a graph node in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];

    /// Returns direct dependencies for a graph node in stable order.
    #[must_use]
    fn dependencies(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];

    /// Returns files that import an external package in stable order.
    #[must_use]
    fn external_importers(&self, package: &str) -> &[roots::RootRelativePath];
}

/// Import source used by graph construction.
pub trait ImportResolver {
    /// Returns resolved import edges for a file in stable order.
    ///
    /// # Errors
    ///
    /// Returns an error when imports cannot be parsed or resolved.
    fn imports_for(&self, path: &roots::RootRelativePath) -> failure::Result<ResolvedImports>;
}

/// Resolved local and external import edges for one file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedImports {
    /// Local root-relative files imported by a source or test file.
    pub dependencies: Box<[roots::RootRelativePath]>,
    /// External package names imported by a source or test file.
    pub external_packages: Box<[Box<str>]>,
}

/// Request object for parser and module resolver backed import extraction.
pub struct LocalImportsRequest<C, R, M, P> {
    /// Configuration view used for dynamic import policy and module aliases.
    pub config: C,
    /// Source reader used by the parser.
    pub reader: R,
    /// Module resolver used for every parsed specifier.
    pub resolver: M,
    /// Filesystem probe used by module resolution.
    pub probe: P,
}

/// Import source that parses files and resolves local module edges.
#[derive(Clone, Debug)]
pub struct LocalImports<C, R, M, P> {
    config: C,
    reader: R,
    resolver: M,
    probe: P,
    failure_mode: ImportFailureMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportFailureMode {
    Strict,
    GraphOutput,
}

impl<C, R, M, P> LocalImports<C, R, M, P> {
    /// Creates a parser and resolver backed import source.
    #[must_use]
    pub fn new(request: LocalImportsRequest<C, R, M, P>) -> Self {
        Self {
            config: request.config,
            reader: request.reader,
            resolver: request.resolver,
            probe: request.probe,
            failure_mode: ImportFailureMode::Strict,
        }
    }

    /// Creates an import source for inspectable graph output.
    #[must_use]
    pub fn for_graph_output(request: LocalImportsRequest<C, R, M, P>) -> Self {
        Self {
            config: request.config,
            reader: request.reader,
            resolver: request.resolver,
            probe: request.probe,
            failure_mode: ImportFailureMode::GraphOutput,
        }
    }
}

impl<C, R, M, P> ImportResolver for LocalImports<C, R, M, P>
where
    C: Clone + settings::View,
    R: Clone + parser::SourceReader,
    M: Clone + modules::ImportResolver,
    P: Clone + modules::FileExistence,
{
    fn imports_for(&self, path: &roots::RootRelativePath) -> failure::Result<ResolvedImports> {
        let imports = parser::imports(parser::Request {
            reader: self.reader.clone(),
            path: path.clone(),
        })?;

        if should_fail_for_dynamic_imports(&DynamicImportFailureRequest {
            config: &self.config,
            mode: self.failure_mode,
            unsupported_count: imports.unsupported_dynamic_imports.len(),
            importer: path,
        })? {
            return Err(failure::AppError::UnknownDynamicImport {
                importer: path.clone(),
            });
        }

        resolve_imports(ResolveImportsRequest {
            config: self.config.clone(),
            resolver: self.resolver.clone(),
            probe: self.probe.clone(),
            importer: path.clone(),
            static_specifiers: imports.static_specifiers,
            dynamic_specifiers: imports.dynamic_specifiers,
            failure_mode: self.failure_mode,
        })
    }
}

/// Request object for graph construction.
pub struct GraphBuildRequest<I> {
    /// Import source used for edge construction.
    pub imports: I,
    /// Source and test files included in the graph.
    pub files: Box<[roots::RootRelativePath]>,
}

/// Deterministic directed dependency graph.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyGraph {
    dependencies: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    reverse_dependencies: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    external_importers: BTreeMap<Box<str>, Box<[roots::RootRelativePath]>>,
    empty: Box<[roots::RootRelativePath]>,
}

impl GraphView for DependencyGraph {
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        self.reverse_dependencies
            .get(path)
            .map_or_else(|| self.empty.as_ref(), Box::as_ref)
    }

    fn dependencies(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        self.dependencies
            .get(path)
            .map_or_else(|| self.empty.as_ref(), Box::as_ref)
    }

    fn external_importers(&self, package: &str) -> &[roots::RootRelativePath] {
        self.external_importers
            .get(package)
            .map_or_else(|| self.empty.as_ref(), Box::as_ref)
    }
}

/// Builds a deterministic dependency graph from resolved imports.
///
/// # Errors
///
/// Returns an error when dependency extraction fails for any graph file.
pub fn build<I>(request: GraphBuildRequest<I>) -> failure::Result<DependencyGraph>
where
    I: ImportResolver + Sync,
{
    let GraphBuildRequest { imports, files } = request;
    let edge_sets = files
        .into_vec()
        .into_par_iter()
        .map(|path| node_edges(&imports, path))
        .collect::<failure::Result<Vec<NodeEdges>>>()?;

    Ok(graph_from_edges(edge_sets))
}

struct ResolveImportsRequest<C, M, P> {
    config: C,
    resolver: M,
    probe: P,
    importer: roots::RootRelativePath,
    static_specifiers: Box<[roots::ImportSpecifier]>,
    dynamic_specifiers: Box<[roots::ImportSpecifier]>,
    failure_mode: ImportFailureMode,
}

struct NodeEdges {
    path: roots::RootRelativePath,
    dependencies: Box<[roots::RootRelativePath]>,
    external_packages: Box<[Box<str>]>,
}

struct DynamicImportFailureRequest<'a, C> {
    config: &'a C,
    mode: ImportFailureMode,
    unsupported_count: usize,
    importer: &'a roots::RootRelativePath,
}

fn should_fail_for_dynamic_imports<C>(
    request: &DynamicImportFailureRequest<'_, C>,
) -> failure::Result<bool>
where
    C: settings::View,
{
    if request.mode != ImportFailureMode::Strict
        || request.unsupported_count == 0
        || request.config.dynamic_imports() != settings::UnknownDynamicImportBehavior::FailClosed
    {
        return Ok(false);
    }

    // Files listed in `dynamicImportIgnore` opt out of the fail-closed policy so a
    // single genuinely-unresolvable dynamic import (e.g. `import(absolutePath)`)
    // does not force the entire suite on every reachable PR.
    Ok(!settings::matches_dynamic_import_ignore(
        request.config,
        request.importer,
    )?)
}

fn resolve_imports<C, M, P>(
    request: ResolveImportsRequest<C, M, P>,
) -> failure::Result<ResolvedImports>
where
    C: Clone + settings::View,
    M: Clone + modules::ImportResolver,
    P: Clone + modules::FileExistence,
{
    let ResolveImportsRequest {
        config,
        resolver,
        probe,
        importer,
        static_specifiers,
        dynamic_specifiers,
        failure_mode,
    } = request;
    let mut dependencies = Vec::<roots::RootRelativePath>::new();
    let mut external_packages = Vec::<Box<str>>::new();
    for specifier in static_specifiers
        .into_vec()
        .into_iter()
        .chain(dynamic_specifiers.into_vec())
    {
        let outcome = resolver.resolve(modules::ResolveRequest {
            config: config.clone(),
            probe: probe.clone(),
            importer: importer.clone(),
            specifier,
        })?;

        match outcome {
            modules::Outcome::Resolved(path) => dependencies.push(path),
            modules::Outcome::External(specifier) => {
                if let Some(package) = external_package_name(specifier.as_str()) {
                    external_packages.push(package);
                }
            }
            modules::Outcome::Unresolved(specifier) => {
                if failure_mode == ImportFailureMode::Strict {
                    return Err(failure::AppError::UnresolvedLocalImport {
                        importer,
                        specifier,
                    });
                }
            }
        }
    }

    Ok(ResolvedImports {
        dependencies: sorted_unique(dependencies),
        external_packages: sorted_unique(external_packages),
    })
}

fn external_package_name(specifier: &str) -> Option<Box<str>> {
    if specifier.starts_with('.') || specifier.starts_with('/') {
        return None;
    }

    let mut segments = specifier.split('/');
    let first = segments.next()?;
    if first.is_empty() || first == "src" || first == "~" {
        return None;
    }
    if first.starts_with('@') {
        return segments
            .next()
            .filter(|second| !second.is_empty())
            .map(|second| format!("{first}/{second}").into_boxed_str());
    }

    Some(Box::<str>::from(first))
}

fn node_edges<I>(imports: &I, path: roots::RootRelativePath) -> failure::Result<NodeEdges>
where
    I: ImportResolver,
{
    let imports = imports.imports_for(&path)?;
    Ok(NodeEdges {
        dependencies: imports.dependencies,
        external_packages: imports.external_packages,
        path,
    })
}

fn graph_from_edges(edge_sets: Vec<NodeEdges>) -> DependencyGraph {
    let mut dependencies =
        BTreeMap::<roots::RootRelativePath, Box<[roots::RootRelativePath]>>::new();
    let mut reverse_edges =
        BTreeMap::<roots::RootRelativePath, Vec<roots::RootRelativePath>>::new();
    let mut external_importers = BTreeMap::<Box<str>, Vec<roots::RootRelativePath>>::new();

    for edge_set in edge_sets {
        reverse_edges.entry(edge_set.path.clone()).or_default();
        for package in &edge_set.external_packages {
            external_importers
                .entry(package.clone())
                .or_default()
                .push(edge_set.path.clone());
        }
        for dependency in &edge_set.dependencies {
            reverse_edges
                .entry(dependency.clone())
                .or_default()
                .push(edge_set.path.clone());
        }
        dependencies.insert(
            edge_set.path,
            sorted_unique(edge_set.dependencies.into_vec()),
        );
    }

    let reverse_dependencies = reverse_edges
        .into_iter()
        .map(|(path, dependents)| (path, sorted_unique(dependents)))
        .collect();
    let external_importers = external_importers
        .into_iter()
        .map(|(package, importers)| (package, sorted_unique(importers)))
        .collect();

    DependencyGraph {
        dependencies,
        reverse_dependencies,
        external_importers,
        empty: Box::from([]),
    }
}

fn sorted_unique<T>(mut values: Vec<T>) -> Box<[T]>
where
    T: Ord,
{
    values.sort();
    values.dedup();
    values.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::failure;
    use crate::modules;
    use crate::roots;
    use crate::settings;
    use crate::vfs;

    #[derive(Clone, Debug)]
    struct FixtureImports {
        edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        external_edges: BTreeMap<roots::RootRelativePath, Box<[Box<str>]>>,
        empty: Box<[roots::RootRelativePath]>,
    }

    impl super::ImportResolver for FixtureImports {
        fn imports_for(
            &self,
            path: &roots::RootRelativePath,
        ) -> failure::Result<super::ResolvedImports> {
            Ok(super::ResolvedImports {
                dependencies: self
                    .edges
                    .get(path)
                    .map_or_else(|| self.empty.clone(), Clone::clone),
                external_packages: self
                    .external_edges
                    .get(path)
                    .map_or_else(|| Box::<[Box<str>]>::from([]), Clone::clone),
            })
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureConfig {
        dynamic_imports: settings::UnknownDynamicImportBehavior,
        dynamic_import_ignore: Box<[settings::Pattern]>,
    }

    impl FixtureConfig {
        fn fail_closed() -> Self {
            Self {
                dynamic_imports: settings::UnknownDynamicImportBehavior::FailClosed,
                dynamic_import_ignore: Box::from([]),
            }
        }

        fn fail_closed_ignoring(globs: &[&str]) -> Self {
            Self {
                dynamic_imports: settings::UnknownDynamicImportBehavior::FailClosed,
                dynamic_import_ignore: globs
                    .iter()
                    .map(|glob| settings::Pattern::try_from(*glob).unwrap())
                    .collect(),
            }
        }
    }

    impl settings::View for FixtureConfig {
        fn source_includes(&self) -> &[settings::Pattern] {
            &[]
        }

        fn excludes(&self) -> &[settings::Pattern] {
            &[]
        }

        fn test_patterns(&self) -> &[settings::TestFilePattern] {
            &[]
        }

        fn global_invalidators(&self) -> &[settings::Pattern] {
            &[]
        }

        fn dynamic_imports(&self) -> settings::UnknownDynamicImportBehavior {
            self.dynamic_imports
        }

        fn dynamic_import_ignore(&self) -> &[settings::Pattern] {
            self.dynamic_import_ignore.as_ref()
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureResolver;

    impl modules::ImportResolver for FixtureResolver {
        fn resolve<C, P>(
            &self,
            request: modules::ResolveRequest<C, P>,
        ) -> failure::Result<modules::Outcome>
        where
            C: settings::View,
            P: modules::FileExistence,
        {
            modules::import(request)
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn builds_forward_edges_and_reverse_edges_for_importers_and_imports() {
        let file_a = path("src/file-a.ts");
        let file_b = path("src/file-b.ts");
        let imports = FixtureImports {
            edges: BTreeMap::from([(file_a.clone(), Box::from([file_b.clone()]))]),
            external_edges: BTreeMap::from([(
                file_a.clone(),
                Box::from([Box::<str>::from("react")]),
            )]),
            empty: Box::from([]),
        };
        let request = super::GraphBuildRequest {
            imports,
            files: Box::from([file_a.clone(), file_b.clone()]),
        };

        // A single A -> B import is the smallest fixture that proves both graph
        // directions are populated for selectors and explain output.
        let graph = super::build(request).unwrap();

        assert_eq!(
            super::GraphView::dependencies(&graph, &file_a),
            std::slice::from_ref(&file_b),
        );
        assert_eq!(
            super::GraphView::reverse_dependents(&graph, &file_b),
            std::slice::from_ref(&file_a)
        );
        assert_eq!(
            super::GraphView::external_importers(&graph, "react"),
            &[file_a]
        );
    }

    #[test]
    fn parser_and_resolver_backed_imports_resolve_local_edges() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([
            (
                path("src/pages/home.ts"),
                Box::<str>::from("import { button } from '../components/button';"),
            ),
            (
                path("src/components/button.ts"),
                Box::<str>::from("export const button = true;"),
            ),
        ]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed(),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // The fixture exercises the production parser and resolver path without
        // touching host files, proving graph build can consume real source text.
        let import_edges =
            super::ImportResolver::imports_for(&imports, &path("src/pages/home.ts")).unwrap();

        assert_eq!(
            import_edges.dependencies,
            Box::<[roots::RootRelativePath]>::from([path("src/components/button.ts")]),
        );
        assert_eq!(import_edges.external_packages, Box::<[Box<str>]>::from([]));
    }

    #[test]
    fn parser_and_resolver_backed_imports_record_external_package_names() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/pages/home.ts"),
            Box::<str>::from(
                "import React from 'react';\nimport { z } from 'zod/v4';\nimport { Button } from '@radix-ui/react-button';",
            ),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed(),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // External imports are normalized to package roots so lockfile changes
        // can be scoped back to local importer files.
        let import_edges =
            super::ImportResolver::imports_for(&imports, &path("src/pages/home.ts")).unwrap();

        assert_eq!(
            import_edges.external_packages,
            Box::<[Box<str>]>::from([
                Box::<str>::from("@radix-ui/react-button"),
                Box::<str>::from("react"),
                Box::<str>::from("zod"),
            ]),
        );
    }

    #[test]
    fn unsupported_dynamic_imports_fail_closed_when_configured() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/router.ts"),
            Box::<str>::from("const page = import(routeName);"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed(),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // Non-literal dynamic imports cannot safely become partial graph edges,
        // so the graph boundary must force callers into full-suite behavior.
        let error =
            super::ImportResolver::imports_for(&imports, &path("src/router.ts")).unwrap_err();

        assert!(matches!(
            error,
            failure::AppError::UnknownDynamicImport { importer } if importer == path("src/router.ts")
        ));
    }

    #[test]
    fn unsupported_dynamic_import_in_ignored_file_does_not_force_full_run() {
        // A file that dynamically imports a runtime-computed path (the real-world
        // `await import(absolutePath)` case) is normally fail-closed, but listing
        // it in dynamicImportIgnore must exempt it so it no longer forces a full
        // suite. The static import edge is still resolved.
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([
            (
                path("src/graphql-contract/collector.ts"),
                Box::<str>::from(
                    "import './types';\nconst absolutePath = join(dir, rel);\nconst m = import(absolutePath);",
                ),
            ),
            (
                path("src/graphql-contract/types.ts"),
                Box::<str>::from("export const kinds = [];"),
            ),
        ]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed_ignoring(&["src/graphql-contract/collector.ts"]),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        let import_edges = super::ImportResolver::imports_for(
            &imports,
            &path("src/graphql-contract/collector.ts"),
        )
        .expect("ignored file must not force a full run");

        assert_eq!(
            import_edges.dependencies,
            Box::<[roots::RootRelativePath]>::from([path("src/graphql-contract/types.ts")]),
        );
    }

    #[test]
    fn unsupported_dynamic_import_outside_ignore_list_still_fails_closed() {
        // The ignore list must be scoped to the files it names: a different file
        // with the same unresolvable pattern keeps the fail-closed safety net.
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/router.ts"),
            Box::<str>::from("const page = import(routeName);"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed_ignoring(&["src/graphql-contract/collector.ts"]),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        let error =
            super::ImportResolver::imports_for(&imports, &path("src/router.ts")).unwrap_err();

        assert!(matches!(
            error,
            failure::AppError::UnknownDynamicImport { importer } if importer == path("src/router.ts")
        ));
    }

    #[test]
    fn dynamic_import_ignore_supports_glob_matching() {
        // A directory glob should exempt every file under the noisy subtree, not
        // just one exact path, so teams can quarantine a whole module.
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/graphql-contract/loader.ts"),
            Box::<str>::from("const m = import(absolutePath);"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed_ignoring(&["src/graphql-contract/**"]),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        super::ImportResolver::imports_for(&imports, &path("src/graphql-contract/loader.ts"))
            .expect("glob-matched file must not force a full run");
    }

    #[test]
    fn graph_output_imports_keep_static_edges_across_dynamic_imports() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([
            (
                path("src/router.ts"),
                Box::<str>::from(
                    "import './routes';\nimport './missing';\nconst page = import(routeName);",
                ),
            ),
            (
                path("src/routes.ts"),
                Box::<str>::from("export const routes = [];"),
            ),
        ]));
        let imports = super::LocalImports::for_graph_output(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed(),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // Graph inspection should remain useful even when selection will fail
        // closed for unsupported dynamic or unresolved local imports.
        let import_edges =
            super::ImportResolver::imports_for(&imports, &path("src/router.ts")).unwrap();

        assert_eq!(
            import_edges.dependencies,
            Box::<[roots::RootRelativePath]>::from([path("src/routes.ts")]),
        );
    }

    #[test]
    fn unresolved_local_imports_fail_closed() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/pages/home.ts"),
            Box::<str>::from("import { missing } from './missing';"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig::fail_closed(),
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // An unresolved relative import means the dependency graph is incomplete;
        // failing closed avoids returning a misleading partial result.
        let error =
            super::ImportResolver::imports_for(&imports, &path("src/pages/home.ts")).unwrap_err();

        assert!(matches!(
            error,
            failure::AppError::UnresolvedLocalImport { importer, specifier }
                if importer == path("src/pages/home.ts") && specifier.as_str() == "./missing"
        ));
    }
}
