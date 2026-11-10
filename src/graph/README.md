# src/graph/

The dependency graph (ADR 0003): what the extractors reported, resolved,
folded and tagged. It reads no files and starts no process; the caller passes
each file's text and facts to `Builder::add`.

- `mod.rs`: the graph types (`Graph`, `Node`, `Edge`, `Site`, `Relation`,
  `Confidence`) and the findings the rules engine consumes (`FileDiagnostic`,
  `Finding`, `Suppression`). An `Edge` keeps every `Site`, because rules and
  suppressions work per site; the graph document keeps `site()` and `count()`.
- `assemble.rs`: `Builder`. Resolves imports, matches calls by name (always
  `heuristic`), links Python and TypeScript to Rust exports by exported name
  (`binds`, always `heuristic`), turns annotations into module and symbol tags
  and findings, and computes each node's effective tags through
  `crate::config`.
- `precise.rs`: `apply`, `--precise` (NC6b). After the build it moves the call
  sites that SCIP indexes resolve from their heuristic edges to `resolved`
  ones (origin `scip`), sets `Graph::backend`, and returns `PreciseStats`
  (counted documents and occurrences, with `warnings`). It takes the index data
  of `src/scip/` and the files' text, and reads nothing itself.
- `locate.rs`: pure import resolution against the set of analysed paths
  (Python dotted and relative imports against configurable roots, TypeScript
  relative specifiers with extension, `.js` to `.ts` and `index` fallbacks).

Rules:

- Output is deterministic: maps and sorted vectors only, no hash iteration.
- Every edge endpoint is a node, and `contains` is the only relation without a
  confidence or a site.
- Not here yet: `tsconfig` `paths` aliases, reading `pyproject.toml` and
  `package.json` roots, Rust `crate::` resolution. The JSON output is `src/output/`.
- `apply` never adds a call site (`Graph::call_site_count` is the same before and
  after; sites that matched no symbol are kept unserialised in the graph so
  that a reference can still resolve them), never touches `imports`, `binds` or
  `contains` edges, and is deterministic: a symbol defined at several nodes
  resolves to the smallest id, and the smallest symbol wins when occurrences
  share a span. Documents with one path are read once, the first in the order
  the caller gives the indexes winning (the repeats are counted and warned
  about). `PreciseStats::unmatched_definitions` counts definitions that map to
  no node; it is a statistic, not a warning, because parameters and locals
  always land there. Its tests are in `tests/graph_precise.rs`.
- Tests are in `tests/graph.rs`; do not add `#[cfg(test)]` modules here.
