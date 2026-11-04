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
  commands (at most 10 per level, with line, endLine, col and endColumn) and one summary line.

Rules:

- Output is deterministic and holds no timestamp or absolute path.
- Every SARIF result has a location with the four region fields, a
  `primaryLocationLineHash` and `properties`; an edge violation also has the
  target as a related location and the confidence and both endpoints' tags.
  The run records the notices as tool-execution notifications and
  `properties.modfence = { precise, callEdgesChecked }`.
- SARIF artifact URIs are relative URI references: `\` becomes `/` and every
  byte outside `A-Za-z0-9-._~/` is percent-encoded.
- A built-in rule may carry its own help text; configured rules use the generic
  advice. `callEdgesChecked` is currently derived from `precise`, so the two
  are equal. `executionSuccessful` is always true: a report is written only
  after an analysis that ran to the end. A fingerprint hash holds the rule and
  the endpoints of an edge, or the rule, path and message of an annotation
  finding, never a line number.
- SARIF holds every result; the 5000-result limit of GitHub code scanning is
  applied by the upload, and `rules::check` adds a notice when it is exceeded.
- Workflow-command data escapes `%`, CR and LF; properties also `:` and `,`.
- Tests are in `tests/report.rs` (the golden file is
  `tests/golden/report.sarif.json`); do not add `#[cfg(test)]` modules here.
