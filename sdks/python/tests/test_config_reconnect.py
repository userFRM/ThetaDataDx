"""ReconnectConfig setters on ``Config`` — Python binding parity with
TypeScript / C++ / FFI.

Pins the Python surface contract for ``reconnect_policy``,
``reconnect_max_attempts``, ``reconnect_max_rate_limited_attempts``,
and ``reconnect_stable_window_secs``. Failure-class semantics
(transient vs rate-limited budget split, stable-window timer reset)
are exercised in the Rust unit tests under
``fpss::session::tests`` and
``fpss::protocol::reconnect_delays_match_policy``; this file pins
only that the Python surface forwards the inputs without dropping
them and rejects invalid policy strings at the boundary.
"""

from __future__ import annotations

import importlib

import pytest


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── reconnect_policy ───────────────────────────────────────────────


def test_reconnect_policy_round_trips_auto_and_manual():
    """``reconnect_policy`` accepts ``"auto"`` and ``"manual"`` (case-insensitive)."""
    mod = _import_module()
    cfg = mod.Config.production()
    for value in ("auto", "AUTO", "manual", "Manual"):
        cfg.reconnect_policy = value
    # Getter normalises to lowercase canonical form.
    cfg.reconnect_policy = "auto"
    assert cfg.reconnect_policy == "auto"
    cfg.reconnect_policy = "manual"
    assert cfg.reconnect_policy == "manual"


def test_reconnect_policy_rejects_unknown_value():
    """An unknown policy string must raise ``ValueError`` at the setter."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(ValueError, match=r"reconnect_policy"):
        cfg.reconnect_policy = "linear-backoff"


# ─── reconnect_max_attempts ─────────────────────────────────────────


def test_reconnect_max_attempts_accepts_non_zero_budgets():
    """``reconnect_max_attempts = N`` accepts every plausible budget."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    for n in (1, 3, 10, 100, 1000):
        cfg.reconnect_max_attempts = n


def test_reconnect_max_attempts_accepts_zero():
    """``reconnect_max_attempts = 0`` is a legal ``u32`` and must not raise.

    The Rust core treats ``0`` as a budget value the auto-driver
    enforces verbatim; the setter is write-only and the Python
    binding does not own the semantic interpretation. Verifying
    round-trip absence-of-error is the contract pinned here.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    cfg.reconnect_max_attempts = 0


def test_reconnect_max_attempts_is_silent_noop_on_manual_policy():
    """The setter is a silent no-op when the policy is not ``Auto(limits)``.

    Matches the cross-binding contract: the setter only mutates
    ``ReconnectAttemptLimits`` fields when the policy variant is
    ``Auto``; under ``Manual`` (or the Rust-only ``Custom``) the
    call is silently absorbed. The Python surface has no getter on
    this knob (parity with FFI / TS / C++ which are all write-only
    on the per-class budgets), so the contract is "does not raise".
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "manual"
    cfg.reconnect_max_attempts = 5


# ─── reconnect_max_rate_limited_attempts ────────────────────────────


def test_reconnect_max_rate_limited_attempts_accepts_non_zero_budgets():
    """``reconnect_max_rate_limited_attempts = N`` accepts every plausible budget."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    for n in (1, 10, 100, 1000):
        cfg.reconnect_max_rate_limited_attempts = n


def test_reconnect_max_rate_limited_attempts_accepts_zero():
    """``reconnect_max_rate_limited_attempts = 0`` is a legal ``u32``."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    cfg.reconnect_max_rate_limited_attempts = 0


# ─── reconnect_stable_window_secs ───────────────────────────────────


def test_reconnect_stable_window_secs_accepts_u64_values():
    """``reconnect_stable_window_secs`` accepts the full ``u64`` range."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    for secs in (0, 1, 30, 60, 300, 3600, 86_400):
        cfg.reconnect_stable_window_secs = secs


def test_reconnect_stable_window_secs_rejects_negative():
    """Negative seconds must be rejected at the Python type level.

    pyo3 maps ``u64`` to a Python integer with a non-negative
    contract; passing a negative value raises ``OverflowError``
    before the setter body runs.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    with pytest.raises(OverflowError):
        cfg.reconnect_stable_window_secs = -1


# ─── Combined invariants ────────────────────────────────────────────


def test_reconnect_setter_state_survives_interleaved_calls():
    """Interleaved reconnect setter and pool-sizing setter calls
    must not interfere with each other.

    Mirrors the TS ``Reconnect setters are independent`` case: the
    reconnect setters have no getters, so we assert via the
    pool-sizing getters that the pool-sizing state survives a
    reconnect setter sequence.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    cfg.reconnect_max_attempts = 7
    cfg.reconnect_max_rate_limited_attempts = 77
    cfg.reconnect_stable_window_secs = 120
    cfg.concurrent_requests = 4
    cfg.decoder_ring_size = 512
    assert cfg.concurrent_requests == 4
    assert cfg.decoder_ring_size == 512
    # Reconnect policy getter still reads the policy we set.
    assert cfg.reconnect_policy == "auto"
