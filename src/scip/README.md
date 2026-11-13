# src/scip/

Reading SCIP indexes for `--precise` (ADR 0003, "Where SCIP indexes come
from" and "How SCIP feeds the graph"). The reader turns the bytes of an
`index.scip` into plain data. It knows nothing of the graph: matching
occurrences to call sites is NC6b, the flags are NC6c and running indexers is
NC6d. It starts no process and opens no connection (NFR-0002).

- `mod.rs`: the data (`Index`, `Document`, `Occurrence`), `ReadStats` with its
  warning lines, `Error`, and the role bits (`DEFINITION`, `IMPORT`).
- `read.rs`: `read_index` (bytes), `load` (a file, size-checked before it is
  read), `check_size` and `MAX_INDEX_BYTES`. Uses the `scip` crate's
  generated types and `protobuf` to parse.
- `position.rs`: `PositionEncoding`, `RawRange` (0-based, as SCIP states it)
  and `LineIndex::span`, which converts a range to a `extract::Span` (1-based,
  code points) using the source text (ADR 0003, invariant 20); `slice` is the
  text a single-line span covers. Conversion is a bisection of one table per
  line and encoding, built on first use (none for an ASCII line), so a very
  long line costs one pass, not one per occurrence.
- `run.rs`: the one place modfence starts another program (`--run-indexers`): `plan` (the
  invocations), `project_name`, `resolve` (a `PATH` search over absolute entries), `command_line`
  (the quoted line shown before a run) and `spawn` (no shell, inherited environment).
- `path.rs`: which document paths are safe, and how the index's
  `project_root` becomes a prefix for them. Private.

Rules:

- The reader never trusts the index. A document path that is empty, absolute,
  has a `..`, `.` or empty segment, or contains a backslash or NUL is dropped
  and counted; nothing in this directory opens a path from an index.
- `project_root` is a `file://` URI. It must be the analysis root or a
  directory below it (the paths then get that directory as a prefix); an empty
  one is read as the analysis root. Anything else (a parent, a sibling, another
  scheme, a malformed escape) yields no documents and sets
  `ReadStats::foreign_root`.
- A range comes from `typed_range` when set, else from the deprecated integer
  list (three or four elements). An occurrence with a negative or backwards
  range, or an empty symbol, is dropped and counted. Enclosing ranges follow
  the same rule and are `None` when invalid.
- A document with no position encoding, or one this version does not know, is
  read as UTF-16 and counted (`unspecified_encoding`): scip-python states none
  and counts UTF-16 code units, which the real index in `tests/fixtures/scip/`
  shows on a line with non-ASCII text.
- `LineIndex` only ever sees valid UTF-8: a source file that is not valid UTF-8
  is skipped by the caller, never decoded lossily, because U+FFFD would shift
  every offset after it relative to the original bytes (ADR 0003, invariant
  23). Its index document is counted as matching no analysed file.
- `LineIndex::span` returns `None` for a line that does not exist, an offset
  past the end of its line, and an offset inside a character. The caller counts
  these as a sign that the index does not match the source (stale).
- Strings from the index that can be printed (the tool name, a parse error, the
  project root) are escaped and truncated here (`printable`); do not print an
  index's text any other way.
- Output order is the index's order; the reader sorts nothing and uses no hash
  iteration.
- A file over `MAX_INDEX_BYTES` (512 MiB) is an error, not a reason to read it.
- Tests are in `tests/scip.rs`, built from indexes written in code, and in
  `tests/scip_real.rs`, which reads the real indexes of `tests/fixtures/scip/`
  (made by the real indexers, with their versions, commands and SHA-256 in that
  directory's `README.md`); do not add `#[cfg(test)]` modules here.
