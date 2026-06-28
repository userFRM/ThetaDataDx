"""
Ring-occupancy observability surface test.

Pins the contract that ``client.stream.ring_occupancy()`` and
``client.stream.ring_capacity()`` are callable across the streaming lifecycle
(pre-start / post-start / post-stop), return non-negative ints
everywhere, and read 0 when streaming has not started.

The pair is the leading back-pressure signal: occupancy is a
point-in-time sample of events published into the streaming event
ring but not yet drained into the callback, and capacity is the
configured ``streaming_ring_size``. ``dropped_event_count()`` only moves
AFTER data has been lost; a rising occupancy approaching capacity
predicts those drops. Both forward through
``thetadatadx::Client`` so the values match every other
binding.

This shape mirrors the TypeScript binding's
``__tests__/ring_occupancy.test.mjs`` (and the existing
``test_dropped_events.py``) to keep the public contract identical
across SDKs.

Surface-existence checks run offline against the pyclass types; the
lifecycle checks are gated on ``THETADATADX_TEST_CREDS=path/to/creds.txt``
because ``Client`` needs a live FPSS handshake, mirroring
the dropped-events test.

What this test does NOT assert:

* a specific non-zero occupancy on a live feed. Occupancy is a racy
  point-in-time sample of a fast consumer; on a healthy session it
  hovers at 0 and any other value is timing-dependent.
* capacity equality with a hardcoded default. The configured ring
  size is an operator knob; the test asserts shape (power of two,
  positive while live) rather than freezing the default.
"""

from __future__ import annotations

import os

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
    thetadatadx = _import_module()

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
    """Minimal callback for lifecycle tests."""


# ── Offline: surface existence on the pyclass types ───────────────────


def test_ring_occupancy_surface_exists_offline() -> None:
    """Both getters must exist on the unified client's `StreamView`
    sub-namespace (reached via ``client.stream``) AND the standalone
    `StreamingClient` pyclass, mirroring `dropped_event_count`. Method
    presence on the type is checkable without a live connection."""
    mod = _import_module()
    for cls_name in ("StreamView", "StreamingClient"):
        cls = getattr(mod, cls_name)
        assert hasattr(cls, "ring_occupancy"), (
            f"{cls_name} must expose ring_occupancy() alongside "
            f"dropped_event_count()"
        )
        assert hasattr(cls, "ring_capacity"), (
            f"{cls_name} must expose ring_capacity() alongside "
            f"dropped_event_count()"
        )


# ── Live: lifecycle accessibility (creds-gated) ───────────────────────


def test_ring_occupancy_zero_before_streaming(client) -> None:
    """Both getters must be callable before `start_streaming(callback)`
    and return 0 -- the streaming slot is empty, so the wrappers
    forward 0 from the unified client."""
    occupancy = client.stream.ring_occupancy()
    capacity = client.stream.ring_capacity()
    assert isinstance(occupancy, int)
    assert isinstance(capacity, int)
    assert occupancy == 0, "pre-stream occupancy must be 0 -- no ring exists"
    assert capacity == 0, "pre-stream capacity must be 0 -- no ring exists"


def test_ring_occupancy_lifecycle_callable(client) -> None:
    """The pair must remain callable across the full lifecycle:
    pre-start / post-start / post-stop. While streaming, capacity is
    the configured ring size (a positive power of two) and occupancy
    is bounded by it."""
    client.stream.start_streaming(_noop_callback)

    capacity = client.stream.ring_capacity()
    assert isinstance(capacity, int)
    assert capacity > 0, "a live ring must report its configured capacity"
    assert capacity & (capacity - 1) == 0, "ring capacity is a power of two"

    occupancy = client.stream.ring_occupancy()
    assert isinstance(occupancy, int)
    assert 0 <= occupancy <= capacity, (
        "occupancy is clamped non-negative and never exceeds capacity"
    )

    client.stream.stop_streaming()
    # After stop_streaming the streaming slot is empty; both getters
    # return 0 in that state.
    assert client.stream.ring_occupancy() == 0
    assert client.stream.ring_capacity() == 0
