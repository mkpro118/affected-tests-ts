//! Typed progress events shared by tracing and renderers.

use crate::roots;
use crate::work;

/// Pipeline phase associated with a progress event.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    /// File discovery is running.
    Discovering,
    /// Import parsing is running.
    Parsing,
    /// Module resolution is running.
    Resolving,
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
    /// A phase completed with deterministic detail text.
    Completed(Step),
    /// A root-relative path started tracing.
    TraceStarted(roots::RootRelativePath),
    /// A root-relative path joined an existing in-flight trace.
    TraceJoined(roots::RootRelativePath),
    /// A root-relative path reused an existing trace.
    TraceReused(roots::RootRelativePath),
    /// A root-relative path completed tracing.
    TraceCompleted(roots::RootRelativePath),
    /// A root-relative path failed tracing.
    TraceFailed(roots::RootRelativePath),
    /// A cycle edge was recorded without blocking progress.
    TraceCycle(Cycle),
    /// A phase completed.
    Finished(Phase),
}

/// Completed phase detail safe to print in non-interactive logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    /// Completed pipeline phase.
    pub phase: Phase,
    /// Deterministic human-readable completion detail.
    pub detail: Box<str>,
    /// Optional elapsed time already formatted by the caller.
    pub elapsed: Option<Box<str>>,
}

/// Deterministic cycle edge observed during tracing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cycle {
    /// Source path in the cycle edge.
    pub from: roots::RootRelativePath,
    /// Target path in the cycle edge.
    pub to: roots::RootRelativePath,
}

impl Phase {
    /// Returns the stable Docker-style label for this phase.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Discovering => "discover",
            Self::Parsing => "parse",
            Self::Resolving => "resolve",
            Self::BuildingGraph => "graph",
            Self::Tracing => "trace",
            Self::Rendering => "output",
        }
    }

    /// Returns the stable rail position used by deterministic renderers.
    #[must_use]
    pub const fn order(self) -> usize {
        match self {
            Self::Discovering => 1,
            Self::Parsing => 2,
            Self::Resolving => 3,
            Self::BuildingGraph => 4,
            Self::Tracing => 5,
            Self::Rendering => 6,
        }
    }
}

impl From<work::CycleEdge> for Cycle {
    fn from(edge: work::CycleEdge) -> Self {
        Self {
            from: edge.from,
            to: edge.to,
        }
    }
}

impl From<work::TraceEvent> for Event {
    fn from(event: work::TraceEvent) -> Self {
        match event {
            work::TraceEvent::Scheduled(path) => Self::TraceStarted(path),
            work::TraceEvent::JoinedInFlight(path) => Self::TraceJoined(path),
            work::TraceEvent::Reused(path) => Self::TraceReused(path),
            work::TraceEvent::Completed(path) => Self::TraceCompleted(path),
            work::TraceEvent::Failed(path) => Self::TraceFailed(path),
            work::TraceEvent::Cycle(edge) => Self::TraceCycle(Cycle::from(edge)),
        }
    }
}

/// Sink for progress events.
pub trait Sink {
    /// Receives a progress event from the pipeline.
    fn send(&self, event: Event);
}

#[cfg(test)]
mod tests {
    use crate::roots;
    use crate::work;

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn trace_events_convert_to_renderer_progress_without_terminal_text() {
        let event = super::Event::from(work::TraceEvent::JoinedInFlight(path("src/file-a.ts")));
        let cycle = super::Event::from(work::TraceEvent::Cycle(work::CycleEdge {
            from: path("src/file-b.ts"),
            to: path("src/file-a.ts"),
        }));

        // Renderers receive scheduler facts, not pre-rendered spinner text, so
        // TTY and CI output can diverge without changing trace behavior.
        assert_eq!(event, super::Event::TraceJoined(path("src/file-a.ts")));
        assert_eq!(
            cycle,
            super::Event::TraceCycle(super::Cycle {
                from: path("src/file-b.ts"),
                to: path("src/file-a.ts"),
            })
        );
    }
}
