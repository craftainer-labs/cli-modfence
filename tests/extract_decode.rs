//! The decoding policy (`NC6f`, ADR 0003 invariant 23): a file that is not valid
//! UTF-8 is still extracted, from its lossy decoding, and gets one
//! `unsupported-encoding` diagnostic.

use modfence::extract::decode::extract;
use modfence::extract::python::PythonExtractor;
use modfence::extract::{DiagnosticCode, NodeKind, SourceFile};

fn error(bytes: &[u8]) -> std::str::Utf8Error {
    std::str::from_utf8(bytes).unwrap_err()
}

#[test]
fn an_undecodable_file_is_extracted_and_names_the_first_bad_byte() {
    let bytes = b"import os\nx = '\xe9'\n";
    let error = error(bytes);
    let text = String::from_utf8_lossy(bytes);
    let file = SourceFile {
        path: "pkg/a.py",
        text: &text,
    };
    let facts = extract(&PythonExtractor, &file, Some(error));
    assert!(facts.nodes.iter().any(|n| n.kind == NodeKind::Module));
    assert!(!facts.imports.is_empty(), "{facts:?}");
    let diagnostic = facts
        .diagnostics
        .iter()
        .find(|d| d.code == DiagnosticCode::UnsupportedEncoding)
        .expect("an unsupported-encoding diagnostic");
    assert!(
        diagnostic.message.contains("offset 15"),
        "{}",
        diagnostic.message
    );
    assert_eq!(diagnostic.span, None);
}

#[test]
fn a_decodable_file_gets_no_encoding_diagnostic() {
    let file = SourceFile {
        path: "a.py",
        text: "import os\n",
    };
    let facts = extract(&PythonExtractor, &file, None);
    assert!(
        facts
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::UnsupportedEncoding)
    );
}
