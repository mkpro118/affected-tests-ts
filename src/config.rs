//! Configuration contracts for discovery, resolution, and invalidation rules.

use crate::failure;
use crate::roots;

/// Behavior used when a dynamic import cannot be statically resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownDynamicImportBehavior {
    /// Treat unresolved dynamic imports as selecting the full suite.
    FailClosed,
    /// Ignore unresolved dynamic imports during graph construction.
    Ignore,
}

/// Immutable configuration values consumed by pure selection modules.
pub trait View {
    /// Returns source include globs.
    #[must_use]
    fn source_includes(&self) -> &[Pattern];

    /// Returns file exclusion globs.
    #[must_use]
    fn excludes(&self) -> &[Pattern];

    /// Returns test file globs.
    #[must_use]
    fn test_patterns(&self) -> &[TestFilePattern];

    /// Returns global invalidator globs.
    #[must_use]
    fn global_invalidators(&self) -> &[Pattern];

    /// Returns dynamic import behavior.
    #[must_use]
    fn dynamic_imports(&self) -> UnknownDynamicImportBehavior;
}

/// Glob pattern text retained as a typed configuration value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pattern {
    value: Box<str>,
}

impl Pattern {
    /// Returns the configured glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

/// Glob pattern used to identify test files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestFilePattern {
    value: Box<str>,
}

impl TestFilePattern {
    /// Returns the configured test-file glob text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_ref()
    }
}

/// Raw configuration loaded from the repository boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    source_includes: Box<[Pattern]>,
    excludes: Box<[Pattern]>,
    test_patterns: Box<[TestFilePattern]>,
    global_invalidators: Box<[Pattern]>,
    dynamic_imports: UnknownDynamicImportBehavior,
}

/// Resolved configuration consumed by discovery, resolution, and selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    source_includes: Box<[Pattern]>,
    excludes: Box<[Pattern]>,
    test_patterns: Box<[TestFilePattern]>,
    global_invalidators: Box<[Pattern]>,
    dynamic_imports: UnknownDynamicImportBehavior,
}

impl View for ResolvedConfig {
    fn source_includes(&self) -> &[Pattern] {
        self.source_includes.as_ref()
    }

    fn excludes(&self) -> &[Pattern] {
        self.excludes.as_ref()
    }

    fn test_patterns(&self) -> &[TestFilePattern] {
        self.test_patterns.as_ref()
    }

    fn global_invalidators(&self) -> &[Pattern] {
        self.global_invalidators.as_ref()
    }

    fn dynamic_imports(&self) -> UnknownDynamicImportBehavior {
        self.dynamic_imports
    }
}

/// Filesystem capability required by configuration loading.
pub trait LoaderFileSystem {
    /// Reads UTF-8 text from a root-relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the adapter cannot read the requested path.
    fn read_text(&self, path: &roots::RootRelativePath) -> failure::Result<Box<str>>;
}

/// Request object for loading repository configuration.
pub struct LoadRequest<F> {
    /// Filesystem adapter used at the configuration boundary.
    pub file_system: F,
    /// Optional root-relative config file path.
    pub config_path: Option<roots::RootRelativePath>,
}

/// Loads repository configuration from disk or defaults.
///
/// # Errors
///
/// Returns an error when configuration input cannot be read or parsed.
pub fn load<F>(_request: LoadRequest<F>) -> failure::Result<ResolvedConfig>
where
    F: LoaderFileSystem,
{
    unimplemented!()
}
