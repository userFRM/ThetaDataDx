"""Historical pool-sizing setter on `Config`.

Locks the contract that the ``concurrent_requests`` property exposed
by ``Config`` round-trips through the pyo3 binding to the underlying
Rust ``HistoricalConfig`` correctly.

Live behaviour (the tier clamp at connect time, the auto-detect
default = 0 sentinels) is covered by the Rust unit tests under
``mdds::client::pool_size_tests``; this file pins only the Python
surface contract.
"""

from __future__ import annotations

import importlib


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── concurrent_requests ────────────────────────────────────────────


def test_concurrent_requests_defaults_to_auto_detect_sentinel():
    """`concurrent_requests = 0` is the auto-detect sentinel.

    The Rust core resolves the value at connect time off the Nexus
    subscription tier. Production defaults keep the sentinel so the
    most common user (`Config.production()` + connect) never has to
    set it.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.concurrent_requests == 0


def test_concurrent_requests_round_trips():
    """`concurrent_requests = N` round-trips through the binding."""
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (1, 2, 4, 8, 16):
        cfg.concurrent_requests = n
        assert cfg.concurrent_requests == n


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
