//! `modfence trace`: the selector arguments and the text output (ADR 0004,
//! "Traceability outputs").

use std::collections::BTreeSet;

use toml::{Table, Value};

use super::Failure;
use crate::config::Selector;
use crate::extract::{NodeId, Tags};
use crate::graph::{Confidence, Graph, Node, Relation};
use crate::rules::subject;

/// Splits the value of `key=value` into its alternatives at commas outside
/// double quotes, and drops the quotes. An alternative is either quoted as a
/// whole or has no quote at all, and none is empty.
fn split_values(raw: &str) -> Result<Vec<String>, &'static str> {
    let mut values = Vec::new();
    let mut rest = raw;
    loop {
        let (value, after) = match rest.strip_prefix('"') {
            Some(quoted) => quoted
                .split_once('"')
                .ok_or("has an unterminated double quote")?,
            None => rest.split_at(rest.find(',').unwrap_or(rest.len())),
        };
        if value.contains('"') || !(after.is_empty() || after.starts_with(',')) {
            return Err("has a double quote that does not wrap a whole value");
        }
        if value.is_empty() {
            return Err("has an empty value");
        }
        values.push(value.to_owned());
        match after.strip_prefix(',') {
            Some(next) => rest = next,
            None => return Ok(values),
        }
    }
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
            .map_err(|reason| Failure::usage(format!("`{arg}` {reason}")))?;
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

/// The graph's `external` nodes that exist only for imports that could not be
/// placed: every import of them is a heuristic one.
fn unplaced(graph: &Graph) -> BTreeSet<&NodeId> {
    let targets = |confidence| -> BTreeSet<&NodeId> {
        graph
            .edges
            .iter()
            .filter(|edge| edge.relation == Relation::Imports && edge.confidence == confidence)
            .map(|edge| &edge.target)
            .collect()
    };
    let placed = targets(Some(Confidence::Resolved));
    let guessed = targets(Some(Confidence::Heuristic));
    guessed.difference(&placed).copied().collect()
}

/// The ids of the nodes `selector` selects. A node is judged by the same
/// subject as in a rule of `modfence.toml` (`rules::subject`).
pub(super) fn select(graph: &Graph, selector: &Selector) -> BTreeSet<NodeId> {
    let unplaced = unplaced(graph);
    graph
        .nodes
        .values()
        .filter(|node| selector.matches(&subject(node, unplaced.contains(&node.id))))
        .map(|node| node.id.clone())
        .collect()
}

/// The selected nodes and the edges of `part` (see [`output::incident`]).
///
/// [`output::incident`]: crate::output::incident
pub(super) fn render(part: &Graph, selected: &BTreeSet<NodeId>) -> String {
    let nodes: Vec<&Node> = part
        .nodes
        .values()
        .filter(|node| selected.contains(&node.id))
        .collect();
    let mut lines = vec![format!("nodes: {}", nodes.len())];
    for node in &nodes {
        let line = format!("  {}  {}", node.id, tags(&node.tags));
        lines.push(line.trim_end().to_owned());
    }
    lines.push(format!("edges: {}", part.edges.len()));
    for edge in &part.edges {
        let confidence = edge.confidence.map_or(String::new(), |confidence| {
            format!(" {}", confidence.as_str())
        });
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
