//! The graph document (ADR 0003, "Output format"): a JSON Graph Format v2
//! specialisation described by `schema/modfence-graph.schema.json`.

use serde_json::{Map, Value, json};

use super::{Renderer, View};
use crate::extract::{Lang, Span};
use crate::graph::{Edge, FileDiagnostic, Graph, Node};

/// The `major.minor` version of the document format.
pub const SCHEMA_VERSION: &str = "1.0";

/// The graph document. It always carries every node, edge and diagnostic of
/// the graph it is given, whatever the view says.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json;

impl Renderer for Json {
    fn render(&self, graph: &Graph, _view: &View<'_>) -> String {
        format!("{:#}\n", document(graph))
    }
}

fn span(span: Span) -> Value {
    json!({
        "start_line": span.start_line,
        "start_column": span.start_column,
        "end_line": span.end_line,
        "end_column": span.end_column,
    })
}

fn node(node: &Node) -> Value {
    let mut metadata = Map::new();
    metadata.insert("kind".to_owned(), json!(node.kind.as_str()));
    metadata.insert("lang".to_owned(), json!(node.lang.as_str()));
    if let Some(path) = &node.path {
        metadata.insert("path".to_owned(), json!(path));
    }
    if let Some(kind) = node.symbol_kind {
        metadata.insert("symbol_kind".to_owned(), json!(kind.as_str()));
    }
    if let Some(at) = node.span {
        metadata.insert("span".to_owned(), span(at));
    }
    if !node.tags.is_empty() {
        metadata.insert("tags".to_owned(), json!(node.tags));
    }
    if let Some(binding) = &node.binding {
        metadata.insert(
            "binding".to_owned(),
            json!({
                "framework": binding.framework.as_str(),
                "exported_name": binding.exported_name,
            }),
        );
    }
    json!({ "label": node.label, "metadata": metadata })
}

fn edge(edge: &Edge) -> Value {
    let mut metadata = Map::new();
    if let Some(confidence) = edge.confidence {
        metadata.insert("confidence".to_owned(), json!(confidence.as_str()));
        metadata.insert("origin".to_owned(), json!("tree-sitter"));
        metadata.insert("count".to_owned(), json!(edge.count()));
    }
    if let Some(specifier) = &edge.specifier {
        metadata.insert("specifier".to_owned(), json!(specifier));
    }
    if let Some(site) = edge.site() {
        metadata.insert(
            "site".to_owned(),
            json!({ "path": site.path, "span": span(site.span) }),
        );
    }
    json!({
        "source": edge.source.as_str(),
        "target": edge.target.as_str(),
        "relation": edge.relation.as_str(),
        "metadata": metadata,
    })
}

fn diagnostic(diagnostic: &FileDiagnostic) -> Value {
    json!({
        "path": diagnostic.path,
        "code": diagnostic.code.as_str(),
        "message": diagnostic.message,
    })
}

/// The document for `graph`.
fn document(graph: &Graph) -> Value {
    let nodes: Map<String, Value> = graph
        .nodes
        .iter()
        .map(|(id, graph_node)| (id.as_str().to_owned(), node(graph_node)))
        .collect();
    let languages: Vec<&str> = graph.languages.iter().copied().map(Lang::as_str).collect();
    let diagnostics: Vec<Value> = graph.diagnostics.iter().map(diagnostic).collect();
    let edges: Vec<Value> = graph.edges.iter().map(edge).collect();
    json!({
        "graph": {
            "directed": true,
            "type": "modfence.dependency-graph",
            "metadata": {
                "schema_version": SCHEMA_VERSION,
                "tool": { "name": "modfence", "version": env!("CARGO_PKG_VERSION") },
                "root": ".",
                "backend": "tree-sitter",
                "languages": languages,
                "diagnostics": diagnostics,
            },
            "nodes": nodes,
            "edges": edges,
        },
    })
}
