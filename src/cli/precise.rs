//! `--precise` and `--scip`: reading SCIP indexes and upgrading the graph with
//! them (ADR 0003, "Where SCIP indexes come from").

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::args::Common;
use super::{Failure, IO_ERROR, USAGE};
use crate::graph::{Graph, precise};
use crate::scip::{self, Error};

/// Where bare `--precise` looks for indexes, below the analysis root.
const INDEX_DIR: &str = ".modfence/scip";

/// The warning for a bare `--precise` with no index to read.
const NONE_FOUND: &str = "no SCIP indexes in .modfence/scip; call edges were not checked";

/// The warning for indexes that held no document of the analysis.
const NOT_USABLE: &str = "no usable SCIP documents; call edges were not checked";

/// The text of the analysed files, kept only when the command line asks for
/// SCIP (`--precise`, or any `--scip`), to convert the indexes' positions.
pub(super) struct Sources {
    wanted: bool,
    texts: BTreeMap<String, String>,
}

impl Sources {
    /// Keeps files when `common` asks for SCIP.
    pub(super) fn new(common: &Common) -> Self {
        Self {
            wanted: common.precise || !common.scip.is_empty(),
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
/// A `--scip` file that cannot be read exits 74 and one that is not a SCIP
/// index exits 2. The same problem with a file found in `.modfence/scip/` is a
/// warning and the file is skipped. Indexes are applied in command-line order,
/// or in file-name order for the directory; the first of two documents with
/// one path wins.
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
    let (analysis_root, found) = match locate(root, !explicit) {
        Ok(located) => located,
        Err(error) => return Err(Failure::io(shown(root), &error)),
    };
    let paths = if explicit { common.scip.clone() } else { found };
    if paths.is_empty() {
        writeln!(err, "warning: {NONE_FOUND}")?;
        return Ok(false);
    }
    let mut indexes = Vec::new();
    for path in &paths {
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
            Err(error) if explicit => return Err(failure(path, &error)),
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

/// A path safe to print: control characters escaped, so a file name cannot
/// forge diagnostic lines or terminal escapes.
fn shown(path: &Path) -> String {
    scip::printable(&path.display().to_string(), 500)
}

/// A `--scip` file that failed: exit 74 when it cannot be read, 2 when it is
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
