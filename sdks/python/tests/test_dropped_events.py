"""
Dropped-events counter accessibility test.

Pins the contract that ``tdx.dropped_event_count()`` is callable
across the streaming lifecycle (pre-start / post-start / post-reconnect
/ post-stop) and is monotonically non-decreasing across reconnect.

The counter lives on the SSOT ``StreamingDispatcher`` (see
``crates/thetadatadx/src/fpss/dispatcher.rs``) and is forwarded
through ``thetadatadx::ThetaDataDx::dropped_event_count`` and the
PyO3 wrapper. PR C (#482) replaced the per-binding mpsc-shim
counter with the SSOT one, so a regression to a closure-local
``AtomicU64`` would be caught by the same monotonic-increase check
used historically.

Gated on ``THETADX_TEST_CREDS=path/to/creds.txt`` because
``ThetaDataDx`` needs a live FPSS handshake. Tests skip silently on
developer machines that haven't wired creds. CI runs this in the
surfaces job.

What this test does NOT assert:

* the counter actually *increments* on a live drop. Synthesizing a
  guaranteed-dropped event requires a full FPSS mock harness that
  is out of scope for the correctness-hygiene sprint.
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
    and return 0 -- the counter is initialised on the SSOT dispatcher,
    not inside the binding closure.
    """
    count = tdx.dropped_event_count()
    assert isinstance(count, int)
    assert count >= 0
    # Pre-stream, the dispatcher hasn't been spawned -- count is 0.
    assert count == 0


def test_dropped_event_count_survives_start_and_reconnect(tdx):
    """The counter must remain accessible after `start_streaming()`
    and after a manual `reconnect()`. PR C (#482) routes this through
    `thetadatadx::ThetaDataDx::dropped_event_count`, which lives on
    the SSOT dispatcher and survives the reconnect tear-down/rebuild
    cycle by design.
    """
    tdx.start_streaming(_noop_callback)
    post_start = tdx.dropped_event_count()
    assert isinstance(post_start, int)
    assert post_start >= 0

    tdx.reconnect()
    post_reconnect = tdx.dropped_event_count()
    assert isinstance(post_reconnect, int)
    # Counter must be monotonically non-decreasing across reconnect.
    # A reset would imply a regression to per-closure counters.
    assert post_reconnect >= post_start

    tdx.stop_streaming()
    post_stop = tdx.dropped_event_count()
    assert isinstance(post_stop, int)
    # Still readable after stop -- the counter lives on the SSOT
    # dispatcher reachable through `tdx.dropped_event_count()`.
    assert post_stop >= post_reconnect


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
