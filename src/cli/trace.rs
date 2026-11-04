//! `modfence trace`: the selector arguments and the text output (ADR 0004,
//! "Traceability outputs").

use std::collections::BTreeSet;

use toml::{Table, Value};

use super::Failure;
use crate::config::{Selector, Subject};
use crate::extract::{NodeId, NodeKind, Tags};
use crate::graph::{Graph, Node};

/// Splits the value of `key=value` at commas outside double quotes and drops
/// the quotes; `None` when a quote is left open.
fn split_values(raw: &str) -> Option<Vec<String>> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in raw.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => values.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    values.push(current);
    (!quoted).then_some(values)
}

/// Reads the selector arguments into one [`Selector`]. The keys and values go
/// through the same parsers as a rule's selector in `modfence.toml`.
pub(super) fn parse(args: &[String]) -> Result<Selector, Failure> {
    let mut table = Table::new();
    for arg in args {
        let (key, raw) = arg
            .split_once('=')
            .ok_or_else(|| Failure::usage(format!("`{arg}` is not a key=value selector")))?;
        let values = split_values(raw)
            .ok_or_else(|| Failure::usage(format!("`{arg}` has an unterminated double quote")))?;
        let array = Value::Array(values.into_iter().map(Value::String).collect());
        if table.insert(key.to_owned(), array).is_some() {
            return Err(Failure::usage(format!(
                "`{key}` is given twice; separate alternatives with commas"
            )));
        }
    }
    Value::Table(table)
        .try_into::<Selector>()
        .map_err(|error| Failure::usage(error.message().to_owned()))
}

/// A node's tags as `key=a,b key=c`.
fn tags(tags: &Tags) -> String {
    let items: Vec<String> = tags
        .iter()
        .map(|(key, values)| {
            let values: Vec<&str> = values.iter().map(String::as_str).collect();
            format!("{key}={}", values.join(","))
        })
        .collect();
    items.join(" ")
}

/// Whether `selector` selects `node`.
fn selected(selector: &Selector, node: &Node) -> bool {
    selector.matches(&Subject {
        kind: node.kind,
        lang: node.lang,
        path: node.path.as_deref(),
        package: (node.kind == NodeKind::External).then_some(node.label.as_str()),
        tags: &node.tags,
    })
}

/// The matching nodes and the edges with a matching endpoint.
pub(super) fn render(graph: &Graph, selector: &Selector) -> String {
    let nodes: Vec<&Node> = graph
        .nodes
        .values()
        .filter(|node| selected(selector, node))
        .collect();
    let ids: BTreeSet<&NodeId> = nodes.iter().map(|node| &node.id).collect();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| ids.contains(&edge.source) || ids.contains(&edge.target))
        .collect();
    let mut lines = vec![format!("nodes: {}", nodes.len())];
    for node in &nodes {
        let line = format!("  {}  {}", node.id, tags(&node.tags));
        lines.push(line.trim_end().to_owned());
    }
    lines.push(format!("edges: {}", edges.len()));
    for edge in &edges {
        let confidence = edge
            .confidence
            .map_or(String::new(), |confidence| format!(" {}", confidence.as_str()));
        lines.push(format!(
            "  {} -> {}  {}{confidence}",
            edge.source,
            edge.target,
            edge.relation.as_str(),
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}
