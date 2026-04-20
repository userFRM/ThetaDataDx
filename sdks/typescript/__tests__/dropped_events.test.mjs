// Dropped-events counter accessibility test.
//
// Verifies the fix for audit finding `A-02` in
// todo.md / the security-audit branch: the per-closure AtomicU64
// counter used to be local to each startStreaming / reconnect
// closure, so it reset on every reconnect AND was never reachable
// from JS. The fix lifts the counter to an instance field on
// ThetaDataDx (Rust side) and exposes it as `tdx.droppedEvents(): bigint`.
//
// This test pins the contract: the getter is callable after one
// startStreaming and after a subsequent reconnect, and returns a
// non-negative bigint (u64 on the Rust side).
//
// Gated on THETADX_TEST_CREDS=/path/to/creds.txt — the underlying
// `ThetaDataDx.connectFromFile(...)` needs a live FPSS handshake.
// Skips silently on dev machines without creds; CI runs this in the
// surfaces job. Same pattern as sdks/go/timeout_test.go.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

describe('tdx.droppedEvents()', () => {
  it('is callable before/after startStreaming and after reconnect', async () => {
    const credsPath = process.env.THETADX_TEST_CREDS;
    if (!credsPath) {
      console.log(
        'SKIP: set THETADX_TEST_CREDS=/path/to/creds.txt to enable this live test'
      );
      return;
    }

    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      console.log('SKIP: native addon not built (run `npm run build` first)');
      return;
    }

    const tdx = mod.ThetaDataDx.connectFromFile(credsPath);

    // Pre-stream: counter is initialised on the instance, not inside
    // the startStreaming closure. Must already be readable and 0.
    const pre = tdx.droppedEvents();
    assert.equal(typeof pre, 'bigint', 'droppedEvents() must return bigint');
    assert.ok(pre >= 0n, 'pre-stream count must be non-negative');
    assert.equal(pre, 0n, 'pre-stream count must be 0 -- nothing has dropped');

    tdx.startStreaming();
    const postStart = tdx.droppedEvents();
    assert.equal(typeof postStart, 'bigint');
    assert.ok(postStart >= 0n);

    tdx.reconnect();
    const postReconnect = tdx.droppedEvents();
    assert.equal(typeof postReconnect, 'bigint');
    // Must be monotonically non-decreasing across reconnect. A reset
    // would imply the closure-local regression was reintroduced.
    assert.ok(
      postReconnect >= postStart,
      `counter reset across reconnect: post-start=${postStart} post-reconnect=${postReconnect}`
    );

    tdx.stopStreaming();
    const postStop = tdx.droppedEvents();
    assert.equal(typeof postStop, 'bigint');
    // Still readable after stop -- counter lives on the handle, not
    // the receiver channel.
    assert.ok(postStop >= postReconnect);
  });
});
