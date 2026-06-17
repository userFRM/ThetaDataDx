// Slow-callback watchdog accessibility + round-trip test.
//
// The watchdog counter forwards to
// `thetadatadx::Client::slow_callback_count` and is surfaced to JS as
// `client.stream.slowCallbackCount(): bigint`. The threshold setter
// forwards to `set_slow_callback_threshold` and crosses the boundary as
// microseconds via `client.stream.setSlowCallbackThresholdUs(bigint)`.
// Pass `0n` to disable the watchdog. It is observability-only: the
// watchdog never cancels the callback.
//
// This test pins the contract: the getter is callable before streaming,
// after `startStreaming(callback)`, after a subsequent `reconnect()`,
// and after `stopStreaming()`; the value is non-negative across the
// cycle; the setter round-trips without throwing.
//
// Gated on THETADATADX_TEST_CREDS=/path/to/creds.txt — the underlying
// `Client.connectFromFile(...)` needs a live FPSS handshake. Skips
// silently on dev machines without creds.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('client.stream slow-callback watchdog', () => {
  it('getter is callable and setter round-trips across the lifecycle', async () => {
    const credsPath = process.env.THETADATADX_TEST_CREDS;
    if (!credsPath) {
      console.log(
        'SKIP: set THETADATADX_TEST_CREDS=/path/to/creds.txt to enable this live test'
      );
      return;
    }

    const client = await mod.Client.connectFromFile(credsPath);

    // Pre-stream: the FPSS client does not exist yet, so the count is 0.
    // The threshold setter is a no-op in this state and must not throw.
    const pre = client.stream.slowCallbackCount();
    assert.equal(typeof pre, 'bigint', 'slowCallbackCount() must return bigint');
    assert.equal(pre, 0n, 'pre-stream count must be 0 -- nothing has run slow');
    client.stream.setSlowCallbackThresholdUs(2_500n);
    assert.equal(client.stream.slowCallbackCount(), 0n);

    let received = 0n;
    await client.stream.startStreaming(() => {
      received += 1n;
    });
    // Configure a 1 ms budget on the live session.
    client.stream.setSlowCallbackThresholdUs(1_000n);
    const postStart = client.stream.slowCallbackCount();
    assert.equal(typeof postStart, 'bigint');
    assert.ok(postStart >= 0n);

    await client.stream.reconnect();
    const postReconnect = client.stream.slowCallbackCount();
    assert.equal(typeof postReconnect, 'bigint');
    // reconnect rebuilds the FPSS client and zeroes the counter; assert
    // non-negative rather than monotone (monotone is not promised).
    assert.ok(postReconnect >= 0n);

    client.stream.stopStreaming();
    const postStop = client.stream.slowCallbackCount();
    assert.equal(typeof postStop, 'bigint');
    assert.ok(postStop >= 0n);

    // Disabling the watchdog round-trips too.
    client.stream.setSlowCallbackThresholdUs(0n);
    assert.equal(typeof received, 'bigint');
  });
});
