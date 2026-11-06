# src/output/

How the dependency graph is written out (ADR 0004, "Traceability outputs"):
pure functions from a `graph::Graph` to text. They print nothing and read no
files; `src/cli/` decides where the text goes.

- `mod.rs`: the `Renderer` trait, the one interface every format implements.
- `json.rs`: `Json`, the graph document of `schema/modfence-graph.schema.json`
  (ADR 0003, "Output format"). `SCHEMA_VERSION` is the document's `major.minor`.
- `dot.rs`, `mermaid.rs`: `Dot` and `Mermaid`, the view as a diagram (below).
- `graphml.rs`: `GraphMl`, the whole graph as `GraphML` (below).
- `html.rs`: `Html`, the interactive page of `--format html` (NC5c): one
  self-contained file, its assets in `html/` (below).
- `view.rs`: `View`, the graph as a diagram draws it (a `Level`, the visible
  nodes, the edges rolled up to them, the clusters), and `incident`, the part
  of a graph that touches a set of nodes (what `trace` outputs).

## DOT, Mermaid and GraphML (`dot.rs`, `mermaid.rs`, `graphml.rs`)

Hand-written serializers (no crate). Output is a pure function of the input,
in the order of the view (diagrams) or of the graph (`GraphML`).

- **DOT** (`Dot`): a `digraph "modfence"` with `rankdir=LR`. Nodes are `n0`,
  `n1`, ... in sorted id order, so ids that differ only in a character DOT
  cannot carry stay distinct nodes; the real id is the node's `id` attribute.
  Every id and label is a quoted string with `"` and `\` escaped, a line break
  written `\n` and other control characters replaced by a space. A cluster is `subgraph
  cluster_<n>` (numbered in the view's cluster order) holding its members;
  other nodes follow it. Shapes follow the HTML legend (package `box`, module
  `ellipse`, external `diamond`, symbol `triangle`); an edge's `style` is its
  relation (imports solid, calls dashed, binds dotted, contains solid in light
  grey), the arrowhead is on the target, and a folded edge of several sites has
  a `label` with the count.
- **Mermaid** (`Mermaid`): `flowchart LR`, classic syntax that Mermaid 10 and
  later (mermaid-cli) renders; nothing needs 11.3's `@{ shape }`. Nodes are
  `n0`, `n1`, ... in node order and clusters `c0`, ... (`subgraph c0["label"]`),
  so a label is never read as syntax. Labels are quoted, with `"`, `#`, `<`,
  `>`, `&` and the backtick written as Mermaid entity codes (`#quot;`, `#35;`,
  `#lt;`, `#gt;`, `#amp;`, `#96;`) and control characters as a space. Shapes:
  package `["..."]`, module `(["..."])` (stadium), external `{"..."}`
  (rhombus), symbol `{{"..."}}` (hexagon, standing in for the triangle that
  Mermaid only has from 11.3). Mermaid has no per-edge dash syntax, so every
  edge is `-->` and one `linkStyle` per relation sets `stroke-dasharray` (calls
  `6 3`, binds `2 3`) or the colour (contains); an edge that is not a plain
  import is labelled with its relation, and with its count when it folds
  several sites. Mermaid refuses large diagrams by default (`maxEdges` 500,
  `maxTextSize` 50000 characters), and the renderer does not split them: raise
  those limits in the Mermaid configuration or draw a coarser `--level`.
- **GraphML** (`GraphMl`): a data format, so it writes the whole graph, never
  the view, and is the same for every `--level` and `--group-by`. An XML 1.0
  declaration, `<graphml xmlns="http://graphml.graphdrawing.org/xmlns">`, one
  `<key>` per attribute of the JSON document (node: `id`, `kind`, `label`, `lang`,
  `path`, `symbol_kind`, `span`, `tags`, `binding`; edge: `relation`,
  `confidence`, `origin`, `count`, `specifier`, `sites`) and
  `<graph edgedefault="directed">`. Structured values (`span`, `tags`,
  `binding`, `sites`) are compact JSON text in a string attribute, with
  object keys sorted; `count` is an `int`. Nodes are `n0`, `n1`, ... in sorted id
  order (GraphML ids are NMTOKENs, which a path is not) with the graph's id in
  the `id` data; edge ids are `e0`, `e1`, ... in graph order and `source` and
  `target` are node ids. Text and attribute values escape
  `& < > " '`, write tab, line feed and carriage return as character
  references (so they survive attribute-value normalisation), and replace
  every character XML 1.0 forbids (controls other than those three, U+FFFE and
  U+FFFF) with U+FFFD. XML 1.0 cannot carry those characters at all, so this
  is lossy for text such as a path with a control character; JSON is the
  lossless data format.

## The HTML page (`html.rs`, `html/`)

One offline file that draws the `View` and lists it in tables.

- **Assets** (embedded with `include_str!`, so they ship in the crate):
  `html/page.html` (the template, `@@name@@` placeholders filled in one pass,
  so embedded text is never re-scanned), `html/style.css`, `html/app.js` and
  `html/vendor/d3-force/` (d3-force 3.0.1 and its three runtime dependencies,
  ISC, unmodified `dist/*.min.js`; its `README.md` has the versions, tarball
  URLs, npm `sha512` integrity and the update steps, `SHA256SUMS` the committed
  files' sums, `LICENSE` the licence text). Nothing in it is edited by hand:
  to update the library, follow that README.
- **No network, strict policy.** The page has no external URL, font or script.
  A `Content-Security-Policy` meta (`default-src 'none'`) allows only the one
  inline script and the one inline style, each by its SHA-256, which `html.rs`
  computes (a small FIPS 180-4 implementation, not a crate). Changing
  `style.css`, `app.js` or a vendored file changes the hash; `tests/html.rs`
  pins it and says what to update. The script sets no inline `style`
  attribute and uses no `eval`, so the policy needs no `unsafe-*`.
- **Data.** The view is embedded as a JSON data block (nodes with kind,
  language, path, tags and cluster; edges with relation, confidence and count;
  clusters). Every `<`, `>`, `&`, U+2028 and U+2029 in it is written as a
  `\u` escape, so no label, path or tag value can end the block (the escape
  reads back as the same character). The same node and edge lists are also
  static HTML tables (`escape` for text and attributes), so the graph is
  readable and keyboard-navigable without the script; each node row has a
  button that selects it in the diagram.
- **Accessibility** (checked by axe in a separate workflow, NC5d): `lang`,
  `<title>`, a labelled `role="img"` SVG whose accessible equivalent is the
  tables, `:focus-visible` outlines, AA contrast in light and dark
  (`prefers-color-scheme`), shapes and dashes as well as colour, and
  `prefers-reduced-motion` renders the settled layout with no animation.
  Every colour, size and font is a CSS variable at the top of `style.css`.
- **Randomness** is only the layout in the browser; the text of the page is a
  pure function of the view.

## Adding a format

1. Implement `Renderer` for a unit struct in its own file here and re-export
   it from `mod.rs`: `fn render(&self, graph: &Graph, view: &View<'_>) -> String`.
   The text ends in a newline.
2. In `src/cli/args.rs`, add the `ViewFormat` variant and its
   `Some(&YourFormat)` arm in `ViewArgs::renderer`. Nothing
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
  `contains` edges between two visible nodes are kept. A node with no
  ancestor the level shows (a module whose package is missing, or a
  `contains` cycle) stays visible as itself, so no edge is dropped.
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
  only the languages of those nodes and the diagnostics of their files, and
  drops the findings and suppressions, so it renders as a graph document of
  its own.
- The JSON document has `metadata.tool.version` from `CARGO_PKG_VERSION`, which
  the release stamps; tests compare with the version replaced. Contains edges
  have empty metadata; every other edge has `confidence`, `origin`
  (`tree-sitter` until SCIP), `count`, `specifier` and the first `site`. Node
  tags are arrays, and a node without tags has no `tags` key.
- Golden files are `tests/golden/*.graph.json`; the `graph-json-schema` hook in
  `.pre-commit-config.yaml` validates them, and `schema/examples/`, against the
  schema with ajv (Node), not with a Rust crate.
- Tests are in `tests/output.rs`, `tests/html.rs`, `tests/diagrams.rs` (DOT,
  Mermaid and `GraphML`: escaping, clusters, levels) and `tests/cli.rs` (the
  `tests/golden/graph.*` and `trace.*` files of the three formats); do not
  add `#[cfg(test)]` modules here, with one exception: `html.rs` has a single
  `tests` module with the known-answer vectors (NIST SHA-256, RFC 4648 base64)
  for its private `sha256` and `base64`. They are private, and no public
  surface should be added just to test them. `cargo llvm-cov` leaves
  `#[cfg(test)]` code out of the measured lines, so the 100% gate is unaffected.
