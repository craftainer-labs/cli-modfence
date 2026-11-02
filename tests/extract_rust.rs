//! The Rust extractor (interface I3): the fixture crate's facts, annotation
//! targets, binding exports, the qualified-name round trip and parse errors.

use std::path::Path;

use modfence::extract::rust::{LOCALS, RustExtractor, decode_qualname, encode_qualname};
use modfence::extract::{
    Annotation, BindingFramework, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, NodeId,
    NodeKind, Parsed, SourceFile, Span, SymbolKind, decode_component,
};

fn extract(path: &str, text: &str) -> FileFacts {
    RustExtractor.extract(&SourceFile { path, text }, &ExtractCtx::default())
}

/// The text of a file of the fixture crate, `tests/fixtures/rust/<path>`.
fn fixture(path: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rust")
        .join(path);
    std::fs::read_to_string(file).expect("the fixture file is readable")
}

fn extract_fixture(path: &str) -> FileFacts {
    extract(path, &fixture(path))
}

fn qualname(id: &NodeId) -> &str {
    id.qualname().expect("a symbol id has a qualified name")
}

/// The span of the `//` comment on 1-based `line` of `text`, to the line's end.
fn comment_span(text: &str, line: u32) -> Span {
    let row = usize::try_from(line - 1).unwrap();
    let source = text.lines().nth(row).unwrap();
    let start = u32::try_from(source.find("//").unwrap() + 1).unwrap();
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

/// The imports as (specifier, names).
fn import_pairs(facts: &FileFacts) -> Vec<(&str, Vec<&str>)> {
    let imports = facts.imports.iter();
    imports
        .map(|edge| {
            let names = edge.names.iter().map(String::as_str).collect();
            (edge.specifier.as_str(), names)
        })
        .collect()
}

#[test]
fn extractor_identity() {
    assert_eq!(RustExtractor.language(), Lang::Rs);
    let version = RustExtractor.version();
    assert!(version.contains("tree-sitter-rust-0.24.2"));
    assert!(RustExtractor.handles("src/lib.rs"));
    assert!(!RustExtractor.handles("src/lib.py"));
    assert!(!RustExtractor.handles("src/lib.rsx"));
}

#[test]
fn fixture_util_module() {
    let facts = extract_fixture("src/util.rs");
    let ids: Vec<&str> = facts.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "mod:rs:src/util.rs",
            "sym:rs:src/util.rs#double",
            "sym:rs:src/util.rs#triple",
        ]
    );
    assert_eq!(facts.nodes[0].kind, NodeKind::Module);
    assert_eq!(facts.calls.len(), 1);
    let call = &facts.calls[0];
    assert_eq!(call.callee, "triple");
    assert_eq!(call.receiver, None);
    assert_eq!(call.caller.as_ref().map(qualname), Some("double"));
    assert_eq!(call.span, Span::new(2, 5, 2, 11).unwrap());
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn fixture_model_module() {
    let facts = extract_fixture("src/model/mod.rs");
    assert_eq!(
        symbol_names(&facts),
        ["Engine", "<Engine>", "<Engine>::start", "<Engine>::stop"]
    );
    assert_eq!(
        import_pairs(&facts),
        [
            ("super", vec!["Config"]),
            ("crate::util", vec![]),
            ("crate::util", vec!["double"]),
            ("self::deep", vec![]),
        ]
    );
    assert_eq!(facts.calls.len(), 1);
    let call = &facts.calls[0];
    assert_eq!(call.callee, "double");
    assert_eq!(call.receiver.as_deref(), Some("helpers"));
    assert_eq!(call.caller.as_ref().map(qualname), Some("<Engine>::start"));
    assert_eq!(facts.annotations.len(), 0);
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn fixture_lib_nodes() {
    let facts = extract_fixture("src/lib.rs");
    let nodes: Vec<_> = facts
        .nodes
        .iter()
        .map(|node| {
            let span = node.span.map(|span| (span.start_line, span.start_column));
            (node.id.as_str(), node.symbol_kind, span)
        })
        .collect();
    let function = Some(SymbolKind::Function);
    let method = Some(SymbolKind::Method);
    let imp = Some(SymbolKind::Impl);
    let at = |line, column| Some((line, column));
    let clamp = "sym:rs:src/lib.rs#<Config>::with_retries::<locals>::clamp";
    assert_eq!(
        nodes,
        [
            ("mod:rs:src/lib.rs", None, None),
            ("sym:rs:src/lib.rs#nested::helper", function, at(18, 5)),
            ("sym:rs:src/lib.rs#Config", Some(SymbolKind::Struct), at(26, 1)),
            ("sym:rs:src/lib.rs#Mode", Some(SymbolKind::Enum), at(30, 1)),
            ("sym:rs:src/lib.rs#Runner", Some(SymbolKind::Trait), at(35, 1)),
            ("sym:rs:src/lib.rs#Runner::run", method, at(36, 5)),
            ("sym:rs:src/lib.rs#Runner::twice", method, at(38, 5)),
            ("sym:rs:src/lib.rs#<Config>", imp, at(43, 1)),
            ("sym:rs:src/lib.rs#<Config>::new", method, at(46, 5)),
            ("sym:rs:src/lib.rs#<Config>::with_retries", method, at(50, 5)),
            (clamp, function, at(51, 9)),
            ("sym:rs:src/lib.rs#<Config%20as%20Display>", imp, at(58, 1)),
            ("sym:rs:src/lib.rs#<Config%20as%20Display>::fmt", method, at(59, 5)),
            ("sym:rs:src/lib.rs#<Config>@2", imp, at(64, 1)),
            ("sym:rs:src/lib.rs#<Config>::describe", method, at(65, 5)),
            ("sym:rs:src/lib.rs#LIMIT", Some(SymbolKind::Constant), at(71, 1)),
            ("sym:rs:src/lib.rs#NAME", Some(SymbolKind::Variable), at(72, 1)),
            ("sym:rs:src/lib.rs#Pair", Some(SymbolKind::Type), at(73, 1)),
            ("sym:rs:src/lib.rs#add", function, at(77, 1)),
            ("sym:rs:src/lib.rs#sub", function, at(83, 1)),
            ("sym:rs:src/lib.rs#mul", function, at(88, 1)),
            ("sym:rs:src/lib.rs#run", function, at(92, 1)),
        ]
    );
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn fixture_lib_imports() {
    let facts = extract_fixture("src/lib.rs");
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
            ("std::collections", vec!["BTreeMap", "HashMap"], 3),
            ("std::fmt", vec![], 4),
            ("std::fmt", vec!["Display"], 4),
            ("crate::util", vec!["*"], 5),
            ("self::model", vec!["Engine"], 6),
            ("self::model::deep", vec!["Thing"], 6),
            ("::serde", vec!["Serialize"], 7),
            ("alloc", vec![], 8),
            ("self::model", vec![], 10),
            ("self::util", vec![], 11),
            ("self", vec!["Config"], 14),
            ("super", vec!["outside"], 15),
            ("self::nested::leaf", vec![], 16),
        ]
    );
}

#[test]
fn fixture_lib_calls() {
    let facts = extract_fixture("src/lib.rs");
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
    let clamp = format!("<Config>::with_retries::{LOCALS}::clamp");
    let with_retries = Some("<Config>::with_retries");
    let describe = Some("<Config>::describe");
    assert_eq!(
        calls,
        [
            ("new", Some("Config"), Some("nested::helper"), 19),
            ("run", Some("self"), Some("Runner::twice"), 39),
            ("run", Some("self"), Some("Runner::twice"), 39),
            ("with_retries", Some("Self"), Some("<Config>::new"), 47),
            ("min", Some("value"), Some(clamp.as_str()), 52),
            ("clamp", None, with_retries, 54),
            ("parse", Some("\"7\""), describe, 66),
            ("start", Some("Engine"), describe, 67),
            ("stop", Some("Engine::start(self)"), describe, 67),
            ("limit", None, Some("LIMIT"), 71),
            ("double", Some("util"), Some("run"), 93),
            ("f", None, Some("run"), 94),
            ("helper_call", None, Some("run"), 95),
        ]
    );
    let first = &facts.calls[0];
    assert_eq!(first.span, Span::new(19, 17, 19, 20).unwrap());
}

#[test]
fn fixture_lib_annotations() {
    let text = fixture("src/lib.rs");
    let facts = extract("src/lib.rs", &text);
    let found: Vec<_> = facts
        .annotations
        .iter()
        .map(|annotation| (annotation.span, target_name(annotation)))
        .collect();
    assert_eq!(
        found,
        [
            (comment_span(&text, 1), None),
            (comment_span(&text, 23), Some("Config")),
            (comment_span(&text, 44), Some("<Config>::new")),
            (comment_span(&text, 95), None),
        ]
    );
    assert_eq!(tag_values(&facts.annotations[0], "layer"), ["public"]);
    assert_eq!(tag_values(&facts.annotations[1], "layer"), ["domain"]);
    assert_eq!(tag_values(&facts.annotations[2], "owner"), ["core"]);
    match &facts.annotations[3].parsed {
        Ok(Parsed::Ignore(suppression)) => {
            assert_eq!(suppression.rules, ["layers"]);
            assert_eq!(suppression.reason.as_deref(), Some("entry"));
        }
        other => panic!("expected a suppression, got {other:?}"),
    }
}

#[test]
fn fixture_lib_bindings() {
    let facts = extract_fixture("src/lib.rs");
    let found: Vec<_> = facts
        .bindings
        .iter()
        .map(|binding| {
            (
                qualname(&binding.symbol),
                binding.framework,
                binding.exported_name.as_str(),
                binding.span.start_line,
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("add", BindingFramework::Pyo3, "plus", 75),
            ("sub", BindingFramework::Napi, "subtract", 81),
            ("mul", BindingFramework::WasmBindgen, "mul", 87),
        ]
    );
}

#[test]
fn fixture_symbol_ids_round_trip() {
    for path in ["src/lib.rs", "src/model/mod.rs", "src/util.rs"] {
        for node in extract_fixture(path).nodes.iter().skip(1) {
            let encoded = qualname(&node.id);
            let (components, ordinal) = decode_qualname(encoded).unwrap();
            assert_eq!(encode_qualname(&components, ordinal), encoded);
            assert_eq!(node.label, components.join("::"));
            let shown = decode_component(encoded).unwrap();
            assert_eq!(node.display_qualname(), Some(shown));
            assert_eq!(node.id.path(), path);
        }
    }
}

#[test]
fn fixture_broken_file() {
    let facts = extract_fixture("src/broken.rs");
    assert_eq!(facts.diagnostics.len(), 1);
    let diagnostic = &facts.diagnostics[0];
    assert_eq!(diagnostic.code, DiagnosticCode::ParseError);
    assert!(diagnostic.span.is_some());
    assert!(!diagnostic.message.contains('/'));
    assert_eq!(
        symbol_names(&facts).first(),
        Some(&"ok"),
        "facts before the error are kept"
    );
}

#[test]
fn syntax_errors_are_diagnostics() {
    let broken = ["fn f( {\n", "use a::{;\n", "struct S { a: \n", "fn f() { 1 +\n"];
    for text in broken {
        let facts = extract("m.rs", text);
        assert_eq!(facts.diagnostics.len(), 1, "{text:?}");
        assert_eq!(facts.diagnostics[0].code, DiagnosticCode::ParseError);
    }
}

#[test]
fn a_file_without_a_path_is_unreadable() {
    let facts = extract("", "fn f() {}\n");
    assert_eq!(facts.nodes.len(), 0);
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].code, DiagnosticCode::Unreadable);
}

#[test]
fn annotation_targets() {
    let text = "\
// modfence: a=1
fn top() {}

// modfence: b=2
#[inline]
// modfence: c=3
fn mid() {}
// modfence: d=4

fn blank() {}
struct K;
impl K {
    // modfence: e=5
    fn m(&self) {}  // modfence: f=6
}
// modfence: ignore[x] reason=\"r\"
fn ign() {}
// see modfence: nothing
/* modfence: g=7 */
const END: u8 = 0;
// modfence: h=8
";
    let facts = extract("m.rs", text);
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
            (13, Some("<K>::m")),
            (14, None),
            (16, None),
            (19, Some("END")),
            (21, None),
        ]
    );
}

#[test]
fn annotation_without_a_line_below() {
    let facts = extract("m.rs", "fn f() {}\n// modfence: a=1");
    assert_eq!(facts.annotations.len(), 1);
    assert_eq!(facts.annotations[0].target, None);
}

#[test]
fn comments_that_are_not_annotations() {
    let text = "\
//! modfence: a=1
/// modfence: b=2
const S: &str = \"// modfence: c=3\";
// modfence:
";
    let facts = extract("m.rs", text);
    assert_eq!(
        facts.annotations.len(),
        1,
        "only the empty one is malformed"
    );
    assert!(facts.annotations[0].parsed.is_err());
}

#[test]
fn crlf_line_endings() {
    let facts = extract("m.rs", "// modfence: a=1\r\nfn f() {\r\n}\r\n");
    assert_eq!(facts.annotations.len(), 1);
    assert_eq!(target_name(&facts.annotations[0]), Some("f"));
    assert_eq!(facts.nodes[1].span.unwrap().start_line, 2);
}

#[test]
fn imports_in_every_form() {
    let text = "\
use a::{b::{c, d}, e as f, g::*, self};
use {h, i::j};
use serde;
use k as l;
mod m {
    use super::x;
    mod n {
        use super::super::y;
        use super::super::super::z;
        use self::o::p;
    }
    mod q;
}
fn f() {
    use self::r::s;
}
";
    let facts = extract("m.rs", text);
    assert_eq!(
        import_pairs(&facts),
        [
            ("a::b", vec!["c", "d"]),
            ("a", vec!["e"]),
            ("a::g", vec!["*"]),
            ("a", vec![]),
            ("h", vec![]),
            ("i", vec!["j"]),
            ("serde", vec![]),
            ("k", vec![]),
            ("self", vec!["x"]),
            ("self", vec!["y"]),
            ("super", vec!["z"]),
            ("self::m::n::o", vec!["p"]),
            ("self::m::q", vec![]),
            ("self::r", vec!["s"]),
        ]
    );
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn deeply_nested_use_lists_do_not_overflow_the_stack() {
    let text = format!("use {}x{};\n", "{".repeat(5_000), "}".repeat(5_000));
    let facts = extract("m.rs", &text);
    assert_eq!(import_pairs(&facts), [("x", vec![])]);
}

#[test]
fn calls_and_scopes() {
    let text = "\
struct S(u8);
const C: S = S(make());
fn f(a: u8) -> u8 {
    let g = |x| h(x);
    g.0();
    (f)(1);
    f(1)();
    m::<u8>(a);
    x::y::z();
    a.b.c();
    foo!(skipped());
    a
}
";
    let facts = extract("m.rs", text);
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
            ("S", None, Some("C")),
            ("make", None, Some("C")),
            ("h", None, Some("f")),
            ("f", None, Some("f")),
            ("m", None, Some("f")),
            ("z", Some("x::y"), Some("f")),
            ("c", Some("a.b"), Some("f")),
        ]
    );
    assert_eq!(facts.diagnostics.len(), 0);
}

#[test]
fn symbol_kinds() {
    let text = "\
struct A;
union U { a: u8 }
enum E { X }
trait T { fn sig(&self); }
type Al = u8;
const K: u8 = 1;
static ST: u8 = 2;
extern \"C\" { fn ext(); }
impl A { fn m(&self) {} const AC: u8 = 3; }
";
    let facts = extract("m.rs", text);
    let kinds: Vec<_> = facts
        .nodes
        .iter()
        .skip(1)
        .map(|node| (qualname(&node.id), node.symbol_kind.unwrap().as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("A", "struct"),
            ("U", "struct"),
            ("E", "enum"),
            ("T", "trait"),
            ("T::sig", "method"),
            ("Al", "type"),
            ("K", "constant"),
            ("ST", "variable"),
            ("ext", "function"),
            ("<A>", "impl"),
            ("<A>::m", "method"),
            ("<A>::AC", "constant"),
        ]
    );
}

#[test]
fn modules_and_locals_in_names() {
    let text = "\
mod outer {
    pub fn f() {
        struct Local;
        impl Local { fn g(&self) {} }
    }
    pub mod inner { pub fn h() {} }
}
";
    let facts = extract("m.rs", text);
    assert_eq!(
        symbol_names(&facts),
        [
            "outer::f",
            "outer::f::<locals>::Local",
            "outer::f::<locals>::<Local>",
            "outer::f::<locals>::<Local>::g",
            "outer::inner::h",
        ]
    );
}

#[test]
fn impl_names() {
    let text = "\
impl<T: Clone> From<T> for Wrapper<T> {
    fn from(value: T) -> Self { todo() }
}
impl<'a> Wrapper<&'a [u8]> {}
impl  fmt::Display
    for Pair {}
";
    let facts = extract("m.rs", text);
    let names = symbol_names(&facts);
    assert_eq!(
        names,
        [
            "<Wrapper<T>%20as%20From<T>>",
            "<Wrapper<T>%20as%20From<T>>::from",
            "<Wrapper<&'a%20[u8]>>",
            "<Pair%20as%20fmt%3A%3ADisplay>",
        ]
    );
    for name in names {
        let (components, ordinal) = decode_qualname(name).unwrap();
        assert_eq!(encode_qualname(&components, ordinal), name);
    }
    assert_eq!(facts.nodes[4].label, "<Pair as fmt::Display>");
}

#[test]
fn bindings_in_every_form() {
    let text = "\
#[pymodule]
fn module(m: u8) {}

#[napi_derive::napi]
pub struct Napi;

#[wasm_bindgen(js_name = renamed)]
pub fn wasm() {}

#[pyfunction(name = \"x\")]
fn py() {}

#[napi]
#[allow(dead_code)]
fn plain() {}

#[derive(Debug)]
fn none() {}
";
    let facts = extract("m.rs", text);
    let found: Vec<_> = facts
        .bindings
        .iter()
        .map(|binding| {
            (
                qualname(&binding.symbol),
                binding.framework.as_str(),
                binding.exported_name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("module", "pyo3", "module"),
            ("Napi", "napi", "Napi"),
            ("wasm", "wasm-bindgen", "renamed"),
            ("py", "pyo3", "x"),
            ("plain", "napi", "plain"),
        ]
    );
}

#[test]
fn columns_count_code_points() {
    let facts = extract("m.rs", "fn f() { let s = \"é\"; g(); }\n");
    assert_eq!(facts.calls.len(), 1);
    assert_eq!(facts.calls[0].span, Span::new(1, 23, 1, 24).unwrap());
}

#[test]
fn symbol_spans() {
    let facts = extract("m.rs", "fn f() {\n    1\n}");
    assert_eq!(facts.nodes[1].span, Span::new(1, 1, 3, 2));
    let facts = extract("m.rs", "struct É;\n");
    assert_eq!(facts.nodes[1].span, Span::new(1, 1, 1, 10));
    assert_eq!(facts.nodes[1].id.as_str(), "sym:rs:m.rs#É");
}

#[test]
fn repeated_names_get_ordinals() {
    let facts = extract("m.rs", "fn a() {}\nfn a() {}\nfn a() {}\n");
    assert_eq!(symbol_names(&facts), ["a", "a@2", "a@3"]);
    assert_eq!(decode_qualname("a@3"), Some((vec!["a".to_owned()], 3)));
}

#[test]
fn deeply_nested_expressions_do_not_overflow_the_stack() {
    let text = format!("fn f() {{ let x = {}1; }}\n", "g(1) + ".repeat(20_000));
    let facts = extract("m.rs", &text);
    assert_eq!(facts.diagnostics.len(), 0);
    assert_eq!(facts.calls.len(), 20_000);
}

#[test]
fn extraction_is_deterministic() {
    let text = fixture("src/lib.rs");
    assert_eq!(extract("src/lib.rs", &text), extract("src/lib.rs", &text));
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
        (&["Config"], 1, "Config"),
        (&["Config", "new"], 1, "Config::new"),
        (&["f", LOCALS, "g"], 1, "f::<locals>::g"),
        (&["size"], 2, "size@2"),
        (&["size"], 12, "size@12"),
        (&["<A as b::C>", "d"], 1, "<A%20as%20b%3A%3AC>::d"),
        (&["e@f", "g:h", "i#j"], 3, "e%40f::g%3Ah::i%23j@3"),
        (&["é", "日本"], 1, "é::日本"),
    ];
    for (components, ordinal, expected) in cases {
        let encoded = encode_qualname(components, ordinal);
        assert_eq!(encoded, expected);
        assert!(is_schema_segment(&encoded), "{encoded}");
        let id = NodeId::new(NodeKind::Symbol, Lang::Rs, "a b/m.rs", Some(&encoded)).unwrap();
        assert_eq!(id.qualname(), Some(expected));
        assert_eq!(id.path(), "a b/m.rs");
        let (decoded, decoded_ordinal) = decode_qualname(&encoded).unwrap();
        assert_eq!(decoded, components);
        assert_eq!(decoded_ordinal, ordinal);
    }
}

#[test]
fn malformed_qualified_names_do_not_decode() {
    for bad in [
        "", "a@1", "a@x", "a@+3", "a@03", "a@2@3", "a::::b", "::a", "a::", "a:b", "a:::b", "%zz",
        "a%3a", "a b",
    ] {
        assert_eq!(decode_qualname(bad), None, "{bad:?}");
    }
}
