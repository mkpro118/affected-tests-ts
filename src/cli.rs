//! Command-line argument contracts for selection, graph, and explain commands.

use clap::{Parser, Subcommand, ValueEnum};

/// Parsed top-level CLI arguments.
#[derive(Debug, Parser)]
#[command(name = "affected-tests-ts")]
pub struct Args {
    /// Optional subcommand; selection is the default when omitted.
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Output format for machine or human consumers.
    #[arg(long = "format", value_enum, default_value = "shell", global = true)]
    pub format: Format,
    /// Git base revision used for changed-file detection.
    #[arg(long)]
    pub base: Option<Box<str>>,
    /// Git head revision used for changed-file detection.
    #[arg(long)]
    pub head: Option<Box<str>>,
    /// Include explain reason chains in formats that support them.
    #[arg(long)]
    pub explain: bool,
}

/// CLI subcommands beyond default affected-test selection.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the dependency graph contract.
    Graph,
    /// Explain why a selected test is affected.
    Explain {
        /// Root-relative test path to explain.
        test: Box<str>,
    },
}

/// Output formats accepted by the CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
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
