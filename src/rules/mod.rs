//! The rules engine (ADR 0004, "Evaluation"): judges every import, bind and
//! call site of a [`Graph`] against a [`Config`].
//!
//! [`check`] returns an [`Outcome`]:
//!
//! - **Violations**, one per site: the layers rule first, then the explicit
//!   rules in file order (the first match decides), then the `default`
//!   effect. A violation carries the rule id, the severity, the site's
//!   position and, for an edge, the module-level endpoints it was judged by.
//! - **Suppressions:** a `modfence: ignore[...]` comment on the site's first
//!   line, or the line above, removes the violations of the ids it names. One
//!   that removed nothing is reported as `modfence/unused-ignore`; the graph's
//!   annotation findings are reported as `modfence/invalid-annotation`.
//! - **Notices:** facts about the run that are not violations (call edges not
//!   checked, a rule that can never match, `layers` with no `layer` tag).
//!
//! The engine reads the graph and the config and nothing else; the output is
//! sorted, so it is deterministic.

mod eval;
mod lint;

use std::collections::BTreeSet;

use crate::config::{Config, Severity};
use crate::extract::{NodeId, Span, Tags};
use crate::graph::{Confidence, Graph, INVALID_ANNOTATION, Relation, Suppression};

/// The tag key that `layers` judges.
pub(crate) const LAYER_KEY: &str = "layer";

/// The reserved id of the layers rule.
pub const LAYERS: &str = "layers";

/// The reserved id of the default-deny rule.
pub const DEFAULT_DENY: &str = "default-deny";

/// The built-in id of an ignore that suppressed nothing.
pub const UNUSED_IGNORE: &str = "modfence/unused-ignore";

/// The notice printed when call edges were not checked (no `--precise`).
pub const CALLS_NOT_CHECKED: &str = "call edges were not checked; run with --precise";

/// GitHub code scanning keeps only this many results per run.
pub const GITHUB_RESULT_LIMIT: usize = 5000;

/// One end of a checked dependency: a module, or an external package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    /// The node id.
    pub id: NodeId,
    /// The file, or `None` for an external node.
    pub path: Option<String>,
    /// The node's effective tags.
    pub tags: Tags,
}

/// The edge a violation was found on, at module level.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dependency {
    /// The importing or calling module.
    pub from: Endpoint,
    /// The module or external package it depends on.
    pub to: Endpoint,
    /// The edge's relation.
    pub relation: Relation,
    /// The edge's confidence.
    pub confidence: Confidence,
}

/// A violation, at the site that caused it.
///
/// The field order is the sort order of an [`Outcome`] (ADR 0004,
/// invariant 19): severity, rule id, path and position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    /// How serious it is.
    pub severity: Severity,
    /// The rule id: a config rule, `layers`, `default-deny` or a `modfence/*`
    /// built-in.
    pub rule: String,
    /// The file of the site.
    pub path: String,
    /// The import statement, the callee or the comment.
    pub span: Span,
    /// The edge; `None` for the annotation built-ins.
    pub dependency: Option<Dependency>,
    /// A message for people.
    pub message: String,
}

/// A fact about the run that is not a violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// `note` for information, `warning` for something to fix.
    pub level: Severity,
    /// A message for people.
    pub message: String,
}

/// What [`check`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Every violation, sorted by severity, rule id, path and position.
    pub violations: Vec<Violation>,
    /// The notices, in the order they were found.
    pub notices: Vec<Notice>,
    /// Whether call edges could be resolved (`--precise`); when not, call
    /// edges were not checked.
    pub precise: bool,
}

impl Outcome {
    /// Whether any violation is at or above `fail_on` (`error` is the highest
    /// severity), which makes the exit status 1 (ADR 0004, invariant 21).
    #[must_use]
    pub fn fails(&self, fail_on: Severity) -> bool {
        self.violations
            .iter()
            .any(|violation| violation.severity <= fail_on)
    }
}

/// The relation a rule sees for a graph edge; `None` for `contains`.
fn rule_relation(relation: Relation) -> Option<crate::config::Relation> {
    match relation {
        Relation::Contains => None,
        Relation::Imports => Some(crate::config::Relation::Imports),
        Relation::Calls => Some(crate::config::Relation::Calls),
        Relation::Binds => Some(crate::config::Relation::Binds),
    }
}

/// Which `(suppression, rule id)` pairs have suppressed something.
struct Ignores<'a> {
    all: &'a [Suppression],
    used: BTreeSet<(usize, usize)>,
}

impl<'a> Ignores<'a> {
    fn new(all: &'a [Suppression]) -> Self {
        Self {
            all,
            used: BTreeSet::new(),
        }
    }

    /// Whether a suppression other than `skip` names `rule` and covers a site
    /// starting on `line` of `path` (its own line or the line below it). Marks
    /// every suppression that does as used.
    fn suppresses(&mut self, path: &str, line: u32, rule: &str, skip: Option<usize>) -> bool {
        let mut found = false;
        for (index, ignore) in self.all.iter().enumerate() {
            let start = ignore.span.start_line;
            if Some(index) == skip
                || ignore.path != path
                || (start != line && start.saturating_add(1) != line)
            {
                continue;
            }
            for (slot, id) in ignore.rules.iter().enumerate() {
                if id == rule {
                    self.used.insert((index, slot));
                    found = true;
                }
            }
        }
        found
    }

    /// The `(suppression, slot, id)` triples that suppressed nothing so far.
    fn unused(&self) -> Vec<(usize, &'a Suppression, &'a str)> {
        let mut unused = Vec::new();
        for (index, ignore) in self.all.iter().enumerate() {
            for (slot, id) in ignore.rules.iter().enumerate() {
                if !self.used.contains(&(index, slot)) {
                    unused.push((index, ignore, id.as_str()));
                }
            }
        }
        unused
    }
}

fn unused_ignore(ignore: &Suppression, id: &str) -> Violation {
    Violation {
        severity: Severity::Warning,
        rule: UNUSED_IGNORE.to_owned(),
        path: ignore.path.clone(),
        span: ignore.span,
        dependency: None,
        message: format!("ignore[{id}] suppressed nothing"),
    }
}

/// The violations of every site of every checked edge.
fn edge_violations(graph: &Graph, config: &Config, ledger: &mut Ignores<'_>) -> Vec<Violation> {
    let judge = eval::Judge::new(graph, config);
    let mut violations = Vec::new();
    for edge in &graph.edges {
        let kind = (rule_relation(edge.relation), edge.confidence);
        let (Some(relation), Some(confidence)) = kind else {
            continue;
        };
        let source = graph.nodes.get(graph.rollup(&edge.source));
        let target = graph.nodes.get(graph.rollup(&edge.target));
        let (Some(from), Some(to)) = (source, target) else {
            continue;
        };
        if from.id == to.id {
            continue;
        }
        for site in &edge.sites {
            let Some(hit) = judge.site(from, to, relation, confidence) else {
                continue;
            };
            if ledger.suppresses(&site.path, site.span.start_line, &hit.rule, None) {
                continue;
            }
            violations.push(Violation {
                severity: hit.severity,
                rule: hit.rule,
                path: site.path.clone(),
                span: site.span,
                dependency: Some(Dependency {
                    from: eval::endpoint(from),
                    to: eval::endpoint(to),
                    relation: edge.relation,
                    confidence,
                }),
                message: hit.message,
            });
        }
    }
    violations
}

/// The graph's annotation findings, minus the suppressed ones.
fn annotation_violations(graph: &Graph, ledger: &mut Ignores<'_>) -> Vec<Violation> {
    let mut violations = Vec::new();
    for finding in &graph.findings {
        let line = finding.span.start_line;
        if ledger.suppresses(&finding.path, line, INVALID_ANNOTATION, None) {
            continue;
        }
        violations.push(Violation {
            severity: finding.severity,
            rule: INVALID_ANNOTATION.to_owned(),
            path: finding.path.clone(),
            span: finding.span,
            dependency: None,
            message: finding.message.clone(),
        });
    }
    violations
}

/// The ignores that suppressed nothing. An unused ignore can itself be
/// ignored by another comment, which then counts as used; an unused ignore of
/// `modfence/unused-ignore` is reported last, since nothing can suppress that.
fn unused_violations(ledger: &mut Ignores<'_>) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (index, ignore, id) in ledger.unused() {
        if id == UNUSED_IGNORE {
            continue;
        }
        let start = ignore.span.start_line;
        if !ledger.suppresses(&ignore.path, start, UNUSED_IGNORE, Some(index)) {
            violations.push(unused_ignore(ignore, id));
        }
    }
    for (_, ignore, id) in ledger.unused() {
        if id == UNUSED_IGNORE {
            violations.push(unused_ignore(ignore, id));
        }
    }
    violations
}

/// Judges every site of `graph` against `config`.
///
/// `precise` says whether SCIP indexes were ingested; without them no call
/// edge is `resolved`, so call edges are not checked, and the outcome says so
/// in a notice.
#[must_use]
pub fn check(graph: &Graph, config: &Config, precise: bool) -> Outcome {
    let mut ledger = Ignores::new(&graph.suppressions);
    let mut violations = edge_violations(graph, config, &mut ledger);
    violations.extend(annotation_violations(graph, &mut ledger));
    violations.extend(unused_violations(&mut ledger));
    violations.sort();
    // One import statement can yield an `imports` and a `binds` edge at the
    // same site; a site is reported once per rule (ADR 0004, invariant 9).
    violations.dedup_by(|b, a| a.rule == b.rule && a.path == b.path && a.span == b.span);
    let mut notices = Vec::new();
    if !precise {
        notices.push(Notice {
            level: Severity::Note,
            message: CALLS_NOT_CHECKED.to_owned(),
        });
    }
    notices.extend(lint::notices(graph, config));
    if violations.len() > GITHUB_RESULT_LIMIT {
        notices.push(Notice {
            level: Severity::Warning,
            message: format!(
                "{} results; GitHub code scanning keeps only the top {GITHUB_RESULT_LIMIT}",
                violations.len()
            ),
        });
    }
    Outcome {
        violations,
        notices,
        precise,
    }
}
