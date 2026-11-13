//! The decoding policy for source files (ADR 0003, invariant 23).
//!
//! A file whose bytes are not valid UTF-8 is still extracted, from its lossy
//! decoding, so its imports and calls stay in the graph and the rules see them.
//! Decoding is lossy, though: U+FFFD has a different width from the byte it
//! replaced in every encoding, so a position after the first invalid byte can
//! be off, and a SCIP range computed on the original bytes would drift from
//! the text the extractors read. Such a file therefore gets one
//! [`DiagnosticCode::UnsupportedEncoding`] diagnostic, and the caller gives
//! SCIP no text for it.

use std::str::Utf8Error;

use super::model::{Diagnostic, DiagnosticCode, ExtractCtx, Extractor, FileFacts, SourceFile};

/// The facts of `file` from `extractor`. When `error` says the file's bytes
/// were not valid UTF-8 (`file.text` is then their lossy decoding), the facts
/// also carry an [`DiagnosticCode::UnsupportedEncoding`] diagnostic naming the
/// offset of the first invalid byte.
#[must_use]
pub fn extract(
    extractor: &dyn Extractor,
    file: &SourceFile<'_>,
    error: Option<Utf8Error>,
) -> FileFacts {
    let mut facts = extractor.extract(file, &ExtractCtx::default());
    if let Some(error) = error {
        facts.diagnostics.push(Diagnostic {
            code: DiagnosticCode::UnsupportedEncoding,
            message: format!(
                "the file is not valid UTF-8 (first invalid byte at offset {}); it was analysed \
                 with replacement characters, positions after that byte may be off, and \
                 --precise is skipped for this file",
                error.valid_up_to()
            ),
            span: None,
        });
    }
    facts
}
