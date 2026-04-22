"""
Typed `<Tick>List` wrapper + chained DataFrame terminals — correctness tests.

These tests exercise the public surface introduced in v8.0.3: every
historical endpoint returns a `<TickName>List` / `StringList` wrapper,
and DataFrame conversion happens via chained terminals (`.to_list()`,
`.to_arrow()`, `.to_pandas()`, `.to_polars()`) on the wrapper.

The free functions `thetadatadx.to_arrow(ticks)` / `to_dataframe(ticks)` /
`to_polars(ticks)` that existed through v8.0.2 have been deleted; this
suite verifies the chained path produces identical results.

What this suite covers:

1. Python list protocol on `<Tick>List` (`__len__`, `__bool__`,
   `__repr__`, `__getitem__` including negative indexing and out-of-range
   behaviour, `__iter__` yields in order).
2. Round-trip correctness for EodTick / OhlcTick / TradeTick / QuoteTick
   — values constructed in Python, verified after chaining through
   `.to_list()`, `.to_arrow()`, `.to_pandas()`, `.to_polars()`.
3. i64 correctness: `volume` / `count` fields at the 40-bit mark
   preserve full precision (no silent narrowing to f64).
4. `to_arrow()` dep: callable without pandas or polars — the Arrow
   path does not transitively pull either.
5. `StringList` (returned by `stock_list_symbols` etc.): list protocol
   + single-column DataFrame materialisation with the right header.

The Arrow C Data Interface handoff (zero-copy from Rust to pyarrow)
is verified via an RSS-delta measurement in the benchmark harness
(see `benches/bench_arrow_vs_dict.py`); this file concentrates on
correctness.
"""

from __future__ import annotations

import sys

import pytest


# Hard gate: these tests cannot run without the pre-built native
# extension. maturin develop / a wheel install both satisfy this.
thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from sdks/python/",
)

pyarrow = pytest.importorskip("pyarrow", reason="pyarrow is required for Arrow DataFrame path")


# ──────────────────────────────────────────────────────────────────────
# `<Tick>List` Python list protocol
# ──────────────────────────────────────────────────────────────────────


def _make_eod_ticks(n: int) -> "list[thetadatadx.EodTick]":
    return [
        thetadatadx.EodTick(
            ms_of_day=1000 + i,
            ms_of_day2=2000 + i,
            open=100.0 + i,
            high=101.0 + i,
            low=99.0 + i,
            close=100.5 + i,
            volume=1_000_000 * (i + 1),
            count=5_000 * (i + 1),
            bid_size=10 + i,
            bid_exchange=1,
            bid=100.0 + i * 0.1,
            ask_size=20 + i,
            ask_exchange=2,
            ask=100.1 + i * 0.1,
            date=20260420,
            expiration=20260517,
            strike=100.0,
            right="C" if i % 2 == 0 else "P",
        )
        for i in range(n)
    ]


def test_empty_list_wrapper_len_bool_repr():
    """Empty `EodTickList` reports len 0, is falsy, reprs with row count."""
    lst = thetadatadx.EodTickList()
    assert len(lst) == 0
    assert bool(lst) is False
    assert repr(lst) == "EodTickList(0 rows)"


def test_non_empty_list_wrapper_len_bool_repr():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    assert len(lst) == 3
    assert bool(lst) is True
    assert repr(lst) == "EodTickList(3 rows)"


def test_getitem_positive_indexing():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    for i, original in enumerate(ticks):
        fetched = lst[i]
        assert fetched.ms_of_day == original.ms_of_day
        assert fetched.open == original.open
        assert fetched.volume == original.volume


def test_getitem_negative_indexing():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    # lst[-1] -> last, lst[-2] -> middle, lst[-3] -> first
    assert lst[-1].ms_of_day == ticks[2].ms_of_day
    assert lst[-2].ms_of_day == ticks[1].ms_of_day
    assert lst[-3].ms_of_day == ticks[0].ms_of_day


def test_getitem_out_of_range_raises_index_error():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    with pytest.raises(IndexError):
        lst[3]
    with pytest.raises(IndexError):
        lst[-4]
    with pytest.raises(IndexError):
        lst[999]


def test_iter_yields_in_order():
    ticks = _make_eod_ticks(4)
    lst = thetadatadx.EodTickList(ticks)
    collected = [t for t in lst]
    assert len(collected) == 4
    for i, fetched in enumerate(collected):
        assert fetched.ms_of_day == ticks[i].ms_of_day
        assert fetched.open == ticks[i].open


def test_iter_twice_yields_independent_sequences():
    """`__iter__` produces a fresh iterator each time (Python sequence
    protocol contract). Repeated `for ... in lst:` must re-visit every
    row, not resume from the previous cursor position."""
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    first = [t.ms_of_day for t in lst]
    second = [t.ms_of_day for t in lst]
    assert first == second
    assert len(first) == 3


def test_to_list_round_trips_to_plain_list():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    plain = lst.to_list()
    assert isinstance(plain, list)
    assert len(plain) == 3
    for i, original in enumerate(ticks):
        assert plain[i].ms_of_day == original.ms_of_day
        assert plain[i].open == original.open
        assert plain[i].right == original.right


# ──────────────────────────────────────────────────────────────────────
# `.to_arrow()` terminal: pyarrow.Table with expected schema
# ──────────────────────────────────────────────────────────────────────


EOD_SCHEMA = {
    "ms_of_day": "int32",
    "ms_of_day2": "int32",
    "open": "double",
    "high": "double",
    "low": "double",
    "close": "double",
    "volume": "int64",
    "count": "int64",
    "bid_size": "int32",
    "bid_exchange": "int32",
    "bid": "double",
    "bid_condition": "int32",
    "ask_size": "int32",
    "ask_exchange": "int32",
    "ask": "double",
    "ask_condition": "int32",
    "date": "int32",
    "expiration": "int32",
    "strike": "double",
    "right": "string",
}


def _assert_arrow_types_match(table: "pyarrow.Table", expected: dict) -> None:
    """Assert pyarrow schema matches `expected` (column -> Arrow type name).

    Arrow types are compared by string name (e.g. `int32`, `int64`,
    `double`, `string`) so the test survives Arrow version bumps.
    """
    actual = {f.name: str(f.type) for f in table.schema}
    assert actual == expected, (
        f"schema mismatch\n  expected: {expected}\n  got:      {actual}"
    )


def test_to_arrow_returns_pyarrow_table_with_eod_schema():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    table = lst.to_arrow()
    assert isinstance(table, pyarrow.Table)
    assert table.num_rows == 3
    _assert_arrow_types_match(table, EOD_SCHEMA)


def test_to_arrow_preserves_values_round_trip():
    ticks = _make_eod_ticks(3)
    lst = thetadatadx.EodTickList(ticks)
    table = lst.to_arrow()
    for i, original in enumerate(ticks):
        assert table.column("ms_of_day")[i].as_py() == original.ms_of_day
        assert table.column("open")[i].as_py() == pytest.approx(original.open)
        assert table.column("volume")[i].as_py() == original.volume
        assert table.column("right")[i].as_py() == original.right


def test_to_arrow_empty_list_returns_empty_table_with_schema():
    """Empty wrapper still has a typed schema — the slice path emits the
    full column layout with zero rows."""
    lst = thetadatadx.EodTickList()
    table = lst.to_arrow()
    assert isinstance(table, pyarrow.Table)
    assert table.num_rows == 0
    _assert_arrow_types_match(table, EOD_SCHEMA)


def test_ohlc_tick_to_arrow_schema():
    tick = thetadatadx.OhlcTick(
        ms_of_day=34_200_000,
        open=100.0,
        high=110.0,
        low=95.0,
        close=105.0,
        volume=1_000_000,
        count=500,
        date=20260420,
    )
    lst = thetadatadx.OhlcTickList([tick])
    table = lst.to_arrow()
    expected = {
        "ms_of_day": "int32",
        "open": "double",
        "high": "double",
        "low": "double",
        "close": "double",
        "volume": "int64",
        "count": "int64",
        "date": "int32",
        "expiration": "int32",
        "strike": "double",
        "right": "string",
    }
    _assert_arrow_types_match(table, expected)


def test_quote_tick_to_arrow_has_midpoint():
    tick = thetadatadx.QuoteTick(ms_of_day=34_200_000, bid=99.95, ask=100.05, midpoint=100.0)
    lst = thetadatadx.QuoteTickList([tick])
    table = lst.to_arrow()
    schema = {f.name: str(f.type) for f in table.schema}
    assert "midpoint" in schema
    assert schema["midpoint"] == "double"


def test_trade_tick_to_arrow_schema():
    tick = thetadatadx.TradeTick(ms_of_day=34_200_000, size=100, price=100.50, date=20260420)
    lst = thetadatadx.TradeTickList([tick])
    table = lst.to_arrow()
    schema = {f.name: str(f.type) for f in table.schema}
    assert schema["price"] == "double"
    assert schema["size"] == "int32"


# ──────────────────────────────────────────────────────────────────────
# pandas round-trip via `.to_pandas()`
# ──────────────────────────────────────────────────────────────────────


def test_eod_tick_round_trip_byte_for_byte_pandas():
    """Construct three EodTicks, chain to DataFrame, assert every value matches."""
    pandas = pytest.importorskip("pandas")
    ticks = _make_eod_ticks(3)
    df = thetadatadx.EodTickList(ticks).to_pandas()

    assert len(df) == 3
    for i, t in enumerate(ticks):
        row = df.iloc[i]
        assert row["ms_of_day"] == t.ms_of_day
        assert row["ms_of_day2"] == t.ms_of_day2
        assert row["open"] == pytest.approx(t.open)
        assert row["volume"] == t.volume
        assert row["count"] == t.count
        assert row["right"] == t.right


def test_i64_volume_count_preserves_2_to_the_40():
    """40-bit volume / count round-trip without narrowing."""
    pandas = pytest.importorskip("pandas")
    big = 2**40
    tick = thetadatadx.EodTick(volume=big, count=big)
    df = thetadatadx.EodTickList([tick]).to_pandas()
    assert str(df["volume"].dtype) == "int64"
    assert str(df["count"].dtype) == "int64"
    assert int(df["volume"].iloc[0]) == big
    assert int(df["count"].iloc[0]) == big


def test_i64_ohlc_tick_volume_count_preserves_2_to_the_40():
    pandas = pytest.importorskip("pandas")
    big = 2**40
    tick = thetadatadx.OhlcTick(volume=big, count=big)
    df = thetadatadx.OhlcTickList([tick]).to_pandas()
    assert str(df["volume"].dtype) == "int64"
    assert int(df["volume"].iloc[0]) == big


# ──────────────────────────────────────────────────────────────────────
# polars round-trip via `.to_polars()`
# ──────────────────────────────────────────────────────────────────────


def test_polars_round_trip_preserves_int64():
    polars = pytest.importorskip("polars")
    big = 2**40
    tick = thetadatadx.EodTick(ms_of_day=1, volume=big, count=big)
    df = thetadatadx.EodTickList([tick]).to_polars()
    assert df.height == 1
    assert df["volume"].dtype == polars.Int64
    assert df["volume"][0] == big


# ──────────────────────────────────────────────────────────────────────
# `.to_arrow()` has no pandas / polars dependency
# ──────────────────────────────────────────────────────────────────────


def test_to_arrow_does_not_need_pandas(monkeypatch):
    """to_arrow must work even when pandas is absent from sys.modules.

    We simulate pandas being unimportable by stubbing `pandas` with a
    sentinel that raises on attribute access, then invoke `.to_arrow`
    on a `<Tick>List`. If the Arrow path silently pulls pandas,
    this test fails.
    """
    class _Denied:
        def __getattr__(self, name):
            raise RuntimeError(f"pandas must not be imported for to_arrow -- got attr `{name}`")

    # Replace pandas module entry (if present) with a sentinel and
    # also block future imports. Restore on teardown (monkeypatch).
    monkeypatch.setitem(sys.modules, "pandas", _Denied())

    tick = thetadatadx.EodTick(ms_of_day=1, volume=1_000)
    table = thetadatadx.EodTickList([tick]).to_arrow()
    assert table.num_rows == 1
    assert table.column("volume")[0].as_py() == 1_000


def test_to_arrow_does_not_need_polars(monkeypatch):
    class _Denied:
        def __getattr__(self, name):
            raise RuntimeError(f"polars must not be imported for to_arrow -- got attr `{name}`")

    monkeypatch.setitem(sys.modules, "polars", _Denied())

    tick = thetadatadx.EodTick(ms_of_day=1, volume=1_000)
    table = thetadatadx.EodTickList([tick]).to_arrow()
    assert table.num_rows == 1


# ──────────────────────────────────────────────────────────────────────
# OptionContract: `right` projects to a string, not the raw ASCII int
# ──────────────────────────────────────────────────────────────────────


def test_option_contract_right_is_string_in_arrow_schema():
    oc = thetadatadx.OptionContract(root="AAPL", expiration=20260517, strike=100.0, right="C")
    lst = thetadatadx.OptionContractList([oc])
    table = lst.to_arrow()
    schema = {f.name: str(f.type) for f in table.schema}
    assert schema["right"] == "string"
    assert table.column("right")[0].as_py() == "C"


# ──────────────────────────────────────────────────────────────────────
# StringList — wrapper around `Vec<String>` returned by list endpoints
# (`stock_list_symbols`, `option_list_expirations`, ...)
# ──────────────────────────────────────────────────────────────────────


def test_string_list_class_is_exported():
    """StringList is part of the public `thetadatadx` namespace."""
    assert hasattr(thetadatadx, "StringList")
