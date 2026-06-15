// Ring-occupancy observability surface test.
//
// `tdx.ringOccupancy()` is a point-in-time sample of events published
// into the streaming event ring but not yet drained into the callback;
// `tdx.ringCapacity()` is the configured ring size. The pair is the
// leading back-pressure signal: `droppedEventCount()` only moves AFTER
// data has been lost, while a rising occupancy approaching capacity
// predicts those drops. Both forward to the same Rust core accessors
// as every other binding (C ABI, Python, C++) and are surfaced to JS
// as `bigint` for shape-consistency with the other streaming counters.
//
// This test pins the contract: both getters exist on the class
// surface (offline), are callable before streaming and after
// `stopStreaming()` (returning 0n in both states), and report a
// positive power-of-two capacity with occupancy bounded by it while a
// stream is live.
//
// The live section is gated on THETADX_TEST_CREDS=/path/to/creds.txt —
// the underlying `Client.connectFromFile(...)` needs a live
// FPSS handshake. Skips silently on dev machines without creds; CI
// runs this in the surfaces job. Mirrors dropped_events.test.mjs.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// CI build step is mandatory before `npm test`; fail loud if the addon
// is missing so a broken build does not appear green.
let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('tdx.ringOccupancy() / tdx.ringCapacity()', () => {
  it('exists on the class surface alongside droppedEventCount', () => {
    assert.equal(
      typeof mod.Client.prototype.ringOccupancy,
      'function',
      'ringOccupancy() must be a method on Client'
    );
    assert.equal(
      typeof mod.Client.prototype.ringCapacity,
      'function',
      'ringCapacity() must be a method on Client'
    );
  });

  it('is callable across the streaming lifecycle and reads 0n when stopped', async () => {
    const credsPath = process.env.THETADX_TEST_CREDS;
    if (!credsPath) {
      // Live test — credentials are a legitimate runtime opt-in, not a build artefact.
      console.log(
        'SKIP: set THETADX_TEST_CREDS=/path/to/creds.txt to enable this live test'
      );
      return;
    }

    const tdx = mod.Client.connectFromFile(credsPath);

    // Pre-stream: the FPSS client does not exist yet, so both read 0n.
    const preOccupancy = tdx.ringOccupancy();
    const preCapacity = tdx.ringCapacity();
    assert.equal(typeof preOccupancy, 'bigint', 'ringOccupancy() must return bigint');
    assert.equal(typeof preCapacity, 'bigint', 'ringCapacity() must return bigint');
    assert.equal(preOccupancy, 0n, 'pre-stream occupancy must be 0n -- no ring exists');
    assert.equal(preCapacity, 0n, 'pre-stream capacity must be 0n -- no ring exists');

    // Live: capacity reports the configured ring size (a positive
    // power of two) and occupancy is bounded by it. No exact
    // occupancy value is asserted — it is a racy point-in-time
    // sample of a fast consumer.
    tdx.startStreaming(() => {});
    const capacity = tdx.ringCapacity();
    assert.ok(capacity > 0n, 'a live ring must report its configured capacity');
    assert.equal(capacity & (capacity - 1n), 0n, 'ring capacity is a power of two');
    const occupancy = tdx.ringOccupancy();
    assert.ok(occupancy >= 0n, 'occupancy is clamped non-negative');
    assert.ok(occupancy <= capacity, 'occupancy never exceeds capacity');

    // Stopped: the streaming slot is empty; both forwarders return 0n.
    tdx.stopStreaming();
    assert.equal(tdx.ringOccupancy(), 0n);
    assert.equal(tdx.ringCapacity(), 0n);
  });
});
