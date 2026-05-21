// U7 closure: append `export const Contract: typeof ContractRef;` to
// the napi-emitted `index.d.ts` so the documented `Contract.stock(...)`
// builder name resolves at type-check time. The matching runtime
// alias is set in `index.js` (`module.exports.Contract =
// nativeBinding.ContractRef`).
//
// napi-rs ships the class as `ContractRef` (the bare `Contract`
// name collides with the FPSS event payload type's own `Contract`
// field). This post-build pass keeps the public surface tied to
// the documented name without forcing every user to write
// `import { ContractRef as Contract } from "thetadatadx"`.
//
// Run automatically via `npm run build` (see `scripts.build` in
// package.json). Idempotent: re-running on a tree that already
// carries the alias is a no-op.

import { readFileSync, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dtsPath = resolve(here, "..", "index.d.ts");

const ALIAS_LINE = "export const Contract: typeof ContractRef";
const text = readFileSync(dtsPath, "utf-8");
if (text.includes(ALIAS_LINE)) {
  console.log(`postbuild_alias_contract: alias already present in ${dtsPath}`);
  process.exit(0);
}
const trailer =
  "\n" +
  "// ─────────────────────────────────────────────────────────────\n" +
  "// U7 closure: `Contract` is the documented public name for the\n" +
  "// fluent contract builder. napi-rs emits the class as\n" +
  "// `ContractRef` to avoid colliding with the FPSS event\n" +
  "// payload field; this alias keeps the documented name live.\n" +
  "// ─────────────────────────────────────────────────────────────\n" +
  `${ALIAS_LINE};\n`;
writeFileSync(dtsPath, text + trailer, "utf-8");
console.log(`postbuild_alias_contract: appended Contract alias to ${dtsPath}`);
