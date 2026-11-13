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
- `rustmods.rs`: the module tree of the Rust crates among the analysed files,
  and Rust `use` resolution against it. A crate root is a file Cargo's layout
  names (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, `src/bin/*/main.rs`, and
  single files in `tests/`, `examples/` and `benches/`, not below a `src/`);
  `mod name;` then adds `name.rs` or `name/mod.rs` below the directory of the
  declaring module, recursively. `Cargo.toml` and `#[path]` are not read.

Rules:

- Output is deterministic: maps and sorted vectors only, no hash iteration.
- Every edge endpoint is a node, and `contains` is the only relation without a
  confidence or a site.
- Rust paths: `crate::`, `self::` and `super::` (and a bare path that starts
  with a module of the file's module) resolve to the file of the longest prefix
  that is a module of the crate, so an item, an enum variant or an inline
  module lands on the file that holds it; an import of the file itself makes no
  edge. A file no crate root reaches has no module path: its `crate::`
  imports, and a `super` that climbs above the root, are unplaced (an external
  node and a diagnostic). Any other bare path is an `external` crate; the
  crates of the analysis are not matched to those names.
- A Rust short name (for calls and `--precise`) is the last component of the
  qualified name, decoded by the Rust rules, because `::` also appears inside
  an `impl` component.
- A `#[pymodule] mod x` binding export is on the file's module node (the
  `binding` of a `module` node, `binds` edges that end at it). The
  schema describes `binding` as present on symbols, but does not restrict it.
- Not here yet: `tsconfig` `paths` aliases, reading `pyproject.toml` and
  `package.json` roots, `Cargo.toml` (crate names, workspaces, `#[path]`).
  The JSON output is `src/output/`.
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
