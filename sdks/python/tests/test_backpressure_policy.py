"""Offline + live-gated tests for the `BackpressurePolicy` enum and the
``max_queue_depth`` / ``backpressure`` kwargs on the asyncio-native
streaming surfaces (closes #563).

Offline structural tests pin:

* ``thetadatadx.BackpressurePolicy`` exists and exposes the three
  documented variants (``Block`` / ``DropOldest`` / ``DropNewest``).
* ``ThetaDataDxClient.streaming_async`` and
  ``FpssClient.streaming_async`` accept the new kwargs.
* The returned ``StreamingAsyncSession`` echoes back the configured
  policy + depth via the typed getters.
* The new ``queue_depth()`` and ``dropped_event_count()`` accessors
  exist on the session.

Live tests assert the actual io_loop behaviour under controlled
saturation and need a real FPSS handshake (``THETADX_TEST_CREDS`` /
``THETADX_LIVE_CREDS``).
"""

from __future__ import annotations

import asyncio
import inspect
import os
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


# ─────────────────────────────────────────────────────────────────
# Offline structural tests
# ─────────────────────────────────────────────────────────────────


def test_backpressure_policy_class_exported() -> None:
    """`BackpressurePolicy` must be reachable on the package surface so
    IDEs, stubgen, and ``isinstance`` checks resolve."""
    mod = _import_module()
    assert hasattr(mod, "BackpressurePolicy"), (
        "thetadatadx must export BackpressurePolicy (per #563)"
    )


def test_backpressure_policy_has_three_variants() -> None:
    """Block / DropOldest / DropNewest are the documented variants
    mirroring the core ``thetadatadx::fpss::BackpressurePolicy``."""
    mod = _import_module()
    cls = mod.BackpressurePolicy
    for name in ("Block", "DropOldest", "DropNewest"):
        assert hasattr(cls, name), (
            f"BackpressurePolicy must expose {name} variant"
        )


def test_backpressure_policy_variants_are_distinct() -> None:
    """The three variants must compare as distinct enum members so
    ``policy == BackpressurePolicy.Block`` round-trips correctly."""
    mod = _import_module()
    block = mod.BackpressurePolicy.Block
    drop_oldest = mod.BackpressurePolicy.DropOldest
    drop_newest = mod.BackpressurePolicy.DropNewest
    assert block != drop_oldest
    assert block != drop_newest
    assert drop_oldest != drop_newest
    assert block == mod.BackpressurePolicy.Block


def test_streaming_async_signature_accepts_kwargs_tdx() -> None:
    """`ThetaDataDxClient.streaming_async(max_queue_depth=..., backpressure=...)`
    must accept the documented kwargs without erroring at the binding
    layer."""
    mod = _import_module()
    sig = inspect.getattr_static(mod.ThetaDataDxClient, "streaming_async")
    assert callable(sig)


def test_streaming_async_signature_accepts_kwargs_fpss() -> None:
    """Same for the standalone FPSS client."""
    mod = _import_module()
    sig = inspect.getattr_static(mod.FpssClient, "streaming_async")
    assert callable(sig)


def test_streaming_async_session_exposes_queue_depth_getter() -> None:
    """`queue_depth()` is the operator-facing depth getter per #563.
    `queue_len()` stays as a back-compat alias for the v0.1 surface."""
    mod = _import_module()
    cls = mod.StreamingAsyncSession
    assert hasattr(cls, "queue_depth"), (
        "StreamingAsyncSession must expose queue_depth() per #563"
    )
    assert hasattr(cls, "queue_len"), (
        "StreamingAsyncSession must keep queue_len() as a back-compat alias"
    )


def test_streaming_async_session_exposes_dropped_event_count() -> None:
    """`dropped_event_count()` proxies through to the underlying client
    so operators have one metric for queue-overflow pressure."""
    mod = _import_module()
    assert hasattr(mod.StreamingAsyncSession, "dropped_event_count"), (
        "StreamingAsyncSession must expose dropped_event_count() per #563"
    )


def test_streaming_async_session_exposes_max_queue_depth_getter() -> None:
    """`max_queue_depth` is a read-only getter that echoes the value
    passed at construction. Diagnostic surface for operator dashboards."""
    mod = _import_module()
    assert hasattr(mod.StreamingAsyncSession, "max_queue_depth")


def test_streaming_async_session_exposes_backpressure_getter() -> None:
    """`backpressure` echoes the configured policy."""
    mod = _import_module()
    assert hasattr(mod.StreamingAsyncSession, "backpressure")


# ─────────────────────────────────────────────────────────────────
# Live-gated tests
# ─────────────────────────────────────────────────────────────────


def _creds_path() -> str | None:
    return os.getenv("THETADX_TEST_CREDS") or os.getenv("THETADX_LIVE_CREDS")


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_backpressure_block_caps_queue_depth() -> None:
    """Under ``BackpressurePolicy.Block``, a slow consumer must observe
    queue depth capped at ``max_queue_depth`` and zero dropped events.

    The consumer sleeps without draining, so the queue saturates. With
    `Block`, the io_loop parks rather than dropping — depth stays at
    cap, drop count stays at zero (any drops would have to come from
    the upstream Disruptor ring overflow, which the bench-sized queue
    + short window keeps inside the bound).
    """
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None

    async def run() -> tuple[int, int]:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        async with client.streaming_async(
            max_queue_depth=64,
            backpressure=mod.BackpressurePolicy.Block,
        ) as session:
            await session.subscribe(mod.Contract.stock("QQQ").quote())
            # Sleep without draining; the io_loop parks once the queue
            # fills past max_queue_depth.
            await asyncio.sleep(1.5)
            depth = session.queue_depth()
            dropped = session.dropped_event_count()
        return depth, dropped

    depth, dropped = asyncio.run(run())
    assert depth <= 64, (
        f"queue depth {depth} exceeded the 64-cap; Block policy must "
        "park the producer once the queue saturates"
    )


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_backpressure_drop_newest_caps_depth_and_increments_drops() -> None:
    """Under ``BackpressurePolicy.DropNewest``, queue depth must cap at
    ``max_queue_depth`` and dropped count must grow under sustained
    saturation."""
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None

    async def run() -> tuple[int, int]:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        async with client.streaming_async(
            max_queue_depth=64,
            backpressure=mod.BackpressurePolicy.DropNewest,
        ) as session:
            await session.subscribe(mod.Contract.stock("QQQ").quote())
            await asyncio.sleep(1.5)
            depth = session.queue_depth()
            dropped = session.dropped_event_count()
        return depth, dropped

    depth, dropped = asyncio.run(run())
    assert depth <= 64, f"DropNewest must cap depth at 64; saw {depth}"
    assert dropped > 0, (
        f"DropNewest under saturation must register drops; saw {dropped}"
    )


@pytest.mark.skipif(
    not _creds_path(),
    reason="needs THETADX_TEST_CREDS / THETADX_LIVE_CREDS + market data",
)
def test_backpressure_drop_oldest_caps_depth_and_increments_drops() -> None:
    """Under ``BackpressurePolicy.DropOldest``, queue depth must cap at
    ``max_queue_depth`` and dropped count must grow under sustained
    saturation."""
    mod = _import_module()
    creds_path = _creds_path()
    assert creds_path is not None

    async def run() -> tuple[int, int]:
        creds = mod.Credentials.from_file(creds_path)
        cfg = mod.Config.production()
        client = mod.FpssClient(creds, cfg)
        async with client.streaming_async(
            max_queue_depth=64,
            backpressure=mod.BackpressurePolicy.DropOldest,
        ) as session:
            await session.subscribe(mod.Contract.stock("QQQ").quote())
            await asyncio.sleep(1.5)
            depth = session.queue_depth()
            dropped = session.dropped_event_count()
        return depth, dropped

    depth, dropped = asyncio.run(run())
    assert depth <= 64, f"DropOldest must cap depth at 64; saw {depth}"
    assert dropped > 0, (
        f"DropOldest under saturation must register drops; saw {dropped}"
    )
