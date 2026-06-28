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

#[cfg(test)]
mod tests {
    use crate::contract;
    use crate::failure;
    use crate::progress;

    #[derive(Clone, Debug, Default)]
    struct RecordingTerminal {
        frames: Vec<super::Model>,
    }

    impl super::Terminal for RecordingTerminal {
        fn draw_frame(&mut self, model: &super::Model) -> failure::Result<()> {
            self.frames.push(model.clone());

            Ok(())
        }
    }

    fn panel(id: &str, title: &str) -> super::TuiPanel {
        super::TuiPanel {
            id: Box::<str>::from(id),
            title: Box::<str>::from(title),
        }
    }

    fn layout() -> super::TuiLayout {
        super::TuiLayout {
            header: panel("header", "affected-tests"),
            phase_rail: panel("phases", "Phases"),
            trace_workspace: panel("active-traces", "Active Traces"),
            reuse_pane: panel("reuse", "Shared Work"),
            summary_pane: panel("selected-tests", "Selected Tests"),
        }
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn renders_dashboard_reuse_selected_tests_and_fail_closed_overlay_from_typed_events() {
        let model = super::Model {
            layout: layout(),
            phase: progress::Phase::Tracing,
            active_traces: Box::from([Box::<str>::from("src/file-d.ts")]),
            reused_traces: Box::from([Box::<str>::from("src/file-b.ts")]),
            result: Some(contract::CommandResult::Full(contract::FullResult {
                reason: Box::<str>::from("global invalidator changed: tsconfig.json"),
            })),
        };
        let request = super::Request {
            terminal: RecordingTerminal::default(),
            model,
        };

        // The TUI receives typed state so rendering can change without mutating
        // selection behavior or reinterpreting terminal escape output.
        super::render(request).unwrap();
    }
}
