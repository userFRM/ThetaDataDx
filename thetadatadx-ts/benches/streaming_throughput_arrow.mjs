// TypeScript streaming throughput — Arrow columnar batch lever (offline, no network).
//
// Lever 3 for TypeScript: the columnar analogue of the Python Arrow lever.
// The per-event and array-batch models cap TypeScript at ~0.3M events/s
// because each event still materializes a full StreamEvent JS object. This
// path bypasses per-event JS-object construction entirely:
//
//   - Rust accumulates `batch_size` synthetic trade events into a TradeTick
//     RecordBatch and serializes ONE Arrow IPC byte buffer per batch (the
//     same `TicksArrowExt::to_arrow` -> `arrow_ipc::StreamWriter` path the
//     SDK's `tradeTickToArrowIpc` export uses).
//   - The buffer crosses the ThreadsafeFunction boundary ONCE per batch as a
//     napi `Buffer` (NOT N JS objects).
//   - Node consumes it columnar via `apache-arrow` (`tableFromIPC`) as a
//     Table; no per-event StreamEvent object is ever built on the JS side.
//
// DIFFERENT delivery model than the per-event / array-batch callbacks — Node
// receives a columnar Arrow Table, not typed event objects — so the number
// is the columnar bulk ceiling, the TypeScript analogue of the Python Arrow
// row.
//
// Methodology (matches the other levers)
// --------------------------------------
// * EVENTS_PER_ITER = 100_000 events/sample; batch sweep 256 / 1024 / 4096
//   (override via THETADX_BATCHES, comma-sep).
// * `process.hrtime.bigint()` brackets "flood started" to "JS side has
//   consumed Arrow rows totalling N". Rust marshal + tsfn.call run on a
//   worker thread; module load / addon init excluded.
// * Warmup samples discarded; median (p50) reported with min/max.
// * Zero-drop two ways: the export returns the count of non-Ok tsfn.call
//   statuses (asserted 0) AND the apache-arrow Table.numRows summed across
//   all batch buffers is asserted === N (receive-side columnar check).
//
// `apache-arrow` is a devDependency (added for this bench driver). If it is
// not installed the bench exits with a clear message rather than a silent skip.
//
// Run:
//   node thetadatadx-ts/benches/streaming_throughput_arrow.mjs
//
// Output: line-delimited per-sample summary, then one `JSON {...}` per variant.

import os from 'node:os';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

let mod;
try {
  mod = await import('../index.js');
} catch (err) {
  console.error('FAIL: native addon not built; run `npm run build` first');
  console.error(err);
  process.exit(1);
}
if (typeof mod.__benchFloodEventsArrowIpc !== 'function') {
  console.error('FAIL: __benchFloodEventsArrowIpc not found on the addon. Rebuild with `npm run build`.');
  process.exit(1);
}

let tableFromIPC;
try {
  ({ tableFromIPC } = require('apache-arrow'));
} catch {
  console.error('FAIL: apache-arrow not installed. Run `npm install` (it is a devDependency for this bench).');
  process.exit(1);
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

// One Arrow-batch flood: B trade events per Arrow IPC buffer per tsfn hop.
// The JS callback decodes each buffer columnar via apache-arrow and tallies
// the row count — the same columnar consume path a real integrator uses.
async function runArrow(batchSize) {
  let rows = 0;
  let resolveAll;
  const allConsumed = new Promise((r) => (resolveAll = r));
  const cb = (ipcBuffer) => {
    // Columnar decode: NO per-event JS object construction. tableFromIPC
    // aliases the buffer's columnar arrays into an Arrow Table.
    const table = tableFromIPC(ipcBuffer);
    rows += table.numRows;
    if (rows >= EVENTS_PER_ITER) resolveAll();
  };
  const start = process.hrtime.bigint();
  const flood = mod.__benchFloodEventsArrowIpc(EVENTS_PER_ITER, batchSize, cb);
  const [drops] = await Promise.all([flood, allConsumed]);
  const end = process.hrtime.bigint();
  const elapsedNs = Number(end - start);
  return {
    eventsPerSec: EVENTS_PER_ITER / (elapsedNs / 1e9),
    nsPerEvent: elapsedNs / EVENTS_PER_ITER,
    drops: Number(drops),
    received: rows,
  };
}

async function runVariant(name, runOne) {
  for (let i = 0; i < WARMUP_SAMPLES; i++) {
    const r = await runOne();
    if (r.drops !== 0 || r.received !== EVENTS_PER_ITER) {
      console.error(`FATAL[${name}]: warmup drop (drops=${r.drops}, rows=${r.received} != ${EVENTS_PER_ITER})`);
      process.exit(1);
    }
  }
  const eps = [];
  const nspe = [];
  for (let i = 0; i < MEASURED_SAMPLES; i++) {
    const r = await runOne();
    if (r.drops !== 0 || r.received !== EVENTS_PER_ITER) {
      console.error(`FATAL[${name}]: sample ${i} drop (drops=${r.drops}, rows=${r.received} != ${EVENTS_PER_ITER})`);
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
    `# Node ${process.version}, Arrow-IPC columnar batch, events/iter=${EVENTS_PER_ITER}, warmup=${WARMUP_SAMPLES}, samples=${MEASURED_SAMPLES}`,
  );
  const summaries = [];
  for (const B of BATCHES) {
    const s = await runVariant(`arrow_ipc[${B}]`, () => runArrow(B));
    summaries.push({ variant: 'arrow_ipc', batch_size: B, ...s });
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
