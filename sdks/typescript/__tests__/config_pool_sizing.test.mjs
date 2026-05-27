// MDDS pool-sizing setters on `Config` (issue #584).
//
// Locks the contract that the two properties exposed by the
// `Config` napi class — `concurrentRequests`, `decoderRingSize`
// — round-trip through napi-rs to the underlying Rust
// `MddsConfig` correctly, and that invalid ring sizes raise at
// the setter boundary rather than waiting for connect-time validate.
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

describe('Config.decoderRingSize', () => {
  it('defaults to 256 (production baseline)', () => {
    const cfg = Config.production();
    assert.equal(cfg.decoderRingSize, 256);
  });

  it('accepts every power of two >= 64', () => {
    const cfg = Config.production();
    for (const n of [64, 128, 256, 512, 1024, 2048, 4096]) {
      cfg.setDecoderRingSize(n);
      assert.equal(cfg.decoderRingSize, n);
    }
  });

  it('rejects values below the 64-slot minimum', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setDecoderRingSize(32),
      /decoder_ring_size/,
      'must reject 32 < 64',
    );
  });

  it('rejects non-power-of-two values', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setDecoderRingSize(100),
      /decoder_ring_size/,
      'must reject 100 (not power of two)',
    );
    assert.throws(
      () => cfg.setDecoderRingSize(1023),
      /decoder_ring_size/,
      'must reject 1023 (not power of two)',
    );
  });

  it('rejects zero', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setDecoderRingSize(0),
      /decoder_ring_size/,
      'must reject 0',
    );
  });
});

describe('Config pool-sizing setters are independent', () => {
  it('does not interfere with each other across both properties', () => {
    const cfg = Config.production();
    cfg.setConcurrentRequests(8);
    cfg.setDecoderRingSize(1024);
    assert.equal(cfg.concurrentRequests, 8);
    assert.equal(cfg.decoderRingSize, 1024);
  });
});
