#!/usr/bin/env node
// modfence: layer=cli
const fs = require("node:fs");
const { join } = require('path');

/**
 * Runs.
 */
// modfence: ignore[calls] reason="x"
function main(argv) {
  fs.readFileSync(join("a", "b"));
  return import("./dynamic.js");
}

// modfence: owner=js
module.exports = { main };
main(process.argv);
