// MDDS two-stage decode pipeline knobs on `Config` (Phase 3 of 3).
//
// Locks the contract that the two new properties exposed by the
// `Config` napi class — `decodeThreads` (stage-2 prost-decode +
// Tick-build worker count) and `decodeQueueDepth` (bounded MPSC
// queue between stage-1 and stage-2) — round-trip through napi-rs
// to the underlying Rust `MddsConfig` correctly, including:
//
//   * `null` / `undefined` is the auto-size sentinel (the napi
//     binding folds both to `None` on the Rust side).
//   * A `number` is the explicit override, retained verbatim.
//   * `0` is a legal explicit value (the pool clamps to `1`
//     internally at connect time).
//
// Stage-1 thread count remains controlled by the legacy
// `decoderThreads` knob; this file pins only the new Phase-3
// stage-2 surface.
//
// Live behaviour (auto-sizing at connect time, the `Some(0) -> max(1)`
// clamp, queue depth defaulting to `concurrent_requests * 64`) is
// covered by the Rust unit tests; this file pins only the JS
// surface contract.
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

describe('Config.decodeThreads', () => {
  it('defaults to null (auto-size sentinel)', () => {
    const cfg = Config.production();
    assert.equal(cfg.decodeThreads, null);
  });

  it('round-trips through the setter with an explicit number', () => {
    const cfg = Config.production();
    for (const n of [1, 2, 4, 8, 16, 32]) {
      cfg.setDecodeThreads(n);
      assert.equal(cfg.decodeThreads, n);
    }
  });

  it('accepts null and returns to the auto-size sentinel', () => {
    const cfg = Config.production();
    cfg.setDecodeThreads(8);
    assert.equal(cfg.decodeThreads, 8);
    cfg.setDecodeThreads(null);
    assert.equal(cfg.decodeThreads, null);
  });

  it('accepts undefined and returns to the auto-size sentinel', () => {
    const cfg = Config.production();
    cfg.setDecodeThreads(8);
    assert.equal(cfg.decodeThreads, 8);
    cfg.setDecodeThreads(undefined);
    assert.equal(cfg.decodeThreads, null);
  });

  it('accepts an explicit 0 (pool clamps to 1 internally)', () => {
    const cfg = Config.production();
    cfg.setDecodeThreads(0);
    assert.equal(cfg.decodeThreads, 0);
  });

  it('retains a large value verbatim', () => {
    const cfg = Config.production();
    cfg.setDecodeThreads(4096);
    assert.equal(cfg.decodeThreads, 4096);
  });
});

describe('Config.decodeQueueDepth', () => {
  it('defaults to null (auto-size sentinel)', () => {
    const cfg = Config.production();
    assert.equal(cfg.decodeQueueDepth, null);
  });

  it('round-trips through the setter with an explicit number', () => {
    const cfg = Config.production();
    for (const n of [1, 64, 128, 512, 2048, 8192]) {
      cfg.setDecodeQueueDepth(n);
      assert.equal(cfg.decodeQueueDepth, n);
    }
  });

  it('accepts null and returns to the auto-size sentinel', () => {
    const cfg = Config.production();
    cfg.setDecodeQueueDepth(1024);
    assert.equal(cfg.decodeQueueDepth, 1024);
    cfg.setDecodeQueueDepth(null);
    assert.equal(cfg.decodeQueueDepth, null);
  });

  it('accepts undefined and returns to the auto-size sentinel', () => {
    const cfg = Config.production();
    cfg.setDecodeQueueDepth(1024);
    assert.equal(cfg.decodeQueueDepth, 1024);
    cfg.setDecodeQueueDepth(undefined);
    assert.equal(cfg.decodeQueueDepth, null);
  });

  it('accepts an explicit 0 (queue clamps to 1 internally)', () => {
    const cfg = Config.production();
    cfg.setDecodeQueueDepth(0);
    assert.equal(cfg.decodeQueueDepth, 0);
  });

  it('retains a large value verbatim', () => {
    const cfg = Config.production();
    cfg.setDecodeQueueDepth(65536);
    assert.equal(cfg.decodeQueueDepth, 65536);
  });
});

describe('Config two-stage pipeline setters are independent', () => {
  it('do not interfere with each other', () => {
    const cfg = Config.production();
    cfg.setDecodeThreads(16);
    cfg.setDecodeQueueDepth(4096);
    assert.equal(cfg.decodeThreads, 16);
    assert.equal(cfg.decodeQueueDepth, 4096);
  });

  it('do not interfere with the legacy pool-sizing knobs', () => {
    const cfg = Config.production();
    cfg.setConcurrentRequests(8);
    cfg.setDecoderThreads(4);
    cfg.setDecoderRingSize(1024);
    cfg.setDecodeThreads(16);
    cfg.setDecodeQueueDepth(4096);
    assert.equal(cfg.concurrentRequests, 8);
    assert.equal(cfg.decoderThreads, 4);
    assert.equal(cfg.decoderRingSize, 1024);
    assert.equal(cfg.decodeThreads, 16);
    assert.equal(cfg.decodeQueueDepth, 4096);
  });
});
