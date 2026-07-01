"""Connection-resilience knobs on ``Config`` — Python binding parity
with TypeScript / C++ / FFI.

Pins the Python surface for the reconnect cadence ladder
(``reconnect_wait_max_ms`` / ``reconnect_wait_server_restart_ms``),
the jitter mode, the wall-clock envelope and per-class budgets, the
subscription-replay pacing knobs, the streaming transport knobs (timeouts,
ping cadence, ring size, read slice, watchdog, keepalive schedule,
host selection + shuffle seed), the historical-channel retry envelope,
the flatfile jitter toggle, and the custom reconnect callback
registration. The reconnect-engine semantics themselves are exercised
in the Rust unit tests; this file pins only that the Python surface
forwards values without dropping them and rejects invalid input at
the boundary.
"""

from __future__ import annotations

import importlib

import pytest


def _import_module():
    """Import the freshly-built native module from ``maturin develop``."""
    return importlib.import_module("thetadatadx")


# ─── Reconnect cadence + jitter ─────────────────────────────────────


def test_reconnect_ladder_defaults_and_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_wait_ms == 250
    assert cfg.reconnect_wait_max_ms == 30_000
    assert cfg.reconnect_wait_rate_limited_ms == 130_000
    assert cfg.reconnect_wait_server_restart_ms == 5_000
    cfg.reconnect_wait_ms = 100
    cfg.reconnect_wait_max_ms = 60_000
    cfg.reconnect_wait_server_restart_ms = 2_500
    assert cfg.reconnect_wait_ms == 100
    assert cfg.reconnect_wait_max_ms == 60_000
    assert cfg.reconnect_wait_server_restart_ms == 2_500


def test_reconnect_jitter_round_trips_and_rejects_unknown():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_jitter == "full"
    for mode in ("equal", "DECORRELATED", "none", "Full"):
        cfg.reconnect_jitter = mode
        assert cfg.reconnect_jitter == mode.lower()
    with pytest.raises(ValueError, match=r"reconnect_jitter"):
        cfg.reconnect_jitter = "gaussian"


def test_reconnect_budgets_and_envelope_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_max_attempts == 30
    assert cfg.reconnect_max_rate_limited_attempts == 100
    assert cfg.reconnect_max_server_restart_attempts == 60
    assert cfg.reconnect_max_elapsed_secs == 300
    assert cfg.reconnect_stable_window_secs == 60
    cfg.reconnect_max_server_restart_attempts = 5
    assert cfg.reconnect_max_server_restart_attempts == 5
    cfg.reconnect_max_elapsed_secs = 0  # disables the envelope
    assert cfg.reconnect_max_elapsed_secs == 0


def test_reconnect_replay_pacing_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_replay_burst_size == 50
    assert cfg.reconnect_replay_pace_ms == 5
    cfg.reconnect_replay_burst_size = 200
    cfg.reconnect_replay_pace_ms = 0
    assert cfg.reconnect_replay_burst_size == 200
    assert cfg.reconnect_replay_pace_ms == 0


def test_reconnect_callback_registration_switches_policy():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.reconnect_policy == "auto"
    cfg.reconnect_callback = lambda reason, attempt: 1_000
    assert cfg.reconnect_policy == "custom"
    # None restores the default Auto policy.
    cfg.reconnect_callback = None
    assert cfg.reconnect_policy == "auto"


# ─── Streaming transport ─────────────────────────────────────────────────


def test_streaming_transport_defaults_and_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.streaming_timeout_ms == 3_000
    assert cfg.streaming_connect_timeout_ms == 2_000
    assert cfg.streaming_ping_interval_ms == 250
    assert cfg.streaming_ring_size == 131_072
    assert cfg.streaming_io_read_slice_ms == 25
    assert cfg.streaming_keepalive_idle_secs == 5
    assert cfg.streaming_keepalive_interval_secs == 2
    assert cfg.streaming_keepalive_retries == 2
    cfg.streaming_timeout_ms = 10_000
    cfg.streaming_connect_timeout_ms = 5_000
    cfg.streaming_ping_interval_ms = 1_000
    cfg.streaming_ring_size = 8_192
    cfg.streaming_io_read_slice_ms = 50
    cfg.streaming_keepalive_idle_secs = 10
    cfg.streaming_keepalive_interval_secs = 5
    cfg.streaming_keepalive_retries = 4
    assert cfg.streaming_timeout_ms == 10_000
    assert cfg.streaming_connect_timeout_ms == 5_000
    assert cfg.streaming_ping_interval_ms == 1_000
    assert cfg.streaming_ring_size == 8_192
    assert cfg.streaming_io_read_slice_ms == 50
    assert cfg.streaming_keepalive_idle_secs == 10
    assert cfg.streaming_keepalive_interval_secs == 5
    assert cfg.streaming_keepalive_retries == 4


def test_streaming_host_selection_round_trips_and_rejects_unknown():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.streaming_host_selection == "shuffled"
    cfg.streaming_host_selection = "fixed_order"
    assert cfg.streaming_host_selection == "fixed_order"
    cfg.streaming_host_selection = "SHUFFLED"
    assert cfg.streaming_host_selection == "shuffled"
    with pytest.raises(ValueError, match=r"streaming_host_selection"):
        cfg.streaming_host_selection = "round_robin"


def test_streaming_host_shuffle_seed_round_trips_none_sentinel():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.streaming_host_shuffle_seed is None
    cfg.streaming_host_shuffle_seed = 42
    assert cfg.streaming_host_shuffle_seed == 42
    cfg.streaming_host_shuffle_seed = None
    assert cfg.streaming_host_shuffle_seed is None


# ─── Historical retry envelope + flatfile jitter ────────────────────


def test_retry_envelope_defaults_and_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.retry_max_attempts == 20
    assert cfg.retry_max_elapsed_secs == 300
    cfg.retry_max_elapsed_secs = 0  # disables the envelope
    assert cfg.retry_max_elapsed_secs == 0


def test_flatfiles_budget_defaults_and_jitter_round_trip():
    mod = _import_module()
    cfg = mod.Config.production()
    assert cfg.flatfiles_max_attempts == 10
    assert cfg.flatfiles_max_backoff_secs == 30
    assert cfg.flatfiles_jitter is True
    cfg.flatfiles_jitter = False
    assert cfg.flatfiles_jitter is False


# ─── Client observability surface ───────────────────────────────────


def test_staleness_getters_exist_on_both_clients():
    """The staleness clock + connected-address getters are present on
    the unified client's `StreamView` sub-namespace (reached via
    ``client.stream``) and the standalone streaming client. Values are
    exercised live elsewhere; this pins the surface shape offline."""
    mod = _import_module()
    for cls in (mod.StreamView, mod.StreamingClient):
        for name in (
            "millis_since_last_event",
            "last_event_received_at_unix_nanos",
            "last_connected_addr",
        ):
            assert callable(getattr(cls, name, None)), f"{cls.__name__}.{name} missing"


def test_reconnects_exhausted_event_class_exported():
    """The terminal reconnect event is a typed export with the same
    field shape every binding carries."""
    mod = _import_module()
    cls = mod.ReconnectsExhausted
    assert hasattr(cls, "reason")
    assert hasattr(cls, "attempts")
    assert hasattr(cls, "kind")
    assert hasattr(cls, "reason_name")
