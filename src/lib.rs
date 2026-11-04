//! modfence: a cross-language module-boundary checker for Python, TypeScript
//! and Rust.
//!
//! The binary (`src/main.rs`) only forwards its arguments and streams to
//! [`run`], so the whole program is testable without spawning a process;
//! `tests/cli.rs` also spawns the real binary, as a user would.

mod cli;
pub mod config;
pub mod extract;
pub mod graph;
pub mod report;
pub mod rules;

pub use cli::run;
