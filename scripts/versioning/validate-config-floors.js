#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { readFileSync } = require("fs");
const { join } = require("path");
const Ajv2020 = require("ajv/dist/2020");

const repoRoot = join(__dirname, "..", "..");
const catalogRoot = join(repoRoot, "sdk", "node", "src", "config-floors");
const schema = JSON.parse(readFileSync(join(catalogRoot, "schema.json"), "utf8"));
const catalog = JSON.parse(readFileSync(join(catalogRoot, "catalog.json"), "utf8"));
const validate = new Ajv2020({ allErrors: true, strict: false }).compile(schema);

if (!validate(catalog)) {
  console.error("Config floor catalog validation FAILED:");
  for (const error of validate.errors ?? []) {
    console.error(`  - ${error.instancePath || "/"} ${error.message}`);
  }
  process.exit(1);
}

console.log("Config floor catalog validation passed.");
