# src/config/

`modfence.toml` (ADR 0004): the configuration model, its parser and the tag
precedence. It reads no files and starts nothing; the caller passes the text.

- `mod.rs`: `parse` turns the text into a `Config` (layers, default effect,
  `[tags] accumulate`, `[annotations]`, `[[node]]` entries, `[[rule]]`s) or a
  `ParseError` that carries the byte span, line and column of the key or value
  that is wrong. `Config::effective_tags` combines the matching `[[node]]`
  entries with the inline levels.
- `selector.rs`: `Selector` and `Subject` (what rules and `[[node]]` entries
  match), and the value parsers. Each parser fails inside the value's own
  `Deserialize` impl, so `toml` attaches the span of that key or value; keep
  new checks there rather than after parsing, unless they compare siblings
  (duplicates), which use `toml::Spanned`.
- `precedence.rs`: `validate_tag_key` and `resolve_tags`, the one place tag
  precedence is implemented.

Rules:

- Unknown keys are errors (`deny_unknown_fields`); do not add a catch-all.
- Path and package globs are built with `literal_separator(true)`, so `*` does
  not cross `/`.
- `exclude` patterns follow gitignore: without a `/` (other than a trailing
  one) a pattern matches at any depth; with one it is anchored at the analysis
  root, as is a leading `/`; a trailing `/` matches directories and everything
  below; `*` does not cross `/`, `**` does. Negation (`!`) is not supported.
- `package = "*"` does not match scoped npm names such as `@babel/core`
  (`*` does not cross `/`); use `@*/*` for those, or `**` for all packages.
- Selector tag values go through the same `TagValue` parser as node tags.
  Migration: a selector value that is empty or holds a double quote or a
  control character used to parse and never match; it is now an exit-2 config
  error, so fix or remove such a value.
  Selector keys are lowercase names (`SelectorKey`, the pattern of
  `validate_tag_key`); ADR 0004 allows any tag key, so unknown keys are not
  errors.
- Tag value sets are never empty (the TOML and annotation parsers reject
  them); `resolve_tags` still treats an empty set as no contribution.
- Tests are in `tests/config.rs`; do not add `#[cfg(test)]` modules here.
