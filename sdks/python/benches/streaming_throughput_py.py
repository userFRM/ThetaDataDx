"""Python streaming-callback throughput ceiling (offline, no network).

Measures the events/sec ceiling of the Python streaming path when the
network is removed and the pipeline is saturated — the apples-to-apples
Python row for the cross-binding throughput table. The companion Rust
bench is ``crates/thetadatadx/benches/streaming_throughput.rs``; this
script mirrors its methodology exactly.

What is measured
----------------
The bench-only ``thetadatadx.__bench_flood_events(n, callback)`` hook
drives the SAME LMAX Disruptor pipeline the live FPSS consumer uses
(single producer, single consumer thread, 4096-slot ring) and, for every
delivered event, runs the IDENTICAL per-event handover the generated
``start_streaming`` dispatcher runs in production:

  1. ``Python::attach`` — acquire the GIL on the consumer thread. The
     production granularity is PER-EVENT (the FPSS callback closure in
     ``_generated/streaming_methods.rs`` re-attaches for each delivered
     event), so this bench measures the same granularity.
  2. ``fpss_event_to_typed(py, event)`` — the exact borrowed-event ->
     typed ``#[pyclass]`` marshal the production path calls, reused (not
     reimplemented).
  3. ``callback.call1(py, (typed,))`` — the same 1-tuple vectorcall.

The user callback here is a Python no-op (``def _cb(_e): pass``) so the
number is the SDK boundary ceiling: ring publish + per-event GIL acquire
+ typed-pyclass marshal + Python call dispatch, with the user doing
nothing. Real integrator code does strictly more per event, so this is an
upper bound on the Python streaming rate.

Methodology (matches the Rust bench)
------------------------------------
* ``EVENTS_PER_ITER = 100_000`` events per sample (same as the Rust bench).
* The timed region is the publish loop + consumer drain, measured INSIDE
  Rust and returned as ``elapsed_ns``. Interpreter spin-up, ring
  allocation, and consumer-thread spawn run before the timed region opens
  and are excluded — the same setup-exclusion ``b.iter_custom`` gives the
  Rust bench.
* Warmup samples are discarded.
* Every sample asserts ``delivered == EVENTS_PER_ITER`` — a ceiling with
  silent drops is invalid, so a short delivery aborts the run.
* Reports events/sec p50 (median) + min/max and ns/event, over the
  measured (post-warmup) samples.

Run::

    VIRTUAL_ENV=/tmp/cta-pyvenv /tmp/cta-pyvenv/bin/python \
        sdks/python/benches/streaming_throughput_py.py

Output is line-delimited then a final JSON object so a harness can parse
the summary without a second dependency.
"""

from __future__ import annotations

import json
import platform
import statistics
import sys

import thetadatadx

# Events per sample. Matches `EVENTS_PER_ITER` in the Rust bench so the
# per-sample wall clock dwarfs measurement overhead and the consumer
# thread reaches steady state.
EVENTS_PER_ITER: int = 100_000

# Warmup samples (discarded) + measured samples. The Rust criterion bench
# warms up for 3 s then collects 10 samples; we fix explicit counts so the
# Python and Rust sample budgets are comparable and the run is bounded.
WARMUP_SAMPLES: int = 3
MEASURED_SAMPLES: int = 12


def _noop_callback(_event: object) -> None:
    """User callback: a pure no-op. The measured cost is everything the
    SDK does to get here (GIL acquire + marshal + dispatch), not the
    user's handler body."""
    return None


def _run_one_sample() -> tuple[int, int]:
    """One flood of EVENTS_PER_ITER events. Returns (delivered, elapsed_ns)
    straight from the Rust hook; the timed region excludes setup."""
    return thetadatadx.__bench_flood_events(EVENTS_PER_ITER, _noop_callback)


def main() -> int:
    # Warmup — fault in the consumer thread, JIT the interpreter's call
    # path, warm the allocator. Discarded.
    for _ in range(WARMUP_SAMPLES):
        delivered, _ = _run_one_sample()
        if delivered != EVENTS_PER_ITER:
            print(
                f"FATAL: warmup delivered {delivered} != {EVENTS_PER_ITER} "
                f"(silent drop — ceiling invalid)",
                file=sys.stderr,
            )
            return 1

    events_per_sec: list[float] = []
    ns_per_event: list[float] = []
    for i in range(MEASURED_SAMPLES):
        delivered, elapsed_ns = _run_one_sample()
        if delivered != EVENTS_PER_ITER:
            print(
                f"FATAL: sample {i} delivered {delivered} != {EVENTS_PER_ITER} "
                f"(silent drop — ceiling invalid)",
                file=sys.stderr,
            )
            return 1
        eps = EVENTS_PER_ITER / (elapsed_ns / 1e9)
        nspe = elapsed_ns / EVENTS_PER_ITER
        events_per_sec.append(eps)
        ns_per_event.append(nspe)
        print(
            f"sample {i:2d}: {eps / 1e6:8.3f} Melem/s   "
            f"{nspe:8.2f} ns/event   delivered={delivered}"
        )

    p50 = statistics.median(events_per_sec)
    eps_min = min(events_per_sec)
    eps_max = max(events_per_sec)
    nspe_p50 = statistics.median(ns_per_event)

    summary = {
        "bench": "python_pyo3_gil_noop",
        "granularity": "per_event_gil_attach",
        "events_per_iter": EVENTS_PER_ITER,
        "warmup_samples": WARMUP_SAMPLES,
        "measured_samples": MEASURED_SAMPLES,
        "events_per_sec_p50": p50,
        "events_per_sec_min": eps_min,
        "events_per_sec_max": eps_max,
        "ns_per_event_p50": nspe_p50,
        "zero_drop_verified": True,
        "machine": {
            "python": platform.python_version(),
            "impl": platform.python_implementation(),
            "machine": platform.machine(),
        },
    }
    print("")
    print(
        f"p50: {p50 / 1e6:.3f} Melem/s   "
        f"({nspe_p50:.2f} ns/event)   "
        f"min={eps_min / 1e6:.3f}  max={eps_max / 1e6:.3f} Melem/s   "
        f"zero-drop verified over {MEASURED_SAMPLES} samples"
    )
    print("JSON " + json.dumps(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
