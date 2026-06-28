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
}

/// Import source used by graph construction.
pub trait ImportResolver {
    /// Returns resolved dependencies for a file in stable order.
    ///
    /// # Errors
    ///
    /// Returns an error when imports cannot be parsed or resolved.
    fn dependencies_for(
        &self,
        path: &roots::RootRelativePath,
    ) -> failure::Result<Box<[roots::RootRelativePath]>>;
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
    fn dependencies_for(
        &self,
        path: &roots::RootRelativePath,
    ) -> failure::Result<Box<[roots::RootRelativePath]>> {
        let imports = parser::imports(parser::Request {
            reader: self.reader.clone(),
            path: path.clone(),
        })?;

        if should_fail_for_dynamic_imports(&self.config, imports.unsupported_dynamic_imports.len())
        {
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
}

struct NodeEdges {
    path: roots::RootRelativePath,
    dependencies: Box<[roots::RootRelativePath]>,
}

fn should_fail_for_dynamic_imports<C>(config: &C, unsupported_count: usize) -> bool
where
    C: settings::View,
{
    unsupported_count > 0
        && config.dynamic_imports() == settings::UnknownDynamicImportBehavior::FailClosed
}

fn resolve_imports<C, M, P>(
    request: ResolveImportsRequest<C, M, P>,
) -> failure::Result<Box<[roots::RootRelativePath]>>
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
    } = request;
    let mut dependencies = Vec::<roots::RootRelativePath>::new();
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
            modules::Outcome::External(_specifier) => {}
            modules::Outcome::Unresolved(specifier) => {
                return Err(failure::AppError::UnresolvedLocalImport {
                    importer,
                    specifier,
                });
            }
        }
    }

    Ok(sorted_unique(dependencies))
}

fn node_edges<I>(imports: &I, path: roots::RootRelativePath) -> failure::Result<NodeEdges>
where
    I: ImportResolver,
{
    Ok(NodeEdges {
        dependencies: imports.dependencies_for(&path)?,
        path,
    })
}

fn graph_from_edges(edge_sets: Vec<NodeEdges>) -> DependencyGraph {
    let mut dependencies =
        BTreeMap::<roots::RootRelativePath, Box<[roots::RootRelativePath]>>::new();
    let mut reverse_edges =
        BTreeMap::<roots::RootRelativePath, Vec<roots::RootRelativePath>>::new();

    for edge_set in edge_sets {
        reverse_edges.entry(edge_set.path.clone()).or_default();
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

    DependencyGraph {
        dependencies,
        reverse_dependencies,
        empty: Box::from([]),
    }
}

fn sorted_unique(mut paths: Vec<roots::RootRelativePath>) -> Box<[roots::RootRelativePath]> {
    paths.sort();
    paths.dedup();
    paths.into_boxed_slice()
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
        empty: Box<[roots::RootRelativePath]>,
    }

    impl super::ImportResolver for FixtureImports {
        fn dependencies_for(
            &self,
            path: &roots::RootRelativePath,
        ) -> failure::Result<Box<[roots::RootRelativePath]>> {
            Ok(self
                .edges
                .get(path)
                .map_or_else(|| self.empty.clone(), Clone::clone))
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureConfig {
        dynamic_imports: settings::UnknownDynamicImportBehavior,
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
            config: FixtureConfig {
                dynamic_imports: settings::UnknownDynamicImportBehavior::FailClosed,
            },
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // The fixture exercises the production parser and resolver path without
        // touching host files, proving graph build can consume real source text.
        let dependencies =
            super::ImportResolver::dependencies_for(&imports, &path("src/pages/home.ts")).unwrap();

        assert_eq!(
            dependencies,
            Box::<[roots::RootRelativePath]>::from([path("src/components/button.ts")]),
        );
    }

    #[test]
    fn unsupported_dynamic_imports_fail_closed_when_configured() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/router.ts"),
            Box::<str>::from("const page = import(routeName);"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig {
                dynamic_imports: settings::UnknownDynamicImportBehavior::FailClosed,
            },
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // Non-literal dynamic imports cannot safely become partial graph edges,
        // so the graph boundary must force callers into full-suite behavior.
        let error =
            super::ImportResolver::dependencies_for(&imports, &path("src/router.ts")).unwrap_err();

        assert!(matches!(
            error,
            failure::AppError::UnknownDynamicImport { importer } if importer == path("src/router.ts")
        ));
    }

    #[test]
    fn unresolved_local_imports_fail_closed() {
        let file_system = vfs::VirtualFileSystem::with_files(Box::from([(
            path("src/pages/home.ts"),
            Box::<str>::from("import { missing } from './missing';"),
        )]));
        let imports = super::LocalImports::new(super::LocalImportsRequest {
            config: FixtureConfig {
                dynamic_imports: settings::UnknownDynamicImportBehavior::FailClosed,
            },
            reader: file_system.clone(),
            resolver: FixtureResolver,
            probe: file_system,
        });

        // An unresolved relative import means the dependency graph is incomplete;
        // failing closed avoids returning a misleading partial result.
        let error = super::ImportResolver::dependencies_for(&imports, &path("src/pages/home.ts"))
            .unwrap_err();

        assert!(matches!(
            error,
            failure::AppError::UnresolvedLocalImport { importer, specifier }
                if importer == path("src/pages/home.ts") && specifier.as_str() == "./missing"
        ));
    }
}
