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

/// Whether the command line asks for SCIP: `--precise`, or any `--scip`.
pub(super) fn wanted(common: &Common) -> bool {
    common.precise || !common.scip.is_empty()
}

/// Reads the indexes the command line names (or, for a bare `--precise`, those
/// in `.modfence/scip/`) and upgrades `graph` with them. Returns whether call
/// edges were checked: when no index could be used the run stays on tree-sitter
/// with a warning.
///
/// A `--scip` file that cannot be read exits 74 and one that is not a SCIP
/// index exits 2. The same problem with a file found in `.modfence/scip/` is a
/// warning and the file is skipped. Indexes are applied in command-line order,
/// or in file-name order for the directory; the first of two documents with
/// one path wins.
pub(super) fn ingest(
    common: &Common,
    root: &Path,
    sources: &BTreeMap<String, String>,
    graph: &mut Graph,
    err: &mut impl Write,
) -> Result<bool, Failure> {
    let analysis_root = fs::canonicalize(root)
        .unwrap_or_default()
        .to_string_lossy()
        .replace('\\', "/");
    let explicit = !common.scip.is_empty();
    let paths = if explicit {
        common.scip.clone()
    } else {
        discovered(root)
    };
    if paths.is_empty() {
        writeln!(err, "warning: {NONE_FOUND}")?;
        return Ok(false);
    }
    let mut indexes = Vec::new();
    for path in &paths {
        match scip::load(path, &analysis_root) {
            Ok((index, stats)) => {
                let name = path.display().to_string();
                if let Some(tool) = &index.tool {
                    writeln!(err, "note: read SCIP index {name} from {tool}")?;
                }
                for line in stats.warnings(&name) {
                    writeln!(err, "warning: {line}")?;
                }
                indexes.push(index);
            }
            Err(error) if explicit => return Err(failure(path, &error)),
            Err(error) => writeln!(err, "warning: {}: {error}; skipped", path.display())?,
        }
    }
    if indexes.iter().all(|index| index.documents.is_empty()) {
        writeln!(err, "warning: {NOT_USABLE}")?;
        return Ok(false);
    }
    for line in precise::apply(graph, &indexes, sources).warnings() {
        writeln!(err, "warning: {line}")?;
    }
    Ok(true)
}

/// The `*.scip` files of `.modfence/scip/`, in file-name order.
fn discovered(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root.join(INDEX_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "scip"))
        .collect();
    found.sort();
    found
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
        message: format!("{}: {error}", path.display()),
    }
}
