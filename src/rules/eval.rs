//! Judging one site: the layers rule, the explicit rules and the default
//! effect (ADR 0004, "Evaluation", "Default effect" and "Layers").

use std::collections::BTreeSet;

use super::{DEFAULT_DENY, Endpoint, LAYERS, LAYER_KEY};
use crate::config::{ConfidenceReq, Config, DefaultEffect, Effect, Relation, Severity, Subject};
use crate::extract::{NodeId, NodeKind};
use crate::graph::{Confidence, Graph, Node, Relation as EdgeRelation};

/// What decided a site to be a violation.
pub(super) struct Hit {
    pub(super) rule: String,
    pub(super) severity: Severity,
    pub(super) message: String,
}

/// What `layers` says about a pair of nodes.
#[derive(PartialEq, Eq)]
enum Layered {
    /// Some combination of their layers points upwards; the message says which.
    Violates(String),
    /// Every layer value of both nodes is listed and none points upwards.
    Permits,
    /// A node has no listed layer, or `layers` is not set: layers say nothing.
    Exempt,
}

/// Whether an edge of `relation` and `confidence` counts for a rule that asks
/// for `required` (ADR 0004, "Call edges in rules"): with nothing asked, calls
/// count only when resolved; `resolved` counts only resolved edges; `any`
/// counts every edge.
fn counts(relation: Relation, confidence: Confidence, required: Option<ConfidenceReq>) -> bool {
    match required {
        Some(ConfidenceReq::Any) => true,
        Some(ConfidenceReq::Resolved) => confidence == Confidence::Resolved,
        None => relation != Relation::Calls || confidence == Confidence::Resolved,
    }
}

/// How a node is named in a message.
fn name(node: &Node) -> &str {
    node.path.as_deref().unwrap_or(&node.label)
}

/// The report's view of a node.
pub(super) fn endpoint(node: &Node) -> Endpoint {
    Endpoint {
        id: node.id.clone(),
        path: node.path.clone(),
        tags: node.tags.clone(),
    }
}

/// Judges sites against one config.
pub(super) struct Judge<'a> {
    config: &'a Config,
    /// External nodes made for an import that could not be placed: they are
    /// not packages, so a `package` selector never matches them (ADR 0003,
    /// invariant 6).
    unplaced: BTreeSet<&'a NodeId>,
}

impl<'a> Judge<'a> {
    pub(super) fn new(graph: &'a Graph, config: &'a Config) -> Self {
        let unplaced = graph
            .edges
            .iter()
            .filter(|edge| {
                edge.relation == EdgeRelation::Imports
                    && edge.confidence == Some(Confidence::Heuristic)
            })
            .map(|edge| &edge.target)
            .collect();
        Self { config, unplaced }
    }

    fn subject<'n>(&self, node: &'n Node) -> Subject<'n> {
        let package = node.kind == NodeKind::External && !self.unplaced.contains(&node.id);
        Subject {
            kind: node.kind,
            lang: node.lang,
            path: node.path.as_deref(),
            package: package.then_some(node.label.as_str()),
            tags: &node.tags,
        }
    }

    /// What `layers` says about an edge from `from` to `to`. Every pair of
    /// listed `layer` values is judged; any pair pointing from a lower layer to
    /// a higher one violates.
    fn layered(&self, from: &Node, to: &Node) -> Layered {
        let values = (from.tags.get(LAYER_KEY), to.tags.get(LAYER_KEY));
        let (Some(from_layers), Some(to_layers)) = values else {
            return Layered::Exempt;
        };
        let position = |value: &str| self.config.layers.iter().position(|l| l.as_str() == value);
        let mut all_listed = true;
        for source in from_layers {
            for target in to_layers {
                match (position(source.as_str()), position(target.as_str())) {
                    (Some(low), Some(high)) if low > high => {
                        return Layered::Violates(format!(
                            "layer `{source}` ({}) must not depend on layer `{target}` ({})",
                            name(from),
                            name(to)
                        ));
                    }
                    (Some(_), Some(_)) => {}
                    _ => all_listed = false,
                }
            }
        }
        if all_listed {
            Layered::Permits
        } else {
            Layered::Exempt
        }
    }

    /// The violation at one site of an edge from module `from` to `to`, if
    /// any: layers first, then the explicit rules in file order (the first
    /// match decides), then the default.
    pub(super) fn site(
        &self,
        from: &Node,
        to: &Node,
        relation: Relation,
        confidence: Confidence,
    ) -> Option<Hit> {
        let by_default = counts(relation, confidence, None);
        let layered = if by_default {
            self.layered(from, to)
        } else {
            Layered::Exempt
        };
        let permits = layered == Layered::Permits;
        if let Layered::Violates(message) = layered {
            return Some(Hit {
                rule: LAYERS.to_owned(),
                severity: Severity::Error,
                message,
            });
        }
        let (source, target) = (self.subject(from), self.subject(to));
        for rule in &self.config.rules {
            let matched = rule.kinds.contains(&relation)
                && counts(relation, confidence, rule.confidence)
                && rule.from.matches(&source)
                && rule.to.matches(&target);
            if !matched {
                continue;
            }
            return match rule.effect {
                Effect::Allow => None,
                Effect::Forbid => Some(Hit {
                    rule: rule.id.clone(),
                    severity: rule.severity,
                    message: rule.message.clone().unwrap_or_else(|| {
                        format!(
                            "{} must not depend on {} (rule `{}`)",
                            name(from),
                            name(to),
                            rule.id
                        )
                    }),
                }),
            };
        }
        let internal = from.kind != NodeKind::External && to.kind != NodeKind::External;
        let denied =
            self.config.default == DefaultEffect::Deny && by_default && internal && !permits;
        denied.then(|| Hit {
            rule: DEFAULT_DENY.to_owned(),
            severity: Severity::Error,
            message: "No rule allows this dependency.".to_owned(),
        })
    }
}
