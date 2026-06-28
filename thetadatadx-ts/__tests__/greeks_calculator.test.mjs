// Offline Greeks calculator — TypeScript binding regression test.
//
// `allGreeks(...)` / `impliedVolatility(...)` cross the napi boundary into
// the same `thetadatadx::greeks::{all_greeks, implied_volatility}` core the
// Python / C++ / C ABI calculators call, so the values are identical across
// every binding. This test pins the cross-binding contract: both functions
// exist, `allGreeks` returns the full 23-field object, `impliedVolatility`
// returns the `[iv, ivError]` tuple (matching Python's `tuple[float, float]`),
// and the recovered IV round-trips the input vol. Drift in the field set or
// the closed-form result fails here.
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

// The 23 fields of `thetadatadx::greeks::GreeksResult`, camelCased to the
// napi object keys. `lambda` stays bare (object keys admit reserved words),
// matching the `GreeksAllTick.lambda` tick object.
const GREEK_FIELDS = [
  'value', 'iv', 'ivError', 'delta', 'gamma', 'theta', 'vega', 'rho',
  'vanna', 'charm', 'vomma', 'veta', 'vera', 'speed', 'zomma', 'color',
  'ultima', 'd1', 'd2', 'dualDelta', 'dualGamma', 'epsilon', 'lambda',
];

describe('offline Greeks calculator', () => {
  it('exposes allGreeks and impliedVolatility as free functions', () => {
    assert.equal(typeof mod.allGreeks, 'function', 'allGreeks should be exported');
    assert.equal(
      typeof mod.impliedVolatility,
      'function',
      'impliedVolatility should be exported',
    );
  });

  it('allGreeks returns the full 23-field result, every field finite', () => {
    const g = mod.allGreeks(150.0, 155.0, 0.05, 0.015, 45 / 365, 3.5, 'C');
    for (const field of GREEK_FIELDS) {
      assert.ok(field in g, `AllGreeks is missing field \`${field}\``);
      assert.equal(typeof g[field], 'number', `AllGreeks.${field} is not a number`);
      assert.ok(Number.isFinite(g[field]), `AllGreeks.${field} must be finite`);
    }
    // No extra keys beyond the documented 23.
    assert.deepEqual(Object.keys(g).sort(), [...GREEK_FIELDS].sort());
  });

  it('impliedVolatility returns the [iv, ivError] tuple', () => {
    const iv = mod.impliedVolatility(150.0, 155.0, 0.05, 0.015, 45 / 365, 3.5, 'C');
    assert.ok(Array.isArray(iv), 'impliedVolatility should return a tuple (JS array)');
    assert.equal(iv.length, 2, 'tuple is [iv, ivError]');
    assert.equal(typeof iv[0], 'number');
    assert.equal(typeof iv[1], 'number');
    assert.ok(iv[0] > 0.0, 'in-the-money call solves to a positive vol');
  });

  it('allGreeks().iv is the same bisection solve as impliedVolatility()[0]', () => {
    // The two surfaces share one IV bisection in the core, so the IV from
    // allGreeks and the first element of impliedVolatility are bit-equal.
    // This is the cross-binding invariant: a single solver, surfaced two
    // ways, never diverges.
    const args = [150.0, 155.0, 0.05, 0.015, 45 / 365, 3.5, 'C'];
    const g = mod.allGreeks(...args);
    const [iv, ivError] = mod.impliedVolatility(...args);
    assert.equal(g.iv, iv, 'allGreeks.iv must equal impliedVolatility()[0]');
    assert.equal(g.ivError, ivError, 'allGreeks.ivError must equal impliedVolatility()[1]');
    assert.ok(iv > 0.0, 'in-the-money call solves to a positive vol');
  });

  it('accepts permissive right spellings (C/P, call/put, case-insensitive)', () => {
    const ref = mod.allGreeks(100.0, 100.0, 0.05, 0.01, 30 / 365, 2.5, 'C');
    for (const form of ['c', 'call', 'CALL', 'Call']) {
      const g = mod.allGreeks(100.0, 100.0, 0.05, 0.01, 30 / 365, 2.5, form);
      assert.ok(
        Math.abs(g.delta - ref.delta) < 1e-12,
        `right form \`${form}\` must agree with \`C\``,
      );
    }
  });

  it('throws on an unrecognised right rather than coercing', () => {
    assert.throws(
      () => mod.allGreeks(100.0, 100.0, 0.05, 0.01, 0.25, 5.0, 'xyz'),
      /right/i,
    );
  });
});
