# src/cli/

The command line (ADR 0004): `modfence check`, `modfence graph` and
`modfence trace`. `run` is the whole program, taking the arguments and the
output streams; `src/main.rs` only forwards the process's.

- `mod.rs`: `run`, the exit codes and `Failure` (an exit status plus the
  message for standard error).
- `args.rs`: the `clap` derive structs. Keep default features off (NFR-0002:
  no network-capable crates).
- `commands.rs`: the three subcommands, configuration discovery and the
  analysis (`analyse`: config, walk, extract, build the graph).
- `walker.rs`: the file walker over the analysis root.
- `text.rs`: the human-readable output of `check` and `graph`, and the
  file-problem lines.
- `trace.rs`: the selector arguments, the selection (`select`) and the text
  output of `trace`.

The machine formats of `graph` and `trace` are `src/output/` (see its
`README.md`); `ViewArgs::renderer` in `args.rs` maps `--format` to a renderer.

Rules:

- Exit status: 0 when nothing is at or above `--fail-on` (`error` by default,
  also `warning` and `note`), 1 when something is, 2 for a usage or
  configuration error (including clap's own), 74 when a file or stream cannot
  be read or written (a missing or unreadable configuration file included, so
  `--config typo.toml` is 74; one that is read but invalid is 2). The status does not depend on `--format`.
- Human text is the default. Machine formats need an explicit `--format`:
  `check` has `text|sarif|github`; `graph` and `trace` have `text|json`.
  `dot|mermaid|graphml` are accepted by the parser and then refused with
  "not implemented yet (NC5)" (exit 2), so scripts fail loudly instead of
  getting text.
- `--level package|module|symbol` (default `module`) and `--group-by <tag key>`
  (an invalid key is a usage error) shape the diagram formats. `text` and
  `json` ignore them: JSON always carries everything. `trace --format json` is
  the graph document restricted to the selection (its nodes, their edges and
  those edges' other ends), with no extra metadata.
- Output goes to standard output, or to `--output <file>`. Notices, file
  problems and errors go to standard error and never into the report.
- Configuration: `--config <path>`, else the nearest `modfence.toml` found
  walking up from the working directory, else the defaults. The analysis root
  is the directory of the configuration file, or the working directory when
  there is none.
- A failed write to standard output says "cannot write output"; every failure
  that involves a path names it ("cannot walk <dir>", "cannot read <file>").
- The walker reads Python and TypeScript/JavaScript files (Rust arrives with
  its extractor). It always skips `.git/` and `.modfence/`, does not follow
  symbolic links, and leaves out what the configuration's `exclude` matches. An
  excluded directory is not read at all (so an unreadable one in `vendor/` is
  harmless); a directory that is not excluded and cannot be listed exits 74
  and names the path.
  It does not read `.gitignore` files: list `node_modules/`, virtual
  environments and build output in `exclude`. A file that is not valid UTF-8
  is decoded lossily.
- No per-file extraction cache yet: `FileFacts` has no serialised form, so
  `--cache-dir` and `--no-cache` do not exist. The cache (ADR 0003, keyed by
  content hash and `Extractor::version`) comes with that form.
- `trace` selectors are `key=value` arguments, AND-ed. A key given twice is an
  error (use commas for alternatives). A value is a comma-separated list of
  alternatives; each is either wrapped in double quotes as a whole or has no
  quote, and none may be empty, otherwise the selector is a usage error. The
  values are then parsed by the config's own `Selector`, so they are validated
  as in `modfence.toml`. A node is judged by `rules::subject`, the subject a
  rule would use, so `package=` skips the `external` node of an import that
  could not be placed, as in a rule. `trace` selects nodes, not edges: a node
  is unplaced only when no resolved import targets it, the node-level reading
  of the rules' per-edge exemption.
- There is no `--precise` flag until NC6, so call edges are not checked and
  `check` prints that notice.
- Tests are in `tests/cli.rs` (fixtures in `tests/fixtures/cli/`); do not add
  `#[cfg(test)]` modules here.
