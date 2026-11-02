//! The Rust extractor (interface I3): a tree-sitter walk that reports a file's
//! module and symbol nodes, imports, call sites, binding exports and
//! `modfence:` annotations, unresolved (ADR 0003 and ADR 0004).
//!
//! What it reports:
//!
//! - nodes: the module (`mod:rs:<path>`), then a symbol for every `fn`,
//!   `struct`, `union`, `enum`, `trait`, `impl`, `type`, `const` and `static`,
//!   nested ones included, in source order. A `fn` inside an `impl` or a
//!   `trait` is a method; a `fn` without a body (a trait method, an `extern`
//!   function) is a symbol too. Inline modules (`mod a { .. }`) are not
//!   symbols: they add a component to the qualified names of what they hold.
//!   Enum variants, fields, associated types, macros and closures are not
//!   symbols; a call inside a closure belongs to the enclosing symbol.
//! - imports: one edge per module path a `use` takes names from, with the path
//!   as written (spaces removed) and the names taken (`*` for a glob). A
//!   `self` in a list, or a path that names a module only, is an edge with no
//!   names. A `mod name;` declaration is an edge to `self::name` (the file
//!   module declared there), and `extern crate name;` an edge to `name`. A
//!   leading `self` or `super` is rewritten to be relative to the file's
//!   module, whatever inline modules the statement sits in (`super::x` in
//!   `mod a` is `self::x`; see ADR 0003).
//! - calls: calls whose callee is a name (`f()`), a path (`Type::new()`,
//!   `a::b::f()`, `f::<T>()`) or a field (`x.run()`), with the span of the last
//!   name. The receiver of a path call is the path before the name. Calls on
//!   other expressions (`f()()`) and macro invocations are skipped. The caller
//!   is the innermost enclosing symbol.
//! - annotations: `modfence:` comments. An own-line comment directly above a
//!   symbol (attributes and further comments may sit between) targets it;
//!   every other comment has no target.
//! - bindings: items marked `#[pyfunction]`, `#[pymodule]`, `#[napi]` or
//!   `#[wasm_bindgen]`, with the exported name (`name = ".."` in a `#[pyo3]`
//!   attribute, `js_name = ".."` for the others, else the item's name).
//!
//! Qualified names are encoded by [`encode_qualname`] and decoded by
//! [`decode_qualname`].

mod qualname;
mod uses;
mod walk;

pub use qualname::{LOCALS, decode_qualname, encode_qualname};

use tree_sitter::{Parser, Tree};

use super::{Diagnostic, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, Node, SourceFile};

/// Identifies the extraction logic and the grammar for the per-file cache.
/// Change it whenever the output could change.
const VERSION: &str = "rs-1/tree-sitter-rust-0.24.2";

/// The Rust extractor.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustExtractor;

fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    parser.parse(text, None)
}

impl Extractor for RustExtractor {
    fn language(&self) -> Lang {
        Lang::Rs
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn extract(&self, file: &SourceFile<'_>, _ctx: &ExtractCtx) -> FileFacts {
        let mut facts = FileFacts::default();
        match Node::module(Lang::Rs, file.path) {
            Ok(module) => facts.nodes.push(module),
            Err(error) => {
                facts.diagnostics.push(Diagnostic {
                    code: DiagnosticCode::Unreadable,
                    message: error.to_string(),
                    span: None,
                });
                return facts;
            }
        }
        let tree = parse(file.text);
        walk::Walker::new(file, facts).run(tree.as_ref())
    }
}
