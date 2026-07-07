#!/usr/bin/env node
// Drop a freshly built mcp binary into its platform package before publish.
// The release workflow builds one binary per Rust target triple and calls
// this once per triple; it copies the binary into the matching
// `thetadatadx-mcp-server-<platform>` package and marks it executable.
//
//   node scripts/populate.mjs <rust-target-triple> <path-to-binary>
//
// Run from `tools/mcp/npm/` (or anywhere — paths resolve against this file).

import { chmodSync, copyFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// Rust target triple -> [platform package dir, binary name].
const TARGETS = {
  "x86_64-unknown-linux-musl": ["linux-x64", "thetadatadx-mcp-server"],
  "aarch64-unknown-linux-musl": ["linux-arm64", "thetadatadx-mcp-server"],
  "x86_64-apple-darwin": ["darwin-x64", "thetadatadx-mcp-server"],
  "aarch64-apple-darwin": ["darwin-arm64", "thetadatadx-mcp-server"],
  "x86_64-pc-windows-msvc": ["win32-x64", "thetadatadx-mcp-server.exe"],
};

const [, , triple, binary] = process.argv;
if (!triple || !binary) {
  console.error("usage: node scripts/populate.mjs <target-triple> <binary-path>");
  process.exit(2);
}

const entry = TARGETS[triple];
if (!entry) {
  console.error(`unknown target triple: ${triple}`);
  console.error(`known: ${Object.keys(TARGETS).join(", ")}`);
  process.exit(2);
}
if (!existsSync(binary)) {
  console.error(`binary not found: ${binary}`);
  process.exit(2);
}

const [pkgDir, exe] = entry;
const dest = join(dirname(fileURLToPath(import.meta.url)), "..", pkgDir, exe);
copyFileSync(binary, dest);
chmodSync(dest, 0o755);
console.log(`populated ${pkgDir}/${exe} from ${binary}`);
