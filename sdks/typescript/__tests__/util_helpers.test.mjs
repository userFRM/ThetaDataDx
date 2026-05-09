// Cross-language utility helpers — TypeScript binding smoke test (issue #424).
//
// Verifies the `Util` namespace exposes condition / exchange / sequence
// lookups identical to the Rust core. The reference values mirror the
// Rust unit tests in `crates/tdbe/src/conditions/mod.rs` and
// `crates/tdbe/src/exchange.rs` so cross-language drift fails this test.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

describe('Util cross-language helpers (#424)', () => {
  it('exposes the Util namespace and condition / exchange lookups', async () => {
    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      console.log('SKIP: native addon not built (run `npm run build` first)');
      return;
    }
    assert.ok(mod.Util, 'Util namespace should be exported');

    // Trade conditions — values from crates/tdbe/src/conditions/mod.rs tests.
    assert.equal(mod.Util.conditionName(0), 'REGULAR');
    assert.equal(mod.Util.conditionName(40), 'CANC');
    assert.equal(mod.Util.conditionName(-1), 'UNKNOWN');
    assert.equal(mod.Util.conditionName(9999), 'UNKNOWN');

    // Exchange — values from crates/tdbe/src/exchange.rs tests.
    assert.equal(mod.Util.exchangeName(0), 'Composite');
    assert.equal(mod.Util.exchangeName(3), 'NewYorkStockExchange');
    assert.equal(mod.Util.exchangeSymbol(3), 'NYSE');
    assert.equal(mod.Util.exchangeSymbol(5), 'CBOE');
    assert.equal(mod.Util.exchangeName(-1), 'UNKNOWN');
    assert.equal(mod.Util.exchangeSymbol(9999), 'UNKNOWN');

    // Sequence helpers — bidirectional round-trip across the i32
    // wire range, including the asymmetric `i32::MIN` boundary. The
    // upstream Java terminal encodes trade sequences as i32; the SDK
    // widens to i64 internally, but the meaningful round-trip is the
    // i32 range.
    for (const signed of [-1n, 0n, 1n, 2147483647n, -2147483648n]) {
      const unsigned = mod.Util.sequenceSignedToUnsigned(signed);
      assert.equal(typeof unsigned, 'bigint');
      assert.equal(mod.Util.sequenceUnsignedToSigned(unsigned), signed);
    }
  });

  it('rejects BigInt inputs outside the wire range instead of silent coercion', async () => {
    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      console.log('SKIP: native addon not built');
      return;
    }
    // i32::MAX + 1 — out of wire range.
    assert.throws(
      () => mod.Util.sequenceSignedToUnsigned(2147483648n),
      /i32 wire range/,
    );
    // i32::MIN - 1 — out of wire range.
    assert.throws(
      () => mod.Util.sequenceSignedToUnsigned(-2147483649n),
      /i32 wire range/,
    );
    // Negative BigInt for the unsigned helper — never valid.
    assert.throws(
      () => mod.Util.sequenceUnsignedToSigned(-1n),
      /negative BigInt rejected/,
    );
    // Greater than 2^32 - 1 — out of unsigned wire range.
    assert.throws(
      () => mod.Util.sequenceUnsignedToSigned(4294967296n),
      /wire range/,
    );
  });
});
