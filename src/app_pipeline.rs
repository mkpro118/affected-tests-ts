//! Application pipeline composition for config, discovery, graph, and classifiers.

use std::collections::BTreeSet;

use crate::dependencies;
use crate::discovery;
use crate::failure;
use crate::fs;
use crate::impact;
use crate::modules;
use crate::roots;
use crate::settings;

/// Built repository state needed by command handlers.
pub struct Pipeline {
    /// Resolved repository configuration.
    pub config: settings::ResolvedConfig,
    /// Discovered source and test files.
    pub files: discovery::Files,
    /// Dependency graph or fail-closed graph construction result.
    pub graph: failure::Result<dependencies::DependencyGraph>,
}

/// File classifier used by affected selection at the app edge.
#[derive(Clone, Debug)]
pub struct Classifier {
    sources: globset::GlobSet,
    excludes: globset::GlobSet,
    tests: globset::GlobSet,
    invalidators: globset::GlobSet,
}

impl Classifier {
    /// Compiles classification globs from resolved configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when any configured glob cannot compile.
    pub fn try_new<C>(config: &C) -> failure::Result<Self>
    where
        C: settings::View,
    {
        Ok(Self {
            sources: pattern_glob_set(settings::View::source_includes(config))?,
            excludes: pattern_glob_set(settings::View::excludes(config))?,
            tests: test_glob_set(settings::View::test_patterns(config))?,
            invalidators: pattern_glob_set(settings::View::global_invalidators(config))?,
        })
    }
}

impl impact::PathClassifier for Classifier {
    fn is_source(&self, path: &roots::RootRelativePath) -> bool {
        self.sources.is_match(path.as_str())
            && !self.excludes.is_match(path.as_str())
            && !self.tests.is_match(path.as_str())
    }

    fn is_test(&self, path: &roots::RootRelativePath) -> bool {
        self.tests.is_match(path.as_str()) && !self.excludes.is_match(path.as_str())
    }

    fn is_global_invalidator(&self, path: &roots::RootRelativePath) -> bool {
        self.invalidators.is_match(path.as_str())
    }
}

/// Empty always-run provider for the current CLI surface.
#[derive(Clone, Debug, Default)]
pub struct EmptyAlwaysRun {
    tests: Box<[roots::RootRelativePath]>,
}

impl impact::AlwaysRun for EmptyAlwaysRun {
    fn always_run_tests(&self) -> &[roots::RootRelativePath] {
        self.tests.as_ref()
    }
}

#[derive(Clone, Copy, Debug)]
struct Resolver;

impl modules::ImportResolver for Resolver {
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

impl impact::GraphView for dependencies::DependencyGraph {
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        dependencies::GraphView::reverse_dependents(self, path)
    }
}

/// Builds repository pipeline state from a real filesystem root.
///
/// # Errors
///
/// Returns an error when config loading or file discovery fails.
pub fn build(repository_path: Box<str>) -> failure::Result<Pipeline> {
    let file_system = fs::RealFileSystem::for_root(repository_path);
    let config = settings::load(settings::LoadRequest {
        file_system: file_system.clone(),
        config_path: config_path(&file_system)?,
    })?;
    let files = discovery::files(discovery::Request {
        config: config.clone(),
        file_catalog: file_system.clone(),
    })?;
    let graph_files = all_graph_files(&files);
    let imports = dependencies::LocalImports::new(dependencies::LocalImportsRequest {
        config: config.clone(),
        reader: file_system.clone(),
        resolver: Resolver,
        probe: file_system,
    });
    let graph = dependencies::build(dependencies::GraphBuildRequest {
        imports,
        files: graph_files,
    });

    Ok(Pipeline {
        config,
        files,
        graph,
    })
}

/// Returns the stable source and test path set included in graph output.
#[must_use]
pub fn all_graph_files(files: &discovery::Files) -> Box<[roots::RootRelativePath]> {
    files
        .sources
        .iter()
        .chain(files.tests.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn config_path(
    file_system: &fs::RealFileSystem,
) -> failure::Result<Option<roots::RootRelativePath>> {
    for candidate in ["affected-tests.json", "tsconfig.json"] {
        let path = roots::RootRelativePath::try_from(candidate)?;
        if fs::FileExistence::exists(file_system, &path)? {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn pattern_glob_set(patterns: &[settings::Pattern]) -> failure::Result<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!("invalid classifier glob `{}`: {error}", pattern.as_str())
                    .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile classifier globs: {error}").into_boxed_str(),
    })
}

fn test_glob_set(patterns: &[settings::TestFilePattern]) -> failure::Result<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!(
                    "invalid classifier test glob `{}`: {error}",
                    pattern.as_str()
                )
                .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile classifier test globs: {error}").into_boxed_str(),
    })
}
