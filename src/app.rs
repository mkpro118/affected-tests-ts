//! Thin application orchestration boundary for CLI, Git, graph, and output adapters.

use crate::cli;
use crate::failure;

/// Runtime TTY classification used to choose output rendering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalMode {
    /// Standard output is interactive.
    Interactive,
    /// Standard output is redirected, piped, or otherwise non-interactive.
    NonInteractive,
}

/// Top-level application request after edge adapters are chosen.
pub struct Request {
    /// Parsed CLI arguments.
    pub args: cli::Args,
    /// Terminal classification for renderer selection.
    pub terminal_mode: TerminalMode,
}

/// Runs the CLI application from process arguments.
///
/// # Errors
///
/// Returns an error when any pipeline phase or renderer fails.
pub fn run() -> failure::Result<()> {
    unimplemented!()
}

/// Runs the CLI application from an explicit request.
///
/// # Errors
///
/// Returns an error when any pipeline phase or renderer fails.
pub fn run_with(_request: Request) -> failure::Result<()> {
    unimplemented!()
}
