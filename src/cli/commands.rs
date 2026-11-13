//! The subcommands: load the configuration, walk and extract, then check or
//! print.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::args::{CheckArgs, CheckFormat, Cli, Command, Common, TraceArgs, ViewArgs};
use super::{Failure, OK, precise, text, trace, walker};
use crate::config::{self, Config};
use crate::extract::decode;
use crate::extract::python::PythonExtractor;
use crate::extract::rust::RustExtractor;
use crate::extract::typescript::TsExtractor;
use crate::extract::{ExtractCtx, Extractor, SourceFile};
use crate::graph::{Builder, Graph};
use crate::{output, report, rules};

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
    let (config, graph, checked) = analyse(&args.common, err)?;
    let outcome = rules::check(&graph, &config, checked);
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
    let chosen = args.renderer();
    let (_, graph, _) = analyse(&args.common, err)?;
    let rendered = match chosen {
        Some(draw) => draw.render(&graph, &args.view(&graph)),
        None => text::summary(&graph),
    };
    emit(&rendered, &args.common, out)?;
    Ok(OK)
}

fn run_trace(args: &TraceArgs, out: &mut impl Write, err: &mut impl Write) -> Result<u8, Failure> {
    let chosen = args.view.renderer();
    let selector = trace::parse(&args.selectors)?;
    let (_, graph, _) = analyse(&args.view.common, err)?;
    let selected = trace::select(&graph, &selector);
    let part = output::incident(&graph, &selected);
    let rendered = match chosen {
        Some(draw) => draw.render(&part, &args.view.view(&part)),
        None => trace::render(&part, &selected),
    };
    emit(&rendered, &args.view.common, out)?;
    Ok(OK)
}

/// Turns an I/O error on `path` into a failure that names the action and path.
pub(super) fn failed<'a>(
    action: &'a str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> Failure + 'a {
    move |error| Failure::io(format!("cannot {action} {}", path.display()), &error)
}

/// Writes `rendered` to `--output`, or to standard output.
fn emit(rendered: &str, common: &Common, out: &mut impl Write) -> Result<(), Failure> {
    match &common.output {
        Some(path) => fs::write(path, rendered).map_err(failed("write", path)),
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

/// Reads and parses a configuration file: a file that cannot be read is an
/// I/O error (74), content that is not a valid configuration a usage error.
fn load(path: &Path) -> Result<Config, Failure> {
    let source = fs::read_to_string(path).map_err(failed("read", path))?;
    config::parse(&source).map_err(|error| Failure::usage(format!("{}:{error}", path.display())))
}

/// The directory of a configuration file, which is the analysis root. A bare
/// file name has the working directory (`.`) as its root.
fn root_of(config: &Path) -> PathBuf {
    Path::new(".").join(config.parent().unwrap_or(Path::new("")))
}

/// Loads the configuration, walks the analysis root, extracts every Python,
/// TypeScript and Rust file and builds the graph, upgraded by SCIP indexes when
/// `--precise` or `--scip` asks. File problems go to `err`. The last value says
/// whether call edges were checked, which is when an index was read.
fn analyse(common: &Common, err: &mut impl Write) -> Result<(Config, Graph, bool), Failure> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let (config, root) = match common.config.clone().or_else(|| discover(&cwd)) {
        Some(path) => (load(&path)?, root_of(&path)),
        None => (Config::default(), PathBuf::from(".")),
    };
    let extractors: [&dyn Extractor; 3] = [&PythonExtractor, &TsExtractor, &RustExtractor];
    let wanted = |path: &str| extractors.iter().any(|extractor| extractor.handles(path));
    let files = walker::walk(&root, &config, wanted)?;
    let mut sources = precise::Sources::new(common);
    let mut builder = Builder::new(&config);
    for path in &files {
        let full = root.join(path);
        let bytes = fs::read(&full).map_err(failed("read", &full))?;
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) => {
                for extractor in extractors
                    .iter()
                    .filter(|extractor| extractor.handles(path))
                {
                    let facts = decode::undecodable(extractor.language(), path, error);
                    builder.add(&SourceFile { path, text: "" }, facts);
                }
                continue;
            }
        };
        sources.add(path, text);
        let file = SourceFile { path, text };
        for extractor in extractors
            .iter()
            .filter(|extractor| extractor.handles(path))
        {
            builder.add(&file, extractor.extract(&file, &ExtractCtx::default()));
        }
    }
    let mut graph = builder.build();
    for diagnostic in &graph.diagnostics {
        writeln!(err, "{}", text::diagnostic(diagnostic))?;
    }
    let checked = precise::ingest(common, &root, &sources, &mut graph, err)?;
    Ok((config, graph, checked))
}
