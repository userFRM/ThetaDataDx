// U7 closure: append the `Contract` alias to BOTH the napi-emitted
// `index.d.ts` (type alias) AND `index.js` (runtime alias). Without
// both, type-check or runtime resolves to undefined and the
// documented `Contract.stock(...)` API breaks.
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
const jsPath = resolve(here, "..", "index.js");

const DTS_ALIAS_LINE = "export const Contract: typeof ContractRef";
const JS_ALIAS_LINE = "module.exports.Contract = nativeBinding.ContractRef";

// .d.ts side — append at end of file
const dtsText = readFileSync(dtsPath, "utf-8");
if (dtsText.includes(DTS_ALIAS_LINE)) {
  console.log(`postbuild_alias_contract: dts alias already present in ${dtsPath}`);
} else {
  const dtsTrailer =
    "\n" +
    "// ─────────────────────────────────────────────────────────────\n" +
    "// U7 closure: `Contract` is the documented public name for the\n" +
    "// fluent contract builder. napi-rs emits the class as\n" +
    "// `ContractRef` to avoid colliding with the FPSS event\n" +
    "// payload field; this alias keeps the documented name live.\n" +
    "// ─────────────────────────────────────────────────────────────\n" +
    `${DTS_ALIAS_LINE};\n`;
  writeFileSync(dtsPath, dtsText + dtsTrailer, "utf-8");
  console.log(`postbuild_alias_contract: appended dts alias to ${dtsPath}`);
}

// index.js side — inject right after the existing ContractRef export.
// napi-rs regenerates index.js every build and strips any line not in
// its template, so we re-inject here.
const jsText = readFileSync(jsPath, "utf-8");
if (jsText.includes(JS_ALIAS_LINE)) {
  console.log(`postbuild_alias_contract: js alias already present in ${jsPath}`);
} else {
  const marker = "module.exports.ContractRef = nativeBinding.ContractRef";
  const idx = jsText.indexOf(marker);
  if (idx === -1) {
    console.error(
      `postbuild_alias_contract: could not find ContractRef export in ${jsPath}; ` +
        "napi-rs output shape changed — update this script."
    );
    process.exit(1);
  }
  const insertAt = jsText.indexOf("\n", idx) + 1;
  const jsAlias =
    "// U7 closure: alias the documented public name `Contract` to the\n" +
    "// napi-emitted `ContractRef` constructor. Without this, `require\n" +
    "// ('thetadatadx').Contract.stock(...)` is undefined at runtime.\n" +
    `${JS_ALIAS_LINE};\n`;
  const out = jsText.slice(0, insertAt) + jsAlias + jsText.slice(insertAt);
  writeFileSync(jsPath, out, "utf-8");
  console.log(`postbuild_alias_contract: injected js alias into ${jsPath}`);
}

// ─── @deprecated propagation for setDecoderThreads / decoderThreads ───
//
// Mirrors the Rust rustdoc + Python pyi + C++ Doxygen deprecation on
// the legacy decoder_threads alias. napi-rs forwards `///` doc-text
// into JSDoc but a future napi-rs version could reformat tags away.
// This pass guarantees the `@deprecated` tag survives every build.
{
  const dtsTextAfter = readFileSync(dtsPath, "utf-8");
  // Setter block (multi-line JSDoc) — keep narrow markers so a doc
  // rewrite upstream still matches.
  const setterMarker = "  setDecoderThreads(n: number): void";
  const setterIdx = dtsTextAfter.indexOf(setterMarker);
  // Getter block — single-line JSDoc.
  const getterMarker = "  get decoderThreads(): number";
  const getterIdx = dtsTextAfter.indexOf(getterMarker);
  if (setterIdx === -1 || getterIdx === -1) {
    console.error(
      `postbuild_alias_contract: setDecoderThreads / decoderThreads markers not found in ${dtsPath}`
    );
    process.exit(1);
  }
  // Walk backwards from each marker to find the opening `/**` of its
  // JSDoc block, then ensure `@deprecated` is inside that block.
  let modified = dtsTextAfter;
  const injectDeprecated = (text, markerIdx, tag) => {
    const blockEnd = text.lastIndexOf("*/", markerIdx);
    const blockStart = text.lastIndexOf("/**", blockEnd);
    if (blockStart === -1 || blockEnd === -1) {
      // No JSDoc block (napi may emit single-line `/** ... */`). Insert
      // a fresh block immediately above the marker.
      const lineStart = text.lastIndexOf("\n", markerIdx) + 1;
      const indent = text.substring(lineStart, markerIdx).match(/^\s*/)[0];
      const insertion = `${indent}/** @deprecated ${tag} */\n`;
      return text.slice(0, lineStart) + insertion + text.slice(lineStart);
    }
    const blockText = text.substring(blockStart, blockEnd);
    if (blockText.includes("@deprecated")) {
      return text;
    }
    // Append `* @deprecated ...` immediately before the closing `*/`.
    const indentMatch = blockText.match(/\n(\s*)\*/);
    const indent = indentMatch ? indentMatch[1] : "  ";
    const insertion = `${indent}*\n${indent}* @deprecated ${tag}\n${indent}`;
    return text.slice(0, blockEnd) + insertion + text.slice(blockEnd);
  };
  modified = injectDeprecated(
    modified,
    modified.indexOf(setterMarker),
    "since v10.0.1, use setDecodeThreads()."
  );
  modified = injectDeprecated(
    modified,
    modified.indexOf(getterMarker),
    "since v10.0.1, use decodeThreads."
  );
  if (modified !== dtsTextAfter) {
    writeFileSync(dtsPath, modified, "utf-8");
    console.log(
      `postbuild_alias_contract: ensured @deprecated on decoderThreads in ${dtsPath}`
    );
  } else {
    console.log(
      `postbuild_alias_contract: @deprecated on decoderThreads already present in ${dtsPath}`
    );
  }
}
