// FlatFilesConfig setters on `Config` — TypeScript binding parity
// with Python / C++ / FFI. Pins the JS surface contract for
// `setFlatfilesMaxAttempts`, `setFlatfilesInitialBackoffSecs`,
// `setFlatfilesMaxBackoffSecs`, `setFlatfilesConnectTimeoutSecs`, and
// `setFlatfilesReadTimeoutSecs`.
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
  it('expose the FlatFilesConfig field defaults', () => {
    const cfg = Config.production();
    assert.equal(cfg.flatfilesMaxAttempts, 10);
    assert.equal(cfg.flatfilesInitialBackoffSecs, 1n);
    assert.equal(cfg.flatfilesMaxBackoffSecs, 30n);
    assert.equal(cfg.flatfilesConnectTimeoutSecs, 10n);
    assert.equal(cfg.flatfilesReadTimeoutSecs, 60n);
  });
});

describe('Config.setFlatfilesMaxAttempts', () => {
  it('round-trips through the setter across the documented u32 range', () => {
    const cfg = Config.production();
    // `1` disables retry; the documented valid range is `[1, 100]`, so the
    // setter floors at 1 (validated at the napi boundary). `0` is rejected
    // below rather than round-tripped.
    for (const n of [1, 3, 5, 10, 100, 1_000]) {
      cfg.setFlatfilesMaxAttempts(n);
      assert.equal(cfg.flatfilesMaxAttempts, n);
    }
  });

  it('rejects 0 and other hostile inputs at the napi boundary', () => {
    const cfg = Config.production();
    for (const bad of [0, -1, 1.5, 2 ** 32, Number.NaN]) {
      assert.throws(
        () => cfg.setFlatfilesMaxAttempts(bad),
        /flatfilesMaxAttempts/,
        `setFlatfilesMaxAttempts(${bad}) must reject, not silently rewrite via ToUint32`,
      );
    }
  });
});

describe('Config.setFlatfilesInitialBackoffSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 2n, 4n, 10n, 60n, 3_600n, 86_400n]) {
      cfg.setFlatfilesInitialBackoffSecs(secs);
      assert.equal(cfg.flatfilesInitialBackoffSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatfilesInitialBackoffSecs(1n << 65n),
      /setFlatfilesInitialBackoffSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Config.setFlatfilesMaxBackoffSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 4n, 10n, 60n, 3_600n, 86_400n]) {
      cfg.setFlatfilesMaxBackoffSecs(secs);
      assert.equal(cfg.flatfilesMaxBackoffSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatfilesMaxBackoffSecs(1n << 65n),
      /setFlatfilesMaxBackoffSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Config.setFlatfilesConnectTimeoutSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 4n, 10n, 60n, 3_600n]) {
      cfg.setFlatfilesConnectTimeoutSecs(secs);
      assert.equal(cfg.flatfilesConnectTimeoutSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatfilesConnectTimeoutSecs(1n << 65n),
      /setFlatfilesConnectTimeoutSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('Config.setFlatfilesReadTimeoutSecs', () => {
  it('round-trips through the setter across the documented u64 range', () => {
    const cfg = Config.production();
    for (const secs of [0n, 1n, 4n, 10n, 60n, 3_600n, 86_400n]) {
      cfg.setFlatfilesReadTimeoutSecs(secs);
      assert.equal(cfg.flatfilesReadTimeoutSecs, secs);
    }
  });

  it('rejects BigInt magnitudes above u64::MAX', () => {
    const cfg = Config.production();
    assert.throws(
      () => cfg.setFlatfilesReadTimeoutSecs(1n << 65n),
      /setFlatfilesReadTimeoutSecs/,
      'magnitude above u64::MAX must be rejected at the boundary',
    );
  });
});

describe('FlatFiles setter state survives interleaved historical tuning calls', () => {
  it('FlatFiles setter mutations land independently of historical tuning mutations', () => {
    const cfg = Config.production();
    cfg.setFlatfilesMaxAttempts(7);
    cfg.setFlatfilesInitialBackoffSecs(3n);
    cfg.setFlatfilesMaxBackoffSecs(12n);
    cfg.setFlatfilesConnectTimeoutSecs(20n);
    cfg.setFlatfilesReadTimeoutSecs(45n);
    cfg.setWarnOnBufferedThresholdBytes(8n * 1024n * 1024n);
    assert.equal(cfg.flatfilesMaxAttempts, 7);
    assert.equal(cfg.flatfilesInitialBackoffSecs, 3n);
    assert.equal(cfg.flatfilesMaxBackoffSecs, 12n);
    assert.equal(cfg.flatfilesConnectTimeoutSecs, 20n);
    assert.equal(cfg.flatfilesReadTimeoutSecs, 45n);
    assert.equal(cfg.warnOnBufferedThresholdBytes, 8n * 1024n * 1024n);
  });
});
