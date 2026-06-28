//! Import resolution contracts for relative paths, extensions, index files, and aliases.

use crate::failure;
use crate::roots;
use crate::settings;

/// Filesystem probing capability used by import resolution.
pub trait FileExistence {
    /// Reports whether a candidate root-relative path exists.
    ///
    /// # Errors
    ///
    /// Returns an error when probing cannot complete.
    fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool>;
}

/// Request object for resolving one import specifier.
pub struct ResolveRequest<C, P> {
    /// Configuration view used for aliases and extension policy.
    pub config: C,
    /// Filesystem probe used for candidate checks.
    pub probe: P,
    /// Importing file path.
    pub importer: roots::RootRelativePath,
    /// Import specifier to resolve.
    pub specifier: roots::ImportSpecifier,
}

/// Resolution outcome for an import specifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The import resolved to a root-relative source path.
    Resolved(roots::RootRelativePath),
    /// The import is external to the repository graph.
    External(roots::ImportSpecifier),
    /// The import is local but unresolved.
    Unresolved(roots::ImportSpecifier),
}

/// Import resolver capability used by graph construction.
pub trait ImportResolver {
    /// Resolves one import request into a graph edge outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when filesystem probing or configuration access fails.
    fn resolve<C, P>(&self, request: ResolveRequest<C, P>) -> failure::Result<Outcome>
    where
        C: settings::View,
        P: FileExistence;
}

/// Resolves an import specifier into a graph edge target when possible.
///
/// # Errors
///
/// Returns an error when filesystem probing or configuration access fails.
pub fn import<C, P>(_request: ResolveRequest<C, P>) -> failure::Result<Outcome>
where
    C: settings::View,
    P: FileExistence,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::failure;
    use crate::roots;
    use crate::settings;

    #[derive(Clone, Debug, Default)]
    struct FixtureConfig {
        source_includes: Box<[settings::Pattern]>,
        excludes: Box<[settings::Pattern]>,
        test_patterns: Box<[settings::TestFilePattern]>,
        global_invalidators: Box<[settings::Pattern]>,
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
            self.global_invalidators.as_ref()
        }

        fn dynamic_imports(&self) -> settings::UnknownDynamicImportBehavior {
            settings::UnknownDynamicImportBehavior::FailClosed
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureProbe {
        existing_paths: BTreeSet<roots::RootRelativePath>,
    }

    impl super::FileExistence for FixtureProbe {
        fn exists(&self, path: &roots::RootRelativePath) -> failure::Result<bool> {
            Ok(self.existing_paths.contains(path))
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn specifier(value: &str) -> roots::ImportSpecifier {
        roots::ImportSpecifier::try_from(value).unwrap()
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn resolves_relative_extensions_indexes_and_ts_path_aliases() {
        let probe = FixtureProbe {
            existing_paths: BTreeSet::from([
                path("src/components/button.tsx"),
                path("src/components/menu/index.ts"),
                path("src/shared/date.ts"),
            ]),
        };
        let relative_request = super::ResolveRequest {
            config: FixtureConfig::default(),
            probe: probe.clone(),
            importer: path("src/pages/home.tsx"),
            specifier: specifier("../components/button"),
        };

        // The fixture names extensionless, index, and alias-shaped imports because
        // these are the resolution cases that determine graph completeness.
        assert_eq!(
            super::import(relative_request).unwrap(),
            super::Outcome::Resolved(path("src/components/button.tsx")),
        );

        let index_request = super::ResolveRequest {
            config: FixtureConfig::default(),
            probe: probe.clone(),
            importer: path("src/pages/home.tsx"),
            specifier: specifier("../components/menu"),
        };

        assert_eq!(
            super::import(index_request).unwrap(),
            super::Outcome::Resolved(path("src/components/menu/index.ts")),
        );

        let alias_request = super::ResolveRequest {
            config: FixtureConfig::default(),
            probe,
            importer: path("src/pages/home.tsx"),
            specifier: specifier("@shared/date"),
        };

        assert_eq!(
            super::import(alias_request).unwrap(),
            super::Outcome::Resolved(path("src/shared/date.ts")),
        );
    }
}
