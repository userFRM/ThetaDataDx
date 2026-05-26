// FlatFilesConfig setters on `Config` — TypeScript binding parity
// with Python / C++ / FFI. Pins the JS surface contract for
// `setFlatFilesMaxAttempts`, `setFlatFilesInitialBackoffSecs`, and
// `setFlatFilesMaxBackoffSecs`.
//
// The Rust core enforces the `[1, 10]` range on `max_attempts` and
// the `max_backoff >= initial_backoff` invariant at
// `DirectConfig::validate` time, not at the napi setter; this file
// pins only that the JS surface forwards the inputs without dropping
// them and rejects malformed BigInts at the boundary.
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

describe('Config.flatFiles* — defaults mirror FlatFilesConfig::production_defaults', () => {
  it('expose the three FlatFilesConfig field defaults', () => {
    const cfg = Config.production();
    assert.equal(cfg.flatFilesMaxAttempts, 3);
    assert.equal(cfg.flatFilesInitialBackoffSecs, 1n);
    assert.equal(cfg.flatFilesMaxBackoffSecs, 4n);
  });
});

describe('Config.setFlatFilesMaxAttempts', () => {
  it('round-trips through the setter across the documented u32 range', () => {
    const cfg = Config.production();
    for (const n of [0, 1, 3, 5, 10, 100, 1_000]) {
      cfg.setFlatFilesMaxAttempts(n);
      assert.equal(cfg.flatFilesMaxAttempts, n);
    }
  });
});

describe('Config.setFlatFilesInitialBackoffSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 2n, 4n, 10n, 60n, 3_600n, 86_400n]) {
      cfg.setFlatFilesInitialBackoffSecs(secs);
      assert.equal(cfg.flatFilesInitialBackoffSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatFilesInitialBackoffSecs(1n << 65n),
      /setFlatFilesInitialBackoffSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Config.setFlatFilesMaxBackoffSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 4n, 10n, 60n, 3_600n, 86_400n]) {
      cfg.setFlatFilesMaxBackoffSecs(secs);
      assert.equal(cfg.flatFilesMaxBackoffSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatFilesMaxBackoffSecs(1n << 65n),
      /setFlatFilesMaxBackoffSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('FlatFiles setter state survives interleaved pool-sizing calls', () => {
  it('FlatFiles setter mutations land independently of pool-sizing mutations', () => {
    const cfg = Config.production();
    cfg.setFlatFilesMaxAttempts(7);
    cfg.setFlatFilesInitialBackoffSecs(3n);
    cfg.setFlatFilesMaxBackoffSecs(12n);
    cfg.setConcurrentRequests(4);
    cfg.setDecoderRingSize(512);
    assert.equal(cfg.flatFilesMaxAttempts, 7);
    assert.equal(cfg.flatFilesInitialBackoffSecs, 3n);
    assert.equal(cfg.flatFilesMaxBackoffSecs, 12n);
    assert.equal(cfg.concurrentRequests, 4);
    assert.equal(cfg.decoderRingSize, 512);
  });
});
