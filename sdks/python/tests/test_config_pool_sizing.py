"""Historical tuning setters on `Config`.

Locks the contract that the historical tuning properties exposed by
``Config`` (such as ``request_timeout_secs``) round-trip through the
pyo3 binding to the underlying Rust ``HistoricalConfig`` correctly.

Live behaviour (the per-tier connection-pool concurrency limit
resolved at connect time) is covered by the Rust unit tests under
``mdds::client::pool_size_tests``; this file pins only the Python
surface contract.
"""

from __future__ import annotations

import importlib


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── request_timeout_secs ───────────────────────────────────────────


def test_request_timeout_secs_defaults_to_300():
    """`request_timeout_secs` defaults to the 300s per-request deadline."""
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.request_timeout_secs == 300


def test_request_timeout_secs_round_trips():
    """`request_timeout_secs = N` round-trips through the binding;
    ``0`` disables the default deadline."""
    mod = _import_module()
    cfg = mod.Config.production()
    for secs in (0, 1, 45, 120, 600):
        cfg.request_timeout_secs = secs
        assert cfg.request_timeout_secs == secs
