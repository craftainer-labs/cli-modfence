# 0004. Configure boundaries in modfence.toml and inline `modfence:` comments, evaluated by ordered rules

## Status

Proposed. The G-api decisions in the table "Decisions (G-api)", and the items
A to E below it (including the tag `accumulate` option, the orchestrator's
proposal), were approved by the user on 2026-11-01. NC4 (config, annotations,
rules and reports: cli-modfence #10, #11, #12, #13 and #16) and NC5 (`trace`
and the tag outputs: #17, #18, #19 and the accessibility check) implemented
most of it. It becomes Accepted, per [`README.md`](README.md), after the rest
lands: NC3c (the Rust extractor, #8, so `// modfence:` annotations work in
Rust) and NC6 (the `--precise` flag). NC5e (validating SARIF against the
2.1.0 schema and the remaining tests) is done; the Mermaid tag tooltip is
dropped, because Mermaid has no tooltip without a URL or callback (see
`src/output/README.md`).

## Context

Users must state architecture once, for three languages, and get
violations in CI and in GitHub code scanning. Prior art: import-linter
(contracts in a config file), tach (modules and dependencies in TOML),
dependency-cruiser (`forbidden`, `allowed` and `allowedSeverity` rules).
Nodes need tags (layer, owner, boundary, deprecated) that come from path
globs most of the time, and from the source itself when a glob can't say it
(one symbol, one deprecated module). Teams also want to trace code to
decisions and requirements (ADR ids, requirement ids), which are tags with
several values per node. The graph model is [ADR 0003](0003-graph-model.md);
it defines nodes, edge relations and confidence.

Reporting constraints, from GitHub's SARIF support page
(<https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/sarif-support-for-code-scanning>)
and the SARIF 2.1.0 standard
(<https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>):

- upload limit 10 MB gzipped, 20 runs per file; per run, 25,000 results of
  which only the top 5,000 (by severity) are kept;
- required: each rule's `id`, `shortDescription.text`, `fullDescription.text`
  and `help.text`; each result's `message.text`, a location with all four
  region fields, and `partialFingerprints`;
- only the `primaryLocationLineHash` fingerprint is used for matching alerts
  across runs. The `upload-sarif` action fills it in when absent; the raw
  API does not;
- the page contradicts itself on the number of locations per result (10 in
  the property text, 1,000 in the limits table).

Workflow-command annotations are capped (10 errors, 10 warnings and 10 notices
per step, 50 per job;
<https://github.com/actions/toolkit/blob/main/docs/problem-matchers.md>).

## Decision

**Config file.** `modfence.toml` at the repository root, found by walking up
from the working directory (`--config` overrides). It has `version = 1`;
unknown keys are errors, so a typo cannot silently disable a rule. Its schema
is `schema/modfence-config.schema.json`, for editors and CI.

**Tags.** A tag is a lowercase key with one or more string values (no double
quotes or control characters). The keys `layer`, `owner`, `boundary` and
`deprecated` are conventional; any key is allowed (`adr`, `req` and the like
for traceability) except the reserved selector keys `path`, `package`,
`lang` and `kind`, which are errors as tag keys in config and inline. A
`[[node]]` entry gives `tags` to the nodes matched by a path glob (`match`)
or by an external package name glob (`package`); in config a value is a
string or a list of strings. An entry applies by file path to every node in
the matching files. A node's tag values are a sorted, de-duplicated list in
every output.

**Tag precedence** (the default is the user's; the `accumulate` option was
the orchestrator's proposal, approved on 2026-11-01, Decisions A). Levels, lowest
first: `[[node]]` entries in file order, the module's inline annotations, the
symbol's own inline annotations. A symbol inherits its module's effective
tags. By default the highest level that sets a key supplies that key's whole
value list and replaces the lower ones. Keys listed in `[tags] accumulate`
instead take the union of the values from all levels, so a symbol can add
`adr=0029` to its module's `adr=0028` and carry both. Several annotations at
the same level union their values for a key.

**Inline annotations.** One syntax in all three languages, in a comment on
its own line: `# modfence: layer=domain owner=team-a` (Python),
`// modfence: ...` (TypeScript, Rust; `/* modfence: ... */` on one line is
accepted too). Space-separated `key=value` pairs; a bare key means the value
`true` (serialised as `["true"]`); a value with spaces is double-quoted; a key may repeat to give several
values (`# modfence: adr=0028 adr=0029`). An annotation directly above a
symbol (no blank line between; decorators, attributes and doc comments may
sit between) tags that symbol; any other own-line annotation tags the whole
module, so a file-header annotation needs a blank line after it (an annotation
above an import is a module tag). A comment whose body starts with `ignore[`
is only ever a suppression, never a tag annotation. Annotations are read from
comment nodes of the syntax tree, never from strings; a shebang or encoding
line is not an annotation.

**Suppressions.** `modfence: ignore[rule-id, ...] reason="..."` on the line of
the import or call, or on the line above, suppresses violations of those
rules whose site (the statement or expression) starts on that line, so for a
multi-line import the first line counts. The ids `layers`, `default-deny` and
the built-in `modfence/*` ids can be ignored too. The reason is required
unless `[annotations] require_reason = false`. An ignore that suppresses
nothing is itself a `warning`, and a malformed annotation is an `error`, under
the built-in rule ids `modfence/unused-ignore` and
`modfence/invalid-annotation`.

**Rules.** A `[[rule]]` has an `id`, `from` and `to` selectors, an
`effect` of `forbid` or `allow`, and optionally `kinds` (default
`["imports", "binds", "calls"]`), `confidence`, `severity` (`error`,
`warning`, `note`; default `error`) and `message`. A selector is a table: keys
are AND-ed, a list of values is an OR, and a key matches a node if **any** of
the node's values for it matches (so `{ adr = "0028" }` selects every node
traced to ADR 0028). Keys are tag names or the reserved `path`, `package`,
`lang` and `kind`. Rules apply to module-level edges.

**Call edges in rules** (user decision). `calls` is a default kind, but only
`resolved` call edges count. Heuristic call edges are ignored unless a rule
opts in with `confidence = "any"`. With `confidence` omitted, `imports` and
`binds` edges of any confidence count and `calls` edges only when `resolved`;
`confidence = "resolved"` counts only resolved edges (so it never matches a
`binds` edge); `confidence = "any"` counts every edge, heuristic calls
included. Without `--precise` no call edge is ever `resolved`, so call edges
are not checked; the tool prints one notice on stderr ("call edges were not
checked; run with --precise"), and records it in the SARIF run as a
tool-execution notification at level `note` plus
`runs[].properties.modfence = { "precise": false, "callEdgesChecked": false }`.

**Evaluation.** Rules are evaluated per site: each import or call site of a
module-level edge (not `contains`, endpoints differing) is judged on its own
and reported at its own location, so suppressions and SARIF locations are
exact even when ADR 0003 folds many sites into one graph edge. For each site:

1. The layers rule (below) is tried first. A violation is reported and no
   explicit rule can override it.
2. Otherwise explicit rules are tried in file order and the first one whose
   selectors, `kinds` and `confidence` match decides: `forbid` reports a
   violation with that rule's severity, `allow` permits the site and stops.
3. A site no rule decided is permitted, unless the default is `deny`.

Each violation records the confidence of its edge, so heuristic findings are
visibly so.

**Default effect** (user decision). `default = "allow"` (the default) or
`default = "deny"` at the top level of the config. In deny mode a site whose
source and target are both nodes of the repository (not `external`), and
which no explicit rule and no layers permission decided, is a violation of the
reserved rule `default-deny` at severity `error`: "No rule allows this
dependency." Imports of external packages stay allowed unless a rule forbids
them. Deny mode interacts with the other features as follows (the executor's
definitions, approved on 2026-11-01, Decisions B):

- *Layers:* a site counts as permitted by layers when every `layer` value of
  both nodes is listed in `layers` and no combination of the two nodes' values
  violates the order, so `layers` alone can describe a whole architecture in
  deny mode. A site that violates the order for any combination is reported as
  `layers`, never also as `default-deny`.
- *Exempt nodes:* a node with no `layer` tag, or a value not listed in
  `layers`, is exempt from the `layers` rule, which means layers cannot
  permit a site that touches it. In deny mode such a site needs an explicit
  `allow` rule.
- *Unresolved imports:* an import that looks internal but could not be placed
  (ADR 0003, Open issues) is an `external` edge and also an
  `unresolved-import` diagnostic; deny mode and `layers` do not see it, so
  the diagnostic is a warning in `check` output.
- *Confidence:* `default-deny` considers only sites an explicit rule with
  `confidence` omitted could match: `imports` and `binds` sites, and `calls`
  sites only when resolved. A heuristic call site is never reported by it
  (`binds` sites are name matches, so deny mode inherits their false-positive
  risk, noted in Consequences).
- *Explicit rules:* `allow` rules are what deny mode is for; `forbid` rules
  still decide first-match as before and so give a specific message.
- *Suppressions:* `modfence: ignore[default-deny] reason="..."` suppresses it
  at a site, and an unused one warns like any other.

**Layers.** `layers = ["app", "infra", "domain"]`, highest first, is
shorthand for a rule that forbids an edge from a lower layer to a higher one,
with the reserved id `layers`, severity `error` and the same `kinds` and
`confidence` rules as above. A layer may use itself and every layer below
(user decision: non-strict). Nodes without a listed `layer` tag are exempt;
for a node with several `layer` values the site is judged against each
combination and any violation reports.

**Reports.**

- *SARIF 2.1.0:* one run per invocation. `ruleId` is the rule id, and levels
  map one to one (`error`, `warning`, `note`). Every rule, built-in ones
  included, carries non-empty short and full descriptions and help text. The
  primary location is the import or call site, the target node is a related
  location, and `columnKind` is `unicodeCodePoints`. Each result has
  `properties = { "confidence": ..., "from": { "node": id, "tags": {...} },
  "to": { "node": id, "tags": {...} } }`, the tags being the two endpoints'
  tags (key to list of values), so traceability data reaches code scanning.
- *Fingerprint* (user decision): `partialFingerprints.primaryLocationLineHash`
  is `<hash>:<n>` (the form GitHub uses). For an edge violation the hash
  covers rule id, source node id, target node id and relation; for the
  built-in rules it covers the rule id and the path. `n` is the 1-based
  position of the result among results with the same hash, ordered by path and
  position, so it does not depend on iteration order. The alert survives line
  moves.
- *GitHub annotations* (`--format github`): `::error`, `::warning` or
  `::notice` workflow commands, with properties and message escaped, limited
  to the per-step caps and followed by one summary line.
- *Exit status:* 0 when nothing is at or above `--fail-on` (default `error`),
  1 when something is, 2 for a usage or configuration error.

**Traceability outputs** (owned by NC5, not NC4). Tags are visible wherever
the graph is: the DOT, Mermaid, GraphML and HTML outputs show a node's tags
on hover (DOT `tooltip`, GraphML data keys, HTML title text; Mermaid as far
as its click/tooltip support goes); the HTML output can filter by any tag
value; and `--group-by <key>` clusters nodes by that tag's values, in place of
the default package grouping. A node with several values for the key sits in
the cluster of the first value in sorted order and lists the others on hover
(see Decisions, C). `modfence trace <selector>...` prints the matching nodes
and the edges incident to them, as text or JSON (`--format`); the selector
arguments are `key=value` pairs, AND-ed, with commas for OR within a key
(`modfence trace adr=0028 layer=domain,infra`), reusing the rule selector
semantics. A value containing a comma, `=` or a space is written in double
quotes (`owner="team a,team b"`), and a comma inside quotes is literal. The JSON form is a graph document restricted to the matching nodes,
their incident edges and those edges' other endpoints, so it validates against
the graph schema.

## Invariants for NC4 and NC5

1. Unknown config keys, duplicate rule ids, the reserved ids `layers`,
   `default-deny` and `modfence/*`, a reserved word as a tag key, and
   duplicate names in `layers` are exit-2 errors with the file and line.
2. Selector keys must be lowercase names; `lang` and `kind` values must come
   from their enums; anything else is an exit-2 error.
3. Evaluation order per site is: layers, then explicit rules in file order
   (first match decides), then the default. No `allow` overrides `layers`.
4. An edge whose endpoints are the same node is never reported, and
   `contains` edges are never checked.
5. Violations are reported per site, in the file and at the position of the
   import or call; a symbol-level site is judged with the module-level
   endpoints it rolls up to.
6. A rule applies to `imports`, `binds` and `calls` unless `kinds` says
   otherwise. A rule that can never match, which is `confidence =
   "resolved"` with `kinds` containing only `binds` (binds edges are always
   heuristic), is an exit-2 config error.
7. With `confidence` omitted, a heuristic `calls` edge never matches and an
   `imports` or `binds` edge always can; with `"resolved"` no heuristic edge
   matches; with `"any"` every edge can.
8. Without `--precise`, one notice that call edges were not checked is
   printed once and recorded in the SARIF run, and no call edge produces a
   finding unless a rule says `confidence = "any"`.
9. In deny mode, an in-repository site that no rule and no layers permission
   decided is reported once as `default-deny`; an `external` target is never
   reported by it, and neither is a heuristic `calls` site. Layers
   permission requires every `layer` value of both nodes to be listed and no
   combination to violate the order.
10. A site that violates `layers` is never also reported as `default-deny`.
11. A node's effective tags are computed level by level (config entries in
    order, module inline, symbol inline). A non-accumulating key takes the
    highest level that sets it; an accumulating key takes the union; the
    result is sorted and de-duplicated. A symbol keeps every module tag it
    does not override.
12. A selector value matches a node if any of the node's values for that key
    equals it.
13. `[tags] accumulate` lists tag keys only, never reserved words, and a key
    may appear once.
14. An annotation above a symbol (no blank line between) tags the symbol;
    otherwise it tags the module.
15. `modfence:` text inside a string or docstring is never an annotation.
16. An ignore matches by rule id and the first line of the site (or the line
    above); it never suppresses another rule's violation. A comment starting
    `ignore[` is never read as a tag annotation.
17. An unused ignore is reported as `modfence/unused-ignore` at `warning`.
18. SARIF output validates against the SARIF 2.1.0 schema, and every result
    has a location with all four region fields, a fingerprint and `properties`
    holding the confidence and both endpoints' tags.
19. SARIF results are ordered by severity, then rule id, then path and line;
    above 5,000 results the tool prints a warning that GitHub keeps only the
    top 5,000.
20. Annotation output escapes `%`, CR and LF in messages and `%`, CR, LF, `:`
    and `,` in properties, and never exceeds 10 errors, 10 warnings and
    10 notices.
21. Exit status follows the table above, independent of the output format.
22. The analysis walker always skips `.modfence/`, and rules never see it.
23. `modfence trace` output in JSON validates against the graph schema, and
    for any selector it returns every matching node and every edge with a
    matching endpoint, nothing else.

## Decisions (G-api)

Approved by the user on 2026-11-01:

| # | Decision |
| --- | --- |
| 1 | Inline prefix `modfence:` (not `arch:`). |
| 2 | Alert fingerprint: our own semantic hash under `primaryLocationLineHash`. |
| 3 | Default effect is allow, and `default = "deny"` exists in v1 (internal edges only; external imports stay allowed). |
| 4 | Layers are non-strict. |
| 5 | `calls` is a default rule kind but only for `resolved` edges; heuristic calls need `confidence = "any"` per rule; a notice and SARIF property when `--precise` is absent. |
| 6 | Tags are free-form and repeatable (`adr=0028 adr=0029`); config accepts lists; a selector matches on any value. |
| 7 | Outputs show tags (SARIF result properties, DOT, Mermaid, HTML hover and filter, `--group-by <key>`) and `modfence trace <selector>` (text or JSON), owned by NC5. |
| 8 | Module scope marker: stays the blank-line rule; add an explicit `modfence: module ...` scope word only if NC3 finds the rule error-prone. |
| 9 | Config file at the repository root, `version = 1`, unknown keys are errors, published JSON schema; tags are strings; inline overrides config per key; suppression needs a reason and an unused ignore warns; selector shape and reserved keys; `[[node]] package`; exit codes 0, 1, 2 with `--fail-on`; SARIF rule id equals config rule id, one run per invocation. |

Decisions A to E: proposals made in this revision, approved by the user on
2026-11-01 as leaned (each lean below is what was approved):

A. **Tag accumulation.** The orchestrator proposed it: per-key override stays
   the default, `[tags] accumulate = ["adr", "req"]` makes chosen keys union
   across config, module and symbol. Alternative: all keys accumulate, or no
   key does (a symbol restates the module's values). Lean: as proposed,
   because it keeps layers safe (a symbol must not gain a second `layer`) and
   makes traceability keys additive.
B. **Layers in deny mode.** A layers-compliant site counts as permitted (as
   written), or every internal edge still needs an `allow` rule. Lean: as
   written, so layers alone can describe an architecture.
C. **`--group-by` with several values.** The first sorted value (as written),
   or the node duplicated into every cluster (HTML only; DOT and Mermaid
   cannot share a node between clusters cleanly). Lean: first value.
D. **`trace` scope.** Matching nodes plus incident edges (as written), or the
   transitive closure with `--depth N`. Lean: incident edges now, `--depth`
   later.
E. **Deny severity.** `default-deny` is always `error`, or settable
   (`default_severity`). Lean: always `error`; a team that wants softer
   findings keeps `default = "allow"` and writes `forbid` rules.

## Open issues

- The graph schema's tag shape (arrays, sorted) is `schema_version` 1.0
  before the first release. If decisions A to E change tag semantics, the
  schema is amended in the same change; after the first release it is a
  major bump.
- NC4 proves tag precedence with table tests: one row per combination of
  config entries, module inline, symbol inline, `accumulate` membership and
  repeated keys, with the expected sorted list, before any rule consumes it.
- `modfence trace` is implemented by NC5; the SARIF properties by NC4.
- Whether a SCIP reference with no tree-sitter call site should create a call
  edge (ADR 0003, A): resolved, no. ADR 0003 Decision A (the SCIP rules, "Only
  existing tree-sitter call sites can be upgraded") makes SCIP upgrade
  existing call sites only, and NC6b (cli-modfence #24) implemented it.

## Deferred (not in v1)

- Cycle detection (dependency-cruiser's `circular`, import-linter's
  independence contracts) is not a rule type here. It should become a
  built-in `modfence/cycle` rule later; v1 rules judge single edges, and a
  cycle needs a graph algorithm and a new rule type.
- A baseline file for adopting the tool on a repository with existing
  violations: out of scope for v1; a team can start with `default = "allow"`
  and add `forbid` rules.

## Consequences

- Architecture lives in one file that works across languages, and the same
  text lets a non-engineer read the rules. TOML has no anchors or includes,
  so large monorepos repeat selectors; a future `include` would be a new
  config version.
- First-match-wins is easy to explain and easy to get wrong: a broad `allow`
  placed early silently disables later `forbid` rules (not `layers`, which
  runs first). A future `modfence check --explain` can show which rule decided
  an edge.
- Deny mode makes the rule file a complete allow-list of internal
  dependencies, which is strict and costs one `allow` per permitted pair
  unless `layers` covers them; external imports are not allow-listed. A
  heuristic `binds` edge that is a false match can fail CI in deny mode until
  it is allowed or ignored.
- Heuristic call edges are invisible to rules by default, so a first run
  without `--precise` checks imports only; the notice says so. A rule that
  opts in to `confidence = "any"` accepts false positives.
- Inline annotations put architecture facts next to the code they describe,
  at the cost of a comment convention in three languages that the extractors
  must parse exactly (decorators, attributes, doc comments).
- Multi-valued tags make traceability queries (`trace`, selectors) cheap, but
  `accumulate` is a second precedence regime that users must learn, and
  selectors on a multi-valued key match more nodes than they might expect.
- The semantic fingerprint changes if a node is renamed or moved, which
  reopens that alert once; that is intended.
- A built-in finding's fingerprint is the rule and the path plus `:n`, so
  fixing an earlier finding of the same rule in the same file renumbers the
  later `:n` suffixes, and GitHub may close one alert and keep another open
  (GitHub's own `primaryLocationLineHash` `:n` behaves the same way).
- GitHub keeps only 5,000 results per upload, so the first run on a repository
  with a bad rule can hide violations. The warning and the severity ordering
  limit the damage.
