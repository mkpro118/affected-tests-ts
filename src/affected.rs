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
