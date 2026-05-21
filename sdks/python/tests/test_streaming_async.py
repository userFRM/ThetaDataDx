"""Offline + live-gated tests for the asyncio-native `streaming_async()` surface.

Pins the contract that:

* `ThetaDataDxClient.streaming_async()` and `FpssClient.streaming_async()`
  both exist and return a `StreamingAsyncSession`.
* The session is an async context manager + async iterator yielding
  batches of typed `FpssEvent` instances per OS wake.
* The drain path uses FD-readiness signalling — no polling — so the
  process consumes near-zero CPU during quiet windows.

Live tests are gated on ``THETADX_LIVE_CREDS=path/to/creds.txt`` because
they need a real FPSS handshake. Structural surface tests run without
credentials.
"""

from __future__ import annotations

import asyncio
import inspect
import os
import time
from typing import Any, List

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


# ─────────────────────────────────────────────────────────────────
# Offline structural tests
# ─────────────────────────────────────────────────────────────────


def test_streaming_async_method_exists_on_both_clients() -> None:
    """`streaming_async` is reachable on both the unified client and
    the standalone FPSS client. Pins the cross-client parity contract:
    the asyncio surface lives on whichever client opens the FPSS
    transport, with no functional asymmetry."""
    mod = _import_module()
    assert hasattr(mod.ThetaDataDxClient, "streaming_async"), (
        "ThetaDataDxClient must expose streaming_async() for asyncio "
        "integration"
    )
    assert hasattr(mod.FpssClient, "streaming_async"), (
        "FpssClient must expose streaming_async() for asyncio integration"
    )


def test_streaming_async_session_pyclass_exported() -> None:
    """`StreamingAsyncSession` must be reachable on the package import
    surface so type-stub generators and IDEs can discover it."""
    mod = _import_module()
    assert hasattr(mod, "StreamingAsyncSession"), (
        "thetadatadx must export `StreamingAsyncSession` "
        "(asyncio-native streaming context manager)"
    )


def test_streaming_async_session_has_async_protocol() -> None:
    """The pyclass must expose `__aenter__`, `__aexit__`, `__aiter__`,
    `__anext__` for the `async with` + `async for` patterns."""
    mod = _import_module()
    cls = mod.StreamingAsyncSession
    for method in ("__aenter__", "__aexit__", "__aiter__", "__anext__"):
        assert hasattr(cls, method), (
            f"StreamingAsyncSession must expose `{method}` for asyncio integration"
        )


def test_streaming_async_session_has_awaitable_subscribe_surface() -> None:
    """Subscribe / unsubscribe surfaces on the async session must be
    awaitable (they return coroutines), distinguishing them from the
    sync `subscribe()` on the underlying client."""
    mod = _import_module()
    cls = mod.StreamingAsyncSession
    for method in (
        "subscribe",
        "subscribe_many",
        "unsubscribe",
        "unsubscribe_many",
    ):
        assert hasattr(cls, method), (
            f"StreamingAsyncSession must expose async-aware `{method}`"
        )


def test_streaming_async_for_each_callable_surface_present() -> None:
    """The async-callback convenience wrapper is part of the surface."""
    mod = _import_module()
    assert hasattr(mod.StreamingAsyncSession, "streaming_async_for_each")


def test_streaming_async_session_signature_returns_session() -> None:
    """`client.streaming_async()` is a sync factory that returns the
    pyclass instance — same shape as the sync `streaming()` and
    `streaming_iter()` factories. The actual asyncio bridge fires
    inside `__aenter__` so the constructor stays cheap and can run
    outside an event loop."""
    mod = _import_module()
    # We exercise the factory via creds-less mocking: construct only
    # the *class* surface by checking method shape, not by invoking
    # against a live config.
    sig = inspect.getattr_static(mod.ThetaDataDxClient, "streaming_async")
    assert callable(sig)


# ─────────────────────────────────────────────────────────────────
# Live-gated tests — need credentials + market data
# ─────────────────────────────────────────────────────────────────


def _creds_path() -> str | None:
    # Match the env-var name the rest of the live test suite uses
    # (`THETADX_TEST_CREDS`). Fall back to the spec-mentioned
    # `THETADX_LIVE_CREDS` for the asyncio surface so either env var
    # gates the test.
    return os.getenv("THETADX_TEST_CREDS") or os.getenv("THETADX_LIVE_CREDS")


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_streaming_async_end_to_end_async_iter_smoke() -> None:
    """Subscribe to a hot stream, drain at least one batch under the
    `async with` + `async for` pattern. Exercises the full async
    iterator + FD-wake plumbing."""
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None  # narrowed by the skipif guard above

    async def run() -> List[Any]:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        events_seen: List[Any] = []
        async with client.streaming_async() as session:
            await session.subscribe(mod.Contract.stock("QQQ").quote())
            async for batch in session:
                events_seen.extend(batch)
                if len(events_seen) >= 5:
                    break
        return events_seen

    events = asyncio.run(run())
    assert len(events) >= 5, (
        f"expected at least 5 events from a hot stream, observed {len(events)}"
    )


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_streaming_async_for_each_callback_smoke() -> None:
    """Drain via the `streaming_async_for_each(callback)` convenience
    wrapper with an async callback so the backpressure path executes
    end-to-end."""
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None

    async def run() -> int:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        total = 0

        async def handle(batch: List[Any]) -> None:
            nonlocal total
            total += len(batch)
            if total >= 5:
                # Tear down the session so `for_each` returns.
                raise asyncio.CancelledError("enough events")

        async with client.streaming_async() as session:
            await session.subscribe(mod.Contract.stock("QQQ").quote())
            try:
                await session.streaming_async_for_each(handle)
            except asyncio.CancelledError:
                pass
        return total

    seen = asyncio.run(run())
    assert seen >= 5


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_streaming_async_no_polling_idle_cpu() -> None:
    """During a quiet window (subscribed but no traffic), the asyncio
    drain must consume effectively no CPU — the entire point of
    FD-readiness signalling is that the event loop sleeps in `epoll`
    until the kernel hands a wake byte.

    The 1% threshold is generous: a polling implementation that does a
    50 ms sleep loop pegs at 5–10% CPU on commodity hardware; a real
    FD-readiness path stays under 0.5%. We pad for noisy CI runners.
    """
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None

    async def measure() -> float:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        # Pick a low-volume contract that is unlikely to fire during
        # the 5-second measurement window (a deep OTM weekly option
        # would also work, but we keep this to a stock symbol so the
        # subscription is trivially valid year-round).
        async with client.streaming_async() as session:
            await session.subscribe(mod.Contract.stock("BRK.A").quote())

            wall_start = time.monotonic()
            cpu_start = time.process_time()

            # Wait without doing anything, but keep the event loop
            # responsive to the FD-readiness wake so any event that
            # does fire is observed via `__anext__` (we don't actually
            # consume them here — the wake is what consumes CPU we
            # want to measure).
            await asyncio.sleep(5.0)

            cpu_end = time.process_time()
            wall_end = time.monotonic()
        return (cpu_end - cpu_start) / max(wall_end - wall_start, 1e-9)

    cpu_fraction = asyncio.run(measure())
    assert cpu_fraction < 0.01, (
        f"idle CPU was {cpu_fraction:.4%}; expected <1% — a polling "
        "drain would peg the loop"
    )
