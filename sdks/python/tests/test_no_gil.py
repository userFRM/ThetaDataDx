"""GIL-release verification on the historical-endpoint hot path.

Pattern from the Kairos streaming reference: spawn a CPU-bound peer
thread, then run a series of historical queries serially. If the
binding holds the GIL across `block_on`, the CPU thread starves the
dispatcher and elapsed-time grows linearly with the iteration count.
If the binding wraps every `block_on` in `Python::detach`, the two
threads run in parallel and elapsed-time stays bounded by the
dispatcher alone.

The live test requires `THETADX_TEST_CREDS` in the environment and
runs against the production gRPC endpoint. The structural test runs
unconditionally and checks the shape of the audit grep that gates
the binding: every `block_on` call site in `sdks/python/src/` MUST
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
    """Locate `sdks/python/src/` from any working directory the test
    runner picks. We walk up from the test file until the layout
    appears so the audit grep does not depend on `pytest`'s cwd."""
    here = Path(__file__).resolve().parent
    for candidate in [here, *here.parents]:
        target = candidate / "src"
        if target.is_dir() and (target / "lib.rs").is_file():
            return target
    raise RuntimeError("could not locate sdks/python/src from test file")


def test_every_block_on_is_preceded_by_detach() -> None:
    """Audit gate: no bare `block_on` call site in the binding.

    Walks every `.rs` file under `sdks/python/src/`, finds every
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


# ── runtime verification (live, GIL-release on hot path) ────────────


def _busy_cpu(stop: threading.Event) -> None:
    s = 0
    while not stop.is_set():
        for _ in range(1_000_000):
            s = (s + 1) & 0xFFFFFFFF


@pytest.mark.skipif(
    not os.getenv("THETADX_TEST_CREDS"),
    reason="needs THETADX_TEST_CREDS (production gRPC credentials file)",
)
def test_historical_releases_gil() -> None:
    """Live GIL-drop probe — must overhead < 1.5x on a healthy SDK."""
    import thetadatadx as td

    creds = td.Credentials.from_file(os.environ["THETADX_TEST_CREDS"])
    config = td.Config.production()
    client = td.ThetaDataDxClient(creds, config)

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
            client.stock_snapshot_quote("AAPL")

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
    import-time regression on either GIL or nogil interpreters."""
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
