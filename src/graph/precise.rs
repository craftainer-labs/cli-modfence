//! `--precise`: upgrading call edges with SCIP indexes (ADR 0003, invariants 7,
//! 8 and 20).
//!
//! Tree-sitter finds the call sites; a SCIP index only says which definition a
//! reference at a given place refers to. [`apply`] therefore moves a call site
//! from its heuristic edge to a `resolved` one when the index has a reference at
//! exactly the site's callee span whose definition is one of the graph's symbol
//! nodes. It never adds a call site, and it leaves `imports` and `binds` edges
//! alone.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::assemble::short_name_of;
use super::{Backend, Confidence, Edge, Graph, Relation, Site};
use crate::extract::{NodeId, NodeKind, Span};
use crate::scip::{IMPORT, Index as ScipIndex, LineIndex};

/// What [`apply`] used, left out or had to choose between.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PreciseStats {
    /// Documents read (after leaving out repeated and unknown ones).
    pub documents: usize,
    /// Documents whose path an earlier document already used; the first wins.
    pub duplicate_documents: usize,
    /// Documents whose path matches no analysed file.
    pub unknown_documents: usize,
    /// Occurrences whose position does not exist in the source: a sign that the
    /// index is stale.
    pub stale_occurrences: usize,
    /// Symbols defined at more than one node; the node with the smallest id wins.
    pub conflicting_symbols: usize,
    /// Call sites moved to `resolved` edges.
    pub resolved_sites: usize,
}

impl PreciseStats {
    /// One line per thing that was left out or chosen between, for standard
    /// error. They hold no text from the index.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.duplicate_documents > 0 {
            lines.push(format!(
                "SCIP: {} documents repeat a path already read; the first was used",
                self.duplicate_documents
            ));
        }
        if self.unknown_documents > 0 {
            lines.push(format!(
                "SCIP: {} documents match no analysed file and were ignored",
                self.unknown_documents
            ));
        }
        if self.stale_occurrences > 0 {
            lines.push(format!(
                "SCIP: {} occurrences lie outside their source file; the index may be stale",
                self.stale_occurrences
            ));
        }
        if self.conflicting_symbols > 0 {
            lines.push(format!(
                "SCIP: {} symbols are defined at more than one node; the smallest id was used",
                self.conflicting_symbols
            ));
        }
        lines
    }
}

/// A SCIP symbol, scoped to its document when it is a `local` one.
type SymbolKey = (String, String);

/// A symbol node of one file, by short name.
type FileSymbols<'a> = BTreeMap<String, Vec<(&'a NodeId, Span)>>;

/// Upgrades the call sites of `graph` that `indexes` resolve. `sources` maps a
/// root-relative path to the file's text, which converts SCIP positions.
///
/// Sets [`Graph::backend`] to `tree-sitter+scip` when `indexes` is not empty.
/// Idempotent for the same indexes; with others it only adds. Deterministic: the
/// result does not depend on hash order, and a symbol defined at several nodes
/// resolves to the smallest id.
pub fn apply(
    graph: &mut Graph,
    indexes: &[ScipIndex],
    sources: &BTreeMap<String, String>,
) -> PreciseStats {
    let mut stats = PreciseStats::default();
    if indexes.is_empty() {
        return stats;
    }
    graph.backend = Backend::TreeSitterScip;
    let call_sites = heuristic_call_sites(graph);
    let symbols = file_symbols(graph);
    let mut definitions: BTreeMap<SymbolKey, &NodeId> = BTreeMap::new();
    let mut references: BTreeMap<Site, SymbolKey> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for document in indexes.iter().flat_map(|index| &index.documents) {
        if !seen.insert(document.path.as_str()) {
            stats.duplicate_documents += 1;
            continue;
        }
        let Some(source) = sources.get(&document.path) else {
            stats.unknown_documents += 1;
            continue;
        };
        stats.documents += 1;
        let lines = LineIndex::new(source);
        let wanted = call_sites.get(document.path.as_str());
        let here = symbols.get(document.path.as_str());
        for occurrence in &document.occurrences {
            let Some(span) = lines.span(occurrence.range, document.encoding) else {
                stats.stale_occurrences += 1;
                continue;
            };
            if occurrence.is_definition() {
                let Some(node) = here.and_then(|names| definition_node(&lines, span, names)) else {
                    continue;
                };
                let key = symbol_key(&document.path, &occurrence.symbol);
                let current = definitions.entry(key).or_insert(node);
                if *current != node {
                    stats.conflicting_symbols += 1;
                    *current = (*current).min(node);
                }
            } else if occurrence.roles & IMPORT == 0 && wanted.is_some_and(|w| w.contains(&span)) {
                let site = Site {
                    path: document.path.clone(),
                    span,
                };
                let key = symbol_key(&document.path, &occurrence.symbol);
                match references.entry(site) {
                    Entry::Vacant(slot) => {
                        slot.insert(key);
                    }
                    Entry::Occupied(mut slot) => {
                        if key < *slot.get() {
                            slot.insert(key);
                        }
                    }
                }
            }
        }
    }
    let resolved: BTreeMap<Site, NodeId> = references
        .into_iter()
        .filter_map(|(site, key)| Some((site, (*definitions.get(&key)?).clone())))
        .collect();
    stats.resolved_sites = fold(graph, &resolved);
    stats
}

/// The spans of the heuristic call sites, by file.
fn heuristic_call_sites(graph: &Graph) -> BTreeMap<&str, BTreeSet<Span>> {
    let mut sites: BTreeMap<&str, BTreeSet<Span>> = BTreeMap::new();
    for edge in &graph.edges {
        if edge.relation == Relation::Calls && edge.confidence == Some(Confidence::Heuristic) {
            for site in &edge.sites {
                sites.entry(site.path.as_str()).or_default().insert(site.span);
            }
        }
    }
    sites
}

/// The symbol nodes of each file, by short name.
fn file_symbols(graph: &Graph) -> BTreeMap<&str, FileSymbols<'_>> {
    let mut files: BTreeMap<&str, FileSymbols<'_>> = BTreeMap::new();
    for node in graph.nodes.values() {
        if let (NodeKind::Symbol, Some(path), Some(span)) = (node.kind, &node.path, node.span) {
            files
                .entry(path.as_str())
                .or_default()
                .entry(short_name_of(&node.id))
                .or_default()
                .push((&node.id, span));
        }
    }
    files
}

/// A symbol string made unique across documents: a `local` one belongs to its
/// document alone.
fn symbol_key(path: &str, symbol: &str) -> SymbolKey {
    let scope = if symbol.starts_with("local ") { path } else { "" };
    (scope.to_owned(), symbol.to_owned())
}

/// The symbol node a definition occurrence at `span` defines: the innermost
/// node of the file that contains `span` and has the text there as its name
/// (ties: the smallest id). `None` for definitions the graph has no node for,
/// such as parameters and local variables.
fn definition_node<'a>(
    lines: &LineIndex<'_>,
    span: Span,
    names: &FileSymbols<'a>,
) -> Option<&'a NodeId> {
    let candidates = names.get(lines.slice(span)?)?;
    let start = |s: Span| (s.start_line, s.start_column);
    let end = |s: Span| (s.end_line, s.end_column);
    candidates
        .iter()
        .filter(|(_, outer)| start(*outer) <= start(span) && end(span) <= end(*outer))
        .max_by(|(a, outer_a), (b, outer_b)| {
            start(*outer_a).cmp(&start(*outer_b)).then_with(|| b.cmp(a))
        })
        .map(|(id, _)| *id)
}

/// Moves the `resolved` call sites from their heuristic edges to resolved ones,
/// and returns how many it moved. A site that sits on several heuristic edges
/// (several nodes share its name) leaves all of them.
fn fold(graph: &mut Graph, resolved: &BTreeMap<Site, NodeId>) -> usize {
    if resolved.is_empty() {
        return 0;
    }
    let mut folded: BTreeMap<EdgeKey, Vec<(Site, Option<String>)>> = BTreeMap::new();
    let mut moved: BTreeMap<(NodeId, NodeId), BTreeMap<Site, Option<String>>> = BTreeMap::new();
    for edge in std::mem::take(&mut graph.edges) {
        let heuristic_call =
            edge.relation == Relation::Calls && edge.confidence == Some(Confidence::Heuristic);
        let entry = folded
            .entry((edge.source.clone(), edge.target.clone(), edge.relation, edge.confidence))
            .or_default();
        for site in edge.sites {
            match resolved.get(&site) {
                Some(target) if heuristic_call => {
                    moved
                        .entry((edge.source.clone(), target.clone()))
                        .or_default()
                        .insert(site, edge.specifier.clone());
                }
                _ => entry.push((site, edge.specifier.clone())),
            }
        }
    }
    let count = moved.values().map(BTreeMap::len).sum();
    for ((source, target), sites) in moved {
        folded
            .entry((source, target, Relation::Calls, Some(Confidence::Resolved)))
            .or_default()
            .extend(sites);
    }
    // An edge that lost every site goes with them; `contains` edges never had
    // any and stay.
    graph.edges = folded
        .into_iter()
        .filter_map(|((source, target, relation, confidence), mut sites)| {
            sites.sort();
            sites.dedup_by(|a, b| a.0 == b.0);
            let keeps = relation == Relation::Contains || !sites.is_empty();
            keeps.then(|| Edge {
                source,
                target,
                relation,
                confidence,
                specifier: sites.first().and_then(|(_, specifier)| specifier.clone()),
                sites: sites.into_iter().map(|(site, _)| site).collect(),
            })
        })
        .collect();
    count
}

/// The fold key of an edge: source, target, relation and confidence.
type EdgeKey = (NodeId, NodeId, Relation, Option<Confidence>);
