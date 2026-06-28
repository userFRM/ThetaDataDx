"""FlatFilesConfig setters on ``Config`` — Python binding parity with
TypeScript / C++ / FFI.

Pins the Python surface contract for ``flatfiles_max_attempts``,
``flatfiles_initial_backoff_secs``, ``flatfiles_max_backoff_secs``,
``flatfiles_connect_timeout_secs``, and ``flatfiles_read_timeout_secs``.
The Rust core enforces the ``[1, 10]`` range on ``max_attempts`` and
the ``max_backoff >= initial_backoff`` invariant at
``DirectConfig::validate`` time; this file pins only that the Python
surface round-trips the inputs without dropping them.
"""

from __future__ import annotations

import importlib

import pytest


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── Defaults ───────────────────────────────────────────────────────


def test_flatfiles_defaults_mirror_production_defaults() -> None:
    """Defaults mirror ``FlatFilesConfig::production_defaults``:
    ``max_attempts=3`` / ``initial_backoff_secs=1`` /
    ``max_backoff_secs=4``.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.flatfiles_max_attempts == 10
    assert cfg.flatfiles_initial_backoff_secs == 1
    assert cfg.flatfiles_max_backoff_secs == 30
    assert cfg.flatfiles_connect_timeout_secs == 10
    assert cfg.flatfiles_read_timeout_secs == 60


# ─── Round-trip ─────────────────────────────────────────────────────


def test_flatfiles_max_attempts_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u32 range.

    The Rust core validates ``[1, 10]`` at ``DirectConfig::validate``
    time, not at the Python setter, so any ``u32`` lands on the field
    here verbatim.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    for n in (0, 1, 3, 5, 10, 100, 1_000):
        cfg.flatfiles_max_attempts = n
        assert cfg.flatfiles_max_attempts == n


def test_flatfiles_initial_backoff_secs_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u64 range."""
    mod = _import_module()
    cfg = mod.Config.production()
    for secs in (0, 1, 2, 4, 10, 60, 3_600, 86_400):
        cfg.flatfiles_initial_backoff_secs = secs
        assert cfg.flatfiles_initial_backoff_secs == secs


def test_flatfiles_max_backoff_secs_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u64 range."""
    mod = _import_module()
    cfg = mod.Config.production()
    for secs in (0, 1, 4, 10, 60, 3_600, 86_400):
        cfg.flatfiles_max_backoff_secs = secs
        assert cfg.flatfiles_max_backoff_secs == secs


def test_flatfiles_connect_timeout_secs_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u64 range."""
    mod = _import_module()
    cfg = mod.Config.production()
    for secs in (0, 1, 4, 10, 60, 3_600):
        cfg.flatfiles_connect_timeout_secs = secs
        assert cfg.flatfiles_connect_timeout_secs == secs


def test_flatfiles_read_timeout_secs_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u64 range."""
    mod = _import_module()
    cfg = mod.Config.production()
    for secs in (0, 1, 4, 10, 60, 3_600, 86_400):
        cfg.flatfiles_read_timeout_secs = secs
        assert cfg.flatfiles_read_timeout_secs == secs


# ─── Boundary cases ─────────────────────────────────────────────────


def test_flatfiles_max_attempts_rejects_negative() -> None:
    """Negative ``max_attempts`` must reject at the pyo3 ``u32``
    extractor (``OverflowError``).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(OverflowError):
        cfg.flatfiles_max_attempts = -1


def test_flatfiles_backoff_secs_rejects_negative() -> None:
    """Negative seconds reject at the pyo3 ``u64`` extractor."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(OverflowError):
        cfg.flatfiles_initial_backoff_secs = -1
    with pytest.raises(OverflowError):
        cfg.flatfiles_max_backoff_secs = -1
    with pytest.raises(OverflowError):
        cfg.flatfiles_connect_timeout_secs = -1
    with pytest.raises(OverflowError):
        cfg.flatfiles_read_timeout_secs = -1


def test_flatfiles_backoff_secs_rejects_above_u64() -> None:
    """Magnitudes above ``u64::MAX`` reject at the pyo3 boundary."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(OverflowError):
        cfg.flatfiles_initial_backoff_secs = 1 << 65
    with pytest.raises(OverflowError):
        cfg.flatfiles_max_backoff_secs = 1 << 65
    with pytest.raises(OverflowError):
        cfg.flatfiles_connect_timeout_secs = 1 << 65
    with pytest.raises(OverflowError):
        cfg.flatfiles_read_timeout_secs = 1 << 65


# ─── Composed state ─────────────────────────────────────────────────


def test_flatfiles_field_setters_compose_into_consistent_config() -> None:
    """After mutating all three fields the underlying ``flatfiles``
    sub-config must reflect the composed shape — proves the setters
    target the same struct rather than duplicating state.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.flatfiles_max_attempts = 5
    cfg.flatfiles_initial_backoff_secs = 2
    cfg.flatfiles_max_backoff_secs = 30
    cfg.flatfiles_connect_timeout_secs = 15
    cfg.flatfiles_read_timeout_secs = 90
    assert cfg.flatfiles_max_attempts == 5
    assert cfg.flatfiles_initial_backoff_secs == 2
    assert cfg.flatfiles_max_backoff_secs == 30
    assert cfg.flatfiles_connect_timeout_secs == 15
    assert cfg.flatfiles_read_timeout_secs == 90


def test_flatfiles_setter_state_survives_interleaved_calls() -> None:
    """Interleaved flatfile setter and historical tuning setter calls
    must not interfere with each other. Mirrors the TS / C++ contract.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.flatfiles_max_attempts = 7
    cfg.flatfiles_initial_backoff_secs = 3
    cfg.flatfiles_max_backoff_secs = 12
    cfg.flatfiles_connect_timeout_secs = 20
    cfg.flatfiles_read_timeout_secs = 45
    cfg.warn_on_buffered_threshold_bytes = 8 * 1024 * 1024
    assert cfg.warn_on_buffered_threshold_bytes == 8 * 1024 * 1024
    assert cfg.flatfiles_max_attempts == 7
    assert cfg.flatfiles_initial_backoff_secs == 3
    assert cfg.flatfiles_max_backoff_secs == 12
    assert cfg.flatfiles_connect_timeout_secs == 20
    assert cfg.flatfiles_read_timeout_secs == 45
