//! Source and test file discovery contracts.

use crate::failure;
use crate::roots;
use crate::settings;

/// Filesystem capability consumed by discovery.
pub trait SourceDiscoverer {
    /// Returns candidate paths before config classification.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing catalog cannot be read.
    fn candidate_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>>;
}

/// Request object for file discovery.
pub struct Request<C, F> {
    /// Configuration view used for include and exclude decisions.
    pub config: C,
    /// Filesystem catalog used to enumerate files.
    pub file_catalog: F,
}

/// Discovered source and test paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Files {
    /// Stable sorted source files.
    pub sources: Box<[roots::RootRelativePath]>,
    /// Stable sorted test files.
    pub tests: Box<[roots::RootRelativePath]>,
}

/// Discovers source and test files from configured roots.
///
/// # Errors
///
/// Returns an error when file enumeration or pattern matching fails.
pub fn files<C, F>(request: Request<C, F>) -> failure::Result<Files>
where
    C: settings::View,
    F: SourceDiscoverer,
{
    let Request {
        config,
        file_catalog,
    } = request;
    let patterns = DiscoveryPatterns::compile(&config)?;
    let mut sources = Vec::<roots::RootRelativePath>::new();
    let mut tests = Vec::<roots::RootRelativePath>::new();

    for path in file_catalog.candidate_paths()? {
        if !patterns.should_include(&path) {
            continue;
        }

        if patterns.is_test(&path) {
            tests.push(path);
        } else {
            sources.push(path);
        }
    }

    Ok(Files {
        sources: sorted_unique(sources),
        tests: sorted_unique(tests),
    })
}

struct DiscoveryPatterns {
    includes: globset::GlobSet,
    excludes: globset::GlobSet,
    tests: globset::GlobSet,
}

impl DiscoveryPatterns {
    fn compile<C>(config: &C) -> failure::Result<Self>
    where
        C: settings::View,
    {
        Ok(Self {
            includes: source_glob_set(config.source_includes())?,
            excludes: source_glob_set(config.excludes())?,
            tests: test_glob_set(config.test_patterns())?,
        })
    }

    fn should_include(&self, path: &roots::RootRelativePath) -> bool {
        self.includes.is_match(path.as_str()) && !self.excludes.is_match(path.as_str())
    }

    fn is_test(&self, path: &roots::RootRelativePath) -> bool {
        self.tests.is_match(path.as_str())
    }
}

fn source_glob_set(patterns: &[settings::Pattern]) -> failure::Result<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!("invalid discovery glob `{}`: {error}", pattern.as_str())
                    .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile discovery glob set: {error}").into_boxed_str(),
    })
}

fn test_glob_set(patterns: &[settings::TestFilePattern]) -> failure::Result<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            globset::Glob::new(pattern.as_str()).map_err(|error| failure::AppError::Config {
                message: format!(
                    "invalid discovery test glob `{}`: {error}",
                    pattern.as_str()
                )
                .into_boxed_str(),
            })?;
        builder.add(glob);
    }

    builder.build().map_err(|error| failure::AppError::Config {
        message: format!("failed to compile discovery test glob set: {error}").into_boxed_str(),
    })
}

fn sorted_unique(mut paths: Vec<roots::RootRelativePath>) -> Box<[roots::RootRelativePath]> {
    paths.sort();
    paths.dedup();
    paths.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use crate::failure;
    use crate::roots;
    use crate::settings;
    use crate::vfs;

    #[derive(Clone, Debug)]
    struct FixtureConfig {
        source_includes: Box<[settings::Pattern]>,
        excludes: Box<[settings::Pattern]>,
        test_patterns: Box<[settings::TestFilePattern]>,
    }

    impl settings::View for FixtureConfig {
        fn source_includes(&self) -> &[settings::Pattern] {
            self.source_includes.as_ref()
        }

        fn excludes(&self) -> &[settings::Pattern] {
            self.excludes.as_ref()
        }

        fn test_patterns(&self) -> &[settings::TestFilePattern] {
            self.test_patterns.as_ref()
        }

        fn global_invalidators(&self) -> &[settings::Pattern] {
            &[]
        }

        fn dynamic_imports(&self) -> settings::UnknownDynamicImportBehavior {
            settings::UnknownDynamicImportBehavior::FailClosed
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn pattern(value: &str) -> settings::Pattern {
        settings::Pattern::try_from(value).unwrap()
    }

    fn test_pattern(value: &str) -> settings::TestFilePattern {
        settings::TestFilePattern::try_from(value).unwrap()
    }

    fn config() -> FixtureConfig {
        FixtureConfig {
            source_includes: Box::from([pattern("src/**/*.ts"), pattern("src/**/*.tsx")]),
            excludes: Box::from([pattern("src/generated/**")]),
            test_patterns: Box::from([test_pattern("**/*.test.ts"), test_pattern("**/*.test.tsx")]),
        }
    }

    fn file_system() -> vfs::VirtualFileSystem {
        vfs::VirtualFileSystem::with_files(Box::from([
            (
                path("src/accounts/service.ts"),
                Box::<str>::from("export const service = true;"),
            ),
            (
                path("src/accounts/service.test.ts"),
                Box::<str>::from("import './service';"),
            ),
            (
                path("src/generated/types.ts"),
                Box::<str>::from("export type Generated = string;"),
            ),
            (
                path("scripts/build.ts"),
                Box::<str>::from("export const build = true;"),
            ),
        ]))
    }

    #[test]
    fn include_exclude_and_test_patterns_partition_discovered_files() {
        let request = super::Request {
            config: config(),
            file_catalog: file_system(),
        };

        // The fixture has an included source, an included test, an excluded
        // generated file, and an out-of-root script to prove every partition.
        let files = super::files(request).unwrap();

        assert_eq!(
            files.sources,
            Box::<[roots::RootRelativePath]>::from([path("src/accounts/service.ts")]),
        );
        assert_eq!(
            files.tests,
            Box::<[roots::RootRelativePath]>::from([path("src/accounts/service.test.ts")]),
        );
    }

    #[test]
    fn discovery_errors_when_candidate_catalog_fails() {
        #[derive(Clone, Debug)]
        struct FailingCatalog;

        impl super::SourceDiscoverer for FailingCatalog {
            fn candidate_paths(&self) -> failure::Result<Box<[roots::RootRelativePath]>> {
                Err(failure::AppError::FileSystem {
                    message: Box::<str>::from("catalog unavailable"),
                })
            }
        }

        let request = super::Request {
            config: config(),
            file_catalog: FailingCatalog,
        };

        // Discovery owns classification only; catalog failures must surface
        // rather than silently producing an empty partial graph.
        assert!(super::files(request).is_err());
    }
}
