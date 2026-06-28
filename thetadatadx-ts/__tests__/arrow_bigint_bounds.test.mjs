// Arrow-IPC reconstruct rejects a `bigint` `volume` / `count` outside the
// `i64` destination instead of silently truncating its wrapped low bits.
//
// The `<tick>ToArrowIpc(rows)` terminal copies the JS object back into the
// columnar `tick::T`, where `volume` / `count` are `i64`. A JS `bigint`
// outside `i64::MIN..=i64::MAX` (e.g. `2n ** 100n`) cannot be represented and
// must raise `InvalidParameterError`, matching the config setters' rejection
// of an out-of-range `bigint` rather than passing a wrapped value. Imports the
// package entry (`streaming-session.js`) so the `[InvalidParameterError]`
// prefix the native binding raises reaches the typed subclass the same way a
// caller sees it.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const mod = await import('../streaming-session.js').then((m) => m.default ?? m);
const { InvalidParameterError } = mod;

const I64_MAX = 2n ** 63n - 1n;
const I64_MIN = -(2n ** 63n);
const OVER_I64 = 2n ** 100n; // far above i64::MAX
const UNDER_I64 = I64_MIN - 1n; // one below i64::MIN

// Minimal valid row per columnar i64-bearing tick type. Only the non-optional
// fields are supplied; `volume` / `count` carry an in-range default that each
// test overrides to probe the bound. Optional contract-identity and
// `*TimestampMs` fields are left undefined (the documented absent shape).
const ROWS = {
  eodTickToArrowIpc: {
    createdMsOfDay: 0, lastTradeMsOfDay: 0, open: 1, high: 2, low: 0.5, close: 1.5,
    volume: 1000n, count: 10n, bidSize: 0, bidExchange: 0, bid: 0, bidCondition: 0,
    askSize: 0, askExchange: 0, ask: 0, askCondition: 0, date: 20260115,
  },
  greeksEodTickToArrowIpc: {
    msOfDay: 0, open: 1, high: 2, low: 0.5, close: 1.5, volume: 1000n, count: 10n,
    bidSize: 0, bidExchange: 0, bid: 0, bidCondition: 0, askSize: 0, askExchange: 0,
    ask: 0, askCondition: 0, delta: 0, theta: 0, vega: 0, rho: 0, epsilon: 0,
    lambda: 0, gamma: 0, vanna: 0, charm: 0, vomma: 0, veta: 0, vera: 0, speed: 0,
    zomma: 0, color: 0, ultima: 0, d1: 0, d2: 0, dualDelta: 0, dualGamma: 0,
    impliedVolatility: 0, ivError: 0, underlyingMsOfDay: 0, underlyingPrice: 0,
    date: 20260115,
  },
  ohlcTickToArrowIpc: {
    msOfDay: 0, open: 1, high: 2, low: 0.5, close: 1.5, volume: 1000n, count: 10n,
    vwap: 0, date: 20260115,
  },
};

function looksLikeArrowIpcStream(buf) {
  return buf.length >= 8 && buf[0] === 0xff && buf[1] === 0xff && buf[2] === 0xff && buf[3] === 0xff;
}

describe('Arrow IPC reconstruct rejects out-of-i64 bigint volume/count', () => {
  for (const [fn, base] of Object.entries(ROWS)) {
    it(`${fn} rejects 2n ** 100n volume with InvalidParameterError`, () => {
      assert.throws(
        () => mod[fn]([{ ...base, volume: OVER_I64 }]),
        (err) => err instanceof InvalidParameterError && /volume/.test(err.message),
        'an over-i64 volume must reject, not truncate',
      );
    });

    it(`${fn} rejects a below-i64::MIN count with InvalidParameterError`, () => {
      assert.throws(
        () => mod[fn]([{ ...base, count: UNDER_I64 }]),
        (err) => err instanceof InvalidParameterError && /count/.test(err.message),
        'a below-i64::MIN count must reject, not truncate',
      );
    });

    it(`${fn} accepts i64-range volume/count`, () => {
      const buf = mod[fn]([{ ...base, volume: I64_MAX, count: I64_MIN }]);
      assert.ok(looksLikeArrowIpcStream(buf), 'an i64-range row must serialise to an Arrow IPC stream');
    });
  }
});
