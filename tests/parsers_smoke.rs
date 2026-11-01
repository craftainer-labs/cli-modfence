//! The tree-sitter core accepts each grammar crate (ABI match) and each
//! parses a one-line snippet without error nodes.

use tree_sitter::{Language, Parser};

/// Parses `source` with `language` and asserts the tree has no error nodes.
fn assert_parses(language: &Language, source: &str) {
    let mut parser = Parser::new();
    parser
        .set_language(language)
        .expect("grammar ABI is loadable by the pinned tree-sitter");
    let tree = parser.parse(source, None).expect("parse returns a tree");
    assert!(
        !tree.root_node().has_error(),
        "error node in {source:?}: {}",
        tree.root_node().to_sexp()
    );
}

#[test]
fn python_parses() {
    assert_parses(&tree_sitter_python::LANGUAGE.into(), "import os\n");
}

#[test]
fn typescript_parses() {
    assert_parses(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "import { a } from './a';\n",
    );
}

#[test]
fn tsx_parses() {
    assert_parses(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        "const x = <div>{a}</div>;\n",
    );
}

#[test]
fn rust_parses() {
    assert_parses(&tree_sitter_rust::LANGUAGE.into(), "use std::fmt;\n");
}
