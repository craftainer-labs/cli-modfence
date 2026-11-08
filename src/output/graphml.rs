//! `GraphML` (`--format graphml`): the whole graph as XML for graph tools.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Value, json};

use super::{Renderer, View};
use crate::extract::{NodeId, Span};
use crate::graph::{Edge, Graph, Node};

/// The `GraphML` document of the graph.
///
/// A data format, like [`Json`](super::Json): it always carries every node
/// and edge and ignores the view. Every node and edge attribute of the JSON
/// document is a `<key>` and a `<data>` element; structured values (tags,
/// span, binding, sites) are JSON text in a string attribute. The graph is
/// `edgedefault="directed"`. Nodes are `n0`, `n1`, ... in sorted id order (the
/// real id is the `node_id` data), so ids XML cannot carry never collide.
#[derive(Debug, Clone, Copy, Default)]
pub struct GraphMl;

/// The `<key>` declarations: id, scope, name and `attr.type`.
const KEYS: [(&str, &str, &str, &str); 15] = [
    ("node_id", "node", "id", "string"),
    ("n_kind", "node", "kind", "string"),
    ("n_label", "node", "label", "string"),
    ("n_lang", "node", "lang", "string"),
    ("n_path", "node", "path", "string"),
    ("n_symbol_kind", "node", "symbol_kind", "string"),
    ("n_span", "node", "span", "string"),
    ("n_tags", "node", "tags", "string"),
    ("n_binding", "node", "binding", "string"),
    ("e_relation", "edge", "relation", "string"),
    ("e_confidence", "edge", "confidence", "string"),
    ("e_origin", "edge", "origin", "string"),
    ("e_count", "edge", "count", "int"),
    ("e_specifier", "edge", "specifier", "string"),
    ("e_sites", "edge", "sites", "string"),
];

/// Whether XML 1.0 allows `c` in a document.
fn valid(c: char) -> bool {
    let code = u32::from(c);
    matches!(code, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF)
}

/// `text` as XML character data or an attribute value: the five markup
/// characters and line breaks and tabs as references (so an attribute keeps
/// them), and every character XML 1.0 forbids as U+FFFD.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if valid(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

fn data(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "      <data key=\"{key}\">{}</data>", escape(value));
}

fn span(span: Span) -> Value {
    json!({
        "start_line": span.start_line,
        "start_column": span.start_column,
        "end_line": span.end_line,
        "end_column": span.end_column,
    })
}

fn node(out: &mut String, number: usize, node: &Node) {
    let _ = writeln!(out, "    <node id=\"n{number}\">");
    data(out, "node_id", node.id.as_str());
    data(out, "n_kind", node.kind.as_str());
    data(out, "n_label", &node.label);
    data(out, "n_lang", node.lang.as_str());
    if let Some(path) = &node.path {
        data(out, "n_path", path);
    }
    if let Some(kind) = node.symbol_kind {
        data(out, "n_symbol_kind", kind.as_str());
    }
    if let Some(at) = node.span {
        data(out, "n_span", &span(at).to_string());
    }
    if !node.tags.is_empty() {
        data(out, "n_tags", &json!(node.tags).to_string());
    }
    if let Some(binding) = &node.binding {
        let value = json!({
            "framework": binding.framework.as_str(),
            "exported_name": binding.exported_name,
        });
        data(out, "n_binding", &value.to_string());
    }
    out.push_str("    </node>\n");
}

fn edge(out: &mut String, number: usize, numbers: &BTreeMap<&NodeId, usize>, edge: &Edge) {
    let _ = writeln!(
        out,
        "    <edge id=\"e{number}\" source=\"n{}\" target=\"n{}\">",
        numbers[&edge.source], numbers[&edge.target]
    );
    data(out, "e_relation", edge.relation.as_str());
    if let Some(confidence) = edge.confidence {
        data(out, "e_confidence", confidence.as_str());
        data(out, "e_origin", edge.origin().as_str());
        data(out, "e_count", &edge.count().to_string());
    }
    if let Some(specifier) = &edge.specifier {
        data(out, "e_specifier", specifier);
    }
    if !edge.sites.is_empty() {
        let sites: Vec<Value> = edge
            .sites
            .iter()
            .map(|site| json!({ "path": site.path, "span": span(site.span) }))
            .collect();
        data(out, "e_sites", &Value::Array(sites).to_string());
    }
    out.push_str("    </edge>\n");
}

impl Renderer for GraphMl {
    fn render(&self, graph: &Graph, _view: &View<'_>) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n",
        );
        for (id, scope, name, kind) in KEYS {
            let _ = writeln!(
                out,
                "  <key id=\"{id}\" for=\"{scope}\" attr.name=\"{name}\" attr.type=\"{kind}\"/>"
            );
        }
        out.push_str("  <graph id=\"modfence\" edgedefault=\"directed\">\n");
        let numbers: BTreeMap<&NodeId, usize> = graph
            .nodes
            .keys()
            .enumerate()
            .map(|(number, id)| (id, number))
            .collect();
        for (number, graph_node) in graph.nodes.values().enumerate() {
            node(&mut out, number, graph_node);
        }
        for (number, graph_edge) in graph.edges.iter().enumerate() {
            edge(&mut out, number, &numbers, graph_edge);
        }
        out.push_str("  </graph>\n</graphml>\n");
        out
    }
}
