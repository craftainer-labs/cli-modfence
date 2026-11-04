//! The output layer (ADR 0004, "Traceability outputs"): the view of a graph at
//! each level with its clusters, the part of a graph that touches a selection,
//! and the JSON document.

use std::collections::BTreeSet;

use serde_json::Value;

use modfence::config::{self, Config};
use modfence::extract::python::PythonExtractor;
use modfence::extract::{
    BindingFramework, DiagnosticCode, ExtractCtx, Extractor, Lang, NodeId, NodeKind, SourceFile,
    Span,
};
use modfence::graph::{Binding, Builder, FileDiagnostic, Graph, Relation};
use modfence::output::{Json, Level, Renderer, View, incident};

const CONFIG: &str = r#"
version = 1

[[node]]
match = "app/**"
tags = { layer = "app", owner = ["team-b", "team-a"] }

[[node]]
match = "lib/**"
tags = { layer = "lib" }

[[node]]
package = "requests"
tags = { boundary = "network" }
"#;

const FILES: [(&str, &str); 4] = [
    ("app/__init__.py", ""),
    (
        "app/a.py",
        "import requests\nimport app.b\n\n\ndef run():\n    go()\n",
    ),
    ("app/b.py", "def go():\n    pass\n"),
    ("lib/c.py", "import app.a\nimport app.a\n"),
];

fn graph() -> Graph {
    let config: Config = config::parse(CONFIG).unwrap();
    let mut builder = Builder::new(&config);
    for (path, text) in FILES {
        let file = SourceFile { path, text };
        builder.add(
            &file,
            PythonExtractor.extract(&file, &ExtractCtx::default()),
        );
    }
    builder.build()
}

fn id(kind: NodeKind, path: &str, qualname: Option<&str>) -> NodeId {
    NodeId::new(kind, Lang::Py, path, qualname).unwrap()
}

fn ids<'a>(view: &View<'a>) -> Vec<&'a str> {
    view.nodes.iter().map(|node| node.id.as_str()).collect()
}

/// `source -> target relation count` for each edge of the view.
fn edges(view: &View<'_>) -> Vec<String> {
    view.edges
        .iter()
        .map(|edge| {
            format!(
                "{} -> {} {} {}",
                edge.source,
                edge.target,
                edge.relation.as_str(),
                edge.count
            )
        })
        .collect()
}

fn clusters(view: &View<'_>) -> Vec<String> {
    view.clusters
        .iter()
        .map(|cluster| {
            let members: Vec<&str> = cluster.members.iter().map(|id| id.as_str()).collect();
            format!("{}: {}", cluster.label, members.join(" "))
        })
        .collect()
}

#[test]
fn a_package_view_has_packages_and_externals_with_rolled_up_edges() {
    let graph = graph();
    let view = View::new(&graph, Level::Package, None);
    assert_eq!(view.level, Level::Package);
    assert_eq!(view.group_by, None);
    assert_eq!(ids(&view), ["ext:py:requests", "pkg:py:app", "pkg:py:lib"]);
    // app/a.py -> app/b.py stays inside one package and is dropped, the two
    // imports of lib/c.py are folded, and no contains edge survives.
    assert_eq!(
        edges(&view),
        [
            "pkg:py:app -> ext:py:requests imports 1",
            "pkg:py:lib -> pkg:py:app imports 2",
        ]
    );
    assert_eq!(view.clusters.len(), 0);
}

#[test]
fn a_module_view_is_the_default_level_and_clusters_by_package() {
    assert_eq!(Level::default(), Level::Module);
    let graph = graph();
    let view = View::new(&graph, Level::Module, None);
    assert_eq!(
        ids(&view),
        [
            "ext:py:requests",
            "mod:py:app/__init__.py",
            "mod:py:app/a.py",
            "mod:py:app/b.py",
            "mod:py:lib/c.py",
            "pkg:py:app",
            "pkg:py:lib",
        ]
    );
    // The call from run to go is rolled up to the modules and kept apart from
    // the import between them.
    assert_eq!(
        edges(&view),
        [
            "mod:py:app/a.py -> ext:py:requests imports 1",
            "mod:py:app/a.py -> mod:py:app/b.py imports 1",
            "mod:py:app/a.py -> mod:py:app/b.py calls 1",
            "mod:py:lib/c.py -> mod:py:app/a.py imports 2",
            "pkg:py:app -> mod:py:app/__init__.py contains 0",
            "pkg:py:app -> mod:py:app/a.py contains 0",
            "pkg:py:app -> mod:py:app/b.py contains 0",
            "pkg:py:lib -> mod:py:lib/c.py contains 0",
        ]
    );
    assert_eq!(
        clusters(&view),
        [
            "app: mod:py:app/__init__.py mod:py:app/a.py mod:py:app/b.py",
            "lib: mod:py:lib/c.py",
        ]
    );
}

#[test]
fn a_symbol_view_shows_everything() {
    let graph = graph();
    let view = View::new(&graph, Level::Symbol, None);
    assert_eq!(view.nodes.len(), graph.nodes.len());
    assert!(ids(&view).contains(&"sym:py:app/a.py#run"));
    let all = edges(&view);
    assert!(all.contains(&"mod:py:app/a.py -> sym:py:app/a.py#run contains 0".to_owned()));
    assert!(all.contains(&"sym:py:app/a.py#run -> sym:py:app/b.py#go calls 1".to_owned()));
    assert_eq!(
        clusters(&view)[0],
        "app: mod:py:app/__init__.py mod:py:app/a.py mod:py:app/b.py \
         sym:py:app/a.py#run sym:py:app/b.py#go"
    );
}

// ADR 0004, "Traceability outputs": `--group-by` clusters by a tag, in place
// of the package grouping; several values go to the first in sorted order.
#[test]
fn group_by_clusters_the_visible_nodes_by_a_tag() {
    let graph = graph();
    let view = View::new(&graph, Level::Module, Some("layer"));
    assert_eq!(view.group_by.as_deref(), Some("layer"));
    assert_eq!(
        clusters(&view),
        [
            "app: mod:py:app/__init__.py mod:py:app/a.py mod:py:app/b.py",
            "lib: mod:py:lib/c.py",
        ]
    );
    let owners = View::new(&graph, Level::Module, Some("owner"));
    assert_eq!(
        clusters(&owners),
        ["team-a: mod:py:app/__init__.py mod:py:app/a.py mod:py:app/b.py"]
    );
    // The other values stay on the node, for the hover text.
    let node = owners
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "mod:py:app/a.py");
    let values: Vec<&str> = node.unwrap().tags["owner"]
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(values, ["team-a", "team-b"]);
    // An external package is clustered by its tags like any node.
    let network = View::new(&graph, Level::Package, Some("boundary"));
    assert_eq!(clusters(&network), ["network: ext:py:requests"]);
    let unknown = View::new(&graph, Level::Module, Some("nope"));
    assert_eq!(unknown.clusters.len(), 0);
}

#[test]
fn an_empty_graph_has_an_empty_view() {
    let graph = Graph::default();
    let view = View::new(&graph, Level::Symbol, None);
    assert_eq!(ids(&view), Vec::<&str>::new());
    assert!(view.edges.is_empty() && view.clusters.is_empty());
}

#[test]
fn levels_show_the_kinds_they_name() {
    let kinds = [
        NodeKind::Package,
        NodeKind::Module,
        NodeKind::Symbol,
        NodeKind::External,
    ];
    let shown = |level: Level| kinds.map(|kind| level.shows(kind));
    assert_eq!(shown(Level::Package), [true, false, false, true]);
    assert_eq!(shown(Level::Module), [true, true, false, true]);
    assert_eq!(shown(Level::Symbol), [true, true, true, true]);
}

// ADR 0004, invariant 23: every selected node, every edge with a selected
// endpoint, and nothing else.
#[test]
fn incident_keeps_the_selection_its_edges_and_their_other_ends() {
    let graph = graph();
    let requests = id(NodeKind::External, "requests", None);
    let part = incident(&graph, &BTreeSet::from([requests]));
    let nodes: Vec<&str> = part.nodes.keys().map(NodeId::as_str).collect();
    assert_eq!(nodes, ["ext:py:requests", "mod:py:app/a.py"]);
    assert_eq!(part.edges.len(), 1);
    assert_eq!(part.languages, BTreeSet::from([Lang::Py]));
    assert!(part.diagnostics.is_empty());
    let nothing = incident(&graph, &BTreeSet::new());
    assert!(nothing.nodes.is_empty() && nothing.edges.is_empty());
}

// The metadata of a trace describes its own nodes: a trace of one language
// omits the other, and a diagnostic of a file outside the trace is left out.
#[test]
fn incident_keeps_only_the_languages_and_diagnostics_of_its_nodes() {
    let mut graph = graph();
    let module = id(NodeKind::Module, "lib/c.py", None);
    graph.nodes.get_mut(&module).unwrap().lang = Lang::Ts;
    graph.languages.insert(Lang::Ts);
    for path in ["lib/c.py", "app/b.py"] {
        graph.diagnostics.push(FileDiagnostic {
            path: path.to_owned(),
            span: None,
            code: DiagnosticCode::ParseError,
            message: "syntax error".to_owned(),
        });
    }
    let requests = id(NodeKind::External, "requests", None);
    let python = incident(&graph, &BTreeSet::from([requests]));
    assert_eq!(python.languages, BTreeSet::from([Lang::Py]));
    assert!(python.diagnostics.is_empty());
    let both = incident(&graph, &BTreeSet::from([module]));
    assert_eq!(both.languages, BTreeSet::from([Lang::Py, Lang::Ts]));
    let paths: Vec<&str> = both.diagnostics.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["lib/c.py"]);
}

// A node with no ancestor the level shows stays visible as itself, so none of
// its edges is dropped.
#[test]
fn a_module_without_a_package_keeps_its_edges_at_the_package_level() {
    let mut graph = graph();
    graph.nodes.remove(&id(NodeKind::Package, "lib", None));
    let view = View::new(&graph, Level::Package, None);
    assert!(ids(&view).contains(&"mod:py:lib/c.py"), "{:?}", ids(&view));
    assert!(
        edges(&view).contains(&"mod:py:lib/c.py -> pkg:py:app imports 2".to_owned()),
        "{:?}",
        edges(&view)
    );
}

// A `contains` cycle ends the walk up the tree instead of hanging it.
#[test]
fn a_contains_cycle_does_not_hang_the_view() {
    let mut graph = graph();
    let template = graph
        .edges
        .iter()
        .find(|edge| edge.relation == Relation::Contains)
        .unwrap()
        .clone();
    let a = id(NodeKind::Module, "app/a.py", None);
    let b = id(NodeKind::Module, "app/b.py", None);
    for (source, target) in [(&a, &b), (&b, &a)] {
        let mut edge = template.clone();
        edge.source = source.clone();
        edge.target = target.clone();
        graph.edges.push(edge);
    }
    let view = View::new(&graph, Level::Package, None);
    assert!(ids(&view).contains(&"mod:py:app/a.py"), "{:?}", ids(&view));
    assert!(ids(&view).contains(&"mod:py:app/b.py"), "{:?}", ids(&view));
}

fn document(graph: &Graph) -> Value {
    let view = View::new(graph, Level::Module, None);
    let text = Json.render(graph, &view);
    assert!(text.ends_with("}\n"));
    serde_json::from_str(&text).unwrap()
}

// ADR 0003, "Output format": the document of `schema/modfence-graph.schema.json`.
#[test]
fn the_json_document_has_everything_the_schema_names() {
    let mut graph = graph();
    graph.diagnostics.push(FileDiagnostic {
        path: "lib/c.py".to_owned(),
        span: Span::new(1, 1, 1, 5),
        code: DiagnosticCode::ParseError,
        message: "syntax error".to_owned(),
    });
    let run = id(NodeKind::Symbol, "app/a.py", Some("run"));
    graph.nodes.get_mut(&run).unwrap().binding = Some(Binding {
        framework: BindingFramework::Pyo3,
        exported_name: "run".to_owned(),
    });
    let value = document(&graph);
    let body = &value["graph"];
    assert_eq!(body["directed"], true);
    assert_eq!(body["type"], "modfence.dependency-graph");
    let metadata = &body["metadata"];
    assert_eq!(metadata["schema_version"], modfence::output::SCHEMA_VERSION);
    assert_eq!(metadata["tool"]["name"], "modfence");
    assert_eq!(metadata["tool"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(metadata["root"], ".");
    assert_eq!(metadata["backend"], "tree-sitter");
    assert_eq!(metadata["languages"], serde_json::json!(["py"]));
    assert_eq!(
        metadata["diagnostics"],
        serde_json::json!([
            { "path": "lib/c.py", "code": "parse-error", "message": "syntax error" }
        ])
    );
    let nodes = body["nodes"].as_object().unwrap();
    assert_eq!(nodes.len(), graph.nodes.len());
    let module = &nodes["mod:py:app/a.py"];
    assert_eq!(module["label"], "app/a.py");
    assert_eq!(
        module["metadata"]["tags"]["owner"],
        serde_json::json!(["team-a", "team-b"])
    );
    let symbol = &nodes["sym:py:app/a.py#run"]["metadata"];
    assert_eq!(symbol["symbol_kind"], "function");
    assert_eq!(symbol["span"]["start_line"], 5);
    assert_eq!(
        symbol["binding"],
        serde_json::json!({ "framework": "pyo3", "exported_name": "run" })
    );
    let external = &nodes["ext:py:requests"]["metadata"];
    assert!(external.get("path").is_none() && external.get("span").is_none());
    assert_eq!(external["tags"]["boundary"], serde_json::json!(["network"]));
    let edges = body["edges"].as_array().unwrap();
    assert_eq!(edges.len(), graph.edges.len());
    let contains = edges
        .iter()
        .find(|edge| edge["relation"] == "contains")
        .unwrap();
    assert_eq!(contains["metadata"], serde_json::json!({}));
    let import = edges
        .iter()
        .find(|edge| edge["target"] == "ext:py:requests")
        .unwrap();
    assert_eq!(import["relation"], "imports");
    assert_eq!(
        import["metadata"],
        serde_json::json!({
            "confidence": "resolved",
            "origin": "tree-sitter",
            "specifier": "requests",
            "count": 1,
            "site": {
                "path": "app/a.py",
                "span": { "start_line": 1, "start_column": 1, "end_line": 1, "end_column": 16 }
            }
        })
    );
    let folded = edges
        .iter()
        .find(|edge| edge["source"] == "mod:py:lib/c.py")
        .unwrap();
    assert_eq!(folded["metadata"]["count"], 2);
}

// ADR 0004: JSON "always carries everything", whatever the view says.
#[test]
fn the_json_document_ignores_the_view() {
    let graph = graph();
    let render = |level, group_by| Json.render(&graph, &View::new(&graph, level, group_by));
    let full = render(Level::Symbol, None);
    assert_eq!(full, render(Level::Package, Some("layer")));
}
