"""
Streaming context manager (`with tdx.streaming(callback) as session:`)
lifecycle tests.

Pins the contract that the wrapper:

* registers the callback via `start_streaming(callback)` on `__enter__`;
* pairs `stop_streaming()` + `await_drain(5000)` on `__exit__`;
* emits a `RuntimeWarning` when the drain barrier times out, without
  swallowing the original exception from the `with` body;
* exposes every public `subscribe_*` / `unsubscribe_*` method on the
  underlying `Client` via `StreamingSession.__getattr__` proxy --
  no hand-listed mirror, single source of truth.

Live tests are gated on ``THETADX_TEST_CREDS=path/to/creds.txt``
because the underlying `Client` needs a real FPSS handshake.
Static surface tests run without credentials.
"""

from __future__ import annotations

import os
import warnings
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


@pytest.fixture
def tdx():
    """Build a real `Client` client or skip the test."""
    creds_path = os.environ.get("THETADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    mod = _import_module()
    creds = mod.Credentials.from_file(creds_path)
    config = mod.Config.production()
    client = mod.Client(creds, config)
    yield client
    try:
        client.stream.stop_streaming()
    except Exception:
        pass


def _noop_callback(_event: Any) -> None:
    """Callback used for lifecycle assertions (no per-event work).

    The LMAX Disruptor consumer invokes this under the GIL for every
    FPSS event. The test harness cares about the context-manager
    lifecycle hooks, not about per-event delivery.
    """


def test_streaming_session_class_exported() -> None:
    """`StreamingSession` is exported alongside `Client` so users
    can type-annotate the bound name from `with tdx.streaming(cb) as s`.
    """
    mod = _import_module()
    assert hasattr(mod, "StreamingSession"), "StreamingSession should be a public symbol"


def test_thetadatadx_has_streaming_factory() -> None:
    """`tdx.streaming(callback)` is the user-facing entry point. Verify
    the method exists on the class without needing a live connection.
    """
    mod = _import_module()
    assert hasattr(mod.Client, "streaming")


def test_context_manager_enter_exit_lifecycle(tdx) -> None:
    """`with tdx.streaming(callback) as session:` enters by calling
    `start_streaming(callback)` and exits by calling
    `stop_streaming()` + `await_drain(5000)`.
    """
    assert tdx.stream.is_streaming() is False
    with tdx.streaming(_noop_callback) as session:
        # `session` is the StreamingSession; subscribe methods proxy
        # through __getattr__ to the underlying Client.
        assert tdx.stream.is_streaming() is True
        # Exercise the proxy SSOT: a method that lives on
        # `Client` is reachable on `session` without a hand-listed
        # mirror.
        active = session.active_subscriptions()
        assert isinstance(active, list)
    # __exit__ must have called stop_streaming() (not just dropped the
    # ref) so is_streaming() flips back to False.
    assert tdx.stream.is_streaming() is False


def test_context_manager_swallows_no_exceptions(tdx) -> None:
    """`__exit__` returns False so exceptions raised inside the `with`
    body propagate. The wrapper must NOT mask body errors with its own
    drain-timeout warning logic.
    """
    sentinel = RuntimeError("body sentinel -- must propagate through __exit__")
    with pytest.raises(RuntimeError, match="body sentinel"):
        with tdx.streaming(_noop_callback) as _session:
            raise sentinel
    # is_streaming flipped to False -- stop_streaming ran in __exit__.
    assert tdx.stream.is_streaming() is False


def test_context_manager_proxies_subscribe_methods(tdx) -> None:
    """SSOT: every public method on `Client` is reachable on the
    bound session via `StreamingSession.__getattr__`. There is NO
    hand-listed mirror -- adding a new subscribe method to
    `Client` makes it callable through the session automatically.
    """
    from thetadatadx import Contract

    with tdx.streaming(_noop_callback) as session:
        # The polymorphic `subscribe(sub)` lives on `Client`, not
        # on `StreamingSession`. Proxy must forward.
        session.subscribe(Contract.stock("AAPL").quote())
        # `dropped_event_count` lives on `Client`, not on
        # `StreamingSession`. Proxy must forward and return an int.
        count = session.dropped_event_count()
        assert isinstance(count, int)
        assert count >= 0
        # `unsubscribe(sub)` round-trips back to a clean state.
        session.unsubscribe(Contract.stock("AAPL").quote())


def test_double_enter_raises(tdx) -> None:
    """Re-entering the same session is a programming error: each
    `__enter__` consumes the stored callback. The second enter must
    raise rather than silently re-register.
    """
    cm = tdx.streaming(_noop_callback)
    with cm as _session:
        pass
    with pytest.raises(RuntimeError, match="callback already consumed"):
        cm.__enter__()
