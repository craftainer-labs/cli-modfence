// Validates SARIF documents against the vendored SARIF 2.1.0 schema (draft-04).
// Run by the `sarif-json-schema` hook (node language, so ajv, ajv-draft-04 and
// ajv-formats come from the hook's pinned environment). Usage: node <this> <file>...
"use strict";
const fs = require("node:fs");
const Ajv = require("ajv-draft-04");
const addFormats = require("ajv-formats");

const schemaPath = "schema/vendor/sarif-schema-2.1.0.json";
const ajv = new Ajv({ allErrors: true, strict: false });
addFormats(ajv);
const validate = ajv.compile(JSON.parse(fs.readFileSync(schemaPath, "utf8")));

let failed = false;
let checked = 0;
// The hook also runs when the schema or this script changes; those are not reports.
for (const file of process.argv.slice(2).filter((f) => f.endsWith(".sarif.json"))) {
  checked += 1;
  if (validate(JSON.parse(fs.readFileSync(file, "utf8")))) {
    console.log(`${file} valid`);
  } else {
    failed = true;
    console.error(`${file} invalid`);
    for (const error of validate.errors) {
      console.error(`  ${error.instancePath || "/"} ${error.message}`);
    }
  }
}
if (checked === 0 && process.argv.length > 2) {
  console.log("no SARIF report among the arguments");
}
process.exit(failed ? 1 : 0);
