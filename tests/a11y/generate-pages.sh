#!/usr/bin/env bash
# Writes the HTML pages the a11y tests check: modfence's own output for the
# test fixtures, at the default level, at --level symbol, with --group-by, and
# for one trace. Usage: generate-pages.sh <modfence binary> <output directory>
# (run from the repository root).
set -euo pipefail

bin="$1"
out="$2"
config="tests/fixtures/cli/app/modfence.toml"

mkdir -p "$out"
"$bin" graph --config "$config" --format html --output "$out/graph.html"
"$bin" graph --config "$config" --format html --level symbol --output "$out/graph-symbol.html"
"$bin" graph --config "$config" --format html --group-by layer --output "$out/graph-grouped.html"
"$bin" trace layer=data --config "$config" --format html --output "$out/trace.html"
ls -l "$out"
