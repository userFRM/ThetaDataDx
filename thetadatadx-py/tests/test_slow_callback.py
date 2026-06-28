"""
Slow-callback watchdog accessibility + round-trip test.

Pins the contract that ``client.stream.slow_callback_count()`` is
callable across the streaming lifecycle (pre-start / post-start /
post-reconnect / post-stop), is non-negative everywhere, and that
``client.stream.set_slow_callback_threshold_us(threshold_us)`` round-trips
as a no-throw configuration call. The threshold crosses the binding
boundary as microseconds (the core watchdog measures user-callback
wall-clock against it); pass 0 to disable the watchdog.

The counter and threshold live on the live streaming client, forwarded
through ``thetadatadx::Client::slow_callback_count`` /
``set_slow_callback_threshold`` and the PyO3 wrapper. Because the state
lives on the live client, ``reconnect()`` (which calls
``stop_streaming() + start_streaming()`` internally) rebuilds the client
and resets the count to 0; ``stop_streaming()`` clears the slot and the
getter returns 0 in that state.

This shape mirrors the TypeScript binding's
``__tests__/slow_callback.test.mjs`` and the C++
``tests/thetadatadx_client.cpp`` slow-callback coverage to keep the
public contract identical across SDKs.

Gated on ``THETADATADX_TEST_CREDS=path/to/creds.txt`` because ``Client``
needs a live FPSS handshake. Tests skip silently on developer machines
that haven't wired creds.

What this test does NOT assert: that the counter actually *increments*
on a genuinely slow callback. Synthesizing a guaranteed-over-budget
invocation requires a full FPSS mock harness that is out of scope here;
the watchdog is observability-only and never cancels the callback.
"""

from __future__ import annotations

import os

import pytest


@pytest.fixture
def client():
    """Build a real `Client` or skip the test."""
    creds_path = os.environ.get("THETADATADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADATADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    try:
        import thetadatadx
    except ImportError:
        pytest.skip(
            "thetadatadx native extension not built "
            "-- run `maturin develop` from thetadatadx-py/"
        )

    creds = thetadatadx.Credentials.from_file(creds_path)
    config = thetadatadx.Config.production()
    client = thetadatadx.Client(creds, config)
    yield client
    try:
        client.stream.stop_streaming()
    except Exception:
        pass


def _noop_callback(_event):
    """Minimal callback for lifecycle tests; does no work."""


def test_slow_callback_count_callable_before_streaming(client):
    """The getter must be callable before `start_streaming(callback)`
    and return 0 -- the streaming slot is empty, so the wrapper
    forwards 0 from the unified client.
    """
    count = client.stream.slow_callback_count()
    assert isinstance(count, int)
    assert count == 0


def test_set_slow_callback_threshold_us_noop_before_streaming(client):
    """The threshold setter is a no-op before a session is live and must
    not raise. The watchdog count stays 0.
    """
    client.stream.set_slow_callback_threshold_us(2_500)
    assert client.stream.slow_callback_count() == 0
    # Passing 0 disables the watchdog.
    client.stream.set_slow_callback_threshold_us(0)


def test_slow_callback_lifecycle_round_trip(client):
    """The setter round-trips and the counter stays callable across the
    full lifecycle: pre-start / post-start / post-reconnect / post-stop.
    The value is non-negative everywhere; it is NOT monotone across
    reconnect because reconnect rebuilds the FPSS client and zeros it.
    """
    client.stream.start_streaming(_noop_callback)
    # Configure a 1 ms budget on the live session.
    client.stream.set_slow_callback_threshold_us(1_000)
    post_start = client.stream.slow_callback_count()
    assert isinstance(post_start, int)
    assert post_start >= 0

    client.stream.reconnect()
    post_reconnect = client.stream.slow_callback_count()
    assert isinstance(post_reconnect, int)
    assert post_reconnect >= 0

    client.stream.stop_streaming()
    post_stop = client.stream.slow_callback_count()
    assert isinstance(post_stop, int)
    assert post_stop >= 0
