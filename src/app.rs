//! Thin application entry and command dispatch for the affected-tests-ts binary.

use std::env;
use std::io::{self, IsTerminal};

use clap::Parser;

use crate::app_contract;
use crate::app_pipeline;
use crate::app_render;
use crate::cli;
use crate::failure;
use crate::impact;
use crate::package_scope;
use crate::presentation;
use crate::roots;
use crate::vcs;
use crate::vcs::ChangeSetView;

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

struct SelectionCommand {
    base: Box<str>,
    head: Box<str>,
    format: presentation::Format,
    include_reasons: bool,
    terminal_mode: TerminalMode,
}

#[derive(Clone, Copy, Debug)]
struct GraphCommand {
    format: presentation::Format,
}

struct ExplainCommand {
    format: presentation::Format,
    path: roots::RootRelativePath,
    terminal_mode: TerminalMode,
}

/// Runs the CLI application from process arguments.
///
/// # Errors
///
/// Returns an error when any pipeline phase or renderer fails.
pub fn run() -> failure::Result<()> {
    let terminal_mode = if io::stdout().is_terminal() {
        TerminalMode::Interactive
    } else {
        TerminalMode::NonInteractive
    };

    run_with(Request {
        args: cli::Args::parse(),
        terminal_mode,
    })
}

/// Runs the CLI application from an explicit request.
///
/// # Errors
///
/// Returns an error when any pipeline phase or renderer fails.
pub fn run_with(request: Request) -> failure::Result<()> {
    let Request {
        args,
        terminal_mode,
    } = request;
    match &args.command {
        Some(cli::Command::Graph) => run_graph(GraphCommand {
            format: app_render::format(args.format),
        }),
        Some(cli::Command::Explain { test }) => run_explain(ExplainCommand {
            format: app_render::format(args.format),
            path: roots::RootRelativePath::try_from(test.clone())?,
            terminal_mode,
        }),
        None => run_selection(SelectionCommand {
            base: args
                .base
                .clone()
                .unwrap_or_else(|| Box::<str>::from("origin/main")),
            head: args
                .head
                .clone()
                .unwrap_or_else(|| Box::<str>::from("HEAD")),
            format: app_render::format(args.format),
            include_reasons: args.explain,
            terminal_mode,
        }),
    }
}

fn run_selection(request: SelectionCommand) -> failure::Result<()> {
    let repository_path = repository_root()?;
    let changes = vcs::changed_files(vcs::ChangesRequest {
        repository: repository.clone(),
    let repository = vcs::ProcessRepository::for_root(repository_path.clone());
        base: request.base.clone(),
        head: request.head.clone(),
    })?;
    let pipeline = app_pipeline::build(repository_path)?;
    let classifier = app_pipeline::Classifier::try_new(&pipeline.config)?;
    let result = impact::select(impact::SelectionRequest {
        graph: pipeline.graph,
        classifier,
    let selection_changes = match &pipeline.graph {
        Ok(graph) => package_scope::scoped_changes(&package_scope::ScopeRequest {
            repository: &repository,
            base: request.base.as_ref(),
            head: request.head.as_ref(),
            changes: &changes,
            graph,
        })?,
        Err(_error) => changes,
    };
        always_run: app_pipeline::EmptyAlwaysRun::default(),
        changes: selection_changes.clone(),
    })?;
    let contract = app_contract::command_result(app_contract::SelectionOutputRequest {
        result,
        changes: &selection_changes,
        files: &pipeline.files,
        include_reasons: request.include_reasons,
    });

    app_render::render(app_render::Command {
        format: request.format,
        result: contract,
        terminal_mode: request.terminal_mode,
        base: request.base,
        head: request.head,
        changed_file_count: selection_changes.files().len(),
    })
}

fn run_graph(request: GraphCommand) -> failure::Result<()> {
    let pipeline = app_pipeline::build(repository_root()?)?;
    let graph = pipeline.output_graph?;
    let graph_result = app_contract::graph_result(app_contract::GraphRequest {
        graph: &graph,
        files: &pipeline.files,
    });
    match request.format {
        presentation::Format::Json => app_render::write_json(&graph_result),
        presentation::Format::Shell
        | presentation::Format::Plain
        | presentation::Format::Docker
        | presentation::Format::Tui => {
            app_render::write_stdout(app_contract::graph_plain(&graph_result).as_ref())
        }
    }
}

fn run_explain(request: ExplainCommand) -> failure::Result<()> {
    let pipeline = app_pipeline::build(repository_root()?)?;
    let changes = vcs::ChangeSet {
        files: Box::from([vcs::ChangedFile {
            status: vcs::ChangedFileStatus::Modified,
            path: request.path,
            previous_path: None,
        }]),
    };
    let classifier = app_pipeline::Classifier::try_new(&pipeline.config)?;
    let result = impact::select(impact::SelectionRequest {
        graph: pipeline.graph,
        classifier,
        always_run: app_pipeline::EmptyAlwaysRun::default(),
        changes: changes.clone(),
    })?;
    let contract = app_contract::command_result(app_contract::SelectionOutputRequest {
        result,
        changes: &changes,
        include_reasons: true,
    });

        files: &pipeline.files,
    app_render::render(app_render::Command {
        format: request.format,
        result: contract,
        terminal_mode: request.terminal_mode,
        base: Box::<str>::from("explain"),
        head: Box::<str>::from("workspace"),
        changed_file_count: changes.files().len(),
    })
}

fn repository_root() -> failure::Result<Box<str>> {
    env::current_dir()
        .map(|path| path.to_string_lossy().into_owned().into_boxed_str())
        .map_err(|error| failure::AppError::FileSystem {
            message: format!("failed to read current directory: {error}").into_boxed_str(),
        })
}
