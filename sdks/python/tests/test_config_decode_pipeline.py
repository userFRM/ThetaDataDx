"""MDDS two-stage decode pipeline knobs on ``Config`` (Phase 3 of 3).

Pins the contract that the two new properties exposed by ``Config`` —
``decode_threads`` (stage-2 prost-decode + Tick-build worker count) and
``decode_queue_depth`` (bounded MPSC queue between stage-1 and stage-2)
— round-trip through the pyo3 binding to the underlying Rust
``MddsConfig`` correctly, including the ``None`` auto-size sentinel,
the explicit-``0`` (clamps internally to ``1`` at pool construction)
case, and the rejection of negative values at the setter boundary.

Stage-1 thread count remains controlled by the legacy
``decoder_threads`` knob (now deprecated-alias-only — see the rustdoc
on ``MddsConfig::decoder_threads``); this file pins only the new
Phase-3 stage-2 surface.

Live behaviour (auto-sizing at connect time, the pool's
``Some(0) -> max(1)`` clamp, queue depth defaulting to
``concurrent_requests * 64``) is covered by the Rust unit tests under
``crate::config::tests`` and ``crate::mdds::client``; this file pins
only the Python surface contract.
"""

from __future__ import annotations

import importlib

import pytest


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── decode_threads ─────────────────────────────────────────────────


def test_decode_threads_defaults_to_none_auto_size_sentinel():
    """``decode_threads = None`` is the auto-size sentinel.

    Matches the Rust core's ``MddsConfig::decode_threads = None``
    default. The pool resolves ``None`` to
    ``std::thread::available_parallelism()`` at connect time.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.decode_threads is None


def test_decode_threads_round_trips_with_none():
    """Writing ``None`` after an explicit value returns to auto-size."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_threads = 8
    assert cfg.decode_threads == 8
    cfg.decode_threads = None
    assert cfg.decode_threads is None


def test_decode_threads_round_trips_with_explicit_int():
    """Explicit ``int`` values round-trip verbatim."""
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (1, 2, 4, 8, 16, 32, 64):
        cfg.decode_threads = n
        assert cfg.decode_threads == n


def test_decode_threads_explicit_zero_is_legal():
    """Explicit ``0`` is a legal value — the pool clamps to ``1`` internally.

    The Python setter does NOT raise on ``0``: the Rust core's
    ``Some(0) -> n.max(1)`` clamp at pool construction makes ``0`` a
    semantically valid input distinct from ``None`` (auto-size).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_threads = 0
    assert cfg.decode_threads == 0


def test_decode_threads_rejects_negative_values():
    """Negative ``decode_threads`` must raise ``ValueError`` at the setter."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"decode_threads"):
        cfg.decode_threads = -1


def test_decode_threads_retains_large_value_verbatim():
    """Operators on under-detected hosts may pin a large explicit count."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_threads = 4096
    assert cfg.decode_threads == 4096


# ─── decode_queue_depth ─────────────────────────────────────────────


def test_decode_queue_depth_defaults_to_none_auto_size_sentinel():
    """``decode_queue_depth = None`` is the auto-size sentinel.

    The pool resolves ``None`` to ``concurrent_requests * 64`` at
    connect time (with a floor of 64).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.decode_queue_depth is None


def test_decode_queue_depth_round_trips_with_none():
    """Writing ``None`` after an explicit value returns to auto-size."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_queue_depth = 1024
    assert cfg.decode_queue_depth == 1024
    cfg.decode_queue_depth = None
    assert cfg.decode_queue_depth is None


def test_decode_queue_depth_round_trips_with_explicit_int():
    """Explicit ``int`` values round-trip verbatim."""
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (1, 64, 128, 512, 2048, 8192):
        cfg.decode_queue_depth = n
        assert cfg.decode_queue_depth == n


def test_decode_queue_depth_explicit_zero_is_legal():
    """Explicit ``0`` is a legal value — the queue clamps to ``1`` internally.

    Matches the ``Some(0) -> max(1)`` clamp in
    ``crate::mdds::client::pool_with_decoders``.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_queue_depth = 0
    assert cfg.decode_queue_depth == 0


def test_decode_queue_depth_rejects_negative_values():
    """Negative ``decode_queue_depth`` must raise ``ValueError`` at the setter."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"decode_queue_depth"):
        cfg.decode_queue_depth = -1


def test_decode_queue_depth_retains_large_value_verbatim():
    """Wide-strike backfills may pin a queue depth far above the default."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_queue_depth = 65_536
    assert cfg.decode_queue_depth == 65_536


# ─── Combined invariants ────────────────────────────────────────────


def test_decode_threads_and_queue_depth_are_independent():
    """The two stage-2 setters do not interfere with each other.

    Round-tripping each property after writing the other must return
    the values last written.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.decode_threads = 16
    cfg.decode_queue_depth = 4096
    assert cfg.decode_threads == 16
    assert cfg.decode_queue_depth == 4096


def test_decode_pipeline_setters_are_independent_from_legacy_pool_sizing():
    """The Phase-3 setters do not collide with the legacy pool-sizing knobs.

    ``concurrent_requests`` (channel pool), ``decoder_threads`` (stage-1
    zstd decompress), and ``decoder_ring_size`` (per-thread Disruptor
    depth) must round-trip unchanged when the two-stage knobs are also
    set on the same ``Config``.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.concurrent_requests = 8
    cfg.decoder_threads = 4
    cfg.decoder_ring_size = 1024
    cfg.decode_threads = 16
    cfg.decode_queue_depth = 4096
    assert cfg.concurrent_requests == 8
    assert cfg.decoder_threads == 4
    assert cfg.decoder_ring_size == 1024
    assert cfg.decode_threads == 16
    assert cfg.decode_queue_depth == 4096
