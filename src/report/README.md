# src/report/

Output of the rules engine (ADR 0004, "Reports"): pure functions from a
`rules::Outcome` to text. They print nothing, and they do not decide the exit
status (`Outcome::fails` does).

- `sarif.rs`: `render` builds a SARIF 2.1.0 document by hand with
  `serde_json`. Keys come out sorted, results in the outcome's order. Rules
  are the four built-ins (`layers`, `default-deny`, `modfence/invalid-annotation`,
  `modfence/unused-ignore`) then the config's. Fingerprints are `<hash>:<n>`;
  the hash is FNV-1a over fixed fields, defined here because the standard
  library's hashers are not specified.
- `github.rs`: `render` writes `::error`, `::warning` and `::notice` workflow
  commands (at most 10 per level) and one summary line.

Rules:

- Output is deterministic and holds no timestamp or absolute path.
- Every SARIF result has a location with the four region fields, a
  `primaryLocationLineHash` and `properties`; an edge violation also has the
  target as a related location and the confidence and both endpoints' tags.
  The run records the notices as tool-execution notifications and
  `properties.modfence = { precise, callEdgesChecked }`.
- Workflow-command data escapes `%`, CR and LF; properties also `:` and `,`.
- Tests are in `tests/report.rs` (the golden file is
  `tests/golden/report.sarif.json`); do not add `#[cfg(test)]` modules here.
