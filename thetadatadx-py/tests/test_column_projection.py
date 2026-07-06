"""End-to-end column projection on the Python `<Tick>List` DataFrame terminals.

A real MDDS response decoded through the production decode path yields a
`<Tick>List` whose `.to_arrow()` carries only the columns that response's
wire sent (terminal-exact). The `decode_response_bytes` offline hook runs the
exact decode -> WireColumns::present_columns -> Ticks -> `<Tick>List` chain a
live endpoint uses, so feeding it a checked-in capture proves the projection
end to end without a network round-trip.

Two long-standing symptoms are one bug:
  * gRPC trade endpoints never send condition_flags / price_flags /
    volume_type / records_back -> they showed as always-0 columns.
  * equity / index endpoints never send expiration / strike / right -> they
    showed as always-null columns.
Both must be absent from the projected frame.
"""

from __future__ import annotations

import importlib
from pathlib import Path

import pytest

pytest.importorskip("pyarrow")
zstandard = pytest.importorskip("zstandard")

thetadatadx = importlib.import_module("thetadatadx")

CAPTURES = (
    Path(__file__).resolve().parents[2] / "thetadatadx-rs" / "tests" / "fixtures" / "captures"
)


def _response_bytes(endpoint: str) -> bytes:
    """Raw `ResponseData` proto bytes for a checked-in capture.

    Fixtures beginning with the zstd magic wrap the `ResponseData` in an
    outer zstd frame (matches the Rust `capture_loader` sniff); decompress it
    so `decode_response_bytes` sees the inner proto bytes.
    """
    raw = (CAPTURES / f"{endpoint}.pb.zst").read_bytes()
    if raw[:4] == b"\x28\xb5\x2f\xfd":
        return zstandard.ZstdDecompressor().decompress(raw)
    return raw


def _columns(endpoint: str):
    lst = thetadatadx.decode_response_bytes(endpoint, [_response_bytes(endpoint)])
    return lst, [f.name for f in lst.to_arrow().schema]


def test_stock_trade_quote_projects_out_flags_and_contract_id():
    lst, cols = _columns("stock_history_trade_quote")
    assert len(lst) > 0, "fixture is non-empty"
    for absent in (
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ):
        assert absent not in cols, f"stock trade_quote must not emit {absent}; got {cols}"
    for kept in ("ms_of_day", "quote_ms_of_day", "bid", "ask", "price"):
        assert kept in cols, f"missing {kept} in {cols}"


def test_option_trade_keeps_contract_id_drops_flags():
    _lst, cols = _columns("option_history_trade")
    for cid in ("expiration", "strike", "right"):
        assert cid in cols, f"option trade must keep {cid}; got {cols}"
    for flag in ("condition_flags", "price_flags", "volume_type", "records_back"):
        assert flag not in cols, f"no gRPC trade response carries {flag}; got {cols}"


def test_stock_eod_keeps_trading_date_and_drops_contract_id():
    lst, cols = _columns("stock_history_eod")
    for cid in ("expiration", "strike", "right"):
        assert cid not in cols, f"stock EOD must not carry {cid}; got {cols}"
    # The EOD wire sends one `created` Timestamp the schema splits into
    # `created_ms_of_day` (time) AND `date` (YYYYMMDD). Both must survive the
    # projection so the trading day is not lost: `created_ms_of_day` is a
    # ~16:00 ET near-constant across rows, so without `date` the rows are
    # indistinguishable. Mirrors the Rust `test_column_projection` stock-EOD
    # assertions (thetadatadx-rs/tests/test_column_projection.rs).
    assert "created_ms_of_day" in cols, f"got {cols}"
    assert "date" in cols, f"stock EOD must keep the trading `date`; got {cols}"
    dates = lst.to_arrow().column("date").to_pylist()
    assert dates == [20240102, 20240103, 20240104, 20240105], (
        f"date column must carry the distinct YYYYMMDD trading days; got {dates}"
    )


def test_hand_built_list_keeps_full_schema():
    # A List a caller assembles itself never touched a wire -> every column.
    lst = thetadatadx.TradeTickList()
    cols = [f.name for f in lst.to_arrow().schema]
    for every in (
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ):
        assert every in cols, f"hand-built list must keep {every}; got {cols}"
