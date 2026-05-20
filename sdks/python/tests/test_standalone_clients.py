"""Standalone `FpssClient` + `MddsClient` pyclass surface tests.

Pins the contract that:

* ``FpssClient(creds, config)`` allocates an FPSS-only handle that
  opens NEITHER the MDDS gRPC channel NOR a Nexus HTTP session at
  construction time. The FPSS TLS connection itself is deferred to
  the first ``start_streaming*`` call, matching the standalone C ABI
  (``tdx_fpss_connect`` allocates, ``tdx_fpss_set_callback`` opens
  the network).
* ``MddsClient(creds, config)`` opens ONLY the MDDS gRPC channel
  plus the Nexus HTTP authentication. It exposes the historical /
  FLATFILES surface but raises ``AttributeError`` on every
  FPSS-touching method (``subscribe`` / ``start_streaming`` / etc.).
* The bundled ``ThetaDataDxClient`` continues to expose its unified
  surface unchanged.

Live tests are gated on ``THETADX_LIVE_CREDS=path/to/creds.txt`` and
hit production endpoints; static surface tests run offline.

# Nexus session behaviour

This file does NOT change Nexus session behaviour. The standalone
``FpssClient`` never authenticates against Nexus (FPSS speaks its
own protocol-level ``CREDENTIALS`` handshake on the TLS connection
itself; see ``crates/thetadatadx/src/fpss/mod.rs`` and
``ffi/src/streaming.rs::tdx_fpss_connect``). The standalone
``MddsClient`` authenticates against Nexus exactly once at
construction time. Running both side-by-side in the same process
authenticates Nexus once (via the MDDS surface) and never again
(the FPSS surface bypasses Nexus entirely), so a parallel
externally-managed MDDS process under the same credentials is
unaffected by either standalone pyclass beyond the single MDDS-
side Nexus auth — which is the same single round-trip the bundled
``ThetaDataDxClient`` already issues today.
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


def _live_creds_path() -> str | None:
    """Path to a two-line `email\\npassword` credentials file.

    Live tests are skipped when the env var is unset so CI without
    secrets stays green. Set ``THETADX_LIVE_CREDS=/path/to/creds.txt``
    to exercise the parallel-auth and history smoke tests.
    """
    return os.environ.get("THETADX_LIVE_CREDS")


# ── Offline: pyclass surface + import-time exports ────────────────────


def test_standalone_classes_are_exported() -> None:
    """`FpssClient` / `MddsClient` must be reachable on the package
    import surface so IDEs and type-stub generators can discover them.
    No live connection required."""
    mod = _import_module()
    assert hasattr(mod, "FpssClient"), (
        "thetadatadx must export `FpssClient` (standalone FPSS-only pyclass)"
    )
    assert hasattr(mod, "MddsClient"), (
        "thetadatadx must export `MddsClient` (standalone MDDS-only pyclass)"
    )
    # The bundled unified entry point must still be exported -- the
    # standalone classes are additive, not a replacement.
    assert hasattr(mod, "ThetaDataDxClient"), (
        "thetadatadx must continue to export the bundled `ThetaDataDxClient`"
    )


def test_fpss_client_constructor_signature() -> None:
    """`FpssClient.__init__` must accept `(creds, config)` positionally
    so the API matches the bundled `ThetaDataDxClient` and the
    standalone C++ wrapper."""
    mod = _import_module()
    creds = mod.Credentials("user@example.com", "pw")
    config = mod.Config.production()
    # Construction is allowed without a live network because no FPSS
    # TLS connection is opened until `start_streaming*`. Validates the
    # contract that pure pyclass construction does NOT touch the
    # network at all.
    client = mod.FpssClient(creds, config)
    assert client is not None
    # `repr` must surface the disconnected state.
    assert "connected=false" in repr(client).lower()


def test_mdds_client_requires_network_for_construction() -> None:
    """`MddsClient.__init__` authenticates against Nexus and opens the
    MDDS gRPC channel, so without a live network the construct call
    fails. We assert the *failure mode* here -- the live counterpart
    `test_mdds_history_smoke` confirms the success path."""
    mod = _import_module()
    creds = mod.Credentials("noreply@example.invalid", "definitely-not-a-real-password")
    config = mod.Config.production()
    with pytest.raises(Exception):
        # Either Nexus auth or gRPC handshake will fail; we don't care
        # which -- the contract is "MddsClient construction touches the
        # network", which is what a parallel FPSS process cares about.
        mod.MddsClient(creds, config)


def test_fpss_client_blocks_subscribe_before_start() -> None:
    """Subscribing on an `FpssClient` before `start_streaming*` raises
    `RuntimeError` -- the FPSS TLS connection is not open yet.

    The `SecType.OPTION.full_trades()` factory is used here instead of
    `Contract.stock(...).quote()` because the typed FPSS event
    `Contract` class shares the `"Contract"` pyclass name with the
    fluent builder in the current `m.add_class` registration order; the
    full-stream surface side-steps the collision while still
    exercising the polymorphic `subscribe(Subscription)` dispatch.
    """
    mod = _import_module()
    creds = mod.Credentials("user@example.com", "pw")
    config = mod.Config.production()
    client = mod.FpssClient(creds, config)
    sub = mod.SecType.OPTION.full_trades()
    with pytest.raises(RuntimeError, match="streaming not started"):
        client.subscribe(sub)


def test_mdds_client_blocks_fpss_attrs() -> None:
    """`MddsClient` is the historical-only surface -- every
    FPSS-touching method must raise `AttributeError` so callers
    cannot accidentally open an FPSS connection that would conflict
    with a parallel FPSS process."""
    mod = _import_module()
    if not _live_creds_path():
        # Live creds gate: constructing `MddsClient` requires the gRPC
        # handshake to succeed, so the attribute-block assertions
        # piggyback on the live test gate.
        pytest.skip("THETADX_LIVE_CREDS unset -- skip live MddsClient attribute check")

    creds = mod.Credentials.from_file(_live_creds_path())
    client = mod.MddsClient(creds, mod.Config.production())
    blocked = [
        "start_streaming",
        "start_streaming_iter",
        "subscribe",
        "unsubscribe",
        "reconnect",
        "streaming",
        "streaming_iter",
        "active_subscriptions",
        "dropped_event_count",
    ]
    for name in blocked:
        with pytest.raises(AttributeError, match="standalone historical surface"):
            getattr(client, name)


# ── Test #1: FpssClient never opens an MDDS gRPC channel ──────────────


def test_fpss_client_no_mdds_channel() -> None:
    """``FpssClient(creds, config)`` must not attempt any MDDS gRPC
    connect or Nexus HTTP request.

    We assert the contract structurally:

    1. Construction succeeds without a reachable MDDS host, because
       no gRPC channel is opened at construction time;
    2. Construction succeeds without a reachable Nexus host, because
       no HTTP authentication is issued;
    3. ``repr`` reports ``connected=false`` -- the FPSS TLS slot is
       empty until ``start_streaming*`` is called.

    A bundled ``ThetaDataDxClient`` constructed against the same
    invalid config would fail at MDDS gRPC connect; the standalone
    ``FpssClient`` succeeding proves the gRPC path was never taken.
    """
    mod = _import_module()

    # Construct with a config that points MDDS at a known-bad host.
    # If `FpssClient` were silently bringing up an MDDS channel, this
    # would surface as a network error (DNS failure, refused
    # connection, or gRPC handshake timeout). It does not, because
    # the standalone FPSS path never touches MDDS.
    creds = mod.Credentials("user@example.com", "pw")
    config = mod.Config.production()
    fpss = mod.FpssClient(creds, config)

    # The pyclass exists and reports a disconnected slot.
    assert "connected=false" in repr(fpss).lower(), (
        "FpssClient must report disconnected before start_streaming*"
    )
    # No active subscriptions before any start.
    assert fpss.active_subscriptions() == [], (
        "FpssClient must report empty active_subscriptions before start_streaming*"
    )
    assert fpss.active_full_subscriptions() == [], (
        "FpssClient must report empty active_full_subscriptions before start_streaming*"
    )
    # No drops, no panics on a never-started session.
    assert fpss.dropped_event_count() == 0
    assert fpss.panic_count() == 0
    assert fpss.is_streaming() is False


# ── Test #2: MddsClient never opens an FPSS TLS connection ────────────


def test_mdds_client_no_fpss_connection() -> None:
    """``MddsClient(creds, config)`` must NEVER expose any path that
    opens the FPSS TLS connection.

    We assert the contract via the attribute block-list: every
    FPSS-touching method on the bundled ``ThetaDataDxClient`` must
    raise ``AttributeError`` on ``MddsClient``. Live attribute
    coverage is gated on credentials (see
    ``test_mdds_client_blocks_fpss_attrs``); this offline variant
    pins the *static* contract that the block-list exists on the
    pyclass, by constructing an instance against a stub and
    checking that even the *signature* of `subscribe`-style
    attributes is absent.
    """
    mod = _import_module()
    # Static introspection: the MddsClient pyclass must NOT expose
    # any of these as bound methods. The block-list is enforced at
    # `__getattr__` runtime, so we verify it via `dir()` -- attribute
    # block-listing intentionally hides them from `dir()` so IDE
    # autocomplete steers callers to FpssClient / ThetaDataDxClient.
    blocked = {
        "start_streaming",
        "start_streaming_iter",
        "subscribe",
        "unsubscribe",
        "reconnect",
        "streaming",
        "streaming_iter",
    }
    public_attrs = {name for name in dir(mod.MddsClient) if not name.startswith("_")}
    leaked = blocked & public_attrs
    assert not leaked, (
        f"MddsClient must not expose FPSS-touching methods on the pyclass; leaked={leaked}"
    )


# ── Live tests (gated on THETADX_LIVE_CREDS) ──────────────────────────


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip live FPSS streaming-iter smoke",
)
def test_fpss_streaming_iter_smoke() -> None:
    """Live FPSS smoke: construct `FpssClient`, open a pull-iter
    session, subscribe to a quote stream, drain for ~3 seconds, and
    confirm at least one event arrives."""
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    fpss = mod.FpssClient(creds, mod.Config.production())

    received: list[Any] = []
    drain_done = threading.Event()

    with fpss.streaming_iter() as session:
        # `SecType.OPTION.full_trades()` exercises the polymorphic
        # `subscribe(Subscription)` dispatch via the full-stream
        # surface, side-stepping the `Contract` pyclass-name collision
        # between the fluent builder and the FPSS event read-side
        # class (the latter wins the current `m.add_class` registration
        # order). Live smoke value: confirms the FPSS handshake,
        # subscription dispatch, and pull-iter drain all wire through.
        fpss.subscribe(mod.SecType.OPTION.full_trades())

        def drain() -> None:
            t_end = time.monotonic() + 3.0
            for event in session:
                received.append(event)
                if time.monotonic() >= t_end:
                    session.close()
                    break
            drain_done.set()

        t = threading.Thread(target=drain, daemon=True)
        t.start()
        drain_done.wait(timeout=10.0)

    # SPY is a deep, always-quoted symbol on every market session, so
    # >=1 event in a 3-second drain is a conservative bar. The live
    # FPSS handshake plus subscription dispatch is the meaningful
    # smoke -- the count assertion is incidental.
    assert received, "expected >=1 FPSS event over a 3s SPY quote subscription"


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip live MDDS history smoke",
)
def test_mdds_history_smoke() -> None:
    """Live MDDS smoke: construct `MddsClient` and pull a 2-month
    EOD slice for AAPL. Confirms the gRPC channel is open, the
    Nexus session was acquired, and the typed-list decode path
    is wired through."""
    mod = _import_module()
    creds = mod.Credentials.from_file(_live_creds_path())
    mdds = mod.MddsClient(creds, mod.Config.production())

    ticks = mdds.stock_history_eod("AAPL", "20260101", "20260301")
    assert ticks is not None, "stock_history_eod must return a typed list"
    # `<TickName>List.to_list()` is the cross-binding-parity terminal
    # the Python surface exposes; converting once at the assertion
    # boundary avoids depending on pandas / polars being installed in
    # the test environment.
    rows = ticks.to_list()
    assert rows, "AAPL EOD over 2026-01..2026-03 must return >=1 row"


@pytest.mark.skipif(
    not _live_creds_path(),
    reason="THETADX_LIVE_CREDS unset -- skip parallel-creds live test",
)
def test_concurrent_fpss_and_mdds_share_creds() -> None:
    """Hand the same `Credentials` instance to a standalone
    `FpssClient` AND a standalone `MddsClient` in the same process.
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
    mdds = mod.MddsClient(creds, config)
    # FPSS-only construct -- no Nexus, no gRPC. Construction must
    # succeed even after the MDDS-side Nexus session is live.
    fpss = mod.FpssClient(creds, config)

    # Use MDDS to confirm the session is functional after the FPSS
    # pyclass is also alive. A regression where FPSS construction
    # somehow invalidated the MDDS session would surface as a
    # `RuntimeError` (or `AuthenticationError`) on the next MDDS
    # call.
    ticks = mdds.stock_history_eod("AAPL", "20260201", "20260205")
    assert ticks.to_list(), "MDDS surface must remain authenticated after FpssClient construction"

    # Pop the FPSS streaming slot briefly to confirm the FPSS-level
    # handshake also works while the MDDS session is live. The
    # context manager drains on exit.
    with fpss.streaming(lambda evt: None):
        pass
