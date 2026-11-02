//! The TypeScript and JavaScript extractor (interface I3, ADR 0003).
//!
//! JavaScript goes through the TypeScript grammar and counts as language `ts`.
//! `.ts`, `.mts` and `.cts` files use the TypeScript grammar; every other
//! extension (`.tsx`, `.jsx`, `.js`, `.mjs`, `.cjs`) uses the TSX grammar, which
//! also reads plain JavaScript and JSX.
//!
//! - [`TsExtractor`]: the [`Extractor`] implementation.
//! - [`encode_segment`], [`join`], [`split`] and [`render`]: the encoding of
//!   qualified names, specified in ADR 0003.

mod qualname;
mod walker;

use tree_sitter::{Language, Parser, Tree};

pub use qualname::{Segment, Sep, encode_segment, join, render, split};

use super::{ExtractCtx, Extractor, FileFacts, Lang, SourceFile};

/// The extractor's version, part of the per-file cache key: change it whenever
/// the output for some input could change, and with the grammar crate.
pub const VERSION: &str = "typescript-1/tree-sitter-typescript-0.23.2";

/// Reads TypeScript and JavaScript files.
#[derive(Debug, Clone, Copy, Default)]
pub struct TsExtractor;

impl Extractor for TsExtractor {
    fn language(&self) -> Lang {
        Lang::Ts
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn extract(&self, file: &SourceFile<'_>, _ctx: &ExtractCtx) -> FileFacts {
        walker::extract(file, parse(file).as_ref())
    }
}

/// The grammar for a file: TypeScript for `.ts`, `.mts` and `.cts`, TSX for the
/// rest.
fn grammar(path: &str) -> Language {
    match path.rsplit_once('.') {
        Some((_, "ts" | "mts" | "cts")) => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

fn parse(file: &SourceFile<'_>) -> Option<Tree> {
    let mut parser = Parser::new();
    let language = parser.set_language(&grammar(file.path));
    language.ok().and_then(|()| parser.parse(file.text, None))
}
