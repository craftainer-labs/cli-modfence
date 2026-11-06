# tests/a11y/

Browser checks of the page `modfence graph|trace --format html` writes. They
are not Rust tests and not part of the crate (`Cargo.toml`'s `include` ships
only `src/`); the `A11y` workflow (`../../.github/workflows/a11y.yml`) runs
them on every pull request and push to `main`, and nothing else does.

- `generate-pages.sh <binary> <directory>` writes four pages from
  `../fixtures/cli/app`: `graph` (the default level), `graph-symbol`
  (`--level symbol`), `graph-grouped` (`--group-by layer`) and `trace`
  (`layer=data`).
- `html.spec.js` opens each page in Chromium, reading the directory from
  `A11Y_PAGES`, and checks:
  - axe with the tags `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa` and
    `wcag22aa`: zero violations, in light, dark, reduced-motion and
    dark-plus-reduced-motion, with a node selected, and on the page with its
    script removed (axe cannot run in a context with JavaScript off);
  - with JavaScript on, the diagram and controls are visible and the zoom,
    pan and reset buttons and the tables' select buttons work from the
    keyboard;
  - with JavaScript off, the tables are shown and the diagram, controls and
    legend are hidden;
  - the page loads with every request but the file blocked, with no console
    error, page error or `securitypolicyviolation`.
- `package.json` and `package-lock.json` pin exact versions of
  `@playwright/test`, `@axe-core/playwright` and `playwright-core` (pinned
  too, because axe's peer range would otherwise pull the newest). The browser
  build follows from the lockfile, through `npx playwright install`.

## Do

- Run it the way CI does, from the repository root: build with
  `cargo build --release --locked`, run
  `bash tests/a11y/generate-pages.sh target/release/modfence target/a11y-pages`,
  then in this directory `npm ci`, `npx playwright install --with-deps
  chromium` and `A11Y_PAGES=$PWD/../../target/a11y-pages npx playwright test`.
- Fix a finding in `src/output/html/` and update the CSP hashes in
  `../html.rs` (its comment says how), not here.
- Pick dependency versions that are at least two weeks old, and regenerate
  the lockfile with `npm install --package-lock-only`.

## Don't

- Add a rule exclusion or a disabled axe check to get green.
- Let a test load anything but the generated file: the offline check blocks
  every other request.
