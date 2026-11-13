//! Interface I3: what a per-language extractor receives and returns, plus the
//! shared parser for `modfence:` annotation comments.
//!
//! - [`model`]: [`Extractor`], [`SourceFile`], [`ExtractCtx`], [`FileFacts`] and
//!   the facts inside it, [`Span`] and [`Lang`].
//! - [`ident`]: [`NodeId`] construction, with the percent-encoding and the
//!   pattern of `schema/modfence-graph.schema.json`. The qualified name of a
//!   symbol is encoded by each extractor, which amends ADR 0003 with its
//!   language's rules; [`NodeId::new`] validates it.
//! - [`annotations`]: [`parse_comment`] and [`comment_body`], the grammar of
//!   ADR 0004's inline annotations.
//!
//! - [`decode`]: [`decode::extract`], the facts of a file, with a diagnostic when
//!   it is not valid UTF-8 (it is then analysed from a lossy decoding).
//!
//! The per-language extractors live in the `python`, `typescript` and `rust`
//! submodules, added by their own items.

pub mod annotations;
pub mod decode;
pub mod ident;
pub mod model;
pub mod python;
pub mod rust;
pub mod typescript;

pub use annotations::{
    AnnotationError, ErrorKind, PREFIX, Parsed, RESERVED_KEYS, Suppression, Tags, comment_body,
    merge_tags, parse_comment,
};
pub use ident::{NodeError, NodeId, decode_component, encode_component, is_encoded};
pub use model::{
    Annotation, BindingExport, BindingFramework, CallSite, Diagnostic, DiagnosticCode, ExtractCtx,
    Extractor, FileFacts, ImportEdge, Lang, Node, NodeKind, SourceFile, Span, SymbolKind,
    column_from_byte,
};
