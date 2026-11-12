# tests/fixtures/scip/

Small real SCIP indexes, made by the real indexers from the three fixture
projects here, so the tests read what the tools write rather than what a test
helper builds (NC6e). The reader and the graph upgrade are otherwise tested with
indexes written in code (`tests/scip.rs`, `tests/graph_precise.rs`).

- `python/`, `typescript/`, `rust/`: the fixture projects. Each has a class
  with a method, a function imported under another name (`run as go`, so the
  call to `go()` matches no symbol by name), a call through an instance, and a
  line with non-ASCII text (`é𝄞`, 2 UTF-8 bytes and 1 UTF-16 unit, then 4 bytes
  and 2 units) before the aliased call, so a wrong position encoding misses it.
  `rust/Cargo.toml.in` is the crate's manifest under another name, so Cargo does
  not take the fixture for a member of this repository's workspace.
- `indexes/`: the committed indexes (below).
- `evidence/`: what the generator printed: the tools' versions, their `--help`
  text and the SHA-256 of each index.
- `indexers/`: `package.json` and `package-lock.json` pin the two npm indexers.
- `generate.sh`: makes the indexes; `../../../.github/workflows/scip-fixtures.yml`
  runs it.

## The committed indexes

Generated on 2026-11-12 by workflow run 43331746324 (`SCIP fixtures`, artifact
`scip-fixtures`) on `ubuntu-24.04`, from the sources in this directory.

| File | Tool | Version | Command (as modfence runs it) | SHA-256 |
| --- | --- | --- | --- | --- |
| `indexes/python.scip` | `scip-python` | 0.6.8 | `scip-python index . --project-name=python --project-version=0.0.0 --output <root>/.modfence/scip/python.scip`, in `python/` | `644f5c3eb4cb8500223b2571c5a616670bea53ac8904007f6a30794fa21a21dd` |
| `indexes/typescript.scip` | `scip-typescript` | 0.6.2 | `scip-typescript index --infer-tsconfig --output <root>/.modfence/scip/typescript.scip`, in `typescript/` | `48c8638ea78eeefed31f46b4296e9f729c7ae396e267f602e82a7731464bbdc3` |
| `indexes/rust.scip` | `rust-analyzer` | 1.99.1 (58cf8f9 2026-10-13) | `rust-analyzer scip <root> --output <out>/rust.scip`, in a copy of `rust/` with `Cargo.toml.in` renamed | `9a8207a62d91efd71da9a8a10e25bca14f27f8ddb24bb5c158762632736db85b` |

Python and TypeScript were run by `modfence graph --run-indexers`, so those
command lines are the ones `scip::run::plan` builds. modfence does not read Rust
yet, so rust-analyzer was started by `generate.sh` with the arguments `plan`
gives it for Rust. `<root>` is a temporary directory of the job; it is also the
`project_root` inside each index (`file:///tmp/tmp.R2tjPUiIc3/<language>`), which
the tests rewrite in memory to the directory they copy the fixture to. The files
here are never changed.

What the indexes record (see `tests/scip_real.rs`):

- `scip-python` states no position encoding. Its columns count UTF-16 code
  units: on the non-ASCII line the reference to `go` starts at column 31 where a
  code-point count gives 30. The reader therefore takes an unspecified encoding
  as UTF-16 (`src/scip/README.md`).
- `scip-typescript` states UTF-16, `rust-analyzer` UTF-8.

## Flags checked against the real tools

Each argument `scip::run::plan` passes, with where it appears in the tool's own
`--help` (`evidence/help/`) or in a real run:

| Indexer | Argument | Evidence |
| --- | --- | --- |
| `scip-python` | `index` | `Usage: scip-python index [options]` |
| `scip-python` | `--project-name=<name>` | `--project-name <name>` |
| `scip-python` | `--project-version=0.0.0` | `--project-version <version>`; without it the run crashes outside a git checkout (`Cannot read properties of undefined (reading 'indexOf')`, first run of the workflow), because the version defaults to the git revision |
| `scip-python` | `--output <path>` | `--output <path>`: "If this path is relative, it is interpreted relative to the value for --cwd"; the index was written there |
| `scip-python` | `.` | not a documented argument; accepted and ignored, as the working directory is the project (`--cwd` defaults to it) |
| `scip-typescript` | `index` | `Usage: scip-typescript index [options] [projects...]` |
| `scip-typescript` | `--infer-tsconfig` | `--infer-tsconfig` ("whether to infer the tsconfig.json file, if it's missing") |
| `scip-typescript` | `--output <path>` | `--output <path>` ("path to the output file") |
| `rust-analyzer` | `scip <path>` | `rust-analyzer scip` with `<path>` under ARGS |
| `rust-analyzer` | `--output <path>` | `--output <path>` ("The output path where the SCIP file will be written to") |

The arguments are separate words except `--project-name=`/`--project-version=`,
which keep a repository-controlled name from being read as an option.

## Regenerating

Run the `SCIP fixtures` workflow by hand (Actions, then "Run workflow"); it needs
the network and nothing else, and pushes nothing. Download the `scip-fixtures`
artifact, copy the three `*.scip` files to `indexes/`, `versions.txt`,
`SHA256SUMS` and `help/` to `evidence/`, and update the date, run id and hashes
above. A new tool version is a change to `indexers/package.json` (and its
lockfile, with `npm install --package-lock-only` there) or to the toolchain pin,
and to the versions above.
