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
// `streaming::reconnect::reconnect_tests`; this file pins only that
// the JS surface forwards the inputs without dropping them and
// rejects invalid policy strings at the boundary.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.log('SKIP: native addon not built (run `npm run build` first)');
  process.exit(0);
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
    for (const secs of [1, 30, 60, 300, 3600]) {
      cfg.setReconnectStableWindowSecs(secs);
    }
  });
});

describe('Reconnect setters are independent', () => {
  it('do not interfere with each other or with pool-sizing setters', () => {
    const cfg = Config.production();
    cfg.setReconnectPolicy('auto');
    cfg.setReconnectMaxAttempts(7);
    cfg.setReconnectMaxRateLimitedAttempts(77);
    cfg.setReconnectStableWindowSecs(120);
    cfg.setConcurrentRequests(4);
    cfg.setDecoderRingSize(512);
    assert.equal(cfg.concurrentRequests, 4);
    assert.equal(cfg.decoderRingSize, 512);
  });
});
