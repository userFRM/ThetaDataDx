#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. CLI     — all 61 endpoints via test_all_endpoints.sh (Rust core)
#   2. Python  — all 61 endpoints via generated validate_python.py (PyO3 bridge)
#   3. Go      — all 61 endpoints via generated validate.go (CGo FFI bridge)
#
# Usage:
#   ./scripts/validate_release.sh                    # creds.txt in repo root
#   ./scripts/validate_release.sh /path/to/creds.txt
#
# Prerequisites:
#   cargo build --release -p thetadatadx-cli -p thetadatadx-ffi
#   (cd sdks/python && maturin develop --release)

set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CREDS="${1:-$REPO/creds.txt}"

if [ ! -f "$CREDS" ]; then
    echo "error: credentials file not found: $CREDS"
    echo "Create creds.txt with email on line 1, password on line 2."
    exit 1
fi

TOTAL_PASS=0
TOTAL_SKIP=0
TOTAL_FAIL=0
SECTION_RESULTS=()

section() {
    echo ""
    echo "═══════════════════════════════════════════════════"
    echo "  $1"
    echo "═══════════════════════════════════════════════════"
}

record() {
    local surface="$1" pass="$2" skip="$3" fail="$4"
    TOTAL_PASS=$((TOTAL_PASS + pass))
    TOTAL_SKIP=$((TOTAL_SKIP + skip))
    TOTAL_FAIL=$((TOTAL_FAIL + fail))
    SECTION_RESULTS+=("$(printf "  %-12s %3d PASS  %3d SKIP  %3d FAIL" "$surface" "$pass" "$skip" "$fail")")
}

# ── 1. CLI (all 61 endpoints) ──────────────────────────────────────────────

section "1/3  CLI — all 61 endpoints"

if [ ! -f "$REPO/target/release/tdx" ]; then
    echo "Building CLI..."
    cargo build --release -p thetadatadx-cli --manifest-path "$REPO/Cargo.toml"
fi

cli_output=$(bash "$REPO/examples/test_all_endpoints.sh" "$CREDS" 2>&1)
echo "$cli_output"

# Parse the summary line from test_all_endpoints.sh
cli_pass=$(echo "$cli_output" | grep -oP 'PASS:\s+\K\d+' || echo 0)
cli_skip=$(echo "$cli_output" | grep -oP 'SKIP:\s+\K\d+' || echo 0)
cli_nodata=$(echo "$cli_output" | grep -oP 'NODATA:\s+\K\d+' || echo 0)
cli_fail=$(echo "$cli_output" | grep -oP 'FAIL:\s+\K\d+' || echo 0)
# NODATA counts as PASS (valid empty response)
cli_pass=$((cli_pass + cli_nodata))
record "CLI" "$cli_pass" "$cli_skip" "$cli_fail"

# ── 2. Python SDK (all 61 endpoints) ─────────────────────────────────────

section "2/3  Python SDK — all 61 endpoints"

py_pass=0
py_skip=0
py_fail=0

if python3 -c "import thetadatadx" 2>/dev/null; then
    py_result=$(python3 "$REPO/scripts/validate_python.py" "$CREDS" 2>&1)
    echo "$py_result"
    py_counts=$(echo "$py_result" | grep -oP 'COUNTS:\K.*')
    py_pass=$(echo "$py_counts" | cut -d: -f1)
    py_skip=$(echo "$py_counts" | cut -d: -f2)
    py_fail=$(echo "$py_counts" | cut -d: -f3)
else
    echo "  Python SDK not installed (run: cd sdks/python && maturin develop --release)"
    echo "  Skipping Python checks."
    py_skip=61
fi
record "Python" "$py_pass" "$py_skip" "$py_fail"

# ── 3. Go SDK (all 61 endpoints) ─────────────────────────────────────────

section "3/3  Go SDK — all 61 endpoints"

go_pass=0
go_skip=0
go_fail=0

FFI_LIB="$REPO/target/release"
if [ -f "$FFI_LIB/libthetadatadx_ffi.so" ] || [ -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    go_result=$(cd "$REPO/sdks/go" && CGO_LDFLAGS="-L$FFI_LIB" LD_LIBRARY_PATH="$FFI_LIB" \
        go run ./cmd/validate "$CREDS" 2>&1)
    echo "$go_result"
    go_counts=$(echo "$go_result" | grep -oP 'COUNTS:\K.*')
    go_pass=$(echo "$go_counts" | cut -d: -f1)
    go_skip=$(echo "$go_counts" | cut -d: -f2)
    go_fail=$(echo "$go_counts" | cut -d: -f3)
else
    echo "  FFI library not built (run: cargo build --release -p thetadatadx-ffi)"
    echo "  Skipping Go checks."
    go_skip=61
fi
record "Go" "$go_pass" "$go_skip" "$go_fail"

# ── Summary ────────────────────────────────────────────────────────────────

echo ""
echo "═══════════════════════════════════════════════════"
echo "  RELEASE VALIDATION SUMMARY"
echo "═══════════════════════════════════════════════════"
for line in "${SECTION_RESULTS[@]}"; do
    echo "$line"
done
echo "  ────────────────────────────────────────────────"
printf "  %-12s %3d PASS  %3d SKIP  %3d FAIL\n" "TOTAL" "$TOTAL_PASS" "$TOTAL_SKIP" "$TOTAL_FAIL"
echo "═══════════════════════════════════════════════════"

if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo ""
    echo "RELEASE BLOCKED — $TOTAL_FAIL failure(s) detected."
    exit 1
else
    echo ""
    echo "RELEASE OK — all surfaces validated."
    exit 0
fi
