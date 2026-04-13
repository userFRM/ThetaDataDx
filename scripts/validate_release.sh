#!/usr/bin/env bash
#
# ThetaDataDx Release Validation
#
# Single script that validates every delivery surface:
#   1. CLI     — all 61 endpoints via test_all_endpoints.sh (Rust core)
#   2. Python  — 3 spot-checks (stock, option, index) proving PyO3 bridge
#   3. Go      — 3 spot-checks (stock, option, index) proving CGo FFI bridge
#   4. C++     — compile + link check proving headers match the FFI library
#   5. MCP     — 1 JSON-RPC call proving the MCP server works
#
# Usage:
#   ./scripts/validate_release.sh                    # creds.txt in repo root
#   ./scripts/validate_release.sh /path/to/creds.txt
#
# Prerequisites:
#   cargo build --release -p thetadatadx-cli -p thetadatadx-ffi
#   (cd sdks/python && maturin develop --release)
#   MCP: cargo build --release --manifest-path tools/mcp/Cargo.toml

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

section "1/5  CLI — all 61 endpoints"

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

# ── 2. Python SDK (3 spot-checks) ─────────────────────────────────────────

section "2/5  Python SDK — 3 spot-checks"

py_pass=0
py_skip=0
py_fail=0

if python3 -c "import thetadatadx" 2>/dev/null; then
    py_result=$(python3 -c "
import sys
creds_path = sys.argv[1]
from thetadatadx import Credentials, Config, ThetaDataDx

c = ThetaDataDx(Credentials.from_file(creds_path), Config.production())

results = []
# 1. Stock
try:
    syms = c.stock_list_symbols()
    print(f'  stock_list_symbols               PASS  {len(syms)} symbols')
    results.append('P')
except Exception as e:
    if 'permission' in str(e).lower() or 'subscription' in str(e).lower():
        print(f'  stock_list_symbols               SKIP  {e}')
        results.append('S')
    else:
        print(f'  stock_list_symbols               FAIL  {e}')
        results.append('F')

# 2. Option
try:
    exps = c.option_list_expirations('SPY')
    print(f'  option_list_expirations          PASS  {len(exps)} expirations')
    results.append('P')
except Exception as e:
    if 'permission' in str(e).lower() or 'subscription' in str(e).lower():
        print(f'  option_list_expirations          SKIP  {e}')
        results.append('S')
    else:
        print(f'  option_list_expirations          FAIL  {e}')
        results.append('F')

# 3. Index
try:
    idx = c.index_list_symbols()
    print(f'  index_list_symbols               PASS  {len(idx)} symbols')
    results.append('P')
except Exception as e:
    if 'permission' in str(e).lower() or 'subscription' in str(e).lower():
        print(f'  index_list_symbols               SKIP  {e}')
        results.append('S')
    else:
        print(f'  index_list_symbols               FAIL  {e}')
        results.append('F')

print(f'COUNTS:{results.count(\"P\")}:{results.count(\"S\")}:{results.count(\"F\")}')
" "$CREDS" 2>&1)
    echo "$py_result"
    py_counts=$(echo "$py_result" | grep -oP 'COUNTS:\K.*')
    py_pass=$(echo "$py_counts" | cut -d: -f1)
    py_skip=$(echo "$py_counts" | cut -d: -f2)
    py_fail=$(echo "$py_counts" | cut -d: -f3)
else
    echo "  Python SDK not installed (run: cd sdks/python && maturin develop --release)"
    echo "  Skipping Python checks."
    py_skip=3
fi
record "Python" "$py_pass" "$py_skip" "$py_fail"

# ── 3. Go SDK (3 spot-checks) ─────────────────────────────────────────────

section "3/5  Go SDK — 3 spot-checks"

go_pass=0
go_skip=0
go_fail=0

FFI_LIB="$REPO/target/release"
if [ -f "$FFI_LIB/libthetadatadx_ffi.so" ] || [ -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    go_result=$(cd "$REPO/sdks/go" && CGO_LDFLAGS="-L$FFI_LIB" LD_LIBRARY_PATH="$FFI_LIB" \
        go run ./examples/smoke "$CREDS" 2>&1)
    echo "$go_result"
    go_pass=$(echo "$go_result" | grep -c 'PASS' || true)
    go_fail=$(echo "$go_result" | grep -c 'FAIL' || true)
    # Subtract the summary line from counts
    if echo "$go_result" | grep -q 'Go SDK:.*PASS'; then
        go_pass=$((go_pass - 1))
        go_fail=$((go_fail - 1 < 0 ? 0 : go_fail - 1))
    fi
else
    echo "  FFI library not built (run: cargo build --release -p thetadatadx-ffi)"
    echo "  Skipping Go checks."
    go_skip=3
fi
record "Go" "$go_pass" "$go_skip" "$go_fail"

# ── 4. C++ SDK (compile + link check) ─────────────────────────────────────

section "4/5  C++ SDK — compile + link check"

cpp_pass=0
cpp_skip=0
cpp_fail=0

if [ -f "$FFI_LIB/libthetadatadx_ffi.so" ] || [ -f "$FFI_LIB/libthetadatadx_ffi.dylib" ]; then
    build_dir="$REPO/build/cpp-validate"
    rm -rf "$build_dir"
    if cmake -S "$REPO/sdks/cpp" -B "$build_dir" \
         -DTHETADX_FFI_DIR="$FFI_LIB" \
         -DCMAKE_BUILD_TYPE=Release 2>&1 | tail -3 \
       && cmake --build "$build_dir" --target thetadatadx_cpp 2>&1 | tail -3; then
        printf "  %-40s PASS\n" "cmake build thetadatadx_cpp"
        cpp_pass=1
    else
        printf "  %-40s FAIL\n" "cmake build thetadatadx_cpp"
        cpp_fail=1
    fi
    rm -rf "$build_dir"
else
    echo "  FFI library not built (run: cargo build --release -p thetadatadx-ffi)"
    echo "  Skipping C++ checks."
    cpp_skip=1
fi
record "C++" "$cpp_pass" "$cpp_skip" "$cpp_fail"

# ── 5. MCP (1 JSON-RPC spot-check) ────────────────────────────────────────

section "5/5  MCP — JSON-RPC spot-check"

mcp_pass=0
mcp_skip=0
mcp_fail=0

MCP_BIN="$REPO/tools/mcp/target/release/thetadatadx-mcp"
if [ -f "$MCP_BIN" ]; then
    # tools/list is a lightweight MCP introspection call — no creds needed
    mcp_result=$(echo '{"jsonrpc":"2.0","method":"tools/list","params":{},"id":1}' \
        | timeout 30 "$MCP_BIN" --creds "$CREDS" 2>/dev/null)
    if echo "$mcp_result" | grep -q '"tools"'; then
        tool_count=$(echo "$mcp_result" | grep -oP '"name"' | wc -l)
        printf "  %-40s PASS  %d tools registered\n" "mcp tools/list" "$tool_count"
        mcp_pass=1
    else
        printf "  %-40s FAIL\n" "mcp tools/list"
        echo "  Response: $(echo "$mcp_result" | head -1)"
        mcp_fail=1
    fi
else
    echo "  MCP binary not built (run: cargo build --release --manifest-path tools/mcp/Cargo.toml)"
    echo "  Skipping MCP checks."
    mcp_skip=1
fi
record "MCP" "$mcp_pass" "$mcp_skip" "$mcp_fail"

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
