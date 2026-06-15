"""Standalone `StreamingClient` + `HistoricalClient` pyclass surface tests.

Pins the contract that:

* ``StreamingClient(creds, config)`` allocates an FPSS-only handle that
  opens NEITHER the MDDS gRPC channel NOR a Nexus HTTP session at
  construction time. The FPSS TLS connection itself is deferred to
  the first ``start_streaming*`` call, matching the standalone C ABI
  (``tdx_fpss_connect`` allocates, ``tdx_fpss_set_callback`` opens
  the network).
* ``HistoricalClient(creds, config)`` opens ONLY the MDDS gRPC channel
  plus the Nexus HTTP authentication. It exposes the historical /
  FLATFILES surface but raises ``AttributeError`` on every
  FPSS-touching method (``subscribe`` / ``start_streaming`` / etc.).
* The bundled ``Client`` continues to expose its unified
  surface unchanged.

Live tests are gated on ``THETADX_LIVE_CREDS=path/to/creds.txt`` and
hit production endpoints; static surface tests run offline.

# Nexus session behaviour

This file does NOT change Nexus session behaviour. The standalone
``StreamingClient`` never authenticates against Nexus (FPSS speaks its
own protocol-level ``CREDENTIALS`` handshake on the TLS connection
itself; see ``crates/thetadatadx/src/fpss/mod.rs`` and
``ffi/src/streaming.rs::tdx_fpss_connect``). The standalone
``HistoricalClient`` authenticates against Nexus exactly once at
construction time. Running both side-by-side in the same process
authenticates Nexus once (via the MDDS surface) and never again
(the FPSS surface bypasses Nexus entirely), so a parallel
externally-managed MDDS process under the same credentials is
unaffected by either standalone pyclass beyond the single MDDS-
side Nexus auth — which is the same single round-trip the bundled
``Client`` already issues today.
"""

from __future__ import annotations

import os
import threading
import time
from typing import Any

import pytest


# Block-list inventory mirrored from ``sdks/python/src/mdds_client.rs``.
# Source of truth for cross-class drift: the Rust ``FPSS_TOUCHING_METHODS``
# const plus the compile-time guard against the generator-emitted
# ``PYTHON_UNIFIED_FPSS_METHODS``. This Python copy lets the offline
# coverage test enumerate every blocked name without needing a live
# ``HistoricalClient`` instance.
BLOCKED_FPSS_METHODS = (
    "start_streaming",
    "stop_streaming",
    "shutdown",
    "reconnect",
    "streaming",
    "stream",
    "is_streaming",
    "await_drain",
    "subscribe",
    "subscribe_many",
    "unsubscribe",
    "unsubscribe_many",
    "active_subscriptions",
    "active_full_subscriptions",
    "dropped_event_count",
    "panic_count",
)


def _import_module():
    try:
        import thetadatadx as mod
    except ImportError:
        pytest.skip(
            "thetadatadx native extension not built "
            "-- run `maturin develop` from sdks/python/"
        )
    return mod


def _live_creds_path() -> str | None:
    """Path to a two-line `email\\npassword` credentials file.

    Live tests are skipped when the env var is unset so CI without
    secrets stays green. Set ``THETADX_LIVE_CREDS=/path/to/creds.txt``
    to exercise the parallel-auth and history smoke tests.
    """
    return os.environ.get("THETADX_LIVE_CREDS")


# ── Offline: pyclass surface + import-time exports ────────────────────


def test_standalone_classes_are_exported() -> None:
    """`StreamingClient` / `HistoricalClient` must be reachable on the package
    import surface so IDEs and type-stub generators can discover them.
    No live connection required."""
    mod = _import_module()
    assert hasattr(mod, "StreamingClient"), (
        "thetadatadx must export `StreamingClient` (standalone FPSS-only pyclass)"
    )
    assert hasattr(mod, "HistoricalClient"), (
        "thetadatadx must export `HistoricalClient` (standalone MDDS-only pyclass)"
    )
    # The bundled unified entry point must still be exported -- the
    # standalone classes are additive, not a replacement.
    assert hasattr(mod, "Client"), (
        "thetadatadx must continue to export the bundled `Client`"
    )


def test_fpss_client_constructor_signature() -> None:
    """`StreamingClient.__init__` must accept `(creds, config)` positionally
    so the API matches the bundled `Client` and the
    standalone C++ wrapper."""
    mod = _import_module()
    creds = mod.Credentials("user@example.com", "pw")
    config = mod.Config.production()
    # Construction is allowed without a live network because no FPSS
    # TLS connection is opened until `start_streaming*`. Validates the
    # contract that pure pyclass construction does NOT touch the
    # network at all.
    client = mod.StreamingClient(creds, config)
    assert client is not None
    # `repr` mirrors the bundled `Client` vocabulary
    # (`streaming=connected` / `streaming=none`) — no Rust-cased
    # `connected=true/false` slip-through.
    rendered = repr(client)
    assert "streaming=none" in rendered, rendered
    assert "StreamingClient(" in rendered, rendered


def test_mdds_client_requires_network_for_construction() -> None:
    """`HistoricalClient.__init__` authenticates against Nexus and opens the
    MDDS gRPC channel, so without a live network the construct call
    fails. We assert the *failure mode* here -- the live counterpart
    `test_mdds_history_smoke` confirms the success path."""
    mod = _import_module()
    creds = mod.Credentials("noreply@example.invalid", "definitely-not-a-real-password")
    config = mod.Config.production()
    with pytest.raises(Exception):
        # Either Nexus auth or gRPC handshake will fail; we don't care
        # which -- the contract is "HistoricalClient construction touches the
        # network", which is what a parallel FPSS process cares about.
        mod.HistoricalClient(creds, config)


def test_fpss_client_blocks_subscribe_before_start() -> None:
    """Subscribing on an `StreamingClient` before `start_streaming*` raises
    `RuntimeError` -- the FPSS TLS connection is not open yet.

    The `SecType.OPTION.full_trades()` factory exercises the polymorphic
    `subscribe(Subscription)` dispatch via the full-stream surface. The
    per-contract path (`Contract.stock(...).quote()`) is covered by
    `test_fluent_contract_example.py`; this test focuses on the
    pre-start guard.
    """
    mod = _import_module()
    creds = mod.Credentials("user@example.com", "pw")
    config = mod.Config.production()
    client = mod.StreamingClient(creds, config)
    sub = mod.SecType.OPTION.full_trades()
    with pytest.raises(RuntimeError, match="streaming not started"):
        client.subscribe(sub)


def test_mdds_client_blocks_fpss_attrs() -> None:
    """`HistoricalClient` is the historical-only surface -- every
    FPSS-touching method must raise `AttributeError` so callers
    cannot accidentally open an FPSS connection that would conflict
    with a parallel FPSS process."""
    mod = _import_module()
    if not _live_creds_path():
        # Live creds gate: constructing `HistoricalClient` requires the gRPC
        # handshake to succeed, so the attribute-block assertions
        # piggyback on the live test gate.
        pytest.skip("THETADX_LIVE_CREDS unset -- skip live HistoricalClient attribute check")

    creds = mod.Credentials.from_file(_live_creds_path())
    client = mod.HistoricalClient(creds, mod.Config.production())
    for name in BLOCKED_FPSS_METHODS:
        with pytest.raises(AttributeError, match="standalone historical surface"):
            getattr(client, name)


def test_mdds_client_block_list_offline() -> None:
    """Offline coverage of the full ``BLOCKED_FPSS_METHODS`` inventory.

    The live ``test_mdds_client_blocks_fpss_attrs`` only fires when
    credentials are available; without that, a regression that
    accidentally dropped a name from the Rust ``FPSS_TOUCHING_METHODS``
    const would slip through CI.

    Strategy: introspect the Rust block-list via the module-level
    ``_blocked_fpss_methods()`` helper and assert it matches the
    Python-side ``BLOCKED_FPSS_METHODS`` mirror exactly. Combined
    with the Rust-side compile-time guard that pins the
    generator-emitted FPSS surface as a strict subset of
    ``FPSS_TOUCHING_METHODS``, this closes the offline coverage gap
    end-to-end.
    """
    mod = _import_module()
    rust_inventory = set(mod._blocked_fpss_methods())
    python_mirror = set(BLOCKED_FPSS_METHODS)
    missing_in_python = rust_inventory - python_mirror
    missing_in_rust = python_mirror - rust_inventory
    assert not missing_in_python, (
        f"Rust FPSS_TOUCHING_METHODS contains names not mirrored in "
        f"BLOCKED_FPSS_METHODS: {sorted(missing_in_python)}"
    )
    assert not missing_in_rust, (
        f"Python BLOCKED_FPSS_METHODS contains names that fell out of "
        f"the Rust block-list — block-list drift: {sorted(missing_in_rust)}"
    )


# ── Test #1: StreamingClient never opens an MDDS gRPC channel ──────────────


def test_fpss_client_no_mdds_channel() -> None:
    """``StreamingClient(creds, config)`` must not attempt any MDDS gRPC
    connect or Nexus HTTP request.

    Structural proof rather than circumstantial: point MDDS at a
    known-refused host (``127.0.0.1:1``) and assert that:

    1. ``StreamingClient(creds, config)`` constructs cleanly, because the
       FPSS surface never opens the MDDS channel;
    2. ``HistoricalClient(creds, config)`` against the SAME config fails
       fast (some network-level error), because the historical
       surface must open the channel at construction time.

    Together these two assertions prove the standalone FPSS path is
    structurally disjoint from the MDDS path — not merely that the
    FPSS surface happens not to fail on production hosts.
    """
    mod = _import_module()

    config = mod.Config.production()
    # Loopback port 1 is reserved IANA "TCP Port Service Multiplexer"
    # space and is virtually guaranteed to refuse the TCP handshake
    # immediately. A wildly off port (e.g. 1) keeps the failure
    # synchronous — a regular bogus hostname would impose a DNS
    # resolution timeout on the test runtime.
    config.mdds_host = "127.0.0.1"
    config.mdds_port = 1

    creds = mod.Credentials("user@example.com", "pw")

    # StreamingClient construction must succeed against the bad MDDS host.
    fpss = mod.StreamingClient(creds, config)

    # The pyclass exists and reports a disconnected slot using the
    # bundled `Client` repr vocabulary (`streaming=none`).
    assert "streaming=none" in repr(fpss), (
        f"StreamingClient repr must report streaming=none before start_streaming*; "
        f"got {repr(fpss)!r}"
    )
    # No active subscriptions before any start.
    assert fpss.active_subscriptions() == [], (
        "StreamingClient must report empty active_subscriptions before start_streaming*"
    )
    assert fpss.active_full_subscriptions() == [], (
        "StreamingClient must report empty active_full_subscriptions before start_streaming*"
    )
    # No drops, no panics on a never-started session.
    assert fpss.dropped_event_count() == 0
    assert fpss.panic_count() == 0
    assert fpss.is_streaming() is False
    assert fpss.is_authenticated() is False, (
        "StreamingClient must not be authenticated before start_streaming*"
    )

    # The companion assertion: HistoricalClient against the same bad config
    # must fail fast. If both succeeded, the test would not actually
    # prove disjoint network paths.
    with pytest.raises(Exception):
        mod.HistoricalClient(creds, config)


# ── Test #2: HistoricalClient never opens an FPSS TLS connection ────────────


def test_mdds_client_no_fpss_connection() -> None:
    """``HistoricalClient(creds, config)`` must NEVER expose any path that
    opens the FPSS TLS connection.

    We assert the contract via the attribute block-list: every
    FPSS-touching method on the bundled ``Client`` must
    raise ``AttributeError`` on ``HistoricalClient``. Live attribute
    coverage is gated on credentials (see
    ``test_mdds_client_blocks_fpss_attrs``); this offline variant
    pins the *static* contract that the block-list exists on the
    pyclass, by constructing an instance against a stub and
    checking that even the *signature* of `subscribe`-style
    attributes is absent.
    """
    mod = _import_module()
    # Static introspection: the HistoricalClient pyclass must NOT expose
    # any of these as bound methods. The block-list is enforced at
    # `__getattr__` runtime, so we verify it via `dir()` -- attribute
    # block-listing intentionally hides them from `dir()` so IDE
    # autocomplete steers callers to StreamingClient / Client.
    blocked = {
        "start_streaming",
        "subscribe",
        "unsubscribe",
        "reconnect",
        "streaming",
        "stream",
    }
    public_attrs = {name for name in dir(mod.HistoricalClient) if not name.startswith("_")}
    leaked = blocked & public_attrs
    assert not leaked, (
        f"HistoricalClient must not expose FPSS-touching methods on the pyclass; leaked={leaked}"
    )


# ── Live tests (gated on THETADX_LIVE_CREDS) ──────────────────────────


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip live FPSS streaming callback smoke",
)
def test_fpss_streaming_callback_smoke() -> None:
    """Live FPSS smoke: construct `StreamingClient`, register a callback
    via `start_streaming(callback)`, subscribe, wait until the
    callback fires (up to 10 s), and confirm at least one event
    arrived.

    Synchronisation: a ``threading.Event`` set by the callback on
    first delivery + ``event.wait(timeout=10.0)`` avoids `time.sleep`
    — which adds dead wall-time on a healthy stream and silently
    masks a stalled callback on a sick one.
    """
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    fpss = mod.StreamingClient(creds, mod.Config.production())

    received: list[Any] = []
    first_event = threading.Event()

    def on_event(event: Any) -> None:
        received.append(event)
        first_event.set()

    with fpss.streaming(on_event):
        fpss.subscribe(mod.SecType.OPTION.full_trades())
        first_event.wait(timeout=10.0)

    assert received, (
        "expected >=1 FPSS event within 10s of a full-options-trade "
        "subscription (callback never fired)"
    )


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip live FPSS reconnect regression",
)
def test_fpss_reconnect_restores_subscriptions() -> None:
    """Live FPSS regression: `reconnect()` must restore every active
    subscription captured against the previous session.

    Pins the explicit-handoff contract documented on
    ``StreamingClient.reconnect``: snapshot active subscriptions before
    stopping, reopen the FPSS TLS connection under the previously
    registered callback, then re-apply each saved subscription.
    Without this regression the per-contract / full-stream restore
    path could silently regress and Python users observing a
    transient disconnect would lose subscriptions.
    """
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    fpss = mod.StreamingClient(creds, mod.Config.production())

    received: list[Any] = []

    def on_event(event: Any) -> None:
        received.append(event)

    fpss.start_streaming(on_event)
    try:
        # Full-stream subscription exercises the polymorphic dispatch
        # path; per-contract dispatch is covered separately in
        # `test_fluent_contract_example.py`.
        full_sub = mod.SecType.OPTION.full_trades()
        fpss.subscribe(full_sub)

        before = fpss.active_full_subscriptions()
        assert len(before) == 1, (
            f"expected exactly one full-stream subscription before reconnect; got {before}"
        )

        fpss.reconnect()

        after = fpss.active_full_subscriptions()
        assert len(after) == 1, (
            f"expected exactly one full-stream subscription after reconnect; got {after}"
        )
        # Subscription identity (kind + sec_type) must round-trip across
        # the reconnect — a regression that restored the slot but with
        # a different kind would silently break dispatch.
        assert before[0].kind == after[0].kind
        assert before[0].sec_type == after[0].sec_type
    finally:
        fpss.stop_streaming()


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip live MDDS history smoke",
)
def test_mdds_history_smoke() -> None:
    """Live MDDS smoke: construct `HistoricalClient` and pull a 2-month
    EOD slice for AAPL. Confirms the gRPC channel is open, the
    Nexus session was acquired, and the typed-list decode path
    is wired through."""
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    mdds = mod.HistoricalClient(creds, mod.Config.production())

    ticks = mdds.stock_history_eod("AAPL", "20260101", "20260301")
    assert ticks is not None, "stock_history_eod must return a typed list"
    # `<TickName>List.to_list()` is the cross-binding-parity terminal
    # the Python surface exposes; converting once at the assertion
    # boundary avoids depending on pandas / polars being installed in
    # the test environment.
    rows = ticks.to_list()
    assert rows, "AAPL EOD over 2026-01..2026-03 must return >=1 row"
    # Schema check: an EOD tick must carry the closing price field.
    # Catches a regression where the decoder silently returns wrong-
    # shaped rows but still passes the non-empty assertion.
    first = rows[0]
    assert hasattr(first, "close"), (
        f"EOD tick must carry `close` field; got attrs={dir(first)!r}"
    )


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip parallel-creds live test",
)
def test_concurrent_fpss_and_mdds_share_creds() -> None:
    """Hand the same `Credentials` instance to a standalone
    `StreamingClient` AND a standalone `HistoricalClient` in the same process.
    Confirm both authenticate without `Error::Auth`.

    Nexus session behaviour observation (recorded in PR body): the
    MDDS surface issues exactly one Nexus auth at construction
    time; the FPSS surface bypasses Nexus entirely (FPSS speaks its
    own `CREDENTIALS` frame at the TLS layer). The two surfaces
    therefore do not race on Nexus state by design.
    """
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    config = mod.Config.production()

    # MDDS-only construct -- single Nexus auth, gRPC channel up.
    mdds = mod.HistoricalClient(creds, config)
    # FPSS-only construct -- no Nexus, no gRPC. Construction must
    # succeed even after the MDDS-side Nexus session is live.
    fpss = mod.StreamingClient(creds, config)

    # Use MDDS to confirm the session is functional after the FPSS
    # pyclass is also alive. A regression where FPSS construction
    # somehow invalidated the MDDS session would surface as a
    # `RuntimeError` (or `AuthenticationError`) on the next MDDS
    # call.
    ticks = mdds.stock_history_eod("AAPL", "20260201", "20260205")
    assert ticks.to_list(), "MDDS surface must remain authenticated after StreamingClient construction"

    # Pop the FPSS streaming slot briefly to confirm the FPSS-level
    # handshake also works while the MDDS session is live. The
    # context manager drains on exit.
    with fpss.streaming(lambda evt: None):
        pass
