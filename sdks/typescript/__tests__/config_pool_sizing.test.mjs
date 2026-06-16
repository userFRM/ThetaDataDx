// Historical pool-sizing setter on `Config`.
//
// Locks the contract that the `concurrentRequests` property exposed
// by the `Config` napi class round-trips through napi-rs to the
// underlying Rust `HistoricalConfig` correctly.
//
// Live behaviour (the tier clamp at connect time) is covered by the
// Rust unit tests under `mdds::client::pool_size_tests`; this file
// pins only the JS surface contract.
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

describe('Config.concurrentRequests', () => {
  it('defaults to 0 (auto-detect sentinel)', () => {
    const cfg = Config.production();
    assert.equal(cfg.concurrentRequests, 0);
  });

  it('round-trips through the setter', () => {
    const cfg = Config.production();
    for (const n of [1, 2, 4, 8, 16]) {
      cfg.setConcurrentRequests(n);
      assert.equal(cfg.concurrentRequests, n);
    }
  });
});

describe('Config.requestTimeoutSecs', () => {
  it('defaults to 300n (5-minute per-request deadline)', () => {
    const cfg = Config.production();
    assert.equal(cfg.requestTimeoutSecs, 300n);
  });

  it('round-trips through the setter; 0n disables the default', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 45n, 120n, 600n]) {
      cfg.setRequestTimeoutSecs(secs);
      assert.equal(cfg.requestTimeoutSecs, secs);
    }
  });
});
