// Test runner — walks `__tests__/` for every `*.test.mjs` file and
// hands the list to `node --test` as explicit argv.
//
// Why a script: `node --test "__tests__/*.test.mjs"` (Node-internal
// glob) is Node-22+ only and we still ship `engines.node >= 20`;
// `node --test __tests__/*.test.mjs` (shell glob) breaks on Windows
// PowerShell, where the literal `*.test.mjs` reaches node unchanged.
// An explicit-argv runner sidesteps both — works on every supported
// Node version and every supported shell.
//
// Walks one level only (the test directory is flat by convention);
// emits one ENOENT-style exit code if no tests are found so a
// silent-skip regression is impossible.

import { readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const testsDir = resolve(here, "..", "__tests__");

let entries;
try {
  entries = readdirSync(testsDir);
} catch (err) {
  console.error(`run_tests: cannot read ${testsDir}: ${err.message}`);
  process.exit(1);
}

const tests = entries
  .filter((name) => name.endsWith(".test.mjs"))
  .map((name) => join(testsDir, name))
  .filter((path) => statSync(path).isFile())
  .sort();

if (tests.length === 0) {
  console.error(`run_tests: no *.test.mjs files in ${testsDir}`);
  process.exit(1);
}

console.error(`run_tests: invoking node --test on ${tests.length} files`);

const result = spawnSync(process.execPath, ["--test", ...tests], {
  stdio: "inherit",
});

process.exit(result.status ?? 1);
