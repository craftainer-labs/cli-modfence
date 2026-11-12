//! Reading SCIP indexes for `--precise` (ADR 0003). The reader turns the bytes
//! of an `index.scip` into plain data ([`Index`]); it knows nothing of the
//! graph, starts no process and opens no connection (NFR-0002).

mod path;
mod position;
mod read;
pub mod run;

pub use position::{LineIndex, PositionEncoding, RawRange};
pub use read::{MAX_INDEX_BYTES, check_size, load, load_with_limit, read_index};

use std::fmt;

/// The `Definition` bit of `symbol_roles`.
pub const DEFINITION: i32 = 0x1;
/// The `Import` bit of `symbol_roles`.
pub const IMPORT: i32 = 0x2;

/// One occurrence of a symbol in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The SCIP symbol string; never empty. `local N` symbols are kept as they
    /// are (the caller decides they resolve nothing outside their document).
    pub symbol: String,
    /// The `SymbolRole` bitset.
    pub roles: i32,
    /// Where the symbol appears.
    pub range: RawRange,
    /// The enclosing range of a definition, when the indexer gave a valid one.
    pub enclosing: Option<RawRange>,
}

impl Occurrence {
    /// Whether this occurrence defines its symbol ([`DEFINITION`]).
    #[must_use]
    pub const fn is_definition(&self) -> bool {
        self.roles & DEFINITION != 0
    }
}

/// A document of an index, with a path already made relative to the analysis
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The path relative to the analysis root, `/`-separated, safe to join
    /// below it (no `..`, not absolute).
    pub path: String,
    /// How the occurrences' offsets are counted.
    pub encoding: PositionEncoding,
    /// The occurrences with a valid range and a symbol, in index order.
    pub occurrences: Vec<Occurrence>,
}

/// A parsed index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    /// The indexer's name and version, when the index states them.
    pub tool: Option<String>,
    /// The usable documents, in index order.
    pub documents: Vec<Document>,
}

/// What the reader left out or had to assume. The caller prints
/// [`ReadStats::warnings`] on standard error.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ReadStats {
    /// Documents dropped for an unsafe path (absolute, `..`, a backslash), or
    /// all of them when the index root is outside the analysis root.
    pub rejected_documents: usize,
    /// The index's `project_root` when it is not the analysis root or below it.
    pub foreign_root: Option<String>,
    /// Occurrences dropped for an invalid range or an empty symbol.
    pub malformed_occurrences: usize,
    /// Documents that state no (or an unknown) position encoding, read as
    /// UTF-8.
    pub unspecified_encoding: usize,
}

impl ReadStats {
    /// One line per thing the reader skipped or assumed, for index `name`.
    #[must_use]
    pub fn warnings(&self, name: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(root) = &self.foreign_root {
            let root = printable(root, 200);
            lines.push(format!(
                "{name}: index root {root} is outside the analysis root; {} documents ignored",
                self.rejected_documents
            ));
        } else if self.rejected_documents > 0 {
            lines.push(format!(
                "{name}: {} documents with an unsafe path were ignored",
                self.rejected_documents
            ));
        }
        if self.malformed_occurrences > 0 {
            lines.push(format!(
                "{name}: {} occurrences with an invalid range or no symbol were ignored",
                self.malformed_occurrences
            ));
        }
        if self.unspecified_encoding > 0 {
            lines.push(format!(
                "{name}: {} documents state no position encoding; read as UTF-8",
                self.unspecified_encoding
            ));
        }
        lines
    }
}

/// Why an index could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The index is larger than [`MAX_INDEX_BYTES`].
    TooLarge {
        /// Its size in bytes.
        bytes: u64,
        /// The most that was accepted, in bytes.
        limit: u64,
    },
    /// The file could not be read (the operating system's message).
    Io(String),
    /// The bytes are not a SCIP index (the parser's message).
    Parse(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, limit } => write!(
                f,
                "the index is {bytes} bytes, more than the limit of {limit}"
            ),
            Self::Io(message) => write!(f, "cannot read the index: {message}"),
            Self::Parse(message) => write!(f, "not a SCIP index: {}", printable(message, 200)),
        }
    }
}

impl std::error::Error for Error {}

/// `text` from an index (or from a parser reading one), safe to print: control
/// characters escaped, at most `limit` characters. Every string that comes from
/// an index passes through here before it can reach standard error.
pub(crate) fn printable(text: &str, limit: usize) -> String {
    text.escape_debug().take(limit).collect()
}
