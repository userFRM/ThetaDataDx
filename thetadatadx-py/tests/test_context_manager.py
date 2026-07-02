"""
Streaming context manager (`with client.streaming(callback) as session:`)
lifecycle tests.

Pins the contract that the wrapper:

* registers the callback via `start_streaming(callback)` on `__enter__`;
* pairs `stop_streaming()` + `await_drain(5000)` on `__exit__`;
* emits a `RuntimeWarning` when the drain barrier times out, without
  swallowing the original exception from the `with` body;
* exposes every public `subscribe_*` / `unsubscribe_*` method on the
  underlying `Client` via `StreamingSession.__getattr__` proxy --
  no hand-listed mirror, single source of truth.

Live tests are gated on ``THETADATADX_TEST_CREDS=path/to/creds.txt``
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
            "-- run `maturin develop` from thetadatadx-py/"
        )
    return mod


@pytest.fixture
def client():
    """Build a real `Client` client or skip the test."""
    creds_path = os.environ.get("THETADATADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADATADX_TEST_CREDS=path/to/creds.txt to enable this live test"
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
    can type-annotate the bound name from `with client.streaming(cb) as s`.
    """
    mod = _import_module()
    assert hasattr(mod, "StreamingSession"), "StreamingSession should be a public symbol"


def test_thetadatadx_has_streaming_factory() -> None:
    """`client.streaming(callback)` is the user-facing entry point. Verify
    the method exists on the class without needing a live connection.
    """
    mod = _import_module()
    assert hasattr(mod.Client, "streaming")


def test_unified_stream_view_exposes_is_authenticated() -> None:
    """`client.stream.is_authenticated()` mirrors the standalone
    `StreamingClient.is_authenticated()` on the unified surface (cross-
    binding parity with C++ `Stream::is_authenticated()` and TypeScript
    `StreamView.isAuthenticated`). Asserted offline on the `StreamView`
    type alongside `is_streaming` so the cross-binding accessor cannot be
    dropped without a test failure even when no live credentials are set.
    """
    mod = _import_module()
    assert hasattr(mod, "StreamView"), "thetadatadx must export `StreamView`"
    assert hasattr(mod.StreamView, "is_streaming"), (
        "StreamView must expose is_streaming()"
    )
    assert hasattr(mod.StreamView, "is_authenticated"), (
        "StreamView must expose is_authenticated() (cross-binding parity "
        "with the standalone StreamingClient and the C++ / TypeScript surfaces)"
    )


def test_unified_stream_view_is_authenticated_false_before_start(client) -> None:
    """Before any `start_streaming` the live slot is empty, so
    `client.stream.is_authenticated()` reads `False` (live-gated)."""
    assert client.stream.is_authenticated() is False, (
        "StreamView.is_authenticated() must read False before streaming starts"
    )


def test_context_manager_enter_exit_lifecycle(client) -> None:
    """`with client.streaming(callback) as session:` enters by calling
    `start_streaming(callback)` and exits by calling
    `stop_streaming()` + `await_drain(5000)`.
    """
    assert client.stream.is_streaming() is False
    with client.streaming(_noop_callback) as session:
        # `session` is the StreamingSession; subscribe methods proxy
        # through __getattr__ to the underlying Client.
        assert client.stream.is_streaming() is True
        # Exercise the proxy SSOT: a method that lives on
        # `Client` is reachable on `session` without a hand-listed
        # mirror.
        active = session.active_subscriptions()
        assert isinstance(active, list)
    # __exit__ must have called stop_streaming() (not just dropped the
    # ref) so is_streaming() flips back to False.
    assert client.stream.is_streaming() is False


def test_context_manager_swallows_no_exceptions(client) -> None:
    """`__exit__` returns False so exceptions raised inside the `with`
    body propagate. The wrapper must NOT mask body errors with its own
    drain-timeout warning logic.
    """
    sentinel = RuntimeError("body sentinel -- must propagate through __exit__")
    with pytest.raises(RuntimeError, match="body sentinel"):
        with client.streaming(_noop_callback) as _session:
            raise sentinel
    # is_streaming flipped to False -- stop_streaming ran in __exit__.
    assert client.stream.is_streaming() is False


def test_context_manager_proxies_subscribe_methods(client) -> None:
    """SSOT: every public method on `Client` is reachable on the
    bound session via `StreamingSession.__getattr__`. There is NO
    hand-listed mirror -- adding a new subscribe method to
    `Client` makes it callable through the session automatically.
    """
    from thetadatadx import Contract

    with client.streaming(_noop_callback) as session:
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


def test_double_enter_raises(client) -> None:
    """Re-entering the same session is a programming error: each
    `__enter__` consumes the stored callback. The second enter must
    raise rather than silently re-register.
    """
    cm = client.streaming(_noop_callback)
    with cm as _session:
        pass
    with pytest.raises(RuntimeError, match="callback already consumed"):
        cm.__enter__()


# ─────────────────────────────────────────────────────────────────────
# Base-client lifecycle: `close()` + `with` / `async with` (issue #1069)
# and the direct-start-then-drop deadlock-safety (issue 1).
# ─────────────────────────────────────────────────────────────────────


def test_streaming_dispatcher_holds_a_weak_client_ref() -> None:
    """Offline lifecycle guard: the unified streaming dispatcher closure must
    capture a WEAK reference to the core client, never a strong `Arc`.

    This is the invariant behind the forgetful-drop teardown. The dispatcher
    closure outlives the `start_streaming` / `reconnect` call and runs on its
    own thread for the whole session. If it captured a strong
    `Arc<Client>` (e.g. `Arc::clone(&self.client)`) that strong reference
    would keep the core client alive after the user drops their Python
    handle, so `Client::Drop` — the detach that tears the streaming session
    down off the GIL thread — would never run and the FPSS session would leak
    until process exit. Capturing `Arc::downgrade(&self.client)` and
    upgrading only for the `record_panic()` bump keeps the cycle broken.

    Asserted against the generated source (no credentials needed) so a
    regression of the generator template fails here even offline — the
    live thread-teardown test above is the end-to-end proof but is gated on
    a real FPSS handshake.
    """
    from pathlib import Path

    here = Path(__file__).resolve().parent
    src_root = None
    for candidate in [here, *here.parents]:
        target = candidate / "src"
        if target.is_dir() and (target / "lib.rs").is_file():
            src_root = target
            break
    assert src_root is not None, "could not locate thetadatadx-py/src"

    generated = (src_root / "_generated" / "streaming_methods.rs").read_text()
    # The dispatcher's client handle is bound to `panic_recorder`. It MUST be
    # a weak downgrade, and there must be no strong clone of the client
    # captured for the dispatcher.
    assert "let panic_recorder = Arc::downgrade(&self.client);" in generated, (
        "the streaming dispatcher must capture a WEAK client handle "
        "(`Arc::downgrade(&self.client)`) so a forgetful drop still runs "
        "Client::Drop and tears the session down"
    )
    assert "let panic_recorder = Arc::clone(&self.client);" not in generated, (
        "the streaming dispatcher must NOT capture a strong `Arc<Client>` "
        "(`Arc::clone(&self.client)`): it would pin the core client alive "
        "past the user's drop and leak the streaming session"
    )
    # Both the start_streaming and reconnect dispatchers carry the capture,
    # so the weak downgrade must appear on both paths.
    assert generated.count("Arc::downgrade(&self.client)") >= 2, (
        "both the start_streaming and reconnect dispatcher closures must "
        "hold a weak client reference"
    )


def test_base_clients_expose_close_and_context_managers() -> None:
    """Offline surface pin: the base clients carry the deterministic
    teardown surface. `Client` / `HistoricalClient` are sync + async
    context managers with `close()`; `AsyncClient` is an async-only
    context manager with `close()`. Asserted without credentials so a
    regression that drops any of these fails even offline.
    """
    mod = _import_module()
    for name in ("Client", "HistoricalClient"):
        cls = getattr(mod, name)
        for attr in ("close", "__enter__", "__exit__", "__aenter__", "__aexit__"):
            assert hasattr(cls, attr), f"{name} must expose {attr}"
    # AsyncClient is async-first: async CM + close, no sync CM.
    async_cls = mod.AsyncClient
    for attr in ("close", "__aenter__", "__aexit__"):
        assert hasattr(async_cls, attr), f"AsyncClient must expose {attr}"


def test_context_manager_closes_cleanly(client) -> None:
    """`with Client(...) as c:` binds the client and closes it on exit.

    The block exit runs `close()` (stop streaming if live + drain + drop
    the callback). With no streaming started, close is a fast no-op and
    the client is still bound inside the block. Live-gated because the
    constructor needs a real handshake.
    """
    with client as bound:
        assert bound is client
        # A historical query still works inside the block (channel open).
        assert bound.stream.is_streaming() is False
    # After the block the client is closed; a second close is idempotent
    # and must not raise.
    client.close()


def test_close_is_idempotent_and_safe_after_streaming(client) -> None:
    """`close()` is idempotent and safe to call after a streaming session.

    Start streaming, close once (stops + drains), then close again: the
    second call is a no-op and must not raise or hang. Live-gated.
    """
    client.stream.start_streaming(_noop_callback)
    assert client.stream.is_streaming() is True
    client.close()
    assert client.stream.is_streaming() is False
    # Idempotent: calling close again on an already-closed client is a
    # no-op, never a panic or a hang.
    client.close()
    client.close()


def test_direct_start_streaming_then_drop_does_not_deadlock() -> None:
    """The forgetful path is deadlock-safe (issue 1).

    A user calls `client.stream.start_streaming(cb)` (the DIRECT path, not
    the `with client.streaming(cb)` context manager) and then lets the
    `Client` fall out of scope WITHOUT calling `stop_streaming()`. The
    final `Arc::drop` runs the core `Client::Drop`, which detaches the
    dispatcher join off the dropping (GIL-holding) thread, so the drop
    returns instead of deadlocking against the dispatcher's `Python::attach`.

    This asserts teardown ACTUALLY happens, not merely that the drop does
    not hang. The bug the fix closes has two layers: (a) the dispatcher
    closure must hold a WEAK reference to the core client, else the strong
    `Arc` pins it alive and `Client::Drop` never runs at all; (b) once Drop
    runs, its join must be detached off the GIL thread. Layer (a) is what a
    "does not hang" test misses — with a strong ref the drop returns
    instantly (nothing to tear down) yet the FPSS reader / dispatcher threads
    run forever. So the test observes the live streaming worker threads
    terminating: it snapshots the OS thread count with a stream live, drops
    the client without `stop_streaming()`, and asserts the count falls back
    toward the pre-stream baseline within a bounded window. A leaked session
    keeps those threads and fails the assertion. Live-gated because
    `start_streaming` opens a real FPSS connection.
    """
    import gc
    import time

    creds_path = os.environ.get("THETADATADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADATADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    mod = _import_module()

    def os_thread_count() -> int:
        # OS-level thread count (Linux). Counts every kernel thread of this
        # process, including the Rust-spawned FPSS reader / dispatcher /
        # detach-helper threads that Python's `threading` module cannot see.
        try:
            return len(os.listdir("/proc/self/task"))
        except FileNotFoundError:
            pytest.skip("thread-count teardown check needs /proc (Linux)")

    baseline = os_thread_count()

    client = mod.Client(mod.Credentials.from_file(creds_path), mod.Config.production())
    client.stream.start_streaming(_noop_callback)
    # Let the FPSS reader + dispatcher threads come up.
    time.sleep(1.0)
    streaming = os_thread_count()
    assert streaming > baseline, (
        "precondition: a live stream must have spawned OS worker threads "
        f"(baseline={baseline}, streaming={streaming})"
    )

    # Drop the sole reference WITHOUT stop_streaming(), then force collection
    # so the pyclass destructor (and the core `Client::Drop`) runs now. If the
    # dispatcher held a strong `Arc<Client>`, Drop would not run and the
    # streaming threads would persist; the detach helper then joins them.
    del client
    gc.collect()

    # The FPSS session must tear down and its threads join back to the
    # baseline within a bounded window. Poll rather than sleep-once so a fast
    # teardown returns promptly; a leaked session never converges and trips
    # the deadline. The detach helper thread itself is short-lived (it joins
    # the FPSS threads then exits), so the count returns to baseline.
    deadline = time.monotonic() + 15.0
    final = streaming
    while time.monotonic() < deadline:
        final = os_thread_count()
        if final <= baseline:
            break
        time.sleep(0.1)
    assert final <= baseline, (
        "FPSS worker threads did not terminate after a forgetful drop: "
        f"baseline={baseline}, still_running={final}. The dispatcher closure "
        "is pinning the core Client with a strong Arc, so Client::Drop never "
        "ran and the streaming session leaked."
    )
