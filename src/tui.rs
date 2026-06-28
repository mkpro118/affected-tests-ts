//! Rich interactive terminal dashboard contracts.

use crate::contract;
use crate::failure;
use crate::progress;

/// Layout contract for the interactive dashboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiLayout {
    /// Header panel shown above the trace workspace.
    pub header: TuiPanel,
    /// Left rail panel for pipeline phases.
    pub phase_rail: TuiPanel,
    /// Center panel for active trace work.
    pub trace_workspace: TuiPanel,
    /// Right panel for shared trace reuse.
    pub reuse_pane: TuiPanel,
    /// Bottom panel for selected tests and explanations.
    pub summary_pane: TuiPanel,
}

/// Panel contract used by the TUI layout model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiPanel {
    /// Stable panel identifier for tests and renderers.
    pub id: Box<str>,
    /// Human-readable panel title.
    pub title: Box<str>,
}

/// Terminal backend capability consumed by the TUI renderer.
pub trait Terminal {
    /// Draws a dashboard frame from typed progress and result state.
    ///
    /// # Errors
    ///
    /// Returns an error when the terminal backend cannot draw.
    fn draw_frame(&mut self, model: &Model) -> failure::Result<()>;
}

/// Snapshot model rendered by the terminal dashboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    /// Dashboard layout contract.
    pub layout: TuiLayout,
    /// Current pipeline phase.
    pub phase: progress::Phase,
    /// Stable list of active trace paths.
    pub active_traces: Box<[Box<str>]>,
    /// Stable list of reused trace paths.
    pub reused_traces: Box<[Box<str>]>,
    /// Optional final result for summary panes.
    pub result: Option<contract::CommandResult>,
}

/// Request object for interactive dashboard rendering.
pub struct Request<T> {
    /// Terminal backend selected by the application edge.
    pub terminal: T,
    /// Initial dashboard model.
    pub model: Model,
}

/// Runs the interactive terminal dashboard.
///
/// # Errors
///
/// Returns an error when terminal setup, drawing, or teardown fails.
pub fn render<T>(_request: Request<T>) -> failure::Result<()>
where
    T: Terminal,
{
    unimplemented!()
}
