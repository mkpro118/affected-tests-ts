//! Core library for selecting TypeScript tests affected by a Git change set.
//! Side effects are composed at the binary edge while modules expose typed contracts.

#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::panic))]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(unsafe_code))]

pub mod app;
pub(crate) mod app_contract;
pub(crate) mod app_pipeline;
pub(crate) mod app_render;
pub mod cli;
pub mod contract;
#[path = "tui.rs"]
pub mod dashboard;
#[path = "graph.rs"]
pub mod dependencies;
#[path = "discover.rs"]
pub mod discovery;
#[path = "error.rs"]
pub mod failure;
pub mod fs;
#[path = "affected.rs"]
pub mod impact;
#[path = "docker_output.rs"]
pub mod logs;
#[path = "resolve.rs"]
pub mod modules;
pub mod parser;
#[path = "output.rs"]
pub mod presentation;
pub mod progress;
#[path = "path.rs"]
pub mod roots;
#[path = "config.rs"]
pub mod settings;
mod static_refs;
#[path = "git.rs"]
pub mod vcs;
#[path = "trace.rs"]
pub mod work;

#[cfg(any(test, feature = "test-support"))]
pub mod vfs;
