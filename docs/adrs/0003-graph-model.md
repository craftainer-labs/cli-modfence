# 0003. Model the codebase as one typed dependency graph, built by tree-sitter and upgraded by SCIP

## Status

Proposed. Becomes Accepted when its implementation merges (NC3 to NC6),
per [`README.md`](README.md).

## Context

modfence checks module boundaries across Python, TypeScript and Rust with one
rule set. Today that takes import-linter or tach (Python), dependency-cruiser
(TypeScript) and cargo-modules (Rust), each with its own model and config.
Three forces shape the graph all of them must share:

- Imports can be extracted from syntax with close to exact results. Calls
  cannot: resolving `a.b()` to a definition needs type information. A parser
  alone yields call edges that are guesses.
- stack-graphs, the usual answer to name resolution on top of tree-sitter,
  was archived on 2025-09-09, so we don't build on it. SCIP indexers
  (scip-python, scip-typescript, `rust-analyzer scip`) do resolve names, but
  are slow, need the project's dependencies installed, and are not always
  available in CI.
- The tool must run offline ([NFR-0002](../nfrs/NFR-0002-offline-operation.md))
  and within the budget of [NFR-0001](../nfrs/NFR-0001-analysis-performance.md),
  and its output is consumed by other tools: a stable, versioned format.

Sources: tree-sitter ABI compatibility
(<https://github.com/tree-sitter/tree-sitter/blob/master/docs/src/using-parsers/7-abi-versions.md>:
library >= 0.25 loads parsers of ABI 13 to 15); SCIP
(<https://github.com/scip-code/scip>: occurrences carry a symbol string, a
role bitset and, for definitions, an enclosing range; ranges are 0-based and
half-open, and a document declares its position encoding); JSON Graph Format
v2 (<https://jsongraphformat.info/v2.0/json-graph-schema.json>).

## Decision

**Hybrid backend.** The default backend is tree-sitter. It produces import
edges and call edges, and tags each edge `confidence = "heuristic"` unless
the target is certain. `--precise` ingests SCIP indexes and upgrades call
edges it can confirm to `resolved`; it may also add `resolved` edges the
parser missed. Without `--precise` the tool needs nothing installed beyond
itself. A name-resolution engine of our own is out of scope.

**Node kinds.** `package` (a directory with Python modules, a TypeScript
package directory, or a Rust crate), `module` (one per source file; a Rust
module resolves to its file), `symbol` (function, method, class, type, impl,
trait, constant) and `external` (a third-party or standard-library package,
named, never analysed). Ids are strings of the form
`pkg|mod|sym|ext:<py|ts|rs>:<path or package>[#<qualified name>]`, paths
relative to the analysis root with `/` separators.

**Edge relations.** `contains` (package to module to symbol; never checked
by rules), `imports` (module to module or external), `calls` (symbol to
symbol, folded to one edge per source, target and relation with a site count)
and `binds` (a call or import that crosses a language boundary).

**Rules see module-level edges.** Symbol-level `calls` and `binds` edges
roll up to their enclosing modules for rule evaluation, so one rule set
covers all three relations. Symbol nodes still exist in the graph and in the
JSON output, for exploration and for `--precise` joins.

**Cross-language edges.** Extractors report binding exports (PyO3
`#[pyfunction]`/`#[pymodule]`, napi `#[napi]`, wasm-bindgen
`#[wasm_bindgen]`) with the exported name. The core links a Python or
TypeScript import or call whose target name matches an export of the
extension module it resolves to, as a `binds` edge with
`confidence = "heuristic"`. Matching is by exported name only.

**Resolution is the core's job.** Extractors return unresolved specifiers and
call text with spans (interface I3); the core resolves them against package
roots it reads from `pyproject.toml`, `package.json`/`tsconfig.json` and
`Cargo.toml` directly. It never runs `cargo`, `npm` or `python`.

**Per-file cache.** Extraction results are cached per file, keyed by content
hash and extractor version, in `.modfence/cache/` under the analysis root
(`--cache-dir` and `--no-cache` override). Resolution and rules always re-run
over the cached facts. Content hashes, not salsa, keep the design small.

**Output format.** `modfence graph --format json` emits a strict
specialisation of JSON Graph Format v2: every document is also valid JGF v2,
because all modfence fields sit in the JGF `metadata` objects. The schema is
`schema/modfence-graph.schema.json`, the format version is
`graph.metadata.schema_version` (`major.minor`; a minor bump only adds
optional fields). JGF v2 forbids extra top-level keys, which is why the
version lives in `graph.metadata`.

**Positions.** 1-based lines and columns, columns counted in Unicode code
points, ends exclusive. SCIP positions are converted using each document's
declared encoding; SARIF output sets `columnKind` to `unicodeCodePoints`.

## Invariants for NC3 to NC6

1. Output is deterministic: nodes, edges and diagnostics are sorted, and the
   same inputs produce byte-identical output on every platform.
2. Output contains no timestamps, absolute paths or host names.
3. Every emitted path is relative to the analysis root and uses `/`.
4. Every node id matches the `nodeId` pattern in the schema, and every
   edge endpoint exists as a node.
5. A file that fails to parse or read yields a `diagnostics` entry and the
   facts that could be recovered; analysis continues.
6. Import edges to files inside the analysis root are `resolved`; an import
   the core cannot place becomes an edge to an `external` node, never a
   dropped edge.
7. A tree-sitter call edge is never `resolved`; only `--precise` makes one.
8. A SCIP reference becomes a call edge only when it lies inside the
   enclosing range of a definition that maps to one of our symbol nodes.
9. `binds` edges are always `heuristic`.
10. A cached fact is used only if the file hash and the extractor version
    both match; a corrupt or unwritable cache is a warning, never an error.
11. Every emitted document validates against the schema, and the schema is
    checked against its metaschema in CI.
12. The graph core reads only files below the analysis root, the config file
    and the SCIP files named on the command line.
13. tree-sitter-typescript's parser ABI is asserted at test time to be
    loadable by the pinned tree-sitter (NC3b).

## Decisions for the user (G-api)

Open (sources disagree, or a matter of taste):

1. **SCIP: prebuilt only, or run indexers?** (a) Ingest `.scip` files given
   with `--scip <file>`; the user or CI produces them. (b) Also run installed
   indexers behind an explicit `--run-indexers`. (a) keeps the tool free of
   subprocesses and side effects; (b) is one command for users, but indexers
   may resolve dependencies over the network, which conflicts with
   NFR-0002 unless the user opts in. Lean: (a) now, (b) as an opt-in flag.
2. **JavaScript.** Should `.js`, `.jsx`, `.mjs` and `.cjs` go through the
   TypeScript grammar and count as language `ts`? It matches how mixed
   repositories look and costs nothing; it makes the language tag
   imprecise. Lean: yes.
3. **Rule granularity.** Rules on module-level edges (as decided above) or
   also on symbol-level edges. Symbol-level rules give finer control and
   many more false positives from heuristic calls. Lean: module-level only
   in v1; the symbol nodes keep the door open.

Confirmations (proposed; no real alternative with a different cost):

4. JGF v2 specialisation with `schema_version` in `graph.metadata`.
5. Node id scheme and the four node kinds.
6. Cache in `.modfence/cache/` with `--cache-dir` and `--no-cache`.
7. Heuristic matching of `binds` edges by exported name only.
8. Positions in Unicode code points, 1-based.

## Consequences

- One graph and one rule engine serve all three languages and all outputs
  (DOT, Mermaid, JSON, GraphML, HTML, SARIF).
- Without `--precise`, call-based findings are only as good as syntax allows;
  the confidence tag is how the tool stays honest about that, and rules can
  require `resolved` edges (ADR 0004).
- Cross-language edges are name matches: a renamed export that collides with
  another module's name produces a false edge until `--precise` exists for
  both sides, and SCIP does not cover the Python-to-Rust boundary at all.
- The JSON format is public API once released. A major `schema_version`
  bump is a breaking change for every consumer.
- The tool depends on three grammars' release cadence; a grammar upgrade
  can change extraction and therefore needs the fixtures to be re-checked.
- Reading project metadata instead of invoking toolchains keeps the run
  offline and fast, but monorepo setups that rely on toolchain-only
  resolution (for example TypeScript project references with exotic
  `paths`) resolve partly, falling back to `external` edges.
