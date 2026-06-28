//! Static Docker-style non-TTY progress and result rendering contracts.

use crate::contract;
use crate::failure;
use crate::progress;

/// Docker-style static output contract for non-interactive terminals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerOutput {
    /// Stable step lines emitted before the final result.
    pub steps: Box<[DockerStep]>,
    /// Final machine-readable command result.
    pub result: contract::CommandResult,
}

/// One static progress line in Docker-style output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerStep {
    /// Phase associated with the line.
    pub phase: progress::Phase,
    /// Human-readable step text.
    pub message: Box<str>,
}

/// Line-oriented step output sink for non-interactive progress output.
pub trait StepOutputSink {
    /// Writes one rendered line.
    ///
    /// # Errors
    ///
    /// Returns an error when the line cannot be written.
    fn write_line(&self, line: &str) -> failure::Result<()>;
}

/// Request object for Docker-style rendering.
pub struct RenderRequest<S> {
    /// Output sink used for rendered lines.
    pub sink: S,
    /// Progress events accumulated during the run.
    pub events: Box<[progress::Event]>,
    /// Final command result.
    pub result: contract::CommandResult,
}

/// Renders static progress and final result for non-TTY environments.
///
/// # Errors
///
/// Returns an error when rendering or writing fails.
pub fn render<S>(_request: RenderRequest<S>) -> failure::Result<()>
where
    S: StepOutputSink,
{
    unimplemented!()
}
