//! The Python extractor (interface I3): a tree-sitter walk that reports a
//! file's module and symbol nodes, imports, call sites and `modfence:`
//! annotations, unresolved (ADR 0003 and ADR 0004).
//!
//! What it reports:
//!
//! - nodes: the module (`mod:py:<path>`), then a symbol for every `def` and
//!   `class`, nested ones included, in source order. Lambdas are not symbols;
//!   a call inside a lambda belongs to the enclosing symbol. Module-level
//!   variables are not symbols either.
//! - imports: one edge per `import` target and one per `from` statement, with
//!   the specifier as written (leading dots kept, spaces removed) and, for
//!   `from`, the imported names (`*` for a star import). `from __future__`
//!   imports are not reported.
//! - calls: calls whose callee is a name (`f()`) or an attribute (`a.b()`),
//!   with the span of the name. Calls on the result of another expression
//!   (`f()()`, `x[0]()`) are skipped. The caller is the innermost enclosing
//!   symbol; decorators, default values and base classes belong to the scope
//!   the definition sits in.
//! - annotations: `modfence:` comments. An own-line comment directly above a
//!   symbol (decorators and further comments may sit between) targets it;
//!   every other comment has no target.
//!
//! Qualified names are encoded by [`encode_qualname`] and decoded by
//! [`decode_qualname`].

mod qualname;
mod walk;

pub use qualname::{LOCALS, decode_qualname, encode_qualname};

use tree_sitter::{Parser, Tree};

use super::{
    Diagnostic, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, Node, SourceFile,
};

/// Identifies the extraction logic and the grammar for the per-file cache.
/// Change it whenever the output could change.
const VERSION: &str = "py-1/tree-sitter-python-0.25.0";

/// The Python extractor.
#[derive(Debug, Clone, Copy, Default)]
pub struct PythonExtractor;

fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    parser.parse(text, None)
}

impl Extractor for PythonExtractor {
    fn language(&self) -> Lang {
        Lang::Py
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn extract(&self, file: &SourceFile<'_>, _ctx: &ExtractCtx) -> FileFacts {
        let mut facts = FileFacts::default();
        match Node::module(Lang::Py, file.path) {
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
