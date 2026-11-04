//! The subcommands: load the configuration, walk and extract, then check or
//! print.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::args::{CheckArgs, CheckFormat, Cli, Command, Common, TraceArgs, ViewArgs};
use super::{Failure, OK, text, trace, walker};
use crate::config::{self, Config};
use crate::extract::python::PythonExtractor;
use crate::extract::typescript::TsExtractor;
use crate::extract::{ExtractCtx, Extractor, SourceFile};
use crate::graph::{Builder, Graph};
use crate::{report, rules};

/// The name of the configuration file.
const CONFIG_FILE: &str = "modfence.toml";

/// Runs the subcommand and returns the exit status: 0, or 1 when `check`
/// found a violation at or above `--fail-on`.
pub(super) fn execute(
    cli: &Cli,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<u8, Failure> {
    match &cli.command {
        Command::Check(args) => run_check(args, out, err),
        Command::Graph(args) => run_graph(args, out, err),
        Command::Trace(args) => run_trace(args, out, err),
    }
}

fn run_check(args: &CheckArgs, out: &mut impl Write, err: &mut impl Write) -> Result<u8, Failure> {
    let (config, graph) = analyse(&args.common, err)?;
    let outcome = rules::check(&graph, &config, false);
    for notice in &outcome.notices {
        writeln!(err, "{}: {}", text::label(notice.level), notice.message)?;
    }
    let rendered = match args.format {
        CheckFormat::Text => text::violations(&outcome),
        CheckFormat::Sarif => report::sarif::render(&outcome, &config),
        CheckFormat::Github => report::github::render(&outcome),
    };
    emit(&rendered, &args.common, out)?;
    Ok(u8::from(outcome.fails(args.fail_on.severity())))
}

fn run_graph(args: &ViewArgs, out: &mut impl Write, err: &mut impl Write) -> Result<u8, Failure> {
    args.require_text()?;
    let (_, graph) = analyse(&args.common, err)?;
    emit(&text::summary(&graph), &args.common, out)?;
    Ok(OK)
}

fn run_trace(args: &TraceArgs, out: &mut impl Write, err: &mut impl Write) -> Result<u8, Failure> {
    args.view.require_text()?;
    let selector = trace::parse(&args.selectors)?;
    let (_, graph) = analyse(&args.view.common, err)?;
    emit(&trace::render(&graph, &selector), &args.view.common, out)?;
    Ok(OK)
}

/// Writes `rendered` to `--output`, or to standard output.
fn emit(rendered: &str, common: &Common, out: &mut impl Write) -> Result<(), Failure> {
    match &common.output {
        Some(path) => fs::write(path, rendered)
            .map_err(|error| Failure::usage(format!("cannot write {}: {error}", path.display()))),
        None => out.write_all(rendered.as_bytes()).map_err(Failure::from),
    }
}

/// The nearest `modfence.toml` in `start` or above it.
fn discover(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join(CONFIG_FILE))
        .find(|candidate| candidate.is_file())
}

/// Reads and parses a configuration file.
fn load(path: &Path) -> Result<Config, Failure> {
    let source = fs::read_to_string(path)
        .map_err(|error| Failure::usage(format!("cannot read {}: {error}", path.display())))?;
    config::parse(&source).map_err(|error| Failure::usage(format!("{}:{error}", path.display())))
}

/// The directory of a configuration file, which is the analysis root. A bare
/// file name has the working directory (`.`) as its root.
fn root_of(config: &Path) -> PathBuf {
    Path::new(".").join(config.parent().unwrap_or(Path::new("")))
}

/// Loads the configuration, walks the analysis root, extracts every Python and
/// TypeScript file and builds the graph. File problems go to `err`.
fn analyse(common: &Common, err: &mut impl Write) -> Result<(Config, Graph), Failure> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let (config, root) = match common.config.clone().or_else(|| discover(&cwd)) {
        Some(path) => (load(&path)?, root_of(&path)),
        None => (Config::default(), PathBuf::from(".")),
    };
    let extractors: [&dyn Extractor; 2] = [&PythonExtractor, &TsExtractor];
    let wanted = |path: &str| extractors.iter().any(|extractor| extractor.handles(path));
    let files = walker::walk(&root, &config, wanted)?;
    let mut builder = Builder::new(&config);
    for path in &files {
        let bytes = fs::read(root.join(path))?;
        let decoded = String::from_utf8_lossy(&bytes);
        let file = SourceFile {
            path,
            text: &decoded,
        };
        for extractor in extractors
            .iter()
            .filter(|extractor| extractor.handles(path))
        {
            builder.add(&file, extractor.extract(&file, &ExtractCtx::default()));
        }
    }
    let graph = builder.build();
    for diagnostic in &graph.diagnostics {
        writeln!(err, "{}", text::diagnostic(diagnostic))?;
    }
    Ok((config, graph))
}
