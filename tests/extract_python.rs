//! The Python extractor (interface I3): the fixture repo's facts, annotation
//! targets, the qualified-name round trip and parse errors.

use std::path::Path;

use modfence::extract::python::{LOCALS, PythonExtractor, decode_qualname, encode_qualname};
use modfence::extract::{
    Annotation, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, NodeId, NodeKind, Parsed,
    SourceFile, Span, SymbolKind, decode_component,
};

fn extract(path: &str, text: &str) -> FileFacts {
    PythonExtractor.extract(&SourceFile { path, text }, &ExtractCtx::default())
}

/// The text of a file of the fixture repo, `tests/fixtures/python/<path>`.
fn fixture(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/python")
        .join(path);
    std::fs::read_to_string(file).expect("the fixture file is readable")
}

fn extract_fixture(path: &str) -> FileFacts {
    extract(path, &fixture(path))
}

fn qualname(id: &NodeId) -> &str {
    id.qualname().expect("a symbol id has a qualified name")
}

/// The span of the `#` comment on 1-based `line` of `text`, to the line's end.
fn comment_span(text: &str, line: u32) -> Span {
    let row = usize::try_from(line - 1).unwrap();
    let source = text.lines().nth(row).unwrap();
    let start = u32::try_from(source.find('#').unwrap() + 1).unwrap();
    let end = u32::try_from(source.chars().count() + 1).unwrap();
    Span::new(line, start, line, end).unwrap()
}

fn tag_values(annotation: &Annotation, key: &str) -> Vec<String> {
    match &annotation.parsed {
        Ok(Parsed::Tags(tags)) => tags[key].iter().cloned().collect(),
        other => panic!("expected tags, got {other:?}"),
    }
}

/// The encoded qualified names of the symbols, in order.
fn symbol_names(facts: &FileFacts) -> Vec<&str> {
    let symbols = facts.nodes.iter().skip(1);
    symbols.map(|node| qualname(&node.id)).collect()
}

fn target_name(annotation: &Annotation) -> Option<&str> {
    annotation.target.as_ref().map(qualname)
}

#[test]
fn extractor_identity() {
    assert_eq!(PythonExtractor.language(), Lang::Py);
    let version = PythonExtractor.version();
    assert!(version.contains("tree-sitter-python-0.25.0"));
    assert!(PythonExtractor.handles("pkg/core.py"));
    assert!(!PythonExtractor.handles("pkg/core.ts"));
    assert!(!PythonExtractor.handles("pkg/core.pyi"));
}

#[test]
fn fixture_package_init() {
    let text = fixture("pkg/__init__.py");
    let facts = extract("pkg/__init__.py", &text);
    assert_eq!(facts.nodes.len(), 1);
    let module = &facts.nodes[0];
    assert_eq!(module.id.as_str(), "mod:py:pkg/__init__.py");
    assert_eq!(module.kind, NodeKind::Module);
    let imports: Vec<_> = facts
        .imports
        .iter()
        .map(|edge| {
            let names: Vec<&str> = edge.names.iter().map(String::as_str).collect();
            (edge.specifier.as_str(), names, edge.span.start_line)
        })
        .collect();
    assert_eq!(
        imports,
        [
            (".core", vec!["Engine", "run_all"], 3),
            (".", vec!["helpers"], 4),
            ("..", vec!["sibling"], 5),
            ("pkg.core", vec![], 6),
        ]
    );
    assert_eq!(facts.calls.len(), 0);
    assert_eq!(facts.diagnostics.len(), 0);
    assert_eq!(facts.annotations.len(), 1);
    let header = &facts.annotations[0];
    assert_eq!(header.target, None, "a blank line follows the header");
    assert_eq!(header.span, comment_span(&text, 1));
    assert_eq!(tag_values(header, "layer"), ["public"]);
}

#[test]
fn fixture_helpers_module() {
    let facts = extract_fixture("pkg/helpers.py");
    let ids: Vec<&str> = facts.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "mod:py:pkg/helpers.py",
            "sym:py:pkg/helpers.py#go",
            "sym:py:pkg/helpers.py#stop",
        ]
    );
    assert_eq!(facts.calls.len(), 1);
    let call = &facts.calls[0];
    assert_eq!(call.callee, "stop");
    assert_eq!(call.receiver, None);
    assert_eq!(call.caller.as_ref().map(qualname), Some("go"));
    assert_eq!(call.span, Span::new(2, 12, 2, 16).unwrap());
}

#[test]
fn fixture_core_nodes() {
    let facts = extract_fixture("pkg/core.py");
    let nodes: Vec<_> = facts
        .nodes
        .iter()
        .map(|node| {
            let span = node.span.map(|span| (span.start_line, span.start_column));
            (node.id.as_str(), node.symbol_kind, span)
        })
        .collect();
    let class = Some(SymbolKind::Class);
    let method = Some(SymbolKind::Method);
    let function = Some(SymbolKind::Function);
    let inner = "sym:py:pkg/core.py#Engine.run.<locals>.inner";
    let local = "sym:py:pkg/core.py#Engine.run.<locals>.inner.<locals>.Local";
    let deep = "sym:py:pkg/core.py#Engine.run.<locals>.inner.<locals>.Local.deep";
    let at = |line, column| Some((line, column));
    assert_eq!(
        nodes,
        [
            ("mod:py:pkg/core.py", None, None),
            ("sym:py:pkg/core.py#Engine", class, at(15, 1)),
            ("sym:py:pkg/core.py#Engine.make", method, at(20, 5)),
            ("sym:py:pkg/core.py#Engine.run", method, at(23, 5)),
            (inner, function, at(24, 9)),
            (local, class, at(25, 13)),
            (deep, method, at(26, 17)),
            ("sym:py:pkg/core.py#Engine.size", method, at(35, 5)),
            ("sym:py:pkg/core.py#Engine.size@2", method, at(39, 5)),
            ("sym:py:pkg/core.py#Engine.Config", class, at(42, 5)),
            ("sym:py:pkg/core.py#run_all", function, at(48, 1)),
            ("sym:py:pkg/core.py#run_all@2", function, at(52, 1)),
        ]
    );
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn fixture_core_imports() {
    let facts = extract_fixture("pkg/core.py");
    let imports: Vec<_> = facts
        .imports
        .iter()
        .map(|edge| {
            let names: Vec<&str> = edge.names.iter().map(String::as_str).collect();
            (
                edge.specifier.as_str(),
                names,
                edge.span.start_line,
                edge.span.end_line,
            )
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("os", vec![], 2, 2),
            ("os.path", vec![], 3, 3),
            ("sys", vec![], 3, 3),
            ("collections", vec!["OrderedDict", "defaultdict"], 4, 7),
            (".helpers", vec!["*"], 8, 8),
            ("..other.deep", vec!["thing"], 9, 9),
        ]
    );
}

#[test]
fn fixture_core_calls() {
    let facts = extract_fixture("pkg/core.py");
    let calls: Vec<_> = facts
        .calls
        .iter()
        .map(|call| {
            (
                call.callee.as_str(),
                call.receiver.as_deref(),
                call.caller.as_ref().map(qualname),
                call.span.start_line,
            )
        })
        .collect();
    let deep = "Engine.run.<locals>.inner.<locals>.Local.deep";
    assert_eq!(
        calls,
        [
            ("register", None, None, 14),
            ("Engine", None, Some("Engine.make"), 21),
            ("default_mode", None, Some("Engine"), 23),
            ("go", Some("helpers"), Some(deep), 27),
            ("step", Some("self"), Some("Engine.run"), 31),
            ("finish", Some("self.step(1)"), Some("Engine.run"), 31),
            ("len", None, Some("Engine.Config"), 43),
            ("go", Some("helpers"), Some("run_all"), 49),
            ("str", None, Some("run_all"), 49),
            ("run_all", None, None, 56),
            ("make", None, None, 57),
        ]
    );
    let register = &facts.calls[0];
    assert_eq!(register.span, Span::new(14, 2, 14, 10).unwrap());
}

#[test]
fn fixture_core_annotations() {
    let text = fixture("pkg/core.py");
    let facts = extract("pkg/core.py", &text);
    let found: Vec<_> = facts
        .annotations
        .iter()
        .map(|annotation| (annotation.span, target_name(annotation)))
        .collect();
    assert_eq!(
        found,
        [
            (comment_span(&text, 12), Some("Engine")),
            (comment_span(&text, 18), Some("Engine.make")),
            (comment_span(&text, 46), None),
            (comment_span(&text, 56), None),
        ]
    );
    assert_eq!(tag_values(&facts.annotations[0], "layer"), ["domain"]);
    assert_eq!(tag_values(&facts.annotations[1], "owner"), ["core"]);
    assert_eq!(tag_values(&facts.annotations[2], "layer"), ["orphan"]);
    match &facts.annotations[3].parsed {
        Ok(Parsed::Ignore(suppression)) => {
            assert_eq!(suppression.rules, ["layers"]);
            assert_eq!(suppression.reason.as_deref(), Some("entry"));
        }
        other => panic!("expected a suppression, got {other:?}"),
    }
}

#[test]
fn fixture_symbol_ids_round_trip() {
    for path in ["pkg/core.py", "pkg/helpers.py"] {
        for node in extract_fixture(path).nodes.iter().skip(1) {
            let encoded = qualname(&node.id);
            let (components, ordinal) = decode_qualname(encoded).unwrap();
            assert_eq!(encode_qualname(&components, ordinal), encoded);
            assert_eq!(node.label, components.join("."));
            let shown = decode_component(encoded).unwrap();
            assert_eq!(node.display_qualname(), Some(shown));
            assert_eq!(node.id.path(), path);
        }
    }
}

#[test]
fn annotation_targets() {
    let text = "\
# modfence: a=1
def top(): pass

# modfence: b=2
@dec
# modfence: c=3
def mid(): pass
# modfence: d=4

def blank(): pass
class K:
    # modfence: e=5
    def m(self): pass  # modfence: f=6
# modfence: ignore[x] reason=\"r\"
def ign(): pass
# see modfence: nothing
# modfence: g=7
";
    let facts = extract("m.py", text);
    let found: Vec<_> = facts
        .annotations
        .iter()
        .map(|annotation| (annotation.span.start_line, target_name(annotation)))
        .collect();
    assert_eq!(
        found,
        [
            (1, Some("top")),
            (4, Some("mid")),
            (6, Some("mid")),
            (8, None),
            (12, Some("K.m")),
            (13, None),
            (14, None),
            (17, None),
        ]
    );
}

#[test]
fn annotation_without_a_line_below() {
    let facts = extract("m.py", "def f(): pass\n# modfence: a=1");
    assert_eq!(facts.annotations.len(), 1);
    assert_eq!(facts.annotations[0].target, None);
}

#[test]
fn comments_that_are_not_annotations() {
    let text =
        "#!/usr/bin/env python\n# -*- coding: utf-8 -*-\nx = '# modfence: a=1'\n# modfence:\n";
    let facts = extract("m.py", text);
    assert_eq!(
        facts.annotations.len(),
        1,
        "only the empty one is malformed"
    );
    assert!(facts.annotations[0].parsed.is_err());
}

#[test]
fn crlf_line_endings() {
    let facts = extract("m.py", "# modfence: a=1\r\ndef f():\r\n    pass\r\n");
    assert_eq!(facts.annotations.len(), 1);
    assert_eq!(target_name(&facts.annotations[0]), Some("f"));
    assert_eq!(facts.nodes[1].span.unwrap().start_line, 2);
}

#[test]
fn imports_anywhere_and_spacing() {
    let text = "\
import a . b
def f():
    import c.d as e
    from . pkg import x
    from .. import (y, z as w)
    from __future__ import annotations
";
    let facts = extract("m.py", text);
    let imports: Vec<_> = facts
        .imports
        .iter()
        .map(|edge| (edge.specifier.as_str(), edge.names.clone()))
        .collect();
    let owned = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        imports,
        [
            ("a.b", vec![]),
            ("c.d", vec![]),
            (".pkg", owned(&["x"])),
            ("..", owned(&["y", "z"])),
        ]
    );
}

#[test]
fn calls_and_scopes() {
    let text = "\
class A(make_base()):
    x = build()


@deco(1)
async def f(a=dflt()):
    await g.h.i()
    return [k(z) for z in zs]


f()()
x[0]()
(lambda: 1)()
";
    let facts = extract("m.py", text);
    let calls: Vec<_> = facts
        .calls
        .iter()
        .map(|call| {
            (
                call.callee.as_str(),
                call.receiver.as_deref(),
                call.caller.as_ref().map(qualname),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("make_base", None, None),
            ("build", None, Some("A")),
            ("deco", None, None),
            ("dflt", None, None),
            ("i", Some("g.h"), Some("f")),
            ("k", None, Some("f")),
            ("f", None, None),
        ]
    );
    let kinds: Vec<_> = facts.nodes.iter().map(|node| node.symbol_kind).collect();
    assert_eq!(
        kinds,
        [None, Some(SymbolKind::Class), Some(SymbolKind::Function)]
    );
}

#[test]
fn columns_count_code_points() {
    let facts = extract("m.py", "x = 'é'; f()\n");
    assert_eq!(facts.calls.len(), 1);
    assert_eq!(facts.calls[0].span, Span::new(1, 10, 1, 11).unwrap());
}

#[test]
fn symbol_spans() {
    let facts = extract("m.py", "def f():\n    pass");
    assert_eq!(facts.nodes[1].span, Span::new(1, 1, 2, 9));
    let facts = extract("m.py", "class É: pass\n");
    assert_eq!(facts.nodes[1].span, Span::new(1, 1, 1, 14));
    assert_eq!(facts.nodes[1].id.as_str(), "sym:py:m.py#É");
}

#[test]
fn repeated_names_get_ordinals() {
    let facts = extract("m.py", "def a(): pass\ndef a(): pass\ndef a(): pass\n");
    assert_eq!(symbol_names(&facts), ["a", "a@2", "a@3"]);
    assert_eq!(decode_qualname("a@3"), Some((vec!["a".to_owned()], 3)));
}

#[test]
fn syntax_errors_are_diagnostics() {
    let facts = extract("m.py", "def ok():\n    pass\n\ndef (:\n    pass\n");
    assert_eq!(facts.diagnostics.len(), 1);
    let diagnostic = &facts.diagnostics[0];
    assert_eq!(diagnostic.code, DiagnosticCode::ParseError);
    assert!(diagnostic.span.is_some());
    assert!(!diagnostic.message.contains('/'));
    assert_eq!(
        symbol_names(&facts),
        ["ok"],
        "facts before the error are kept"
    );

    let broken = [
        "x = (1,\n",
        "x = 1\ny = $\n",
        "class A:\n    def f(self):\n        return (1 +\n",
    ];
    for text in broken {
        let facts = extract("m.py", text);
        assert_eq!(facts.diagnostics.len(), 1, "{text:?}");
        assert_eq!(facts.diagnostics[0].code, DiagnosticCode::ParseError);
    }
}

#[test]
fn a_file_without_a_path_is_unreadable() {
    let facts = extract("", "x = 1\n");
    assert_eq!(facts.nodes.len(), 0);
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].code, DiagnosticCode::Unreadable);
}

#[test]
fn deeply_nested_expressions_do_not_overflow_the_stack() {
    let text = format!("x = {}1\n", "f(1) + ".repeat(20_000));
    let facts = extract("m.py", &text);
    assert_eq!(facts.diagnostics.len(), 0);
    assert_eq!(facts.calls.len(), 20_000);
}

#[test]
fn extraction_is_deterministic() {
    let text = fixture("pkg/core.py");
    assert_eq!(extract("pkg/core.py", &text), extract("pkg/core.py", &text));
}

/// The schema's `nodeId` qualified-name segment: `([^\s#%]|%[0-9A-Fa-f]{2})+`.
fn is_schema_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if c == '%' {
            let hex = |c: Option<char>| c.is_some_and(|c| c.is_ascii_hexdigit());
            if !(hex(chars.next()) && hex(chars.next())) {
                return false;
            }
        } else if c.is_whitespace() || c.is_control() || c == '\u{feff}' || c == '#' {
            return false;
        }
    }
    any
}

#[test]
fn qualified_name_round_trip() {
    let cases: [(&[&str], u32, &str); 8] = [
        (&["Engine"], 1, "Engine"),
        (&["Engine", "run"], 1, "Engine.run"),
        (&["f", LOCALS, "g"], 1, "f.<locals>.g"),
        (&["size"], 2, "size@2"),
        (&["size"], 12, "size@12"),
        (&["a.b", "c@d", "e f"], 1, "a%2Eb.c%40d.e%20f"),
        (&["g#h", "i%j", "k\tl"], 3, "g%23h.i%25j.k%09l@3"),
        (&["é", "日本"], 1, "é.日本"),
    ];
    for (components, ordinal, expected) in cases {
        let encoded = encode_qualname(components, ordinal);
        assert_eq!(encoded, expected);
        assert!(is_schema_segment(&encoded), "{encoded}");
        let id = NodeId::new(NodeKind::Symbol, Lang::Py, "a b/m.py", Some(&encoded)).unwrap();
        assert_eq!(id.qualname(), Some(expected));
        assert_eq!(id.path(), "a b/m.py");
        let (decoded, decoded_ordinal) = decode_qualname(&encoded).unwrap();
        assert_eq!(decoded, components);
        assert_eq!(decoded_ordinal, ordinal);
    }
}

#[test]
fn malformed_qualified_names_do_not_decode() {
    for bad in [
        "", "a@1", "a@x", "a@+3", "a@03", "a@2@3", "a..b", ".a", "a.", "%zz", "a%2e", "a b",
    ] {
        assert_eq!(decode_qualname(bad), None, "{bad:?}");
    }
}
