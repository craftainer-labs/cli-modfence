//! Mermaid (`--format mermaid`): the view as a `flowchart LR`, one `subgraph`
//! per cluster. Targets Mermaid 10 and later (classic flowchart syntax, no
//! `@{ shape }` nodes), so mermaid-cli renders it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::{Renderer, View};
use crate::extract::{NodeId, NodeKind};
use crate::graph::{Graph, Node, Relation};

/// The Mermaid flowchart of the view.
///
/// Nodes are `n0`, `n1`, ... in node order, so no id or label is ever parsed
/// as syntax; clusters are `c0`, `c1`, .... Shapes follow the HTML legend as
/// far as Mermaid has them (package rectangle, module stadium, external
/// package rhombus, symbol hexagon, which stands in for the triangle Mermaid
/// lacks before version 11.3). Every edge is `-->`; a `linkStyle` per relation
/// gives the line style (calls dashed, binds dotted, contains light grey),
/// and every edge but a plain import is labelled with its relation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mermaid;

/// `text` as the inside of a Mermaid quoted string: `"` and the characters
/// that Mermaid reads as markup (`#`, `<`, `>`, `&`, a backtick, which starts
/// a markdown string) as entity codes, any control character as a space.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("#quot;"),
            '#' => out.push_str("#35;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            '`' => out.push_str("#96;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn node(name: &str, kind: NodeKind, label: &str) -> String {
    let label = quote(label);
    match kind {
        NodeKind::Package => format!("{name}[\"{label}\"]"),
        NodeKind::Module => format!("{name}([\"{label}\"])"),
        NodeKind::External => format!("{name}{{\"{label}\"}}"),
        NodeKind::Symbol => format!("{name}{{{{\"{label}\"}}}}"),
    }
}

impl Renderer for Mermaid {
    fn render(&self, _graph: &Graph, view: &View<'_>) -> String {
        let mut out = String::from("flowchart LR\n");
        let names: BTreeMap<&NodeId, String> = view
            .nodes
            .iter()
            .enumerate()
            .map(|(number, node)| (&node.id, format!("n{number}")))
            .collect();
        let nodes: BTreeMap<&NodeId, &Node> =
            view.nodes.iter().map(|shown| (&shown.id, *shown)).collect();
        let declare = |out: &mut String, indent: &str, id: &NodeId| {
            let shown = nodes[&id];
            let _ = writeln!(
                out,
                "{indent}{}",
                node(&names[&id], shown.kind, &shown.label)
            );
        };
        let mut clustered = BTreeSet::new();
        for (number, cluster) in view.clusters.iter().enumerate() {
            let _ = writeln!(out, "  subgraph c{number}[\"{}\"]", quote(&cluster.label));
            for member in &cluster.members {
                declare(&mut out, "    ", member);
                clustered.insert(*member);
            }
            out.push_str("  end\n");
        }
        for shown in &view.nodes {
            if !clustered.contains(&shown.id) {
                declare(&mut out, "  ", &shown.id);
            }
        }
        let mut styles: BTreeMap<Relation, Vec<usize>> = BTreeMap::new();
        for (number, edge) in view.edges.iter().enumerate() {
            let mut label = Vec::new();
            if edge.relation != Relation::Imports {
                label.push(edge.relation.as_str().to_owned());
            }
            if edge.count > 1 {
                label.push(edge.count.to_string());
            }
            let _ = writeln!(
                out,
                "  {} -->{} {}",
                names[&edge.source],
                if label.is_empty() {
                    String::new()
                } else {
                    format!("|\"{}\"|", label.join(" "))
                },
                names[&edge.target]
            );
            styles.entry(edge.relation).or_default().push(number);
        }
        for (relation, numbers) in &styles {
            let line = match relation {
                Relation::Imports => continue,
                Relation::Calls => "stroke-dasharray:6 3",
                Relation::Binds => "stroke-dasharray:2 3",
                Relation::Contains => "stroke:#bfbfbf",
            };
            let list: Vec<String> = numbers.iter().map(ToString::to_string).collect();
            let _ = writeln!(out, "  linkStyle {} {line}", list.join(","));
        }
        out
    }
}
