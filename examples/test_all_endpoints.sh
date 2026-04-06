#!/usr/bin/env bash
#
# ThetaDataDx -- Live Endpoint Test Suite
#
# Tests all 61 endpoints against the ThetaData MDDS server.
# Requires: creds.txt (email line 1, password line 2)
#
# Usage:
#   ./examples/test_all_endpoints.sh                    # default creds.txt
#   ./examples/test_all_endpoints.sh /path/to/creds.txt # custom creds path
#
# Output: endpoint_test_results.md (query + truncated response for each endpoint)

set -uo pipefail

CREDS="${1:-creds.txt}"
TDX="$(dirname "$0")/../target/release/tdx"

if [ ! -f "$TDX" ]; then
    echo "Building CLI..."
    cargo build --release -p thetadatadx-cli
fi

if [ ! -f "$CREDS" ]; then
    echo "Error: credentials file not found: $CREDS"
    echo "Create creds.txt with email on line 1, password on line 2."
    exit 1
fi

OUT="endpoint_test_results.md"
PASS=0
FAIL=0
DENIED=0
NODATA=0
TOTAL=0

echo "# ThetaDataDx -- Live Endpoint Test Results" > "$OUT"
echo "" >> "$OUT"
echo "Date: $(date -u '+%Y-%m-%d %H:%M UTC')" >> "$OUT"
echo "Version: $($TDX --version 2>&1 || echo 'unknown')" >> "$OUT"
echo "" >> "$OUT"

run() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    printf "  %-45s" "$name"

    local start_ms=$(($(date +%s%N) / 1000000))
    result=$("$TDX" --creds "$CREDS" "$@" 2>&1 | head -15)
    local end_ms=$(($(date +%s%N) / 1000000))
    local elapsed_ms=$((end_ms - start_ms))

    echo "## $TOTAL. $name" >> "$OUT"
    echo '```' >> "$OUT"
    echo "\$ tdx $*" >> "$OUT"
    echo "$result" >> "$OUT"
    echo '```' >> "$OUT"
    echo "**Latency: ${elapsed_ms}ms**" >> "$OUT"

    if echo "$result" | grep -q 'panicked'; then
        echo "PANIC (${elapsed_ms}ms)"
        echo "**PANIC** -- CLI crash" >> "$OUT"
        FAIL=$((FAIL + 1))
    elif echo "$result" | grep -qi 'does not have permission'; then
        tier=$(echo "$result" | grep -oP 'requiring a \K\w+')
        echo "SKIP ($tier tier) (${elapsed_ms}ms)"
        echo "**SKIP** -- requires $tier subscription" >> "$OUT"
        DENIED=$((DENIED + 1))
    elif echo "$result" | grep -q 'No data found'; then
        echo "OK (no data) (${elapsed_ms}ms)"
        echo "**OK** -- no data for query (valid response)" >> "$OUT"
        NODATA=$((NODATA + 1))
    elif echo "$result" | grep -q '^error:'; then
        echo "FAIL (${elapsed_ms}ms)"
        echo "**FAIL**" >> "$OUT"
        FAIL=$((FAIL + 1))
    else
        rows=$(echo "$result" | grep -c '│' || true)
        echo "PASS ($rows rows) (${elapsed_ms}ms)"
        echo "**PASS** ($rows rows)" >> "$OUT"
        PASS=$((PASS + 1))
    fi
    echo "" >> "$OUT"
}

echo "Testing all 61 endpoints..."
echo ""

# ── Stock List (2) ──────────────────────────────────────────────
echo "Stock List:"
run "stock list_symbols"                     stock list_symbols
run "stock list_dates"                       stock list_dates TRADE AAPL

# ── Stock Snapshot (4) ──────────────────────────────────────────
echo "Stock Snapshot:"
run "stock snapshot_ohlc"                    stock snapshot_ohlc AAPL
run "stock snapshot_trade"                   stock snapshot_trade AAPL
run "stock snapshot_quote"                   stock snapshot_quote AAPL
run "stock snapshot_market_value"            stock snapshot_market_value AAPL

# ── Stock History (6) ───────────────────────────────────────────
echo "Stock History:"
run "stock history_eod"                      stock history_eod AAPL 20260401 20260404
run "stock history_ohlc"                     stock history_ohlc AAPL 20260402 60000
run "stock history_ohlc_range"               stock history_ohlc_range AAPL 20260401 20260402 60000
run "stock history_trade"                    stock history_trade AAPL 20260402
run "stock history_quote"                    stock history_quote AAPL 20260402 60000
run "stock history_trade_quote"              stock history_trade_quote AAPL 20260402

# ── Stock At-Time (2) ──────────────────────────────────────────
echo "Stock At-Time:"
run "stock at_time_trade"                    stock at_time_trade AAPL 20260401 20260402 34200000
run "stock at_time_quote"                    stock at_time_quote AAPL 20260401 20260402 34200000

# ── Option List (5) ─────────────────────────────────────────────
echo "Option List:"
run "option list_symbols"                    option list_symbols
run "option list_dates"                      option list_dates TRADE SPY 20260417 550 C
run "option list_expirations"                option list_expirations SPY
run "option list_strikes"                    option list_strikes SPY 20260417
run "option list_contracts"                  option list_contracts TRADE SPY 20260402

# ── Option Snapshot (5) ─────────────────────────────────────────
echo "Option Snapshot:"
run "option snapshot_ohlc"                   option snapshot_ohlc SPY 20260417 550 C
run "option snapshot_trade"                  option snapshot_trade SPY 20260417 550 C
run "option snapshot_quote"                  option snapshot_quote SPY 20260417 550 C
run "option snapshot_open_interest"          option snapshot_open_interest SPY 20260417 550 C
run "option snapshot_market_value"           option snapshot_market_value SPY 20260417 550 C

# ── Option Snapshot Greeks (5) ──────────────────────────────────
echo "Option Snapshot Greeks:"
run "option snapshot_greeks_iv"              option snapshot_greeks_implied_volatility SPY 20260417 550 C
run "option snapshot_greeks_all"             option snapshot_greeks_all SPY 20260417 550 C
run "option snapshot_greeks_first_order"     option snapshot_greeks_first_order SPY 20260417 550 C
run "option snapshot_greeks_second_order"    option snapshot_greeks_second_order SPY 20260417 550 C
run "option snapshot_greeks_third_order"     option snapshot_greeks_third_order SPY 20260417 550 C

# ── Option History (6) ──────────────────────────────────────────
echo "Option History:"
run "option history_eod"                     option history_eod SPY 20260417 550 C 20260401 20260402
run "option history_ohlc"                    option history_ohlc SPY 20260417 550 C 20260402 60000
run "option history_trade"                   option history_trade SPY 20260417 550 C 20260402
run "option history_quote"                   option history_quote SPY 20260417 550 C 20260402 60000
run "option history_trade_quote"             option history_trade_quote SPY 20260417 550 C 20260402
run "option history_open_interest"           option history_open_interest SPY 20260417 550 C 20260402

# ── Option History Greeks (6) ───────────────────────────────────
echo "Option History Greeks:"
run "option history_greeks_eod"              option history_greeks_eod SPY 20260417 550 C 20260401 20260402
run "option history_greeks_all"              option history_greeks_all SPY 20260417 550 C 20260402 60000
run "option history_greeks_first_order"      option history_greeks_first_order SPY 20260417 550 C 20260402 60000
run "option history_greeks_second_order"     option history_greeks_second_order SPY 20260417 550 C 20260402 60000
run "option history_greeks_third_order"      option history_greeks_third_order SPY 20260417 550 C 20260402 60000
run "option history_greeks_iv"               option history_greeks_implied_volatility SPY 20260417 550 C 20260402 60000

# ── Option Trade Greeks (5) ─────────────────────────────────────
echo "Option Trade Greeks:"
run "option history_trade_greeks_all"        option history_trade_greeks_all SPY 20260417 550 C 20260402
run "option history_trade_greeks_first"      option history_trade_greeks_first_order SPY 20260417 550 C 20260402
run "option history_trade_greeks_second"     option history_trade_greeks_second_order SPY 20260417 550 C 20260402
run "option history_trade_greeks_third"      option history_trade_greeks_third_order SPY 20260417 550 C 20260402
run "option history_trade_greeks_iv"         option history_trade_greeks_implied_volatility SPY 20260417 550 C 20260402

# ── Option At-Time (2) ──────────────────────────────────────────
echo "Option At-Time:"
run "option at_time_trade"                   option at_time_trade SPY 20260417 550 C 20260401 20260402 34200000
run "option at_time_quote"                   option at_time_quote SPY 20260417 550 C 20260401 20260402 34200000

# ── Index List (2) ──────────────────────────────────────────────
echo "Index List:"
run "index list_symbols"                     index list_symbols
run "index list_dates"                       index list_dates SPX

# ── Index Snapshot (3) ──────────────────────────────────────────
echo "Index Snapshot:"
run "index snapshot_ohlc"                    index snapshot_ohlc SPX
run "index snapshot_price"                   index snapshot_price SPX
run "index snapshot_market_value"            index snapshot_market_value SPX

# ── Index History (3) ───────────────────────────────────────────
echo "Index History:"
run "index history_eod"                      index history_eod SPX 20260401 20260402
run "index history_ohlc"                     index history_ohlc SPX 20260401 20260402 60000
run "index history_price"                    index history_price 20260402 SPX 60000

# ── Index At-Time (1) ──────────────────────────────────────────
echo "Index At-Time:"
run "index at_time_price"                    index at_time_price SPX 20260401 20260402 34200000

# ── Calendar (3) ────────────────────────────────────────────────
echo "Calendar:"
run "calendar open_today"                    calendar open_today
run "calendar on_date"                       calendar on_date 20260406
run "calendar year"                          calendar year 2026

# ── Rate (1) ────────────────────────────────────────────────────
echo "Rate:"
run "rate history_eod"                       rate history_eod SOFR 20260401 20260402

# ── Summary ─────────────────────────────────────────────────────
echo ""
echo "========================================="
echo "  TOTAL:   $TOTAL / 61 endpoints"
echo "  PASS:    $PASS"
echo "  SKIP:    $DENIED (subscription tier)"
echo "  NODATA:  $NODATA (valid empty response)"
echo "  FAIL:    $FAIL"
echo "========================================="

echo "---" >> "$OUT"
echo "## Summary" >> "$OUT"
echo "" >> "$OUT"
echo "| Status | Count |" >> "$OUT"
echo "|--------|-------|" >> "$OUT"
echo "| PASS | $PASS |" >> "$OUT"
echo "| SKIP (subscription) | $DENIED |" >> "$OUT"
echo "| OK (no data) | $NODATA |" >> "$OUT"
echo "| FAIL | $FAIL |" >> "$OUT"
echo "| **Total** | **$TOTAL** |" >> "$OUT"

echo ""
echo "Results: $OUT"
