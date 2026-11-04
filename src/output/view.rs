//! The view of a graph that the diagram formats draw: the nodes of one level,
//! the edges rolled up to them, and the clusters. A pure function of the
//! graph; it reads no files and prints nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::extract::{NodeId, NodeKind};
use crate::graph::{Confidence, Graph, Node, Relation};

/// How much detail a view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Packages and external packages.
    Package,
    /// Packages, external packages and modules. The default of the diagrams.
    #[default]
    Module,
    /// Everything, symbols included.
    Symbol,
}

impl Level {
    /// Whether a node of `kind` is drawn at this level.
    #[must_use]
    pub const fn shows(self, kind: NodeKind) -> bool {
        match kind {
            NodeKind::Package | NodeKind::External => true,
            NodeKind::Module => !matches!(self, Self::Package),
            NodeKind::Symbol => matches!(self, Self::Symbol),
        }
    }
}

/// An edge between two visible nodes: the graph's edges with the same
/// relation and confidence, rolled up and folded together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewEdge<'a> {
    /// The visible source.
    pub source: &'a NodeId,
    /// The visible target.
    pub target: &'a NodeId,
    /// The relation.
    pub relation: Relation,
    /// `None` for `contains`.
    pub confidence: Option<Confidence>,
    /// The number of sites folded into the edge; 0 for `contains`.
    pub count: usize,
}

/// A set of nodes drawn together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster<'a> {
    /// The package directory, or the tag value, that the members share.
    pub label: String,
    /// The members, in node order.
    pub members: Vec<&'a NodeId>,
}

/// A graph at one level, with its clusters.
///
/// - **Nodes:** those the [`Level`] shows, in node order.
/// - **Edges:** every edge with both endpoints rolled up to the nearest
///   visible node (a symbol to its module, a module to its package); an edge
///   that ends up inside one node is dropped, and edges with the same source,
///   target, relation and confidence are folded, adding their site counts.
/// - **Clusters:** by default one per package, holding its modules and
///   symbols. With `group_by` one per value of that tag key, holding the
///   visible nodes that have it; a node with several values sits in the
///   cluster of the first value in sorted order (the others are in its
///   `tags`). Nodes without a cluster are drawn outside any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<'a> {
    /// The level.
    pub level: Level,
    /// The tag key that was clustered by, if any.
    pub group_by: Option<String>,
    /// The visible nodes.
    pub nodes: Vec<&'a Node>,
    /// The edges between them, ordered by source, target, relation and
    /// confidence.
    pub edges: Vec<ViewEdge<'a>>,
    /// The clusters, ordered by label.
    pub clusters: Vec<Cluster<'a>>,
}

/// The containment tree of a graph.
struct Tree<'a> {
    graph: &'a Graph,
    parents: BTreeMap<&'a NodeId, &'a NodeId>,
}

impl<'a> Tree<'a> {
    fn new(graph: &'a Graph) -> Self {
        let parents = graph
            .edges
            .iter()
            .filter(|edge| edge.relation == Relation::Contains)
            .map(|edge| (&edge.target, &edge.source))
            .collect();
        Self { graph, parents }
    }

    /// The node `id` or its nearest ancestor that `accept` takes.
    fn up(&self, id: &NodeId, accept: impl Fn(&Node) -> bool) -> Option<&'a Node> {
        let start = self.graph.nodes.get(id)?;
        // At most one step per node: a longer walk has looped on a `contains`
        // cycle.
        std::iter::successors(Some(start), |node| {
            self.parents
                .get(&node.id)
                .and_then(|parent| self.graph.nodes.get(*parent))
        })
        .take(self.graph.nodes.len() + 1)
        .find(|node| accept(node))
    }

    /// The label of the cluster `node` belongs to.
    fn cluster(&self, node: &Node, group_by: Option<&str>) -> Option<String> {
        match group_by {
            Some(key) => node.tags.get(key)?.iter().next().cloned(),
            None if matches!(node.kind, NodeKind::Module | NodeKind::Symbol) => self
                .up(&node.id, |ancestor| ancestor.kind == NodeKind::Package)
                .map(|package| package.label.clone()),
            None => None,
        }
    }
}

impl<'a> View<'a> {
    /// The view of `graph` at `level`, clustered by the tag key `group_by` or,
    /// without one, by package.
    #[must_use]
    pub fn new(graph: &'a Graph, level: Level, group_by: Option<&str>) -> Self {
        let tree = Tree::new(graph);
        // A node with no ancestor the level shows stays visible as itself, so
        // no edge is dropped.
        let anchor = |node: &'a Node| -> &'a NodeId {
            tree.up(&node.id, |ancestor| level.shows(ancestor.kind))
                .map_or(&node.id, |shown| &shown.id)
        };
        let nodes: Vec<&Node> = graph
            .nodes
            .values()
            .filter(|node| anchor(node) == &node.id)
            .collect();
        let visible = |id: &NodeId| graph.nodes.get(id).map(anchor);
        let mut folded: BTreeMap<(&NodeId, &NodeId, Relation, Option<Confidence>), usize> =
            BTreeMap::new();
        for edge in &graph.edges {
            if let (Some(source), Some(target)) = (visible(&edge.source), visible(&edge.target))
                && source != target
            {
                let key = (source, target, edge.relation, edge.confidence);
                *folded.entry(key).or_default() += edge.count();
            }
        }
        let edges = folded
            .into_iter()
            .map(|((source, target, relation, confidence), count)| ViewEdge {
                source,
                target,
                relation,
                confidence,
                count,
            })
            .collect();
        let mut groups: BTreeMap<String, Vec<&NodeId>> = BTreeMap::new();
        for node in &nodes {
            if let Some(label) = tree.cluster(node, group_by) {
                groups.entry(label).or_default().push(&node.id);
            }
        }
        let clusters = groups
            .into_iter()
            .map(|(label, members)| Cluster { label, members })
            .collect();
        Self {
            level,
            group_by: group_by.map(str::to_owned),
            nodes,
            edges,
            clusters,
        }
    }
}

/// The part of `graph` that touches `selected`: those nodes, every edge with
/// one of them as an endpoint, and the other endpoints of those edges. Like
/// the graph, it validates against the graph schema. Its languages are those
/// of its nodes and its diagnostics those of their files.
#[must_use]
pub fn incident(graph: &Graph, selected: &BTreeSet<NodeId>) -> Graph {
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| selected.contains(&edge.source) || selected.contains(&edge.target))
        .cloned()
        .collect();
    let ends = edges.iter().flat_map(|edge| [&edge.source, &edge.target]);
    let keep: BTreeSet<&NodeId> = selected.iter().chain(ends).collect();
    let nodes: BTreeMap<NodeId, Node> = graph
        .nodes
        .iter()
        .filter(|(id, _)| keep.contains(id))
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect();
    let languages = nodes.values().map(|node| node.lang).collect();
    let paths: BTreeSet<&str> = nodes
        .values()
        .filter_map(|node| node.path.as_deref())
        .collect();
    let diagnostics = graph
        .diagnostics
        .iter()
        .filter(|diagnostic| paths.contains(diagnostic.path.as_str()))
        .cloned()
        .collect();
    Graph {
        nodes,
        edges,
        languages,
        diagnostics,
        ..Graph::default()
    }
}
