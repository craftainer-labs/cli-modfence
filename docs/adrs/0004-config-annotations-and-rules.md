# 0004. Configure boundaries in modfence.toml and inline `arch:` comments, evaluated by ordered rules

## Status

Proposed. Becomes Accepted when its implementation merges (NC4), per
[`README.md`](README.md).

## Context

Users must state architecture once, for three languages, and get
violations in CI and in GitHub code scanning. Prior art: import-linter
(contracts in a config file), tach (modules and dependencies in TOML),
dependency-cruiser (`forbidden`, `allowed` and `allowedSeverity` rules).
Nodes need tags (layer, owner, boundary, deprecated) that come from path
globs most of the time, and from the source itself when a glob can't say it
(one symbol, one deprecated module). The graph model is
[ADR 0003](0003-graph-model.md); it defines nodes, edge relations and
confidence.

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

**Tags.** A tag is a lowercase key with a string value (no double quotes
or control characters). `layer`, `owner`, `boundary` and `deprecated` are
conventional; any key is allowed except the reserved selector keys `path`,
`package`, `lang` and `kind`, which are errors as tag keys, in config and
inline. A
`[[node]]` entry gives `tags` to the nodes matched by a path glob (`match`)
or by an external package name glob (`package`). Entries are applied in file
order, tags accumulate, and a later entry wins per key. Module tags apply to
the module's symbols unless the symbol is annotated itself.

**Inline annotations.** One syntax in all three languages, in a comment on its
own line: `# arch: layer=domain owner=team-a` (Python), `// arch: ...`
(TypeScript, Rust; `/* arch: ... */` on one line is accepted too).
Space-separated `key=value` pairs; a bare key means `true`; a value with
spaces is double-quoted. An annotation directly above a symbol (decorators,
attributes and doc comments may sit between) tags that symbol; any other
own-line annotation tags the whole module (so an annotation above an import
is a module tag). A comment whose body starts with `ignore[` is only ever a
suppression, never a tag annotation. Inline values override config values per
key. Annotations are read from comment nodes of the syntax tree, never from
strings; a shebang or encoding line is not an annotation.

**Suppressions.** `arch: ignore[rule-id, ...] reason="..."` on the line of
the import or call, or on the line above, suppresses violations of those
rules whose site (the statement or expression) starts on that line, so for a
multi-line import the first line counts. The ids `layers` and the built-in
`modfence/*` ids can be ignored too. The reason is required unless
`[annotations] require_reason = false`. An ignore that suppresses nothing is
itself a `warning`, and a malformed annotation is an `error`, under the
built-in rule ids `modfence/unused-ignore` and `modfence/invalid-annotation`.

**Rules.** A `[[rule]]` has an `id`, `from` and `to` selectors, an
`effect` of `forbid` or `allow`, and optionally `kinds` (default
`["imports", "binds"]`; `calls` must be asked for), `confidence`
(`any`, the default, or `resolved`), `severity` (`error`, `warning`,
`note`; default `error`) and `message`. A selector is a table: keys are
AND-ed, a list of values is an OR. Keys are tag names or the reserved
`path`, `package`, `lang` and `kind`.

**Evaluation.** For each module-level edge that is not `contains` and whose
endpoints differ, rules are tried in file order and the first one whose
selectors, `kinds` and `confidence` match decides: `forbid` reports a
violation with that rule's severity, `allow` permits the edge and stops. An
edge no rule matches is permitted. The layers rule is evaluated before every
explicit rule, so no `allow` can bypass it; it is the one rule that is not
order-dependent. Each violation records the edge's confidence, so heuristic findings are
visibly so.

**Layers.** `layers = ["app", "domain", "infra"]`, highest first, is
shorthand for a rule that forbids an edge from a lower layer to a higher one,
with the reserved id `layers` and severity `error`. A layer may use itself and every layer below.
Nodes without a listed `layer` tag are exempt.

**Reports.**

- *SARIF 2.1.0:* one run per invocation. `ruleId` is the rule id, and levels
  map one to one (`error`, `warning`, `note`). Every rule, built-in ones
  included, carries non-empty short and full descriptions and help text. The
  primary location is the import or call site, the target node is a related
  location, and `columnKind` is `unicodeCodePoints`. Each result has
  `properties.confidence`.
- *Fingerprint:* `partialFingerprints.primaryLocationLineHash` is
  `<hash>:<n>` (the form GitHub uses). For an edge violation the hash covers
  rule id, source node id, target node id and relation; for the built-in
  rules it covers the rule id and the path. `n` is the 1-based position of
  the result among results with the same hash, ordered by path and position,
  so it does not depend on iteration order. The alert survives line moves (see
  Decision 2 below).
- *GitHub annotations* (`--format github`): `::error`, `::warning` or
  `::notice` workflow commands, with properties and message escaped, limited
  to the per-step caps and followed by one summary line.
- *Exit status:* 0 when nothing is at or above `--fail-on` (default `error`),
  1 when something is, 2 for a usage or configuration error.

## Invariants for NC4

1. Unknown config keys, duplicate rule ids, the reserved ids `layers` and
   `modfence/*`, and duplicate names in `layers` are exit-2 errors with the
   file and line.
2. Rules are evaluated in file order and the first match decides; the layers
   rule runs before all of them and no `allow` overrides it.
3. An edge whose endpoints are the same node is never reported, and
   `contains` edges are never checked.
4. A rule applies to `imports` and `binds` unless `kinds` says otherwise.
5. A rule with `confidence = "resolved"` never matches a heuristic edge.
6. Effective tags are computed as: config entries in order, then the module's
   inline tags, then the symbol's own inline tags; the last writer wins per
   key.
7. An annotation above a symbol tags the symbol; otherwise it tags the
   module.
8. `arch:` text inside a string or docstring is never an annotation.
9. An ignore matches by rule id and the first line of the site (or the line
   above); it never suppresses another rule's violation. A comment starting
   `ignore[` is never read as a tag annotation.
10. An unused ignore is reported as `modfence/unused-ignore` at `warning`.
11. SARIF output validates against the SARIF 2.1.0 schema, and every result
    has a location with all four region fields and a fingerprint.
12. SARIF results are ordered by severity, then rule id, then path and line;
    above 5,000 results the tool prints a warning that GitHub keeps only the
    top 5,000.
13. Annotation output escapes `%`, CR and LF in messages and `%`, CR, LF, `:`
    and `,` in properties, and never exceeds 10 errors, 10 warnings and
    10 notices.
14. Exit status follows the table above, independent of the output format.

## Decisions for the user (G-api)

Open (sources disagree, or a matter of taste):

1. **Inline prefix.** `arch:` (the plan's draft; tool-neutral, survives a
   rename) or `modfence:` (cannot collide with another tool's comments).
   Lean: `arch:`.
2. **Alert fingerprint.** (a) Our semantic hash under
   `primaryLocationLineHash`: stable across edits, but it repurposes a key
   whose documented meaning is a line hash. (b) Omit it and let the
   `upload-sarif` action compute it: standard, but alerts reopen when lines
   move, and the raw API gets duplicates. Lean: (a).
3. **Default effect.** First match wins and an unmatched edge is allowed
   (as above), or an optional `default = "deny"` allow-list mode like
   dependency-cruiser's `allowed`. Allow-lists are stricter but need every
   permitted dependency listed. Lean: default allow now; deny mode later if
   asked.
4. **Layers strictness.** A layer may use any lower layer (as above), or only
   the adjacent one (import-linter's `layers` contract has both through a
   `strict` option). Lean: non-strict now.
5. **Where `calls` lives.** Opt-in per rule (as above), or on by default
   with `confidence = "resolved"` implied. Opt-in avoids false positives from
   heuristic edges; the other gives more coverage with `--precise`. Lean:
   opt-in.

Confirmations (proposed; no real alternative with a different cost):

6. `modfence.toml` at the repository root, `version = 1`, unknown keys are
   errors, with a published JSON schema.
7. Tags are string key/value pairs; inline overrides config per key.
8. Suppression needs a reason, and an unused ignore warns.
9. Selector shape (tables, AND across keys, OR within a list) and the
   reserved keys `path`, `package`, `lang`, `kind`.
10. `[[node]]` with `package` tags external nodes.
11. Exit codes 0, 1, 2 and `--fail-on`.
12. SARIF rule ids equal config rule ids; one run per invocation.

## Consequences

- Architecture lives in one file that works across languages, and the same
  text lets a non-engineer read the rules. TOML has no anchors or includes,
  so large monorepos repeat selectors; a future `include` would be a new
  config version.
- First-match-wins is easy to explain and easy to get wrong: a broad `allow`
  placed early silently disables later `forbid` rules. The invariants make
  the order explicit and a future `modfence check --explain` can show which
  rule decided an edge.
- Inline annotations put architecture facts next to the code they describe,
  at the cost of a comment convention in three languages that the extractors
  must parse exactly (decorators, attributes, doc comments).
- Cycle detection (dependency-cruiser's `circular`, import-linter's
  independence contracts) is not a rule type here. It is deferred and
  should be added as a built-in `modfence/cycle` rule.
- The semantic fingerprint changes if a node is renamed or moved, which
  reopens that alert once; that is intended.
- GitHub keeps only 5,000 results per upload, so the first run on a repository
  with a bad rule can hide violations. The warning and the severity ordering
  limit the damage; a baseline file is out of scope.
