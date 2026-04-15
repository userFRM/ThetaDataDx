#!/usr/bin/env python3
"""Static cross-check that CLI `--format json-raw` field schemas match
the canonical SDK schemas in `sdks/python/src/tick_columnar.rs`.

The agreement validator compares CLI vs Python/Go cell-by-cell on
`first_row` content. If the CLI emits `time` while Python emits
`ms_of_day`, or if the CLI drops `expiration`, every cell in those
endpoints will false-diff. This script parses both files and asserts
the field sets match per tick type.

Run: `python3 scripts/check_first_row_schema_alignment.py`
Exits 1 on schema drift; 0 on alignment.

This is a static check -- no live data required. CI runs it in the
`surfaces` job alongside `validate_agreement_test.py`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLI_MAIN = ROOT / "tools" / "cli" / "src" / "main.rs"
PYTHON_COLUMNAR = ROOT / "sdks" / "python" / "src" / "tick_columnar.rs"

# Map render_* function name in CLI -> Python columnar function name.
# Defined explicitly because the names don't auto-derive (CLI uses
# `render_eod` / `render_ohlc` / `render_trades`; Python uses
# `eod_ticks_to_columnar` / `ohlc_ticks_to_columnar` / `trade_ticks_to_columnar`).
RENDERER_PAIRS: list[tuple[str, str]] = [
    ("render_eod", "eod_ticks_to_columnar"),
    ("render_ohlc", "ohlc_ticks_to_columnar"),
    ("render_trades", "trade_ticks_to_columnar"),
    ("render_quotes", "quote_ticks_to_columnar"),
    ("render_trade_quotes", "trade_quote_ticks_to_columnar"),
    ("render_open_interest", "open_interest_ticks_to_columnar"),
    ("render_market_value", "market_value_ticks_to_columnar"),
    ("render_greeks", "greeks_ticks_to_columnar"),
    ("render_iv", "iv_ticks_to_columnar"),
    ("render_price", "price_ticks_to_columnar"),
    ("render_calendar", "calendar_days_to_columnar"),
    ("render_interest_rates", "interest_rate_ticks_to_columnar"),
    ("render_option_contracts", "option_contracts_to_columnar"),
]


def extract_cli_raw_headers(source: str, fn_name: str) -> list[str]:
    """Pull the `set_raw_headers(vec![...])` field list out of a
    `fn fn_name(...)` block in main.rs."""
    fn_pattern = re.compile(
        rf"fn {re.escape(fn_name)}\b[^{{]*\{{",
    )
    fn_match = fn_pattern.search(source)
    if not fn_match:
        raise SystemExit(f"error: CLI function `{fn_name}` not found in {CLI_MAIN}")
    body = source[fn_match.end():]
    headers_match = re.search(
        r"set_raw_headers\(vec!\[\s*((?:\"[^\"]*\"\s*,?\s*)+)\s*\]\)",
        body,
    )
    if not headers_match:
        raise SystemExit(
            f"error: `{fn_name}` does not call `set_raw_headers(vec![...])`"
        )
    inner = headers_match.group(1)
    return re.findall(r'"([^"]+)"', inner)


def extract_python_columnar_keys(source: str, fn_name: str) -> list[str]:
    """Pull the `dict.set_item("key", ...)` calls out of a python
    columnar function in tick_columnar.rs, in declaration order."""
    fn_pattern = re.compile(
        rf"fn {re.escape(fn_name)}\b[^{{]*\{{",
    )
    fn_match = fn_pattern.search(source)
    if not fn_match:
        raise SystemExit(
            f"error: Python columnar function `{fn_name}` not found in {PYTHON_COLUMNAR}"
        )
    # Capture from the fn open brace through the matching close. Track
    # nesting -- the function body has many `dict.set_item(...)` calls
    # with parentheses. A simple brace counter is enough since strings
    # in this file don't contain braces.
    body_start = fn_match.end()
    depth = 1
    body_end = body_start
    for i, ch in enumerate(source[body_start:], start=body_start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                body_end = i
                break
    body = source[body_start:body_end]
    return re.findall(r'set_item\("([^"]+)"', body)


def main() -> int:
    cli_source = CLI_MAIN.read_text()
    python_source = PYTHON_COLUMNAR.read_text()

    failures: list[str] = []
    print(f"Checking schema alignment between:")
    print(f"  CLI:    {CLI_MAIN.relative_to(ROOT)}")
    print(f"  Python: {PYTHON_COLUMNAR.relative_to(ROOT)}\n")

    for cli_fn, py_fn in RENDERER_PAIRS:
        cli_headers = extract_cli_raw_headers(cli_source, cli_fn)
        py_keys = extract_python_columnar_keys(python_source, py_fn)
        if cli_headers == py_keys:
            print(f"  \u2713 {cli_fn:25s} == {py_fn:35s} ({len(cli_headers)} fields)")
        else:
            extra_in_cli = [k for k in cli_headers if k not in py_keys]
            extra_in_py = [k for k in py_keys if k not in cli_headers]
            misorderings = (
                set(cli_headers) == set(py_keys)
                and cli_headers != py_keys
            )
            failures.append(
                f"\n  \u2717 {cli_fn} schema diverges from {py_fn}:\n"
                f"      CLI:    {cli_headers}\n"
                f"      Python: {py_keys}\n"
                f"      extra in CLI: {extra_in_cli}\n"
                f"      extra in Python: {extra_in_py}\n"
                + (
                    "      (same field set but different ordering)\n"
                    if misorderings
                    else ""
                )
            )

    if failures:
        print("\nSCHEMA DRIFT:", file=sys.stderr)
        for f in failures:
            print(f, file=sys.stderr)
        return 1
    print(f"\n\u2713 all {len(RENDERER_PAIRS)} renderer pairs aligned")
    return 0


if __name__ == "__main__":
    sys.exit(main())
