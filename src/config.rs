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

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;

    #[derive(Clone, Debug, Default)]
    struct FixtureFileSystem {
        content: Option<Box<str>>,
    }

    impl super::LoaderFileSystem for FixtureFileSystem {
        fn read_text(&self, _path: &roots::RootRelativePath) -> failure::Result<Box<str>> {
            self.content
                .clone()
                .ok_or_else(|| failure::AppError::Config {
                    message: Box::<str>::from("fixture config missing"),
                })
        }
    }

    fn pattern_values(patterns: &[super::Pattern]) -> Box<[Box<str>]> {
        patterns
            .iter()
            .map(|pattern| Box::<str>::from(pattern.as_str()))
            .collect()
    }

    fn test_pattern_values(patterns: &[super::TestFilePattern]) -> Box<[Box<str>]> {
        patterns
            .iter()
            .map(|pattern| Box::<str>::from(pattern.as_str()))
            .collect()
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn defaults_match_prd_patterns_and_invalidators_are_stable() {
        let request = super::LoadRequest {
            file_system: FixtureFileSystem::default(),
            config_path: None,
        };

        let config = super::load(request).unwrap();

        // These defaults represent the V1 TypeScript project shape from the PRD.
        assert_eq!(
            test_pattern_values(super::View::test_patterns(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("**/*.test.ts"),
                Box::<str>::from("**/*.test.tsx"),
                Box::<str>::from("**/*.spec.ts"),
                Box::<str>::from("**/*.spec.tsx"),
                Box::<str>::from("**/__tests__/**/*"),
            ]),
        );
        assert_eq!(
            pattern_values(super::View::global_invalidators(&config)),
            Box::<[Box<str>]>::from([
                Box::<str>::from("package.json"),
                Box::<str>::from("bun.lockb"),
                Box::<str>::from("tsconfig.json"),
            ]),
        );
        assert_eq!(
            super::View::dynamic_imports(&config),
            super::UnknownDynamicImportBehavior::FailClosed,
        );
    }
}
