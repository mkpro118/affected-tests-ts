//! Concurrent reverse tracing contracts with shared in-flight path collapse.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use crate::failure;
use crate::roots;

/// Graph capability required by reverse tracing.
pub trait GraphView {
    /// Returns direct reverse dependents in stable order.
    #[must_use]
    fn reverse_dependents(&self, path: &roots::RootRelativePath) -> &[roots::RootRelativePath];
}

/// Test classification capability required by reverse tracing.
pub trait TestClassifier {
    /// Reports whether a path is a test file.
    #[must_use]
    fn is_test(&self, path: &roots::RootRelativePath) -> bool;
}

/// Progress sink capability used by the trace scheduler.
pub trait TraceProgressSink {
    /// Receives a typed trace event.
    fn send(&self, event: TraceEvent);
}

/// Trace status stored for each graph node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceStatus {
    /// No worker has started tracing this node.
    Unseen,
    /// A worker owns this node and other workers should join or observe it.
    InFlight,
    /// Tracing completed successfully.
    Complete(TraceResult),
    /// Tracing failed for a deterministic reason.
    Failed(Box<str>),
}

/// Trace handle returned when a trace request touches memoized state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceHandle {
    /// Caller should schedule new work for the path.
    Scheduled(roots::RootRelativePath),
    /// Caller should wait for an existing in-flight trace.
    Waiting(roots::RootRelativePath),
    /// Caller can immediately reuse a completed result.
    Completed(TraceResult),
}

/// Deterministic trace output for one graph node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceResult {
    /// Stable sorted tests reached from the node.
    pub tests: Box<[roots::RootRelativePath]>,
    /// Stable sorted shortest reason chains.
    pub reasons: Box<[TraceReason]>,
    /// Stable cycle edges encountered while tracing.
    pub cycle_edges: Box<[CycleEdge]>,
}

/// Shortest reason chain produced by tracing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceReason {
    /// Starting node for the trace.
    pub start: roots::RootRelativePath,
    /// Selected test reached by tracing.
    pub test: roots::RootRelativePath,
    /// Ordered reverse dependency path.
    pub path: Box<[roots::RootRelativePath]>,
}

/// Deterministic representation of a cycle edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CycleEdge {
    /// Source path in the cycle edge.
    pub from: roots::RootRelativePath,
    /// Target path in the cycle edge.
    pub to: roots::RootRelativePath,
}

/// Trace scheduler contract that owns parallel work coordination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceScheduler {
    worker_count: NonZeroUsize,
}

impl TraceScheduler {
    /// Creates a scheduler contract with a non-zero worker count.
    #[must_use]
    pub const fn new(worker_count: NonZeroUsize) -> Self {
        Self { worker_count }
    }

    /// Returns the maximum worker count available to the scheduler.
    #[must_use]
    pub const fn worker_count(&self) -> NonZeroUsize {
        self.worker_count
    }
}

/// Progress event emitted by trace scheduling and memo reuse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    /// A path was scheduled for first-time tracing.
    Scheduled(roots::RootRelativePath),
    /// A path joined an in-flight trace.
    JoinedInFlight(roots::RootRelativePath),
    /// A path reused a completed trace result.
    Reused(roots::RootRelativePath),
    /// A path completed with a deterministic result.
    Completed(roots::RootRelativePath),
    /// A cycle edge was recorded without deadlocking.
    Cycle(CycleEdge),
}

/// Shared memo table for trace coordination.
#[derive(Clone, Debug, Default)]
pub struct TraceMemo {
    statuses: Arc<Mutex<BTreeMap<roots::RootRelativePath, TraceStatus>>>,
}

impl TraceMemo {
    /// Returns the shared status storage.
    #[must_use]
    pub fn statuses(&self) -> Arc<Mutex<BTreeMap<roots::RootRelativePath, TraceStatus>>> {
        Arc::clone(&self.statuses)
    }
}

/// Request object for tracing changed paths.
pub struct Request<G, C, S> {
    /// Reverse graph view.
    pub graph: G,
    /// Test classifier.
    pub classifier: C,
    /// Progress sink for typed trace events.
    pub progress: S,
    /// Shared trace memo.
    pub memo: TraceMemo,
    /// Scheduler configuration for worker coordination.
    pub scheduler: TraceScheduler,
    /// Changed files to trace.
    pub changed_files: Box<[roots::RootRelativePath]>,
}

/// Traces changed files through the reverse graph with shared memoization.
///
/// # Errors
///
/// Returns an error when tracing cannot produce deterministic results.
pub fn changed_files<G, C, S>(_request: Request<G, C, S>) -> failure::Result<TraceResult>
where
    G: GraphView,
    C: TestClassifier,
    S: TraceProgressSink,
{
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};

    use crate::roots;

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
    struct FixtureClassifier {
        test_paths: BTreeSet<roots::RootRelativePath>,
    }

    impl super::TestClassifier for FixtureClassifier {
        fn is_test(&self, path: &roots::RootRelativePath) -> bool {
            self.test_paths.contains(path)
        }
    }

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<super::TraceEvent>>>,
    }

    impl super::TraceProgressSink for RecordingSink {
        fn send(&self, event: super::TraceEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    fn scheduler() -> super::TraceScheduler {
        super::TraceScheduler::new(NonZeroUsize::new(4).unwrap())
    }

    fn classifier(test_paths: Box<[roots::RootRelativePath]>) -> FixtureClassifier {
        FixtureClassifier {
            test_paths: test_paths.into_vec().into_iter().collect(),
        }
    }

    fn graph(
        reverse_edges: BTreeMap<roots::RootRelativePath, Box<[roots::RootRelativePath]>>,
    ) -> FixtureGraph {
        FixtureGraph {
            reverse_edges,
            empty: Box::from([]),
        }
    }

    fn trace(
        graph: FixtureGraph,
        changed_files: Box<[roots::RootRelativePath]>,
    ) -> super::TraceResult {
        let request = super::Request {
            graph,
            classifier: classifier(Box::from([path("tests/file-b.test.ts"), path("fileE")])),
            progress: RecordingSink::default(),
            memo: super::TraceMemo::default(),
            scheduler: scheduler(),
            changed_files,
        };

        super::changed_files(request).unwrap()
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn starts_multiple_changed_files_in_parallel() {
        let graph = graph(BTreeMap::from([
            (path("fileB"), Box::from([path("tests/file-b.test.ts")])),
            (path("fileD"), Box::from([path("fileE")])),
        ]));

        // Two independent roots are enough to prove the scheduler does not serialize
        // work before discovering shared paths.
        let result = trace(graph, Box::from([path("fileB"), path("fileD")]));

        assert_eq!(
            result.tests,
            Box::from([path("fileE"), path("tests/file-b.test.ts")]),
        );
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn reuses_completed_trace_results_when_a_later_path_reaches_the_same_node() {
        let memo = super::TraceMemo::default();
        let completed = super::TraceResult {
            tests: Box::from([path("tests/file-b.test.ts")]),
            reasons: Box::from([]),
            cycle_edges: Box::from([]),
        };
        memo.statuses().lock().unwrap().insert(
            path("fileB"),
            super::TraceStatus::Complete(completed.clone()),
        );
        let request = super::Request {
            graph: graph(BTreeMap::from([(
                path("fileA"),
                Box::from([path("fileB")]),
            )])),
            classifier: classifier(Box::from([path("tests/file-b.test.ts")])),
            progress: RecordingSink::default(),
            memo,
            scheduler: scheduler(),
            changed_files: Box::from([path("fileA")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result, completed);
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn joins_in_flight_trace_results_without_retracing_the_same_path() {
        let memo = super::TraceMemo::default();
        memo.statuses()
            .lock()
            .unwrap()
            .insert(path("fileB"), super::TraceStatus::InFlight);
        let request = super::Request {
            graph: graph(BTreeMap::from([(
                path("fileA"),
                Box::from([path("fileB")]),
            )])),
            classifier: classifier(Box::from([path("tests/file-b.test.ts")])),
            progress: RecordingSink::default(),
            memo,
            scheduler: scheduler(),
            changed_files: Box::from([path("fileA")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result.tests, Box::from([path("tests/file-b.test.ts")]));
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn collapses_file_b_overlap_while_independent_file_d_branch_continues() {
        let graph = graph(BTreeMap::from([
            (path("fileD"), Box::from([path("fileA"), path("fileC")])),
            (path("fileA"), Box::from([path("fileB")])),
            (path("fileC"), Box::from([path("fileE")])),
            (path("fileB"), Box::from([path("tests/file-b.test.ts")])),
        ]));

        // The overlapping branch should join fileB tracing while fileC -> fileE
        // continues, preserving both selected tests in the final result.
        let result = trace(graph, Box::from([path("fileB"), path("fileD")]));

        assert_eq!(
            result.tests,
            Box::from([path("fileE"), path("tests/file-b.test.ts")]),
        );
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn cycles_complete_without_deadlock_or_duplicate_work() {
        let graph = graph(BTreeMap::from([
            (path("src/a.ts"), Box::from([path("src/b.ts")])),
            (
                path("src/b.ts"),
                Box::from([path("src/a.ts"), path("tests/a.test.ts")]),
            ),
        ]));
        let request = super::Request {
            graph,
            classifier: classifier(Box::from([path("tests/a.test.ts")])),
            progress: RecordingSink::default(),
            memo: super::TraceMemo::default(),
            scheduler: scheduler(),
            changed_files: Box::from([path("src/a.ts")]),
        };

        let result = super::changed_files(request).unwrap();

        assert_eq!(result.tests, Box::from([path("tests/a.test.ts")]));
        assert_eq!(
            result.cycle_edges,
            Box::from([super::CycleEdge {
                from: path("src/b.ts"),
                to: path("src/a.ts"),
            }]),
        );
    }
}
