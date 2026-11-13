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
- `precise.rs`: `wanted` and `ingest`, the SCIP flags (below).
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
  `check` has `text|sarif|github`; `graph` and `trace` have
  `text|json|dot|mermaid|graphml|html`.
- `--level package|module|symbol` (default `module`) and `--group-by <tag key>`
  (an invalid key is a usage error) shape the diagram formats (`dot`, `mermaid`, `html`). `text`, `json` and
  `graphml` ignore them: the data formats always carry everything. `trace --format json` is
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
- `--precise` and `--scip <file>` (repeatable, relative to the working
  directory; `--scip` implies `--precise`) are flags of all three commands
  (`Common` in `args.rs`); `precise.rs` handles them. Without either, call
  edges are not checked and `check` prints that notice.
  - With `--scip`, exactly those files are read, in command-line order. A file
    that cannot be read (missing, not a regular file, a read error) exits 74;
    one that is not a SCIP index, or is over the size cap, exits 2.
  - Bare `--precise` reads the `*.scip` files in `.modfence/scip/` below the
    analysis root, in file-name order. None there is a warning, and the run
    goes on as without `--precise`. A broken file there exits like a broken
    `--scip` file: 74 when it cannot be read, 2 when it is not an index or is
    over the size cap. Files without the `.scip` extension are ignored.
  - Call edges count as checked (`rules::check`'s `precise`, SARIF's
    `callEdgesChecked`) only when at least one document of an index was used.
    An index with none (its root is outside the analysis root, say) is a
    warning and leaves the run on tree-sitter.
  - Everything the reader or `graph::precise::apply` skipped is a `warning:`
    line, and each index with a tool name gets one `note:` line. Text from an
    index is escaped before it is printed (`scip::printable`).
  - Of two documents with one path, the first read wins (so command-line order
    decides); the repeat is counted in a warning.
  - `--run-indexers` (implies `--precise`) runs `scip-python`,
    `scip-typescript` and `rust-analyzer` for the languages analysed, writing
    `.modfence/scip/python.scip`, `typescript.scip` and `rust.scip`, then reads
    those (after any `--scip` files). The command lines are `scip::run::plan`'s;
    `PATH` is searched by modfence itself (absolute entries only) and each
    command line is printed as `note: running ...` before it runs. No shell, the
    environment is inherited, there is no timeout, and the indexer's standard
    output goes to standard error. A missing indexer, a failure or no output
    is a warning that leaves that language on tree-sitter and does not change
    the exit status; an index that an indexer wrote but that cannot be read
    exits like a broken `--scip` file (74, or 2 when it is not an index). An
    index left by an earlier run is removed first.
    Unix only: elsewhere it warns and runs nothing (CVE-2024-24576).
  - Memory: the text of every analysed file is kept for the position
    conversion (`Sources`) and every index is held until `apply` has run,
    because `apply` settles duplicate documents and symbols across all of them.
    The cost is the source size plus the encoded indexes (each at most 512 MiB)
    times a small factor for the parsed form; an index is far larger than the
    source it describes, so the copy of the source is not the term that matters.
    It is not reduced further because that would need `apply` to read files
    itself, which the graph code does not do (`src/graph/README.md`).
  - Indexes in `.modfence/scip/` that this run did not plan (a language that is
    not analysed, or a file from an earlier run) are left alone, not deleted and,
    with `--run-indexers`, not read: only what the indexers just wrote is. A
    `.modfence` or `.modfence/scip` that is a symbolic link is neither written
    through nor read through (a warning for `--run-indexers`); nor is a
    symbolic link to an index inside the directory.
  - `PATH` lookup treats a file as executable when any execute bit is set,
    like the lookup of most tools; one the user cannot run then fails when it is
    started, which is a warning like any other failure to start.
  - There are no `modfence.toml` keys for any of this.
- Tests are in `tests/cli.rs` (fixtures in `tests/fixtures/cli/`),
  `tests/indexers.rs` (`--run-indexers` with fake indexers on `PATH`) and
  `tests/offline.rs` (the binary under `unshare -n`, NFR-0002; falls back to `sudo -n unshare -n`,
  and `MODFENCE_SKIP_SANDBOX` skips it); do not add
  `#[cfg(test)]` modules here.
