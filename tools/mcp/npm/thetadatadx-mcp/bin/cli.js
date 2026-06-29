#!/usr/bin/env node
// Launcher for the thetadatadx-mcp server. The server is a native Rust
// binary shipped as one optional dependency per platform (the
// esbuild / biome model); this script resolves the binary for the host
// platform and execs it, forwarding stdin/stdout (the MCP JSON-RPC
// channel), stderr (logs), argv, and the environment (so THETADATA_API_KEY
// and friends reach the server).

const { spawnSync } = require("node:child_process");
const { dirname, join } = require("node:path");

// `${platform}-${arch}` -> the platform package that carries its binary.
// Keys are Node's `process.platform`/`process.arch` values, so the lookup
// needs no abi/libc guessing — the published set is exactly these.
const PACKAGES = {
  "linux-x64": "thetadatadx-mcp-linux-x64",
  "linux-arm64": "thetadatadx-mcp-linux-arm64",
  "darwin-x64": "thetadatadx-mcp-darwin-x64",
  "darwin-arm64": "thetadatadx-mcp-darwin-arm64",
  "win32-x64": "thetadatadx-mcp-win32-x64",
};

function binaryPath() {
  const key = `${process.platform}-${process.arch}`;
  const pkg = PACKAGES[key];
  if (!pkg) {
    throw new Error(
      `thetadatadx-mcp does not ship a prebuilt binary for ${key}.\n` +
        `Supported: ${Object.keys(PACKAGES).join(", ")}.\n` +
        `Build from source instead:\n` +
        `  cargo install thetadatadx-mcp --git https://github.com/userFRM/ThetaDataDx`,
    );
  }
  const exe = process.platform === "win32" ? "thetadatadx-mcp.exe" : "thetadatadx-mcp";
  try {
    // Resolve the platform package's `package.json` (not the binary
    // directly): an extensionless executable is not a resolvable module
    // specifier, so anchor on the manifest and join the binary beside it.
    return join(dirname(require.resolve(`${pkg}/package.json`)), exe);
  } catch {
    throw new Error(
      `The ${pkg} package is not installed.\n` +
        `It is an optional dependency for ${key} and npm skips it when the\n` +
        `host does not match, or when optional dependencies are disabled\n` +
        `(e.g. --no-optional, --omit=optional, or a locked-down CI install).\n` +
        `Reinstall with optional dependencies enabled, or build from source:\n` +
        `  cargo install thetadatadx-mcp --git https://github.com/userFRM/ThetaDataDx`,
    );
  }
}

const result = spawnSync(binaryPath(), process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  throw result.error;
}
// Re-raise the child's terminating signal as our own exit so a parent
// supervisor sees the real cause; otherwise mirror the exit code.
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
