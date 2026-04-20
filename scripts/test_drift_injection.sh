#!/usr/bin/env bash
#
# FPSS drift-injection regression test.
#
# Purpose:
#   Prove that the `static_assert(offsetof(...))` guards inside
#   `sdks/cpp/include/thetadx.hpp` ACTUALLY fire when a FPSS schema
#   field is reordered. Without this test, a silent break in the
#   guards (e.g. someone adding `#if 0 ... #endif`) would go unnoticed
#   until a customer saw wire-format corruption.
#
# Mechanism:
#   1. Swap `bid` <-> `ask` in the `Quote` variant of
#      `crates/thetadatadx/fpss_event_schema.toml`. This changes the
#      generated C struct layout but leaves the hand-rolled
#      `static_assert(offsetof)` guards in `thetadx.hpp` pointing at
#      the ORIGINAL offsets.
#   2. Regenerate SDK surfaces (build.rs + generate_sdk_surfaces),
#      which rewrites `sdks/cpp/include/fpss_event_structs.h.inc` with
#      the swapped field order.
#   3. Rebuild the C++ SDK. A healthy guard set MUST fail compilation
#      because at least one `static_assert` will now disagree with the
#      regenerated offset.
#
# Contract:
#   - script exits 0 ONLY when the C++ build fails during drift
#     (the guards did their job).
#   - script exits 1 if the C++ build succeeds despite drift
#     (the guards are silently broken).
#
# Usage:
#   ./scripts/test_drift_injection.sh
#
# CI: wired into .github/workflows/ci.yml (drift-injection job).

set -euo pipefail

# Derive repo root from the script location so the test works from any
# cwd (including CI runners that invoke scripts by absolute path).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

SCHEMA="crates/thetadatadx/fpss_event_schema.toml"
CPP_BUILD_DIR="build/drift-injection-cpp"
BACKUP_DIR="build/drift-injection-backup"

# Running `cargo run generate_sdk_surfaces` also triggers
# `crates/thetadatadx/build.rs`, which regenerates every SDK surface in
# the workspace. We cannot enumerate the full transitive closure of
# generator outputs (it shifts when upstream adds / removes emitters),
# so we take a byte-level snapshot of every tracked file the generator
# could touch and restore it on exit. This keeps local runs safe for
# in-progress work while remaining a no-op on a clean CI checkout.
#
# Scope: every file under the language SDK trees and the FFI bridge,
# plus the schema itself. Everything else (workspace roots, Cargo.toml,
# src/) is untouched by the generators.
SNAPSHOT_PATHS=(
    "$SCHEMA"
    "ffi/src"
    "sdks/cpp/include"
    "sdks/cpp/src"
    "sdks/cpp/examples"
    "sdks/go"
    "sdks/python/src"
    "sdks/typescript/src"
    "tools/cli/src"
    "tools/mcp/src"
    "scripts/validate_cli.py"
    "scripts/validate_python.py"
)

# Byte-level pre-test snapshot. Covers in-progress work too -- a plain
# `git checkout` would overwrite it with HEAD, which is wrong.
echo "[drift-injection] snapshotting tracked files before mutation"
mkdir -p "$BACKUP_DIR"
for path in "${SNAPSHOT_PATHS[@]}"; do
    if [ -e "$path" ]; then
        mkdir -p "$BACKUP_DIR/$(dirname "$path")"
        cp -r "$path" "$BACKUP_DIR/$path"
    fi
done

cleanup() {
    echo ""
    echo "[drift-injection] restoring snapshotted files"
    for path in "${SNAPSHOT_PATHS[@]}"; do
        if [ -e "$BACKUP_DIR/$path" ]; then
            rm -rf "$path"
            cp -r "$BACKUP_DIR/$path" "$path"
        fi
    done
    rm -rf "$CPP_BUILD_DIR" "$BACKUP_DIR"
}
trap cleanup EXIT

echo "[drift-injection] 1/3 swapping bid <-> ask in Quote variant"
python3 - "$SCHEMA" <<'PY'
import pathlib, sys

path = pathlib.Path(sys.argv[1])
content = path.read_text()

if 'name = "bid"' not in content or 'name = "ask"' not in content:
    sys.stderr.write(f"ERROR: expected both bid and ask fields in {path}\n")
    sys.exit(2)

# Three-way swap via placeholder so we don't double-rename.
content = content.replace('name = "bid",', 'name = "__TMP_BID__",', 1)
content = content.replace('name = "ask",', 'name = "bid",', 1)
content = content.replace('name = "__TMP_BID__",', 'name = "ask",', 1)

path.write_text(content)
print(f"  swapped bid <-> ask in {path}")
PY

echo "[drift-injection] 2/3 regenerating SDK surfaces"
# Run the generator with --write so it rewrites the .inc files.
# `--check` mode would diff-report only; we need the actual mutation
# so the C++ build sees the drifted layout. CI uses a fresh checkout
# where Cargo.lock is up to date, so the `cargo run` below does not
# mutate it; adding `--locked` here would make local runs fail when
# in-progress dependency edits have not yet refreshed the lockfile.
cargo run \
    --manifest-path crates/thetadatadx/Cargo.toml \
    --features config-file \
    --bin generate_sdk_surfaces

echo "[drift-injection] 3/3 attempting C++ build (MUST fail)"
# Build only the core C++ target — no need to link the FFI or build
# examples. This keeps the test narrow: we only care whether the
# static_assert guards in thetadx.hpp fire.
cmake -S sdks/cpp -B "$CPP_BUILD_DIR" >/dev/null
set +e
cmake --build "$CPP_BUILD_DIR" --config Release --target thetadatadx_cpp 2>&1 | tee "$CPP_BUILD_DIR/drift-build.log"
build_status=${PIPESTATUS[0]}
set -e

if [ "$build_status" -eq 0 ]; then
    echo ""
    echo "FAIL: drift injection did NOT cause the C++ build to fail." >&2
    echo "      The static_assert(offsetof) guards in thetadx.hpp are" >&2
    echo "      silently broken. Investigate before merging." >&2
    exit 1
fi

# Confirm the failure was specifically a static_assert trigger, not
# some unrelated CMake / linker error. The guard strings in
# thetadx.hpp all end with `offset drifted` or `total size drifted`.
if ! grep -qE "offset drifted|total size drifted" "$CPP_BUILD_DIR/drift-build.log"; then
    echo ""
    echo "FAIL: C++ build failed, but NOT due to a static_assert drift" >&2
    echo "      guard. See $CPP_BUILD_DIR/drift-build.log for details." >&2
    exit 1
fi

echo ""
echo "PASS: drift injection correctly failed the C++ build via static_assert guards."
exit 0
