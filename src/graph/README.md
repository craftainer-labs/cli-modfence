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
- `locate.rs`: pure import resolution against the set of analysed paths
  (Python dotted and relative imports against configurable roots, TypeScript
  relative specifiers with extension, `.js` to `.ts` and `index` fallbacks).

Rules:

- Output is deterministic: maps and sorted vectors only, no hash iteration.
- Every edge endpoint is a node, and `contains` is the only relation without a
  confidence or a site.
- Not here yet: `tsconfig` `paths` aliases, reading `pyproject.toml` and
  `package.json` roots, Rust `crate::` resolution, SCIP. The JSON output is `src/output/`.
- Tests are in `tests/graph.rs`; do not add `#[cfg(test)]` modules here.
