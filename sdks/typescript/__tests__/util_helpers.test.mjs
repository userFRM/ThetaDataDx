// Cross-language utility helpers — TypeScript binding smoke test (issue #424).
//
// Verifies the `Util` namespace exposes condition / exchange / sequence
// lookups identical to the Rust core. The reference values mirror the
// Rust unit tests in `crates/tdbe/src/conditions/mod.rs` and
// `crates/tdbe/src/exchange.rs` so cross-language drift fails this test.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// CI build step is mandatory before `npm test`; fail loud if the addon
// is missing so a broken build does not appear green.
let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('Util cross-language helpers (#424)', () => {
  it('exposes the Util namespace and condition / exchange lookups', () => {
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
    // the JVM terminal encodes trade sequences as i32; the SDK
    // widens to i64 internally, but the meaningful round-trip is the
    // i32 range.
    for (const signed of [-1n, 0n, 1n, 2147483647n, -2147483648n]) {
      const unsigned = mod.Util.sequenceSignedToUnsigned(signed);
      assert.equal(typeof unsigned, 'bigint');
      assert.equal(mod.Util.sequenceUnsignedToSigned(unsigned), signed);
    }
  });

  it('exposes calendarStatusName and timestampMs with the cross-binding sentinels', () => {
    // Calendar status — vocabulary from the C ABI thetadatadx_calendar_status_name
    // / the core CalendarStatus enum. Out-of-table codes return "UNKNOWN".
    assert.equal(mod.Util.calendarStatusName(0), 'open');
    assert.equal(mod.Util.calendarStatusName(1), 'early_close');
    assert.equal(mod.Util.calendarStatusName(2), 'full_close');
    assert.equal(mod.Util.calendarStatusName(3), 'weekend');
    assert.equal(mod.Util.calendarStatusName(99), 'UNKNOWN');
    assert.equal(mod.Util.calendarStatusName(-1), 'UNKNOWN');

    // timestamp_ms combines an Eastern-Time YYYYMMDD date + ms-of-day
    // into epoch ms as a BigInt. 2024-01-02 09:30 ET = 14:30 UTC.
    const epoch = mod.Util.timestampMs(20240102, 34200000);
    assert.equal(typeof epoch, 'bigint');
    assert.equal(epoch, 1704205800000n);

    // Out-of-domain inputs return null (the std::nullopt contract the
    // C++ thetadatadx::timestamp_ms shares), never a coerced sentinel value.
    assert.equal(mod.Util.timestampMs(0, 0), null);
    assert.equal(mod.Util.timestampMs(20240102, -1), null);
  });

  it('rejects BigInt inputs outside the wire range instead of silent coercion', () => {
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
