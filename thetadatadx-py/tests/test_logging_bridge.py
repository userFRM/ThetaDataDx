"""Rust `tracing` events reach Python's stdlib `logging`.

The bridge decides whether an event is worth forwarding by asking the Python
logger for its level, and asking needs the GIL. It now caches that answer per
target for a short window so a blocking call does not reacquire the GIL once
per event, which is what `test_no_gil.py::test_market_data_releases_gil`
measures. This covers the other side of that change: the events a caller has
asked for still arrive.

The opposite direction is not testable from here and does not need to be. The
bridge emits through ``Logger.log``, which re-checks ``isEnabledFor`` itself,
so no answer the cache can give will forward a record the caller switched off.
A test asserting that would pass with the whole Rust-side filter deleted. The
cache's own failure direction is pinned in Rust by
``the_level_cache_answers_only_for_a_fresh_known_target``.

Live-gated. Nothing in the SDK emits a `tracing` event until it opens a
session, so a real client is the only way to drive the bridge end to end.
"""

from __future__ import annotations

import logging
import os

import pytest


def _creds_path() -> str:
    path = os.environ.get("THETADATADX_TEST_CREDS")
    if not path:
        pytest.skip(
            "set THETADATADX_TEST_CREDS=path/to/creds.txt to enable this live test"
        )
    return path


class _Capture(logging.Handler):
    """Collect every record the `thetadatadx` logger tree emits."""

    def __init__(self) -> None:
        super().__init__(level=logging.NOTSET)
        self.records: list[logging.LogRecord] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


@pytest.fixture
def captured_logger():
    """Attach a capturing handler to the `thetadatadx` logger and restore it."""
    logger = logging.getLogger("thetadatadx")
    handler = _Capture()
    previous_level = logger.level
    previous_propagate = logger.propagate
    logger.addHandler(handler)
    # Keep the records out of pytest's own capture; this test reads them
    # from the handler.
    logger.propagate = False
    try:
        yield logger, handler
    finally:
        logger.removeHandler(handler)
        logger.setLevel(previous_level)
        logger.propagate = previous_propagate


def _open_a_session(creds_path: str):
    import thetadatadx

    creds = thetadatadx.Credentials.from_file(creds_path)
    return thetadatadx.Client(creds, thetadatadx.Config.production())


def test_debug_events_reach_python_logging(captured_logger):
    """With the logger at DEBUG, Rust-side debug events arrive as records.

    This is the claim the module's docstring makes and the one the level
    cache could silently break: a cache that answered "not enabled" for a
    target it had never asked about would drop every event.
    """
    creds_path = _creds_path()
    logger, handler = captured_logger
    logger.setLevel(logging.DEBUG)

    client = _open_a_session(creds_path)
    try:
        assert handler.records, "opening a session emits Rust tracing events"
        assert any(r.levelno == logging.DEBUG for r in handler.records), (
            "a logger at DEBUG receives the debug events, not only the "
            f"higher ones: {sorted({r.levelno for r in handler.records})}"
        )
        assert all(r.name.startswith("thetadatadx.") for r in handler.records), (
            "targets are normalised onto the Python logger hierarchy: "
            f"{sorted({r.name for r in handler.records})}"
        )
    finally:
        client.close()
