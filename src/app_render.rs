//! Application rendering edge for stdout, Docker logs, and TUI dispatch.
//! Explicit TUI output falls back to plain text when stdout is not interactive.

use std::io::{self, Write};

use crate::app;
use crate::cli;
use crate::contract;
use crate::dashboard;
use crate::failure;
use crate::logs;
use crate::presentation;
use crate::progress;

/// Request for rendering a completed command.
pub struct Command {
    /// Selected output format.
    pub format: presentation::Format,
    /// Command result to render.
    pub result: contract::CommandResult,
    /// Current terminal interactivity.
    pub terminal_mode: app::TerminalMode,
    /// Git base label shown in TUI status.
    pub base: Box<str>,
    /// Git head label shown in TUI status.
    pub head: Box<str>,
    /// Number of changed files considered.
    pub changed_file_count: usize,
}

#[derive(Clone, Copy, Debug)]
struct StdoutSink;

impl presentation::ResultRenderer for StdoutSink {
    fn write(&self, content: &str) -> failure::Result<()> {
        write_stdout(content)
    }
}

#[derive(Clone, Copy, Debug)]
struct DockerSink;

impl logs::StepOutputSink for DockerSink {
    fn write_line(&self, line: &str) -> failure::Result<()> {
        let mut content = String::from(line);
        content.push('\n');
        write_stdout(content.as_ref())
    }
}

/// Renders command output to stdout.
///
/// # Errors
///
/// Returns an error when serialization, terminal drawing, or stdout writing fails.
pub fn render(request: Command) -> failure::Result<()> {
    match request.format {
        presentation::Format::Shell | presentation::Format::Json | presentation::Format::Plain => {
            presentation::render(presentation::RenderRequest {
                sink: StdoutSink,
                format: request.format,
                result: request.result,
            })
        }
        presentation::Format::Docker => logs::render(logs::RenderRequest {
            sink: DockerSink,
            events: progress_events(&request.result, request.changed_file_count),
            result: request.result,
        }),
        presentation::Format::Tui => render_tui(request),
    }
}

/// Maps CLI format values to presentation format values.
#[must_use]
pub const fn format(format: cli::Format) -> presentation::Format {
    match format {
        cli::Format::Shell => presentation::Format::Shell,
        cli::Format::Json => presentation::Format::Json,
        cli::Format::Tui => presentation::Format::Tui,
        cli::Format::Docker => presentation::Format::Docker,
        cli::Format::Plain => presentation::Format::Plain,
    }
}

/// Writes a serializable value as one JSON line.
///
/// # Errors
///
/// Returns an error when serialization or stdout writing fails.
pub fn write_json<T>(value: &T) -> failure::Result<()>
where
    T: serde::Serialize,
{
    serde_json::to_string(value)
        .map(|json| {
            let mut content = json;
            content.push('\n');
            content
        })
        .map_err(|error| failure::AppError::Output {
            message: format!("failed to serialize JSON output: {error}").into_boxed_str(),
        })
        .and_then(|content| write_stdout(content.as_ref()))
}

/// Writes raw content to stdout.
///
/// # Errors
///
/// Returns an error when stdout writing fails.
pub fn write_stdout(content: &str) -> failure::Result<()> {
    io::stdout()
        .lock()
        .write_all(content.as_bytes())
        .map_err(|error| failure::AppError::Output {
            message: format!("failed to write stdout: {error}").into_boxed_str(),
        })
}

fn render_tui(request: Command) -> failure::Result<()> {
    if request.terminal_mode == app::TerminalMode::NonInteractive {
        return render_non_interactive_tui(request.result);
    }

    let selected_test_count = selected_test_count(&request.result);
    let model = dashboard::Model {
        layout: dashboard_layout(),
        header_status: dashboard::HeaderStatus {
            mode: run_mode(&request.result),
            base: request.base,
            head: request.head,
            changed_file_count: request.changed_file_count,
            selected_test_count,
            elapsed: Box::<str>::from("0ms"),
            full_run_status: full_run_status(&request.result),
        },
        phase: progress::Phase::Rendering,
        active_traces: Box::from([]),
        shared_work: dashboard::SharedWork {
            completed_nodes: Box::from([]),
            in_flight_nodes: Box::from([]),
            reuse_count: 0,
            collapse_points: Box::from([]),
        },
        result: Some(request.result),
    };
    let mut session = dashboard::CrosstermSession::enter(io::stdout())?;
    let frame = dashboard::frame_from_model(&model);

    dashboard::Terminal::draw_frame(session.renderer(), &frame)?;
    session.exit()
}

fn render_non_interactive_tui(result: contract::CommandResult) -> failure::Result<()> {
    presentation::render(presentation::RenderRequest {
        sink: StdoutSink,
        format: presentation::Format::Plain,
        result,
    })
}

fn progress_events(
    result: &contract::CommandResult,
    changed_file_count: usize,
) -> Box<[progress::Event]> {
    let mut events = vec![
        progress::Event::Completed(progress::Step {
            phase: progress::Phase::Discovering,
            detail: format!("detected changed files: {changed_file_count}").into_boxed_str(),
            elapsed: None,
        }),
        progress::Event::Completed(progress::Step {
            phase: progress::Phase::BuildingGraph,
            detail: Box::<str>::from("built dependency graph"),
            elapsed: None,
        }),
    ];
    if matches!(result, contract::CommandResult::Partial(_)) {
        events.push(progress::Event::Completed(progress::Step {
            phase: progress::Phase::Tracing,
            detail: Box::<str>::from("traced changed files"),
            elapsed: None,
        }));
    }

    events.into_boxed_slice()
}

fn selected_test_count(result: &contract::CommandResult) -> usize {
    match result {
        contract::CommandResult::Partial(partial) => partial.tests.len(),
        contract::CommandResult::Full(_)
        | contract::CommandResult::None(_)
        | contract::CommandResult::Error(_) => 0,
    }
}

const fn run_mode(result: &contract::CommandResult) -> dashboard::RunMode {
    match result {
        contract::CommandResult::Partial(_) => dashboard::RunMode::Partial,
        contract::CommandResult::Full(_) => dashboard::RunMode::Full,
        contract::CommandResult::None(_) => dashboard::RunMode::None,
        contract::CommandResult::Error(_) => dashboard::RunMode::Error,
    }
}

const fn full_run_status(result: &contract::CommandResult) -> dashboard::FullRunStatus {
    if matches!(result, contract::CommandResult::Full(_)) {
        dashboard::FullRunStatus::Required
    } else {
        dashboard::FullRunStatus::NotRequired
    }
}

fn dashboard_layout() -> dashboard::TuiLayout {
    dashboard::TuiLayout {
        header: panel("header", "affected-tests-ts"),
        phase_rail: panel("phases", "Phases"),
        trace_workspace: panel("active-traces", "Active Traces"),
        reuse_pane: panel("reuse", "Shared Work"),
        summary_pane: panel("selected-tests", "Selected Tests"),
    }
}

fn panel(id: &str, title: &str) -> dashboard::TuiPanel {
    dashboard::TuiPanel {
        id: Box::<str>::from(id),
        title: Box::<str>::from(title),
    }
}
