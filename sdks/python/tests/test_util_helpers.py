"""Cross-language utility helpers — Python binding parity tests.

Verifies the `thetadatadx.util` submodule exposes the same lookup surface
as the Rust core (`thetadatadx::utils::{conditions, exchange, sequences}`).
Reference values mirror the Rust unit tests in
`crates/thetadatadx/src/tdbe/conditions/mod.rs` and
`crates/thetadatadx/src/tdbe/exchange.rs`, so cross-language drift trips this test.
"""

from __future__ import annotations

import importlib

import pytest


@pytest.fixture(scope="module")
def util():
    """Load the native module's `util` submodule, skipping if unavailable."""
    try:
        thetadatadx = importlib.import_module("thetadatadx")
    except ImportError:
        pytest.skip("native module thetadatadx not built")
    util_mod = getattr(thetadatadx, "util", None)
    if util_mod is None:
        pytest.skip("thetadatadx.util submodule not registered")
    return util_mod


def test_condition_name_known_codes(util):
    # Mirrors crates/thetadatadx/src/tdbe/conditions/mod.rs: condition_name(0) == "REGULAR",
    # (40) == "CANC", (148) == "EXTENDEDHOURSTRADE".
    assert util.condition_name(0) == "REGULAR"
    assert util.condition_name(40) == "CANC"


def test_condition_name_out_of_range(util):
    assert util.condition_name(-1) == "UNKNOWN"
    assert util.condition_name(9999) == "UNKNOWN"


def test_exchange_name_known_codes(util):
    # Mirrors crates/thetadatadx/src/tdbe/exchange.rs: exchange_name(0) == "Composite",
    # (3) == "NewYorkStockExchange".
    assert util.exchange_name(0) == "Composite"
    assert util.exchange_name(3) == "NewYorkStockExchange"
    assert util.exchange_symbol(3) == "NYSE"
    assert util.exchange_symbol(5) == "CBOE"


def test_exchange_out_of_range(util):
    assert util.exchange_name(-1) == "UNKNOWN"
    assert util.exchange_symbol(9999) == "UNKNOWN"


def test_calendar_status_name(util):
    # Vocabulary from the core CalendarStatus enum / the C ABI
    # thetadatadx_calendar_status_name. Out-of-table codes return "UNKNOWN".
    assert util.calendar_status_name(0) == "open"
    assert util.calendar_status_name(1) == "early_close"
    assert util.calendar_status_name(2) == "full_close"
    assert util.calendar_status_name(3) == "weekend"
    assert util.calendar_status_name(99) == "UNKNOWN"
    assert util.calendar_status_name(-1) == "UNKNOWN"


def test_timestamp_ms(util):
    # Combines an Eastern-Time YYYYMMDD date + ms-of-day into epoch ms.
    # 2024-01-02 09:30 ET = 14:30 UTC. Matches the TypeScript
    # Util.timestampMs and the C++ thetadatadx::timestamp_ms for the same input.
    assert util.timestamp_ms(20240102, 34_200_000) == 1_704_205_800_000


def test_timestamp_ms_out_of_domain_returns_none(util):
    # Out-of-domain inputs return None (the std::nullopt contract the C++
    # thetadatadx::timestamp_ms shares), never a coerced sentinel.
    assert util.timestamp_ms(0, 0) is None
    assert util.timestamp_ms(20240102, -1) is None


def test_quote_condition_lookups(util):
    # quote_condition_name(0) is well-defined in the table; out-of-range
    # falls back to "UNKNOWN".
    assert isinstance(util.quote_condition_name(0), str)
    assert util.quote_condition_name(-1) == "UNKNOWN"


def test_sequence_round_trip(util):
    # Sequence helpers are bidirectional. The Rust `signed_to_unsigned`
    # / `unsigned_to_signed` pair is a `transmute`-equivalent reinterpret;
    # round-trip through both directions returns the original input.
    for s in (0, 1, -1, 1_000_000, -1_000_000):
        u = util.sequence_signed_to_unsigned(s)
        assert util.sequence_unsigned_to_signed(u) == s


def test_sequence_wire_range_boundaries_round_trip(util):
    # The wire domain is the i32 cycle for the signed direction and
    # 0 ..= 2^32 - 1 for the unsigned direction; the boundary values
    # convert without raising.
    assert util.sequence_signed_to_unsigned(2_147_483_647) is not None
    assert util.sequence_signed_to_unsigned(-2_147_483_648) is not None
    assert util.sequence_unsigned_to_signed(4_294_967_295) is not None


def test_sequence_out_of_wire_range_raises_value_error(util):
    # A representable integer outside the wire domain is a rejected value,
    # not a silent reinterpret. It must raise ValueError, matching the
    # TypeScript InvalidParameterError / C++ InvalidParameterError for the
    # same input. Before the fix, sequence_unsigned_to_signed(2^32)
    # returned 0.
    with pytest.raises(ValueError):
        util.sequence_signed_to_unsigned(2_147_483_648)  # i32::MAX + 1
    with pytest.raises(ValueError):
        util.sequence_signed_to_unsigned(-2_147_483_649)  # i32::MIN - 1
    with pytest.raises(ValueError):
        util.sequence_unsigned_to_signed(4_294_967_296)  # 2^32


def test_sequence_representation_overflow_stays_overflow_error(util):
    # A value that does not fit the parameter's own integer type is a
    # representation overflow, surfaced by pyo3 argument coercion as the
    # built-in OverflowError. This is intentionally distinct from the
    # wire-range ValueError above and must not be reclassified: a negative
    # passed into the u64 parameter overflows before the wire-range check
    # can run.
    with pytest.raises(OverflowError):
        util.sequence_unsigned_to_signed(-1)
