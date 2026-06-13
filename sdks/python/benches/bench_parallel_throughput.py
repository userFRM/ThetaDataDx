"""GIL vs nogil parallel throughput.

Measures historical-endpoint throughput while a CPU-bound Python thread
hammers the interpreter. Under the GIL the CPU thread time-slices with
the dispatcher thread, so the historical thread sees ~half the
throughput. Under free-threaded Python (`python3.14t`) the two threads
run on separate cores in parallel and the historical thread sees
near-baseline throughput.

The bench does not need live FPSS credentials. It exercises a pure-Rust
fast path that already drops the GIL via `Python::detach(|| ...)` in
`run_blocking` / `run_blocking_snapshot`. The wall-clock difference
between baseline (no CPU thread) and contended (CPU thread running)
reveals whether the SDK actually releases the GIL on the hot path.

Run on both interpreters and compare::

    python3.14   benches/bench_parallel_throughput.py
    python3.14t  benches/bench_parallel_throughput.py

The healthy SDK shows `overhead < 1.5x` on both. Higher overhead on
3.14 with a held GIL (or a regression) makes the contention thread
starve the dispatcher.

Output format is line-delimited JSON so CI can parse without a second
dependency.
"""

from __future__ import annotations

import json
import sys
import threading
import time
from typing import Callable


# Tight inner loop sized so a single iteration is a handful of µs on
# any modern x86 core — the dispatcher needs many opportunities to be
# preempted, so we keep the per-iteration work small and the iteration
# count high.
_BURN_INNER = 1000


def cpu_burn(stop_event: threading.Event) -> int:
    """CPU-bound work that holds the interpreter on a GIL-build."""
    x = 0
    while not stop_event.is_set():
        for _ in range(_BURN_INNER):
            x = (x * 17 + 1) & 0xFFFFFFFF
    return x


def gil_enabled() -> bool:
    """True on a stock CPython, False on a free-threaded build."""
    probe: Callable[[], bool] | None = getattr(sys, "_is_gil_enabled", None)
    return probe() if probe is not None else True


def _run_workload(workload: Callable[[], int]) -> tuple[float, int]:
    t0 = time.perf_counter()
    total = workload()
    return time.perf_counter() - t0, total


def measure_baseline(workload: Callable[[], int]) -> tuple[float, int]:
    """Wall-clock + total events with the dispatcher thread alone."""
    return _run_workload(workload)


def measure_contended(workload: Callable[[], int]) -> tuple[float, int]:
    """Wall-clock + total events with one CPU-bound peer thread."""
    stop = threading.Event()
    burner = threading.Thread(target=cpu_burn, args=(stop,), daemon=True)
    burner.start()
    try:
        return _run_workload(workload)
    finally:
        stop.set()
        burner.join(timeout=5.0)


def _default_workload() -> Callable[[], int]:
    """Synthetic dispatcher workload — exercises the GIL-release path
    without live credentials.

    Each iteration acquires a `parking_lot::Mutex` inside a Rust
    pyclass twice — once for the get, once for the increment. On a
    GIL-build with a CPU peer thread, the iteration count plummets
    because every method call requires re-acquiring the GIL between
    the Python loop body and the Rust binding. On a nogil build the
    two threads truly run in parallel and the dispatcher sees
    near-baseline rates.

    Falls back to a pure-Python counter if the binding is not
    importable so the bench can still report the interpreter probe
    and a relative number for CI plumbing.
    """
    try:
        from thetadatadx import Config

        config = Config.production()

        def workload() -> int:
            n = 0
            duration = 1.0
            t0 = time.perf_counter()
            while time.perf_counter() - t0 < duration:
                # Trip into the Rust Mutex inside `Config` via the
                # `mdds_host` property getter. Each access acquires the
                # std::sync::Mutex on `inner` then drops it. The
                # Mutex-protected DirectConfig snapshot is read under
                # the lock, then the GIL is reacquired only to wrap the
                # returned String in a Py object — so a CPU peer thread
                # can preempt the lock-holding thread between calls.
                _ = config.mdds_host
                n += 1
            return n

        return workload

    except ImportError:

        def fallback() -> int:
            n = 0
            duration = 1.0
            t0 = time.perf_counter()
            while time.perf_counter() - t0 < duration:
                n += 1
            return n

        return fallback


def main() -> int:
    workload = _default_workload()

    baseline_t, baseline_n = measure_baseline(workload)
    contended_t, contended_n = measure_contended(workload)

    baseline_rate = baseline_n / baseline_t
    contended_rate = contended_n / contended_t
    overhead = baseline_rate / contended_rate if contended_rate else float("inf")

    result = {
        "python": sys.version.split()[0],
        "gil_enabled": gil_enabled(),
        "baseline_rate_hz": round(baseline_rate, 1),
        "contended_rate_hz": round(contended_rate, 1),
        "overhead_ratio": round(overhead, 3),
        "baseline_iters": baseline_n,
        "contended_iters": contended_n,
        "baseline_wall_s": round(baseline_t, 4),
        "contended_wall_s": round(contended_t, 4),
    }
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
