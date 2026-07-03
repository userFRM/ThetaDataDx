"""Slicing a multi-symbol snapshot `<Tick>List` keeps each row's OWN symbol.

A multi-symbol snapshot (`stock_snapshot_quote(["AAPL", "MSFT"])`) carries one
`symbol` value per row on the decoded list. Slicing or reversing the list must
carry the correspondingly-permuted per-row symbols, not the response-order ones:

  * `lst[::-1]` reverses the rows; the symbol column must reverse with them,
    else each row is attributed to a neighbour's underlying (silent wrong data).
  * `lst[0:1]` keeps one row; the symbol column must shrink to match, else the
    per-row length (2) mismatches the row count (1) and `.to_arrow()` raises.

Driven through the offline `decode_response_bytes` hook (same
decode -> WireColumns -> per-row-symbol -> `<Tick>List` chain a live endpoint
uses) over a synthesised two-row (AAPL, MSFT) `stock_snapshot_quote` response.
"""

from __future__ import annotations

import importlib

import pytest

pytest.importorskip("pyarrow")

thetadatadx = importlib.import_module("thetadatadx")

# A two-row `stock_snapshot_quote` `ResponseData` (identity compression):
#   row 0: root=AAPL, ms_of_day=34_200_000, bid=10_000, ask=10_100
#   row 1: root=MSFT, ms_of_day=34_200_000, bid=20_000, ask=20_100
# Generated from `thetadatadx::wire` proto types; `response_symbol` classifies
# it PerRow(["AAPL", "MSFT"]).
RESPONSE_HEX = (
    "0a670a04726f6f740a096d735f6f665f6461790a036269640a0361736b0a0464617465"
    "12200a060a044141504c0a0510c0b3a7100a0310904e0a0310f44e0a051093cad40912"
    "220a060a044d5346540a0510c0b3a7100a0410a09c010a0410849d010a051093cad409"
    "12001867"
)


def _snapshot_list():
    chunk = bytes.fromhex(RESPONSE_HEX)
    lst = thetadatadx.decode_response_bytes("stock_snapshot_quote", [chunk])
    assert len(lst) == 2, "fixture decodes to two rows"
    # Sanity: full list attributes each row to its own underlying.
    full = lst.to_arrow().to_pydict()
    assert full["symbol"] == ["AAPL", "MSFT"]
    assert full["bid"] == [10_000.0, 20_000.0]
    return lst


def test_reversed_slice_keeps_each_rows_symbol():
    lst = _snapshot_list()
    rev = lst[::-1].to_arrow().to_pydict()
    # Rows reversed -> the symbol column must reverse alongside the data, so
    # each row still carries the symbol of the bid it sits next to.
    assert rev["bid"] == [20_000.0, 10_000.0]
    assert rev["symbol"] == ["MSFT", "AAPL"], (
        "reversed slice misattributed the per-row symbol"
    )


def test_subset_slice_shrinks_symbol_column_without_error():
    lst = _snapshot_list()
    # A shorter subset must not leave a length-2 symbol column against a
    # length-1 row set (that raises inside Arrow's RecordBatch on main).
    head = lst[0:1].to_arrow().to_pydict()
    assert head["bid"] == [10_000.0]
    assert head["symbol"] == ["AAPL"]

    tail = lst[1:2].to_arrow().to_pydict()
    assert tail["bid"] == [20_000.0]
    assert tail["symbol"] == ["MSFT"]
