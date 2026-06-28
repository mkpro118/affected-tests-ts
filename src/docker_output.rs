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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::contract;
    use crate::failure;
    use crate::progress;

    #[derive(Clone, Debug, Default)]
    struct RecordingSink {
        lines: Arc<Mutex<Vec<Box<str>>>>,
    }

    impl super::StepOutputSink for RecordingSink {
        fn write_line(&self, line: &str) -> failure::Result<()> {
            self.lines.lock().unwrap().push(Box::<str>::from(line));

            Ok(())
        }
    }

    #[test]
    #[should_panic(expected = "not implemented")]
    fn non_tty_output_emits_only_completed_steps_without_spinners_or_cursor_control() {
        let sink = RecordingSink::default();
        let request = super::RenderRequest {
            sink: sink.clone(),
            events: Box::from([
                progress::Event::Started(progress::Phase::Discovering),
                progress::Event::Finished(progress::Phase::Discovering),
                progress::Event::Started(progress::Phase::Tracing),
                progress::Event::Finished(progress::Phase::Tracing),
            ]),
            result: contract::CommandResult::Partial(contract::PartialResult {
                tests: Box::from([Box::<str>::from("src/file-a.test.ts")]),
                reasons: Box::from([]),
            }),
        };

        // Non-TTY logs are copied into CI output, so they must avoid transient
        // terminal controls that make build logs noisy or nondeterministic.
        super::render(request).unwrap();

        let lines = sink.lines.lock().unwrap().clone();

        assert!(lines.iter().all(|line| !line.contains('\u{1b}')));
        assert!(lines.iter().all(|line| !line.contains('\u{280b}')));
        assert!(lines.iter().all(|line| !line.contains("in progress")));
    }
}
