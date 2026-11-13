//! The decoding policy (NC6f, ADR 0003 invariant 23): a file that is not valid
//! UTF-8 yields its module node and one `unsupported-encoding` diagnostic.

use modfence::extract::decode::undecodable;
use modfence::extract::{DiagnosticCode, Lang, NodeKind};

fn error(bytes: &[u8]) -> std::str::Utf8Error {
    std::str::from_utf8(bytes).unwrap_err()
}

#[test]
fn an_undecodable_file_keeps_its_module_and_names_the_first_bad_byte() {
    let facts = undecodable(Lang::Py, "pkg/a.py", error(b"ab\xe9cd"));
    assert_eq!(facts.nodes.len(), 1);
    assert_eq!(facts.nodes[0].kind, NodeKind::Module);
    assert_eq!(facts.diagnostics.len(), 1);
    let diagnostic = &facts.diagnostics[0];
    assert_eq!(diagnostic.code, DiagnosticCode::UnsupportedEncoding);
    assert!(diagnostic.message.contains("offset 2"), "{}", diagnostic.message);
    assert_eq!(diagnostic.span, None);
    assert!(facts.calls.is_empty() && facts.imports.is_empty());
}

#[test]
fn an_undecodable_file_without_a_path_is_unreadable() {
    let facts = undecodable(Lang::Rs, "", error(b"\xff"));
    assert!(facts.nodes.is_empty());
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].code, DiagnosticCode::Unreadable);
}
