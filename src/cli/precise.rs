//! `--precise` and `--scip`: reading SCIP indexes and upgrading the graph with
//! them (ADR 0003, "Where SCIP indexes come from").

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::args::Common;
use super::commands::failed;
use super::{Failure, IO_ERROR, USAGE};
use crate::extract::Lang;
use crate::graph::{Graph, precise};
use crate::scip::{self, Error};

/// Where bare `--precise` looks for indexes, below the analysis root.
const INDEX_DIR: &str = ".modfence/scip";

/// The warning for a bare `--precise` with no index to read.
const NONE_FOUND: &str = "no SCIP indexes in .modfence/scip; call edges were not checked";

/// The warning for `--run-indexers` when no indexer produced an index.
const NONE_PRODUCED: &str = "no indexer produced an index; call edges were not checked";

/// The warning for `--run-indexers` where it is not supported.
#[cfg(not(unix))]
const UNSUPPORTED: &str = "--run-indexers is not supported on this platform; no indexer was run";

/// The warning for indexes that held no document of the analysis.
const NOT_USABLE: &str = "no usable SCIP documents; call edges were not checked";

/// The text of the analysed files, kept only when the command line asks for
/// SCIP (`--precise`, `--run-indexers` or any `--scip`), to convert the indexes' positions.
pub(super) struct Sources {
    wanted: bool,
    texts: BTreeMap<String, String>,
}

impl Sources {
    /// Keeps files when `common` asks for SCIP.
    pub(super) fn new(common: &Common) -> Self {
        Self {
            wanted: common.precise || common.run_indexers || !common.scip.is_empty(),
            texts: BTreeMap::new(),
        }
    }

    /// Remembers the text of `path`, if SCIP was asked for.
    pub(super) fn add(&mut self, path: &str, text: &str) {
        if self.wanted {
            self.texts.insert(path.to_owned(), text.to_owned());
        }
    }
}

/// Reads the indexes the command line names (or, for a bare `--precise`, those
/// in `.modfence/scip/`) and upgrades `graph` with them. Returns whether call
/// edges were checked: false without SCIP flags, and when no index could be
/// used, in which case the run stays on tree-sitter with a warning.
///
/// A file that cannot be read exits 74 and one that is not a SCIP index exits
/// 2, whether it was named with `--scip` or found in `.modfence/scip/`. With
/// `--run-indexers` the indexers run first and the indexes they wrote are read
/// after the named ones; a bad one of those is a warning. Indexes are applied
/// in command-line order, or in file-name order for the directory; the first of
/// two documents with one path wins.
pub(super) fn ingest(
    common: &Common,
    root: &Path,
    sources: &Sources,
    graph: &mut Graph,
    err: &mut impl Write,
) -> Result<bool, Failure> {
    if !sources.wanted {
        return Ok(false);
    }
    let explicit = !common.scip.is_empty();
    let discover = !explicit && !common.run_indexers;
    let (analysis_root, found) = match locate(root, discover) {
        Ok(located) => located,
        Err(error) => return Err(Failure::io(shown(root), &error)),
    };
    // Files named with `--scip` and found in the directory must be good; what
    // an indexer just wrote is not trusted that far (ADR 0003, invariant 18).
    let named = if explicit { common.scip.clone() } else { found };
    let mut paths: Vec<(PathBuf, bool)> = named.into_iter().map(|path| (path, true)).collect();
    if common.run_indexers {
        let produced = run_indexers(Path::new(&analysis_root), &graph.languages, err)?;
        paths.extend(produced.into_iter().map(|path| (path, false)));
    }
    if paths.is_empty() {
        let warning = if common.run_indexers {
            NONE_PRODUCED
        } else {
            NONE_FOUND
        };
        writeln!(err, "warning: {warning}")?;
        return Ok(false);
    }
    let mut indexes = Vec::new();
    for (path, strict) in &paths {
        match scip::load(path, &analysis_root) {
            Ok((index, stats)) => {
                let name = shown(path);
                if let Some(tool) = &index.tool {
                    writeln!(err, "note: read SCIP index {name} from {tool}")?;
                }
                for line in stats.warnings(&name) {
                    writeln!(err, "warning: {line}")?;
                }
                indexes.push(index);
            }
            Err(error) if *strict => return Err(failure(path, &error)),
            Err(error) => writeln!(err, "warning: {}: {error}; skipped", shown(path))?,
        }
    }
    let stats = precise::apply(graph, &indexes, &sources.texts);
    for line in stats.warnings() {
        writeln!(err, "warning: {line}")?;
    }
    if stats.documents == 0 {
        writeln!(err, "warning: {NOT_USABLE}")?;
        return Ok(false);
    }
    Ok(true)
}

/// The canonical analysis root (with `/` separators) and, when `discover` is
/// set, the `*.scip` files of `.modfence/scip/` in file-name order. A missing
/// directory holds none; any other read error is returned.
fn locate(root: &Path, discover: bool) -> std::io::Result<(String, Vec<PathBuf>)> {
    let analysis_root = fs::canonicalize(root)?.to_string_lossy().replace('\\', "/");
    let mut found = Vec::new();
    if discover {
        match fs::read_dir(root.join(INDEX_DIR)) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry?.path();
                    if path.extension().is_some_and(|ext| ext == "scip") {
                        found.push(path);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    found.sort();
    Ok((analysis_root, found))
}

/// Runs the indexers for the languages analysed and returns the indexes they
/// wrote. A missing indexer, a failure or an empty result is a warning and
/// leaves that language on tree-sitter; nothing here changes the exit status.
/// An index left by an earlier run is removed first, so it cannot pass for this
/// run's. modfence writes only below `.modfence/`.
#[cfg(unix)]
fn run_indexers(
    root: &Path,
    languages: &BTreeSet<Lang>,
    err: &mut impl Write,
) -> Result<Vec<PathBuf>, Failure> {
    use crate::scip::run;

    let out_dir = root.join(INDEX_DIR);
    let has_tsconfig = root.join("tsconfig.json").is_file();
    let name = run::project_name(root);
    let plans = run::plan(languages, root, &out_dir, &name, has_tsconfig);
    if plans.is_empty() {
        return Ok(Vec::new());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut produced = Vec::new();
    for plan in plans {
        let (program, lang) = (plan.program, plan.lang.as_str());
        let stays = format!("{lang} stays on tree-sitter");
        let Some(exe) = run::resolve(program, &path) else {
            writeln!(err, "warning: {program} not found on PATH; {stays}")?;
            continue;
        };
        let line = run::command_line(&exe, &plan.args);
        writeln!(err, "note: running {line}")?;
        fs::create_dir_all(&out_dir).map_err(failed("create", &out_dir))?;
        let _ = fs::remove_file(&plan.output);
        match run::spawn(&exe, &plan.args, root) {
            Ok(status) if status.success() && plan.output.is_file() => {
                produced.push(plan.output);
            }
            Ok(status) => writeln!(err, "warning: {program}: {status}, or no index; {stays}")?,
            Err(error) => writeln!(err, "warning: {program} cannot start: {error}; {stays}")?,
        }
    }
    Ok(produced)
}

/// `--run-indexers` runs other programs by their path, which is only safe where
/// no shell or `.bat` interpretation can come into play (CVE-2024-24576); it
/// is not supported on this platform.
#[cfg(not(unix))]
fn run_indexers(
    _root: &Path,
    _languages: &BTreeSet<Lang>,
    err: &mut impl Write,
) -> Result<Vec<PathBuf>, Failure> {
    writeln!(err, "warning: {UNSUPPORTED}")?;
    Ok(Vec::new())
}

/// A path safe to print: control characters escaped, so a file name cannot
/// forge diagnostic lines or terminal escapes.
fn shown(path: &Path) -> String {
    scip::printable(&path.display().to_string(), 500)
}

/// An index file that failed: exit 74 when it cannot be read, 2 when it is
/// not an index (or too large to be read as one). The message escapes whatever
/// came from the index (`scip::Error`'s `Display`).
fn failure(path: &Path, error: &Error) -> Failure {
    let code = if matches!(error, Error::Io(_)) {
        IO_ERROR
    } else {
        USAGE
    };
    Failure {
        code,
        message: format!("{}: {error}", shown(path)),
    }
}
