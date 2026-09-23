"""
Dropped-events counter accessibility test.

Pins the contract that ``client.stream.dropped_event_count()`` is callable
across the streaming lifecycle (pre-start / post-start /
post-reconnect / post-stop) and is non-negative everywhere.

The counter is owned by the live ``StreamingClient`` (LMAX Disruptor
ring overflow recorded by ``Producer::try_publish`` failures) and
is forwarded through ``thetadatadx::Client::dropped_event_count``
and the PyO3 wrapper. Because the counter lives on the live client,
``reconnect()`` (which calls ``stop_streaming() + start_streaming()``
internally) rebuilds the client and resets the count to 0.
``stop_streaming()`` clears the streaming slot, and the getter
returns 0 in that state. Snapshot the value BEFORE reconnect if
you need to accumulate drops across session boundaries.

This shape mirrors the TypeScript binding's
``__tests__/dropped_events.test.mjs`` to keep the public contract
identical across SDKs.

Gated on ``THETADATADX_TEST_CREDS=path/to/creds.txt`` because
``Client`` needs a live FPSS handshake. Tests skip silently on
developer machines that haven't wired creds. CI runs this in the
surfaces job.

What this test does NOT assert:

* the counter actually *increments* on a live drop. Synthesizing a
  guaranteed-dropped event requires a full FPSS mock harness that
  is out of scope for the correctness-hygiene sprint.
* monotonicity across reconnect. Reconnect rebuilds the FPSS
  client and resets the counter; locking in a monotone-across-
  reconnect invariant would freeze in implementation detail we
  explicitly do NOT promise.
"""

from __future__ import annotations

import os

import pytest


@pytest.fixture
def client():
    """Build a real `Client` client or skip the test."""
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
    # Best-effort teardown; stop_streaming on a client that never
    # started is a noop per the Rust side contract.
    try:
        client.stream.stop_streaming()
    except Exception:
        pass


def _noop_callback(_event):
    """Minimal callback for lifecycle tests.

    The LMAX Disruptor consumer invokes this under the GIL for every
    FPSS event. We deliberately do nothing here -- the test harness
    only cares about lifecycle hooks (counter accessibility across
    start/reconnect/stop), not about per-event delivery.
    """


def test_dropped_event_count_callable_before_streaming(client):
    """The getter must be callable before `start_streaming(callback)`
    and return 0 -- the streaming slot is empty, so the wrapper
    forwards 0 from the unified client.
    """
    count = client.stream.dropped_event_count()
    assert isinstance(count, int)
    assert count >= 0
    # Pre-stream, the FPSS client hasn't been spawned -- count is 0.
    assert count == 0


def test_dropped_event_count_lifecycle_callable(client):
    """The counter must remain callable across the full lifecycle:
    pre-start / post-start / post-reconnect / post-stop. The value
    is non-negative everywhere, and non-decreasing: a reconnect rebuilds
    the FPSS client, and the drops the retired session recorded stay in
    the total.
    """
    client.stream.start_streaming(_noop_callback)
    post_start = client.stream.dropped_event_count()
    assert isinstance(post_start, int)
    assert post_start >= 0

    client.stream.reconnect()
    post_reconnect = client.stream.dropped_event_count()
    assert isinstance(post_reconnect, int)
    # Reconnect calls stop_streaming + start_streaming, which retires the
    # FPSS client. Its drops are folded into the running total first, so the
    # counter never goes backwards across a session boundary.
    assert post_reconnect >= post_start

    client.stream.stop_streaming()
    post_stop = client.stream.dropped_event_count()
    assert isinstance(post_stop, int)
    # The session is gone, but the drops it recorded are not.
    assert post_stop >= post_reconnect


def test_start_streaming_rejects_a_non_callable_at_registration(client):
    """`start_streaming` rejects a non-callable argument at the call site.

    Accepting it would connect the stream and then fail on the first event,
    on the consumer thread, as an unraisable TypeError: the caller sees a
    session that came up and quietly never delivered. The standalone
    `StreamingClient` rejects it the same way, so the two streaming surfaces
    answer the same mistake identically.
    """
    import thetadatadx

    with pytest.raises(thetadatadx.InvalidParameterError, match="must be callable"):
        client.stream.start_streaming(42)
    # The rejected registration left no reservation behind, so a correct
    # callback still starts.
    client.stream.start_streaming(_noop_callback)
    client.stream.stop_streaming()


def test_reconnect_without_callback_raises(client):
    """`reconnect()` requires a previously installed callback. The
    binding must surface a clear `RuntimeError` rather than silently
    starting a callback-less stream.
    """
    with pytest.raises(RuntimeError, match="no callback registered"):
        client.stream.reconnect()
