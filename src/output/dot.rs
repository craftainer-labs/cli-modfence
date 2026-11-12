//! Graphviz DOT (`--format dot`): the view as a `digraph`, one `subgraph
//! cluster_<n>` per cluster.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::{Renderer, View};
use crate::extract::{NodeId, NodeKind};
use crate::graph::{Graph, Node, Relation};

/// The DOT document of the view (Graphviz `digraph`).
///
/// Node shapes follow the HTML legend (package box, module ellipse, external
/// package diamond, symbol triangle) and edge styles the relation (imports
/// solid, calls dashed, binds dotted, contains solid and light grey). The
/// arrowhead is on the target. Nodes are named `n0`, `n1`, ... in sorted id
/// order, so ids that differ only in characters DOT cannot carry stay distinct
/// nodes; the real id is the node's quoted `id` attribute. A node with tags has
/// a `tooltip` of `key: value, value` lines, so Graphviz viewers show them on
/// hover (ADR 0004, "Traceability outputs").
#[derive(Debug, Clone, Copy, Default)]
pub struct Dot;

/// `text` as the inside of a DOT quoted string: `"` and `\` escaped, a line
/// break written as `\n`, any other control character replaced by a space.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The `, tooltip="..."` attribute of `node`: one `key: value, value` line per
/// tag key (keys and values in their stored, sorted order). Empty for a node
/// without tags, so it gets no tooltip.
fn tooltip(node: &Node) -> String {
    if node.tags.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = node
        .tags
        .iter()
        .map(|(key, values)| {
            let values: Vec<&str> = values.iter().map(String::as_str).collect();
            format!("{key}: {}", values.join(", "))
        })
        .collect();
    format!(", tooltip=\"{}\"", quote(&lines.join("\n")))
}

const fn shape(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Package => "box",
        NodeKind::Module => "ellipse",
        NodeKind::External => "diamond",
        NodeKind::Symbol => "triangle",
    }
}

const fn style(relation: Relation) -> &'static str {
    match relation {
        Relation::Imports => "solid",
        Relation::Calls => "dashed",
        Relation::Binds => "dotted",
        Relation::Contains => "solid, color=\"gray75\"",
    }
}

impl Renderer for Dot {
    fn render(&self, _graph: &Graph, view: &View<'_>) -> String {
        let mut out = String::from("digraph \"modfence\" {\n  rankdir=LR;\n");
        let clustered: BTreeSet<&NodeId> = view
            .clusters
            .iter()
            .flat_map(|cluster| cluster.members.iter().copied())
            .collect();
        let nodes: BTreeMap<&NodeId, &Node> =
            view.nodes.iter().map(|node| (&node.id, *node)).collect();
        let numbers: BTreeMap<&NodeId, usize> = nodes
            .keys()
            .copied()
            .enumerate()
            .map(|(number, id)| (id, number))
            .collect();
        let declare = |out: &mut String, indent: &str, id: &NodeId| {
            let node = nodes[&id];
            let _ = writeln!(
                out,
                "{indent}n{} [id=\"{}\", label=\"{}\", shape={}{}];",
                numbers[&id],
                quote(id.as_str()),
                quote(&node.label),
                shape(node.kind),
                tooltip(node)
            );
        };
        for (number, cluster) in view.clusters.iter().enumerate() {
            let _ = writeln!(out, "  subgraph cluster_{number} {{");
            let _ = writeln!(out, "    label=\"{}\";", quote(&cluster.label));
            for member in &cluster.members {
                declare(&mut out, "    ", member);
            }
            out.push_str("  }\n");
        }
        for node in &view.nodes {
            if !clustered.contains(&node.id) {
                declare(&mut out, "  ", &node.id);
            }
        }
        for edge in &view.edges {
            let _ = write!(
                out,
                "  n{} -> n{} [style={}, arrowhead=normal",
                numbers[&edge.source],
                numbers[&edge.target],
                style(edge.relation)
            );
            if edge.count > 1 {
                let _ = write!(out, ", label=\"{}\"", edge.count);
            }
            out.push_str("];\n");
        }
        out.push_str("}\n");
        out
    }
}
