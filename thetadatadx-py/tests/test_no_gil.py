"""GIL-release verification on the historical-endpoint hot path.

Pattern from the Kairos streaming reference: spawn a CPU-bound peer
thread, then run a series of historical queries serially. If the
binding holds the GIL across `block_on`, the CPU thread starves the
dispatcher and elapsed-time grows linearly with the iteration count.
If the binding wraps every `block_on` in `Python::detach`, the two
threads run in parallel and elapsed-time stays bounded by the
dispatcher alone.

The live test requires `THETADATADX_TEST_CREDS` in the environment and
runs against the production gRPC endpoint. The structural test runs
unconditionally and checks the shape of the audit grep that gates
the binding: every `block_on` call site in `thetadatadx-py/src/` MUST
be preceded by `py.detach(||` so the GIL is released for the
duration of the blocking future.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest


# ── structural audit ────────────────────────────────────────────────


# Comments + docstrings reference `block_on` for explanation; the
# audit grep below only flags *call sites* in code. We exclude lines
# that are pure comments (start with `//` after optional whitespace)
# and doc-comments (`///`).
_BLOCK_ON_CALL_SITE = re.compile(r"\.?block_on\s*\(")
_COMMENT_LINE = re.compile(r"^\s*(///?|\*)")


def _sdk_src_root() -> Path:
    """Locate `thetadatadx-py/src/` from any working directory the test
    runner picks. We walk up from the test file until the layout
    appears so the audit grep does not depend on `pytest`'s cwd."""
    here = Path(__file__).resolve().parent
    for candidate in [here, *here.parents]:
        target = candidate / "src"
        if target.is_dir() and (target / "lib.rs").is_file():
            return target
    raise RuntimeError("could not locate thetadatadx-py/src from test file")


def test_every_block_on_is_preceded_by_detach() -> None:
    """Audit gate: no bare `block_on` call site in the binding.

    Walks every `.rs` file under `thetadatadx-py/src/`, finds every
    non-comment line containing a `block_on(` call, and asserts the
    immediately preceding non-blank, non-comment line opens a
    `py.detach(||` envelope. Matches the Kairos meta-rule that no
    blocking call may hold the GIL.
    """
    root = _sdk_src_root()
    offenders: list[tuple[Path, int, str]] = []
    for path in sorted(root.rglob("*.rs")):
        lines = path.read_text().splitlines()
        for idx, line in enumerate(lines):
            if _COMMENT_LINE.match(line):
                continue
            if not _BLOCK_ON_CALL_SITE.search(line):
                continue
            # Walk backwards up to a small window of code lines looking
            # for the enclosing `py.detach(||` (or `.detach(||`) frame.
            # A bare `block_on` may span two source lines (`runtime` on
            # one line, `.block_on(...)` on the next), so we permit up
            # to four non-comment, non-blank code lines of separation
            # — enough to cover formatter-induced splits without
            # smuggling in unrelated frames.
            window = []
            for back in range(idx - 1, -1, -1):
                if _COMMENT_LINE.match(lines[back]):
                    continue
                if not lines[back].strip():
                    continue
                window.append(lines[back])
                if len(window) >= 5:
                    break
            if not any("detach(" in w for w in window):
                offenders.append((path, idx + 1, line.strip()))

    assert not offenders, (
        "every `block_on(...)` call must be preceded by a `py.detach(||` "
        "envelope so the GIL is released for the duration of the future. "
        "Offenders:\n"
        + "\n".join(f"  {p}:{ln}  {src}" for p, ln, src in offenders)
    )


def test_fpss_streaming_paths_release_the_gil() -> None:
    """Audit gate: the standalone FPSS streaming connect, reconnect, and
    subscribe paths release the GIL across their blocking I/O.

    The FPSS standalone client does not route through `block_on`; its
    blocking work is a synchronous TLS connect / handshake
    (`StreamingClientBuilder::build`) and a synchronous wire write
    (`StreamingClient::subscribe` / `unsubscribe`). The audit above
    only covers `block_on` call sites, so this test pins the FPSS
    surface separately: the connect must run inside `py.detach`, and
    the shared `with_live` helper that brackets every subscribe /
    unsubscribe wire write must release the GIL too.
    """
    src = (_sdk_src_root() / "fpss_client.rs").read_text()

    # The connect (`.build()`) must sit inside a `py.detach` envelope.
    assert ".detach(|| self.params.builder().build())" in src, (
        "StreamingClient.start_streaming must wrap the blocking FPSS "
        "connect (`builder().build()`) in `py.detach` so a sibling "
        "Python thread keeps running during the TLS handshake"
    )

    # The `with_live` helper -- the single chokepoint every subscribe /
    # unsubscribe wire write flows through -- must release the GIL
    # around the cloned-out Rust handle.
    assert "py.detach(move || f(&client)).map_err(to_py_err)" in src, (
        "StreamingClient.with_live must release the GIL across the "
        "blocking wire write so subscribe / unsubscribe do not hold "
        "the GIL during the socket send"
    )

    # And every subscribe / unsubscribe entry point must flow through
    # that helper rather than touching the live client directly.
    for method in ("subscribe", "subscribe_many", "unsubscribe", "unsubscribe_many"):
        sig = f"fn {method}(&self, py: Python<'_>"
        assert sig in src, (
            f"StreamingClient.{method} must take a `py` token and route "
            f"its wire write through `with_live(py, ...)`; missing `{sig}`"
        )


def test_record_batch_reader_releases_the_gil() -> None:
    """Audit gate: the pull-based Arrow batch reader releases the GIL on
    every blocking path.

    The reader pulls market-data batches off a blocking ring queue. Two
    blocking paths must release the GIL so a sibling Python thread keeps
    running: the FPSS connect when the reader is opened, and the blocking
    `__next__` pull. The async `__anext__` runs the blocking pull on a
    tokio blocking-pool worker (no GIL held). This pins the structural
    shape so a refactor cannot silently start holding the GIL across the
    ring wait.
    """
    src = (_sdk_src_root() / "streaming_batches.rs").read_text()

    # The connect (`builder.build()`) must run inside a `py.detach` envelope.
    assert "py.detach(|| builder.build())" in src, (
        "RecordBatchStream open (`open_reader`) must wrap the blocking FPSS "
        "connect (`builder.build()`) in `py.detach` so a sibling Python "
        "thread keeps running during the handshake"
    )

    # The synchronous blocking pull (`__next__`) must release the GIL across
    # the ring wait via `py.detach`, re-acquiring only to build the pyarrow
    # object after a batch lands.
    assert ".detach(|| inner.next_blocking())" in src, (
        "RecordBatchStream.__next__ must wrap the blocking ring pull in "
        "`py.detach` so other Python threads run while it waits for a batch"
    )

    # The async pull must run on a blocking-pool worker (no GIL held during
    # the wait), re-acquiring the GIL only inside `Python::attach` to build
    # the pyarrow object.
    assert "spawn_blocking(move || inner.next_blocking())" in src, (
        "RecordBatchStream.__anext__ must run the blocking pull on a "
        "blocking-pool worker so the async executor thread never holds the GIL"
    )

    # close() must signal shutdown OUTSIDE the GIL: the teardown shuts the
    # client (which detaches its own join, re-acquiring the GIL per batch on
    # the dispatcher), so holding the GIL across it would deadlock.
    assert "py.detach(move || inner.close_shared())" in src, (
        "RecordBatchStream.close must signal `close_shared` inside `py.detach` "
        "so the dispatcher teardown (which re-acquires the GIL) cannot deadlock"
    )


# ── runtime verification (live, GIL-release on hot path) ────────────


def test_fpss_connect_releases_the_gil(monkeypatch) -> None:
    """Runtime probe: a sibling Python thread makes progress while
    `StreamingClient.start_streaming` blocks on the FPSS connect.

    No credentials or live server required. We point the primary
    streaming host at a non-routable address so the connect blocks on
    the TCP handshake until `streaming_connect_timeout_ms`, then run a
    counter-incrementing peer thread. If the binding held the GIL
    across the connect the peer thread could not advance until the
    blocking call returned; the assertion floor is far above what the
    GIL-held path could reach inside the connect window.
    """
    import thetadatadx as td

    # Point the primary streaming host at an RFC 5737 TEST-NET-1 address
    # (guaranteed non-routable) so the TCP connect blocks until
    # `streaming_connect_timeout_ms` on every runner, instead of failing
    # fast against a reachable production host. `Config.production()`
    # applies the THETADATA_STREAMING_* overrides.
    monkeypatch.setenv("THETADATA_STREAMING_HOST", "192.0.2.1")
    monkeypatch.setenv("THETADATA_STREAMING_PORT", "20000")

    creds = td.Credentials("user@example.com", "pw")
    config = td.Config.production()
    # The env override only rewrites the primary host slot, leaving the
    # other production hosts in place; pin fixed-order selection so the
    # blackhole primary is the first connect attempt and the call blocks
    # there rather than failing fast against a reachable host.
    config.streaming_host_selection = "fixed_order"
    # Bound the connect window so the test stays fast while still being
    # long enough for the peer thread to accumulate a decisive count.
    config.streaming_connect_timeout_ms = 1500

    fpss = td.StreamingClient(creds, config)

    counter = 0
    stop = threading.Event()

    def peer() -> None:
        nonlocal counter
        while not stop.is_set():
            counter += 1

    t = threading.Thread(target=peer)
    t.start()
    try:
        t0 = time.perf_counter()
        # Blocks on the (unreachable) FPSS connect, then raises. The
        # exception type is irrelevant here -- we only care that the
        # call blocked AND the peer thread advanced during the block.
        with pytest.raises(Exception):
            fpss.start_streaming(lambda _event: None)
        blocked_for = time.perf_counter() - t0
    finally:
        stop.set()
        t.join(timeout=5.0)

    # The probe is only meaningful if the connect actually blocked. On a
    # runner that RSTs the blackhole address (rather than dropping the
    # SYN) the call returns immediately and the GIL-release window never
    # opens -- skip rather than fail, since the structural test
    # (`test_fpss_streaming_paths_release_the_gil`) covers the guarantee
    # unconditionally.
    if blocked_for <= 0.2:
        pytest.skip(
            f"connect returned in {blocked_for:.3f}s -- the blackhole host "
            "did not produce a blocking connect on this runner, so the "
            "GIL-release probe is vacuous"
        )
    # If the GIL were held across the connect, the peer thread would be
    # frozen for the entire `blocked_for` window and `counter` would be
    # ~0. Releasing the GIL lets it spin freely; even a slow CI box
    # clears this floor by orders of magnitude.
    assert counter > 100_000, (
        f"sibling thread advanced only {counter} steps while "
        f"start_streaming blocked for {blocked_for:.3f}s -- the GIL "
        "appears to be held across the FPSS connect"
    )


def _busy_cpu(stop: threading.Event) -> None:
    s = 0


def _busy_cpu(stop: threading.Event) -> None:
    s = 0
    while not stop.is_set():
        for _ in range(1_000_000):
            s = (s + 1) & 0xFFFFFFFF


@pytest.mark.skipif(
    not os.getenv("THETADATADX_TEST_CREDS"),
    reason="needs THETADATADX_TEST_CREDS (production gRPC credentials file)",
)
def test_historical_releases_gil() -> None:
    """Live GIL-drop probe — must overhead < 1.5x on a healthy SDK."""
    import thetadatadx as td

    creds = td.Credentials.from_file(os.environ["THETADATADX_TEST_CREDS"])
    config = td.Config.production()
    client = td.Client(creds, config)

    # Use a same-day stock snapshot to keep the wire payload small and
    # the per-call wall clock dominated by the network round-trip and
    # gRPC compute — exactly the path we want to confirm releases the
    # GIL across `block_on`.
    iters = 10
    contended_thread = threading.Event()

    def hot_path() -> None:
        for _ in range(iters):
            # `today` is fine here — the snapshot endpoint always
            # returns the most recent quote regardless of intraday
            # state, so we exercise the network path without
            # depending on a specific session being open.
            client.market_data.stock_snapshot_quote("AAPL")

    # Baseline
    t0 = time.perf_counter()
    hot_path()
    baseline = time.perf_counter() - t0

    # Contended
    cpu_thread = threading.Thread(target=_busy_cpu, args=(contended_thread,))
    cpu_thread.start()
    try:
        t0 = time.perf_counter()
        hot_path()
        contended = time.perf_counter() - t0
    finally:
        contended_thread.set()
        cpu_thread.join(timeout=5.0)

    overhead = contended / baseline if baseline > 0 else float("inf")
    assert overhead < 1.5, (
        f"binding appears to hold the GIL during block_on: "
        f"contended {contended:.3f}s vs baseline {baseline:.3f}s "
        f"(overhead {overhead:.2f}x). The GIL-release audit grep must "
        f"have missed a call site."
    )


# ── parallel-throughput probe (no creds, structural shape) ──────────


def test_parallel_throughput_bench_runs() -> None:
    """Smoke-runs the parallel-throughput bench so CI catches an
    import-time regression on either GIL or nogil interpreters.

    When running on a free-threaded interpreter (where
    ``sys._is_gil_enabled() is False``), this test ALSO asserts the
    nogil claim — the binding must achieve `overhead_ratio < 1.8` and
    `gil_enabled == False` on the bench's output. The 1.8x threshold
    matches the CI gate; a regression that re-acquires the GIL on the
    hot path pushes the ratio toward ~2.0x and trips both this test
    and the GH Actions gate.
    """
    bench = Path(__file__).resolve().parent.parent / "benches" / "bench_parallel_throughput.py"
    assert bench.is_file(), f"missing bench script: {bench}"
    result = subprocess.run(
        [sys.executable, str(bench)],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, (
        f"bench exited non-zero: stdout={result.stdout!r} stderr={result.stderr!r}"
    )
    # The bench prints exactly one JSON line — parse to confirm the
    # interpreter probe + rates are populated.
    import json

    payload = json.loads(result.stdout.strip().splitlines()[-1])
    assert "gil_enabled" in payload
    assert payload["baseline_rate_hz"] > 0
    assert payload["contended_rate_hz"] > 0
    assert "overhead_ratio" in payload

    # On a free-threaded interpreter, pin the actual nogil claim. The
    # `sys._is_gil_enabled` probe is the load-bearing source of truth
    # (Python 3.14t); falling back via `getattr` so the GIL-
    # build path leaves this assertion latent.
    is_gil_enabled = getattr(sys, "_is_gil_enabled", None)
    if is_gil_enabled is not None and is_gil_enabled() is False:
        assert payload["gil_enabled"] is False, (
            "bench reported gil_enabled=True on a free-threaded interpreter — "
            "the binding's `gil_used = false` attribute may have regressed"
        )
        ratio = payload["overhead_ratio"]
        assert ratio < 1.8, (
            f"nogil overhead ratio {ratio:.3f}x ≥ 1.8x — the GIL may be "
            "held on the streaming hot path (audit the `block_on` call "
            "sites in `thetadatadx-py/src/lib.rs`)"
        )
