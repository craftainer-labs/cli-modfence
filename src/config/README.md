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
- Tests are in `tests/config.rs`; do not add `#[cfg(test)]` modules here.
