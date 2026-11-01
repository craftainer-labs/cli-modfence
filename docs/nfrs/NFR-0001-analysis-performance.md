# NFR-0001. Analysis finishes within a fixed time and memory budget on a CI runner

## Status

Proposed

## Attribute

Performance (time, memory, size).

## Description

On a 4-vCPU Linux runner (the GitHub-hosted `ubuntu-latest` size for public
repositories), analysing a repository of mixed Python, TypeScript and Rust
source (`modfence check`, rules and SARIF output included) meets these
budgets. "LOC" counts non-blank source lines of the supported languages.

| Measure | 100k LOC (about 1,000 files) | 1M LOC (about 10,000 files) |
| --- | --- | --- |
| Cold run, no cache | <= 1 s wall | <= 10 s wall |
| Warm run, nothing changed | <= 0.5 s wall | <= 3 s wall |
| Warm run, 1% of files changed | <= 0.6 s wall | <= 3.5 s wall |
| Peak resident memory | <= 200 MiB | <= 1 GiB |

Also: `modfence --version` and a run on an empty directory finish in <= 50 ms,
and the release binary for each target is <= 30 MiB.

These figures are starting budgets, not measurements. They are an estimate
from the design (parallel per-file tree-sitter parsing on 4 cores, cheap
queries, a graph of tens of thousands of nodes, file I/O dominating) with a
large margin, because no code exists yet. They are re-baselined once, at NC4
(the first item that can measure the whole pipeline), on the reference
corpora below. After that a budget changes only through a superseding NFR.

## Source

Users who run the tool in pre-commit and in CI: a pre-commit hook must feel
instant on a warm cache and a CI step must not become the slowest job.
Design: [ADR 0003](../adrs/0003-graph-model.md) (per-file content-hash
cache, parallel extraction). Plan decision NR3 (8) asks that the musl
allocator be measured against this NFR.

## Verification

- **Reference corpora.** Fixture generators (not committed output) build
  repositories of 100k and 1M LOC by replicating pinned, permissively
  licensed sample projects of the three languages. NC4 owns the generator.
- **Benchmarks.** NC4 adds a benchmark per row of the table, run in CI on the
  4-vCPU runner and compared with the template's benchmark job. A row more
  than 20% over budget fails the job; the numbers are reported on every PR.
  Peak memory comes from `/usr/bin/time -v` on the release binary.
- **Allocator (owned by NC4).** The static musl binary and a glibc build run
  the cold 1M-LOC case. If the musl build misses a budget, or is more than
  20% slower than the glibc build, `mimalloc` is added as the global
  allocator for the musl targets and the comparison repeated. The result and
  the decision are recorded in this NFR when the status moves to Implemented.
- **Size.** Checked per target at release in `release-binaries.yml`'s
  output; a binary over 30 MiB fails the review of the dependency change that
  caused it.
