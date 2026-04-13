#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. CLI     — all 61 endpoints via generated validate_cli.py (Rust core)
#   2. Python  — all 61 endpoints via generated validate_python.py (PyO3 bridge)
#   3. Go      — all 61 endpoints via generated validate.go (CGo FFI bridge)
#
# Usage:
#   ./scripts/validate_release.sh                    # creds.txt in repo root
#   ./scripts/validate_release.sh /path/to/creds.txt
#
# Prerequisites:
#   Rust, Go, Python, and a working compiler toolchain
#
# The script will build missing local artifacts as needed. If the Python SDK is
# not installed into the current interpreter, it bootstraps a local virtualenv
# under `.venv-release-validate` and installs the PyO3 extension there.

set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CREDS="${1:-$REPO/creds.txt}"

if [ ! -f "$CREDS" ]; then
    echo "error: credentials file not found: $CREDS"
    echo "Create creds.txt with email on line 1, password on line 2."
    exit 1
fi

CREDS="$(cd "$(dirname "$CREDS")" && pwd)/$(basename "$CREDS")"

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

ensure_python_sdk() {
    local py_bin="${PYTHON_BIN:-python3}"
    if "$py_bin" -c "import thetadatadx" >/dev/null 2>&1; then
        PYTHON_BIN="$py_bin"
        return 0
    fi

    local venv_dir="$REPO/.venv-release-validate"
    echo "  Python SDK not installed; bootstrapping $venv_dir"

    if [ ! -x "$venv_dir/bin/python" ]; then
        python3 -m venv "$venv_dir" || return 1
    fi

    "$venv_dir/bin/python" -m pip install --upgrade pip maturin >/dev/null || return 1
    (
        export VIRTUAL_ENV="$venv_dir"
        export PATH="$venv_dir/bin:$PATH"
        cd "$REPO/sdks/python" &&
        "$venv_dir/bin/maturin" develop --release >/dev/null
    ) || return 1

    PYTHON_BIN="$venv_dir/bin/python"
}

# ── 1. CLI (all 61 endpoints) ──────────────────────────────────────────────

section "1/3  CLI — all 61 endpoints"

if [ ! -f "$REPO/target/release/tdx" ]; then
    echo "Building CLI..."
    cargo build --release -p thetadatadx-cli --manifest-path "$REPO/Cargo.toml"
fi

cli_output=$(python3 "$REPO/scripts/validate_cli.py" "$CREDS" 2>&1)
echo "$cli_output"

cli_counts=$(echo "$cli_output" | grep -oP 'COUNTS:\K.*')
cli_pass=$(echo "$cli_counts" | cut -d: -f1)
cli_skip=$(echo "$cli_counts" | cut -d: -f2)
cli_fail=$(echo "$cli_counts" | cut -d: -f3)
record "CLI" "$cli_pass" "$cli_skip" "$cli_fail"

# ── 2. Python SDK (all 61 endpoints) ─────────────────────────────────────

section "2/3  Python SDK — all 61 endpoints"

py_pass=0
py_skip=0
py_fail=0
PYTHON_BIN="${PYTHON_BIN:-python3}"

if ensure_python_sdk; then
    py_result=$("$PYTHON_BIN" "$REPO/scripts/validate_python.py" "$CREDS" 2>&1)
    echo "$py_result"
    py_counts=$(echo "$py_result" | grep -oP 'COUNTS:\K.*')
    py_pass=$(echo "$py_counts" | cut -d: -f1)
    py_skip=$(echo "$py_counts" | cut -d: -f2)
    py_fail=$(echo "$py_counts" | cut -d: -f3)
else
    echo "  Python SDK bootstrap failed."
    py_fail=61
fi
record "Python" "$py_pass" "$py_skip" "$py_fail"

# ── 3. Go SDK (all 61 endpoints) ─────────────────────────────────────────

section "3/3  Go SDK — all 61 endpoints"

go_pass=0
go_skip=0
go_fail=0

FFI_LIB="$REPO/target/release"
if [ ! -f "$FFI_LIB/libthetadatadx_ffi.so" ] && [ ! -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    echo "Building FFI library..."
    cargo build --release -p thetadatadx-ffi --manifest-path "$REPO/Cargo.toml"
fi

if [ -f "$FFI_LIB/libthetadatadx_ffi.so" ] || [ -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    go_result=$(cd "$REPO/sdks/go" && CGO_LDFLAGS="-L$FFI_LIB" LD_LIBRARY_PATH="$FFI_LIB" \
        go run ./cmd/validate "$CREDS" 2>&1)
    echo "$go_result"
    go_counts=$(echo "$go_result" | grep -oP 'COUNTS:\K.*')
    go_pass=$(echo "$go_counts" | cut -d: -f1)
    go_skip=$(echo "$go_counts" | cut -d: -f2)
    go_fail=$(echo "$go_counts" | cut -d: -f3)
else
    echo "  FFI library build failed."
    go_fail=61
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
