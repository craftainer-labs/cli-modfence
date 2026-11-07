// Validates SARIF documents against the vendored SARIF 2.1.0 schema (draft-04).
// Run by the `sarif-json-schema` hook (node language, so ajv, ajv-draft-04 and
// ajv-formats come from the hook's pinned environment). Usage: node <this> <file>...
"use strict";
const fs = require("node:fs");
const Ajv = require("ajv-draft-04");
const addFormats = require("ajv-formats");

const path = require("node:path");

const root = path.join(__dirname, "..");
const schemaPath = path.join(root, "schema", "vendor", "sarif-schema-2.1.0.json");
const ajv = new Ajv({ allErrors: true, strict: false });
addFormats(ajv);
const validate = ajv.compile(JSON.parse(fs.readFileSync(schemaPath, "utf8")));

let failed = false;
let checked = 0;
// The hook also runs when only the schema or this script changed; then no
// report is among the arguments, so validate every golden report instead.
let files = process.argv.slice(2).filter((f) => f.endsWith(".sarif.json"));
if (files.length === 0) {
  const dir = path.join(root, "tests", "golden");
  files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".sarif.json"))
    .map((f) => path.join(dir, f));
}
for (const file of files) {
  checked += 1;
  let errors = null;
  try {
    if (!validate(JSON.parse(fs.readFileSync(file, "utf8")))) {
      errors = validate.errors.map((e) => `  ${e.instancePath || "/"} ${e.message}`);
    }
  } catch (err) {
    errors = [`  ${err.message}`];
  }
  if (errors === null) {
    console.log(`${file} valid`);
  } else {
    failed = true;
    console.error(`${file} invalid`);
    for (const line of errors) console.error(line);
  }
}
if (checked === 0) {
  console.error("no SARIF report found to validate");
  failed = true;
}
process.exit(failed ? 1 : 0);
