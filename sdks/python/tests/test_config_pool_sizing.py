"""MDDS pool-sizing setters on `Config` (issue #584).

Locks the contract that the three new properties exposed by
``Config`` — ``concurrent_requests`` / ``decoder_threads`` /
``decoder_ring_size`` — round-trip through the pyo3 binding to the
underlying Rust ``MddsConfig`` correctly, and that invalid ring sizes
raise ``ValueError`` at the setter boundary rather than waiting for
the connect-time `validate()` call to fail.

Live behaviour (the tier clamp at connect time, the auto-detect
default = 0 sentinels) is covered by the Rust unit tests under
``mdds::client::pool_size_tests``; this file pins only the Python
surface contract.
"""

from __future__ import annotations

import importlib

import pytest


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


# ─── decoder_threads ────────────────────────────────────────────────


def test_decoder_threads_defaults_to_auto_detect_sentinel():
    """`decoder_threads = 0` is the auto-detect sentinel."""
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.decoder_threads == 0


def test_decoder_threads_round_trips():
    """`decoder_threads = N` round-trips through the binding."""
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (1, 2, 4, 8, 16):
        cfg.decoder_threads = n
        assert cfg.decoder_threads == n


# ─── decoder_ring_size ──────────────────────────────────────────────


def test_decoder_ring_size_default_is_production_baseline():
    """`decoder_ring_size = 256` is the production default."""
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.decoder_ring_size == 256


def test_decoder_ring_size_accepts_power_of_two_above_minimum():
    """All valid ring sizes round-trip."""
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (64, 128, 256, 512, 1024, 2048, 4096):
        cfg.decoder_ring_size = n
        assert cfg.decoder_ring_size == n


def test_decoder_ring_size_rejects_below_minimum():
    """A ring size below 64 must raise ValueError at the setter.

    The Rust core's `check_ring_size` enforces this at connect-time
    validation; surfacing the rejection at the setter boundary gives
    the Python user immediate feedback instead of a delayed connect
    failure.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"decoder_ring_size"):
        cfg.decoder_ring_size = 32


def test_decoder_ring_size_rejects_non_power_of_two():
    """Non-power-of-two ring sizes must raise ValueError."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"decoder_ring_size"):
        cfg.decoder_ring_size = 100
    with pytest.raises(ValueError, match=r"decoder_ring_size"):
        cfg.decoder_ring_size = 1023


def test_decoder_ring_size_rejects_zero():
    """Zero is not a valid ring size — the disruptor crate rejects it."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"decoder_ring_size"):
        cfg.decoder_ring_size = 0


# ─── Combined invariants ────────────────────────────────────────────


def test_all_three_setters_independent():
    """The three setters do not interfere with each other.

    Round-tripping each property after writing the others must
    return the values that were last written.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.concurrent_requests = 8
    cfg.decoder_threads = 16
    cfg.decoder_ring_size = 1024
    assert cfg.concurrent_requests == 8
    assert cfg.decoder_threads == 16
    assert cfg.decoder_ring_size == 1024
