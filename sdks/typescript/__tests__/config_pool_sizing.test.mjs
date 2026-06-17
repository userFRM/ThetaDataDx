// Historical tuning setters on `Config`.
//
// Locks the contract that the historical tuning properties exposed by
// the `Config` napi class (such as `requestTimeoutSecs`) round-trip
// through napi-rs to the underlying Rust `HistoricalConfig` correctly.
//
// Live behaviour (the per-tier connection-pool concurrency limit
// resolved at connect time) is covered by the Rust unit tests under
// `mdds::client::pool_size_tests`; this file pins only the JS surface
// contract.
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
