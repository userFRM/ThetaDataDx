"""Cross-language utility helpers — Python binding parity tests (issue #424).

Verifies the `thetadatadx.util` submodule exposes the same lookup surface
as the Rust core (`tdbe::{conditions, exchange, sequences}`). Reference
values mirror the Rust unit tests in
`crates/tdbe/src/conditions/mod.rs` and
`crates/tdbe/src/exchange.rs`, so cross-language drift trips this test.
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
    # Mirrors crates/tdbe/src/conditions/mod.rs: condition_name(0) == "REGULAR",
    # (40) == "CANC", (148) == "EXTENDEDHOURSTRADE".
    assert util.condition_name(0) == "REGULAR"
    assert util.condition_name(40) == "CANC"


def test_condition_name_out_of_range(util):
    assert util.condition_name(-1) == "UNKNOWN"
    assert util.condition_name(9999) == "UNKNOWN"


def test_exchange_name_known_codes(util):
    # Mirrors crates/tdbe/src/exchange.rs: exchange_name(0) == "Composite",
    # (3) == "NewYorkStockExchange".
    assert util.exchange_name(0) == "Composite"
    assert util.exchange_name(3) == "NewYorkStockExchange"
    assert util.exchange_symbol(3) == "NYSE"
    assert util.exchange_symbol(5) == "CBOE"


def test_exchange_out_of_range(util):
    assert util.exchange_name(-1) == "UNKNOWN"
    assert util.exchange_symbol(9999) == "UNKNOWN"


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
