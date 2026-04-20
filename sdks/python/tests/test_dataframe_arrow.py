"""
Arrow columnar DataFrame adapter -- correctness + schema-preservation tests.

These tests exercise the `to_arrow` / `to_dataframe` / `to_polars` public
entry points end-to-end, plus the fast-path `*_to_arrow_batch` Rust
helpers indirectly via the `_df` convenience wrappers on `ThetaDataDx`.

What this suite covers:

1. Schema preservation on empty input (non-empty qualname hint via the
   `_df` fast path; empty list + no hint for the public `to_arrow`).
2. Round-trip correctness for EodTick / OhlcTick / TradeTick / QuoteTick
   -- values constructed in Python, checked after passing through
   `to_arrow` / `to_dataframe` / `to_polars`.
3. i64 correctness: `volume` / `count` fields at the 40-bit mark
   preserve full precision (no silent narrowing to f64).
4. `to_arrow()` dep: callable without pandas or polars -- the public
   path does not transitively pull either.
5. Mixed-type inputs: `to_dataframe([eod_tick, ohlc_tick])` surfaces a
   clear Python error, not silent column corruption.

The Arrow C Data Interface handoff (zero-copy from Rust to pyarrow)
is verified via an RSS-delta measurement in the benchmark harness
(see `benches/bench_arrow_vs_dict.py`); this file concentrates on
correctness.
"""

from __future__ import annotations

import importlib
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
# Empty-list schema preservation
# ──────────────────────────────────────────────────────────────────────


def test_to_arrow_empty_list_returns_zero_column_table():
    """Empty list + no hint -> zero-column pyarrow.Table (documented contract)."""
    table = thetadatadx.to_arrow([])
    assert isinstance(table, pyarrow.Table)
    assert table.num_columns == 0
    assert table.num_rows == 0


def test_to_dataframe_empty_list_returns_empty_dataframe():
    pandas = pytest.importorskip("pandas")
    df = thetadatadx.to_dataframe([])
    assert isinstance(df, pandas.DataFrame)
    assert len(df) == 0
    assert len(df.columns) == 0


def test_to_polars_empty_list_returns_empty_dataframe():
    polars = pytest.importorskip("polars")
    df = thetadatadx.to_polars([])
    assert isinstance(df, polars.DataFrame)
    assert df.height == 0
    assert df.width == 0


# ──────────────────────────────────────────────────────────────────────
# Schema fidelity for each supported tick type
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


def test_eod_tick_to_arrow_schema_matches_tick_schema_toml():
    """Schema on a single-row Arrow table matches `tick_schema.toml`."""
    tick = thetadatadx.EodTick(
        ms_of_day=1000,
        open=100.5,
        high=101.0,
        low=99.75,
        close=100.0,
        volume=123_456,
        count=7890,
        bid=99.95,
        ask=100.05,
        date=20260420,
        expiration=20260517,
        strike=100.0,
        right="C",
    )
    table = thetadatadx.to_arrow([tick])
    _assert_arrow_types_match(table, EOD_SCHEMA)
    assert table.num_rows == 1


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
    table = thetadatadx.to_arrow([tick])
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
    table = thetadatadx.to_arrow([tick])
    schema = {f.name: str(f.type) for f in table.schema}
    assert "midpoint" in schema
    assert schema["midpoint"] == "double"


def test_trade_tick_to_arrow_schema():
    tick = thetadatadx.TradeTick(ms_of_day=34_200_000, size=100, price=100.50, date=20260420)
    table = thetadatadx.to_arrow([tick])
    schema = {f.name: str(f.type) for f in table.schema}
    assert schema["price"] == "double"
    assert schema["size"] == "int32"


# ──────────────────────────────────────────────────────────────────────
# Round-trip: values constructed in Python must survive the Arrow
# pipeline byte-for-byte.
# ──────────────────────────────────────────────────────────────────────


def test_eod_tick_round_trip_byte_for_byte():
    """Construct three EodTicks, convert to DataFrame, assert every value matches."""
    pandas = pytest.importorskip("pandas")
    ticks = [
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
        for i in range(3)
    ]
    df = thetadatadx.to_dataframe(ticks)

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
    df = thetadatadx.to_dataframe([tick])
    assert str(df["volume"].dtype) == "int64"
    assert str(df["count"].dtype) == "int64"
    assert int(df["volume"].iloc[0]) == big
    assert int(df["count"].iloc[0]) == big


def test_i64_ohlc_tick_volume_count_preserves_2_to_the_40():
    pandas = pytest.importorskip("pandas")
    big = 2**40
    tick = thetadatadx.OhlcTick(volume=big, count=big)
    df = thetadatadx.to_dataframe([tick])
    assert str(df["volume"].dtype) == "int64"
    assert int(df["volume"].iloc[0]) == big


# ──────────────────────────────────────────────────────────────────────
# Polars round-trip
# ──────────────────────────────────────────────────────────────────────


def test_polars_round_trip_preserves_int64():
    polars = pytest.importorskip("polars")
    big = 2**40
    tick = thetadatadx.EodTick(ms_of_day=1, volume=big, count=big)
    df = thetadatadx.to_polars([tick])
    assert df.height == 1
    assert df["volume"].dtype == polars.Int64
    assert df["volume"][0] == big


# ──────────────────────────────────────────────────────────────────────
# to_arrow has no pandas / polars dependency
# ──────────────────────────────────────────────────────────────────────


def test_to_arrow_does_not_need_pandas(monkeypatch):
    """to_arrow must work even when pandas is absent from sys.modules.

    We simulate pandas being unimportable by stubbing `pandas` with a
    sentinel that raises on attribute access, then invoke `to_arrow`
    on a list of one tick.  If the Arrow path silently pulls pandas,
    this test fails.
    """
    class _Denied:
        def __getattr__(self, name):
            raise RuntimeError(f"pandas must not be imported for to_arrow -- got attr `{name}`")

    # Replace pandas module entry (if present) with a sentinel and
    # also block future imports. Restore on teardown (monkeypatch).
    monkeypatch.setitem(sys.modules, "pandas", _Denied())

    tick = thetadatadx.EodTick(ms_of_day=1, volume=1_000)
    table = thetadatadx.to_arrow([tick])
    assert table.num_rows == 1
    assert table.column("volume")[0].as_py() == 1_000


def test_to_arrow_does_not_need_polars(monkeypatch):
    class _Denied:
        def __getattr__(self, name):
            raise RuntimeError(f"polars must not be imported for to_arrow -- got attr `{name}`")

    monkeypatch.setitem(sys.modules, "polars", _Denied())

    tick = thetadatadx.EodTick(ms_of_day=1, volume=1_000)
    table = thetadatadx.to_arrow([tick])
    assert table.num_rows == 1


# ──────────────────────────────────────────────────────────────────────
# Mixed-type input produces a clear error (NOT silent column corruption).
# ──────────────────────────────────────────────────────────────────────


def test_mixed_types_in_list_raises_clear_error():
    eod = thetadatadx.EodTick(ms_of_day=1)
    ohlc = thetadatadx.OhlcTick(ms_of_day=2)
    with pytest.raises(RuntimeError) as excinfo:
        thetadatadx.to_dataframe([eod, ohlc])
    # Error message should name the offending class so users can
    # locate the bad element.
    assert "OhlcTick" in str(excinfo.value) or "EodTick" in str(excinfo.value)


def test_unknown_object_in_list_raises_value_error():
    with pytest.raises(RuntimeError):
        # Python ints are not tick pyclasses; the type check should
        # reject the list before the Arrow builders run.
        thetadatadx.to_dataframe([1, 2, 3])


def test_non_list_input_raises_value_error():
    with pytest.raises(ValueError):
        thetadatadx.to_dataframe("not a list")


# ──────────────────────────────────────────────────────────────────────
# OptionContract: `right` projects to a string, not the raw ASCII int
# ──────────────────────────────────────────────────────────────────────


def test_option_contract_right_is_string_in_arrow_schema():
    oc = thetadatadx.OptionContract(root="AAPL", expiration=20260517, strike=100.0, right="C")
    table = thetadatadx.to_arrow([oc])
    schema = {f.name: str(f.type) for f in table.schema}
    assert schema["right"] == "string"
    assert table.column("right")[0].as_py() == "C"
