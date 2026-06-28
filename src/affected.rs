//! Affected-test selection contracts and fail-closed decision types.

use crate::failure;
use crate::roots;
use crate::vcs;

/// Reverse graph access required by affected selection.
pub trait GraphView {
    /// Returns direct reverse dependents in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];
}

/// File classification required by affected selection.
pub trait PathClassifier {
    /// Reports whether a path is a test file.
    #[must_use]
    fn is_test(&self, path: &roots::RootRelativePath) -> bool;

    /// Reports whether a path is a global invalidator.
    #[must_use]
    fn is_global_invalidator(&self, path: &roots::RootRelativePath) -> bool;
}

/// Additional test source required by selection.
pub trait AlwaysRun {
    /// Returns tests that must always run in partial mode.
    #[must_use]
    fn always_run_tests(&self) -> &[roots::RootRelativePath];
}

/// Selection strategy when a change cannot be mapped precisely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Fail closed by selecting all tests.
    Full,
    /// Select only graph-reachable tests when possible.
    Partial,
    /// Report no tests when nothing is affected.
    None,
}

/// User-visible reason why the full suite must run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FullReason {
    /// A configured global invalidator changed.
    GlobalInvalidator(roots::RootRelativePath),
    /// A deleted file prevents reliable reverse tracing.
    DeletedFile(roots::RootRelativePath),
    /// Import analysis could not guarantee a safe partial set.
    UnresolvedDynamicImport(roots::RootRelativePath),
}

/// Selection result before rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AffectedResult {
    /// Specific tests are affected.
    Partial(PartialDecision),
    /// The full suite is required.
    Full(FullReason),
    /// No tests are affected.
    None,
}

/// Partial selection details and explain paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialDecision {
    /// Stable sorted selected tests.
    pub tests: Box<[roots::RootRelativePath]>,
    /// Stable sorted reason chains.
    pub reasons: Box<[SelectionReason]>,
}

/// One explanation chain from change to test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionReason {
    /// Changed path that started the chain.
    pub changed_file: roots::RootRelativePath,
    /// Selected test reached by traversal.
    pub test_file: roots::RootRelativePath,
    /// Ordered path from change to selected test.
    pub path: Box<[roots::RootRelativePath]>,
}

/// Request object for affected-test selection.
pub struct SelectionRequest<G, C, A, T> {
    /// Reverse graph view.
    pub graph: G,
    /// File classifier.
    pub classifier: C,
    /// Always-run test provider.
    pub always_run: A,
    /// Changed files detected by Git.
    pub changes: T,
}

/// Selects tests affected by a changed-file set.
///
/// # Errors
///
/// Returns an error when graph traversal cannot produce a deterministic result.
pub fn select<G, C, A, T>(_request: SelectionRequest<G, C, A, T>) -> failure::Result<AffectedResult>
where
    G: GraphView,
    C: PathClassifier,
    A: AlwaysRun,
    T: vcs::ChangeSetView,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::roots;
    use crate::vcs;

    #[derive(Clone, Debug)]
    struct FixtureGraph {
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
        empty: Box<[roots::RootRelativePath]>,
    }

    impl super::GraphView for FixtureGraph {
        fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
            self.reverse_edges
                .get(path)
                .map_or_else(|| self.empty.as_ref(), Box::as_ref)
        }
    }

    #[derive(Clone, Debug)]
    struct AlternateGraph {
        dependent: roots::RootRelativePath,
    }

    impl super::GraphView for AlternateGraph {
        fn reverse_dependents(
            &self,
            _path: &roots::RootRelativePath,
        ) -> &[roots::RootRelativePath] {
            std::slice::from_ref(&self.dependent)
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureClassifier {
        tests: BTreeSet<roots::RootRelativePath>,
        invalidators: BTreeSet<roots::RootRelativePath>,
    }

    impl super::PathClassifier for FixtureClassifier {
        fn is_test(&self, path: &roots::RootRelativePath) -> bool {
            self.tests.contains(path)
        }

        fn is_global_invalidator(&self, path: &roots::RootRelativePath) -> bool {
            self.invalidators.contains(path)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct FixtureAlwaysRun {
        tests: Box<[roots::RootRelativePath]>,
    }

    impl super::AlwaysRun for FixtureAlwaysRun {
        fn always_run_tests(&self) -> &[roots::RootRelativePath] {
            self.tests.as_ref()
        }
    }

    #[derive(Clone, Debug)]
    struct FixtureChanges {
        files: Box<[vcs::ChangedFile]>,
    }

    impl vcs::ChangeSetView for FixtureChanges {
        fn files(&self) -> &[vcs::ChangedFile] {
            self.files.as_ref()
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn changed(status: vcs::ChangedFileStatus, value: &str) -> vcs::ChangedFile {
        vcs::ChangedFile {
            status,
            path: path(value),
            previous_path: None,
        }
    }

    fn classifier() -> FixtureClassifier {
        FixtureClassifier {
            tests: BTreeSet::from([path("src/file-a.test.ts"), path("src/file-d.test.tsx")]),
            invalidators: BTreeSet::from([path("tsconfig.json")]),
        }
    }

    fn graph() -> FixtureGraph {
        FixtureGraph {
            reverse_edges: BTreeMap::from([
                (
                    path("src/file-a.ts"),
                    Box::from([path("src/file-a.test.ts")]),
                ),
                (
                    path("src/file-d.ts"),
                    Box::from([path("src/file-d.test.tsx")]),
                ),
            ]),
            empty: Box::from([]),
        }
    }

    fn request(
        changes: Box<[vcs::ChangedFile]>,
    ) -> super::SelectionRequest<FixtureGraph, FixtureClassifier, FixtureAlwaysRun, FixtureChanges>
    {
        super::SelectionRequest {
            graph: graph(),
            classifier: classifier(),
            always_run: FixtureAlwaysRun::default(),
            changes: FixtureChanges { files: changes },
        }
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn returns_partial_for_transitive_dependents_and_includes_changed_tests() {
        let request = request(Box::from([
            changed(vcs::ChangedFileStatus::Modified, "src/file-a.ts"),
            changed(vcs::ChangedFileStatus::Modified, "src/file-d.test.tsx"),
        ]));

        // A source change plus a changed test proves selector behavior covers both
        // reverse graph traversal and the direct "run the edited test" rule.
        let result = super::select(request).unwrap();

        assert_eq!(
            result,
            super::AffectedResult::Partial(super::PartialDecision {
                tests: Box::from([path("src/file-a.test.ts"), path("src/file-d.test.tsx")]),
                reasons: Box::from([]),
            }),
        );
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn returns_none_for_docs_only_changes_without_always_run_tests() {
        let result = super::select(request(Box::from([changed(
            vcs::ChangedFileStatus::Modified,
            "docs/usage.md",
        )])))
        .unwrap();

        assert_eq!(result, super::AffectedResult::None);
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn returns_full_for_global_invalidators_and_deleted_source_files() {
        let global_result = super::select(request(Box::from([changed(
            vcs::ChangedFileStatus::Modified,
            "tsconfig.json",
        )])))
        .unwrap();

        assert_eq!(
            global_result,
            super::AffectedResult::Full(super::FullReason::GlobalInvalidator(path(
                "tsconfig.json",
            ))),
        );

        let deleted_result = super::select(request(Box::from([changed(
            vcs::ChangedFileStatus::Deleted,
            "src/file-a.ts",
        )])))
        .unwrap();

        assert_eq!(
            deleted_result,
            super::AffectedResult::Full(super::FullReason::DeletedFile(path("src/file-a.ts"))),
        );
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn public_api_accepts_alternate_trait_implementations_without_module_changes() {
        let request = super::SelectionRequest {
            graph: AlternateGraph {
                dependent: path("src/file-a.test.ts"),
            },
            classifier: classifier(),
            always_run: FixtureAlwaysRun::default(),
            changes: FixtureChanges {
                files: Box::from([changed(vcs::ChangedFileStatus::Modified, "src/file-a.ts")]),
            },
        };

        // This alternate graph proves callers depend on the selector capability
        // traits, not on the concrete graph type owned by the graph module.
        let result = super::select(request).unwrap();

        assert_eq!(
            result,
            super::AffectedResult::Partial(super::PartialDecision {
                tests: Box::from([path("src/file-a.test.ts")]),
                reasons: Box::from([]),
            }),
        );
    }
}
