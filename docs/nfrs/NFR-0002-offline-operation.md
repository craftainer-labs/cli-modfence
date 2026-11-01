# NFR-0002. The tool works fully offline and makes no network connection

## Status

Proposed

## Attribute

Security and portability (air-gapped and sandboxed use).

## Description

No command of `modfence` opens a network connection, in any mode. `modfence`'s
own process never does, with no exception. The one carve-out is the opt-in
`--run-indexers` flag ([ADR 0003](../adrs/0003-graph-model.md), approved
2026-11-01): the indexers it starts (`scip-python`, `scip-typescript`,
`rust-analyzer`) are the user's own programs and may use the network as they
see fit. No other flag, command or mode starts a subprocess or has such a
carve-out. Specifically:

- no telemetry, update check, schema or grammar download, or license check;
- `--precise` and `--scip` read SCIP index files and never fetch anything;
  `--run-indexers` runs only executables already on `PATH`, never downloads,
  installs or resolves an indexer or a dependency, and prints each command
  line before running it;
- the interactive HTML output is one self-contained file with no external
  script, style, font or image reference, and loads nothing when opened;
- the tool parses source only below the analysis root, reads only project
  metadata, ignore files, the config file and the SCIP files named on the
  command line besides (ADR 0003 invariant 12), and writes only to the cache
  directory and to the output files or streams the user asks for; it works
  with a read-only checkout (an unwritable cache is a warning);
- every grammar and library it needs is compiled into the binary.

## Source

Enterprise and air-gapped users (the plan's enterprise baseline EB-5:
offline operation behind proxies and without internet); the distribution
choice in [ADR 0002](../adrs/0002-rust-cli-template.md) of a static binary
and an npm package that downloads nothing at install time.

## Verification

- **Dependency ban.** `deny.toml` bans HTTP and socket-client crates
  (for example `reqwest`, `hyper`, `ureq`, `curl`, `attohttpc`) in the
  dependency tree of the binary, enforced by `cargo-deny` in every CI run,
  and clippy's `disallowed-types` forbids `std::net` sockets in our own code.
  The no-network test below is the authoritative gate; the bans catch
  regressions early.
- **No-network test.** An integration test runs the binary on the fixture
  repositories in a network-less sandbox (`unshare -n` on Linux) for every
  command and output format, and asserts a normal result. It does not run
  `--run-indexers`; that flag is covered by the next two checks.
- **No subprocess without the flag.** A test puts fake `scip-python`,
  `scip-typescript` and `rust-analyzer` executables that record each
  invocation on `PATH`, runs every command without `--run-indexers`, and
  asserts none was started.
- **Indexer flag.** With the flag, the same test asserts that only those
  executables from `PATH` ran, that the command line printed on stderr equals
  the one that ran, and that nothing was written outside `.modfence/scip/`.
  With the fakes removed from `PATH` it asserts a warning and exit status 0,
  and that nothing was downloaded.
- **HTML check.** A test parses the generated HTML and fails on any `src`,
  `href` or `url(...)` that is not a fragment or a `data:` URI.
- **Read-only run.** A test runs the tool on a read-only copy of a fixture
  and asserts the same findings, with only a cache warning.
