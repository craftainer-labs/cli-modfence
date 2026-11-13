//! The decoding policy for source files (ADR 0003, invariant 23).
//!
//! A file is analysed only when it is valid UTF-8. Decoding a file lossily
//! would replace each invalid byte with U+FFFD, which has a different width
//! from the byte it replaced in every encoding, so a position computed on the
//! original bytes (a SCIP range) would drift from the text the extractors read.
//! Such a file is skipped instead: it keeps its module node and gets one
//! [`DiagnosticCode::UnsupportedEncoding`] diagnostic, and analysis continues.

use std::str::Utf8Error;

use super::model::{Diagnostic, DiagnosticCode, FileFacts, Lang, Node};

/// The facts of a file whose bytes are not valid UTF-8: its module node and an
/// [`DiagnosticCode::UnsupportedEncoding`] diagnostic naming the offset of the
/// first invalid byte. A path that cannot name a module is
/// [`DiagnosticCode::Unreadable`] instead, as in the extractors.
#[must_use]
pub fn undecodable(lang: Lang, path: &str, error: Utf8Error) -> FileFacts {
    let mut facts = FileFacts::default();
    match Node::module(lang, path) {
        Ok(module) => {
            facts.nodes.push(module);
            facts.diagnostics.push(Diagnostic {
                code: DiagnosticCode::UnsupportedEncoding,
                message: format!(
                    "the file is not valid UTF-8 (first invalid byte at offset {}); it was skipped",
                    error.valid_up_to()
                ),
                span: None,
            });
        }
        Err(error) => facts.diagnostics.push(Diagnostic {
            code: DiagnosticCode::Unreadable,
            message: error.to_string(),
            span: None,
        }),
    }
    facts
}
