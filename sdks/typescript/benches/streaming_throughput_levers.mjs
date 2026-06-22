// TypeScript streaming throughput — batched delivery lever (offline, no network).
//
// Measures the TRUE MAX TypeScript streaming throughput by amortizing the
// per-event ThreadsafeFunction hop, against the per-event baseline (~233k
// events/s) the companion `streaming_throughput.mjs` records.
//
// Variants
// --------
// 1. per_event     — baseline: one `tsfn.call` (one event) per hop.
//                    (`__benchFloodEvents`)
// 2. batched[B]    — Lever 1: one `tsfn.call` carrying an Array<StreamEvent>
//                    of B events per hop (`__benchFloodEventsBatched`).
//                    Amortizes the threadsafe-function crossing + the V8
//                    callback invocation over a whole batch. Same per-event
//                    marshal (`fpss_event_to_buffered` -> `buffered_event_to_typed`).
//
// Batch sweep: 256 / 1024 / 4096 (override via THETADX_BATCHES, comma-sep).
//
// Methodology (matches the per-event bench + the core Rust bench)
// ---------------------------------------------------------------
// * EVENTS_PER_ITER = 100_000 events per sample.
// * `process.hrtime.bigint()` brackets "flood started" to "JS side has
//   received all N events". The export's marshal + `tsfn.call(.., Blocking)`
//   run on a worker thread (off the libuv main thread), exactly as the live
//   dispatcher; module load / addon init are excluded.
// * Warmup samples discarded; median (p50) reported with min/max.
// * Zero-drop two ways per sample: the export returns the count of non-Ok
//   `tsfn.call` statuses (asserted 0) AND the JS side's received-event count
//   is asserted === N (for batched, summed across all batch callbacks).
//
// Run:
//   node sdks/typescript/benches/streaming_throughput_levers.mjs
//
// Output: line-delimited per-sample summary, then one `JSON {...}` per variant.

import os from 'node:os';

let mod;
try {
  mod = await import('../index.js');
} catch (err) {
  console.error('FAIL: native addon not built; run `npm run build` first');
  console.error(err);
  process.exit(1);
}
for (const fn of ['__benchFloodEvents', '__benchFloodEventsBatched']) {
  if (typeof mod[fn] !== 'function') {
    console.error(`FAIL: ${fn} not found on the addon. Rebuild with \`npm run build\`.`);
    process.exit(1);
  }
}

const EVENTS_PER_ITER = 100_000;
const WARMUP_SAMPLES = 3;
const MEASURED_SAMPLES = 12;
const BATCHES = (process.env.THETADX_BATCHES || '256,1024,4096')
  .split(',')
  .map((s) => parseInt(s, 10));

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

// One per-event flood: one event per tsfn hop.
async function runPerEvent() {
  let received = 0;
  let resolveAll;
  const allDelivered = new Promise((r) => (resolveAll = r));
  const cb = (_e) => {
    received += 1;
    if (received === EVENTS_PER_ITER) resolveAll();
  };
  const start = process.hrtime.bigint();
  const flood = mod.__benchFloodEvents(EVENTS_PER_ITER, cb);
  const [drops] = await Promise.all([flood, allDelivered]);
  const end = process.hrtime.bigint();
  return finalize(start, end, Number(drops), received);
}

// One batched flood: B events per tsfn hop (callback receives an array).
async function runBatched(batchSize) {
  let received = 0;
  let resolveAll;
  const allDelivered = new Promise((r) => (resolveAll = r));
  const cb = (events) => {
    received += events.length;
    if (received >= EVENTS_PER_ITER) resolveAll();
  };
  const start = process.hrtime.bigint();
  const flood = mod.__benchFloodEventsBatched(EVENTS_PER_ITER, batchSize, cb);
  const [drops] = await Promise.all([flood, allDelivered]);
  const end = process.hrtime.bigint();
  return finalize(start, end, Number(drops), received);
}

function finalize(start, end, drops, received) {
  const elapsedNs = Number(end - start);
  return {
    eventsPerSec: EVENTS_PER_ITER / (elapsedNs / 1e9),
    nsPerEvent: elapsedNs / EVENTS_PER_ITER,
    drops,
    received,
  };
}

async function runVariant(name, runOne) {
  for (let i = 0; i < WARMUP_SAMPLES; i++) {
    const r = await runOne();
    if (r.drops !== 0 || r.received !== EVENTS_PER_ITER) {
      console.error(`FATAL[${name}]: warmup drop (drops=${r.drops}, received=${r.received} != ${EVENTS_PER_ITER})`);
      process.exit(1);
    }
  }
  const eps = [];
  const nspe = [];
  for (let i = 0; i < MEASURED_SAMPLES; i++) {
    const r = await runOne();
    if (r.drops !== 0 || r.received !== EVENTS_PER_ITER) {
      console.error(`FATAL[${name}]: sample ${i} drop (drops=${r.drops}, received=${r.received} != ${EVENTS_PER_ITER})`);
      process.exit(1);
    }
    eps.push(r.eventsPerSec);
    nspe.push(r.nsPerEvent);
  }
  const p50 = median(eps);
  const nspeP50 = median(nspe);
  console.log(
    `  ${name.padEnd(20)}: p50 ${(p50 / 1e6).toFixed(3).padStart(8)} Melem/s  ${nspeP50.toFixed(2).padStart(8)} ns/event  (zero-drop ${MEASURED_SAMPLES}x)`,
  );
  return { p50, epsMin: Math.min(...eps), epsMax: Math.max(...eps), nspeP50 };
}

async function main() {
  console.log(
    `# Node ${process.version}, events/iter=${EVENTS_PER_ITER}, warmup=${WARMUP_SAMPLES}, samples=${MEASURED_SAMPLES}`,
  );
  const summaries = [];

  let s = await runVariant('per_event', runPerEvent);
  summaries.push({ variant: 'per_event', batch_size: 1, ...s });

  for (const B of BATCHES) {
    s = await runVariant(`batched[${B}]`, () => runBatched(B));
    summaries.push({ variant: 'batched', batch_size: B, ...s });
  }

  const machine = { node: process.version, arch: process.arch, cpus: os.cpus().length };
  console.log('');
  for (const sm of summaries) {
    console.log(
      'JSON ' +
        JSON.stringify({
          binding: 'typescript',
          variant: sm.variant,
          batch_size: sm.batch_size,
          events_per_sec_p50: sm.p50,
          events_per_sec_min: sm.epsMin,
          events_per_sec_max: sm.epsMax,
          ns_per_event_p50: sm.nspeP50,
          events_per_iter: EVENTS_PER_ITER,
          warmup_samples: WARMUP_SAMPLES,
          measured_samples: MEASURED_SAMPLES,
          zero_drop_verified: true,
          machine,
        }),
    );
  }
}

await main();
