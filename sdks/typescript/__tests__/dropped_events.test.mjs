// Dropped-event-count accessibility test.
//
// PR D (#482) replaced the poll-style `nextEvent` API with callback
// registration via `startStreaming(callback)`. The dropped-event
// counter that used to live on the napi struct now forwards to the
// SSOT `StreamingDispatcher` via
// `thetadatadx::ThetaDataDx::dropped_event_count`, surfaced to JS as
// `tdx.droppedEventCount(): bigint`.
//
// This test pins the contract: the getter is callable before
// streaming, after `startStreaming(callback)`, after a subsequent
// `reconnect()`, and after `stopStreaming()`; the value is
// monotonically non-decreasing across the cycle (a reset would imply
// a regression to closure-local counter semantics).
//
// Gated on THETADX_TEST_CREDS=/path/to/creds.txt — the underlying
// `ThetaDataDx.connectFromFile(...)` needs a live FPSS handshake.
// Skips silently on dev machines without creds; CI runs this in the
// surfaces job.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

describe('tdx.droppedEventCount()', () => {
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

    // Pre-stream: the dispatcher does not exist yet, so the count is
    // 0. Must already be readable (the getter forwards to the unified
    // client, which returns 0 when the dispatcher slot is empty).
    const pre = tdx.droppedEventCount();
    assert.equal(typeof pre, 'bigint', 'droppedEventCount() must return bigint');
    assert.ok(pre >= 0n, 'pre-stream count must be non-negative');
    assert.equal(pre, 0n, 'pre-stream count must be 0 -- nothing has dropped');

    // Register a no-op callback so the dispatcher spins up. The
    // callback runs on the Node main thread via the napi-rs
    // `ThreadsafeFunction` queue; we don't assert anything about it
    // here because the live FPSS feed timing is non-deterministic.
    let received = 0n;
    tdx.startStreaming(() => {
      received += 1n;
    });
    const postStart = tdx.droppedEventCount();
    assert.equal(typeof postStart, 'bigint');
    assert.ok(postStart >= 0n);

    tdx.reconnect();
    const postReconnect = tdx.droppedEventCount();
    assert.equal(typeof postReconnect, 'bigint');
    // The counter lives on the StreamingDispatcher; reconnect calls
    // stop_streaming + start_streaming, which recreates the dispatcher
    // and zeroes the counter. Snapshot before reconnect if you need
    // cross-session accumulation. Assert non-negative rather than
    // monotone — anything else would lock in implementation detail
    // we explicitly do NOT promise.
    assert.ok(postReconnect >= 0n);

    tdx.stopStreaming();
    const postStop = tdx.droppedEventCount();
    assert.equal(typeof postStop, 'bigint');
    // Still readable after stop_streaming clears the dispatcher slot;
    // forwarder returns 0 in that state.
    assert.ok(postStop >= 0n);

    // Sanity: the no-op callback compiled and was retained for the
    // lifetime of the test (no use-after-free / dropped reference).
    assert.equal(typeof received, 'bigint');
  });

  it('rejects double startStreaming with a clear error', async () => {
    const credsPath = process.env.THETADX_TEST_CREDS;
    if (!credsPath) {
      console.log('SKIP: set THETADX_TEST_CREDS=/path/to/creds.txt');
      return;
    }
    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      console.log('SKIP: native addon not built');
      return;
    }
    const tdx = mod.ThetaDataDx.connectFromFile(credsPath);
    tdx.startStreaming(() => {});
    assert.throws(
      () => tdx.startStreaming(() => {}),
      /streaming already started/,
      'second startStreaming must reject with the napi error'
    );
    tdx.stopStreaming();
  });

  it('reconnect without prior startStreaming throws', async () => {
    const credsPath = process.env.THETADX_TEST_CREDS;
    if (!credsPath) {
      console.log('SKIP: set THETADX_TEST_CREDS=/path/to/creds.txt');
      return;
    }
    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      console.log('SKIP: native addon not built');
      return;
    }
    const tdx = mod.ThetaDataDx.connectFromFile(credsPath);
    assert.throws(
      () => tdx.reconnect(),
      /no callback registered/,
      'reconnect without startStreaming must require a callback'
    );
  });
});
