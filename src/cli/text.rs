//! Human-readable output: the default of every command.

use crate::config::Severity;
use crate::extract::{Lang, NodeKind};
use crate::graph::{FileDiagnostic, Graph, Relation};
use crate::rules::Outcome;

/// A severity as a lowercase word.
pub(super) fn label(severity: Severity) -> String {
    format!("{severity:?}").to_lowercase()
}

/// One line per violation, then a count line.
pub(super) fn violations(outcome: &Outcome) -> String {
    let mut lines: Vec<String> = outcome
        .violations
        .iter()
        .map(|violation| {
            format!(
                "{}:{}:{}: {} [{}] {}",
                violation.path,
                violation.span.start_line,
                violation.span.start_column,
                label(violation.severity),
                violation.rule,
                violation.message,
            )
        })
        .collect();
    let count = |severity: Severity| {
        outcome
            .violations
            .iter()
            .filter(|violation| violation.severity == severity)
            .count()
    };
    lines.push(format!(
        "modfence: {} error(s), {} warning(s), {} note(s)",
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Note),
    ));
    lines.push(String::new());
    lines.join("\n")
}

/// A file problem as a line for standard error.
pub(super) fn diagnostic(diagnostic: &FileDiagnostic) -> String {
    let line = diagnostic
        .span
        .map_or(String::new(), |span| format!(":{}", span.start_line));
    format!(
        "warning: {}{line}: {}",
        diagnostic.path, diagnostic.message
    )
}

/// The counts of a graph.
pub(super) fn summary(graph: &Graph) -> String {
    let languages: Vec<&str> = graph.languages.iter().copied().map(Lang::as_str).collect();
    let kinds = [
        NodeKind::Package,
        NodeKind::Module,
        NodeKind::Symbol,
        NodeKind::External,
    ]
    .map(|kind| {
        let count = graph.nodes.values().filter(|node| node.kind == kind).count();
        format!("{} {count}", kind.as_str())
    });
    let relations = [
        Relation::Contains,
        Relation::Imports,
        Relation::Calls,
        Relation::Binds,
    ]
    .map(|relation| {
        let count = graph
            .edges
            .iter()
            .filter(|edge| edge.relation == relation)
            .count();
        format!("{} {count}", relation.as_str())
    });
    format!(
        "modfence graph\nlanguages: {}\nnodes: {} ({})\nedges: {} ({})\ndiagnostics: {}\n",
        languages.join(", "),
        graph.nodes.len(),
        kinds.join(", "),
        graph.edges.len(),
        relations.join(", "),
        graph.diagnostics.len(),
    )
}
