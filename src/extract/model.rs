//! The types of interface I3: what an extractor receives and what it returns.
//!
//! Extractors report what one file says and nothing more: unresolved import
//! specifiers, call text, nodes, annotations and binding exports, each with a
//! span. The graph core resolves them, rolls them up and computes tags (ADR
//! 0003 and ADR 0004).

use super::annotations::{AnnotationError, Parsed, parse_comment};
use super::ident::{NodeError, NodeId, decode_component};

/// A source language. JavaScript files are `Ts`: they go through the
/// TypeScript grammar (ADR 0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    /// Python (`.py`).
    Py,
    /// TypeScript and JavaScript (`.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`,
    /// `.mjs`, `.cjs`).
    Ts,
    /// Rust (`.rs`).
    Rs,
}

impl Lang {
    /// The language code used in ids and the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Py => "py",
            Self::Ts => "ts",
            Self::Rs => "rs",
        }
    }

    /// The language of a file extension (without the dot, case-sensitive).
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            "py" => Some(Self::Py),
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::Ts),
            "rs" => Some(Self::Rs),
            _ => None,
        }
    }

    /// The language of a `/`-separated path, by its file extension. A file
    /// whose name is only an extension (`.py`) has none.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or_default();
        let (stem, extension) = name.rsplit_once('.')?;
        if stem.is_empty() {
            None
        } else {
            Self::from_extension(extension)
        }
    }
}

/// The kind of a graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// A directory with modules, or a Rust crate. Built by the core.
    Package,
    /// One source file. Extractors report one per file.
    Module,
    /// A function, class or similar inside a module.
    Symbol,
    /// A third-party or standard-library package. Built by the core.
    External,
}

impl NodeKind {
    /// The id prefix (`pkg`, `mod`, `sym` or `ext`).
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Package => "pkg",
            Self::Module => "mod",
            Self::Symbol => "sym",
            Self::External => "ext",
        }
    }

    /// The `kind` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Module => "module",
            Self::Symbol => "symbol",
            Self::External => "external",
        }
    }
}

/// What a symbol node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolKind {
    /// A function.
    Function,
    /// A method.
    Method,
    /// A class.
    Class,
    /// A struct.
    Struct,
    /// An enum.
    Enum,
    /// An interface.
    Interface,
    /// A type alias.
    Type,
    /// An `impl` block.
    Impl,
    /// A trait.
    Trait,
    /// A constant.
    Constant,
    /// A variable.
    Variable,
}

impl SymbolKind {
    /// The `symbol_kind` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Type => "type",
            Self::Impl => "impl",
            Self::Trait => "trait",
            Self::Constant => "constant",
            Self::Variable => "variable",
        }
    }
}

/// A source range: 1-based lines and columns, columns counted in Unicode code
/// points, the end exclusive (ADR 0003, "Positions").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    /// First line, from 1.
    pub start_line: u32,
    /// First column, from 1.
    pub start_column: u32,
    /// Last line, from 1.
    pub end_line: u32,
    /// Column after the last character, from 1.
    pub end_column: u32,
}

impl Span {
    /// A span, or `None` when a line or column is 0 or the end lies before the
    /// start.
    #[must_use]
    pub const fn new(
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    ) -> Option<Self> {
        let same_line = end_line == start_line;
        let ordered = end_line > start_line || (same_line && end_column >= start_column);
        if start_line == 0 || start_column == 0 || end_line == 0 || end_column == 0 || !ordered {
            None
        } else {
            Some(Self {
                start_line,
                start_column,
                end_line,
                end_column,
            })
        }
    }
}

/// The 1-based column, in code points, of byte offset `byte_offset` in `line`
/// (tree-sitter reports byte columns). The offset of the line's end is valid
/// (the column after the last character); `None` when the offset is beyond it
/// or inside a multi-byte character.
#[must_use]
pub fn column_from_byte(line: &str, byte_offset: usize) -> Option<u32> {
    let before = line.get(..byte_offset)?;
    u32::try_from(before.chars().count() + 1).ok()
}

/// A node an extractor found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The node's id.
    pub id: NodeId,
    /// Its kind: `Module` or `Symbol` from an extractor.
    pub kind: NodeKind,
    /// Its language.
    pub lang: Lang,
    /// A human-readable name (non-empty).
    pub label: String,
    /// The file, relative to the analysis root, with `/` separators.
    pub path: String,
    /// What the symbol is; `Some` exactly for symbols.
    pub symbol_kind: Option<SymbolKind>,
    /// Where the symbol is; `Some` exactly for symbols.
    pub span: Option<Span>,
}

impl Node {
    /// The module node of the file at `path`; its label is the path.
    ///
    /// # Errors
    ///
    /// [`NodeError::EmptyPath`] when `path` is empty.
    pub fn module(lang: Lang, path: &str) -> Result<Self, NodeError> {
        Ok(Self {
            id: NodeId::new(NodeKind::Module, lang, path, None)?,
            kind: NodeKind::Module,
            lang,
            label: path.to_owned(),
            path: path.to_owned(),
            symbol_kind: None,
            span: None,
        })
    }

    /// A symbol node in the file at `path`. `qualname` is the symbol's
    /// qualified name in the language's own encoding, already percent-encoded
    /// (ADR 0003 leaves that encoding to each extractor); it is validated, not
    /// changed.
    ///
    /// # Errors
    ///
    /// [`NodeError`] when the path or the label is empty, or the qualified name
    /// is not valid encoded text.
    pub fn symbol(
        lang: Lang,
        path: &str,
        qualname: &str,
        label: &str,
        symbol_kind: SymbolKind,
        span: Span,
    ) -> Result<Self, NodeError> {
        if label.is_empty() {
            return Err(NodeError::EmptyLabel);
        }
        Ok(Self {
            id: NodeId::new(NodeKind::Symbol, lang, path, Some(qualname))?,
            kind: NodeKind::Symbol,
            lang,
            label: label.to_owned(),
            path: path.to_owned(),
            symbol_kind: Some(symbol_kind),
            span: Some(span),
        })
    }

    /// The qualified name decoded for display, for symbols.
    #[must_use]
    pub fn display_qualname(&self) -> Option<String> {
        let encoded = self.id.qualname()?;
        Some(decode_component(encoded).unwrap_or_else(|| encoded.to_owned()))
    }
}

/// An import as written, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdge {
    /// The module specifier as written: `os.path`, `..pkg.mod` (leading dots
    /// kept), `./util`, `@scope/pkg`, `crate::a::b`.
    pub specifier: String,
    /// Names imported from the specifier (`from m import a, b`, `use m::{a,
    /// b}`, `import { a } from "m"`); empty when the whole module is imported.
    pub names: Vec<String>,
    /// The import statement or expression. Rules and suppressions use the
    /// start line.
    pub span: Span,
}

/// A call site, before resolution. Tree-sitter call edges are always
/// heuristic (ADR 0003, invariant 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// The callee text as written, without the receiver (`bar` in `a.bar()`).
    pub callee: String,
    /// The receiver text as written (`a` in `a.bar()`), if any.
    pub receiver: Option<String>,
    /// The enclosing symbol, or `None` for a call at module level.
    pub caller: Option<NodeId>,
    /// The span of the callee name, which is what a SCIP reference is matched
    /// against (ADR 0003, invariant 8).
    pub span: Span,
}

/// An annotation comment found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// The comment.
    pub span: Span,
    /// The symbol the comment sits directly above (ADR 0004: no blank line
    /// between, decorators, attributes and doc comments allowed between), or
    /// `None` to tag the module. [`Annotation::from_comment`] forces it to
    /// `None` for a suppression.
    pub target: Option<NodeId>,
    /// The parsed annotation, or why it is malformed.
    pub parsed: Result<Parsed, AnnotationError>,
}

impl Annotation {
    /// Parses a comment body (markers stripped, see
    /// [`comment_body`](super::annotations::comment_body)). `None` when the
    /// comment is not an annotation.
    #[must_use]
    pub fn from_comment(body: &str, span: Span, target: Option<NodeId>) -> Option<Self> {
        let parsed = parse_comment(body)?;
        let target = if matches!(parsed, Ok(Parsed::Ignore(_))) {
            None
        } else {
            target
        };
        Some(Self {
            span,
            target,
            parsed,
        })
    }
}

/// A foreign-function framework that exports a symbol across languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BindingFramework {
    /// `PyO3` (`#[pyfunction]`, `#[pymodule]`).
    Pyo3,
    /// napi (`#[napi]`).
    Napi,
    /// wasm-bindgen (`#[wasm_bindgen]`).
    WasmBindgen,
}

impl BindingFramework {
    /// The `framework` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pyo3 => "pyo3",
            Self::Napi => "napi",
            Self::WasmBindgen => "wasm-bindgen",
        }
    }
}

/// A symbol exported across the language boundary. Matching it to Python or
/// TypeScript imports and calls is by `exported_name` only (ADR 0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingExport {
    /// The exporting symbol.
    pub symbol: NodeId,
    /// The framework.
    pub framework: BindingFramework,
    /// The name the other language sees.
    pub exported_name: String,
    /// The attribute or declaration.
    pub span: Span,
}

/// Why a file produced a diagnostic; the values of the schema's `code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticCode {
    /// The file has syntax errors (facts were recovered where possible).
    ParseError,
    /// The file could not be read.
    Unreadable,
    /// The file is not valid UTF-8.
    UnsupportedEncoding,
    /// The file is over the size limit.
    TooLarge,
    /// An import that looks internal could not be placed. Set by the core.
    UnresolvedImport,
}

impl DiagnosticCode {
    /// The `code` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParseError => "parse-error",
            Self::Unreadable => "unreadable",
            Self::UnsupportedEncoding => "unsupported-encoding",
            Self::TooLarge => "too-large",
            Self::UnresolvedImport => "unresolved-import",
        }
    }
}

/// A problem with one file. Analysis continues past it (ADR 0003, invariant 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// What kind of problem.
    pub code: DiagnosticCode,
    /// A message for people; it must not contain absolute paths.
    pub message: String,
    /// Where, when known.
    pub span: Option<Span>,
}

/// One file's source, as the core read and decoded it.
#[derive(Debug, Clone, Copy)]
pub struct SourceFile<'a> {
    /// The path relative to the analysis root, with `/` separators.
    pub path: &'a str,
    /// The decoded text.
    pub text: &'a str,
}

/// Extraction context. It carries no settings yet; fields are added here
/// without changing the [`Extractor`] signature.
#[derive(Debug, Clone, Default)]
pub struct ExtractCtx {}

/// Everything one file says. Nothing here is resolved or folded: that is the
/// core's job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// The module node first, then the file's symbol nodes in source order.
    pub nodes: Vec<Node>,
    /// Imports, in source order.
    pub imports: Vec<ImportEdge>,
    /// Rust only: the `mod name;` declarations of the file (not inline
    /// modules), each as the inline modules it sits in and then the declared
    /// name. A declaration also has an entry in `imports`; this list is what
    /// tells it apart from a `use` of the same path (`use self::a::{self}`).
    pub mod_declarations: Vec<Vec<String>>,
    /// Call sites, in source order.
    pub calls: Vec<CallSite>,
    /// `modfence:` comments, in source order.
    pub annotations: Vec<Annotation>,
    /// Symbols exported across the language boundary.
    pub bindings: Vec<BindingExport>,
    /// Problems found while extracting.
    pub diagnostics: Vec<Diagnostic>,
}

/// A per-language extractor: a tree-sitter grammar plus the walk that turns a
/// syntax tree into [`FileFacts`]. Implementations are stateless and shared
/// between threads. A file that fails to parse yields a
/// [`DiagnosticCode::ParseError`] diagnostic plus whatever facts were
/// recovered; `extract` does not fail.
pub trait Extractor: Send + Sync {
    /// The language this extractor reads.
    fn language(&self) -> Lang;

    /// Identifies the extraction logic and grammar. The per-file cache is keyed
    /// by content hash and this version (ADR 0003), so change it whenever
    /// output could change.
    fn version(&self) -> &'static str;

    /// Whether this extractor reads the file at `path` (`/`-separated). The
    /// default is by file extension.
    fn handles(&self, path: &str) -> bool {
        Lang::from_path(path) == Some(self.language())
    }

    /// Reads one file.
    fn extract(&self, file: &SourceFile<'_>, ctx: &ExtractCtx) -> FileFacts;
}
