//! The TypeScript and JavaScript extractor: the fixture repository's facts,
//! annotation targets, the qualified-name encoding and its round trip, and
//! parse-error diagnostics.

use modfence::extract::typescript::{
    Segment, Sep, TsExtractor, VERSION, encode_segment, join, render, split,
};
use modfence::extract::{
    Annotation, DiagnosticCode, ExtractCtx, Extractor, FileFacts, Lang, NodeId, NodeKind, Parsed,
    SourceFile, Span, SymbolKind,
};

const INDEX: &str = include_str!("fixtures/typescript/src/index.ts");
const MAIN: &str = include_str!("fixtures/typescript/src/main.js");
const VIEW: &str = include_str!("fixtures/typescript/src/view.tsx");
const BROKEN: &str = include_str!("fixtures/typescript/src/broken.ts");

fn run(path: &str, text: &str) -> FileFacts {
    TsExtractor.extract(&SourceFile { path, text }, &ExtractCtx::default())
}

fn sym(path: &str, qualname: &str) -> NodeId {
    NodeId::new(NodeKind::Symbol, Lang::Ts, path, Some(qualname)).expect("a valid id")
}

fn span(start_line: u32, start_column: u32, end_line: u32, end_column: u32) -> Span {
    Span::new(start_line, start_column, end_line, end_column).expect("a valid span")
}

/// The symbols of `facts` as `(qualified name, kind)`, in order.
fn symbols(facts: &FileFacts) -> Vec<(String, SymbolKind)> {
    facts
        .nodes
        .iter()
        .skip(1)
        .map(|node| {
            let qualname = node.id.qualname().expect("a symbol has a qualified name");
            (
                qualname.to_owned(),
                node.symbol_kind.expect("a symbol kind"),
            )
        })
        .collect()
}

fn names(facts: &FileFacts) -> Vec<String> {
    symbols(facts).into_iter().map(|(name, _)| name).collect()
}

fn imports(facts: &FileFacts) -> Vec<(String, Vec<String>)> {
    facts
        .imports
        .iter()
        .map(|edge| (edge.specifier.clone(), edge.names.clone()))
        .collect()
}

/// The calls of `facts` as `(receiver, callee, caller's qualified name)`.
fn calls(facts: &FileFacts) -> Vec<(Option<String>, String, Option<String>)> {
    facts
        .calls
        .iter()
        .map(|call| {
            let caller = call
                .caller
                .as_ref()
                .map(|id| id.qualname().expect("a symbol").to_owned());
            (call.receiver.clone(), call.callee.clone(), caller)
        })
        .collect()
}

fn tag_target(annotation: &Annotation) -> Option<String> {
    annotation
        .target
        .as_ref()
        .map(|id| id.qualname().expect("a symbol").to_owned())
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn the_extractor_reads_typescript_and_javascript() {
    let extractor = TsExtractor;
    assert_eq!(extractor.language(), Lang::Ts);
    assert_eq!(extractor.version(), VERSION);
    assert!(
        extractor
            .version()
            .contains("tree-sitter-typescript-0.23.2")
    );
    for path in [
        "a.ts", "a.tsx", "a.mts", "a.cts", "a.js", "a.jsx", "a.mjs", "a.cjs",
    ] {
        assert!(extractor.handles(path), "{path}");
    }
    assert!(!extractor.handles("a.py"));
    assert!(!extractor.handles("a.rs"));
}

#[test]
fn the_fixture_module_comes_first_and_has_no_diagnostics() {
    for (path, text) in [
        ("src/index.ts", INDEX),
        ("src/main.js", MAIN),
        ("src/view.tsx", VIEW),
    ] {
        let facts = run(path, text);
        assert_eq!(facts.nodes[0].id.as_str(), format!("mod:ts:{path}"));
        assert_eq!(facts.nodes[0].kind, NodeKind::Module);
        assert_eq!(facts.nodes[0].label, path);
        assert!(
            facts.diagnostics.is_empty(),
            "{path}: {:?}",
            facts.diagnostics
        );
        assert!(facts.bindings.is_empty(), "{path}");
    }
}

#[test]
fn the_fixture_index_symbols() {
    let facts = run("src/index.ts", INDEX);
    let expected = [
        ("load", SymbolKind::Function),
        ("Repo", SymbolKind::Class),
        ("Repo.create", SymbolKind::Method),
        ("Repo~items", SymbolKind::Variable),
        ("Repo~handler", SymbolKind::Method),
        ("Repo~run", SymbolKind::Method),
        ("Repo~size", SymbolKind::Method),
        ("Repo~%23secret", SymbolKind::Method),
        ("Repo~odd%2Ename", SymbolKind::Method),
        ("Repo~[Symbol%2Eiterator]", SymbolKind::Method),
        ("Outer.Inner.answer", SymbolKind::Constant),
        ("Outer.Inner.deep", SymbolKind::Function),
        ("Config", SymbolKind::Interface),
        ("Id", SymbolKind::Type),
        ("Color", SymbolKind::Enum),
        ("arrow", SymbolKind::Function),
        ("viaFunction", SymbolKind::Function),
        ("counter", SymbolKind::Variable),
        ("Local", SymbolKind::Class),
        ("trailing", SymbolKind::Constant),
        ("lazy", SymbolKind::Function),
        ("legacyLoad", SymbolKind::Function),
    ];
    let expected: Vec<(String, SymbolKind)> = expected
        .iter()
        .map(|(name, kind)| ((*name).to_owned(), *kind))
        .collect();
    assert_eq!(symbols(&facts), expected);
}

#[test]
fn overloads_and_merged_declarations_widen_one_node() {
    let facts = run("src/index.ts", INDEX);
    let find = |qualname: &str| {
        facts
            .nodes
            .iter()
            .find(|node| node.id.qualname() == Some(qualname))
            .expect("the node exists")
    };
    // The two signatures and the implementation: one node.
    assert_eq!(find("load").span, Some(span(13, 8, 17, 2)));
    assert_eq!(find("Config").span, Some(span(52, 8, 57, 2)));
    assert_eq!(find("Repo~size").label, "size");
    // Every id is unique.
    let mut ids: Vec<&str> = facts.nodes.iter().map(|node| node.id.as_str()).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn the_fixture_index_imports() {
    let facts = run("src/index.ts", INDEX);
    let expected = vec![
        ("node:fs", owned(&["default", "readFile", "Stats"])),
        ("path", vec![]),
        ("./polyfill", vec![]),
        ("./util.js", owned(&["helper"])),
        ("./legacy", vec![]),
        ("./shapes", owned(&["Shape"])),
        ("./all", vec![]),
        ("./ns", vec![]),
        ("./lazy", vec![]),
        ("./legacy-cjs", vec![]),
    ];
    let expected: Vec<(String, Vec<String>)> = expected
        .into_iter()
        .map(|(specifier, names)| (specifier.to_owned(), names))
        .collect();
    assert_eq!(imports(&facts), expected);
    assert_eq!(facts.imports[0].span, span(3, 1, 3, 58));
    assert_eq!(facts.imports[5].span, span(8, 1, 8, 49));
    assert_eq!(facts.imports[8].span.start_line, 71);
}

#[test]
fn the_fixture_index_calls() {
    let facts = run("src/index.ts", INDEX);
    let call = |receiver: Option<&str>, callee: &str, caller: &str| {
        (
            receiver.map(str::to_owned),
            callee.to_owned(),
            Some(caller.to_owned()),
        )
    };
    let expected = vec![
        call(None, "helper", "load"),
        call(None, "String", "load"),
        call(Some("path"), "join", "load"),
        call(Some("this"), "run", "Repo~handler"),
        call(Some("load(\"x\")"), "trim", "Repo~run"),
        call(None, "load", "Repo~run"),
        call(None, "rf", "Repo~run"),
        call(None, "helper", "Outer.Inner.deep"),
        call(None, "arrow", "viaFunction"),
    ];
    assert_eq!(calls(&facts), expected);
    // The callee name's span, not the whole call.
    assert_eq!(facts.calls[0].span, span(16, 10, 16, 16));
    assert_eq!(facts.calls[2].span, span(16, 38, 16, 42));
}

#[test]
fn the_fixture_index_annotations() {
    let facts = run("src/index.ts", INDEX);
    let targets: Vec<Option<String>> = facts.annotations.iter().map(tag_target).collect();
    assert_eq!(
        targets,
        vec![
            None,
            None,
            Some("load".to_owned()),
            Some("Repo".to_owned()),
            Some("Repo~run".to_owned()),
        ]
    );
    let spans: Vec<u32> = facts
        .annotations
        .iter()
        .map(|annotation| annotation.span.start_line)
        .collect();
    // The tag comment after `const trailing` (line 70) is not an annotation.
    assert_eq!(spans, vec![1, 6, 12, 20, 31]);
    assert!(matches!(facts.annotations[0].parsed, Ok(Parsed::Tags(_))));
    let Ok(Parsed::Ignore(ignore)) = &facts.annotations[1].parsed else {
        panic!("line 6 is a suppression");
    };
    assert_eq!(ignore.rules, owned(&["layers"]));
    assert_eq!(ignore.reason.as_deref(), Some("legacy"));
    assert_eq!(facts.annotations[0].span, span(1, 1, 1, 23));
}

#[test]
fn the_fixture_javascript_file() {
    let facts = run("src/main.js", MAIN);
    assert_eq!(
        symbols(&facts),
        vec![
            ("fs".to_owned(), SymbolKind::Constant),
            ("main".to_owned(), SymbolKind::Function),
        ]
    );
    assert_eq!(
        imports(&facts),
        vec![
            ("node:fs".to_owned(), vec![]),
            ("path".to_owned(), vec![]),
            ("./dynamic.js".to_owned(), vec![]),
        ]
    );
    assert_eq!(
        calls(&facts),
        vec![
            (
                Some("fs".to_owned()),
                "readFileSync".to_owned(),
                Some("main".to_owned())
            ),
            (None, "join".to_owned(), Some("main".to_owned())),
            (None, "main".to_owned(), None),
        ]
    );
    let targets: Vec<Option<String>> = facts.annotations.iter().map(tag_target).collect();
    assert_eq!(targets, vec![Some("fs".to_owned()), None, None]);
    assert!(matches!(facts.annotations[1].parsed, Ok(Parsed::Ignore(_))));
}

#[test]
fn the_fixture_tsx_file() {
    let facts = run("src/view.tsx", VIEW);
    assert_eq!(
        symbols(&facts),
        vec![
            ("Props".to_owned(), SymbolKind::Type),
            ("default".to_owned(), SymbolKind::Function),
            ("Label".to_owned(), SymbolKind::Function),
        ]
    );
    assert_eq!(
        imports(&facts),
        vec![
            ("react".to_owned(), owned(&["default"])),
            ("./button".to_owned(), owned(&["Button"])),
        ]
    );
    assert_eq!(
        calls(&facts),
        vec![
            (None, "track".to_owned(), Some("default".to_owned())),
            (None, "format".to_owned(), Some("default".to_owned())),
        ]
    );
    assert_eq!(
        tag_target(&facts.annotations[0]),
        Some("default".to_owned())
    );
}

#[test]
fn the_grammar_follows_the_extension() {
    let assertion = "const a = <number>b;\n";
    for path in ["a.ts", "a.mts", "a.cts"] {
        assert!(run(path, assertion).diagnostics.is_empty(), "{path}");
    }
    for path in ["a.tsx", "a.jsx", "a.js", "a.mjs", "a.cjs"] {
        assert_eq!(run(path, assertion).diagnostics.len(), 1, "{path}");
    }
    let jsx = "const a = <b>{c()}</b>;\n";
    for path in ["a.jsx", "a.js"] {
        let facts = run(path, jsx);
        assert!(facts.diagnostics.is_empty(), "{path}");
    }
    assert_eq!(run("a.ts", jsx).diagnostics.len(), 1);
}

#[test]
fn a_syntax_error_is_a_diagnostic_and_facts_are_kept() {
    let facts = run("src/broken.ts", BROKEN);
    assert_eq!(facts.diagnostics.len(), 1);
    let diagnostic = &facts.diagnostics[0];
    assert_eq!(diagnostic.code, DiagnosticCode::ParseError);
    assert!(!diagnostic.message.contains('/'));
    let at = diagnostic.span.expect("the error has a position");
    assert!(at.start_line >= 4, "{at:?}");
    assert!(names(&facts).contains(&"ok".to_owned()));
}

#[test]
fn nesting_beyond_the_limit_is_skipped_with_one_diagnostic() {
    let deep = format!("({}a{});\nf();\n", "(".repeat(600), ")".repeat(600));
    let facts = run("deep.ts", &deep);
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].code, DiagnosticCode::ParseError);
    assert!(facts.diagnostics[0].message.contains("nesting"));
    // What lies outside the deep expression is still read.
    assert_eq!(calls(&facts), vec![(None, "f".to_owned(), None)]);
    let shallow = format!("({}a{});\n", "(".repeat(100), ")".repeat(100));
    let facts = run("deep.ts", &shallow);
    assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);
}

#[test]
fn namespaces_prefix_what_they_declare() {
    let source = "\
namespace A.B {
  export class C {}
  namespace D {
    export function f() {}
  }
}
declare module \"ext/mod\" {
  export function g(): void;
}
module Legacy {
  export const h = 1;
}
declare global {
  function ignored(): void;
}
";
    let facts = run("ns.ts", source);
    assert_eq!(
        symbols(&facts),
        vec![
            ("A.B.C".to_owned(), SymbolKind::Class),
            ("A.B.D.f".to_owned(), SymbolKind::Function),
            ("ext/mod.g".to_owned(), SymbolKind::Function),
            ("Legacy.h".to_owned(), SymbolKind::Constant),
        ]
    );
    assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);
}

#[test]
fn static_and_instance_members_and_merged_namespaces_do_not_collide() {
    let source = "\
class A {
  static m() {}
  m() {}
  static x = 1;
  x = () => 1;
}
namespace A {
  export function m() {}
}
";
    let facts = run("merge.ts", source);
    assert_eq!(
        symbols(&facts),
        vec![
            ("A".to_owned(), SymbolKind::Class),
            ("A.m".to_owned(), SymbolKind::Method),
            ("A~m".to_owned(), SymbolKind::Method),
            ("A.x".to_owned(), SymbolKind::Variable),
            ("A~x".to_owned(), SymbolKind::Method),
        ]
    );
    // `A.m` is both the static method and the namespace function: one node,
    // spanning both declarations.
    assert_eq!(facts.nodes[2].span, Some(span(2, 3, 8, 25)));
}

#[test]
fn default_exports() {
    let cases = [
        (
            "export default function () {}",
            "default",
            SymbolKind::Function,
        ),
        (
            "export default function named() {}",
            "named",
            SymbolKind::Function,
        ),
        ("export default class {}", "default", SymbolKind::Class),
        ("export default class Named {}", "Named", SymbolKind::Class),
        ("export default () => 1;", "default", SymbolKind::Function),
        (
            "export default async function* () {}",
            "default",
            SymbolKind::Function,
        ),
    ];
    for (source, name, kind) in cases {
        let facts = run("d.ts", source);
        assert_eq!(symbols(&facts), vec![(name.to_owned(), kind)], "{source}");
    }
    let members = run("d.ts", "export default class { m() {} static s() {} }");
    assert_eq!(
        names(&members),
        owned(&["default", "default~m", "default.s"])
    );
    assert_eq!(run("d.ts", "export default foo;").nodes.len(), 1);
    assert_eq!(run("d.ts", "export default { a: 1 };").nodes.len(), 1);
}

#[test]
fn variables_and_nested_declarations() {
    let source = "\
var v = 1;
let [a, b] = pair();
const { c } = obj;
const k = 1, l = () => 2, M = class {};
function outer() {
  function inner() {}
  const local = () => inner();
  return local();
}
interface I { m(): void; p: number }
enum E { X = f() }
declare const ambient: number;
if (v) {
  function skipped() {}
}
";
    let facts = run("v.ts", source);
    assert_eq!(
        symbols(&facts),
        vec![
            ("v".to_owned(), SymbolKind::Variable),
            ("k".to_owned(), SymbolKind::Constant),
            ("l".to_owned(), SymbolKind::Function),
            ("M".to_owned(), SymbolKind::Class),
            ("outer".to_owned(), SymbolKind::Function),
            ("I".to_owned(), SymbolKind::Interface),
            ("E".to_owned(), SymbolKind::Enum),
            ("ambient".to_owned(), SymbolKind::Constant),
        ]
    );
    // Calls inside nested declarations belong to the enclosing symbol; a call
    // in an enum initialiser belongs to the enum.
    assert_eq!(
        calls(&facts),
        vec![
            (None, "pair".to_owned(), None),
            (None, "inner".to_owned(), Some("outer".to_owned())),
            (None, "local".to_owned(), Some("outer".to_owned())),
            (None, "f".to_owned(), Some("E".to_owned())),
        ]
    );
}

#[test]
fn calls_need_a_name_or_a_member() {
    let source = "\
a.b.c();
f()();
x[0]();
super();
(g || h)();
new K();
tag`t`;
import(dynamic);
require(dynamic);
a?.b?.();
";
    let facts = run("c.ts", source);
    assert_eq!(
        calls(&facts),
        vec![
            (Some("a.b".to_owned()), "c".to_owned(), None),
            (None, "f".to_owned(), None),
            (None, "tag".to_owned(), None),
            (None, "require".to_owned(), None),
            (Some("a".to_owned()), "b".to_owned(), None),
        ]
    );
    assert!(facts.imports.is_empty(), "{:?}", facts.imports);
}

#[test]
fn import_forms() {
    let source = "\
import type { T } from \"t\";
import type D from \"d\";
import { \"str-name\" as s, type U } from \"s\";
import def, * as all from \"all\";
export type { V } from \"v\";
export { w as x };
const m = await import(\"dyn\");
function f() { return require('req'); }
";
    let facts = run("i.ts", source);
    assert_eq!(
        imports(&facts),
        vec![
            ("t".to_owned(), owned(&["T"])),
            ("d".to_owned(), owned(&["default"])),
            ("s".to_owned(), owned(&["str-name", "U"])),
            ("all".to_owned(), owned(&["default"])),
            ("v".to_owned(), owned(&["V"])),
            ("dyn".to_owned(), vec![]),
            ("req".to_owned(), vec![]),
        ]
    );
    assert!(calls(&facts).is_empty(), "{:?}", calls(&facts));
}

#[test]
fn annotation_targets() {
    let source = "\
// modfence: a=1
function blank() {}

// modfence: a=2

function afterBlank() {}

// modfence: a=3
// modfence: a=4
function stacked() {}

/* modfence: a=5 */
/**
 * doc
 */
@decorated
@other()
class Decorated {
  // modfence: a=6
  @field()
  member = 1;

  // modfence: a=7

  detached() {}

  // modfence: a=8
  /* modfence: a=9 */
  last() {}
}

const x = 1; /* modfence: a=10 */
// modfence: ignore[r] reason=\"why\"
function ignored() {}
// modfence: BAD
// modfence: a=11
";
    let facts = run("t.ts", source);
    let got: Vec<(u32, Option<String>)> = facts
        .annotations
        .iter()
        .map(|annotation| (annotation.span.start_line, tag_target(annotation)))
        .collect();
    let expect = |line: u32, target: Option<&str>| (line, target.map(str::to_owned));
    assert_eq!(
        got,
        vec![
            expect(1, Some("blank")),
            expect(4, None),
            expect(8, Some("stacked")),
            expect(9, Some("stacked")),
            expect(12, Some("Decorated")),
            expect(19, Some("Decorated~member")),
            expect(23, None),
            expect(27, Some("Decorated~last")),
            expect(28, Some("Decorated~last")),
            expect(33, None),
            expect(35, None),
            expect(36, None),
        ]
    );
    // A malformed annotation is kept, with its error.
    assert!(facts.annotations[10].parsed.is_err());
    // Annotation-like text in a string or template is not read.
    let strings = run(
        "s.ts",
        "const s = \"// modfence: a=1\";\nconst t = `// modfence: a=2`;\n",
    );
    assert!(strings.annotations.is_empty(), "{:?}", strings.annotations);
}

#[test]
fn columns_count_code_points() {
    let facts = run(
        "u.ts",
        "const s = \"\u{1f600}\"; f();\r\nconst t = \"é\"; g();\r\n",
    );
    assert_eq!(facts.calls[0].span, span(1, 16, 1, 17));
    assert_eq!(facts.calls[1].span, span(2, 16, 2, 17));
}

#[test]
fn names_that_need_escaping_stay_valid_ids() {
    let source = "class A { \"\"() {} \"has space\"() {} 'q'() {} 1() {} }";
    let facts = run("e.ts", source);
    assert_eq!(
        names(&facts),
        owned(&["A", "A~\"\"", "A~has%20space", "A~q", "A~1"])
    );
    // A file path with spaces and a hash is encoded in the id, not the name.
    let spaced = run("dir/a b#c.ts", "function f() {}");
    assert_eq!(spaced.nodes[1].id.as_str(), "sym:ts:dir/a%20b%23c.ts#f");
    assert_eq!(spaced.nodes[1].id.path(), "dir/a b#c.ts");
}

#[test]
fn the_encoding_round_trips() {
    let raws = [
        "plain",
        "a.b",
        "x~y",
        "100%",
        "%2E",
        "has space",
        "tab\there",
        "#priv",
        "ünï",
        "[Symbol.iterator]",
        "\"q\"",
        "line\nbreak",
        "a\u{feff}b",
    ];
    for raw in raws {
        let encoded = encode_segment(raw);
        assert!(!encoded.contains(['.', '~', '#', ' ']), "{encoded}");
        let qualname = join("", Sep::Dot, raw);
        assert_eq!(qualname, encoded);
        assert_eq!(
            split(&qualname),
            Some(vec![Segment {
                sep: None,
                name: raw.to_owned()
            }]),
            "{raw}"
        );
        NodeId::new(NodeKind::Symbol, Lang::Ts, "a.ts", Some(qualname.as_str()))
            .expect("a valid id");
    }
    // Several segments, with both separators.
    let mut qualname = String::new();
    for (index, raw) in raws.iter().enumerate() {
        let sep = if index % 2 == 0 { Sep::Dot } else { Sep::Tilde };
        qualname = join(&qualname, sep, raw);
    }
    let segments = split(&qualname).expect("a valid qualified name");
    assert_eq!(segments.len(), raws.len());
    assert_eq!(segments[0].sep, None);
    assert_eq!(segments[1].sep, Some(Sep::Tilde));
    assert_eq!(segments[2].sep, Some(Sep::Dot));
    for (segment, raw) in segments.iter().zip(raws) {
        assert_eq!(segment.name, raw);
    }
    assert_eq!(render(&segments), qualname);
    assert_eq!(Sep::Dot.as_char(), '.');
    assert_eq!(Sep::Tilde.as_char(), '~');
}

#[test]
fn malformed_qualified_names_do_not_split() {
    for bad in ["", ".", "~", "a..b", ".a", "a.", "a~", "a%zz", "a%2", "%FF"] {
        assert_eq!(split(bad), None, "{bad:?}");
    }
}

#[test]
fn every_extracted_name_round_trips() {
    for (path, text) in [
        ("src/index.ts", INDEX),
        ("src/main.js", MAIN),
        ("src/view.tsx", VIEW),
    ] {
        for node in run(path, text).nodes.iter().skip(1) {
            let qualname = node.id.qualname().expect("a symbol");
            let segments = split(qualname).expect("splits");
            assert_eq!(render(&segments), qualname);
            assert_eq!(node.id, sym(path, qualname));
        }
    }
}

#[test]
fn decorated_members_and_abstract_classes() {
    let source = "\
abstract class B<T> implements I {
  // modfence: a=1
  @first()
  @second
  m(): void {}

  abstract n(): void;

  static {
    init();
  }
}
@Component({ selector: make() })
class C {}
";
    let facts = run("dec.ts", source);
    assert_eq!(
        symbols(&facts),
        vec![
            ("B".to_owned(), SymbolKind::Class),
            ("B~m".to_owned(), SymbolKind::Method),
            ("B~n".to_owned(), SymbolKind::Method),
            ("C".to_owned(), SymbolKind::Class),
        ]
    );
    assert_eq!(tag_target(&facts.annotations[0]), Some("B~m".to_owned()));
    assert_eq!(
        calls(&facts),
        vec![
            (None, "first".to_owned(), Some("B".to_owned())),
            (None, "init".to_owned(), Some("B".to_owned())),
            (None, "Component".to_owned(), Some("C".to_owned())),
            (None, "make".to_owned(), Some("C".to_owned())),
        ]
    );
}

#[test]
fn error_recovery_keeps_ids_valid() {
    let sources = [
        "class {",
        "function (",
        "enum { A }",
        "interface {}",
        "type = 1;",
        "const = 1;",
        "class A { ( }",
        "namespace { export function f() {} }",
        "export default",
        "import",
        "export {",
        "a.(",
    ];
    for source in sources {
        let facts = run("bad.ts", source);
        assert_eq!(facts.nodes[0].id.as_str(), "mod:ts:bad.ts", "{source}");
        assert!(facts.diagnostics.len() <= 1, "{source}");
        for node in facts.nodes.iter().skip(1) {
            let qualname = node.id.qualname().expect("a symbol");
            assert!(split(qualname).is_some(), "{source}: {qualname}");
        }
    }
}
