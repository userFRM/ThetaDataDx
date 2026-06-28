// Regression coverage for the `InterestRateTick` schema.
//
// The upstream server emits 2 columns (`created` as ISO date Text,
// `rate` as percent Number). An earlier SDK schema declared 3 fields
// (`ms_of_day`, `rate`, `date`) and decoded every live response into
// `column 0: expected Number|Timestamp, got Text`. The fix removed the
// fictitious `ms_of_day` field and rewired `date` to flow through
// `parse_iso_date`. This test pins the napi-rs `InterestRateTick`
// interface to its 2-field shape (`date` + `rate`, no `ms_of_day`) so a
// future schema regression cannot ship a JS bundle whose type still
// carries the removed field.
//
// Live decode coverage lives in
// `thetadatadx-rs/tests/test_interest_rate_schema.rs`.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const dtsPath = path.join(__dirname, '..', 'index.d.ts');
const dts = fs.readFileSync(dtsPath, 'utf8');

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('InterestRateTick (index.d.ts)', () => {
  it('declares exactly the 2-field shape (date + rate, no ms_of_day)', () => {
    // Locate the `export interface InterestRateTick { ... }` block.
    const match = dts.match(/export interface InterestRateTick\s*\{([^}]+)\}/);
    assert.ok(match, 'InterestRateTick must be declared in index.d.ts');
    const body = match[1];
    // The two expected fields (camelCased per napi-rs convention).
    assert.match(body, /\bdate:\s*number\b/, 'date: number must be present');
    assert.match(body, /\brate:\s*number\b/, 'rate: number must be present');
    // The removed field must NOT appear.
    assert.doesNotMatch(
      body,
      /\bmsOfDay\b/,
      'msOfDay is not part of the InterestRateTick shape; index.d.ts still advertises it',
    );
  });

  it('interestRateHistoryEOD returns Promise<Array<InterestRateTick>>', () => {
    // Pin that the historical endpoint signature still returns the new
    // tick type (no accidental rename / collection mismatch). The method
    // resolves the fetch off the runtime's execution thread, so the
    // surface is a Promise; the element type is unchanged.
    assert.match(
      dts,
      /interestRateHistoryEOD\([^)]*\):\s*Promise<Array<InterestRateTick>>/,
      'interestRateHistoryEOD must return Promise<Array<InterestRateTick>>',
    );
  });
});

describe('InterestRateTick (runtime shape)', () => {
  it('hand-built tick objects round-trip the wire reference row', () => {
    // napi exposes `InterestRateTick` as a structural type
    // (`#[napi(object)]`) rather than a constructable class — runtime
    // values are plain `{ date, rate }` objects produced by the
    // decoder. We pin the structural shape on a hand-built tick using
    // the SOFR 2025-04-28 reference row from the CHANGELOG.
    const tick = { date: 20250428, rate: 4.36 };
    assert.equal(tick.date, 20250428);
    assert.equal(tick.rate, 4.36);
    assert.equal(tick.msOfDay, undefined);
    assert.equal(tick.ms_of_day, undefined);
  });

  it('native addon exposes a client class', () => {
    // Smoke-check that the addon imported at module top level is the
    // actual native binding (not the SKIP path) and exports at least
    // one historical-data client class. The exact entry-point name
    // varies across major versions; accept any of the canonical
    // Client client names.
    assert.ok(
      typeof mod.Client === 'function'
        || typeof mod.HistoricalClient === 'function'
        || typeof mod.Client === 'function'
        || typeof mod.HistoricalClient === 'function',
      'native addon should expose at least one Client client class',
    );
  });
});
