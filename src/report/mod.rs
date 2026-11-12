//! Output of the rules engine's [`Outcome`](crate::rules::Outcome) (ADR 0004,
//! "Reports"): SARIF 2.1.0 for code scanning and workflow commands for
//! GitHub annotations.
//!
//! Both are pure functions from an outcome to text, and deterministic: the
//! same outcome gives the same bytes on every platform. Printing, the
//! stderr notices and the exit status belong to the CLI.

pub mod github;
pub mod sarif;
