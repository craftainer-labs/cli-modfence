//! The command line (ADR 0004, "Decision"): subcommands, flags and values.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use super::Failure;
use crate::config::Severity;

/// A cross-language module-boundary checker for Python, TypeScript and Rust.
#[derive(Debug, Parser)]
#[command(name = "modfence", version, about)]
pub(super) struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub(super) command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub(super) enum Command {
    /// Check the rules and report the violations.
    Check(CheckArgs),
    /// Build the dependency graph and print it.
    Graph(ViewArgs),
    /// Print the nodes whose tags match a selector, with their edges.
    Trace(TraceArgs),
}

/// The flags every subcommand takes.
#[derive(Debug, Args)]
pub(super) struct Common {
    /// The `modfence.toml` to use. By default it is found by walking up from
    /// the working directory; the analysis root is its directory.
    #[arg(long, value_name = "PATH")]
    pub(super) config: Option<PathBuf>,
    /// Write the output to this file instead of standard output.
    #[arg(long, value_name = "FILE")]
    pub(super) output: Option<PathBuf>,
}

/// The output formats of `check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum CheckFormat {
    /// Human-readable lines.
    Text,
    /// SARIF 2.1.0 for code scanning.
    Sarif,
    /// GitHub workflow commands.
    Github,
}

/// The severities `--fail-on` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum FailOn {
    /// Only errors fail the check.
    Error,
    /// Errors and warnings fail the check.
    Warning,
    /// Any violation fails the check.
    Note,
}

impl FailOn {
    /// The lowest severity that fails the check.
    pub(super) const fn severity(self) -> Severity {
        match self {
            Self::Error => Severity::Error,
            Self::Warning => Severity::Warning,
            Self::Note => Severity::Note,
        }
    }
}

/// The arguments of `check`.
#[derive(Debug, Args)]
pub(super) struct CheckArgs {
    /// The common flags.
    #[command(flatten)]
    pub(super) common: Common,
    /// The output format.
    #[arg(long, value_enum, default_value = "text")]
    pub(super) format: CheckFormat,
    /// The lowest severity that makes the exit status 1.
    #[arg(long, value_enum, default_value = "error")]
    pub(super) fail_on: FailOn,
}

/// The output formats of `graph` and `trace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum ViewFormat {
    /// Human-readable text.
    Text,
    /// The graph document (NC5).
    Json,
    /// Graphviz DOT (NC5).
    Dot,
    /// Mermaid (NC5).
    Mermaid,
    /// GraphML (NC5).
    Graphml,
    /// Interactive HTML (NC5).
    Html,
}

/// How much detail a diagram shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum Level {
    /// Packages and external packages.
    Package,
    /// Plus modules.
    Module,
    /// Plus symbols.
    Symbol,
}

/// The arguments of `graph`, and the flags `trace` shares with it.
#[derive(Debug, Args)]
pub(super) struct ViewArgs {
    /// The common flags.
    #[command(flatten)]
    pub(super) common: Common,
    /// The output format. Only `text` exists until NC5.
    #[arg(long, value_enum, default_value = "text")]
    pub(super) format: ViewFormat,
    /// Cluster nodes by the values of this tag key (NC5).
    #[arg(long, value_name = "KEY")]
    pub(super) group_by: Option<String>,
    /// How much a diagram shows (NC5).
    #[arg(long, value_enum)]
    pub(super) level: Option<Level>,
}

impl ViewArgs {
    /// Refuses what the output layer (NC5) does not provide yet.
    pub(super) fn require_text(&self) -> Result<(), Failure> {
        if self.format != ViewFormat::Text {
            let name = format!("{:?}", self.format).to_lowercase();
            return Err(Failure::usage(format!(
                "--format {name} is not implemented yet (NC5)"
            )));
        }
        if self.group_by.is_some() {
            return Err(Failure::usage("--group-by is not implemented yet (NC5)"));
        }
        if self.level.is_some() {
            return Err(Failure::usage("--level is not implemented yet (NC5)"));
        }
        Ok(())
    }
}

/// The arguments of `trace`.
#[derive(Debug, Args)]
pub(super) struct TraceArgs {
    /// The flags shared with `graph`.
    #[command(flatten)]
    pub(super) view: ViewArgs,
    /// Selectors, `key=value` pairs that are AND-ed. Commas separate
    /// alternatives for one key, and double quotes make a comma literal.
    #[arg(required = true, value_name = "KEY=VALUE")]
    pub(super) selectors: Vec<String>,
}
