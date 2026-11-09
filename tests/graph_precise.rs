//! `--precise` (NC6b): SCIP references upgrade tree-sitter call sites to
//! resolved edges (ADR 0003, invariants 7, 8 and 20). The graphs come from the
//! real Python and TypeScript extractors; the indexes are written in code.

use std::collections::BTreeMap;

use modfence::config::Config;
use modfence::extract::python::PythonExtractor;
use modfence::extract::typescript::TsExtractor;
use modfence::extract::{ExtractCtx, Extractor, Lang, NodeId, NodeKind, SourceFile};
use modfence::graph::precise::{PreciseStats, apply};
use modfence::graph::{Backend, Builder, Confidence, Edge, Graph, Origin, Relation};
use modfence::output::{GraphMl, Json, Level, Renderer, View};
use modfence::scip::{DEFINITION, Document, IMPORT, Index, Occurrence, PositionEncoding, RawRange};

const LIB: &str = "def run():\n    pass\n\n\ndef other():\n    pass\n";
const APP: &str = "from lib import run, other\n\n\ndef main():\n    run()\n    other()\n    run()\n\n\ndef helper():\n    run()\n";
const RUN: &str = "scip-python python lib 0.1 `lib`/run().";
const OTHER: &str = "scip-python python lib 0.1 `lib`/other().";

fn build(files: &[(&str, &str)]) -> Graph {
    let config = Config::default();
    let mut builder = Builder::new(&config);
    for &(path, text) in files {
        let file = SourceFile { path, text };
        let facts = match Lang::from_path(path) {
            Some(Lang::Ts) => TsExtractor.extract(&file, &ExtractCtx::default()),
            _ => PythonExtractor.extract(&file, &ExtractCtx::default()),
        };
        builder.add(&file, facts);
    }
    builder.build()
}

fn sources(files: &[(&str, &str)]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|&(path, text)| (path.to_owned(), text.to_owned()))
        .collect()
}

fn id(path: &str, qualname: &str) -> NodeId {
    let lang = if path.rsplit('.').next() == Some("ts") {
        Lang::Ts
    } else {
        Lang::Py
    };
    NodeId::new(NodeKind::Symbol, lang, path, Some(qualname)).unwrap()
}

/// The range of the `skip`-th (from 0) `needle` on line `line` of `source`.
fn at(source: &str, line: u32, needle: &str, skip: usize) -> RawRange {
    let text = source.lines().nth(usize::try_from(line).unwrap()).unwrap();
    let start = text.match_indices(needle).nth(skip).unwrap().0;
    let start = u32::try_from(start).unwrap();
    RawRange {
        start_line: line,
        start_character: start,
        end_line: line,
        end_character: start + u32::try_from(needle.len()).unwrap(),
    }
}

fn occurrence(range: RawRange, symbol: &str, roles: i32) -> Occurrence {
    Occurrence {
        symbol: symbol.to_owned(),
        roles,
        range,
        enclosing: None,
    }
}

fn document(path: &str, occurrences: Vec<Occurrence>) -> Document {
    Document {
        path: path.to_owned(),
        encoding: PositionEncoding::Utf8,
        occurrences,
    }
}

fn index(documents: Vec<Document>) -> Index {
    Index {
        tool: None,
        documents,
    }
}

/// `lib.py` defining `run` and `other`.
fn lib_document() -> Document {
    document(
        "lib.py",
        vec![
            occurrence(at(LIB, 0, "run", 0), RUN, DEFINITION),
            occurrence(at(LIB, 4, "other", 0), OTHER, DEFINITION),
        ],
    )
}

fn files() -> [(&'static str, &'static str); 2] {
    [("lib.py", LIB), ("app.py", APP)]
}

type Row = (String, Option<Confidence>, usize);

fn outgoing(graph: &Graph, relation: Relation, from: &NodeId) -> Vec<Row> {
    graph
        .edges
        .iter()
        .filter(|e| e.relation == relation && &e.source == from)
        .map(|e| (e.target.to_string(), e.confidence, e.count()))
        .collect()
}

fn row(target: &str, confidence: Confidence, count: usize) -> Row {
    (target.to_owned(), Some(confidence), count)
}

fn calls(graph: &Graph, caller: &NodeId) -> Vec<Row> {
    outgoing(graph, Relation::Calls, caller)
}

#[test]
fn a_reference_at_the_callee_span_resolves_the_call() {
    let mut graph = build(&files());
    let before = graph.clone();
    let main = id("app.py", "main");
    assert_eq!(
        calls(&graph, &main),
        [
            row("sym:py:lib.py#other", Confidence::Heuristic, 1),
            row("sym:py:lib.py#run", Confidence::Heuristic, 2),
        ]
    );
    let app = document(
        "app.py",
        vec![
            occurrence(at(APP, 4, "run", 0), RUN, 0),
            occurrence(at(APP, 5, "other", 0), OTHER, 8),
            occurrence(at(APP, 6, "run", 0), RUN, 0),
            occurrence(at(APP, 10, "run", 0), RUN, 0),
        ],
    );
    let stats = apply(
        &mut graph,
        &[index(vec![lib_document(), app])],
        &sources(&files()),
    );
    assert_eq!(graph.backend, Backend::TreeSitterScip);
    assert_eq!(stats.documents, 2);
    assert_eq!(stats.resolved_sites, 4);
    assert_eq!(stats.warnings(), Vec::<String>::new());
    assert_eq!(
        calls(&graph, &main),
        [
            row("sym:py:lib.py#other", Confidence::Resolved, 1),
            row("sym:py:lib.py#run", Confidence::Resolved, 2),
        ]
    );
    let helper = id("app.py", "helper");
    assert_eq!(
        calls(&graph, &helper),
        [row("sym:py:lib.py#run", Confidence::Resolved, 1)]
    );
    for edge in graph.edges.iter().filter(|e| e.relation == Relation::Calls) {
        assert_eq!(edge.origin(), Origin::Scip);
    }
    // Everything but the call edges is as it was, and no site was added.
    let rest = |g: &Graph| {
        g.edges
            .iter()
            .filter(|e| e.relation != Relation::Calls)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(graph.call_site_count(), before.call_site_count());
    assert_eq!(rest(&graph), rest(&before));
    assert!(
        rest(&graph)
            .iter()
            .all(|e| e.origin() == Origin::TreeSitter)
    );
    let sites = |g: &Graph| g.edges.iter().map(Edge::count).sum::<usize>();
    assert_eq!(sites(&graph), sites(&before));
    assert_eq!(graph.nodes, before.nodes);
}

#[test]
fn sites_that_matched_no_name_are_resolved_too() {
    let lib = "def run():\n    pass\n";
    let other = "class Svc:\n    def run_it(self):\n        pass\n";
    let app = "from lib import run as go\n\n\ndef main(svc):\n    go()\n    svc.run_it()\n";
    let files = [("lib.py", lib), ("other.py", other), ("app.py", app)];
    let mut graph = build(&files);
    let main = id("app.py", "main");
    // An aliased import and a method through a receiver: no name matches.
    assert_eq!(calls(&graph, &main), Vec::<Row>::new());
    assert_eq!(graph.call_site_count(), 2);
    let run_it = "scip-python python other 0.1 `other`/Svc#run_it().";
    let lib_doc = document(
        "lib.py",
        vec![occurrence(at(lib, 0, "run", 0), RUN, DEFINITION)],
    );
    let other_doc = document(
        "other.py",
        vec![occurrence(at(other, 1, "run_it", 0), run_it, DEFINITION)],
    );
    let app_doc = document(
        "app.py",
        vec![
            occurrence(at(app, 4, "go", 0), RUN, 0),
            occurrence(at(app, 5, "run_it", 0), run_it, 0),
            // A reference that is not at any call site adds nothing.
            occurrence(at(app, 3, "svc", 0), RUN, 0),
        ],
    );
    let documents = vec![lib_doc, other_doc, app_doc];
    let indexes = [index(documents)];
    let stats = apply(&mut graph, &indexes, &sources(&files));
    assert_eq!(stats.resolved_sites, 2);
    assert_eq!(
        calls(&graph, &main),
        [
            row("sym:py:lib.py#run", Confidence::Resolved, 1),
            row("sym:py:other.py#Svc.run_it", Confidence::Resolved, 1),
        ]
    );
    assert_eq!(graph.call_site_count(), 2);
    let once = graph.clone();
    apply(&mut graph, &indexes, &sources(&files));
    assert_eq!(graph, once);
}

#[test]
fn a_site_the_index_does_not_resolve_stays_heuristic() {
    let mut graph = build(&files());
    let app = document(
        "app.py",
        vec![
            occurrence(at(APP, 4, "run", 0), RUN, 0),
            // Resolved elsewhere, but no definition node in the graph.
            occurrence(
                at(APP, 5, "other", 0),
                "scip-python python os 1 `os`/other().",
                0,
            ),
        ],
    );
    apply(
        &mut graph,
        &[index(vec![lib_document(), app])],
        &sources(&files()),
    );
    let main = id("app.py", "main");
    assert_eq!(
        calls(&graph, &main),
        [
            row("sym:py:lib.py#other", Confidence::Heuristic, 1),
            row("sym:py:lib.py#run", Confidence::Resolved, 1),
            row("sym:py:lib.py#run", Confidence::Heuristic, 1),
        ]
    );
}

#[test]
fn a_name_shared_by_two_nodes_is_settled_by_the_index() {
    let lib = "def run():\n    pass\n\n\nclass Svc:\n    def run(self):\n        pass\n";
    let app = "from lib import run\n\n\ndef main():\n    run()\n";
    let files = [("lib.py", lib), ("app.py", app)];
    let mut graph = build(&files);
    let main = id("app.py", "main");
    assert_eq!(graph.edges.iter().filter(|e| e.source == main).count(), 2);
    let method = "scip-python python lib 0.1 `lib`/Svc#run().";
    let lib_doc = document(
        "lib.py",
        vec![occurrence(at(lib, 5, "run", 0), method, DEFINITION)],
    );
    let app_doc = document("app.py", vec![occurrence(at(app, 4, "run", 0), method, 0)]);
    apply(
        &mut graph,
        &[index(vec![lib_doc, app_doc])],
        &sources(&files),
    );
    assert_eq!(
        calls(&graph, &main),
        [row("sym:py:lib.py#Svc.run", Confidence::Resolved, 1)]
    );
}

#[test]
fn the_innermost_node_with_the_name_is_the_definition() {
    let app = "def f():\n    def f():\n        pass\n    f()\n";
    let files = [("app.py", app)];
    let mut graph = build(&files);
    let outer = id("app.py", "f");
    let inner = graph
        .nodes
        .values()
        .find(|n| n.kind == NodeKind::Symbol && n.label != "f")
        .map(|n| n.id.clone())
        .unwrap();
    let doc = document(
        "app.py",
        vec![
            occurrence(at(app, 0, "f", 1), "py outer", DEFINITION),
            occurrence(at(app, 1, "f", 1), "py inner", DEFINITION),
            occurrence(at(app, 3, "f", 0), "py inner", 0),
        ],
    );
    apply(&mut graph, &[index(vec![doc])], &sources(&files));
    let rows = calls(&graph, &outer);
    assert_eq!(rows, [row(inner.as_str(), Confidence::Resolved, 1)]);
}

#[test]
fn a_definition_that_does_not_name_a_node_defines_nothing() {
    let mut graph = build(&files());
    let lib = document(
        "lib.py",
        vec![
            // `pass`, not `run`.
            occurrence(at(LIB, 1, "pass", 0), RUN, DEFINITION),
            // A name no node has.
            occurrence(at(LIB, 4, "def", 0), OTHER, DEFINITION),
        ],
    );
    let app = document("app.py", vec![occurrence(at(APP, 4, "run", 0), RUN, 0)]);
    apply(&mut graph, &[index(vec![lib, app])], &sources(&files()));
    let main = id("app.py", "main");
    assert!(
        calls(&graph, &main)
            .iter()
            .all(|r| r.1 == Some(Confidence::Heuristic))
    );
}

#[test]
fn imports_and_definitions_are_not_references() {
    let mut graph = build(&files());
    let app = document(
        "app.py",
        vec![
            occurrence(at(APP, 4, "run", 0), RUN, IMPORT),
            occurrence(at(APP, 5, "other", 0), OTHER, DEFINITION),
        ],
    );
    apply(
        &mut graph,
        &[index(vec![lib_document(), app])],
        &sources(&files()),
    );
    let main = id("app.py", "main");
    assert!(
        calls(&graph, &main)
            .iter()
            .all(|r| r.1 == Some(Confidence::Heuristic))
    );
}

#[test]
fn local_symbols_belong_to_their_document() {
    let mut graph = build(&files());
    let lib = document(
        "lib.py",
        vec![occurrence(at(LIB, 0, "run", 0), "local 1", DEFINITION)],
    );
    let app = document(
        "app.py",
        vec![occurrence(at(APP, 4, "run", 0), "local 1", 0)],
    );
    apply(&mut graph, &[index(vec![lib, app])], &sources(&files()));
    let main = id("app.py", "main");
    assert!(
        calls(&graph, &main)
            .iter()
            .all(|r| r.1 == Some(Confidence::Heuristic))
    );

    let nested = "def outer():\n    def inner():\n        pass\n    inner()\n";
    let files = [("n.py", nested)];
    let mut graph = build(&files);
    let doc = document(
        "n.py",
        vec![
            occurrence(at(nested, 1, "inner", 0), "local 0", DEFINITION),
            occurrence(at(nested, 3, "inner", 0), "local 0", 0),
        ],
    );
    apply(&mut graph, &[index(vec![doc])], &sources(&files));
    let outer = id("n.py", "outer");
    assert_eq!(calls(&graph, &outer).len(), 1);
    assert_eq!(calls(&graph, &outer)[0].1, Some(Confidence::Resolved));
}

#[test]
fn a_symbol_defined_twice_resolves_to_the_smallest_node_id() {
    let other = "def run():\n    pass\n";
    let files = [("lib.py", LIB), ("app.py", APP), ("zlib.py", other)];
    let mut graph = build(&files);
    let zlib = document(
        "zlib.py",
        vec![occurrence(at(other, 0, "run", 0), RUN, DEFINITION)],
    );
    let app = document("app.py", vec![occurrence(at(APP, 4, "run", 0), RUN, 0)]);
    let stats = apply(
        &mut graph,
        &[index(vec![zlib, lib_document(), app])],
        &sources(&files),
    );
    assert_eq!(stats.conflicting_symbols, 1);
    assert!(stats.warnings()[0].contains("1 symbols are defined at more than one node"));
    let main = id("app.py", "main");
    let resolved: Vec<_> = calls(&graph, &main)
        .into_iter()
        .filter(|r| r.1 == Some(Confidence::Resolved))
        .collect();
    assert_eq!(
        resolved,
        [row("sym:py:lib.py#run", Confidence::Resolved, 1)]
    );
}

#[test]
fn two_references_at_one_span_resolve_to_the_smaller_symbol() {
    let mut graph = build(&files());
    let app = document(
        "app.py",
        vec![
            occurrence(at(APP, 4, "run", 0), RUN, 0),
            occurrence(at(APP, 4, "run", 0), OTHER, 0),
            occurrence(at(APP, 4, "run", 0), "scip-python python zz 1 `zz`/x().", 0),
        ],
    );
    apply(
        &mut graph,
        &[index(vec![lib_document(), app])],
        &sources(&files()),
    );
    let main = id("app.py", "main");
    // OTHER sorts before RUN, whichever comes first: the site resolves to it.
    assert!(calls(&graph, &main).contains(&row("sym:py:lib.py#other", Confidence::Resolved, 1)));
}

#[test]
fn stale_unknown_and_repeated_documents_are_counted() {
    let mut graph = build(&files());
    let app = document(
        "app.py",
        vec![
            occurrence(at(APP, 4, "run", 0), RUN, 0),
            // A line the file does not have, and an offset past its end.
            occurrence(
                RawRange {
                    start_line: 99,
                    start_character: 0,
                    end_line: 99,
                    end_character: 1,
                },
                RUN,
                0,
            ),
            occurrence(
                RawRange {
                    start_line: 4,
                    start_character: 0,
                    end_line: 4,
                    end_character: 99,
                },
                RUN,
                0,
            ),
        ],
    );
    let repeated = document("app.py", vec![occurrence(at(APP, 6, "run", 0), RUN, 0)]);
    let unknown = document("nope.py", vec![]);
    let first = index(vec![lib_document(), app, unknown]);
    let second = index(vec![repeated]);
    let stats = apply(&mut graph, &[first, second], &sources(&files()));
    assert_eq!(stats.duplicate_documents, 1);
    assert_eq!(stats.unknown_documents, 1);
    assert_eq!(stats.stale_occurrences, 2);
    assert_eq!(stats.documents, 2);
    assert_eq!(stats.warnings().len(), 3);
    let main = id("app.py", "main");
    // Only the first `run` (line 5) was resolved; the repeated document's
    // reference at line 7 was ignored.
    assert!(calls(&graph, &main).contains(&row("sym:py:lib.py#run", Confidence::Resolved, 1)));
    assert!(calls(&graph, &main).contains(&row("sym:py:lib.py#run", Confidence::Heuristic, 1)));
}

#[test]
fn applying_is_idempotent_and_additive() {
    let first = index(vec![
        lib_document(),
        document("app.py", vec![occurrence(at(APP, 4, "run", 0), RUN, 0)]),
    ]);
    let second = index(vec![
        lib_document(),
        document("app.py", vec![occurrence(at(APP, 6, "run", 0), RUN, 0)]),
    ]);
    let mut graph = build(&files());
    apply(&mut graph, std::slice::from_ref(&first), &sources(&files()));
    let once = graph.clone();
    let stats = apply(&mut graph, std::slice::from_ref(&first), &sources(&files()));
    assert_eq!(graph, once);
    assert_eq!(stats.resolved_sites, 0);
    // A second index resolves the other `run` call: one resolved edge of two.
    apply(&mut graph, &[second], &sources(&files()));
    let main = id("app.py", "main");
    assert_eq!(
        calls(&graph, &main),
        [
            row("sym:py:lib.py#other", Confidence::Heuristic, 1),
            row("sym:py:lib.py#run", Confidence::Resolved, 2),
        ]
    );
}

#[test]
fn no_indexes_and_no_matches_change_nothing() {
    let mut graph = build(&files());
    let before = graph.clone();
    assert_eq!(
        apply(&mut graph, &[], &sources(&files())),
        PreciseStats::default()
    );
    assert_eq!(graph, before);
    let unrelated = index(vec![document("lib.py", vec![])]);
    apply(&mut graph, &[unrelated], &sources(&files()));
    assert_eq!(graph.backend, Backend::TreeSitterScip);
    assert_eq!(graph.edges, before.edges);
    assert_eq!(graph.nodes, before.nodes);
}

#[test]
fn typescript_references_in_utf16_documents_resolve() {
    let lib = "export function run() {}\n";
    let app = "import { run } from \"./lib\";\n\nexport function main() {\n  run();\n}\n";
    let files = [("lib.ts", lib), ("app.ts", app)];
    let mut graph = build(&files);
    let symbol = "scip-typescript npm lib 1.0.0 lib/run().";
    let utf16 = |mut document: Document| {
        document.encoding = PositionEncoding::Utf16;
        document
    };
    let lib_doc = utf16(document(
        "lib.ts",
        vec![occurrence(at(lib, 0, "run", 0), symbol, DEFINITION)],
    ));
    let app_doc = utf16(document(
        "app.ts",
        vec![occurrence(at(app, 3, "run", 0), symbol, 0)],
    ));
    apply(
        &mut graph,
        &[index(vec![lib_doc, app_doc])],
        &sources(&files),
    );
    let main = id("app.ts", "main");
    assert_eq!(
        calls(&graph, &main),
        [row("sym:ts:lib.ts#run", Confidence::Resolved, 1)]
    );
}

#[test]
fn the_outputs_say_where_a_resolution_came_from() {
    let mut graph = build(&files());
    let view = |g: &Graph| Json.render(g, &View::new(g, Level::Symbol, None));
    let plain: serde_json::Value = serde_json::from_str(&view(&graph)).unwrap();
    assert_eq!(plain["graph"]["metadata"]["backend"], "tree-sitter");
    let app = document("app.py", vec![occurrence(at(APP, 4, "run", 0), RUN, 0)]);
    apply(
        &mut graph,
        &[index(vec![lib_document(), app])],
        &sources(&files()),
    );
    let json: serde_json::Value = serde_json::from_str(&view(&graph)).unwrap();
    assert_eq!(json["graph"]["metadata"]["backend"], "tree-sitter+scip");
    let edges = json["graph"]["edges"].as_array().unwrap();
    let origin = |relation: &str, confidence: &str| {
        edges
            .iter()
            .filter(|e| e["relation"] == relation && e["metadata"]["confidence"] == confidence)
            .map(|e| e["metadata"]["origin"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(origin("calls", "resolved"), ["scip"]);
    assert!(
        origin("calls", "heuristic")
            .iter()
            .all(|o| o == "tree-sitter")
    );
    assert!(
        origin("imports", "resolved")
            .iter()
            .all(|o| o == "tree-sitter")
    );
    let graphml = GraphMl.render(&graph, &View::new(&graph, Level::Symbol, None));
    assert!(graphml.contains("<data key=\"e_origin\">scip</data>"));
}
