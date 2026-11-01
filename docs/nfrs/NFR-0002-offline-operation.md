# NFR-0002. The tool works fully offline and makes no network connection

## Status

Proposed

## Attribute

Security and portability (air-gapped and sandboxed use).

## Description

No command of `modfence` opens a network connection, in any mode. If the
opt-in `--run-indexers` exists (ADR 0003 decision 1 is open), the indexer
processes the user asks for may use the network; the guarantee then covers
`modfence`'s own process only, and the no-network test does not run that
flag. Without that flag there is no exception.

- no telemetry, update check, schema or grammar download, or license check;
- `--precise` reads SCIP index files given on the command line and never
  fetches indexers or dependencies; running an indexer is allowed only
  behind an explicit flag the user passes ([ADR 0003](../adrs/0003-graph-model.md),
  decision 1), and the tool itself still makes no connection;
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
  command and output format, and asserts a normal result.
- **HTML check.** A test parses the generated HTML and fails on any `src`,
  `href` or `url(...)` that is not a fragment or a `data:` URI.
- **Read-only run.** A test runs the tool on a read-only copy of a fixture
  and asserts the same findings, with only a cache warning.
