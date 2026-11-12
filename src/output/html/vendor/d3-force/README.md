# Vendored d3-force

The force simulation of the interactive HTML output (`src/output/html.rs`,
NC5c; decision of 2026-11-04). The four files are the unmodified
`dist/*.min.js` UMD builds from the npm tarballs below, each a classic script
that adds its exports to the global `d3` when loaded in a page. The page loads
them in the order of the table (dependencies first) and inlines them, so it
makes no network request.

Licenses: all four packages are ISC, Copyright Mike Bostock; the one text
(identical in every tarball, checked by md5) is in `LICENSE`. ISC is allowed by
`deny.toml`.

| Package | Version | License | Tarball | npm `dist.integrity` (sha512, verified against the downloaded tarball) |
|---|---|---|---|---|
| d3-dispatch | 3.0.2 | ISC | <https://registry.npmjs.org/d3-dispatch/-/d3-dispatch-3.0.2.tgz> | `sha512-sAzH4Y4nsLo8eSqw5jKiZt+LU2wZmL/qS+OkxQGjhwazB/D7TY34R/QK1eshfGr7cZEKymKZ/Haem7m+lu98Sg==` |
| d3-quadtree | 3.0.2 | ISC | <https://registry.npmjs.org/d3-quadtree/-/d3-quadtree-3.0.2.tgz> | `sha512-9Lu9mqJKRbIk0d6mKDjpvPURaPdwHVK5CYbcdd0/4vBgz4ldz/7TIOse3kNERsByGNfld58KEco507AztJWyTQ==` |
| d3-timer | 3.0.2 | ISC | <https://registry.npmjs.org/d3-timer/-/d3-timer-3.0.2.tgz> | `sha512-1Il1aliroVSErrv5aulRBxAY+7+DnaoLIWGd1C6SsBTWDeobWiP/V69vwb8vT6abR1kKwaNe1196pmaQ1fK90A==` |
| d3-force | 3.0.1 | ISC | <https://registry.npmjs.org/d3-force/-/d3-force-3.0.1.tgz> | `sha512-kAaaHLFebBYwgi5cYBXAI8vKrQFMCD2MEswj2QuPKKbdU1wYLc8WXtS7O8F/b6POCIe3gJjY6hkDstHi1yMETQ==` |

Source repository: <https://github.com/d3/d3-force> (and the sibling `d3/`
repositories of the other three). The tarball sha256 sums are
`8e6c6ec6c684d1246eed13fc7874c32d830f4bda092877677c4718c86d511238`
(d3-force), `a7761dc785dd08d6b3c57c92bb245b581cfe36e6a581687142a2b6f3df104b23`
(d3-dispatch), `92db92a8f02bd86e25185833f453bb8f179e8a2ff62321461d47627df76ac9c1`
(d3-quadtree) and `94b4d053d0061bfb7a3461234a5a14d5b3200e6dbb28d148559ed91ba787661c`
(d3-timer). `SHA256SUMS` lists the sha256 of each committed file
(`sha256sum -c SHA256SUMS` in this directory).

## Updating

1. Pick exact versions from the registry (`https://registry.npmjs.org/<name>/latest`).
2. Download each tarball into an empty directory, compare
   `openssl dgst -sha512 -binary <tgz> | base64 -w0` with `dist.integrity`,
   and extract with `tar` only; run none of it.
3. Copy `dist/<name>.min.js` unchanged, refresh `LICENSE`, this table and
   `SHA256SUMS`.
4. The page's inline script hash changes with these files: update the
   expected hash in `tests/html.rs` (the test says how).
