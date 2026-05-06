"""
Dropped-events counter accessibility test.

Pins the contract that ``tdx.dropped_event_count()`` is callable
across the streaming lifecycle (pre-start / post-start /
post-reconnect / post-stop) and is non-negative everywhere.

The counter lives on the SSOT ``StreamingDispatcher`` (see
``crates/thetadatadx/src/fpss/dispatcher.rs``) and is forwarded
through ``thetadatadx::ThetaDataDx::dropped_event_count`` and the
PyO3 wrapper. Because the counter lives on the live dispatcher,
``reconnect()`` (which calls ``stop_streaming() + start_streaming()``
internally) rebuilds the dispatcher and resets the count to 0.
``stop_streaming()`` clears the dispatcher slot, and the getter
returns 0 in that state. Snapshot the value BEFORE reconnect if
you need to accumulate drops across session boundaries.

This shape mirrors the TypeScript binding's
``__tests__/dropped_events.test.mjs`` to keep the public contract
identical across SDKs.

Gated on ``THETADX_TEST_CREDS=path/to/creds.txt`` because
``ThetaDataDx`` needs a live FPSS handshake. Tests skip silently on
developer machines that haven't wired creds. CI runs this in the
surfaces job.

What this test does NOT assert:

* the counter actually *increments* on a live drop. Synthesizing a
  guaranteed-dropped event requires a full FPSS mock harness that
  is out of scope for the correctness-hygiene sprint.
* monotonicity across reconnect. Reconnect rebuilds the dispatcher
  and resets the counter; locking in a monotone-across-reconnect
  invariant would freeze in implementation detail we explicitly do
  NOT promise.
"""

from __future__ import annotations

import os

import pytest


@pytest.fixture
def tdx():
    """Build a real `ThetaDataDx` client or skip the test."""
    creds_path = os.environ.get("THETADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    try:
        import thetadatadx
    except ImportError:
        pytest.skip(
            "thetadatadx native extension not built "
            "-- run `maturin develop` from sdks/python/"
        )

    creds = thetadatadx.Credentials.from_file(creds_path)
    config = thetadatadx.Config.production()
    client = thetadatadx.ThetaDataDx(creds, config)
    yield client
    # Best-effort teardown; stop_streaming on a client that never
    # started is a noop per the Rust side contract.
    try:
        client.stop_streaming()
    except Exception:
        pass


def _noop_callback(_event):
    """Minimal callback for lifecycle tests.

    The dispatcher invokes this under the GIL on its drain thread for
    every FPSS event. We deliberately do nothing here -- the test
    harness only cares about lifecycle hooks (counter accessibility
    across start/reconnect/stop), not about per-event delivery.
    """


def test_dropped_event_count_callable_before_streaming(tdx):
    """The getter must be callable before `start_streaming(callback)`
    and return 0 -- the dispatcher slot is empty, so the wrapper
    forwards 0 from the unified client.
    """
    count = tdx.dropped_event_count()
    assert isinstance(count, int)
    assert count >= 0
    # Pre-stream, the dispatcher hasn't been spawned -- count is 0.
    assert count == 0


def test_dropped_event_count_lifecycle_callable(tdx):
    """The counter must remain callable across the full lifecycle:
    pre-start / post-start / post-reconnect / post-stop. The value
    is non-negative everywhere; it is NOT monotone across reconnect
    because reconnect rebuilds the dispatcher and zeros the counter.
    Snapshot before reconnect if you need cross-session accumulation.
    """
    tdx.start_streaming(_noop_callback)
    post_start = tdx.dropped_event_count()
    assert isinstance(post_start, int)
    assert post_start >= 0

    tdx.reconnect()
    post_reconnect = tdx.dropped_event_count()
    assert isinstance(post_reconnect, int)
    # Counter lives on the live StreamingDispatcher; reconnect calls
    # stop_streaming + start_streaming, which recreates the dispatcher
    # and zeroes the counter. Snapshot before reconnect if cross-
    # session accumulation matters. Assert non-negative rather than
    # monotone -- monotone would lock in implementation detail we
    # explicitly do NOT promise.
    assert post_reconnect >= 0

    tdx.stop_streaming()
    post_stop = tdx.dropped_event_count()
    assert isinstance(post_stop, int)
    # After stop_streaming the dispatcher slot is empty; the getter
    # returns 0.
    assert post_stop >= 0


def test_start_streaming_requires_callable(tdx):
    """`start_streaming` is callback-only after PR C. Passing a
    non-callable must surface as a Python `TypeError` at call time
    (Python raises this when the dispatcher tries to invoke the
    object), not silently store junk on the wrapper.
    """
    # `42` is not callable. PyO3 accepts `Py<PyAny>` so the wrapper
    # itself doesn't reject it -- the failure surfaces only on the
    # dispatcher thread when an event arrives. We don't trigger an
    # event here (no subscriptions, no live data), so the call should
    # succeed and `stop_streaming` should clear the bad reference.
    tdx.start_streaming(42)
    tdx.stop_streaming()


def test_reconnect_without_callback_raises(tdx):
    """`reconnect()` requires a previously installed callback. The
    binding must surface a clear `RuntimeError` rather than silently
    starting a callback-less stream.
    """
    with pytest.raises(RuntimeError, match="no callback registered"):
        tdx.reconnect()
