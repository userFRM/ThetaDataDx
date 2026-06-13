"""Typed-surface contract tests.

Pins the cross-binding type semantics on the Python surface:

* Greeks rows expose the keyword-colliding column as ``lambda_``
  (PEP 8 trailing underscore) — reachable with literal attribute
  syntax, while the Arrow / pandas column keeps the logical name
  ``lambda``.
* The fluent ``Contract`` builder takes the strike in dollars as a
  number or string, and reads the same dollar value back.
* ``CalendarDay.status`` carries the vendor day-type vocabulary and
  ``is_open`` is a boolean.
* Absent contract identity on historical rows is ``None`` (the same
  convention the streaming ``ContractRef`` uses), and populated
  identity round-trips through Arrow as nullable columns.
* Rows with ``date`` + milliseconds-of-day expose epoch-millisecond
  convenience properties; raw integer fields stay primary.
"""

from __future__ import annotations

import pytest

thetadatadx = pytest.importorskip(
    "thetadatadx",
    reason="native thetadatadx extension not built -- run `maturin develop` from sdks/python/",
)


# ──────────────────────────────────────────────────────────────────────
# lambda_ keyword escape (Greeks rows)
# ──────────────────────────────────────────────────────────────────────


def test_greeks_lambda_attribute_is_reachable_with_literal_syntax():
    tick = thetadatadx.GreeksAllTick(lambda_=0.25, delta=0.5)
    # Literal attribute syntax — the entire point of the escape.
    assert tick.lambda_ == 0.25
    assert getattr(tick, "lambda_") == 0.25
    # The unescaped spelling is not an attribute (it was never
    # reachable as one; `tick.lambda` is a SyntaxError by grammar).
    assert not hasattr(tick, "lambda")


@pytest.mark.parametrize(
    "cls",
    [
        "GreeksAllTick",
        "GreeksEodTick",
        "GreeksFirstOrderTick",
        "TradeGreeksAllTick",
        "TradeGreeksFirstOrderTick",
    ],
)
def test_every_greeks_class_exposes_lambda_underscore(cls: str):
    tick = getattr(thetadatadx, cls)(lambda_=1.5)
    assert tick.lambda_ == 1.5


def test_lambda_arrow_column_keeps_logical_name():
    pyarrow = pytest.importorskip("pyarrow")
    tick = thetadatadx.GreeksAllTick(lambda_=0.25)
    table = thetadatadx.GreeksAllTickList([tick]).to_arrow()
    assert isinstance(table, pyarrow.Table)
    # Column names are strings, not identifiers — the logical name stays.
    assert "lambda" in table.schema.names
    assert "lambda_" not in table.schema.names
    assert table.column("lambda")[0].as_py() == 0.25


# ──────────────────────────────────────────────────────────────────────
# Strike in dollars on the fluent builder
# ──────────────────────────────────────────────────────────────────────


@pytest.mark.parametrize("strike", [550, 550.0, "550"])
def test_contract_option_strike_accepts_number_or_string(strike):
    option = thetadatadx.Contract.option(
        "SPY", expiration="20260618", strike=strike, right="C"
    )
    # Reads back the dollar value it was given — never the wire integer.
    assert option.strike == 550.0


def test_contract_option_strike_preserves_cents():
    option = thetadatadx.Contract.option(
        "SPX", expiration="20260618", strike="5400.50", right="P"
    )
    assert option.strike == 5400.5
    assert option.expiration == 20260618
    assert option.right == "P"


def test_contract_stock_has_no_option_identity():
    stock = thetadatadx.Contract.stock("AAPL")
    assert stock.strike is None
    assert stock.expiration is None
    assert stock.right is None


# ──────────────────────────────────────────────────────────────────────
# Calendar day vocabulary
# ──────────────────────────────────────────────────────────────────────


def test_calendar_day_status_carries_vendor_vocabulary():
    day = thetadatadx.CalendarDay(
        date=20260102, is_open=True, status="early_close"
    )
    assert day.is_open is True
    assert day.status == "early_close"


def test_calendar_day_defaults_to_closed():
    day = thetadatadx.CalendarDay()
    assert day.is_open is False
    assert day.status == "full_close"


def test_calendar_day_list_rejects_unknown_status_text():
    day = thetadatadx.CalendarDay(date=20260102, status="open")
    thetadatadx.CalendarDayList([day])  # vocabulary value: accepted
    bad = thetadatadx.CalendarDay(date=20260102, status="half_day")
    with pytest.raises(ValueError):
        thetadatadx.CalendarDayList([bad])


def test_calendar_day_arrow_status_is_string_and_is_open_boolean():
    pyarrow = pytest.importorskip("pyarrow")
    day = thetadatadx.CalendarDay(date=20260102, is_open=True, status="open")
    table = thetadatadx.CalendarDayList([day]).to_arrow()
    schema = {f.name: str(f.type) for f in table.schema}
    assert schema["status"] == "string"
    assert schema["is_open"] == "bool"
    assert table.column("status")[0].as_py() == "open"
    assert table.column("is_open")[0].as_py() is True


# ──────────────────────────────────────────────────────────────────────
# Absent contract identity is None
# ──────────────────────────────────────────────────────────────────────


def test_absent_contract_identity_is_none_on_rows():
    tick = thetadatadx.TradeTick(ms_of_day=34_200_000, price=1.5, date=20260420)
    assert tick.expiration is None
    assert tick.strike is None
    assert tick.right is None


def test_populated_contract_identity_round_trips():
    tick = thetadatadx.TradeTick(
        ms_of_day=34_200_000,
        price=1.5,
        date=20260420,
        expiration=20260618,
        strike=550.0,
        right="C",
    )
    assert tick.expiration == 20260618
    assert tick.strike == 550.0
    assert tick.right == "C"


def test_contract_identity_arrow_columns_are_nullable():
    pyarrow = pytest.importorskip("pyarrow")
    absent = thetadatadx.TradeTick(ms_of_day=1, price=1.0, date=20260420)
    present = thetadatadx.TradeTick(
        ms_of_day=2,
        price=2.0,
        date=20260420,
        expiration=20260618,
        strike=550.0,
        right="P",
    )
    table = thetadatadx.TradeTickList([absent, present]).to_arrow()
    assert table.column("expiration")[0].as_py() is None
    assert table.column("strike")[0].as_py() is None
    assert table.column("right")[0].as_py() is None
    assert table.column("expiration")[1].as_py() == 20260618
    assert table.column("strike")[1].as_py() == 550.0
    assert table.column("right")[1].as_py() == "P"


# ──────────────────────────────────────────────────────────────────────
# EOD time semantics + epoch accessors
# ──────────────────────────────────────────────────────────────────────


def test_eod_tick_time_fields_carry_vendor_semantics():
    tick = thetadatadx.EodTick(
        created_ms_of_day=62_273_606,
        last_trade_ms_of_day=57_300_000,
        date=20240102,
    )
    assert tick.created_ms_of_day == 62_273_606
    assert tick.last_trade_ms_of_day == 57_300_000


def test_timestamp_ms_property_combines_date_and_ms():
    # 2026-01-15 09:30:00 ET (EST, UTC-5) == 1_768_487_400_000 epoch ms.
    tick = thetadatadx.TradeTick(ms_of_day=34_200_000, price=1.0, date=20260115)
    assert tick.timestamp_ms == 1_768_487_400_000


def test_timestamp_ms_property_is_none_when_date_absent():
    tick = thetadatadx.TradeTick(ms_of_day=34_200_000, price=1.0)
    assert tick.timestamp_ms is None


def test_eod_tick_exposes_created_and_last_trade_timestamps():
    tick = thetadatadx.EodTick(
        created_ms_of_day=62_100_000,
        last_trade_ms_of_day=57_600_000,
        date=20260115,
    )
    base = 1_768_453_200_000  # 2026-01-15 00:00:00 ET in epoch ms (EST).
    assert tick.created_timestamp_ms == base + 62_100_000
    assert tick.last_trade_timestamp_ms == base + 57_600_000


# ──────────────────────────────────────────────────────────────────────
# ParseError event class name
# ──────────────────────────────────────────────────────────────────────


def test_parse_error_event_class_is_exported():
    assert hasattr(thetadatadx, "ParseError")
    # No class answers to the bare `Error` name anymore — the event
    # payload is `ParseError` and the exception tree roots at
    # `ThetaDataError`.
    assert "Error" not in dir(thetadatadx)
    assert issubclass(thetadatadx.ThetaDataError, Exception)


# ──────────────────────────────────────────────────────────────────────
# TradeTick flag-word accessors
# ──────────────────────────────────────────────────────────────────────


def test_trade_tick_flag_accessors_decode_condition_words():
    """Boolean accessors decode the integer condition / flag columns so a
    caller never hand-decodes `condition_flags` / `price_flags`."""
    fired = thetadatadx.TradeTick(
        ms_of_day=40_000_000,  # within 9:30-16:00 ET
        condition=42,  # cancelled-trade range 40-44
        condition_flags=1,  # NO_LAST bit
        price_flags=1,  # SET_LAST bit
        volume_type=0,  # incremental
        ext_condition1=12,  # seller
    )
    assert fired.is_cancelled is True
    assert fired.regular_trading_hours is True
    assert fired.trade_condition_no_last is True
    assert fired.price_condition_set_last is True
    assert fired.is_incremental_volume is True
    assert fired.is_seller is True

    quiet = thetadatadx.TradeTick(
        ms_of_day=1_000,  # before the open
        condition=5,
        condition_flags=0,
        price_flags=0,
        volume_type=1,  # cumulative
        ext_condition1=0,
    )
    assert quiet.is_cancelled is False
    assert quiet.regular_trading_hours is False
    assert quiet.trade_condition_no_last is False
    assert quiet.price_condition_set_last is False
    assert quiet.is_incremental_volume is False
    assert quiet.is_seller is False


# ──────────────────────────────────────────────────────────────────────
# Offline-analytics return shapes (stub drift guard)
#
# stubtest reads the compiled extension's free functions and frozen
# pyclasses as opaque ``builtin`` descriptors, so a wrong return
# annotation in the ``.pyi`` (e.g. ``-> float`` on a function that
# actually returns a 2-tuple, or a short field list on a frozen result
# class) sails through the gate. These cases call the real runtime and
# assert the concrete shape the stub now declares, so future drift on
# exactly these symbols is caught here instead of slipping silently.
# Inputs are in-the-money so the IV solver converges deterministically.
# ──────────────────────────────────────────────────────────────────────


_GREEKS_ARGS = dict(
    spot=100.0,
    strike=100.0,
    rate=0.05,
    div_yield=0.0,
    tte=0.5,
    option_price=7.0,
    right="C",
)


def test_implied_volatility_returns_iv_and_error_tuple():
    result = thetadatadx.implied_volatility(**_GREEKS_ARGS)
    # The stub declares ``tuple[float, float]`` — the runtime hands back
    # ``(iv, iv_error)``, not the bare ``iv`` float an older stub claimed.
    assert isinstance(result, tuple)
    assert len(result) == 2
    iv, iv_error = result
    assert isinstance(iv, float)
    assert isinstance(iv_error, float)
    assert iv > 0.0  # in-the-money call solves to a positive vol


# Every field the ``AllGreeks`` stub declares, in declaration order. The
# elasticity Greek carries the PEP 8 keyword escape ``lambda_`` (matching
# the ``GreeksAllTick.lambda_`` tick attribute), so it is a normal
# attribute like the rest — no ``getattr`` workaround.
_ALL_GREEKS_STUB_FIELDS = (
    "value",
    "iv",
    "iv_error",
    "delta",
    "gamma",
    "theta",
    "vega",
    "rho",
    "vanna",
    "charm",
    "vomma",
    "veta",
    "vera",
    "speed",
    "zomma",
    "color",
    "ultima",
    "d1",
    "d2",
    "dual_delta",
    "dual_gamma",
    "epsilon",
    "lambda_",
)


def test_all_greeks_exposes_every_stubbed_field_as_float():
    greeks = thetadatadx.all_greeks(**_GREEKS_ARGS)
    assert type(greeks).__name__ == "AllGreeks"
    for field in _ALL_GREEKS_STUB_FIELDS:
        value = getattr(greeks, field)
        assert isinstance(value, float), f"AllGreeks.{field} is not a float"
    # The elasticity Greek is reachable with ordinary attribute syntax via
    # its keyword escape, exactly like the tick-class ``lambda_`` field.
    assert isinstance(greeks.lambda_, float)


def test_all_greeks_runtime_fields_match_the_stub_exactly():
    """No runtime field is missing from the stub and the stub invents no
    field the runtime lacks (covering the ``lambda_`` keyword escape)."""
    greeks = thetadatadx.all_greeks(**_GREEKS_ARGS)
    runtime_fields = {
        name
        for name in dir(greeks)
        if not name.startswith("_") and not callable(getattr(greeks, name))
    }
    stubbed_fields = set(_ALL_GREEKS_STUB_FIELDS)
    assert runtime_fields == stubbed_fields
