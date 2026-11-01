//! Interface I3: node ids against the graph schema's pattern, spans, nodes and
//! the `Extractor` contract.

use modfence::extract::{
    Annotation, BindingFramework, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, Node,
    NodeError, NodeId, NodeKind, Parsed, SourceFile, Span, SymbolKind, column_from_byte,
    decode_component, encode_component, is_encoded,
};

const SCHEMA: &str = include_str!("../schema/modfence-graph.schema.json");

/// The `nodeId` pattern as it appears (JSON-escaped) in the schema file.
const SCHEMA_PATTERN: &str =
    r"^(pkg|mod|sym|ext):(py|ts|rs):([^\\s#%]|%[0-9A-Fa-f]{2})+(#([^\\s#%]|%[0-9A-Fa-f]{2})+)?$";

/// What `\s` matches in ECMAScript and in Python's `re`, for the pattern's
/// negated classes. Written out here so the check does not reuse the code
/// under test.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{b}'
            | '\u{c}'
            | '\r'
            | '\u{1c}'..='\u{1f}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// `([^\s#%]|%[0-9A-Fa-f]{2})+`
fn segment_matches(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex = |c: Option<char>| c.is_some_and(|c| c.is_ascii_hexdigit());
            if !(hex(chars.next()) && hex(chars.next())) {
                return false;
            }
        } else if is_space(c) || c == '#' {
            return false;
        }
    }
    true
}

/// The schema's `nodeId` pattern, spelled out.
fn matches_schema(id: &str) -> bool {
    let mut parts = id.splitn(3, ':');
    let (Some(kind), Some(lang), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if !["pkg", "mod", "sym", "ext"].contains(&kind) || !["py", "ts", "rs"].contains(&lang) {
        return false;
    }
    match rest.split_once('#') {
        Some((path, name)) => segment_matches(path) && segment_matches(name),
        None => segment_matches(rest),
    }
}

struct Rng(u64);

impl Rng {
    fn step(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).unwrap();
        usize::try_from(self.step() % n).unwrap()
    }
}

fn span(line: u32) -> Span {
    Span::new(line, 1, line, 5).expect("a valid span")
}

#[test]
fn the_pattern_used_here_is_the_schemas() {
    assert!(
        SCHEMA.contains(SCHEMA_PATTERN),
        "the nodeId pattern changed"
    );
}

#[test]
fn the_matcher_agrees_with_the_pattern_on_known_ids() {
    for id in [
        "pkg:py:src/app",
        "mod:ts:a/b.ts",
        "sym:rs:src/lib.rs#Foo%3A%3Abar",
        "ext:py:numpy",
        "mod:py:a%20b.py",
        "mod:py:a:b",
    ] {
        assert!(matches_schema(id), "{id}");
    }
    for id in [
        "",
        "mod",
        "mod:py",
        "mod:py:",
        "mod:go:a.go",
        "xyz:py:a",
        "mod:py:a b",
        "mod:py:a\u{3000}b",
        "mod:py:a%",
        "mod:py:a%2",
        "mod:py:a%zz",
        "mod:py:a#",
        "mod:py:#x",
        "sym:py:a#b#c",
        "sym:py:a#b c",
    ] {
        assert!(!matches_schema(id), "{id}");
    }
}

#[test]
fn builds_ids_of_every_kind() {
    let cases = [
        (
            NodeKind::Package,
            Lang::Py,
            "src/app",
            None,
            "pkg:py:src/app",
        ),
        (
            NodeKind::Module,
            Lang::Ts,
            "web/a b#1%.ts",
            None,
            "mod:ts:web/a%20b%231%25.ts",
        ),
        (NodeKind::External, Lang::Rs, "serde", None, "ext:rs:serde"),
        (
            NodeKind::Symbol,
            Lang::Rs,
            "src/lib.rs",
            Some("Foo%3A%3Abar"),
            "sym:rs:src/lib.rs#Foo%3A%3Abar",
        ),
        (
            NodeKind::Symbol,
            Lang::Py,
            "é/ü.py",
            Some("f"),
            "sym:py:é/ü.py#f",
        ),
    ];
    for (kind, lang, path, qualname, expected) in cases {
        let id = NodeId::new(kind, lang, path, qualname).unwrap();
        assert_eq!(id.as_str(), expected);
        assert_eq!(id.to_string(), expected);
        assert!(matches_schema(id.as_str()), "{id}");
        assert_eq!(id.path(), path);
        assert_eq!(id.qualname(), qualname);
    }
}

#[test]
fn rejects_what_the_pattern_would_reject() {
    let new = |kind, path, qualname| NodeId::new(kind, Lang::Py, path, qualname);
    assert_eq!(new(NodeKind::Module, "", None), Err(NodeError::EmptyPath));
    assert_eq!(
        new(NodeKind::Symbol, "a.py", None),
        Err(NodeError::MissingQualName)
    );
    assert_eq!(
        new(NodeKind::Module, "a.py", Some("f")),
        Err(NodeError::UnexpectedQualName)
    );
    for bad in [
        "",
        "a b",
        "a#b",
        "a%",
        "a%2",
        "a%zz",
        "a\tb",
        "a\u{3000}b",
        "a\u{1}b",
    ] {
        assert_eq!(
            new(NodeKind::Symbol, "a.py", Some(bad)),
            Err(NodeError::InvalidQualName),
            "{bad:?}"
        );
    }
    for good in ["f", "A.b", "%41", "a%2Fb", "é", "<lambda>"] {
        assert!(
            new(NodeKind::Symbol, "a.py", Some(good)).is_ok(),
            "{good:?}"
        );
    }
}

#[test]
fn errors_have_distinct_messages() {
    let errors = [
        NodeError::EmptyPath,
        NodeError::EmptyLabel,
        NodeError::MissingQualName,
        NodeError::UnexpectedQualName,
        NodeError::InvalidQualName,
    ];
    let messages: std::collections::BTreeSet<String> =
        errors.iter().map(ToString::to_string).collect();
    assert_eq!(messages.len(), errors.len());
    let boxed: Box<dyn std::error::Error> = Box::new(NodeError::EmptyPath);
    assert_eq!(boxed.to_string(), NodeError::EmptyPath.to_string());
}

#[test]
fn encodes_whitespace_hash_percent_and_controls_only() {
    assert_eq!(encode_component(""), "");
    assert_eq!(encode_component("src/a-b_c.py"), "src/a-b_c.py");
    assert_eq!(encode_component("a b#c%d"), "a%20b%23c%25d");
    assert_eq!(encode_component("a\tb\nc"), "a%09b%0Ac");
    assert_eq!(encode_component("\u{3000}"), "%E3%80%80");
    assert_eq!(
        encode_component("\u{feff}\u{85}\u{1c}"),
        "%EF%BB%BF%C2%85%1C"
    );
    assert_eq!(encode_component("é🦀"), "é🦀");
}

#[test]
fn decodes_and_rejects_bad_escapes() {
    assert_eq!(
        decode_component("a%20b%2fc%2Fd").as_deref(),
        Some("a b/c/d")
    );
    assert_eq!(decode_component("").as_deref(), Some(""));
    assert_eq!(decode_component("é").as_deref(), Some("é"));
    for bad in ["%", "a%2", "%zz", "%2z", "%ff", "%C3"] {
        assert_eq!(decode_component(bad), None, "{bad:?}");
    }
}

#[test]
fn is_encoded_follows_the_segment_pattern() {
    for good in ["a", "%41", "a%2Fb", "é", "a:b", "a/b"] {
        assert!(is_encoded(good), "{good:?}");
    }
    for bad in [
        "",
        "a b",
        "a#b",
        "%",
        "a%",
        "a%2",
        "%2z",
        "a\u{1}",
        "a\u{feff}",
    ] {
        assert!(!is_encoded(bad), "{bad:?}");
    }
}

const ALPHABET: &str = "ab/._-: \t\n#%é\u{3000}\u{feff}\u{1c}\u{85}\u{a0}🦀0%41";

#[test]
fn generated_paths_always_give_valid_ids_that_decode_back() {
    let alphabet: Vec<char> = ALPHABET.chars().collect();
    let mut rng = Rng(0x0F1E_2D3C_4B5A_6978);
    for _ in 0..2000 {
        let length = 1 + rng.below(12);
        let path: String = (0..length)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let encoded = encode_component(&path);
        assert!(is_encoded(&encoded), "{path:?}");
        assert_eq!(decode_component(&encoded).as_deref(), Some(path.as_str()));
        for (kind, qualname) in [
            (NodeKind::Package, None),
            (NodeKind::Module, None),
            (NodeKind::External, None),
            (NodeKind::Symbol, Some("Foo%2Ebar")),
        ] {
            let id = NodeId::new(kind, Lang::Ts, &path, qualname).unwrap();
            assert!(matches_schema(id.as_str()), "{path:?} -> {id}");
            assert_eq!(id.path(), path);
            assert_eq!(id.qualname(), qualname);
        }
    }
}

#[test]
fn ids_sort_by_text() {
    let a = NodeId::new(NodeKind::Module, Lang::Py, "a.py", None).unwrap();
    let b = NodeId::new(NodeKind::Module, Lang::Py, "b.py", None).unwrap();
    assert!(a < b);
    assert_eq!(a.clone(), a);
}

#[test]
fn language_codes_and_extensions() {
    for (lang, code) in [(Lang::Py, "py"), (Lang::Ts, "ts"), (Lang::Rs, "rs")] {
        assert_eq!(lang.as_str(), code);
    }
    for extension in ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"] {
        assert_eq!(
            Lang::from_extension(extension),
            Some(Lang::Ts),
            "{extension}"
        );
    }
    assert_eq!(Lang::from_extension("py"), Some(Lang::Py));
    assert_eq!(Lang::from_extension("rs"), Some(Lang::Rs));
    for extension in ["", "PY", "txt", "pyi", "json"] {
        assert_eq!(Lang::from_extension(extension), None, "{extension}");
    }
    assert_eq!(Lang::from_path("a/b.d/c.py"), Some(Lang::Py));
    assert_eq!(Lang::from_path("c.test.tsx"), Some(Lang::Ts));
    assert_eq!(Lang::from_path("src/lib.rs"), Some(Lang::Rs));
    for path in [
        "Makefile",
        "a.d/Makefile",
        ".py",
        "a/.rs",
        "a.txt",
        "",
        "dir.py/",
    ] {
        assert_eq!(Lang::from_path(path), None, "{path:?}");
    }
}

#[test]
fn kind_names_match_the_schema() {
    let kinds = [
        (NodeKind::Package, "pkg", "package"),
        (NodeKind::Module, "mod", "module"),
        (NodeKind::Symbol, "sym", "symbol"),
        (NodeKind::External, "ext", "external"),
    ];
    for (kind, prefix, name) in kinds {
        assert_eq!(kind.prefix(), prefix);
        assert_eq!(kind.as_str(), name);
        assert!(SCHEMA.contains(&format!("\"{name}\"")), "{name}");
    }
    let symbol_kinds = [
        (SymbolKind::Function, "function"),
        (SymbolKind::Method, "method"),
        (SymbolKind::Class, "class"),
        (SymbolKind::Struct, "struct"),
        (SymbolKind::Enum, "enum"),
        (SymbolKind::Interface, "interface"),
        (SymbolKind::Type, "type"),
        (SymbolKind::Impl, "impl"),
        (SymbolKind::Trait, "trait"),
        (SymbolKind::Constant, "constant"),
        (SymbolKind::Variable, "variable"),
    ];
    for (kind, name) in symbol_kinds {
        assert_eq!(kind.as_str(), name);
        assert!(SCHEMA.contains(&format!("\"{name}\"")), "{name}");
    }
    let frameworks = [
        (BindingFramework::Pyo3, "pyo3"),
        (BindingFramework::Napi, "napi"),
        (BindingFramework::WasmBindgen, "wasm-bindgen"),
    ];
    for (framework, name) in frameworks {
        assert_eq!(framework.as_str(), name);
        assert!(SCHEMA.contains(&format!("\"{name}\"")), "{name}");
    }
    let codes = [
        (DiagnosticCode::ParseError, "parse-error"),
        (DiagnosticCode::Unreadable, "unreadable"),
        (DiagnosticCode::UnsupportedEncoding, "unsupported-encoding"),
        (DiagnosticCode::TooLarge, "too-large"),
        (DiagnosticCode::UnresolvedImport, "unresolved-import"),
    ];
    for (code, name) in codes {
        assert_eq!(code.as_str(), name);
        assert!(SCHEMA.contains(&format!("\"{name}\"")), "{name}");
    }
}

#[test]
fn spans_are_one_based_and_ordered() {
    let span = Span::new(2, 3, 2, 3).expect("an empty span is allowed");
    assert_eq!(
        (
            span.start_line,
            span.start_column,
            span.end_line,
            span.end_column
        ),
        (2, 3, 2, 3)
    );
    assert!(Span::new(1, 1, 3, 1).is_some());
    for (a, b, c, d) in [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (2, 1, 1, 9),
        (1, 5, 1, 4),
    ] {
        assert_eq!(Span::new(a, b, c, d), None, "{a} {b} {c} {d}");
    }
    assert!(span < Span::new(2, 4, 2, 4).unwrap());
}

#[test]
fn columns_count_code_points() {
    assert_eq!(column_from_byte("abc", 0), Some(1));
    assert_eq!(column_from_byte("abc", 3), Some(4));
    assert_eq!(column_from_byte("é€🦀x", 2), Some(2));
    assert_eq!(column_from_byte("é€🦀x", 5), Some(3));
    assert_eq!(column_from_byte("é€🦀x", 9), Some(4));
    assert_eq!(column_from_byte("é", 1), None);
    assert_eq!(column_from_byte("abc", 4), None);
}

#[test]
fn builds_module_and_symbol_nodes() {
    let module = Node::module(Lang::Py, "pkg/a b.py").unwrap();
    assert_eq!(module.id.as_str(), "mod:py:pkg/a%20b.py");
    assert_eq!(module.kind, NodeKind::Module);
    assert_eq!(module.lang, Lang::Py);
    assert_eq!(module.label, "pkg/a b.py");
    assert_eq!(module.path, "pkg/a b.py");
    assert_eq!((module.symbol_kind, module.span), (None, None));
    assert_eq!(module.display_qualname(), None);
    assert_eq!(Node::module(Lang::Py, ""), Err(NodeError::EmptyPath));

    let symbol = Node::symbol(
        Lang::Rs,
        "src/lib.rs",
        "Foo%3A%3Abar",
        "bar",
        SymbolKind::Method,
        span(4),
    )
    .unwrap();
    assert_eq!(symbol.id.as_str(), "sym:rs:src/lib.rs#Foo%3A%3Abar");
    assert_eq!(symbol.kind, NodeKind::Symbol);
    assert_eq!(symbol.label, "bar");
    assert_eq!(symbol.path, "src/lib.rs");
    assert_eq!(symbol.symbol_kind, Some(SymbolKind::Method));
    assert_eq!(symbol.span, Some(span(4)));
    assert_eq!(symbol.display_qualname().as_deref(), Some("Foo::bar"));
    let odd = Node::symbol(Lang::Rs, "a.rs", "%C3", "f", SymbolKind::Function, span(1));
    assert_eq!(odd.unwrap().display_qualname().as_deref(), Some("%C3"));

    let empty_label = Node::symbol(Lang::Rs, "a.rs", "f", "", SymbolKind::Function, span(1));
    assert_eq!(empty_label, Err(NodeError::EmptyLabel));
    let bad_name = Node::symbol(Lang::Rs, "a.rs", "a b", "f", SymbolKind::Function, span(1));
    assert_eq!(bad_name, Err(NodeError::InvalidQualName));
}

#[test]
fn annotations_come_from_comment_bodies() {
    let target = NodeId::new(NodeKind::Symbol, Lang::Py, "a.py", Some("f")).unwrap();
    let found = Annotation::from_comment(" modfence: layer=x", span(3), Some(target.clone()))
        .expect("an annotation");
    assert_eq!(found.span, span(3));
    assert_eq!(found.target, Some(target));
    assert!(matches!(found.parsed, Ok(Parsed::Tags(_))));

    let malformed = Annotation::from_comment("modfence: path=x", span(1), None).unwrap();
    assert!(malformed.parsed.is_err());
    assert_eq!(
        Annotation::from_comment(" just a comment", span(1), None),
        None
    );

    let target = NodeId::new(NodeKind::Symbol, Lang::Py, "a.py", Some("f")).unwrap();
    let ignore = Annotation::from_comment("modfence: ignore[r]", span(1), Some(target));
    assert_eq!(ignore.unwrap().target, None);
}

/// A stand-in extractor, to pin the trait's shape: object safe, `Send + Sync`,
/// and `handles` has a default.
struct Fake;

impl Extractor for Fake {
    fn language(&self) -> Lang {
        Lang::Py
    }

    fn version(&self) -> &'static str {
        "fake-1"
    }

    fn extract(&self, file: &SourceFile<'_>, _ctx: &ExtractCtx) -> FileFacts {
        FileFacts {
            nodes: vec![Node::module(Lang::Py, file.path).unwrap()],
            ..FileFacts::default()
        }
    }
}

fn assert_send_sync<T: Send + Sync + ?Sized>() {}

#[test]
fn the_extractor_contract() {
    assert_send_sync::<dyn Extractor>();
    let extractor: Box<dyn Extractor> = Box::new(Fake);
    assert_eq!(extractor.language(), Lang::Py);
    assert_eq!(extractor.version(), "fake-1");
    assert!(extractor.handles("pkg/a.py"));
    assert!(!extractor.handles("pkg/a.rs"));
    assert!(!extractor.handles("pkg/a"));

    let file = SourceFile {
        path: "pkg/a.py",
        text: "x = 1\n",
    };
    let facts = extractor.extract(&file, &ExtractCtx::default());
    assert_eq!(facts.nodes.len(), 1);
    assert_eq!(facts.nodes[0].id.as_str(), "mod:py:pkg/a.py");
    assert!(facts.imports.is_empty() && facts.calls.is_empty());
    assert!(facts.annotations.is_empty() && facts.bindings.is_empty());
    assert_eq!(facts.diagnostics, vec![]);
    assert_eq!(facts.clone(), facts);
    assert_eq!(file.text, "x = 1\n");
}
