//! Static Docker-style non-TTY progress and result rendering contracts.

use std::collections::BTreeMap;

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
pub fn render<S>(request: RenderRequest<S>) -> failure::Result<()>
where
    S: StepOutputSink,
{
    let RenderRequest {
        sink,
        events,
        result,
    } = request;
    let completed_steps = completed_steps(events.as_ref());
    let total_steps = completed_steps.len() + 1;

    for (step_index, step) in completed_steps.iter().enumerate() {
        sink.write_line(
            render_step(StepRenderRequest {
                step,
                step_number: step_index + 1,
                total_steps,
            })
            .as_ref(),
        )?;
    }

    sink.write_line(render_result(&result, total_steps).as_ref())
}

fn completed_steps(events: &[progress::Event]) -> Box<[progress::Step]> {
    let mut steps = BTreeMap::<progress::Phase, progress::Step>::new();
    for event in events {
        match event {
            progress::Event::Completed(step) => {
                steps.insert(step.phase, step.clone());
            }
            progress::Event::Finished(phase) => {
                steps.entry(*phase).or_insert_with(|| progress::Step {
                    phase: *phase,
                    detail: Box::<str>::from("completed"),
                    elapsed: None,
                });
            }
            progress::Event::Started(_)
            | progress::Event::TraceStarted(_)
            | progress::Event::TraceJoined(_)
            | progress::Event::TraceReused(_)
            | progress::Event::TraceCompleted(_)
            | progress::Event::TraceFailed(_)
            | progress::Event::TraceCycle(_) => {}
        }
    }

    steps.into_values().collect()
}

#[derive(Clone, Copy, Debug)]
struct StepRenderRequest<'a> {
    step: &'a progress::Step,
    step_number: usize,
    total_steps: usize,
}

fn render_step(request: StepRenderRequest<'_>) -> Box<str> {
    let timing = request
        .step
        .elapsed
        .as_ref()
        .map_or_else(String::new, |elapsed| format!(" {elapsed}"));

    format!(
        "=> [{:<8} {}/{}] {}{timing}",
        request.step.phase.label(),
        request.step_number,
        request.total_steps,
        request.step.detail
    )
    .into_boxed_str()
}

fn render_result(result: &contract::CommandResult, total_steps: usize) -> Box<str> {
    let summary = match result {
        contract::CommandResult::Partial(partial) => {
            format!("partial: {}", affected_test_count(partial.tests.len())).into_boxed_str()
        }
        contract::CommandResult::Full(full) => format!(
            "full: {} ({})",
            affected_test_count(full.tests.len()),
            full.reason
        )
        .into_boxed_str(),
        contract::CommandResult::None(_none) => Box::<str>::from("none: no affected tests"),
        contract::CommandResult::Error(error) => {
            format!("error {}: {}", error.code, error.message).into_boxed_str()
        }
    };

    format!("=> [{:<8} {total_steps}/{total_steps}] {summary}", "result").into_boxed_str()
}

fn affected_test_count(test_count: usize) -> Box<str> {
    match test_count {
        1 => Box::<str>::from("1 affected test"),
        count => format!("{count} affected tests").into_boxed_str(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::contract;
    use crate::failure;
    use crate::progress;
    use crate::roots;

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

    fn path(value: &str) -> roots::RootRelativePath {
        roots::RootRelativePath::try_from(value).unwrap()
    }

    #[test]
    fn non_tty_output_emits_only_completed_steps_without_spinners_or_cursor_control() {
        let sink = RecordingSink::default();
        let request = super::RenderRequest {
            sink: sink.clone(),
            events: Box::from([
                progress::Event::Started(progress::Phase::Discovering),
                progress::Event::TraceStarted(path("src/file-d.ts")),
                progress::Event::Completed(progress::Step {
                    phase: progress::Phase::Tracing,
                    detail: Box::<str>::from("traced 2 changed files, 1 collapsed path"),
                    elapsed: Some(Box::<str>::from("31ms")),
                }),
                progress::Event::Started(progress::Phase::Tracing),
                progress::Event::TraceJoined(path("src/file-b.ts")),
                progress::Event::Completed(progress::Step {
                    phase: progress::Phase::Discovering,
                    detail: Box::<str>::from("scanned TypeScript files: 3 files"),
                    elapsed: Some(Box::<str>::from("184ms")),
                }),
                progress::Event::TraceCompleted(path("src/file-d.ts")),
                progress::Event::Completed(progress::Step {
                    phase: progress::Phase::Parsing,
                    detail: Box::<str>::from("extracted import graph: 7 edges"),
                    elapsed: Some(Box::<str>::from("1.3s")),
                }),
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
        let expected = Vec::from([
            Box::<str>::from("=> [discover 1/4] scanned TypeScript files: 3 files 184ms"),
            Box::<str>::from("=> [parse    2/4] extracted import graph: 7 edges 1.3s"),
            Box::<str>::from("=> [trace    3/4] traced 2 changed files, 1 collapsed path 31ms"),
            Box::<str>::from("=> [result   4/4] partial: 1 affected test"),
        ]);

        assert_eq!(lines, expected);
        assert!(lines.iter().all(|line| !line.contains('\u{1b}')));
        assert!(lines.iter().all(|line| !line.contains('\u{280b}')));
        assert!(lines.iter().all(|line| !line.contains("in progress")));
        assert!(lines.iter().all(|line| !line.contains("src/file-d.ts")));
        assert!(lines.iter().all(|line| !line.contains("src/file-b.ts")));
    }

    #[test]
    fn full_result_output_matches_static_docker_contract() {
        let sink = RecordingSink::default();
        let request = super::RenderRequest {
            sink: sink.clone(),
            events: Box::from([
                progress::Event::Completed(progress::Step {
                    phase: progress::Phase::Discovering,
                    detail: Box::<str>::from("detected changed files: 4"),
                    elapsed: Some(Box::<str>::from("19ms")),
                }),
                progress::Event::Completed(progress::Step {
                    phase: progress::Phase::Resolving,
                    detail: Box::<str>::from("global invalidator matched: tsconfig.json"),
                    elapsed: Some(Box::<str>::from("2ms")),
                }),
            ]),
            result: contract::CommandResult::Full(contract::FullResult {
                reason: Box::<str>::from("global invalidator changed: tsconfig.json"),
                tests: Box::from([
                    Box::<str>::from("src/accounts.test.ts"),
                    Box::<str>::from("src/button.test.tsx"),
                ]),
            }),
        };

        // A full selection is copied into CI logs, so the action must name the
        // concrete Bun suite rather than a generic test command.
        super::render(request).unwrap();

        assert_eq!(
            sink.lines.lock().unwrap().clone(),
            Vec::from([
                Box::<str>::from("=> [discover 1/3] detected changed files: 4 19ms"),
                Box::<str>::from("=> [resolve  2/3] global invalidator matched: tsconfig.json 2ms",),
                Box::<str>::from(
                    "=> [result   3/3] full: 2 affected tests (global invalidator changed: tsconfig.json)"
                ),
            ])
        );
    }
}
