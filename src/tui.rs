//! Rich interactive terminal dashboard contracts.

use std::fmt;
use std::io;
use std::rc;

use crossterm::cursor;
use crossterm::event;
use crossterm::execute;
use crossterm::terminal;
use ratatui::backend;
use ratatui::layout;
use ratatui::style;
use ratatui::text;
use ratatui::widgets;

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
    fn draw_frame(&mut self, frame: &Frame) -> failure::Result<()>;
}

/// Ratatui-backed terminal renderer.
pub struct RatatuiTerminal<B>
where
    B: backend::Backend,
{
    terminal: ratatui::Terminal<B>,
}

impl<B> RatatuiTerminal<B>
where
    B: backend::Backend,
{
    /// Creates a ratatui renderer from an initialized backend.
    ///
    /// # Errors
    ///
    /// Returns an error when ratatui cannot initialize terminal state.
    pub fn new(backend: B) -> failure::Result<Self> {
        ratatui::Terminal::new(backend)
            .map(|terminal| Self { terminal })
            .map_err(|error| output_backend_error(&error))
    }

    /// Returns the underlying ratatui terminal.
    #[must_use]
    pub const fn terminal(&self) -> &ratatui::Terminal<B> {
        &self.terminal
    }
}

impl<B> Terminal for RatatuiTerminal<B>
where
    B: backend::Backend,
{
    fn draw_frame(&mut self, frame: &Frame) -> failure::Result<()> {
        self.terminal
            .draw(|terminal_frame| draw_dashboard(terminal_frame, frame))
            .map(|_completed_frame| ())
            .map_err(|error| output_backend_error(&error))
    }
}

/// Crossterm terminal session with raw-mode and alternate-screen lifecycle.
pub struct CrosstermSession<W>
where
    W: io::Write,
{
    terminal: RatatuiTerminal<backend::CrosstermBackend<W>>,
    is_active: bool,
}

impl<W> CrosstermSession<W>
where
    W: io::Write,
{
    /// Enters crossterm raw mode and creates a ratatui terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal setup fails.
    pub fn enter(writer: W) -> failure::Result<Self> {
        terminal::enable_raw_mode().map_err(|error| output_error(&error))?;
        let mut setup_guard = SetupGuard::new();
        setup_guard.state.mark_raw_mode_enabled();

        let backend = backend::CrosstermBackend::new(writer);
        let mut terminal = match RatatuiTerminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                return Err(setup_error_with_cleanup(
                    error,
                    setup_guard.cleanup::<W>(None),
                ));
            }
        };
        setup_guard.state.mark_screen_controls_may_be_enabled();
        if let Err(error) = enter_screen_controls(terminal.terminal.backend_mut()) {
            return Err(setup_error_with_cleanup(
                error,
                setup_guard.cleanup(Some(&mut terminal)),
            ));
        }
        setup_guard.disarm();
        Ok(Self {
            terminal,
            is_active: true,
        })
    }

    /// Returns the ratatui renderer for drawing.
    pub const fn renderer(&mut self) -> &mut RatatuiTerminal<backend::CrosstermBackend<W>> {
        &mut self.terminal
    }

    /// Leaves raw mode and restores the terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal teardown fails.
    pub fn exit(&mut self) -> failure::Result<()> {
        if !self.is_active {
            return Ok(());
        }

        let mut setup_guard = SetupGuard::new();
        setup_guard.state.mark_raw_mode_enabled();
        setup_guard.state.mark_screen_controls_may_be_enabled();
        let cleanup_result = setup_guard.cleanup(Some(&mut self.terminal));
        self.is_active = false;
        cleanup_result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SetupState {
    raw_mode_enabled: bool,
    screen_controls_may_be_enabled: bool,
}

impl SetupState {
    const fn new() -> Self {
        Self {
            raw_mode_enabled: false,
            screen_controls_may_be_enabled: false,
        }
    }

    const fn mark_raw_mode_enabled(&mut self) {
        self.raw_mode_enabled = true;
    }

    const fn mark_screen_controls_may_be_enabled(&mut self) {
        self.screen_controls_may_be_enabled = true;
    }

    const fn needs_raw_mode_cleanup(self) -> bool {
        self.raw_mode_enabled
    }

    const fn needs_screen_cleanup(self) -> bool {
        self.screen_controls_may_be_enabled
    }

    const fn clear(&mut self) {
        self.raw_mode_enabled = false;
        self.screen_controls_may_be_enabled = false;
    }
}

struct SetupGuard {
    state: SetupState,
}

impl SetupGuard {
    const fn new() -> Self {
        Self {
            state: SetupState::new(),
        }
    }

    const fn disarm(&mut self) {
        self.state.clear();
    }

    fn cleanup<W>(
        &mut self,
        terminal: Option<&mut RatatuiTerminal<backend::CrosstermBackend<W>>>,
    ) -> failure::Result<()>
    where
        W: io::Write,
    {
        let screen_result = cleanup_screen_controls(CleanupScreenControlsRequest {
            terminal,
            state: self.state,
        });
        let raw_result = cleanup_raw_mode(self.state);
        self.state.clear();
        screen_result?;
        raw_result
    }
}

struct CleanupScreenControlsRequest<'a, W>
where
    W: io::Write,
{
    terminal: Option<&'a mut RatatuiTerminal<backend::CrosstermBackend<W>>>,
    state: SetupState,
}

fn cleanup_screen_controls<W>(request: CleanupScreenControlsRequest<'_, W>) -> failure::Result<()>
where
    W: io::Write,
{
    if !request.state.needs_screen_cleanup() {
        return Ok(());
    }

    let Some(terminal) = request.terminal else {
        return Ok(());
    };

    exit_screen_controls(terminal.terminal.backend_mut())
}

fn cleanup_raw_mode(state: SetupState) -> failure::Result<()> {
    if state.needs_raw_mode_cleanup() {
        return terminal::disable_raw_mode().map_err(|error| output_error(&error));
    }

    Ok(())
}

fn setup_error_with_cleanup(
    setup_error: failure::AppError,
    cleanup_result: failure::Result<()>,
) -> failure::AppError {
    match cleanup_result {
        Ok(()) => setup_error,
        Err(cleanup_error) => output_message(
            format!("{setup_error}; terminal cleanup also failed: {cleanup_error}")
                .into_boxed_str(),
        ),
    }
}

fn enter_screen_controls<W>(backend: &mut backend::CrosstermBackend<W>) -> failure::Result<()>
where
    W: io::Write,
{
    execute!(
        backend,
        terminal::EnterAlternateScreen,
        event::EnableMouseCapture,
        cursor::Hide
    )
    .map_err(|error| output_error(&error))
}

fn exit_screen_controls<W>(backend: &mut backend::CrosstermBackend<W>) -> failure::Result<()>
where
    W: io::Write,
{
    execute!(
        backend,
        terminal::LeaveAlternateScreen,
        event::DisableMouseCapture,
        cursor::Show
    )
    .map_err(|error| output_error(&error))
}

impl<W> Drop for CrosstermSession<W>
where
    W: io::Write,
{
    fn drop(&mut self) {
        let _exit_result = self.exit();
    }
}

/// Snapshot model rendered by the terminal dashboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    /// Dashboard layout contract.
    pub layout: TuiLayout,
    /// Header and status band state.
    pub header_status: HeaderStatus,
    /// Current pipeline phase.
    pub phase: progress::Phase,
    /// Stable list of active trace paths.
    pub active_traces: Box<[Box<str>]>,
    /// Shared work and reuse state shown in the reuse pane.
    pub shared_work: SharedWork,
    /// Optional final result for summary panes.
    pub result: Option<contract::CommandResult>,
}

impl Model {
    /// Returns whether the dashboard should display the fail-closed overlay.
    #[must_use]
    pub const fn shows_fail_closed_overlay(&self) -> bool {
        matches!(&self.result, Some(contract::CommandResult::Full(_)))
    }
}

/// Typed status band data shown at the top of the dashboard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeaderStatus {
    /// Current selection mode.
    pub mode: RunMode,
    /// Git base revision label.
    pub base: Box<str>,
    /// Git head revision label.
    pub head: Box<str>,
    /// Number of changed files considered by the run.
    pub changed_file_count: usize,
    /// Number of tests selected by the run.
    pub selected_test_count: usize,
    /// Already formatted elapsed runtime.
    pub elapsed: Box<str>,
    /// Whether the full suite is required.
    pub full_run_status: FullRunStatus,
}

/// Shared trace work state shown in the reuse/collapse pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedWork {
    /// Trace nodes completed and available for reuse.
    pub completed_nodes: Box<[CompletedNode]>,
    /// Nodes currently owned by active traces.
    pub in_flight_nodes: Box<[InFlightNode]>,
    /// Total number of completed trace reuses.
    pub reuse_count: usize,
    /// Deterministic collapse points where duplicate work was avoided.
    pub collapse_points: Box<[CollapsePoint]>,
}

/// Completed trace node summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedNode {
    /// Completed graph node path.
    pub path: Box<str>,
    /// Number of selected tests reached through this node.
    pub test_count: usize,
}

/// In-flight trace ownership and waiters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InFlightNode {
    /// Node currently owned by a worker.
    pub owner: Box<str>,
    /// Trace roots waiting on the owner.
    pub waiters: Box<[Box<str>]>,
}

/// Collapsed reverse-trace path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollapsePoint {
    /// Path where duplicate tracing collapsed.
    pub path: Box<str>,
    /// Trace roots that reused this collapse point.
    pub reused_by: Box<[Box<str>]>,
}

/// Selection mode displayed by the rich dashboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    /// Selection is still running.
    Running,
    /// A partial test set is selected.
    Partial,
    /// The full suite is required.
    Full,
    /// No tests are affected.
    None,
    /// The command failed.
    Error,
}

impl RunMode {
    /// Returns the stable display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Partial => "partial",
            Self::Full => "full",
            Self::None => "none",
            Self::Error => "error",
        }
    }
}

/// Full-run status displayed by the rich dashboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullRunStatus {
    /// The full suite is not required.
    NotRequired,
    /// The full-suite decision is still pending.
    Pending,
    /// The full suite is required.
    Required,
}

impl FullRunStatus {
    /// Returns the stable display label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotRequired => "not-required",
            Self::Pending => "pending",
            Self::Required => "required",
        }
    }
}

/// Fully rendered rich dashboard frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    /// Header and status band for the current run.
    pub header: Pane,
    /// Left rail showing deterministic phase state.
    pub phase_rail: Pane,
    /// Center workspace with active trace roots and paths.
    pub trace_workspace: Pane,
    /// Right pane with reused and collapsed shared work.
    pub reuse_pane: Pane,
    /// Bottom pane with selected tests and reason chains.
    pub summary_pane: Pane,
    /// Optional fail-closed overlay shown for full-suite decisions.
    pub fail_closed_overlay: Option<Overlay>,
}

/// Rendered TUI pane content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pane {
    /// Stable pane identifier.
    pub id: Box<str>,
    /// Human-readable pane title.
    pub title: Box<str>,
    /// Deterministic text lines rendered inside the pane.
    pub lines: Box<[Box<str>]>,
}

/// Rendered overlay content for high-priority dashboard states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Overlay {
    /// Human-readable overlay title.
    pub title: Box<str>,
    /// Deterministic overlay body lines.
    pub lines: Box<[Box<str>]>,
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
pub fn render<T>(request: Request<T>) -> failure::Result<()>
where
    T: Terminal,
{
    let Request {
        mut terminal,
        model,
    } = request;
    let frame = frame_from_model(&model);

    terminal.draw_frame(&frame)
}

fn output_error(error: &io::Error) -> failure::AppError {
    output_message(format!("terminal rendering failed: {error}").into_boxed_str())
}

fn output_backend_error<E>(error: &E) -> failure::AppError
where
    E: fmt::Display,
{
    output_message(format!("terminal rendering failed: {error}").into_boxed_str())
}

const fn output_message(message: Box<str>) -> failure::AppError {
    failure::AppError::Output { message }
}

fn draw_dashboard(terminal_frame: &mut ratatui::Frame<'_>, frame: &Frame) {
    let dashboard_areas = dashboard_areas(terminal_frame.area());
    render_pane(PaneRenderRequest {
        terminal_frame,
        area: first_area(dashboard_areas.as_ref()),
        pane: &frame.header,
    });
    render_middle_row(MiddleRowRenderRequest {
        terminal_frame,
        area: area_at(dashboard_areas.as_ref(), 1),
        frame,
    });
    render_pane(PaneRenderRequest {
        terminal_frame,
        area: area_at(dashboard_areas.as_ref(), 2),
        pane: &frame.summary_pane,
    });
    if let Some(overlay) = &frame.fail_closed_overlay {
        let overlay_area = centered_rect(terminal_frame.area());
        render_overlay(OverlayRenderRequest {
            terminal_frame,
            area: overlay_area,
            overlay,
        });
    }
}

fn dashboard_areas(area: layout::Rect) -> rc::Rc<[layout::Rect]> {
    layout::Layout::default()
        .direction(layout::Direction::Vertical)
        .constraints([
            layout::Constraint::Length(5),
            layout::Constraint::Min(8),
            layout::Constraint::Length(7),
        ])
        .split(area)
}

struct MiddleRowRenderRequest<'a, 'b> {
    terminal_frame: &'a mut ratatui::Frame<'b>,
    area: layout::Rect,
    frame: &'a Frame,
}

fn render_middle_row(request: MiddleRowRenderRequest<'_, '_>) {
    let MiddleRowRenderRequest {
        terminal_frame,
        area,
        frame,
    } = request;
    let middle_areas = layout::Layout::default()
        .direction(layout::Direction::Horizontal)
        .constraints([
            layout::Constraint::Percentage(25),
            layout::Constraint::Percentage(45),
            layout::Constraint::Percentage(30),
        ])
        .split(area);
    render_pane(PaneRenderRequest {
        terminal_frame: &mut *terminal_frame,
        area: first_area(middle_areas.as_ref()),
        pane: &frame.phase_rail,
    });
    render_pane(PaneRenderRequest {
        terminal_frame: &mut *terminal_frame,
        area: area_at(middle_areas.as_ref(), 1),
        pane: &frame.trace_workspace,
    });
    render_pane(PaneRenderRequest {
        terminal_frame: &mut *terminal_frame,
        area: area_at(middle_areas.as_ref(), 2),
        pane: &frame.reuse_pane,
    });
}

struct PaneRenderRequest<'a, 'b> {
    terminal_frame: &'a mut ratatui::Frame<'b>,
    area: layout::Rect,
    pane: &'a Pane,
}

fn render_pane(request: PaneRenderRequest<'_, '_>) {
    let PaneRenderRequest {
        terminal_frame,
        area,
        pane,
    } = request;
    let block = widgets::Block::bordered().title(pane.title.as_ref());
    let text = lines_to_text(pane.lines.as_ref());
    let paragraph = widgets::Paragraph::new(text)
        .block(block)
        .wrap(widgets::Wrap { trim: false });
    terminal_frame.render_widget(paragraph, area);
}

struct OverlayRenderRequest<'a, 'b> {
    terminal_frame: &'a mut ratatui::Frame<'b>,
    area: layout::Rect,
    overlay: &'a Overlay,
}

fn render_overlay(request: OverlayRenderRequest<'_, '_>) {
    let OverlayRenderRequest {
        terminal_frame,
        area,
        overlay,
    } = request;
    let block = widgets::Block::bordered()
        .title(overlay.title.as_ref())
        .style(style::Style::default().fg(style::Color::Red));
    let paragraph = widgets::Paragraph::new(lines_to_text(overlay.lines.as_ref()))
        .block(block)
        .wrap(widgets::Wrap { trim: false });
    terminal_frame.render_widget(widgets::Clear, area);
    terminal_frame.render_widget(paragraph, area);
}

fn lines_to_text(lines: &[Box<str>]) -> text::Text<'static> {
    let rendered_lines = lines
        .iter()
        .map(|line| text::Line::from(line.to_string()))
        .collect::<Vec<_>>();

    text::Text::from(rendered_lines)
}

fn first_area(areas: &[layout::Rect]) -> layout::Rect {
    areas.first().copied().unwrap_or_default()
}

fn area_at(areas: &[layout::Rect], area_index: usize) -> layout::Rect {
    areas.get(area_index).copied().unwrap_or_default()
}

fn centered_rect(area: layout::Rect) -> layout::Rect {
    let vertical_areas = layout::Layout::default()
        .direction(layout::Direction::Vertical)
        .constraints([
            layout::Constraint::Percentage(15),
            layout::Constraint::Percentage(70),
            layout::Constraint::Percentage(15),
        ])
        .split(area);
    let horizontal_areas = layout::Layout::default()
        .direction(layout::Direction::Horizontal)
        .constraints([
            layout::Constraint::Percentage(10),
            layout::Constraint::Percentage(80),
            layout::Constraint::Percentage(10),
        ])
        .split(area_at(vertical_areas.as_ref(), 1));

    area_at(horizontal_areas.as_ref(), 1)
}

/// Builds the testable rich dashboard frame from typed state.
#[must_use]
pub fn frame_from_model(model: &Model) -> Frame {
    Frame {
        header: header_pane(model),
        phase_rail: phase_rail_pane(model),
        trace_workspace: trace_workspace_pane(model),
        reuse_pane: reuse_pane(model),
        summary_pane: summary_pane(model),
        fail_closed_overlay: fail_closed_overlay(model),
    }
}

struct PaneRequest<'a> {
    layout: &'a TuiPanel,
    lines: Box<[Box<str>]>,
}

fn pane(request: PaneRequest<'_>) -> Pane {
    Pane {
        id: request.layout.id.clone(),
        title: request.layout.title.clone(),
        lines: request.lines,
    }
}

fn header_pane(model: &Model) -> Pane {
    let status = &model.header_status;
    pane(PaneRequest {
        layout: &model.layout.header,
        lines: Box::from([
            format!("mode {}", status.mode.label()).into_boxed_str(),
            format!("base {}", status.base).into_boxed_str(),
            format!("head {}", status.head).into_boxed_str(),
            format!("changed {}", status.changed_file_count).into_boxed_str(),
            format!("selected {}", status.selected_test_count).into_boxed_str(),
            format!("elapsed {}", status.elapsed).into_boxed_str(),
            format!("full-run {}", status.full_run_status.label()).into_boxed_str(),
            format!("phase {}", model.phase.label()).into_boxed_str(),
        ]),
    })
}

fn phase_rail_pane(model: &Model) -> Pane {
    pane(PaneRequest {
        layout: &model.layout.phase_rail,
        lines: all_phases()
            .iter()
            .map(|phase| phase_line(*phase, model.phase))
            .collect(),
    })
}

fn trace_workspace_pane(model: &Model) -> Pane {
    pane(PaneRequest {
        layout: &model.layout.trace_workspace,
        lines: trace_lines(model.active_traces.as_ref(), "no active traces"),
    })
}

fn reuse_pane(model: &Model) -> Pane {
    pane(PaneRequest {
        layout: &model.layout.reuse_pane,
        lines: reuse_lines(&model.shared_work),
    })
}

fn summary_pane(model: &Model) -> Pane {
    pane(PaneRequest {
        layout: &model.layout.summary_pane,
        lines: summary_lines(model),
    })
}

fn fail_closed_overlay(model: &Model) -> Option<Overlay> {
    let Some(contract::CommandResult::Full(full)) = &model.result else {
        return None;
    };

    Some(Overlay {
        title: Box::<str>::from("Fail Closed"),
        lines: Box::from([
            format!("reason: {}", full.reason).into_boxed_str(),
            Box::<str>::from("Completed Before Stop"),
            Box::<str>::from("CI action: run full bun test suite"),
        ]),
    })
}

fn all_phases() -> Box<[progress::Phase]> {
    Box::from([
        progress::Phase::Discovering,
        progress::Phase::Parsing,
        progress::Phase::Resolving,
        progress::Phase::BuildingGraph,
        progress::Phase::Tracing,
        progress::Phase::Rendering,
    ])
}

fn phase_line(phase: progress::Phase, active_phase: progress::Phase) -> Box<str> {
    let state = match phase.order().cmp(&active_phase.order()) {
        std::cmp::Ordering::Less => "complete",
        std::cmp::Ordering::Equal => "active",
        std::cmp::Ordering::Greater => "pending",
    };

    format!("{state} {}", phase.label()).into_boxed_str()
}

fn trace_lines(paths: &[Box<str>], empty_message: &str) -> Box<[Box<str>]> {
    if paths.is_empty() {
        return Box::from([Box::<str>::from(empty_message)]);
    }

    paths
        .iter()
        .map(|path| format!("active {path}").into_boxed_str())
        .collect()
}

fn reuse_lines(shared_work: &SharedWork) -> Box<[Box<str>]> {
    let mut lines = Vec::<Box<str>>::new();
    lines.push(format!("reuse-count {}", shared_work.reuse_count).into_boxed_str());
    for node in &shared_work.completed_nodes {
        lines.push(format!("complete {} tests {}", node.path, node.test_count).into_boxed_str());
    }
    for node in &shared_work.in_flight_nodes {
        lines.push(in_flight_line(node));
    }
    for collapse_point in &shared_work.collapse_points {
        lines.push(collapse_line(collapse_point));
    }

    if lines.len() == 1 && shared_work.reuse_count == 0 {
        return Box::from([Box::<str>::from("no shared work")]);
    }

    lines.into_boxed_slice()
}

fn in_flight_line(node: &InFlightNode) -> Box<str> {
    format!(
        "in-flight {} waiters {}",
        node.owner,
        join_paths(&node.waiters)
    )
    .into_boxed_str()
}

fn collapse_line(collapse_point: &CollapsePoint) -> Box<str> {
    format!(
        "collapse {} reused-by {}",
        collapse_point.path,
        join_paths(&collapse_point.reused_by)
    )
    .into_boxed_str()
}

fn summary_lines(model: &Model) -> Box<[Box<str>]> {
    match &model.result {
        Some(contract::CommandResult::Partial(partial)) => partial_summary_lines(partial),
        Some(contract::CommandResult::Full(full)) => Box::from([
            Box::<str>::from("full run required"),
            format!("reason {}", full.reason).into_boxed_str(),
        ]),
        Some(contract::CommandResult::None(none)) => none_summary_lines(none),
        Some(contract::CommandResult::Error(error)) => {
            Box::from([format!("error {} {}", error.code, error.message).into_boxed_str()])
        }
        None => Box::from([Box::<str>::from("selection pending")]),
    }
}

fn partial_summary_lines(partial: &contract::PartialResult) -> Box<[Box<str>]> {
    let mut lines = Vec::<Box<str>>::new();
    for test in &partial.tests {
        lines.push(format!("test {test}").into_boxed_str());
    }
    for reason in &partial.reasons {
        lines.push(reason_line(reason));
    }

    if lines.is_empty() {
        Box::from([Box::<str>::from("partial with no selected tests")])
    } else {
        lines.into_boxed_slice()
    }
}

fn none_summary_lines(none: &contract::NoneResult) -> Box<[Box<str>]> {
    if none.changed_files.is_empty() {
        return Box::from([Box::<str>::from("none: no changed files")]);
    }

    none.changed_files
        .iter()
        .map(|path| format!("unchanged test set after {path}").into_boxed_str())
        .collect()
}

fn reason_line(reason: &contract::ReasonChain) -> Box<str> {
    format!(
        "reason {} -> {} via {}",
        reason.changed_file,
        reason.test_file,
        join_paths(reason.path.as_ref())
    )
    .into_boxed_str()
}

fn join_paths(paths: &[Box<str>]) -> Box<str> {
    if paths.is_empty() {
        return Box::<str>::from("");
    }

    let mut content = String::new();
    for path in paths {
        if !content.is_empty() {
            content.push_str(" -> ");
        }
        content.push_str(path);
    }

    content.into_boxed_str()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::contract;
    use crate::failure;
    use crate::progress;
    use ratatui::backend;

    #[derive(Clone, Debug, Default)]
    struct RecordingTerminal {
        frames: Arc<Mutex<Vec<super::Frame>>>,
    }

    impl super::Terminal for RecordingTerminal {
        fn draw_frame(&mut self, frame: &super::Frame) -> failure::Result<()> {
            self.frames.lock().unwrap().push(frame.clone());

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
            header: panel("header", "affected-tests-ts"),
            phase_rail: panel("phases", "Phases"),
            trace_workspace: panel("active-traces", "Active Traces"),
            reuse_pane: panel("reuse", "Shared Work"),
            summary_pane: panel("selected-tests", "Selected Tests"),
        }
    }

    fn partial_header_status() -> super::HeaderStatus {
        super::HeaderStatus {
            mode: super::RunMode::Partial,
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
            changed_file_count: 2,
            selected_test_count: 1,
            elapsed: Box::<str>::from("2.1s"),
            full_run_status: super::FullRunStatus::NotRequired,
        }
    }

    fn full_header_status() -> super::HeaderStatus {
        super::HeaderStatus {
            mode: super::RunMode::Full,
            base: Box::<str>::from("origin/main"),
            head: Box::<str>::from("HEAD"),
            changed_file_count: 4,
            selected_test_count: 0,
            elapsed: Box::<str>::from("19ms"),
            full_run_status: super::FullRunStatus::Required,
        }
    }

    fn shared_work() -> super::SharedWork {
        super::SharedWork {
            completed_nodes: Box::from([super::CompletedNode {
                path: Box::<str>::from("src/file-a.ts"),
                test_count: 1,
            }]),
            in_flight_nodes: Box::from([super::InFlightNode {
                owner: Box::<str>::from("src/file-b.ts"),
                waiters: Box::from([Box::<str>::from("src/file-d.ts")]),
            }]),
            reuse_count: 2,
            collapse_points: Box::from([super::CollapsePoint {
                path: Box::<str>::from("src/file-a.ts"),
                reused_by: Box::from([Box::<str>::from("src/file-d.ts")]),
            }]),
        }
    }

    fn rendered_text(frame: &super::Frame) -> String {
        let mut content = String::new();
        append_pane(&mut content, &frame.header);
        append_pane(&mut content, &frame.phase_rail);
        append_pane(&mut content, &frame.trace_workspace);
        append_pane(&mut content, &frame.reuse_pane);
        append_pane(&mut content, &frame.summary_pane);
        if let Some(overlay) = &frame.fail_closed_overlay {
            content.push_str(&overlay.title);
            content.push('\n');
            for line in &overlay.lines {
                content.push_str(line);
                content.push('\n');
            }
        }

        content
    }

    fn append_pane(content: &mut String, pane: &super::Pane) {
        content.push_str(&pane.title);
        content.push('\n');
        for line in &pane.lines {
            content.push_str(line);
            content.push('\n');
        }
    }

    fn rendered_backend_text(terminal: &super::RatatuiTerminal<backend::TestBackend>) -> String {
        terminal
            .terminal()
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>()
    }

    #[test]
    fn setup_state_keeps_partial_terminal_setup_cleanup_armed_until_success() {
        let mut state = super::SetupState::new();

        // Setup can fail after raw mode or after partial screen-control writes,
        // so cleanup stays armed until the session is fully constructed.
        assert!(!state.needs_raw_mode_cleanup());
        assert!(!state.needs_screen_cleanup());

        state.mark_raw_mode_enabled();
        assert!(state.needs_raw_mode_cleanup());
        assert!(!state.needs_screen_cleanup());

        state.mark_screen_controls_may_be_enabled();
        assert!(state.needs_raw_mode_cleanup());
        assert!(state.needs_screen_cleanup());

        state.clear();
        assert!(!state.needs_raw_mode_cleanup());
        assert!(!state.needs_screen_cleanup());
    }

    #[test]
    fn renders_dashboard_reuse_selected_tests_and_fail_closed_overlay_from_typed_events() {
        let terminal = RecordingTerminal::default();
        let model = super::Model {
            layout: layout(),
            header_status: partial_header_status(),
            phase: progress::Phase::Tracing,
            active_traces: Box::from([Box::<str>::from("src/file-d.ts")]),
            shared_work: shared_work(),
            result: Some(contract::CommandResult::Partial(contract::PartialResult {
                tests: Box::from([Box::<str>::from("src/file-a.test.ts")]),
                reasons: Box::from([contract::ReasonChain {
                    changed_file: Box::<str>::from("src/file-d.ts"),
                    test_file: Box::<str>::from("src/file-a.test.ts"),
                    path: Box::from([
                        Box::<str>::from("src/file-d.ts"),
                        Box::<str>::from("src/file-a.ts"),
                        Box::<str>::from("src/file-a.test.ts"),
                    ]),
                }]),
            })),
        };
        let request = super::Request {
            terminal: terminal.clone(),
            model,
        };

        // The TUI receives typed state so rendering can change without mutating
        // selection behavior or reinterpreting terminal escape output.
        super::render(request).unwrap();

        let frame = {
            let frames = terminal.frames.lock().unwrap();
            frames.first().unwrap().clone()
        };
        let content = rendered_text(&frame);

        assert!(content.contains("affected-tests-ts"));
        assert!(content.contains("mode partial"));
        assert!(content.contains("base origin/main"));
        assert!(content.contains("head HEAD"));
        assert!(content.contains("changed 2"));
        assert!(content.contains("selected 1"));
        assert!(content.contains("elapsed 2.1s"));
        assert!(content.contains("full-run not-required"));
        assert!(content.contains("Phases"));
        assert_eq!(frame.trace_workspace.title.as_ref(), "Active Traces");
        assert!(
            frame
                .trace_workspace
                .lines
                .iter()
                .any(|line| line.as_ref() == "active src/file-d.ts")
        );
        assert!(content.contains("Shared Work"));
        assert!(content.contains("Selected Tests"));
        assert!(content.contains("reuse-count 2"));
        assert!(content.contains("complete src/file-a.ts tests 1"));
        assert!(content.contains("in-flight src/file-b.ts waiters src/file-d.ts"));
        assert!(content.contains("collapse src/file-a.ts reused-by src/file-d.ts"));
        assert!(content.contains("test src/file-a.test.ts"));
        assert!(content.contains(
            "reason src/file-d.ts -> src/file-a.test.ts via src/file-d.ts -> src/file-a.ts -> src/file-a.test.ts"
        ));
    }

    #[test]
    fn ratatui_backend_renders_dashboard_widgets_from_typed_frame() {
        let model = super::Model {
            layout: layout(),
            header_status: partial_header_status(),
            phase: progress::Phase::Tracing,
            active_traces: Box::from([Box::<str>::from("src/file-d.ts")]),
            shared_work: shared_work(),
            result: Some(contract::CommandResult::Partial(contract::PartialResult {
                tests: Box::from([Box::<str>::from("src/file-a.test.ts")]),
                reasons: Box::from([]),
            })),
        };
        let frame = super::frame_from_model(&model);
        let mut terminal = super::RatatuiTerminal::new(backend::TestBackend::new(100, 32)).unwrap();

        // This exercises the ratatui backend path rather than only the abstract
        // frame contract used by orchestration tests.
        super::Terminal::draw_frame(&mut terminal, &frame).unwrap();
        let content = rendered_backend_text(&terminal);

        assert!(content.contains("affected-tests-ts"));
        assert!(content.contains("Active Traces"));
        assert!(content.contains("active src/file-d.ts"));
        assert!(content.contains("Shared Work"));
        assert!(content.contains("collapse src/file-a.ts"));
        assert!(content.contains("Selected Tests"));
        assert!(content.contains("test src/file-a.test.ts"));
    }

    #[test]
    fn renders_fail_closed_overlay_for_full_results() {
        let model = super::Model {
            layout: layout(),
            header_status: full_header_status(),
            phase: progress::Phase::Tracing,
            active_traces: Box::from([Box::<str>::from("src/file-d.ts")]),
            shared_work: shared_work(),
            result: Some(contract::CommandResult::Full(contract::FullResult {
                reason: Box::<str>::from("global invalidator changed: tsconfig.json"),
            })),
        };

        let frame = super::frame_from_model(&model);
        let content = rendered_text(&frame);

        assert!(model.shows_fail_closed_overlay());
        assert!(content.contains("mode full"));
        assert!(content.contains("full-run required"));
        assert!(content.contains("Fail Closed"));
        assert!(content.contains("reason: global invalidator changed: tsconfig.json"));
        assert!(content.contains("Completed Before Stop"));
        assert!(content.contains("CI action: run full bun test suite"));
    }
}
