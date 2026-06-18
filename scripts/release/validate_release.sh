#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. CLI       — generated check_cli.py           (Rust core)
#   2. Python    — generated check_python.py        (PyO3 bridge)
#   3. C++       — generated validate.cpp           (C FFI bridge)
#   4. Agreement — cross-language artifact diff     (scripts/ci/check_agreement.py)
#
# Each SDK validator writes a per-cell JSON artifact to
# `artifacts/validator_<lang>.json`. The agreement step asserts that every
# (endpoint, mode) cell present in >=2 artifacts agrees on status and
# row_count. Mismatches fail the release. See PR #291.
#
# Usage:
#   ./scripts/release/validate_release.sh            # creds.txt in repo root
#   ./scripts/release/validate_release.sh /path/to/creds.txt
#
# Prerequisites:
#   Rust, Python, a C++17 toolchain, and CMake
#
# The script will build missing local artifacts as needed. If the Python SDK is
# not installed into the current interpreter, it bootstraps a local virtualenv
# under `.venv-release-validate` and installs the PyO3 extension there.

set -uo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
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

# ── 1. CLI ──────────────────────────────────────────────────────────────────

section "1/5  CLI — live parameter-mode matrix"

if [ ! -f "$REPO/target/release/thetadatadx" ]; then
    echo "Building CLI..."
    cargo build --release -p thetadatadx-cli --manifest-path "$REPO/Cargo.toml"
fi

cli_output=$(python3 "$REPO/scripts/ci/check_cli.py" "$CREDS" 2>&1)
echo "$cli_output"

cli_counts=$(echo "$cli_output" | grep -oP 'COUNTS:\K.*')
cli_pass=$(echo "$cli_counts" | cut -d: -f1)
cli_skip=$(echo "$cli_counts" | cut -d: -f2)
cli_fail=$(echo "$cli_counts" | cut -d: -f3)
record "CLI" "$cli_pass" "$cli_skip" "$cli_fail"

# ── 2. Python SDK ───────────────────────────────────────────────────────────

section "2/5  Python SDK — live parameter-mode matrix"

py_pass=0
py_skip=0
py_fail=0
PYTHON_BIN="${PYTHON_BIN:-python3}"

if ensure_python_sdk; then
    py_result=$("$PYTHON_BIN" "$REPO/scripts/ci/check_python.py" "$CREDS" 2>&1)
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

FFI_LIB="$REPO/target/release"
if [ ! -f "$FFI_LIB/libthetadatadx_ffi.so" ] && [ ! -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    echo "Building FFI library..."
    cargo build --release -p thetadatadx-ffi --manifest-path "$REPO/Cargo.toml"
fi

# ── 3. C++ SDK ──────────────────────────────────────────────────────────────

section "3/4  C++ SDK — live parameter-mode matrix"

cpp_pass=0
cpp_skip=0
cpp_fail=0

CPP_BUILD="$REPO/sdks/cpp/build"
if [ ! -f "$CPP_BUILD/thetadatadx_validate" ]; then
    echo "Building C++ validator..."
    (cd "$REPO/sdks/cpp" && cmake -B build -S . >/dev/null 2>&1 && cmake --build build --target thetadatadx_validate >/dev/null 2>&1) || true
fi

if [ -x "$CPP_BUILD/thetadatadx_validate" ]; then
    cpp_result=$(cd "$REPO" && LD_LIBRARY_PATH="$FFI_LIB" "$CPP_BUILD/thetadatadx_validate" "$CREDS" 2>&1)
    echo "$cpp_result"
    cpp_counts=$(echo "$cpp_result" | grep -oP 'COUNTS:\K.*')
    cpp_pass=$(echo "$cpp_counts" | cut -d: -f1)
    cpp_skip=$(echo "$cpp_counts" | cut -d: -f2)
    cpp_fail=$(echo "$cpp_counts" | cut -d: -f3)
else
    echo "  C++ validator build failed or target missing."
    cpp_fail=1
fi
record "C++" "$cpp_pass" "$cpp_skip" "$cpp_fail"

# ── 4. Cross-language agreement ─────────────────────────────────────────────

section "4/4  Cross-language agreement"

agreement_result=$(python3 "$REPO/scripts/ci/check_agreement.py" 2>&1)
echo "$agreement_result"
agreement_exit=$?
if [ "$agreement_exit" -ne 0 ]; then
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
    SECTION_RESULTS+=("$(printf "  %-12s %3s       %3s      %3d FAIL" "Agreement" "" "" 1)")
else
    SECTION_RESULTS+=("$(printf "  %-12s %3s PASS  %3s SKIP  %3s FAIL" "Agreement" "1" "0" "0")")
fi

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
