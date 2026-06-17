"""Regression test for the fluent `Contract` builder.

Asserts that the example documented in `fluent.rs:7-10`:

    from thetadatadx import Contract, SecType

    stock  = Contract.stock("AAPL")
    option = Contract.option("SPY", expiration="20260620", strike="550", right="C")

actually runs from a clean `import thetadatadx`. Prior to the
`Contract` → `ContractRef` rename on the FPSS-event-payload side, the
fluent builder was silently shadowed by the event class because both
pyclasses registered under the same Python name, which is
last-write-wins in pyo3.

If this test fails after a future codegen change, run
`cargo run -p thetadatadx --bin generate_sdk_surfaces --features config-file`
and rebuild the wheel (`maturin develop` from `sdks/python`).
"""

from __future__ import annotations

import importlib

import pytest


@pytest.fixture(scope="module")
def thetadatadx_mod():
    """The compiled thetadatadx module — must be importable for the test
    to mean anything."""
    return importlib.import_module("thetadatadx")


def test_contract_stock_factory(thetadatadx_mod) -> None:
    """`Contract.stock("AAPL")` returns a fluent builder, not the event
    payload."""
    contract = thetadatadx_mod.Contract.stock("AAPL")
    # The fluent builder lowers `.quote()` to a Subscription; the event
    # payload (now `ContractRef`) has no such method.
    sub = contract.quote()
    assert type(sub).__name__ == "Subscription"


def test_contract_option_factory(thetadatadx_mod) -> None:
    """`Contract.option(...)` returns a fluent builder with `.trade()`."""
    option = thetadatadx_mod.Contract.option(
        "SPY",
        expiration="20260620",
        strike="550",
        right="C",
    )
    sub = option.trade()
    assert type(sub).__name__ == "Subscription"


def test_sec_type_full_trades(thetadatadx_mod) -> None:
    """`SecType.OPTION.full_trades()` returns a full-stream Subscription."""
    sub = thetadatadx_mod.SecType.OPTION.full_trades()
    assert type(sub).__name__ == "Subscription"


def test_contract_class_is_fluent_builder(thetadatadx_mod) -> None:
    """`thetadatadx.Contract` must be the fluent builder, NOT the event
    payload. The event payload now lives at `thetadatadx.ContractRef`."""
    cls = thetadatadx_mod.Contract
    # The fluent builder has `stock`, `option`, `index`, `quote`, `trade`,
    # `open_interest` — none of which exist on the event-payload struct.
    for method in ("stock", "option", "index", "quote", "trade", "open_interest"):
        assert hasattr(cls, method), (
            f"thetadatadx.Contract is missing fluent method {method!r}; "
            f"is the event payload shadowing the fluent builder again?"
        )


def test_contract_ref_is_event_payload(thetadatadx_mod) -> None:
    """`thetadatadx.ContractRef` exists and carries the read-only event
    fields."""
    ref = thetadatadx_mod.ContractRef
    # Read-only event-payload fields surfaced on every FPSS data event.
    for attr in ("symbol", "sec_type", "expiration", "right", "strike"):
        assert hasattr(ref, attr), f"thetadatadx.ContractRef missing field {attr!r}"
    # The event payload has no factory methods.
    assert not hasattr(ref, "stock"), (
        "thetadatadx.ContractRef should not carry fluent factory methods; "
        "those live on `Contract`."
    )


def test_contract_sec_type_is_symbolic_string(thetadatadx_mod) -> None:
    """`Contract.sec_type` reads as a symbolic uppercase string, the same
    type the streaming `ContractRef.sec_type` event surface uses — one
    concept, one type across the whole surface."""
    stock = thetadatadx_mod.Contract.stock("AAPL")
    assert stock.sec_type == "STOCK"
    assert isinstance(stock.sec_type, str)

    option = thetadatadx_mod.Contract.option(
        "SPY",
        expiration="20260620",
        strike="550",
        right="C",
    )
    assert option.sec_type == "OPTION"
    assert isinstance(option.sec_type, str)

    index = thetadatadx_mod.Contract.index("SPX")
    assert index.sec_type == "INDEX"


def test_contract_str_renders_strike_in_dollars(thetadatadx_mod) -> None:
    """`str(Contract)` renders the strike in dollars, identical to the
    C++ `operator<<` / TypeScript `toString` / Rust `Display` surface — a
    `$550` strike reads `550`, never the wire-level `550000`."""
    option = thetadatadx_mod.Contract.option(
        "SPY",
        expiration="20260620",
        strike="550",
        right="C",
    )
    assert str(option) == "SPY OPTION 20260620 C 550"

    # Fractional strikes keep the needed decimals.
    fractional = thetadatadx_mod.Contract.option(
        "SPY",
        expiration="20260620",
        strike="552.5",
        right="P",
    )
    assert str(fractional) == "SPY OPTION 20260620 P 552.5"

    stock = thetadatadx_mod.Contract.stock("AAPL")
    assert str(stock) == "AAPL STOCK"
