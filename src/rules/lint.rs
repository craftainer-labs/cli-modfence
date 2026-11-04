//! Notices about the configuration that are not violations: a rule that can
//! never match and `layers` that no node can use.

use super::Notice;
use crate::config::{Config, Relation, Selector, Severity};
use crate::extract::NodeKind;
use crate::graph::Graph;

/// Whether `selector` can select no node at the end of a checked edge, where
/// `module` and `external` say which node kinds that end can have. Only
/// external nodes have a package and only the others have a path.
fn dead(selector: &Selector, module: bool, external: bool) -> bool {
    let wants = |kind| selector.kinds.is_empty() || selector.kinds.contains(&kind);
    let module = module && wants(NodeKind::Module) && selector.packages.is_empty();
    let external = external && wants(NodeKind::External) && selector.paths.is_empty();
    !(module || external)
}

/// The notices for `config` as applied to `graph`.
pub(super) fn notices(graph: &Graph, config: &Config) -> Vec<Notice> {
    let mut notices = Vec::new();
    // Edges join modules; only an `imports` edge ends at an external package.
    for rule in &config.rules {
        let reaches_external = rule.kinds.contains(&Relation::Imports);
        let ends = [("from", &rule.from, false), ("to", &rule.to, reaches_external)];
        for (end, selector, external) in ends {
            if dead(selector, true, external) {
                notices.push(Notice {
                    level: Severity::Warning,
                    message: format!(
                        "rule `{}` can never match: no dependency has a node its `{end}` \
                         selector could select",
                        rule.id
                    ),
                });
            }
        }
    }
    let layered = graph.nodes.values().any(|node| node.tags.contains_key("layer"));
    if !config.layers.is_empty() && !layered {
        notices.push(Notice {
            level: Severity::Warning,
            message: "`layers` is set but no node has a `layer` tag, so it checks nothing"
                .to_owned(),
        });
    }
    notices
}
