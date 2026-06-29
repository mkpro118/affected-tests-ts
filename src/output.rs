//! Shell, JSON, and plain output rendering contracts.

use crate::contract;
use crate::failure;

/// Shell-renderer contract for space-delimited selected tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellOutput {
    /// Stable sorted shell arguments.
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
    /// Space-delimited shell output.
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

/// Standard-output interactivity used for automatic human output selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamInteractivity {
    /// Standard output is connected to an interactive terminal.
    Interactive,
    /// Standard output is redirected, piped, or captured by CI.
    NonInteractive,
}

/// User-requested output format before automatic TTY selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatRequest {
    /// No explicit format was requested, so choose human output for the stream.
    Auto,
    /// The user or command selected an exact output format.
    Explicit(Format),
}

/// Request object for output-format selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionRequest {
    /// Requested output mode from CLI parsing or command defaults.
    pub requested: FormatRequest,
    /// Current stdout interactivity.
    pub stdout: StreamInteractivity,
}

/// Selects the concrete renderer without producing command output.
#[must_use]
pub const fn select_format(request: SelectionRequest) -> Format {
    match request.requested {
        FormatRequest::Explicit(format) => format,
        FormatRequest::Auto => match request.stdout {
            StreamInteractivity::Interactive => Format::Tui,
            StreamInteractivity::NonInteractive => Format::Docker,
        },
    }
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
pub fn render<S>(request: RenderRequest<S>) -> failure::Result<()>
where
    S: ResultRenderer,
{
    let RenderRequest {
        sink,
        format,
        result,
    } = request;
    let content = match format {
        Format::Shell => render_shell(&result),
        Format::Json => render_json(&result)?,
        Format::Plain => render_plain(&result),
        Format::Tui => {
            return Err(failure::AppError::Output {
                message: Box::<str>::from("tui output uses the dashboard renderer"),
            });
        }
        Format::Docker => {
            return Err(failure::AppError::Output {
                message: Box::<str>::from("docker output uses the step renderer"),
            });
        }
    };

    sink.write(content.as_ref())
}

fn render_shell(result: &contract::CommandResult) -> Box<str> {
    match result {
        contract::CommandResult::Partial(partial) => space_join(partial.tests.as_ref()),
        contract::CommandResult::Full(_full) => Box::<str>::from(""),
        contract::CommandResult::None(_none) => Box::<str>::from(""),
        contract::CommandResult::Error(_error) => Box::<str>::from(""),
    }
}

fn render_json(result: &contract::CommandResult) -> failure::Result<Box<str>> {
    serde_json::to_string(result)
        .map(|json| format!("{json}\n").into_boxed_str())
        .map_err(|error| failure::AppError::Output {
            message: format!("failed to serialize JSON output: {error}").into_boxed_str(),
        })
}

fn render_plain(result: &contract::CommandResult) -> Box<str> {
    match result {
        contract::CommandResult::Partial(partial) => render_plain_partial(partial),
        contract::CommandResult::Full(full) => format!("full: {}\n", full.reason).into_boxed_str(),
        contract::CommandResult::None(none) => render_plain_none(none),
        contract::CommandResult::Error(error) => {
            format!("error {}: {}\n", error.code, error.message).into_boxed_str()
        }
    }
}

fn render_plain_partial(partial: &contract::PartialResult) -> Box<str> {
    let tests = newline_join(partial.tests.as_ref());
    if tests.is_empty() {
        Box::<str>::from("partial\n")
    } else {
        format!("partial\n{tests}").into_boxed_str()
    }
}

fn render_plain_none(none: &contract::NoneResult) -> Box<str> {
    let changed_files = newline_join(none.changed_files.as_ref());
    if changed_files.is_empty() {
        Box::<str>::from("none\n")
    } else {
        format!("none\n{changed_files}").into_boxed_str()
    }
}

fn newline_join(lines: &[Box<str>]) -> Box<str> {
    if lines.is_empty() {
        return Box::<str>::from("");
    }

    let mut content = String::new();
    for line in lines {
        content.push_str(line.as_ref());
        content.push('\n');
    }

    content.into_boxed_str()
}

fn space_join(arguments: &[Box<str>]) -> Box<str> {
    if arguments.is_empty() {
        return Box::<str>::from("");
    }

    let mut content = String::new();
    for argument in arguments {
        if !content.is_empty() {
            content.push(' ');
        }
        content.push_str(argument.as_ref());
    }

    content.into_boxed_str()
}

#[cfg(test)]
mod tests {
    use std::sync;

    use crate::contract;
    use crate::failure;

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        content: sync::Arc<sync::Mutex<String>>,
    }

    impl super::ResultRenderer for RecordingSink {
        fn write(&self, content: &str) -> failure::Result<()> {
            self.content.lock().unwrap().push_str(content);

            Ok(())
        }
    }

    fn partial_result() -> contract::CommandResult {
        contract::CommandResult::Partial(contract::PartialResult {
            tests: Box::from([
                Box::<str>::from("src/accounts.test.ts"),
                Box::<str>::from("src/button.test.tsx"),
            ]),
            reasons: Box::from([]),
        })
    }

    fn shell_output_for(result: contract::CommandResult) -> String {
        let shell_sink = RecordingSink::default();
        super::render(super::RenderRequest {
            sink: shell_sink.clone(),
            format: super::Format::Shell,
            result,
        })
        .unwrap();

        shell_sink.content.lock().unwrap().clone()
    }

    #[test]
    fn output_renderers_preserve_stable_shell_json_and_plain_contracts() {
        let shell_sink = RecordingSink::default();
        let json_sink = RecordingSink::default();
        let plain_sink = RecordingSink::default();

        // These paths are already sorted so the renderer contract can focus on
        // preserving shell argument order instead of performing selection work.
        super::render(super::RenderRequest {
            sink: shell_sink.clone(),
            format: super::Format::Shell,
            result: partial_result(),
        })
        .unwrap();
        super::render(super::RenderRequest {
            sink: json_sink.clone(),
            format: super::Format::Json,
            result: partial_result(),
        })
        .unwrap();
        super::render(super::RenderRequest {
            sink: plain_sink.clone(),
            format: super::Format::Plain,
            result: partial_result(),
        })
        .unwrap();

        assert_eq!(
            shell_sink.content.lock().unwrap().as_str(),
            "src/accounts.test.ts src/button.test.tsx"
        );
        assert!(
            json_sink
                .content
                .lock()
                .unwrap()
                .contains(r#""status":"partial""#)
        );
        assert_eq!(
            plain_sink.content.lock().unwrap().as_str(),
            "partial\nsrc/accounts.test.ts\nsrc/button.test.tsx\n"
        );
    }

    #[test]
    fn shell_output_emits_only_partial_test_path_arguments() {
        let full_result = contract::CommandResult::Full(contract::FullResult {
            reason: Box::<str>::from("global invalidator changed"),
        });
        let none_result = contract::CommandResult::None(contract::NoneResult {
            changed_files: Box::from([Box::<str>::from("README.md")]),
        });
        let error_result = contract::CommandResult::Error(contract::ErrorResult {
            code: Box::<str>::from("config"),
            message: Box::<str>::from("invalid configuration"),
        });

        // Shell output is consumed through command substitution, so non-path
        // statuses must not become accidental test arguments on stdout.
        assert_eq!(
            shell_output_for(partial_result()),
            "src/accounts.test.ts src/button.test.tsx"
        );
        assert_eq!(shell_output_for(full_result), "");
        assert_eq!(shell_output_for(none_result), "");
        assert_eq!(shell_output_for(error_result), "");
    }

    #[test]
    fn auto_output_selects_tui_only_for_interactive_human_streams() {
        assert_eq!(
            super::select_format(super::SelectionRequest {
                requested: super::FormatRequest::Auto,
                stdout: super::StreamInteractivity::Interactive,
            }),
            super::Format::Tui
        );
        assert_eq!(
            super::select_format(super::SelectionRequest {
                requested: super::FormatRequest::Auto,
                stdout: super::StreamInteractivity::NonInteractive,
            }),
            super::Format::Docker
        );
        assert_eq!(
            super::select_format(super::SelectionRequest {
                requested: super::FormatRequest::Explicit(super::Format::Json),
                stdout: super::StreamInteractivity::Interactive,
            }),
            super::Format::Json
        );
    }
}
