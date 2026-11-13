# modfence

> [!NOTE]
> This repository is vibe coded with [Claude](https://claude.com/product/claude-code).

A cross-language module-boundary checker for Python, TypeScript/JavaScript
and Rust: one rule set in `modfence.toml`, one local CLI, violations reported
as text, GitHub annotations or SARIF for code scanning. It reads source with
tree-sitter, builds a graph of packages, modules, symbols and the imports and
calls between them, and judges every import or call site against your rules.
It is an instance of
[`template-rust-cli`](https://github.com/craftainer/template-rust-cli); see
[`docs/TEMPLATE.md`](docs/TEMPLATE.md) for the conventions shared by every
instance.

modfence works fully offline: no command opens a network connection (see
"Offline guarantee").

## Install

Pick one channel. Every channel installs the same `modfence` program.

| Channel | Command | Notes |
| --- | --- | --- |
| GitHub release | download `modfence-<target>.tar.gz` (`.zip` on Windows) from the [releases](https://github.com/craftainer-labs/cli-modfence/releases) | static binaries for Linux (musl), macOS and Windows on x86_64 and aarch64, each with a `.sha256` file; verify with `gh attestation verify <file> --repo craftainer-labs/cli-modfence` |
| crates.io | `cargo install modfence --locked` | builds from source; needs Rust 1.99 or newer |
| PyPI | `pipx install modfence` or `uv tool install modfence` | a wheel holding the binary, no Python code runs |
| npm | `npm install --global modfence` | a JS shim plus a per-platform package (`@craftainer-labs/modfence-<os>-<cpu>`); nothing is downloaded at install time |
| pre-commit / prek | see "Use it as a pre-commit hook" | builds the tagged source |

Which channels are live depends on the release: a channel is published only
once the maintainers switch it on (see "Releasing the binaries"). Homebrew
and the container image are wired in the release workflow but off by default.
`modfence --version` prints the installed version.

## Usage

```sh
modfence check                 # report violations; exit 1 if any is an error
modfence check --format sarif --output modfence.sarif
modfence check --format github # workflow annotations for a CI log
modfence graph --format html --output graph.html
modfence trace adr=0028 layer=domain,infra
```

- `check` judges every import and call site against the rules and prints the
  violations. `--format` is `text` (default), `sarif` (SARIF 2.1.0) or
  `github`; `--fail-on error|warning|note` sets the lowest severity that
  fails the run (default `error`).
- `graph` prints the dependency graph. `--format` is `text`, `json` (the
  schema in `schema/modfence-graph.schema.json`), `dot`, `mermaid`, `graphml`
  or `html` (one self-contained, offline page with an accessible node
  table). `--level package|module|symbol` and `--group-by <tag key>` shape
  the diagrams.
- `trace` prints the nodes whose tags match `key=value` selectors and their
  edges, as text or JSON.
- Call edges are not checked unless you pass `--precise` (read the SCIP
  indexes in `.modfence/scip/`), `--scip <file>` (read the named indexes) or
  `--run-indexers` (run the `scip-python`, `scip-typescript` and
  `rust-analyzer` found on `PATH` first). `check` prints a notice on stderr
  when call edges were not checked. `--run-indexers` starts programs from your
  `PATH`, which run code of the analysed repository: do not use it on code you
  do not trust.
- Notices and warnings go to standard error, never into the report. Add
  `.modfence/` to `.gitignore`.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | nothing at or above `--fail-on` |
| 1 | at least one violation at or above `--fail-on` |
| 2 | a usage or configuration error (including an unknown key in `modfence.toml`) |
| 74 | a file or stream could not be read or written (for example a missing `--config` file) |

The code does not depend on `--format`. The details are in
[ADR 0004](docs/adrs/0004-config-annotations-and-rules.md) and
[`src/cli/README.md`](src/cli/README.md).

## Configuration

`modfence.toml` sits at the repository root; modfence finds it by walking up
from the working directory (`--config <path>` overrides, and the analysis
root is the file's directory). Without one, the defaults apply and nothing
is forbidden. A complete example is
[`schema/examples/modfence.example.toml`](schema/examples/modfence.example.toml)
and the schema, for editors and CI, is
[`schema/modfence-config.schema.json`](schema/modfence-config.schema.json).

```toml
version = 1
exclude = ["node_modules/**", "target/**"]

# Highest layer first; a layer may use itself and the layers below it.
layers = ["app", "infra", "domain"]

[[node]]
match = "src/domain/**"
tags = { layer = "domain" }

[[node]]
package = "requests"
tags = { boundary = "network" }

[[rule]]
id = "domain-no-network"
from = { layer = "domain" }
to = { boundary = "network" }
effect = "forbid"
message = "The domain layer must not talk to the network."
```

Tags can also come from the source, in a comment on its own line, in all
three languages: `# modfence: layer=domain owner=team-a` (Python) or
`// modfence: layer=domain` (TypeScript, Rust). A finding is suppressed with
`modfence: ignore[rule-id] reason="..."` on its line or the line above.
[ADR 0004](docs/adrs/0004-config-annotations-and-rules.md) is the reference
for rule order, tag precedence, `default = "deny"` and the reports.

## Use it as a pre-commit hook

[`.pre-commit-hooks.yaml`](.pre-commit-hooks.yaml) defines the hook `modfence`
(`modfence check`). It checks the whole repository, because a boundary is
broken by an import in one file of another, and not the staged files. In your
`.pre-commit-config.yaml` (works with [pre-commit](https://pre-commit.com) and
[prek](https://github.com/j178/prek)):

```yaml
repos:
  - repo: https://github.com/craftainer-labs/cli-modfence
    rev: vX.Y.Z # a release tag
    hooks:
      - id: modfence
        # args: [--fail-on, warning]
```

The hook is `language: rust`: the tagged source is built once with `cargo
install` into the hook's private environment, so it needs a network (to
fetch crates) at install time only. The checker never does.

## Use it in GitHub Actions

Run `check` with `--format sarif` and upload the result to code scanning, so
violations appear as alerts on the pull request. Replace `X.Y.Z` with a
release. Every action is pinned to a commit SHA, the token is read-only
except for `security-events`, and the version reaches the shell through
`env`:

```yaml
name: modfence
on:
  pull_request:
  push:
    branches: [main]

permissions: {}

jobs:
  modfence:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write # upload-sarif
    env:
      MODFENCE_VERSION: X.Y.Z
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Install modfence
        run: pipx install "modfence==${MODFENCE_VERSION}"
      - name: Check module boundaries
        run: modfence check --format sarif --output modfence.sarif
      - name: Upload to code scanning
        if: ${{ !cancelled() && hashFiles('modfence.sarif') != '' }}
        uses: github/codeql-action/upload-sarif@33d7b85da1a04171e211befe2c739303b9a4c066 # v4.39.3
        with:
          sarif_file: modfence.sarif
          category: modfence
```

`check` exits 1 when a violation is at or above `--fail-on`, which fails the
job after the SARIF file is written; the upload step still runs. For a log
only, with no code scanning, use `modfence check --format github` instead.
GitHub keeps 5,000 results per upload; modfence warns when a report has more.
Code scanning on private repositories needs GitHub Code Security.

## Offline guarantee

[NFR-0002](docs/nfrs/NFR-0002-offline-operation.md): no command of modfence
opens a network connection, in any mode. There is no telemetry, update check
or download; every grammar is compiled into the binary; `--precise` and
`--scip` only read index files; the HTML output is one file that loads
nothing. Only the opt-in `--run-indexers` starts other programs (the
indexers on your `PATH`), and those may use the network as they see fit.
The npm package downloads nothing at install time. `deny.toml` bans HTTP and
socket crates, and `tests/offline.rs` runs every command in a network-less
sandbox.

## Development

Open the repository in the devcontainer, then run
`prek run --all-files --hook-stage manual`; everything should pass. The
conventions are in [`docs/TEMPLATE.md`](docs/TEMPLATE.md); the pages of
`--format html` are checked for accessibility (axe, in Chromium) by the
`A11y` workflow, see [`tests/a11y/README.md`](tests/a11y/README.md). The
names of the CLI live in `Cargo.toml` (crate, binary, description,
keywords), `pyproject.toml` (PyPI), `packaging.toml` (npm package and scope)
and `.pre-commit-hooks.yaml` (hook); the workflows and `scripts/binaries/`
read them from there.

## Publishing to crates.io

`make publish-packages` is a no-op (with a `::warning::` in the log) until you opt in, per repository, by setting the
repository variable `CARGO_PUBLISH` to any non-empty value (Settings,
Secrets and variables, Actions, Variables). The version lives only in the
git tag `release.yml` computes; `Cargo.toml` stays at `0.0.0` and
`make build` stamps `RELEASE_VERSION` into the build without committing it.

crates.io has no "pending publisher": a trusted publisher can only be added
to a crate that already exists. So the first release of each crate uses a
short-lived bootstrap token, and trusted publishing takes over after it:

1. Create a crates.io API token scoped to the new crate name
   (`publish-new` only for the first release) with a short expiry, and store
   it as the repository secret `CARGO_REGISTRY_TOKEN`.
2. Set `CARGO_PUBLISH=1` and run the release workflow. `make publish-packages` sees
   the token and publishes with it.
3. On crates.io, open the crate's Settings, Trusted Publishing, and add a
   GitHub publisher: the owner, the repository, the workflow `release.yml`,
   and no environment.
4. Delete the `CARGO_REGISTRY_TOKEN` secret and revoke the token. From the
   next release, `make publish-packages` finds no token and exchanges the job's
   GitHub OIDC identity for a 30-minute crates.io token, publishes with it
   and revokes it.

## Releasing the binaries

The `Release` workflow (above) does not build the binaries. Right after it
succeeds, dispatch the second workflow with the same tag, and watch it:

```sh
gh workflow run release-binaries.yml -f tag=vX.Y.Z
gh run watch
```

It builds the six targets on native runners, attests everything
([SLSA Build L3](https://slsa.dev/spec/v1.0/levels), verify with
`gh attestation verify <file> --repo craftainer-labs/cli-modfence`), attaches the archives,
their checksums and the Homebrew formula to the GitHub release, and then
publishes to whichever of these you switched on with a repository variable:

| Variable | Publishes | Needs |
| --- | --- | --- |
| `CLI_PYPI_PUBLISH` = `pypi` or `testpypi` | wheels and sdist, with PEP 740 attestations | a PyPI trusted publisher (a pending one works for the first release) naming workflow `release-binaries.yml` and environment `release` |
| `CLI_NPM_PUBLISH` = any value | the six `<scope>/<binary>-<os>-<cpu>` packages, then the root package | an npm trusted publisher per package naming `release-binaries.yml` and `release`; the first publish of all seven is staged with the stage-only secret `NPM_TOKEN` and approved by hand, as for the family's npm packages |
| `HOMEBREW_TAP` = `<owner>/homebrew-<tap>` | commits `Formula/<binary>.rb` to the tap | the secret `HOMEBREW_TAP_TOKEN` (write access to the tap) |
| `CLI_IMAGE_PUBLISH` = any value | `ghcr.io/<owner>/<repo>:<version>` (and `latest` for a stable release), multi-arch | nothing beyond `GITHUB_TOKEN` |

Create the GitHub environment `release` first (the PyPI, npm and Homebrew
jobs run in it, so you can add required reviewers there). The workflow only
runs from the default branch, for a published release whose tag is on it, and
can be re-run for the same tag. Details, and why it is a manual second step,
are in `docs/TEMPLATE.md`, "Rust CLI template notes".

## License

Licensed under the [GNU General Public License v3.0](LICENSE).
