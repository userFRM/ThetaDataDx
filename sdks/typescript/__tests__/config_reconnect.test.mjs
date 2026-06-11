// ReconnectConfig setters on `Config` — TypeScript binding parity
// with Python / C++ / FFI. Pins the JS surface contract for
// `setReconnectPolicy`, `setReconnectMaxAttempts`,
// `setReconnectMaxRateLimitedAttempts`, and
// `setReconnectStableWindowSecs`.
//
// The setters mutate the underlying `DirectConfig.reconnect` field
// the Rust core consumes at connect time. Failure-class semantics
// (transient vs rate-limited budget split, stable-window timer reset)
// are exercised in the Rust unit tests under
// `fpss::session::tests` and
// `fpss::protocol::reconnect_delays_match_policy`; this file pins
// only that the JS surface forwards the inputs without dropping them
// and rejects invalid policy strings at the boundary.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

const { Config } = mod;

describe('Config.setReconnectPolicy', () => {
  it('accepts "auto" and "manual" (case-insensitive)', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    cfg.setReconnectPolicy('AUTO');
    cfg.setReconnectPolicy('manual');
    cfg.setReconnectPolicy('Manual');
  });

  it('rejects unknown policy strings', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setReconnectPolicy('linear-backoff'),
      /reconnect_policy/,
      'must reject unknown policy name',
    );
  });
});

describe('Config.setReconnectMaxAttempts', () => {
  it('accepts non-zero budgets without throwing', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    for (const n of [1, 3, 10, 100, 1000]) {
      cfg.setReconnectMaxAttempts(n);
    }
  });

  it('is silently a no-op when policy is manual', () => {
    // Matches the Python contract: setter has no effect when the
    // reconnect policy is not the Auto(limits) variant. We assert
    // the setter does not throw — there is no getter on this knob
    // (FFI / Python / C++ are all write-only here).
    const cfg = Config.production();
    cfg.setReconnectPolicy('manual');
    cfg.setReconnectMaxAttempts(5);
  });
});

describe('Config.setReconnectMaxRateLimitedAttempts', () => {
  it('accepts non-zero budgets without throwing', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    for (const n of [1, 10, 100, 1000]) {
      cfg.setReconnectMaxRateLimitedAttempts(n);
    }
  });
});

describe('Config.setReconnectStableWindowSecs', () => {
  it('accepts non-zero window durations without throwing', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    for (const secs of [1n, 30n, 60n, 300n, 3600n]) {
      cfg.setReconnectStableWindowSecs(secs);
    }
  });

  it('rejects negative BigInt', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    assert.throws(
      () => cfg.setReconnectStableWindowSecs(-1n),
      /setReconnectStableWindowSecs/,
      'negative window seconds must be rejected at the boundary',
    );
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    // A BigInt that does not fit in 64 bits must be rejected at
    // the boundary rather than silently truncating to the low
    // 64 bits of the magnitude.
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    assert.throws(
      () => cfg.setReconnectStableWindowSecs(1n << 65n),
      /setReconnectStableWindowSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Pool-sizing setter state survives interleaved reconnect setter calls', () => {
  it('reconnect setters do not interfere with pool-sizing getters', () => {
    // The reconnect setters expose no getters, so the contract
    // we can verify is: after interleaving reconnect setter
    // calls with pool-sizing setter calls, the pool-sizing
    // getters still observe the values that were last written.
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    cfg.setReconnectMaxAttempts(7);
    cfg.setReconnectMaxRateLimitedAttempts(77);
    cfg.setReconnectStableWindowSecs(120n);
    cfg.setConcurrentRequests(4);
    cfg.setDecoderRingSize(512);
    assert.equal(cfg.concurrentRequests, 4);
    assert.equal(cfg.decoderRingSize, 512);
  });
});

describe('Config.setReconnectWaitMs / setReconnectWaitRateLimitedMs', () => {
  it('default to the wire-constant cadences', () => {
    const cfg = Config.production();
    assert.equal(cfg.reconnectWaitMs, 250n);
    assert.equal(cfg.reconnectWaitRateLimitedMs, 130_000n);
  });

  it('round-trip via getters across the documented range', () => {
    const cfg = Config.production();
    for (const ms of [0n, 1n, 500n, 2_000n, 60_000n, 130_000n, 1n << 60n]) {
      cfg.setReconnectWaitMs(ms);
      assert.equal(cfg.reconnectWaitMs, ms);
      cfg.setReconnectWaitRateLimitedMs(ms);
      assert.equal(cfg.reconnectWaitRateLimitedMs, ms);
    }
  });

  it('reject BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setReconnectWaitMs(1n << 65n),
      /setReconnectWaitMs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
    assert.throws(
      () => cfg.setReconnectWaitRateLimitedMs(1n << 65n),
      /setReconnectWaitRateLimitedMs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Config.setTokioWorkerThreadsExplicit', () => {
  it('default tokio_worker_threads is the None (auto) sentinel', () => {
    const cfg = Config.production();
    const got = cfg.tokioWorkerThreads;
    assert.equal(got.hasValue, false, 'default must be None');
    assert.equal(got.n, 0);
  });

  it('round-trip preserves Some(0) and explicit pinned counts', () => {
    const cfg = Config.production();
    for (const n of [0, 1, 2, 4, 8, 16, 64]) {
      cfg.setTokioWorkerThreadsExplicit(true, n);
      const got = cfg.tokioWorkerThreads;
      assert.equal(got.hasValue, true);
      assert.equal(got.n, n);
    }
    // Reset to None.
    cfg.setTokioWorkerThreadsExplicit(false, 999);
    const got = cfg.tokioWorkerThreads;
    assert.equal(got.hasValue, false);
    assert.equal(got.n, 0);
  });
});

describe('Config.setRetry* — RetryPolicy field setters', () => {
  it('expose the four RetryPolicy field defaults', () => {
    const cfg = Config.production();
    assert.equal(cfg.retryInitialDelayMs, 250n);
    assert.equal(cfg.retryMaxDelayMs, 30_000n);
    assert.equal(cfg.retryMaxAttempts, 20);
    assert.equal(cfg.retryJitter, true);
  });

  it('round-trip via per-field setters', () => {
    const cfg = Config.production();
    cfg.setRetryInitialDelayMs(500n);
    cfg.setRetryMaxDelayMs(60_000n);
    cfg.setRetryMaxAttempts(7);
    cfg.setRetryJitter(false);
    assert.equal(cfg.retryInitialDelayMs, 500n);
    assert.equal(cfg.retryMaxDelayMs, 60_000n);
    assert.equal(cfg.retryMaxAttempts, 7);
    assert.equal(cfg.retryJitter, false);
  });

  it('reject BigInt magnitudes above u64::MAX on the duration setters', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setRetryInitialDelayMs(1n << 65n),
      /setRetryInitialDelayMs/,
    );
    assert.throws(
      () => cfg.setRetryMaxDelayMs(1n << 65n),
      /setRetryMaxDelayMs/,
    );
  });
});
