"""Verify the per-callback panic isolation contract on the Python binding.

The behavioral contract — a raised exception in the Python callback is caught
by the per-invocation catch_unwind boundary, panic_count() increments, and
event delivery continues — is validated here at the Python surface level.

The Rust-side isolation machinery (poll_batch catch_unwind) is exercised
exhaustively by the in-crate panic_isolation_tests module in
crates/thetadatadx/src/fpss/mod.rs.  The tests here pin the Python binding's
end-to-end contract: the PyO3 dispatcher catches PyErr-raised exceptions,
routes them through Python's unraisable hook, and increments the same
panic_count() counter visible via the public API.

Live-credential tests are gated on THETADX_TEST_CREDS=path/to/creds.txt and
skip silently on machines without credentials.  The API-surface tests (class
presence + return type) run without any network connection.
"""

from __future__ import annotations

import os
import time

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from sdks/python/",
        allow_module_level=True,
    )


# ---------------------------------------------------------------------------
# API-surface tests (no network required)
# ---------------------------------------------------------------------------


class TestPanicCountApiSurface:
    """panic_count() must be present on both streaming client classes."""

    def test_panic_count_present_on_fpss_client(self) -> None:
        """StreamingClient.panic_count() must exist and be callable."""
        assert hasattr(client.StreamingClient, "panic_count"), (
            "StreamingClient must expose panic_count()"
        )
        assert callable(getattr(client.StreamingClient, "panic_count")), (
            "StreamingClient.panic_count must be callable"
        )

    def test_panic_count_present_on_stream_view(self) -> None:
        """panic_count() lives on the `client.stream` StreamView, not on the
        unified Client directly: every streaming diagnostic is reached through
        the stream sub-namespace."""
        assert hasattr(client.StreamView, "panic_count"), (
            "StreamView must expose panic_count()"
        )
        assert callable(getattr(client.StreamView, "panic_count")), (
            "StreamView.panic_count must be callable"
        )
        assert not hasattr(client.Client, "panic_count"), (
            "panic_count() must NOT be on the unified Client; it lives on "
            "client.stream (StreamView)"
        )

    def test_panic_count_api_surface_smoke(self) -> None:
        """Smoke check: panic_count exists and is callable on StreamingClient.

        Runs in <100 ms with no network connection.  Confirms the binding
        exposes the method even when live tests are skipped.
        """
        assert hasattr(client.StreamingClient, "panic_count"), (
            "StreamingClient.panic_count missing from binding"
        )
        assert callable(getattr(client.StreamingClient, "panic_count")), (
            "StreamingClient.panic_count must be callable"
        )

    def test_panic_count_returns_int_when_not_streaming(
        self, tmp_path: object
    ) -> None:
        """panic_count() returns an integer (0) before streaming starts.

        Uses a minimal StreamingClient constructed without opening a network
        connection — panic_count() must not block or raise.
        """
        creds_file = "/home/theta-gamma/thetadx/creds.txt"
        if not os.path.exists(creds_file):
            pytest.skip("creds.txt not present; skipping live-credential test")

        creds = client.Credentials.from_file(creds_file)
        config = client.Config.production()
        fpss = client.StreamingClient(creds, config)

        count = fpss.panic_count()
        assert isinstance(count, int), (
            f"panic_count() must return int, got {type(count).__name__}"
        )
        assert count == 0, (
            f"panic_count() must be 0 before streaming starts, got {count}"
        )


# ---------------------------------------------------------------------------
# Behavioral tests (live credentials required)
# ---------------------------------------------------------------------------


@pytest.fixture
def fpss_client():
    """Build a standalone StreamingClient or skip if credentials are absent."""
    creds_path = os.environ.get("THETADX_TEST_CREDS")
    if not creds_path:
        pytest.skip(
            "set THETADX_TEST_CREDS=path/to/creds.txt to enable live behavioral tests"
        )
    creds = client.Credentials.from_file(creds_path)
    config = client.Config.production()
    client = client.StreamingClient(creds, config)
    yield client
    try:
        client.stop_streaming()
    except Exception:
        pass


class TestPanicIsolationBehavioral:
    """Behavioral contract: a raised exception in the callback is caught,
    panic_count() increments, and subsequent events are delivered normally.
    """

    def test_exception_in_callback_increments_panic_count(
        self, fpss_client: client.StreamingClient
    ) -> None:
        """An exception raised on the first event increments panic_count()
        to 1, does not stop the dispatcher, and all subsequent events continue
        to be delivered.

        The FPSS server emits at least one control event (Connected) on every
        successful handshake.  The callback raises on its first invocation;
        delivery continues for all subsequent events.  After stop_streaming()
        the shared panic_count() counter must equal 1, and delivered must be
        greater than zero to prove the dispatcher kept running.

        Timeout strategy: the main test thread polls shared state with a hard
        wall-clock deadline rather than joining a worker thread.
        `stop_streaming()` is called from the main thread, which unblocks the
        dispatcher regardless of GIL hold duration.
        """
        EXPECTED_PANICS: int = 1

        raised_once: list[bool] = [False]
        delivered: list[int] = [0]

        def callback(event: object) -> None:
            if not raised_once[0]:
                raised_once[0] = True
                raise RuntimeError("intentional test exception on first event")
            delivered[0] += 1

        fpss_client.start_streaming(callback)

        # Wait up to 5 s for the first event (Connected) to trigger the
        # injected exception.  The server always sends Connected immediately
        # on a successful handshake, so this resolves in well under 1 s.
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            if raised_once[0]:
                break
            time.sleep(0.01)

        # Allow the dispatcher one more batch cycle so at least one
        # post-exception event reaches the callback before we stop.
        deadline2 = time.monotonic() + 2.0
        while time.monotonic() < deadline2:
            if delivered[0] >= 1:
                break
            time.sleep(0.01)

        # Calling stop_streaming() from the main thread unblocks the Rust
        # dispatcher loop via the shutdown flag; this completes even if the
        # dispatcher thread currently holds the GIL.
        fpss_client.stop_streaming()

        count = fpss_client.panic_count()
        assert isinstance(count, int), (
            f"panic_count() must return int after stop_streaming, got {type(count).__name__}"
        )
        assert count == EXPECTED_PANICS, (
            f"panic_count() must equal {EXPECTED_PANICS} after one caught exception; got {count}"
        )
        assert delivered[0] >= 1, (
            f"continued delivery must be >= 1 after {EXPECTED_PANICS} caught callback "
            f"exception(s); got {delivered[0]}"
        )

    def test_non_callable_callback_panic_is_counted(
        self, fpss_client: client.StreamingClient
    ) -> None:
        """A non-callable callback argument causes a TypeError on the
        consumer thread when the first event arrives.  The binding catches that
        via `call1` returning `Err` (not a Rust panic) and increments
        panic_count() via record_panic().

        Timeout strategy: same main-thread polling approach as the sibling
        test above.
        """
        EXPECTED_PANICS: int = 1

        fpss_client.start_streaming(42)  # type: ignore[arg-type]

        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            if fpss_client.panic_count() >= EXPECTED_PANICS:
                break
            time.sleep(0.01)

        fpss_client.stop_streaming()

        count = fpss_client.panic_count()
        assert count == EXPECTED_PANICS, (
            f"panic_count() must equal {EXPECTED_PANICS} after a non-callable "
            f"callback fires on the Connected event; got {count}"
        )
