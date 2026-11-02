// TypeScript streaming-callback throughput ceiling (offline, no network).
//
// Measures the events/sec ceiling of the TypeScript streaming path when
// the network is removed and the pipeline is saturated — the
// apples-to-apples TypeScript row for the cross-binding throughput table.
// The companion Rust bench is
// `thetadatadx-rs/benches/streaming_throughput.rs`; this script
// mirrors its methodology (fixed event count per sample, warmup
// discarded, setup excluded from timing, zero-drop assertion).
//
// What is measured
// ----------------
// The bench-only `__benchFloodEvents(n, callback)` napi export pushes `n`
// synthetic FPSS `Trade` events through the REAL delivery path: the same
// handler and drain scope the production `startStreaming` dispatcher runs,
// so the same per-event marshal, the same arrays of up to
// `STREAMING_CALLBACK_BATCH` events and the same bounded call queue. The
// hand-over runs on a worker thread (off the libuv main thread), exactly as
// the live dispatcher does; the napi `uv_async_t` queue routes each array
// onto the Node main thread, where the package's own `forEachEvent`
// forwarder calls the JS `callback` once per event.
//
// The user callback here is a no-op that only bumps a received counter, so
// the number is the SDK boundary ceiling: Rust->napi marshal, the
// threadsafe-function hops and one V8 callback invocation per event, with
// the user doing nothing. Real integrator code does strictly more per event, so this is
// an upper bound on the TypeScript streaming rate.
//
// Timing
// ------
// `process.hrtime.bigint()` brackets the region from "flood started" to
// "JS callback has fired exactly N times". The promise returned by
// `__benchFloodEvents` resolves once all N events are QUEUED; the last
// queued arrays may still be draining, so we additionally await a
// `received === N` signal before stopping the clock. Module load, addon
// init, and the synthetic-event construction inside Rust are NOT in the
// timed region (construction is counted inside the same loop the Rust and
// Python benches count it in — see note below).
//
// Zero-drop verification (two independent checks)
// -----------------------------------------------
// 1. `__benchFloodEvents` rejects if any hand-over failed.
// 2. The JS callback received-count is asserted === N (receive-side check).
//
// The export is built only with the `bench` feature, so the published addon
// does not carry it. Build it in, run, then rebuild without it before
// committing, because the build regenerates `index.js` and `index.d.ts`:
//   (cd thetadatadx-ts && npx napi build --platform --release --features bench)
//   node thetadatadx-ts/benches/streaming_throughput.mjs
//   (cd thetadatadx-ts && npm run build)
//
// Output is line-delimited then a final `JSON {...}` summary line.

import os from 'node:os';
import { createRequire } from 'node:module';

const { forEachEvent } = createRequire(import.meta.url)('../record_batch_forwarder.js');

let mod;
try {
  mod = await import('../index.js');
} catch (err) {
  console.error('FAIL: native addon not built; run `npx napi build --platform --release --features bench` first');
  console.error(err);
  process.exit(1);
}

if (typeof mod.__benchFloodEvents !== 'function') {
  console.error(
    'FAIL: __benchFloodEvents not found on the addon. Build with `npx napi build --platform --release --features bench`.',
  );
  process.exit(1);
}

// Events per sample. Matches `EVENTS_PER_ITER` in the Rust bench so the
// per-sample wall clock dwarfs measurement overhead and the consumer
// reaches steady state.
const EVENTS_PER_ITER = 100_000;

// Warmup samples (discarded) + measured samples. The Rust criterion bench
// warms up for 3 s then collects 10 samples; we fix explicit counts so the
// budgets are comparable and the run is bounded.
const WARMUP_SAMPLES = 3;
const MEASURED_SAMPLES = 12;

// Run one flood of EVENTS_PER_ITER events through the real tsfn path.
// Returns { eventsPerSec, nsPerEvent, received }.
async function runOneSample() {
  let received = 0;
  // Resolve once the JS side has been invoked exactly N times.
  let resolveAllDelivered;
  const allDelivered = new Promise((resolve) => {
    resolveAllDelivered = resolve;
  });

  // No-op user callback: bumps the received counter and, on the Nth
  // invocation, signals completion. The measured cost is everything the
  // SDK does to get here, not this handler body.
  const callback = (_event) => {
    received += 1;
    if (received === EVENTS_PER_ITER) {
      resolveAllDelivered();
    }
  };

  const start = process.hrtime.bigint();
  // Kick off the flood. The export queues all N through the real bounded
  // tsfn on a worker thread; this promise resolves once all N are queued.
  const floodDone = mod.__benchFloodEvents(EVENTS_PER_ITER, forEachEvent(callback));
  // Wait for BOTH: every event queued (floodDone) AND every JS callback
  // fired (allDelivered). The latter is the true end of delivery.
  await Promise.all([floodDone, allDelivered]);
  const end = process.hrtime.bigint();

  const elapsedNs = Number(end - start);
  const eventsPerSec = EVENTS_PER_ITER / (elapsedNs / 1e9);
  const nsPerEvent = elapsedNs / EVENTS_PER_ITER;
  return { eventsPerSec, nsPerEvent, received };
}

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

async function main() {
  // Warmup — fault in the worker thread, warm V8's call path and the
  // napi queue. Discarded. Also fails loud on any drop during warmup.
  for (let i = 0; i < WARMUP_SAMPLES; i++) {
    const r = await runOneSample();
    if (r.received !== EVENTS_PER_ITER) {
      console.error(
        `FATAL: warmup drop (received=${r.received} != ${EVENTS_PER_ITER}) — ceiling invalid`,
      );
      process.exit(1);
    }
  }

  const eventsPerSec = [];
  const nsPerEvent = [];
  for (let i = 0; i < MEASURED_SAMPLES; i++) {
    const r = await runOneSample();
    if (r.received !== EVENTS_PER_ITER) {
      console.error(
        `FATAL: sample ${i} drop (received=${r.received} != ${EVENTS_PER_ITER}) — ceiling invalid`,
      );
      process.exit(1);
    }
    eventsPerSec.push(r.eventsPerSec);
    nsPerEvent.push(r.nsPerEvent);
    console.log(
      `sample ${String(i).padStart(2)}: ${(r.eventsPerSec / 1e6).toFixed(3).padStart(8)} Melem/s   ` +
        `${r.nsPerEvent.toFixed(2).padStart(8)} ns/event   received=${r.received}`,
    );
  }

  const p50 = median(eventsPerSec);
  const epsMin = Math.min(...eventsPerSec);
  const epsMax = Math.max(...eventsPerSec);
  const nspeP50 = median(nsPerEvent);

  const summary = {
    bench: 'typescript_napi_tsfn_noop',
    granularity: 'event_arrays_per_threadsafe_function_call',
    events_per_iter: EVENTS_PER_ITER,
    warmup_samples: WARMUP_SAMPLES,
    measured_samples: MEASURED_SAMPLES,
    events_per_sec_p50: p50,
    events_per_sec_min: epsMin,
    events_per_sec_max: epsMax,
    ns_per_event_p50: nspeP50,
    zero_drop_verified: true,
    machine: {
      node: process.version,
      arch: process.arch,
      cpus: os.cpus().length,
    },
  };
  console.log('');
  console.log(
    `p50: ${(p50 / 1e6).toFixed(3)} Melem/s   (${nspeP50.toFixed(2)} ns/event)   ` +
      `min=${(epsMin / 1e6).toFixed(3)}  max=${(epsMax / 1e6).toFixed(3)} Melem/s   ` +
      `zero-drop verified over ${MEASURED_SAMPLES} samples`,
  );
  console.log('JSON ' + JSON.stringify(summary));
}

await main();
