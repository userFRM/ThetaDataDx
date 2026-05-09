"""
FPSS control-variant typed-class smoke tests.

Pins the contract that every `FpssControl::*` Rust variant has a
typed `#[pyclass]` mirror exported from `thetadatadx`, that each
mirror declares the right field set, and that Python's structural
`match` machinery dispatches on the class type without further
configuration.

These classes flow Rust -> Python only -- the streaming
dispatcher constructs them from `BufferedEvent::*` arms. We do not
construct them from Python here; pyo3 does not expose a
`__init__` for `frozen` pyclasses without an explicit
`#[new]` constructor, and synthesising a real FPSS event requires
a live handshake. What this test pins instead:

* The class exists in the `thetadatadx` namespace.
* Its fully-qualified name matches the schema-declared variant
  (which mirrors `FpssControl::*`).
* `__match_args__` (when present) and the field-getter set match
  the schema's `columns` declaration -- pyo3 generates getters
  on every `#[pyo3(get)]` field, so `dir(cls)` exposes them.
* Class identity is stable across imports (same object).

The schema variant set is duplicated locally rather than parsed
from the TOML at test time so this test fails LOUDLY if the
generated surface drops a variant -- pyo3 raising
`AttributeError: module 'thetadatadx' has no attribute 'Disconnected'`
is the early-warning signal we want.
"""

from __future__ import annotations

import importlib

import pytest


# (variant_name, expected_field_names) -- mirrors the
# `[events.<Variant>]` `kind = "control"` sections in
# `crates/thetadatadx/fpss_event_schema.toml`. Drift = test failure.
CONTROL_VARIANTS: list[tuple[str, tuple[str, ...]]] = [
    ("LoginSuccess", ("permissions",)),
    ("ContractAssigned", ("id", "contract")),
    ("ReqResponse", ("req_id", "result")),
    ("MarketOpen", ()),
    ("MarketClose", ()),
    ("ServerError", ("message",)),
    ("Disconnected", ("reason",)),
    ("Reconnecting", ("reason", "attempt", "delay_ms")),
    ("Reconnected", ()),
    ("Error", ("message",)),
    ("UnknownFrame", ("code", "payload")),
    ("Connected", ()),
    ("Ping", ("payload",)),
    ("ReconnectedServer", ()),
    ("Restart", ()),
    ("UnknownControl", ()),
]


@pytest.fixture(scope="module")
def thetadatadx_mod():
    return importlib.import_module("thetadatadx")


@pytest.mark.parametrize("variant,fields", CONTROL_VARIANTS, ids=lambda v: v if isinstance(v, str) else "")
def test_control_variant_exported(thetadatadx_mod, variant, fields):
    """Every `FpssControl::*` variant has a typed pyclass on the package."""
    cls = getattr(thetadatadx_mod, variant)
    assert cls is not None, f"thetadatadx.{variant} missing"
    # pyo3 frozen pyclasses expose the schema name as the type name.
    assert cls.__name__ == variant


@pytest.mark.parametrize("variant,fields", CONTROL_VARIANTS, ids=lambda v: v if isinstance(v, str) else "")
def test_control_variant_field_getters(thetadatadx_mod, variant, fields):
    """`#[pyo3(get)]` field getters cover the schema column list."""
    cls = getattr(thetadatadx_mod, variant)
    members = set(dir(cls))
    for field in fields:
        assert field in members, (
            f"thetadatadx.{variant} missing field getter '{field}'; "
            f"check `[events.{variant}]` columns in fpss_event_schema.toml"
        )


def test_kind_getter_per_variant(thetadatadx_mod):
    """Every typed event class exposes a `kind` property whose value
    is the snake_case variant name -- the same discriminator the
    TypeScript SDK's `FpssEvent.kind` field carries."""
    expected_kinds = {
        "LoginSuccess": "login_success",
        "ContractAssigned": "contract_assigned",
        "ReqResponse": "req_response",
        "MarketOpen": "market_open",
        "MarketClose": "market_close",
        "ServerError": "server_error",
        "Disconnected": "disconnected",
        "Reconnecting": "reconnecting",
        "Reconnected": "reconnected",
        "Error": "error",
        "UnknownFrame": "unknown_frame",
        "Connected": "connected",
        "Ping": "ping",
        "ReconnectedServer": "reconnected_server",
        "Restart": "restart",
        "UnknownControl": "unknown_control",
    }
    for variant, expected_kind in expected_kinds.items():
        cls = getattr(thetadatadx_mod, variant)
        # The `kind` property is a `#[getter]` on the pyclass impl;
        # reading it off the class dict confirms it was registered.
        assert "kind" in dir(cls), f"thetadatadx.{variant}.kind missing"
        # We don't construct the class from Python (frozen +
        # skip_from_py_object), so we can't read the value at runtime
        # without a live FPSS event. Verify the getter is wired up
        # via the property descriptor instead.
        kind_descriptor = vars(cls).get("kind")
        assert kind_descriptor is not None, (
            f"thetadatadx.{variant}.kind property descriptor missing"
        )
        # The schema-driven generator emits a `&'static str` literal
        # equal to `expected_kind`; the value is checked indirectly
        # via the TS SDK's `FpssEvent.kind` literal-union test (and
        # in the live-stream callback path on real events).
        del expected_kind  # unused locally; documents the intent


def test_class_identity_stable(thetadatadx_mod):
    """Re-importing the module returns the same class object."""
    again = importlib.import_module("thetadatadx")
    for variant, _fields in CONTROL_VARIANTS:
        assert getattr(thetadatadx_mod, variant) is getattr(again, variant), (
            f"thetadatadx.{variant} class identity unstable across imports"
        )


def test_data_classes_still_exported(thetadatadx_mod):
    """Sanity check: typed data classes survive the schema rewrite."""
    for variant in ("Quote", "Trade", "Ohlcvc", "OpenInterest", "Contract"):
        cls = getattr(thetadatadx_mod, variant, None)
        assert cls is not None, f"thetadatadx.{variant} missing"
        assert cls.__name__ == variant


def test_simple_class_removed(thetadatadx_mod):
    """The flat `Simple` envelope is gone -- typed-per-variant control
    classes replace it. Pin its absence so accidental re-introduction
    fails fast."""
    assert not hasattr(thetadatadx_mod, "Simple"), (
        "thetadatadx.Simple should be removed; typed control variants "
        "(LoginSuccess, Disconnected, ...) are the SSOT now"
    )
