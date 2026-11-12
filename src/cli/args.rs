//! The command line (ADR 0004, "Decision"): subcommands, flags and values.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::config::{Severity, validate_tag_key};
use crate::graph::Graph;
use crate::output::{self, Dot, GraphMl, Html, Json, Mermaid, Renderer, View};

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
    /// Resolve call edges with SCIP indexes. Without `--scip` it reads the
    /// `*.scip` files in `.modfence/scip/` below the analysis root.
    #[arg(long)]
    pub(super) precise: bool,
    /// A prebuilt SCIP index to read; repeatable, relative to the current
    /// directory. Implies `--precise`.
    #[arg(long, value_name = "FILE")]
    pub(super) scip: Vec<PathBuf>,
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
    /// The graph document.
    Json,
    /// Graphviz DOT (`NC5b`).
    Dot,
    /// Mermaid (`NC5b`).
    Mermaid,
    /// `GraphML` (`NC5b`).
    Graphml,
    /// Interactive HTML (`NC5c`).
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

impl From<Level> for output::Level {
    fn from(level: Level) -> Self {
        match level {
            Level::Package => Self::Package,
            Level::Module => Self::Module,
            Level::Symbol => Self::Symbol,
        }
    }
}

/// Reads the value of `--group-by`: a tag key, as in a selector.
fn tag_key(raw: &str) -> Result<String, String> {
    validate_tag_key(raw).map(|()| raw.to_owned())
}

/// The arguments of `graph`, and the flags `trace` shares with it.
#[derive(Debug, Args)]
pub(super) struct ViewArgs {
    /// The common flags.
    #[command(flatten)]
    pub(super) common: Common,
    /// The output format. Machine formats need an explicit `--format`.
    #[arg(long, value_enum, default_value = "text")]
    pub(super) format: ViewFormat,
    /// Cluster the nodes of a diagram by the values of this tag key, instead
    /// of by package.
    #[arg(long, value_name = "KEY", value_parser = tag_key)]
    pub(super) group_by: Option<String>,
    /// How much a diagram shows. JSON always has everything.
    #[arg(long, value_enum, default_value = "module")]
    pub(super) level: Level,
}

impl ViewArgs {
    /// The renderer of `--format`, or `None` for the human text. A format adds
    /// one arm here.
    pub(super) fn renderer(&self) -> Option<&'static dyn Renderer> {
        match self.format {
            ViewFormat::Text => None,
            ViewFormat::Json => Some(&Json),
            ViewFormat::Dot => Some(&Dot),
            ViewFormat::Mermaid => Some(&Mermaid),
            ViewFormat::Graphml => Some(&GraphMl),
            ViewFormat::Html => Some(&Html),
        }
    }

    /// The view of `graph` that `--level` and `--group-by` ask for.
    pub(super) fn view<'a>(&self, graph: &'a Graph) -> View<'a> {
        View::new(graph, self.level.into(), self.group_by.as_deref())
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
