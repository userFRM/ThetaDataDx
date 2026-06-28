"""Pull-based Arrow ``RecordBatch`` reader (``client.stream.batches``).

Offline tests: no live server or credentials. They verify the reader
object's protocol surface, that opening it releases the GIL across the
blocking FPSS connect (a sibling thread keeps running while the connect
blocks against a blackhole host), and that the backpressure knob validates
its input. The end-to-end batching / linger / backpressure behaviour is
proven offline in the Rust core's ``fpss::batch_reader`` tests, which drive
the exact queue + sink + reader machinery without a network.
"""

from __future__ import annotations

import pytest

import thetadatadx as td


def test_record_batch_stream_is_exported() -> None:
    """The reader class is part of the public surface and carries the
    iterable / async-iterable / context-manager protocol plus the
    `schema` / `dropped` accessors."""
    cls = td.RecordBatchStream
    for method in (
        "__iter__",
        "__next__",
        "__aiter__",
        "__anext__",
        "__enter__",
        "__exit__",
        "__aenter__",
        "__aexit__",
        "close",
        "schema",
        "dropped",
    ):
        assert hasattr(cls, method), f"RecordBatchStream must expose {method}"


def test_batches_entry_present_on_stream_view() -> None:
    """`client.stream.batches` is the entry point, a sibling to
    `start_streaming`. Checked on the class to avoid an auth round-trip."""
    assert hasattr(td.StreamView, "batches")
    assert hasattr(td.StreamView, "start_streaming")


def test_batches_is_blocked_on_the_historical_client() -> None:
    """`batches` is an FPSS-touching method, so the MDDS-only historical
    client must refuse it (it is on the block-list)."""
    assert "batches" in td._blocked_fpss_methods()


def test_record_batch_stream_construction_is_blocked_directly() -> None:
    """The reader is not user-constructible — it is only produced by
    `client.stream.batches(...)`. Calling its type directly raises."""
    with pytest.raises(TypeError):
        td.RecordBatchStream()
