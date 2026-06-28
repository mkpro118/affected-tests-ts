//! Shell, JSON, and plain output rendering contracts.

use crate::contract;
use crate::failure;

/// Shell-renderer contract for newline-delimited selected tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellOutput {
    /// Stable sorted shell lines.
    pub lines: Box<[Box<str>]>,
}

/// JSON-renderer contract for machine-readable command results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonOutput {
    /// Strict command result payload.
    pub result: contract::CommandResult,
}

/// Plain graph debug contract for humans inspecting dependency edges.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphDebugOutput {
    /// Stable sorted graph nodes and edges.
    pub graph: contract::GraphResult,
    /// Stable sorted root-relative paths highlighted by the command.
    pub highlighted_paths: Box<[Box<str>]>,
}

/// Output format selected after CLI parsing and TTY detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    /// Newline-delimited shell output.
    Shell,
    /// Strict JSON contract output.
    Json,
    /// Interactive terminal dashboard.
    Tui,
    /// Static Docker-style step output.
    Docker,
    /// Plain human-readable text.
    Plain,
}

/// Result renderer capability used by output orchestration.
pub trait ResultRenderer {
    /// Writes rendered text.
    ///
    /// # Errors
    ///
    /// Returns an error when output cannot be written.
    fn write(&self, content: &str) -> failure::Result<()>;
}

/// Request object for static output rendering.
pub struct RenderRequest<S> {
    /// Selected output sink.
    pub sink: S,
    /// Output format to render.
    pub format: Format,
    /// Machine-readable command result.
    pub result: contract::CommandResult,
}

/// Renders shell, JSON, or plain command output.
///
/// # Errors
///
/// Returns an error when serialization or writing fails.
pub fn render<S>(_request: RenderRequest<S>) -> failure::Result<()>
where
    S: ResultRenderer,
{
    unimplemented!()
}
