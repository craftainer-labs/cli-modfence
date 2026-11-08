//! The dependency graph (ADR 0003): nodes, resolved edges, tags and the
//! diagnostics found while building it.
//!
//! A [`Builder`] receives each file's [`FileFacts`](crate::extract::FileFacts)
//! from its extractor, together with the file's text, and
//! [`build`](Builder::build) returns the [`Graph`]:
//!
//! - **Nodes:** the extractors' module and symbol nodes, one `package` node per
//!   directory with modules (`.` for the root) and one `external` node per
//!   package outside the analysis. Every node carries its effective tags (ADR
//!   0004).
//! - **Edges:** `contains` (package to module to symbol), `imports` (module to
//!   module or external), `calls` (heuristic, folded per source, target and
//!   confidence) and `binds` (Python or TypeScript to a Rust export, matched
//!   by exported name). An edge keeps every site, ordered by path and
//!   position, because rules and suppressions work per site; [`Edge::site`] and
//!   [`Edge::count`] give the graph document's first site and count.
//! - **Findings:** malformed annotations and trailing tags
//!   (`modfence/invalid-annotation`), and the `ignore[...]` suppressions, for
//!   the rules engine to apply.
//!
//! The graph is built only from what the caller passes in: it reads no files
//! and starts no process. Output is deterministic (ADR 0003, invariant 1).

mod assemble;
mod locate;
pub mod precise;

use std::collections::{BTreeMap, BTreeSet};

use crate::config::Severity;
use crate::extract::{
    self, BindingFramework, DiagnosticCode, Lang, NodeId, NodeKind, Span, SymbolKind, Tags,
};

pub use self::assemble::Builder;

/// The built-in rule id of annotation problems.
pub const INVALID_ANNOTATION: &str = "modfence/invalid-annotation";

/// The relation of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// Package to module, module to symbol. Never checked by rules.
    Contains,
    /// A module imports a module or an external package.
    Imports,
    /// A symbol (or a module, for a call at module level) calls a symbol.
    Calls,
    /// A call or import crosses the language boundary.
    Binds,
}

impl Relation {
    /// The `relation` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::Binds => "binds",
        }
    }
}

/// How sure the graph is of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// The target is certain.
    Resolved,
    /// A guess: a name match, or an import that could not be placed.
    Heuristic,
}

impl Confidence {
    /// The `confidence` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Heuristic => "heuristic",
        }
    }
}

/// Which backend produced an edge's current confidence (the `origin` in the
/// JSON output).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// The tree-sitter extractors and the core's resolution.
    TreeSitter,
    /// A SCIP index (ADR 0003, invariants 7 and 8).
    Scip,
}

impl Origin {
    /// The `origin` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TreeSitter => "tree-sitter",
            Self::Scip => "scip",
        }
    }
}

/// The backend a graph was built with (`graph.metadata.backend` in the JSON
/// output).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// Tree-sitter only.
    #[default]
    TreeSitter,
    /// Tree-sitter, with at least one SCIP index ingested by
    /// [`precise::apply`].
    TreeSitterScip,
}

impl Backend {
    /// The `backend` value in the JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TreeSitter => "tree-sitter",
            Self::TreeSitterScip => "tree-sitter+scip",
        }
    }
}

/// Where an edge comes from in the source: an import statement or a call.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Site {
    /// The file, relative to the analysis root.
    pub path: String,
    /// The statement or callee.
    pub span: Span,
}

/// A directed edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The source node.
    pub source: NodeId,
    /// The target node.
    pub target: NodeId,
    /// The relation.
    pub relation: Relation,
    /// `None` for `contains`.
    pub confidence: Option<Confidence>,
    /// The specifier or callee text at the first site.
    pub specifier: Option<String>,
    /// Every site, ordered by path and position; empty for `contains`.
    pub sites: Vec<Site>,
}

impl Edge {
    /// The first site, which the graph document keeps.
    #[must_use]
    pub fn site(&self) -> Option<&Site> {
        self.sites.first()
    }

    /// The number of sites folded into the edge.
    #[must_use]
    pub fn count(&self) -> usize {
        self.sites.len()
    }

    /// Which backend gave the edge its confidence. Only a SCIP match makes a
    /// call `resolved` (ADR 0003, invariant 7), so a resolved `calls` edge is
    /// the one origin `scip` there is; every other edge is tree-sitter's.
    #[must_use]
    pub const fn origin(&self) -> Origin {
        match (self.relation, self.confidence) {
            (Relation::Calls, Some(Confidence::Resolved)) => Origin::Scip,
            _ => Origin::TreeSitter,
        }
    }
}

/// The cross-language export a symbol carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The framework.
    pub framework: BindingFramework,
    /// The name the other language sees.
    pub exported_name: String,
}

/// A node with its effective tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The id.
    pub id: NodeId,
    /// The kind.
    pub kind: NodeKind,
    /// The language.
    pub lang: Lang,
    /// A name for people.
    pub label: String,
    /// The directory or file; `None` for an external node.
    pub path: Option<String>,
    /// What the symbol is; `Some` exactly for symbols.
    pub symbol_kind: Option<SymbolKind>,
    /// Where the symbol is; `Some` exactly for symbols.
    pub span: Option<Span>,
    /// The module a symbol belongs to; `None` for every other kind.
    pub module: Option<NodeId>,
    /// Set on symbols exported across the language boundary.
    pub binding: Option<Binding>,
    /// The effective tags (ADR 0004); empty for a node without tags.
    pub tags: Tags,
}

impl Node {
    /// The graph node for a node an extractor reported, without tags.
    pub(crate) fn from_extracted(node: &extract::Node) -> Self {
        let module = (node.kind == NodeKind::Symbol)
            .then(|| NodeId::new(NodeKind::Module, node.lang, &node.path, None).ok())
            .flatten();
        Self {
            id: node.id.clone(),
            kind: node.kind,
            lang: node.lang,
            label: node.label.clone(),
            path: Some(node.path.clone()),
            symbol_kind: node.symbol_kind,
            span: node.span,
            module,
            binding: None,
            tags: Tags::new(),
        }
    }
}

/// A problem with one file, as in the graph document's `diagnostics`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileDiagnostic {
    /// The file.
    pub path: String,
    /// Where, when known.
    pub span: Option<Span>,
    /// What kind of problem.
    pub code: DiagnosticCode,
    /// A message for people; it holds no absolute path.
    pub message: String,
}

/// A problem with an annotation, reported by the rules engine under
/// [`INVALID_ANNOTATION`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// The file.
    pub path: String,
    /// The comment.
    pub span: Span,
    /// `error` for a malformed annotation, `warning` for a trailing tag.
    pub severity: Severity,
    /// A message for people.
    pub message: String,
}

/// A well-formed `modfence: ignore[...]` comment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Suppression {
    /// The file.
    pub path: String,
    /// The comment. It suppresses sites that start on its line or the next.
    pub span: Span,
    /// The rule ids.
    pub rules: Vec<String>,
    /// The reason, if given.
    pub reason: Option<String>,
}

/// The dependency graph of an analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Graph {
    /// The backend: tree-sitter, or tree-sitter with SCIP ingested.
    pub backend: Backend,
    /// Every node, by id.
    pub nodes: BTreeMap<NodeId, Node>,
    /// Every edge, ordered by source, target, relation and confidence.
    pub edges: Vec<Edge>,
    /// The languages of the analysed files.
    pub languages: BTreeSet<Lang>,
    /// Problems with files, ordered by path and position.
    pub diagnostics: Vec<FileDiagnostic>,
    /// Annotation problems, ordered by path and position.
    pub findings: Vec<Finding>,
    /// Well-formed suppressions, ordered by path and position.
    pub suppressions: Vec<Suppression>,
}

impl Graph {
    /// The module that rules judge an endpoint by: a symbol rolls up to its
    /// module, every other node is its own endpoint (ADR 0003, "Rules see
    /// module-level edges"). An id that is not in the graph is returned as is.
    #[must_use]
    pub fn rollup<'a>(&'a self, id: &'a NodeId) -> &'a NodeId {
        self.nodes
            .get(id)
            .and_then(|node| node.module.as_ref())
            .unwrap_or(id)
    }
}
