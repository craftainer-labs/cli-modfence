# 0003. Model the codebase as one typed dependency graph, built by tree-sitter and upgraded by SCIP

## Status

Proposed. The G-api decisions in the table "Decisions (G-api)", and the items
A and B below it, were approved by the user on 2026-11-01 (C is decided in ADR
0004). It becomes Accepted when its implementation merges (NC3 to NC6), per
[`README.md`](README.md).

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
  except for the one flag that runs indexers, and within the budget of
  [NFR-0001](../nfrs/NFR-0001-analysis-performance.md). Its output is
  consumed by other tools, so it needs a stable, versioned format.

Sources: tree-sitter ABI compatibility
(<https://github.com/tree-sitter/tree-sitter/blob/master/docs/src/using-parsers/7-abi-versions.md>:
library >= 0.25 loads parsers of ABI 13 to 15); SCIP
(<https://github.com/scip-code/scip>: occurrences carry a symbol string, a
role bitset and, for definitions, an enclosing range; ranges are 0-based and
half-open, and a document declares its position encoding); the indexers
(<https://github.com/sourcegraph/scip-typescript> `scip-typescript index`,
with `--infer-tsconfig` for JavaScript and `--output`;
<https://github.com/sourcegraph/scip-python> `scip-python index . --project-name=NAME`;
`rust-analyzer scip <dir> --output index.scip`, which rust-analyzer's own
book does not document, so NC6 checks the flags against the installed
version); JSON Graph Format v2
(<https://jsongraphformat.info/v2.0/json-graph-schema.json>).

## Decision

**Hybrid backend.** The default backend is tree-sitter. It produces import
edges and call edges, and tags each edge `confidence = "heuristic"` unless
the target is certain. `--precise` ingests SCIP indexes and upgrades the call
edges it can confirm to `resolved`. Without `--precise` the tool needs
nothing installed beyond itself. A name-resolution engine of our own is out
of scope.

**Where SCIP indexes come from.** Two ways, both opt-in (user decision):

- `--scip <file>` (repeatable) ingests prebuilt `.scip` files and implies
  `--precise`.
- `--run-indexers` implies `--precise`, runs the indexers listed below, and
  ingests their output. It runs **only executables already on `PATH`**: it
  never downloads, installs or fetches an indexer or a dependency, and never
  goes through `npx`, `pipx`, `cargo install` or similar. It prints the
  full command line of each indexer to stderr before running it. It is the
  only mode in which anything started by modfence may use the network, and
  only because the user's indexer decides to (NFR-0002).

| Language | Indexer on `PATH` | Invocation (NC6 verifies the flags) |
| --- | --- | --- |
| `py` | `scip-python` | `scip-python index . --project-name <name>` |
| `ts` (including JavaScript) | `scip-typescript` | `scip-typescript index`, plus `--infer-tsconfig` when the project has no `tsconfig.json`, and `--output` |
| `rs` | `rust-analyzer` | `rust-analyzer scip <dir> --output <file>` |

An indexer runs only for a language that has files in the analysis, from the
analysis root (or the package root for monorepos), and writes its index below
`.modfence/scip/`. If an indexer cannot redirect its output, modfence moves
the resulting `index.scip` there, and refuses to run if it would overwrite an
`index.scip` it did not create. A missing indexer, a non-zero exit or an
unreadable index is a warning naming the language and leaves that language on
the tree-sitter backend; the run still succeeds.

**How SCIP feeds the graph.** For each SCIP document, definition
occurrences (role `Definition`) are matched to our symbol nodes by file and
range overlap. A reference occurrence at the same span as a tree-sitter call
site's callee resolves that call: the edge's target becomes the matched
definition's node, its `confidence` becomes `resolved` and its `origin`
`scip`. SCIP does not say whether a reference is a call, so a reference with
no tree-sitter call site adds nothing in v1 (an Open issue). SCIP positions
are converted using each document's declared encoding. SCIP does not cover
the Python-to-Rust boundary, so `binds` edges stay heuristic.

**Node kinds.** `package` (a directory with Python modules, a TypeScript
package directory, or a Rust crate), `module` (one per source file; a Rust
module resolves to its file), `symbol` (function, method, class, struct,
enum, interface, type, impl, trait, constant, variable) and `external` (a
third-party or standard-library package, named, never analysed). Ids are
strings of the form
`pkg|mod|sym|ext:<py|ts|rs>:<path or package>[#<qualified name>]`, paths
relative to the analysis root with `/` separators. Whitespace, `#` and `%`
in a path, package or qualified name are percent-encoded (`%20`, `%23`,
`%25`), so every id matches the schema's pattern and `#` separates only the
qualified name. Each extractor amends this ADR with its language's
qualified-name encoding (see Open issues). The id construction and these
encoding rules are implemented in `src/extract/ident.rs` (interface I3), which
also percent-encodes control characters and U+FEFF, because some regex engines
count them as whitespace.

**Languages.** `.py`; `.ts`, `.tsx`, `.mts`, `.cts`; and `.js`, `.jsx`,
`.mjs`, `.cjs`, which go through the TypeScript grammar and count as language
`ts` (user decision). `.rs`.

**Edge relations.** `contains` (package to module to symbol; never checked
by rules), `imports` (module to module or external), `calls` (symbol to
symbol, folded to one edge per source, target, relation and confidence with a
site count, so sites SCIP resolved and sites it did not are separate edges)
and `binds` (a call or import that crosses a language boundary).

**Rules see module-level edges** (user decision). Symbol-level `calls` and
`binds` sites roll up to their enclosing modules for rule evaluation, so one
rule set covers all three relations. Symbol nodes still exist in the graph and
in the JSON output, for exploration, `trace` and `--precise` joins.

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
over the cached facts. Content hashes, not salsa, keep the design small. The
walker always skips `.modfence/`.

**Tags in the graph.** Every node carries its effective tags (ADR 0004),
as `metadata.tags`: an object whose values are non-empty arrays of unique,
sorted strings. One shape for every key keeps consumers free of a
string-or-array union, and makes repeatable keys such as `adr` the same as
single-valued ones.

**Output format.** `modfence graph --format json` emits a strict
specialisation of JSON Graph Format v2: every document is also valid JGF v2,
because all modfence fields sit in the JGF `metadata` objects. The schema is
`schema/modfence-graph.schema.json`, the format version is
`graph.metadata.schema_version` (`major.minor`; a minor bump only adds
optional fields). JGF v2 forbids extra top-level keys, which is why the
version lives in `graph.metadata`. Changing tag values from string to array
is made before the first release, so the version is `1.0`.

**Positions.** 1-based lines and columns, columns counted in Unicode code
points, ends exclusive. SARIF output sets `columnKind` to
`unicodeCodePoints`.

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
7. A tree-sitter call edge is never `resolved`; only a SCIP match makes one.
8. A SCIP reference upgrades a call edge only when it lies at the span of a
   tree-sitter call site and its definition maps to one of our symbol
   nodes; otherwise the edge stays heuristic.
9. `binds` edges are always `heuristic`.
10. A cached fact is used only if the file hash and the extractor version
    both match; a corrupt or unwritable cache is a warning, never an error.
11. Every emitted document validates against the schema, and the schema is
    checked against its metaschema in CI.
12. The graph core parses source only below the analysis root. Besides it,
    it reads project metadata (`pyproject.toml`, `package.json`,
    `tsconfig.json` and what they `extend`, `Cargo.toml` including the
    workspace root), the config file found by ADR 0004, ignore files and the
    SCIP files named on the command line or produced by `--run-indexers`; it
    never parses third-party source such as `node_modules`.
13. Ids percent-encode whitespace, `#` and `%`; decoding an id yields the
    original path.
14. The schema pins `binds` edges and tree-sitter-origin `calls` edges to
    `heuristic`, and `contains` edges carry no metadata.
15. tree-sitter-typescript's parser ABI is asserted at test time to be
    loadable by the pinned tree-sitter (NC3b).
16. Without `--run-indexers`, modfence starts no subprocess; `--scip` only
    reads files.
17. `--run-indexers` starts only executables resolved from `PATH` and prints
    each command line to stderr before starting it. modfence's own writes are
    confined to `.modfence/` (the cache, and the index below `.modfence/scip/`
    that it moves there), and a pre-existing `index.scip` it did not create is
    never overwritten. What an indexer itself writes is outside this
    guarantee (see Consequences).
18. A failed or missing indexer is a warning and leaves that language on
    tree-sitter; it never changes the exit status on its own.
19. Every node's `tags` value is a non-empty array of unique strings sorted
    in byte order; a node with no tags has no `tags` key.
20. SCIP positions are converted from each document's declared position
    encoding to code points before a reference is matched to a call site.

## Decisions (G-api)

Approved by the user on 2026-11-01 (the "lean" of the original proposal
unless noted):

| # | Decision |
| --- | --- |
| 1 | SCIP: prebuilt `.scip` files and `--run-indexers` in v1 (the user went beyond the lean, which was prebuilt only). |
| 2 | JavaScript goes through the TypeScript grammar as language `ts`. |
| 3 | Rules apply to module-level edges only. |
| 4 | JGF v2 specialisation with `schema_version` in `graph.metadata`; the node id scheme and four node kinds; cache in `.modfence/cache/`; `binds` matched by exported name; positions in Unicode code points; first site plus a count per edge. |

Decisions A and B (GA3), approved by the user on 2026-11-01 as leaned
(C is decided in ADR 0004):

A. **Which edges `--run-indexers` upgrades.** Only existing tree-sitter call
   sites (as written), or also references SCIP reports that tree-sitter
   missed. Adding edges needs SCIP to say a reference is a call, which it does
   not. Lean: upgrade only.
B. **Indexer failure.** A warning and fall back (as written), or an exit-2
   error under `--run-indexers` so CI cannot silently lose precision. Lean:
   warning, plus `--require-precise` later if asked.
C. **Multi-value `--group-by`.** Decided in ADR 0004 (it only affects the
   outputs, not the graph).

## Qualified names by language

Each extractor adds its language's subsection here (NC3a Python, NC3b
TypeScript, NC3c Rust), with round-trip tests in `tests/`.

### Python qualified names

Implemented in `src/extract/python/qualname.rs` and tested in
`tests/extract_python.rs`. The path part of the id is the file, so the
qualified name never repeats the module: `sym:py:pkg/core.py#Engine.run`.

- **Components.** The qualified name is the chain of definition names from the
  module down, joined with `.`, as Python's `__qualname__` writes it. A method
  is `Class.method`; a nested class is `Outer.Inner`.
- **Definitions inside a function body.** A `def` or `class` whose enclosing
  scope is a function gets the component `<locals>` between the function and
  its name: `Engine.run.<locals>.inner`, and
  `Engine.run.<locals>.inner.<locals>.Local.deep` for a method of a class
  inside it. A definition inside a class body does not: `Engine.Config`.
  `if`, `try`, `with` and loop bodies are not scopes, so a definition in one is
  named as if the block were not there.
- **Symbols.** Every `def` (also `async def`) and `class` is a symbol, nested
  ones too: a `def` directly in a class body is a `method`, any other `def` a
  `function`, a `class` a `class`. A decorator belongs to the symbol it
  decorates, but the symbol's span starts at `def` or `class`, not at the first
  decorator. Lambdas, comprehensions and module-level or class-level variables
  are not symbols; a call inside a lambda belongs to the enclosing symbol.
- **Repeated names.** Python allows a name to be defined again in one scope
  (property setters, conditional definitions, `typing.overload`). The first
  definition keeps the plain name, and the n-th gets the suffix `@n` for n >= 2
  (`Engine.size`, `Engine.size@2`), so that every id is unique in the file. A
  definition nested in the second one is named after the plain chain
  (`C.m`, then `C.m@2` when the first `C` already has an `m`), because the
  suffix counts per full name. `@` is never part of a component (see below).
- **Escaping.** Each component is percent-encoded as in the rest of this ADR
  (whitespace, control characters, U+FEFF, `#` and `%`, upper-case hex), and
  `.` and `@` are escaped too (`%2E`, `%40`), so `.` only separates components
  and `@` only starts the ordinal. A valid Python identifier holds none of
  these characters; the escapes only matter for text recovered from a file with
  syntax errors. Non-ASCII identifiers are kept as written (no NFKC
  normalisation). `<` and `>` are not escaped; the schema's pattern allows
  them.
- **Round trip.** `decode_qualname` splits at the first `@`, then at `.`, and
  decodes each component; it accepts exactly what `encode_qualname` produces
  (lower-case hex, an empty component and an ordinal of 0, 1 or with a sign or
  leading zero are rejected), so a name has one id and an id has one name.
- **Packages and relative imports.** `pkg/__init__.py` is an ordinary module:
  its module node is `mod:py:pkg/__init__.py` and its symbols are named like
  any others' (`sym:py:pkg/__init__.py#helper`). Mapping the file to the
  package `pkg` is the core's job. An import's specifier is reported as
  written with its leading dots and without spaces: `from .core import X` is
  `.core`, `from . import y` is `.` with the name `y`, `from ..a.b import z` is
  `..a.b`, `import a.b as c` is `a.b`. The core resolves a dotted specifier
  against the importing file's package: one dot is the package of the file
  (the file's own directory), each further dot goes one directory up.

## Open issues

- `--run-indexers` on untrusted roots: a warning, a refusal in CI when the
  workflow is a fork pull request, or a sandbox. Not designed here.
- Indexer inputs: `scip-python`'s `--project-name` is taken from
  `pyproject.toml`'s `project.name`, else the analysis root's directory name;
  indexers run with the analysis root as working directory so SCIP document
  paths are root-relative; documents whose path matches no known file are
  counted and reported in one warning. NC6 verifies the exact flags and may
  amend this.
- An import that is relative or matches a configured alias (`tsconfig`
  `paths`, Python relative imports) but does not resolve is recorded as an
  `unresolved-import` diagnostic as well as an `external` edge, so it is not
  silently outside `layers` and deny mode (ADR 0004).

- Each of NC3a, NC3b and NC3c specifies its language's qualified-name encoding
  (Rust `impl<T> Foo for Bar`, TypeScript overloads, Python nested functions
  and lambdas) as an amendment to this ADR, with tests, before NC4 consumes
  ids.
- The schema does not tie each relation to node kinds (for example `imports`
  from a module); NC4 asserts it in tests.
- Whether `--run-indexers` should accept a per-language command override in
  `modfence.toml` (a monorepo with a custom `scip-typescript` wrapper).
- The `--precise` fallback for Rust workspaces where `rust-analyzer scip`
  needs a built `target/` directory.

## Consequences

- One graph and one rule engine serve all three languages and all outputs
  (DOT, Mermaid, JSON, GraphML, HTML, SARIF, `trace`).
- Without `--precise`, call-based findings do not exist by default: heuristic
  call edges are excluded from rules unless a rule opts in with
  `confidence = "any"` (ADR 0004). The graph always records them for
  exploration.
- `--run-indexers` executes code from the analysed repository: `rust-analyzer`
  runs build scripts and proc-macros, and the JavaScript and Python indexers
  read repository-controlled configuration and environments. Running it on an
  untrusted checkout (for example a pull request from a fork) is arbitrary code
  execution in the CI job. The indexers also write what they write
  (`index.scip`, an inferred `tsconfig.json`, `target/`); modfence only
  guarantees its own writes.
- `--run-indexers` is the one place where modfence runs other people's
  programs. Only what is already on `PATH` runs and every command is printed,
  but a malicious or compromised indexer in `PATH` runs with the user's
  rights. That is the same trust as running it by hand.
- Cross-language edges are name matches: a renamed export that collides with
  another module's name produces a false edge, and SCIP does not cover the
  Python-to-Rust boundary at all.
- The JSON format is public API once released. A major `schema_version`
  bump is a breaking change for every consumer.
- The tool depends on three grammars' release cadence; a grammar upgrade
  can change extraction and therefore needs the fixtures to be re-checked.
- Reading project metadata instead of invoking toolchains keeps the run
  offline and fast, but monorepo setups that rely on toolchain-only
  resolution (for example TypeScript project references with exotic
  `paths`) resolve partly, falling back to `external` edges.
- The in-tree cache and SCIP directory add `.modfence/` to the working tree;
  users add it to `.gitignore`.
