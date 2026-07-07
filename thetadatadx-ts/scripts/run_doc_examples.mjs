// TypeScript doc-example gate.
//
// Type-checks the fenced code examples in the published surface against
// `index.d.ts`, so a broken snippet fails the gate instead of shipping.
// Two sources are scanned:
//
//   1. `index.d.ts` JSDoc — fenced blocks live inside `/** ... */` comments,
//      so every line is prefixed with ` * `. The fence regex only matches
//      column-0 fences, so the leading ` * ` gutter is stripped first;
//      without that step the extractor matched zero blocks and the gate was
//      inert (which is how broken JSDoc shipped unnoticed).
//   2. `README.md` — the package's primary worked examples.
//
// Examples read as a narrative: a block that opens with its own `import`
// starts a self-contained unit, and any following fenced block that carries
// no import is a continuation that reuses the same `client` (and other
// bindings). Blocks are therefore grouped into units — each unit begins at an
// import-bearing block and absorbs the bare continuation fragments after it —
// and every unit is emitted as one module (imports hoisted, bodies run inside
// one async IIFE so top-level `await` is legal) and type-checked with
// `tsc --noEmit`. Bare `import ... from "thetadatadx-ts"` / "@thetadatadx/sdk"
// is re-homed onto the local `index.d.ts` declarations.

import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = resolve(here, "..");
const dtsPath = resolve(pkgRoot, "index.d.ts");
const readmePath = resolve(pkgRoot, "README.md");

// The module specifier examples import (`thetadatadx`) must resolve to the
// package's published type entry, not the raw napi `index.d.ts`. The entry
// (`package.json` `types`) is the wrapper's `streaming-session.d.ts`, which
// re-exports the whole napi surface AND layers the wrapper augmentations on
// top (e.g. `StreamView.batches(...)` returns the async-iterable
// `RecordBatchStream`, not the bare napi `RecordBatchStreamHandle`). Checking
// against `index.d.ts` alone would miss those augmentations, so a documented
// example of a wrapper-only surface could never type-check. Resolve the entry
// from `package.json` so this tracks the published `types` automatically.
const pkgTypesEntry = JSON.parse(
  readFileSync(resolve(pkgRoot, "package.json"), "utf-8")
).types;
const typesEntryPath = resolve(pkgRoot, pkgTypesEntry ?? "index.d.ts");

// Module specifier that resolves the local declarations.
const LOCAL_MODULE = typesEntryPath
  .replace(/\\/g, "/")
  .replace(/\.d\.ts$/, "");

const FENCE_RE = /```(?:ts|typescript|javascript|js)\n([\s\S]*?)\n```/g;

// Strip the leading ` * ` (JSDoc gutter) from every line so fences nested in
// `/** ... */` comments line up at column 0 for FENCE_RE.
function stripJsDocGutter(text) {
  return text
    .split("\n")
    .map((line) => line.replace(/^\s*\* ?/, ""))
    .join("\n");
}

function extractBlocks(source, { stripGutter }) {
  const haystack = stripGutter ? stripJsDocGutter(source) : source;
  const blocks = [];
  for (const match of haystack.matchAll(FENCE_RE)) {
    blocks.push(match[1].trim());
  }
  return blocks;
}

// Re-home bare package imports / requires onto the local declarations.
function rehome(line) {
  return line.replace(
    /(["'])(?:thetadatadx-ts|@thetadatadx\/sdk|thetadatadx)\1/g,
    `"${LOCAL_MODULE}"`
  );
}

const IMPORT_RE = /^\s*import\s/;

function isImportLine(line) {
  return (
    /^\s*import\s.+\sfrom\s.+$/.test(line) ||
    /^\s*import\s+["'].+["'];?\s*$/.test(line)
  );
}

const REQUIRE_RE = /\brequire\s*\(/;

// A block opens a fresh unit if it pulls in its own bindings — via an ESM
// `import` or a CommonJS `require(...)`. Blocks with neither are continuation
// fragments that reuse the open unit's context.
function blockOpensUnit(block) {
  return block.split("\n").some((l) => IMPORT_RE.test(l) || REQUIRE_RE.test(l));
}

// Group blocks into compilation units: a block with its own import opens a
// unit; bare continuation blocks attach to the open unit. A leading bare
// block (no import yet) opens its own unit.
function groupUnits(blocks) {
  const units = [];
  for (const block of blocks) {
    if (units.length === 0 || blockOpensUnit(block)) {
      units.push([block]);
    } else {
      units[units.length - 1].push(block);
    }
  }
  return units;
}

// Emit one compilable module from a unit's blocks: hoist every `import`
// (de-duplicated) to the top, then run the rest inside one async IIFE so a
// top-level `await` type-checks.
function emitUnit(blocks) {
  const imports = new Map();
  const bodies = [];
  for (const block of blocks) {
    const bodyLines = [];
    for (const raw of block.split("\n")) {
      const line = rehome(raw);
      if (isImportLine(line)) {
        imports.set(line.trim(), true);
      } else {
        bodyLines.push(line);
      }
    }
    bodies.push(bodyLines.join("\n"));
  }
  const importBlock = [...imports.keys()].join("\n");
  const bodyBlock = bodies.join("\n\n");

  // Continuation fragments reuse a `client` declared in an earlier prose
  // block that is not part of this unit. When the unit uses `client` but
  // never declares it, supply an ambient binding typed as `Client` so the
  // fragment's method calls are still checked against the real surface
  // (rather than failing on an undeclared name and checking nothing).
  const declaresClient = /\b(?:const|let|var)\s+client\b/.test(bodyBlock);
  const usesClient = /\bclient\b/.test(bodyBlock);
  const ambient =
    usesClient && !declaresClient
      ? `import type { Client as __DocClient } from "${LOCAL_MODULE}";\ndeclare const client: __DocClient;\n`
      : "";

  return `${importBlock}\n${ambient}\nasync function __doc_example__() {\n${bodyBlock}\n}\nvoid __doc_example__;\nexport {};\n`;
}

const sources = [
  { label: "index.d.ts", blocks: extractBlocks(readFileSync(dtsPath, "utf-8"), { stripGutter: true }) },
  { label: "README.md", blocks: extractBlocks(readFileSync(readmePath, "utf-8"), { stripGutter: false }) },
];

const totalBlocks = sources.reduce((n, s) => n + s.blocks.length, 0);
if (totalBlocks === 0) {
  console.error(
    "run_doc_examples: no fenced-code examples found in index.d.ts or README.md. " +
      "The extractor matched nothing — that is the exact bug this gate guards against, " +
      "so fail loudly rather than pass an empty gate."
  );
  process.exit(1);
}

const scratch = mkdtempSync(join(tmpdir(), "thetadatadx-doc-"));
try {
  // Optional peer dependencies referenced by examples (e.g. apache-arrow)
  // are not installed for the gate; declare them as `any`-typed modules so an
  // example exercising the integration still type-checks its own surface
  // usage instead of failing on the unresolved peer.
  const shimsPath = join(scratch, "peer-shims.d.ts");
  writeFileSync(
    shimsPath,
    ['declare module "apache-arrow";', ""].join("\n"),
    "utf-8"
  );

  const files = [shimsPath];
  for (const { label, blocks } of sources) {
    if (blocks.length === 0) continue;
    const slug = label.replace(/[^a-z0-9]+/gi, "_");
    groupUnits(blocks).forEach((unit, i) => {
      const filePath = join(scratch, `${slug}_unit${i}.ts`);
      writeFileSync(filePath, emitUnit(unit), "utf-8");
      files.push(filePath);
    });
  }

  // A standalone tsconfig keeps the example check off the package's own
  // `include` list and sidesteps tsc's "files on the command line ignore
  // tsconfig" diagnostic.
  const tsconfigPath = join(scratch, "tsconfig.json");
  writeFileSync(
    tsconfigPath,
    JSON.stringify(
      {
        compilerOptions: {
          noEmit: true,
          strict: true,
          skipLibCheck: true,
          target: "ES2022",
          module: "ES2022",
          moduleResolution: "node",
          esModuleInterop: true,
          ignoreDeprecations: "6.0",
          typeRoots: [resolve(pkgRoot, "node_modules", "@types").replace(/\\/g, "/")],
          types: ["node"],
        },
        files,
      },
      null,
      2
    ),
    "utf-8"
  );

  const tsc = resolve(pkgRoot, "node_modules", ".bin", "tsc");
  const result = spawnSync(tsc, ["--project", tsconfigPath], {
    cwd: pkgRoot,
    stdio: "inherit",
  });

  if (result.status !== 0) {
    console.error(
      `doc-example type-check failed across ${totalBlocks} extracted example block(s) ` +
        `from ${sources.filter((s) => s.blocks.length).map((s) => s.label).join(", ")}`
    );
    process.exit(1);
  }
  console.log(`ok: ${totalBlocks} doc example block(s) type-checked cleanly`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
