"""Python streaming throughput — optimization levers (offline, no network).

Measures the TRUE MAX Python streaming throughput by amortizing the
per-event language-boundary crossing three ways, against the per-event
baseline (~201k events/s) the companion `streaming_throughput_py.py`
records. All variants drive the SAME LMAX Disruptor pipeline (131_072-slot
ring = production default); only the boundary-crossing shape changes.

Variants
--------
1. per_event              — baseline: 1 GIL attach + 1 marshal + 1 call1 / event.
2. batched_calls[B]       — Lever 1a: 1 GIL attach per batch of B, then B
                            marshals + B call1s inside it (amortize the GIL
                            acquire only).
3. batched_list[B]        — Lever 1b: 1 GIL attach per batch, marshal B events
                            into one Python list, ONE call1(list) (amortize
                            GIL acquire + call dispatch).
4. arrow[B]               — Lever 3: 1 GIL attach per batch, build ONE Arrow
                            RecordBatch from B TradeTick rows, export zero-copy
                            to a pyarrow.Table over the C-Stream Interface, ONE
                            call1(table). DIFFERENT delivery model: Python gets
                            a columnar Table, not B event objects.

Batch sweep: 256 / 1024 / 4096 (override via THETADX_BATCHES env, comma-sep).

Methodology (matches the per-event bench + the core Rust bench)
---------------------------------------------------------------
* EVENTS_PER_ITER = 100_000 events per sample.
* Timed region = publish loop + drain, measured INSIDE Rust and returned as
  elapsed_ns. Interpreter spin-up, ring allocation, consumer-thread spawn run
  before the timed region opens and are excluded.
* Warmup samples discarded; median (p50) reported with min/max.
* Every sample asserts delivered == EVENTS_PER_ITER (zero-drop). For Arrow,
  the callback also tallies pyarrow.Table.num_rows so the consumed row count
  is checked == EVENTS_PER_ITER (receive-side zero-drop).

Run (standard GIL):
    VIRTUAL_ENV=/tmp/cta-pyvenv /tmp/cta-pyvenv/bin/python \
        thetadatadx-py/benches/streaming_throughput_levers_py.py

Run (free-threaded, Lever 2):
    PYTHON_GIL=0 VIRTUAL_ENV=/tmp/cta-pyvenv-ft /tmp/cta-pyvenv-ft/bin/python \
        thetadatadx-py/benches/streaming_throughput_levers_py.py

Output: line-delimited per-sample, then one `JSON {...}` summary per variant.
"""

from __future__ import annotations

import json
import os
import platform
import statistics
import sys

import thetadatadx

EVENTS_PER_ITER = 100_000
WARMUP_SAMPLES = 3
MEASURED_SAMPLES = 12
BATCHES = [int(x) for x in os.environ.get("THETADX_BATCHES", "256,1024,4096").split(",")]


def gil_enabled() -> bool:
    fn = getattr(sys, "_is_gil_enabled", None)
    return bool(fn()) if fn else True


def summarize(name: str, eps_list: list[float], nspe_list: list[float], extra: dict) -> dict:
    p50 = statistics.median(eps_list)
    return {
        "variant": name,
        "events_per_sec_p50": p50,
        "events_per_sec_min": min(eps_list),
        "events_per_sec_max": max(eps_list),
        "ns_per_event_p50": statistics.median(nspe_list),
        "events_per_iter": EVENTS_PER_ITER,
        "warmup_samples": WARMUP_SAMPLES,
        "measured_samples": MEASURED_SAMPLES,
        "zero_drop_verified": True,
        **extra,
    }


def run_variant(name, call_one, verify_received=None) -> dict:
    """`call_one()` runs one flood and returns (delivered, elapsed_ns).
    `verify_received()` (optional) returns the receive-side count to assert
    == EVENTS_PER_ITER (used by the list/arrow variants)."""
    for _ in range(WARMUP_SAMPLES):
        delivered, _ = call_one()
        if delivered != EVENTS_PER_ITER:
            print(f"FATAL[{name}]: warmup delivered {delivered} != {EVENTS_PER_ITER}", file=sys.stderr)
            sys.exit(1)
        if verify_received is not None:
            got = verify_received()
            if got != EVENTS_PER_ITER:
                print(f"FATAL[{name}]: warmup received {got} != {EVENTS_PER_ITER}", file=sys.stderr)
                sys.exit(1)

    eps_list, nspe_list = [], []
    for i in range(MEASURED_SAMPLES):
        delivered, elapsed_ns = call_one()
        if delivered != EVENTS_PER_ITER:
            print(f"FATAL[{name}]: sample {i} delivered {delivered} != {EVENTS_PER_ITER}", file=sys.stderr)
            sys.exit(1)
        if verify_received is not None:
            got = verify_received()
            if got != EVENTS_PER_ITER:
                print(f"FATAL[{name}]: sample {i} received {got} != {EVENTS_PER_ITER}", file=sys.stderr)
                sys.exit(1)
        eps = EVENTS_PER_ITER / (elapsed_ns / 1e9)
        eps_list.append(eps)
        nspe_list.append(elapsed_ns / EVENTS_PER_ITER)
    p50 = statistics.median(eps_list)
    print(f"  {name:28s}: p50 {p50/1e6:7.3f} Melem/s  {statistics.median(nspe_list):8.2f} ns/event  (zero-drop {MEASURED_SAMPLES}x)")
    return eps_list, nspe_list


def main() -> int:
    ft = not gil_enabled()
    print(f"# Python {platform.python_version()} ({'FREE-THREADED' if ft else 'standard GIL'}), "
          f"events/iter={EVENTS_PER_ITER}, warmup={WARMUP_SAMPLES}, samples={MEASURED_SAMPLES}")

    summaries = []

    # 1. per-event baseline
    def noop(_e):
        return None
    eps, nspe = run_variant("per_event", lambda: thetadatadx.__bench_flood_events(EVENTS_PER_ITER, noop))
    summaries.append(summarize("per_event", eps, nspe, {"batch_size": 1}))

    # 2 + 3 + 4: batch sweep
    for B in BATCHES:
        # Lever 1a: batched calls (callback receives one event per call).
        eps, nspe = run_variant(
            f"batched_calls[{B}]",
            lambda B=B: thetadatadx.__bench_flood_events_batched_calls(EVENTS_PER_ITER, B, noop),
        )
        summaries.append(summarize(f"batched_calls", eps, nspe, {"batch_size": B}))

        # Lever 1b: batched list (callback receives a list of events per call).
        recv = {"n": 0}
        def list_cb(lst, _r=recv):
            _r["n"] += len(lst)
        def list_one(B=B, _r=recv):
            _r["n"] = 0
            return thetadatadx.__bench_flood_events_batched_list(EVENTS_PER_ITER, B, list_cb)
        eps, nspe = run_variant(
            f"batched_list[{B}]", list_one, verify_received=lambda _r=recv: _r["n"]
        )
        summaries.append(summarize("batched_list", eps, nspe, {"batch_size": B}))

        # Lever 3: Arrow columnar batch (callback receives a pyarrow.Table).
        arecv = {"rows": 0}
        def arrow_cb(table, _r=arecv):
            _r["rows"] += table.num_rows
        def arrow_one(B=B, _r=arecv):
            _r["rows"] = 0
            return thetadatadx.__bench_flood_events_arrow(EVENTS_PER_ITER, B, arrow_cb)
        eps, nspe = run_variant(
            f"arrow[{B}]", arrow_one, verify_received=lambda _r=arecv: _r["rows"]
        )
        summaries.append(summarize("arrow", eps, nspe, {"batch_size": B}))

    machine = {
        "python": platform.python_version(),
        "gil": "disabled" if ft else "enabled",
        "impl": platform.python_implementation(),
    }
    print("")
    for s in summaries:
        s["machine"] = machine
        print("JSON " + json.dumps(s))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
