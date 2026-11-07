//! Parsing `index.scip` into [`Index`].

use std::path::Path;

use protobuf::Message;
use scip::types::occurrence::{Typed_enclosing_range, Typed_range};
use scip::types::{self as wire, PositionEncoding as Wire};

use super::path::{is_safe_relative, root_prefix};
use super::{Document, Error, Index, Occurrence, PositionEncoding, RawRange, ReadStats};

/// The largest index the reader accepts, in bytes. A larger file is an error
/// rather than a reason to exhaust memory; real indexes of large repositories
/// are tens of megabytes.
pub const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;

/// Fails when an index of `bytes` bytes is larger than [`MAX_INDEX_BYTES`].
///
/// # Errors
///
/// [`Error::TooLarge`] above the limit.
pub fn check_size(bytes: u64) -> Result<(), Error> {
    if bytes > MAX_INDEX_BYTES {
        Err(Error::TooLarge { bytes })
    } else {
        Ok(())
    }
}

/// Reads the index file at `path`, checking its size before reading it.
///
/// # Errors
///
/// [`Error::Io`] when it cannot be read, [`Error::TooLarge`], and
/// everything [`read_index`] reports.
pub fn load(path: &Path, root: &str) -> Result<(Index, ReadStats), Error> {
    let io = |error: std::io::Error| Error::Io(error.to_string());
    check_size(std::fs::metadata(path).map_err(io)?.len())?;
    read_index(&std::fs::read(path).map_err(io)?, root)
}

/// Parses the bytes of an index. Document paths are made relative to
/// `root` (the analysis root: absolute, `/`-separated) using the index's `project_root`;
/// documents whose path is unsafe are dropped and counted, and an index whose
/// root is outside the analysis root yields no documents (see [`ReadStats`]).
///
/// # Errors
///
/// [`Error::TooLarge`] for more than [`MAX_INDEX_BYTES`] and
/// [`Error::Parse`] for bytes that are not a SCIP index (truncated or
/// garbage input included).
pub fn read_index(bytes: &[u8], root: &str) -> Result<(Index, ReadStats), Error> {
    check_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
    let mut raw = wire::Index::parse_from_bytes(bytes).map_err(|e| Error::Parse(e.to_string()))?;
    let mut stats = ReadStats::default();
    let tool = raw
        .metadata
        .tool_info
        .as_ref()
        .filter(|info| !info.name.is_empty())
        .map(|info| {
            format!("{} {}", info.name, info.version)
                .trim_end()
                .to_owned()
        });
    let documents = std::mem::take(&mut raw.documents);
    let project_root = raw.metadata.project_root.clone();
    let Some(prefix) = root_prefix(&project_root, root) else {
        stats.rejected_documents = documents.len();
        stats.foreign_root = Some(project_root);
        let index = Index {
            tool,
            documents: Vec::new(),
        };
        return Ok((index, stats));
    };
    let mut kept = Vec::with_capacity(documents.len());
    for document in documents {
        if !is_safe_relative(&document.relative_path) {
            stats.rejected_documents += 1;
            continue;
        }
        let encoding = match document.position_encoding.enum_value() {
            Ok(Wire::UTF8CodeUnitOffsetFromLineStart) => PositionEncoding::Utf8,
            Ok(Wire::UTF16CodeUnitOffsetFromLineStart) => PositionEncoding::Utf16,
            Ok(Wire::UTF32CodeUnitOffsetFromLineStart) => PositionEncoding::Utf32,
            _ => {
                stats.unspecified_encoding += 1;
                PositionEncoding::Utf8
            }
        };
        let mut occurrences = Vec::with_capacity(document.occurrences.len());
        for occurrence in document.occurrences {
            let range = range_of(&occurrence);
            if let (Some(range), false) = (range, occurrence.symbol.is_empty()) {
                occurrences.push(Occurrence {
                    enclosing: enclosing_of(&occurrence),
                    symbol: occurrence.symbol,
                    roles: occurrence.symbol_roles,
                    range,
                });
            } else {
                stats.malformed_occurrences += 1;
            }
        }
        kept.push(Document {
            path: format!("{prefix}{}", document.relative_path),
            encoding,
            occurrences,
        });
    }
    let index = Index {
        tool,
        documents: kept,
    };
    Ok((index, stats))
}

/// The occurrence's range: the typed form wins over the deprecated integer
/// list, as `scip.proto` says.
fn range_of(occurrence: &wire::Occurrence) -> Option<RawRange> {
    if let Some(Typed_range::SingleLineRange(r)) = &occurrence.typed_range {
        RawRange::from_wire(r.line, r.start_character, r.line, r.end_character)
    } else if let Some(Typed_range::MultiLineRange(r)) = &occurrence.typed_range {
        RawRange::from_wire(r.start_line, r.start_character, r.end_line, r.end_character)
    } else {
        from_list(&occurrence.range)
    }
}

/// The enclosing range of a definition, the same way.
fn enclosing_of(occurrence: &wire::Occurrence) -> Option<RawRange> {
    if let Some(Typed_enclosing_range::SingleLineEnclosingRange(r)) =
        &occurrence.typed_enclosing_range
    {
        RawRange::from_wire(r.line, r.start_character, r.line, r.end_character)
    } else if let Some(Typed_enclosing_range::MultiLineEnclosingRange(r)) =
        &occurrence.typed_enclosing_range
    {
        RawRange::from_wire(r.start_line, r.start_character, r.end_line, r.end_character)
    } else {
        from_list(&occurrence.enclosing_range)
    }
}

/// The deprecated integer list: three elements for one line, four otherwise.
fn from_list(list: &[i32]) -> Option<RawRange> {
    match *list {
        [line, start, end] => RawRange::from_wire(line, start, line, end),
        [start_line, start, end_line, end] => RawRange::from_wire(start_line, start, end_line, end),
        _ => None,
    }
}
