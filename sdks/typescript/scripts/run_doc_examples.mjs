// Gate 3 (issue #546) - TypeScript side: doctest gate.
//
// Extracts tsdoc `@example` / fenced-code blocks from `index.d.ts`
// and runs each one through `tsx` so the example actually
// compiles and executes against the published surface.
//
// Today `index.d.ts` carries zero `@example` blocks (the napi-rs
// emitter doesn't pass them through from the Rust source); the
// script exits 0 with a "no examples found" note so the wiring is
// in place for the moment we start documenting on the TS side.

import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const dtsPath = resolve(here, "..", "index.d.ts");

const text = readFileSync(dtsPath, "utf-8");

const FENCE_RE = /```(?:ts|typescript|javascript|js)\n([\s\S]*?)\n```/g;
const examples = [];
for (const match of text.matchAll(FENCE_RE)) {
  examples.push(match[1].trim());
}

if (examples.length === 0) {
  console.log(
    "run_doc_examples: no fenced-code examples found in index.d.ts. " +
      "Gate is wired but inactive - adding `@example` tsdoc blocks " +
      "to the napi-rs source will start exercising them here."
  );
  process.exit(0);
}

const scratch = mkdtempSync(join(tmpdir(), "tdx-doc-"));
try {
  let failed = 0;
  for (let i = 0; i < examples.length; i++) {
    const body = examples[i];
    const filePath = join(scratch, `example-${i}.mjs`);
    const prelude = "import * as thetadatadx from \"thetadatadx\";\n";
    writeFileSync(filePath, prelude + body, "utf-8");
    const result = spawnSync("npx", ["tsx", filePath], {
      cwd: resolve(here, ".."),
      stdio: "inherit",
    });
    if (result.status !== 0) {
      console.error(`example #${i} (from index.d.ts) failed`);
      failed += 1;
    }
  }
  if (failed > 0) {
    console.error(`${failed} of ${examples.length} doc example(s) failed`);
    process.exit(1);
  }
  console.log(`ok: ${examples.length} doc example(s) executed cleanly`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
