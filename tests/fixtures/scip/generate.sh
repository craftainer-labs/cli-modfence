#!/usr/bin/env bash
# Regenerates the committed SCIP indexes of this directory with the real
# indexers, and writes the evidence for README.md: the tools' versions and
# --help text, the command lines modfence ran, what modfence made of each index
# and the SHA-256 of each file. Needs scip-python, scip-typescript and
# rust-analyzer on PATH (the workflow installs them at pinned versions).
#
#   generate.sh MODFENCE OUT
#
# MODFENCE is a built modfence binary; OUT is a directory for the results.
set -euo pipefail

# The fixtures are indexed outside the repository, where rustup would fall back to
# its default toolchain; rust-analyzer is a component of the repository's one.
RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-$(rustup show active-toolchain | awk '{print $1}')}
export RUSTUP_TOOLCHAIN

modfence=$(realpath "$1")
mkdir -p "$2"
out=$(realpath "$2")
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$out/help"

{
  echo "date: $(date -u +%F)"
  echo "scip-python: $(scip-python --version 2>&1 | head -n 1)"
  echo "scip-typescript: $(scip-typescript --version 2>&1 | head -n 1)"
  echo "rust-analyzer: $(rust-analyzer --version 2>&1 | head -n 1)"
} > "$out/versions.txt"
cat "$out/versions.txt"

scip-python index --help > "$out/help/scip-python.txt" 2>&1 || true
scip-typescript index --help > "$out/help/scip-typescript.txt" 2>&1 || true
rust-analyzer scip --help > "$out/help/rust-analyzer.txt" 2>&1 || true

# Python and TypeScript go through modfence itself, so the command lines are
# the ones `scip::run::plan` builds; a flag the real indexer rejects fails here.
for lang in python typescript; do
  cp -R "$here/$lang" "$work/$lang"
  (
    cd "$work/$lang"
    "$modfence" graph --run-indexers --format json \
      > "$out/$lang.graph.json" 2> "$out/$lang.modfence.txt"
  )
  cp "$work/$lang/.modfence/scip/$lang.scip" "$out/$lang.scip"
done

# modfence does not walk Rust yet, so rust-analyzer runs with the arguments
# `plan` gives it for Rust.
cp -R "$here/rust" "$work/rust"
mv "$work/rust/Cargo.toml.in" "$work/rust/Cargo.toml"
(
  cd "$work/rust"
  printf 'rust-analyzer scip %s --output %s\n' "$work/rust" "$out/rust.scip" \
    > "$out/rust.modfence.txt"
  rust-analyzer scip "$work/rust" --output "$out/rust.scip" 2>> "$out/rust.modfence.txt"
)

for lang in python typescript rust; do
  strings -n 8 "$out/$lang.scip" > "$out/$lang.strings.txt"
done
(cd "$out" && sha256sum ./*.scip > SHA256SUMS && cat SHA256SUMS)
