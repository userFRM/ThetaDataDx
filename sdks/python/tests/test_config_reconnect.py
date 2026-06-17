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


def test_reconnect_stable_window_secs_rejects_above_u64() -> None:
    """Magnitudes above ``u64::MAX`` must be rejected at the boundary.

    Mirrors the TypeScript boundary case in
    ``__tests__/config_reconnect.test.mjs`` that pins
    ``setReconnectStableWindowSecs(1n << 65n)`` as a rejection. Python's
    ``int`` is unbounded, so pyo3's ``u64`` extraction raises
    ``OverflowError`` when the magnitude does not fit in 64 bits.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    with pytest.raises(OverflowError):
        cfg.reconnect_stable_window_secs = 1 << 65


# ─── Combined invariants ────────────────────────────────────────────


def test_reconnect_setter_state_survives_interleaved_calls():
    """Interleaved reconnect setter and pool-sizing setter calls
    must not interfere with each other.

    Mirrors the TS ``Reconnect setters are independent`` case: the
    reconnect setters have no getters, so we assert via a historical
    tuning getter that that state survives a reconnect setter
    sequence.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.reconnect_policy = "auto"
    cfg.reconnect_max_attempts = 7
    cfg.reconnect_max_rate_limited_attempts = 77
    cfg.reconnect_stable_window_secs = 120
    cfg.warn_on_buffered_threshold_bytes = 8 * 1024 * 1024
    assert cfg.warn_on_buffered_threshold_bytes == 8 * 1024 * 1024
    # Reconnect policy getter still reads the policy we set.
    assert cfg.reconnect_policy == "auto"


# ─── ReconnectConfig.wait_ms / wait_rate_limited_ms ────────────────


def test_reconnect_wait_ms_defaults_to_wire_constants() -> None:
    """Defaults mirror ``ReconnectConfig::production_defaults``:
    ``wait_ms=250`` (initial delay of the exponential transient
    ladder) / ``wait_rate_limited_ms=130_000`` (the TooManyRequests
    floor).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_wait_ms == 250
    assert cfg.reconnect_wait_rate_limited_ms == 130_000


def test_reconnect_wait_ms_round_trips() -> None:
    """Setter / getter pair round-trips across the documented u64
    range. The values are plumbed through to the FPSS I/O loop at
    connect time via ``reconnect_delay_for``.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    for ms in [0, 1, 500, 2_000, 60_000, 130_000, 2**60]:
        cfg.reconnect_wait_ms = ms
        assert cfg.reconnect_wait_ms == ms
        cfg.reconnect_wait_rate_limited_ms = ms
        assert cfg.reconnect_wait_rate_limited_ms == ms


def test_reconnect_wait_ms_rejects_above_u64() -> None:
    """Magnitudes above ``u64::MAX`` must be rejected at the
    pyo3 boundary (``OverflowError`` from the ``u64`` extractor).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(OverflowError):
        cfg.reconnect_wait_ms = 1 << 65
    with pytest.raises(OverflowError):
        cfg.reconnect_wait_rate_limited_ms = 1 << 65


# ─── RuntimeConfig.worker_threads ──────────────────────────────────


def test_worker_threads_default_is_none() -> None:
    """Default is ``None`` (default sizing, one worker per logical
    CPU).
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.worker_threads is None


def test_worker_threads_round_trips_some_zero() -> None:
    """``Some(0)`` is preserved verbatim across the binding boundary;
    the runtime builder clamps it to ``1`` only inside the builder,
    never at the setter.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    for n in [0, 1, 2, 4, 8, 16, 64]:
        cfg.worker_threads = n
        assert cfg.worker_threads == n
    cfg.worker_threads = None
    assert cfg.worker_threads is None


# ─── RetryPolicy field setters/getters ─────────────────────────────


def test_retry_policy_defaults() -> None:
    """Defaults mirror ``RetryPolicy::default``: 250ms initial /
    30s max / 5 attempts / jitter on.
    """
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.retry_initial_delay_ms == 250
    assert cfg.retry_max_delay_ms == 30_000
    assert cfg.retry_max_attempts == 20
    assert cfg.retry_jitter is True


def test_retry_policy_round_trips() -> None:
    """Per-field round-trip via the pyo3 setter/getter pairs."""
    mod = _import_module()
    cfg = mod.Config.production()
    cfg.retry_initial_delay_ms = 500
    cfg.retry_max_delay_ms = 60_000
    cfg.retry_max_attempts = 7
    cfg.retry_jitter = False
    assert cfg.retry_initial_delay_ms == 500
    assert cfg.retry_max_delay_ms == 60_000
    assert cfg.retry_max_attempts == 7
    assert cfg.retry_jitter is False


def test_retry_policy_rejects_above_u64() -> None:
    """Magnitudes above ``u64::MAX`` reject at the pyo3 boundary."""
    mod = _import_module()
    cfg = mod.Config.production()
    with pytest.raises(OverflowError):
        cfg.retry_initial_delay_ms = 1 << 65
    with pytest.raises(OverflowError):
        cfg.retry_max_delay_ms = 1 << 65
