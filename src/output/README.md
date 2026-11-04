# src/output/

How the dependency graph is written out (ADR 0004, "Traceability outputs"):
pure functions from a `graph::Graph` to text. They print nothing and read no
files; `src/cli/` decides where the text goes.

- `mod.rs`: the `Renderer` trait, the one interface every format implements.
- `json.rs`: `Json`, the graph document of `schema/modfence-graph.schema.json`
  (ADR 0003, "Output format"). `SCHEMA_VERSION` is the document's `major.minor`.
- `view.rs`: `View`, the graph as a diagram draws it (a `Level`, the visible
  nodes, the edges rolled up to them, the clusters), and `incident`, the part
  of a graph that touches a set of nodes (what `trace` outputs).

## Adding a format (NC5b, NC5c)

1. Implement `Renderer` for a unit struct in its own file here and re-export
   it from `mod.rs`: `fn render(&self, graph: &Graph, view: &View<'_>) -> String`.
   The text ends in a newline.
2. In `src/cli/args.rs`, replace the `Err(Self::pending("..."))` arm of
   `ViewArgs::renderer` for the format with `Ok(Some(&YourFormat))`. Nothing
   else in `src/cli/` changes: `graph` and `trace` both go through the
   renderer, and `trace` hands it the `incident` part of the graph.
3. Add its tests to `tests/` and, for a document with a schema, its golden file.

A renderer receives both the graph and the view. Data formats (JSON, GraphML)
write the graph and ignore the view, so they always carry everything. Diagram
formats (DOT, Mermaid, HTML) draw the view and never the graph directly.

## The view

`View::new(graph, level, group_by)`:

- `Level::Package` shows packages and external nodes, `Level::Module` (the
  default) adds modules, `Level::Symbol` adds symbols. `Level::shows` says
  which kinds a level draws.
- An edge is rolled up to the nearest visible node at each end (symbol to
  module, module to package, along the `contains` edges). An edge that then
  starts and ends at one node is dropped, which also removes every `contains`
  edge that a level hides. Edges with the same source, target, relation and
  confidence are folded; `count` adds up their sites (0 for `contains`).
  `contains` edges between two visible nodes are kept.
- Clusters: without `group_by`, one per package, labelled with the package's
  directory and holding its visible modules and symbols. With `group_by`, one
  per value of that tag key, holding the visible nodes that have the tag; a node
  with several values sits in the cluster of the first in sorted order, and
  keeps all of them in `tags` for a hover text. A node without a cluster
  (external nodes by default, untagged nodes) is drawn outside any.
- Everything is ordered (nodes by id, edges by source, target, relation and
  confidence, clusters by label), so the output is deterministic.

## Rules

- Output is deterministic and holds no timestamp or absolute path.
- `incident(graph, selected)` is the selected nodes, every edge with a selected
  endpoint, and those edges' other endpoints (ADR 0004, invariant 23). It keeps
  the graph's languages and diagnostics and drops its findings and
  suppressions, so it renders as a graph document of its own.
- The JSON document has `metadata.tool.version` from `CARGO_PKG_VERSION`, which
  the release stamps; tests compare with the version replaced. Contains edges
  have empty metadata; every other edge has `confidence`, `origin`
  (`tree-sitter` until SCIP), `count`, `specifier` and the first `site`. Node
  tags are arrays, and a node without tags has no `tags` key.
- Golden files are `tests/golden/*.graph.json`; the `graph-json-schema` hook in
  `.pre-commit-config.yaml` validates them, and `schema/examples/`, against the
  schema with ajv (Node), not with a Rust crate.
- Tests are in `tests/output.rs` and `tests/cli.rs`; do not add `#[cfg(test)]`
  modules here.
