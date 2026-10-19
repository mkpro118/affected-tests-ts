//! Binary entry point for the affected-tests-ts command-line interface.

#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::panic))]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(unsafe_code))]

use std::process::ExitCode;

fn main() -> ExitCode {
    match affected_tests_ts::app::run() {
        Ok(affected_tests_ts::app::RunOutcome::Complete) => ExitCode::SUCCESS,
        Ok(affected_tests_ts::app::RunOutcome::FullRun) => ExitCode::from(2),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
