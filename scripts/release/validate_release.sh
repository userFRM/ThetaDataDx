#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. Python     — generated check_python.py       (PyO3 bridge, live)
#   2. C++        — generated validate.cpp          (C FFI bridge, live)
#   3. TypeScript — emit_validator_manifest.mjs     (public-surface shape)
#   4. Agreement  — cross-language artifact diff    (scripts/ci/check_agreement.py)
#
# Each surface writes a per-cell JSON artifact to
# `artifacts/validator_<lang>.json`. The agreement step asserts that every
# (endpoint, mode) cell present in >=2 artifacts agrees on status and
# row_count. Mismatches fail the release. See PR #291.
#
# Every artifact compared here is produced by this run: the artifacts
# directory is cleared first and each binding is rebuilt from the working
# tree, so a release is never validated against output left by an earlier
# run or another branch.
#
# Usage:
#   ./scripts/release/validate_release.sh            # creds.txt in repo root
#   ./scripts/release/validate_release.sh /path/to/creds.txt
#
# Prerequisites:
#   Rust, Python, a C++17 toolchain, and CMake
#
# The script builds every local artifact it validates. The Python extension is
# compiled from source into a local virtualenv under `.venv-release-validate`;
# set PYTHON_BIN to point at an interpreter you have prepared yourself instead,
# in which case keeping it current is yours to do.

set -uo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
CREDS="${1:-$REPO/creds.txt}"

if [ ! -f "$CREDS" ]; then
    echo "error: credentials file not found: $CREDS"
    echo "Create creds.txt with email on line 1, password on line 2."
    exit 1
fi

CREDS="$(cd "$(dirname "$CREDS")" && pwd)/$(basename "$CREDS")"

# Everything compared below has to come from this run. `check_agreement.py`
# reads whatever sits in `artifacts/`, so a file left by an earlier run, or by
# another branch, would be diffed against today's output and counted as
# agreement between two bindings that were never built together.
rm -f "$REPO"/artifacts/validator_*.json
mkdir -p "$REPO/artifacts"

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
    # A release validates THIS tree. An importable `thetadatadx` may be a
    # released wheel off PyPI or a months-old `maturin develop`, and taking it
    # would validate code that is not the code being shipped -- silently, since
    # a stale extension imports and answers exactly like a current one. So the
    # extension is compiled from source every run. PYTHON_BIN is the way to
    # take that over, and then its freshness is the caller's to own.
    if [ -n "${PYTHON_BIN:-}" ]; then
        echo "  PYTHON_BIN set; using $PYTHON_BIN as given"
        return 0
    fi

    local venv_dir="$REPO/.venv-release-validate"
    echo "  Building the Python extension from source into $venv_dir"

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

section "1/4  Python SDK — live parameter-mode matrix"

py_pass=0
py_skip=0
py_fail=0

if ensure_python_sdk; then
    py_result=$("$PYTHON_BIN" "$REPO/scripts/ci/check_python.py" "$CREDS" 2>&1)
    py_exit=$?
    echo "$py_result"
    parse_counts "Python" "$py_result" "$py_exit" py_pass py_skip py_fail || true
else
    echo "  Python extension build failed."
    py_fail=61
fi
record "Python" "$py_pass" "$py_skip" "$py_fail"

# Rebuilt every run rather than reused when present: `target/release` survives
# a branch switch, so the library the C++ validator loads could otherwise be
# one built from code that is not in this tree. The build is incremental, so
# it costs nothing when it is already current.
FFI_LIB="$REPO/target/release"
echo "Building FFI library..."
if ! cargo build --release -p thetadatadx-ffi --manifest-path "$REPO/Cargo.toml"; then
    echo "  FFI library build failed; the C++ validator cannot load this tree."
    TOTAL_FAIL=$((TOTAL_FAIL + 1))
    SECTION_RESULTS+=("$(printf "  %-12s %3s       %3s      %3d FAIL" "FFI build" "" "" 1)")
fi

# ── 2. C++ SDK ──────────────────────────────────────────────────────────────

section "2/4  C++ SDK — live parameter-mode matrix"

cpp_pass=0
cpp_skip=0
cpp_fail=0

# Same as the FFI library: rebuilt, not reused. The build result is part of
# the condition rather than swallowed, so a failed build cannot fall through to
# a binary an earlier run left behind and report that binary's answers.
CPP_BUILD="$REPO/thetadatadx-cpp/build"
echo "Building C++ validator..."
cpp_built=0
if (cd "$REPO/thetadatadx-cpp" && cmake -B build -S . >/dev/null && cmake --build build --target thetadatadx_validate >/dev/null); then
    cpp_built=1
else
    echo "  C++ validator build failed."
fi

if [ "$cpp_built" -eq 1 ] && [ -x "$CPP_BUILD/thetadatadx_validate" ]; then
    cpp_result=$(cd "$REPO" && LD_LIBRARY_PATH="$FFI_LIB" "$CPP_BUILD/thetadatadx_validate" "$CREDS" 2>&1)
    cpp_exit=$?
    echo "$cpp_result"
    parse_counts "C++" "$cpp_result" "$cpp_exit" cpp_pass cpp_skip cpp_fail || true
else
    echo "  C++ validator build failed or target missing."
    cpp_fail=1
fi
record "C++" "$cpp_pass" "$cpp_skip" "$cpp_fail"

# ── 3. TypeScript shape manifest ────────────────────────────────────────────

section "3/4  TypeScript SDK — public-surface shape manifest"

# Emitted from the committed `index.d.ts`, so it needs node and nothing else:
# no napi build, no credentials, no live traffic. The agreement step compares
# its field SET against the runtime artifacts, and `--require-all-sdks` counts
# it as one of the surfaces that must be present.
ts_fail=0
if node "$REPO/thetadatadx-ts/scripts/emit_validator_manifest.mjs"; then
    echo "  wrote artifacts/validator_typescript.json"
else
    echo "  TypeScript shape manifest emit failed."
    ts_fail=1
fi
record "TypeScript" "$((1 - ts_fail))" 0 "$ts_fail"

# ── 4. Cross-language agreement ─────────────────────────────────────────────

section "4/4  Cross-language agreement"

# `--require-all-sdks`: without it a missing artifact is soft-skipped, so a
# binding can be absent and the gate still passes. A release is exactly where
# every binding must be present to compare. The three steps above produce the
# three artifacts this demands; `LangsAreProducibleTest` keeps that true.
agreement_result=$(python3 "$REPO/scripts/ci/check_agreement.py" --require-all-sdks 2>&1)
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
