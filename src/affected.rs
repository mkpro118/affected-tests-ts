//! Affected-test selection contracts and fail-closed decision types.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::failure;
use crate::roots;
use crate::vcs;
use crate::work;

/// Reverse graph access required by affected selection.
pub trait GraphView {
    /// Returns direct reverse dependents in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];
}

/// File classification required by affected selection.
pub trait PathClassifier {
    /// Reports whether a path is a source file.
    #[must_use]
    fn is_source(&self, path: &roots::RootRelativePath) -> bool;

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
    /// A deleted source file prevents reliable reverse tracing.
    DeletedSourceFile(roots::RootRelativePath),
    /// A local import could not be resolved safely.
    UnresolvedLocalImport(roots::RootRelativePath),
    /// A non-literal dynamic import requires the full suite.
    UnknownDynamicImport(roots::RootRelativePath),
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
    /// Reverse graph view or a typed graph-build failure.
    pub graph: failure::Result<G>,
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
pub fn select<G, C, A, T>(request: SelectionRequest<G, C, A, T>) -> failure::Result<AffectedResult>
where
    G: GraphView + Sync,
    C: PathClassifier + Sync,
    A: AlwaysRun,
    T: vcs::ChangeSetView,
{
    let SelectionRequest {
        graph: graph_result,
        classifier,
        always_run,
        changes,
    } = request;

    if let Some(reason) = full_reason(&classifier, &changes) {
        return Ok(AffectedResult::Full(reason));
    }
    let graph = match graph_result {
        Ok(graph) => graph,
        Err(error) => return full_result_from_graph_error(error),
    };

    let trace_result = work::changed_files(work::Request {
        graph: TraceGraph { graph: &graph },
        classifier: TraceClassifier {
            classifier: &classifier,
        },
        progress: SilentProgress,
        memo: work::TraceMemo::default(),
        scheduler: work::TraceScheduler::new(worker_count()),
        changed_files: changed_paths(&changes),
    })?;

    Ok(decision_from_trace(
        trace_result,
        always_run.always_run_tests(),
    ))
}

#[derive(Clone, Copy, Debug)]
struct TraceGraph<'a, G> {
    graph: &'a G,
}

impl<G> work::GraphView for TraceGraph<'_, G>
where
    G: GraphView,
{
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath] {
        self.graph.reverse_dependents(path)
    }
}

#[derive(Clone, Copy, Debug)]
struct TraceClassifier<'a, C> {
    classifier: &'a C,
}

impl<C> work::TestClassifier for TraceClassifier<'_, C>
where
    C: PathClassifier,
{
    fn is_test(&self, path: &roots::RootRelativePath) -> bool {
        self.classifier.is_test(path)
    }
}

#[derive(Clone, Copy, Debug)]
struct SilentProgress;

impl work::TraceProgressSink for SilentProgress {
    fn send(&self, _event: work::TraceEvent) {}
}

fn full_reason<C, T>(classifier: &C, changes: &T) -> Option<FullReason>
where
    C: PathClassifier,
    T: vcs::ChangeSetView,
{
    for change in changes.files() {
        if classifier.is_global_invalidator(&change.path) {
            return Some(FullReason::GlobalInvalidator(change.path.clone()));
        }
        if change.status == vcs::ChangedFileStatus::Deleted && classifier.is_source(&change.path) {
            return Some(FullReason::DeletedSourceFile(change.path.clone()));
        }
    }

    None
}

fn full_result_from_graph_error(error: failure::AppError) -> failure::Result<AffectedResult> {
    match error {
        failure::AppError::UnresolvedLocalImport { importer, .. } => Ok(AffectedResult::Full(
            FullReason::UnresolvedLocalImport(importer),
        )),
        failure::AppError::UnknownDynamicImport { importer } => Ok(AffectedResult::Full(
            FullReason::UnknownDynamicImport(importer),
        )),
        other_error @ (failure::AppError::InvalidRootRelativePath { .. }
        | failure::AppError::InvalidImportSpecifier { .. }
        | failure::AppError::Config { .. }
        | failure::AppError::Parse { .. }
        | failure::AppError::FileSystem { .. }
        | failure::AppError::Git { .. }
        | failure::AppError::Graph { .. }
        | failure::AppError::Output { .. }) => Err(other_error),
    }
}

fn changed_paths<T>(changes: &T) -> Box<[roots::RootRelativePath]>
where
    T: vcs::ChangeSetView,
{
    changes
        .files()
        .iter()
        .map(|change| change.path.clone())
        .collect()
}

fn decision_from_trace(
    trace_result: work::TraceResult,
    always_run_tests: &[roots::RootRelativePath],
) -> AffectedResult {
    let tests = selected_tests(&trace_result.tests, always_run_tests);
    if tests.is_empty() {
        return AffectedResult::None;
    }

    AffectedResult::Partial(PartialDecision {
        tests,
        reasons: selection_reasons(trace_result.reasons),
    })
}

fn selected_tests(
    trace_tests: &[roots::RootRelativePath],
    always_run_tests: &[roots::RootRelativePath],
) -> Box<[roots::RootRelativePath]> {
    trace_tests
        .iter()
        .chain(always_run_tests)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn selection_reasons(trace_reasons: Box<[work::TraceReason]>) -> Box<[SelectionReason]> {
    trace_reasons
        .into_vec()
        .into_iter()
        .map(|reason| SelectionReason {
            changed_file: reason.start,
            test_file: reason.test,
            path: reason.path,
        })
        .collect()
}

fn worker_count() -> NonZeroUsize {
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
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
        sources: BTreeSet<roots::RootRelativePath>,
        tests: BTreeSet<roots::RootRelativePath>,
        invalidators: BTreeSet<roots::RootRelativePath>,
    }

    impl super::PathClassifier for FixtureClassifier {
        fn is_source(&self, path: &roots::RootRelativePath) -> bool {
            self.sources.contains(path)
        }

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
            sources: BTreeSet::from([path("src/file-a.ts"), path("src/file-d.ts")]),
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
            graph: Ok(graph()),
            classifier: classifier(),
            always_run: FixtureAlwaysRun::default(),
            changes: FixtureChanges { files: changes },
        }
    }

    #[test]
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
                reasons: Box::from([
                    super::SelectionReason {
                        changed_file: path("src/file-a.ts"),
                        test_file: path("src/file-a.test.ts"),
                        path: Box::from([path("src/file-a.ts"), path("src/file-a.test.ts")]),
                    },
                    super::SelectionReason {
                        changed_file: path("src/file-d.test.tsx"),
                        test_file: path("src/file-d.test.tsx"),
                        path: Box::from([path("src/file-d.test.tsx")]),
                    },
                ]),
            }),
        );
    }

    #[test]
    fn returns_none_for_docs_only_changes_without_always_run_tests() {
        let result = super::select(request(Box::from([changed(
            vcs::ChangedFileStatus::Modified,
            "docs/usage.md",
        )])))
        .unwrap();

        assert_eq!(result, super::AffectedResult::None);
    }

    #[test]
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
            super::AffectedResult::Full(super::FullReason::DeletedSourceFile(path(
                "src/file-a.ts"
            ))),
        );
    }

    #[test]
    fn public_api_accepts_alternate_trait_implementations_without_module_changes() {
        let request = super::SelectionRequest {
            graph: Ok(AlternateGraph {
                dependent: path("src/file-a.test.ts"),
            }),
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
                reasons: Box::from([super::SelectionReason {
                    changed_file: path("src/file-a.ts"),
                    test_file: path("src/file-a.test.ts"),
                    path: Box::from([path("src/file-a.ts"), path("src/file-a.test.ts")]),
                }]),
            }),
        );
    }

    #[test]
    fn returns_partial_with_always_run_tests_for_docs_only_changes() {
        let selection_request = super::SelectionRequest {
            graph: Ok(graph()),
            classifier: classifier(),
            always_run: FixtureAlwaysRun {
                tests: Box::from([path("src/file-d.test.tsx")]),
            },
            changes: FixtureChanges {
                files: Box::from([changed(vcs::ChangedFileStatus::Modified, "docs/usage.md")]),
            },
        };

        // Always-run tests make docs-only changes observable in partial mode
        // without inventing graph reason chains that do not exist.
        let result = super::select(selection_request).unwrap();

        assert_eq!(
            result,
            super::AffectedResult::Partial(super::PartialDecision {
                tests: Box::from([path("src/file-d.test.tsx")]),
                reasons: Box::from([]),
            }),
        );
    }

    #[test]
    fn converts_fail_closed_graph_errors_to_full_selection_reasons() {
        let unresolved_result = super::select(super::SelectionRequest {
            graph: Err::<FixtureGraph, crate::failure::AppError>(
                crate::failure::AppError::UnresolvedLocalImport {
                    importer: path("src/file-a.ts"),
                    specifier: roots::ImportSpecifier::try_from("./missing").unwrap(),
                },
            ),
            classifier: classifier(),
            always_run: FixtureAlwaysRun::default(),
            changes: FixtureChanges {
                files: Box::from([changed(vcs::ChangedFileStatus::Modified, "src/file-a.ts")]),
            },
        })
        .unwrap();

        assert_eq!(
            unresolved_result,
            super::AffectedResult::Full(super::FullReason::UnresolvedLocalImport(path(
                "src/file-a.ts"
            ))),
        );

        let dynamic_result = super::select(super::SelectionRequest {
            graph: Err::<FixtureGraph, crate::failure::AppError>(
                crate::failure::AppError::UnknownDynamicImport {
                    importer: path("src/file-d.ts"),
                },
            ),
            classifier: classifier(),
            always_run: FixtureAlwaysRun::default(),
            changes: FixtureChanges {
                files: Box::from([changed(vcs::ChangedFileStatus::Modified, "src/file-d.ts")]),
            },
        })
        .unwrap();

        assert_eq!(
            dynamic_result,
            super::AffectedResult::Full(super::FullReason::UnknownDynamicImport(path(
                "src/file-d.ts"
            ))),
        );
    }

    #[test]
    fn deleted_docs_only_changes_follow_docs_only_selection_rules() {
        let none_result = super::select(request(Box::from([changed(
            vcs::ChangedFileStatus::Deleted,
            "docs/usage.md",
        )])))
        .unwrap();

        assert_eq!(none_result, super::AffectedResult::None);

        let partial_result = super::select(super::SelectionRequest {
            graph: Ok(graph()),
            classifier: classifier(),
            always_run: FixtureAlwaysRun {
                tests: Box::from([path("src/file-a.test.ts")]),
            },
            changes: FixtureChanges {
                files: Box::from([changed(vcs::ChangedFileStatus::Deleted, "docs/usage.md")]),
            },
        })
        .unwrap();

        assert_eq!(
            partial_result,
            super::AffectedResult::Partial(super::PartialDecision {
                tests: Box::from([path("src/file-a.test.ts")]),
                reasons: Box::from([]),
            }),
        );
    }
}
