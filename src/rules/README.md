# src/rules/

The rules engine (ADR 0004, "Evaluation"). It reads a `graph::Graph` and a
`config::Config` and nothing else: no files, no processes.

- `mod.rs`: `check` and its result, `Outcome` (sorted `Violation`s, `Notice`s,
  whether the run was `--precise`). Applies the `modfence: ignore[...]`
  suppressions, reports the unused ones (`modfence/unused-ignore`) and turns
  the graph's annotation findings into `modfence/invalid-annotation`
  violations.
- `eval.rs`: `Judge`, which judges one site: layers, then the explicit rules in
  file order (first match decides), then the `default` effect.
- `lint.rs`: notices about the configuration (a rule that can never match,
  `layers` with no `layer` tag).

Rules:

- A site is judged on the module-level endpoints of its edge (`Graph::rollup`);
  `contains` edges, edges inside one module and edges with an endpoint the
  graph lacks are never judged.
- Which edges count: with `confidence` omitted, `imports` and `binds` edges of
  any confidence and `calls` edges only when `resolved`; `resolved` counts only
  resolved edges; `any` counts all. The layers rule and `default-deny` use the
  "omitted" rule.
- A `package` selector never matches the `external` node of an import that
  could not be placed (ADR 0003, invariant 6). This is decided per edge: only
  a `heuristic` `imports` edge is exempt, and a resolved edge to the same
  node still matches by package.
- An ignore covers the sites that start on its own line or the next, in its
  own file, and is judged per rule id: each id that suppressed nothing is one
  `modfence/unused-ignore` warning. An unused ignore can itself be ignored by
  another comment naming `modfence/unused-ignore`; a comment never covers its
  own warning.
- Notices are warnings about the run, not violations, and have no rule id.
  Turning a dead rule into a configuration error would need an ADR 0004
  decision (invariant 6 lists only one case).
- `Outcome::violations` is sorted by severity, rule id, path and position (the
  field order of `Violation`); the SARIF fingerprints rely on it being
  deterministic.
- Tests are in `tests/rules.rs`; do not add `#[cfg(test)]` modules here.
