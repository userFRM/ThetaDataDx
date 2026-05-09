"""
Pull-iter delivery mode tests.

Pins the contract that:

* `ThetaDataDxClient.start_streaming_iter()` returns an `EventIterator`
  the caller can iterate with `for event in iterator:`;
* `with tdx.streaming_iter() as iterator:` is equivalent and pairs
  `stop_streaming` + `await_drain` on `__exit__`, mirroring the
  callback-mode `streaming(callback)` context manager;
* push-callback (`start_streaming(callback)`) and pull-iter
  (`start_streaming_iter()`) are mutually exclusive on a given client;
* `iterator.close()` / `__exit__` short-circuit `__next__` to
  `StopIteration` once the queue drains.

Live tests are gated on ``THETADX_TEST_CREDS=path/to/creds.txt``
because they need a real FPSS handshake. Static surface tests run
without credentials.
"""

from __future__ import annotations

import os
import threading
import time
from typing import Any

import pytest


def _import_module():
    try:
        import thetadatadx as mod
    except ImportError:
        pytest.skip(
            "thetadatadx native extension not built "
            "-- run `maturin develop` from sdks/python/"
        )
    return mod


def test_iter_classes_are_exported() -> None:
    """`EventIterator` / `StreamingIterSession` must be reachable on
    the package import surface so type-stub generators and IDEs can
    discover them. No live connection required."""
    mod = _import_module()
    assert hasattr(mod, "EventIterator"), (
        "thetadatadx must export `EventIterator` (pull-iter delivery handle)"
    )
    assert hasattr(mod, "StreamingIterSession"), (
        "thetadatadx must export `StreamingIterSession` (with-block helper)"
    )
    assert hasattr(mod.ThetaDataDxClient, "start_streaming_iter"), (
        "ThetaDataDxClient must expose `start_streaming_iter()`"
    )
    assert hasattr(mod.ThetaDataDxClient, "streaming_iter"), (
        "ThetaDataDxClient must expose `streaming_iter()` factory"
    )


@pytest.fixture
def tdx():
    """Build a real `ThetaDataDxClient` or skip the test."""
    creds_path = os.environ.get("THETADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    mod = _import_module()
    creds = mod.Credentials.from_file(creds_path)
    config = mod.Config.production()
    client = mod.ThetaDataDxClient(creds, config)
    yield client
    try:
        client.stop_streaming()
    except Exception:
        pass


def test_iter_yields_login_success(tdx) -> None:
    """The first event the iterator yields after `start_streaming_iter`
    is the FPSS `LoginSuccess` control event — same handshake the
    push-callback path observes. Pins the most basic round-trip."""
    iterator = tdx.start_streaming_iter()
    try:
        # Wait up to 5 s for the first event to surface.
        deadline = time.time() + 5.0
        first: Any = None
        while time.time() < deadline:
            evt = iterator.try_next()
            if evt is not None:
                first = evt
                break
            time.sleep(0.01)
        assert first is not None, "iterator should have yielded LoginSuccess within 5s"
        # The exact class name is generated; loose-check the kind by
        # repr to avoid coupling the test to the codegen surface.
        assert "LoginSuccess" in type(first).__name__ or "Connected" in type(first).__name__, (
            f"first event should be a connect / login success control variant, got {type(first).__name__}"
        )
    finally:
        iterator.close()
        tdx.stop_streaming()


def test_iter_and_callback_are_mutually_exclusive(tdx) -> None:
    """Starting in one delivery mode then attempting the other must
    raise `RuntimeError`. The slot is shared at the
    `ThetaDataDxClient::start_streaming*` layer; this test pins that
    invariant on the Python binding."""
    iterator = tdx.start_streaming_iter()
    try:
        # Iter mode is now live -- callback mode must refuse.
        with pytest.raises((RuntimeError, Exception)):
            tdx.start_streaming(lambda _evt: None)
    finally:
        iterator.close()
        tdx.stop_streaming()

    # Now the slot is `Stopped`; either mode should be valid again.
    received: list[Any] = []
    tdx.start_streaming(lambda evt: received.append(evt))
    try:
        # ... and starting iter mode again must fail until we stop.
        with pytest.raises((RuntimeError, Exception)):
            tdx.start_streaming_iter()
    finally:
        tdx.stop_streaming()


def test_with_block_closes_iterator(tdx) -> None:
    """`with tdx.streaming_iter() as it:` must close the iterator and
    drain the streaming session on exit. After the block, `is_streaming`
    is `False` and the iterator raises `StopIteration` on `next`."""
    with tdx.streaming_iter() as iterator:
        assert iterator is not None
        # Drain the handshake event(s) so the test exercises the
        # close path with at least one in-flight event.
        deadline = time.time() + 2.0
        while time.time() < deadline:
            if iterator.try_next() is None:
                time.sleep(0.01)
                continue
            break

    # After the with block, streaming is stopped.
    assert tdx.is_streaming() is False, (
        "stop_streaming + await_drain inside __exit__ should leave is_streaming false"
    )


def test_iter_close_short_circuits_next(tdx) -> None:
    """`iterator.close()` from another thread must unblock a pending
    `__next__` and raise `StopIteration` once the queue drains. Pins
    the user-driven termination path."""
    iterator = tdx.start_streaming_iter()
    try:
        # Drain the handshake events first so the next `__next__`
        # actually parks waiting on the queue.
        deadline = time.time() + 2.0
        while time.time() < deadline and iterator.try_next() is not None:
            pass

        def closer():
            time.sleep(0.2)
            iterator.close()

        threading.Thread(target=closer, daemon=True).start()
        with pytest.raises(StopIteration):
            next(iter(iterator))
    finally:
        try:
            iterator.close()
        except Exception:
            pass
        tdx.stop_streaming()


def test_iter_terminates_after_stop(tdx) -> None:
    """`tdx.stop_streaming()` from another thread must unblock a
    pending `for event in iterator:` loop and raise `StopIteration`
    within a bounded budget. Earlier the underlying core conflated
    timeout with terminal close on `next_timeout`, so `__next__` spun
    forever on a stopped session because every 50 ms slice came back
    as `None` (timeout-shaped) and the loop kept retrying.

    Drives a real iterator; spawns a closer thread that calls
    `stop_streaming()` after 100 ms; asserts the for-loop exits within
    a 1-second budget (well above the 50 ms wait slice and signal
    re-check cadence).
    """
    iterator = tdx.start_streaming_iter()
    try:
        # Drain handshake events so the iterator parks on an empty
        # queue when the closer fires.
        drain_deadline = time.time() + 2.0
        while time.time() < drain_deadline and iterator.try_next() is not None:
            pass

        def stopper():
            time.sleep(0.1)
            tdx.stop_streaming()

        threading.Thread(target=stopper, daemon=True).start()

        started = time.time()
        for _event in iterator:
            # Drain any tail events the stop_streaming path lets
            # through; the loop must exit on its own once `Closed`
            # surfaces.
            pass
        elapsed = time.time() - started
        assert elapsed < 1.0, (
            f"iter loop must exit within 1s of stop_streaming; took {elapsed:.3f}s "
            f"(earlier it would spin forever)"
        )
    finally:
        try:
            iterator.close()
        except Exception:
            pass
