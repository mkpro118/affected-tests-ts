//! Typed progress events shared by tracing and renderers.

use crate::roots;

/// Pipeline phase associated with a progress event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// File discovery is running.
    Discovering,
    /// Graph construction is running.
    BuildingGraph,
    /// Changed-file tracing is running.
    Tracing,
    /// Output rendering is running.
    Rendering,
}

/// Progress event emitted by pure logic and consumed by renderers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// A phase started.
    Started(Phase),
    /// A root-relative path started tracing.
    TraceStarted(roots::RootRelativePath),
    /// A root-relative path reused an existing trace.
    TraceReused(roots::RootRelativePath),
    /// A root-relative path completed tracing.
    TraceCompleted(roots::RootRelativePath),
    /// A phase completed.
    Finished(Phase),
}

/// Sink for progress events.
pub trait Sink {
    /// Receives a progress event from the pipeline.
    fn send(&self, event: Event);
}
