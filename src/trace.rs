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
