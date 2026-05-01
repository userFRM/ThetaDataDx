"""
Dropped-events counter accessibility test.

Verifies the fix for review finding `A-02` in
`todo.md` / the security-review branch: the per-closure `AtomicU64`
counter used to be local to each `start_streaming` / `reconnect`
closure, so it reset on every reconnect AND was not reachable from
Python. The fix lifts the counter to an instance field on
`ThetaDataDx` and exposes it via `tdx.dropped_events()`. This test
pins the contract: the getter must be callable after one
`start_streaming` and after a subsequent `reconnect()`, and must
return a non-negative integer (u64 on the Rust side).

Gated on `THETADX_TEST_CREDS=path/to/creds.txt` because `ThetaDataDx`
needs a live FPSS handshake. Tests skip silently on developer
machines that haven't wired creds. CI runs this in the surfaces job
(same pattern as `sdks/go/timeout_test.go`).

What this test does NOT assert:

* the counter actually *increments* on a live drop. Synthesizing a
  guaranteed-dropped event requires a full FPSS mock harness that
  is out of scope for the correctness-hygiene sprint (noted in
  `todo.md` A-02 rationale).
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


def test_dropped_events_callable_before_streaming(tdx):
    """The getter must be callable before `start_streaming()` and
    return 0 — the counter is initialised on the instance, not inside
    the `start_streaming` closure.
    """
    count = tdx.dropped_events()
    assert isinstance(count, int)
    assert count >= 0
    # Pre-stream, nothing has dropped.
    assert count == 0


def test_dropped_events_survives_start_and_reconnect(tdx):
    """The counter must remain accessible after `start_streaming()`
    and after a manual `reconnect()`. This is the core A-02 contract:
    a closure-local counter would have been rebuilt on reconnect and
    never visible to Python at all.
    """
    tdx.start_streaming()
    post_start = tdx.dropped_events()
    assert isinstance(post_start, int)
    assert post_start >= 0

    tdx.reconnect()
    post_reconnect = tdx.dropped_events()
    assert isinstance(post_reconnect, int)
    # Counter must be monotonically non-decreasing across reconnect.
    # A reset would imply the closure-local regression was reintroduced.
    assert post_reconnect >= post_start

    tdx.stop_streaming()
    post_stop = tdx.dropped_events()
    assert isinstance(post_stop, int)
    # Still readable after stop — the counter lives on the handle,
    # not the receiver channel.
    assert post_stop >= post_reconnect
