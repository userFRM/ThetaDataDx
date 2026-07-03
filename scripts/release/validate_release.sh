#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. Python    — generated check_python.py        (PyO3 bridge)
#   2. C++       — generated validate.cpp           (C FFI bridge)
#   3. Agreement — cross-language artifact diff     (scripts/ci/check_agreement.py)
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

parse_counts() {
    local surface="$1" result="$2" exit_code="$3" pass_var="$4" skip_var="$5" fail_var="$6"
    local counts pass skip fail status=0

    counts=$(printf "%s\n" "$result" | sed -n 's/^.*COUNTS:\([0-9][0-9]*:[0-9][0-9]*:[0-9][0-9]*\).*$/\1/p' | tail -n 1)
    if [ -z "$counts" ]; then
        echo "  $surface validator did not emit COUNTS:p:s:f."
        if [ "$exit_code" -ne 0 ]; then
            echo "  $surface validator exited with status $exit_code."
        fi
        printf -v "$pass_var" "%d" 0
        printf -v "$skip_var" "%d" 0
        printf -v "$fail_var" "%d" 1
        return 1
    fi

    IFS=: read -r pass skip fail <<<"$counts"
    if [ "$exit_code" -ne 0 ]; then
        echo "  $surface validator exited with status $exit_code."
        if [ "$fail" -eq 0 ]; then
            fail=1
        fi
        status=1
    fi

    printf -v "$pass_var" "%d" "$pass"
    printf -v "$skip_var" "%d" "$skip"
    printf -v "$fail_var" "%d" "$fail"
    return "$status"
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
        cd "$REPO/thetadatadx-py" &&
        "$venv_dir/bin/maturin" develop --release >/dev/null
    ) || return 1

    PYTHON_BIN="$venv_dir/bin/python"
}

# ── 1. Python SDK ───────────────────────────────────────────────────────────

section "1/3  Python SDK — live parameter-mode matrix"

py_pass=0
py_skip=0
py_fail=0
PYTHON_BIN="${PYTHON_BIN:-python3}"

if ensure_python_sdk; then
    py_result=$("$PYTHON_BIN" "$REPO/scripts/ci/check_python.py" "$CREDS" 2>&1)
    py_exit=$?
    echo "$py_result"
    parse_counts "Python" "$py_result" "$py_exit" py_pass py_skip py_fail || true
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

# ── 2. C++ SDK ──────────────────────────────────────────────────────────────

section "2/3  C++ SDK — live parameter-mode matrix"

cpp_pass=0
cpp_skip=0
cpp_fail=0

CPP_BUILD="$REPO/thetadatadx-cpp/build"
if [ ! -f "$CPP_BUILD/thetadatadx_validate" ]; then
    echo "Building C++ validator..."
    (cd "$REPO/thetadatadx-cpp" && cmake -B build -S . >/dev/null 2>&1 && cmake --build build --target thetadatadx_validate >/dev/null 2>&1) || true
fi

if [ -x "$CPP_BUILD/thetadatadx_validate" ]; then
    cpp_result=$(cd "$REPO" && LD_LIBRARY_PATH="$FFI_LIB" "$CPP_BUILD/thetadatadx_validate" "$CREDS" 2>&1)
    cpp_exit=$?
    echo "$cpp_result"
    parse_counts "C++" "$cpp_result" "$cpp_exit" cpp_pass cpp_skip cpp_fail || true
else
    echo "  C++ validator build failed or target missing."
    cpp_fail=1
fi
record "C++" "$cpp_pass" "$cpp_skip" "$cpp_fail"

# ── 3. Cross-language agreement ─────────────────────────────────────────────

section "3/3  Cross-language agreement"

agreement_result=$(python3 "$REPO/scripts/ci/check_agreement.py" 2>&1)
agreement_exit=$?
echo "$agreement_result"
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
