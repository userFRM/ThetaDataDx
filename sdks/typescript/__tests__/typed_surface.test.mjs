// Typed-surface contract tests (structural, via index.d.ts + the
// compiled addon). Pins the cross-binding type semantics on the
// TypeScript surface:
//
// * Endpoint methods take required parameters positionally plus ONE
//   optional trailing options object (`<Method>Options`), never
//   positional optional holes.
// * `strike` is dollars everywhere: the fluent builder accepts
//   `number | string` and reads dollars back; the streaming `Contract`
//   payload and historical rows type it `number`.
// * `EodTick` carries `createdMsOfDay` / `lastTradeMsOfDay` (the
//   vendor's v3 semantics) instead of an enumerated `msOfDay2`.
// * `CalendarDay.isOpen` is boolean and `status` is the vendor
//   vocabulary string.
// * Absent contract identity is `undefined`/`null` (optional fields),
//   matching the streaming payload convention.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { createRequire } from 'node:module';

const __dirname = dirname(fileURLToPath(import.meta.url));
const dts = readFileSync(resolve(__dirname, '..', 'index.d.ts'), 'utf8');
const require = createRequire(import.meta.url);
const addon = require('../index.js');

describe('endpoint options objects', () => {
  it('stockHistoryEod takes required params plus one trailing options object', () => {
    assert.match(
      dts,
      /stockHistoryEOD\(symbol: string, startDate: string \| Date, endDate: string \| Date, options\?: StockHistoryEodOptions/,
      'stockHistoryEOD must end in a single optional options object'
    );
  });

  it('options interfaces carry camelCase keys plus timeoutMs', () => {
    const block = dts.match(/export interface StockHistoryTradeQuoteOptions\s*\{[^}]*\}/s);
    assert.ok(block, 'StockHistoryTradeQuoteOptions missing from index.d.ts');
    assert.match(block[0], /timeoutMs\?\s*:\s*number/);
  });

  it('no endpoint method declares a positional timeoutMs parameter', () => {
    assert.ok(
      !/\(\s*[^)]*timeoutMs\?: number[^)]*\): (Array|Promise)/.test(dts),
      'timeoutMs must ride inside the options object, not positionally'
    );
  });
});

describe('historical methods resolve off the execution thread', () => {
  // The 61 data-fetch methods declared on ThetaDataDxClient. Each runs
  // the network round-trip on a worker and resolves a Promise, so a
  // fetch never holds the Node event loop. Element types are unchanged
  // — only the surrounding shape becomes a Promise.
  //
  // Pulled from the single client interface block so streaming lifecycle
  // declarations elsewhere in the file (awaitDrain etc.) cannot dilute
  // the assertion.
  const clientBlock = dts.match(
    /export declare class ThetaDataDxClient \{[\s\S]*?\n\}/
  );
  assert.ok(clientBlock, 'ThetaDataDxClient class missing from index.d.ts');
  const body = clientBlock[0];

  // Every endpoint method names a return type; collect each declared
  // `methodName(...): <ret>` whose name matches the data-fetch families.
  const familyRe =
    /^\s+((?:stock|option|index)History\w*|\w*Snapshot\w*|\w*AtTime\w*|\w*List\w*|calendar\w*|interestRate\w*)\(/;
  const methodLines = body
    .split('\n')
    .filter((line) => familyRe.test(line));

  it('every data-fetch method is present (61 of them)', () => {
    // Pin the count so a generator change that drops a method, or leaks
    // a streaming lifecycle method into the data-fetch families, is
    // caught here rather than silently shrinking the async surface.
    assert.equal(
      methodLines.length,
      61,
      `expected 61 data-fetch methods, found ${methodLines.length}`
    );
  });

  it('every data-fetch method returns a Promise', () => {
    for (const line of methodLines) {
      assert.match(
        line,
        /\):\s*Promise<Array<[^>]+>>/,
        `data-fetch method must return Promise<Array<...>>: ${line.trim()}`
      );
    }
  });

  it('no data-fetch method returns a bare Array (would block the event loop)', () => {
    for (const line of methodLines) {
      assert.doesNotMatch(
        line,
        /\):\s*Array</,
        `data-fetch method must not return a bare Array: ${line.trim()}`
      );
    }
  });

  it('the streaming awaitDrain lifecycle method is left async and untouched', () => {
    assert.match(
      body,
      /awaitDrain\(timeoutMs: number\): Promise<boolean>/,
      'awaitDrain must stay Promise<boolean> (streaming lifecycle, not a data fetch)'
    );
  });
});

describe('strike is dollars everywhere', () => {
  it('fluent builder accepts number | string and reads dollars back', () => {
    for (const strike of [550, '550']) {
      const option = addon.ContractRef.option('SPY', { expiration: '20260618', strike: strike, right: 'C' });
      assert.equal(option.strike, 550);
    }
    const cents = addon.ContractRef.option('SPX', { expiration: '20260618', strike: '5400.50', right: 'P' });
    assert.equal(cents.strike, 5400.5);
    assert.equal(cents.expiration, 20260618);
    assert.equal(cents.right, 'P');
  });

  it('streaming Contract payload types strike as a number with no wire-integer twin', () => {
    const block = dts.match(/export interface Contract\s*\{[^}]*\}/s);
    assert.ok(block, 'Contract interface missing from index.d.ts');
    assert.match(block[0], /strike\?\s*:\s*number/);
    assert.ok(!/strikeDollars/.test(block[0]), 'strikeDollars must collapse into strike');
  });

  it('stock contracts carry no option identity', () => {
    const stock = addon.ContractRef.stock('AAPL');
    assert.equal(stock.strike, null);
    assert.equal(stock.expiration, null);
    assert.equal(stock.right, null);
  });
});

describe('EOD and calendar row shapes', () => {
  it('EodTick exposes createdMsOfDay and lastTradeMsOfDay', () => {
    const block = dts.match(/export interface EodTick\s*\{[^}]*\}/s);
    assert.ok(block, 'EodTick interface missing from index.d.ts');
    assert.match(block[0], /createdMsOfDay\s*:\s*number/);
    assert.match(block[0], /lastTradeMsOfDay\s*:\s*number/);
    assert.ok(!/msOfDay2/.test(block[0]), 'msOfDay2 must not survive the rename');
  });

  it('CalendarDay carries boolean isOpen and the vendor status vocabulary', () => {
    const block = dts.match(/export interface CalendarDay\s*\{[^}]*\}/s);
    assert.ok(block, 'CalendarDay interface missing from index.d.ts');
    assert.match(block[0], /isOpen\s*:\s*boolean/);
    assert.match(block[0], /status\s*:\s*string/);
  });

  it('contract identity on rows is optional (absent = undefined)', () => {
    const block = dts.match(/export interface TradeTick\s*\{[^}]*\}/s);
    assert.ok(block, 'TradeTick interface missing from index.d.ts');
    assert.match(block[0], /expiration\?\s*:\s*number/);
    assert.match(block[0], /strike\?\s*:\s*number/);
    assert.match(block[0], /right\?\s*:\s*string/);
  });
});

describe('epoch-millisecond tick accessors (cross-binding parity)', () => {
  // Every tick row carrying a `date` column plus one or more
  // milliseconds-of-day columns surfaces a `*TimestampMs` epoch field —
  // the camelCase parity of the Python `*_timestamp_ms` property and the
  // C++ `tdx::timestamp_ms` free function. The value is Unix epoch
  // milliseconds (UTC, DST-aware), computed once at conversion time
  // through the one shared core (`tdbe::time::date_ms_to_epoch_ms`); the
  // raw milliseconds-of-day columns stay primary and the field is
  // optional (`undefined` when `date` is absent). These assertions pin
  // the JS-visible shape — name, `bigint` type, optionality — so a
  // generator change that drops the accessor family, mistypes it, or
  // breaks camelCase parity with Python fails here.

  it('EodTick exposes createdTimestampMs and lastTradeTimestampMs as optional bigint', () => {
    const block = dts.match(/export interface EodTick\s*\{[\s\S]*?\n\}/);
    assert.ok(block, 'EodTick interface missing from index.d.ts');
    assert.match(block[0], /createdTimestampMs\?\s*:\s*bigint/);
    assert.match(block[0], /lastTradeTimestampMs\?\s*:\s*bigint/);
  });

  it('QuoteTick / TradeTick / OhlcTick expose timestampMs as optional bigint', () => {
    for (const name of ['QuoteTick', 'TradeTick', 'OhlcTick']) {
      const block = dts.match(new RegExp(`export interface ${name}\\s*\\{[\\s\\S]*?\\n\\}`));
      assert.ok(block, `${name} interface missing from index.d.ts`);
      assert.match(block[0], /timestampMs\?\s*:\s*bigint/, `${name}.timestampMs must be optional bigint`);
    }
  });

  it('Greeks rows expose both timestampMs and underlyingTimestampMs as optional bigint', () => {
    for (const name of ['GreeksAllTick', 'GreeksEodTick', 'IvTick']) {
      const block = dts.match(new RegExp(`export interface ${name}\\s*\\{[\\s\\S]*?\\n\\}`));
      assert.ok(block, `${name} interface missing from index.d.ts`);
      assert.match(block[0], /timestampMs\?\s*:\s*bigint/, `${name}.timestampMs must be optional bigint`);
      assert.match(
        block[0],
        /underlyingTimestampMs\?\s*:\s*bigint/,
        `${name}.underlyingTimestampMs must be optional bigint`
      );
    }
  });

  it('TradeQuoteTick exposes the prefixed quoteTimestampMs alongside timestampMs', () => {
    const block = dts.match(/export interface TradeQuoteTick\s*\{[\s\S]*?\n\}/);
    assert.ok(block, 'TradeQuoteTick interface missing from index.d.ts');
    assert.match(block[0], /timestampMs\?\s*:\s*bigint/);
    assert.match(block[0], /quoteTimestampMs\?\s*:\s*bigint/);
  });

  it('the accessor names are the camelCase parity of the Python property names', () => {
    // Python exposes snake_case `created_timestamp_ms`; the napi object
    // key camelCases it to `createdTimestampMs`. Pin the mapping so a
    // future rename on either side cannot silently break parity.
    const pairs = [
      ['created_timestamp_ms', 'createdTimestampMs'],
      ['last_trade_timestamp_ms', 'lastTradeTimestampMs'],
      ['timestamp_ms', 'timestampMs'],
      ['underlying_timestamp_ms', 'underlyingTimestampMs'],
      ['quote_timestamp_ms', 'quoteTimestampMs'],
    ];
    for (const [snake, camel] of pairs) {
      const expected = snake.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
      assert.equal(camel, expected, `${snake} must camelCase to ${camel}`);
      assert.ok(dts.includes(`${camel}?: bigint`), `${camel} must appear as an optional bigint field`);
    }
  });

  it('the accessor is never a primary column — the raw msOfDay fields stay separate', () => {
    // The epoch field rides alongside, never replaces, the raw
    // milliseconds-of-day integer columns (raw-ms doctrine).
    const block = dts.match(/export interface EodTick\s*\{[\s\S]*?\n\}/);
    assert.ok(block, 'EodTick interface missing from index.d.ts');
    assert.match(block[0], /createdMsOfDay\s*:\s*number/);
    assert.match(block[0], /lastTradeMsOfDay\s*:\s*number/);
  });

  it('every epoch field is computed through the one shared DST-aware core, not reimplemented', () => {
    // The generated factory resolves each accessor by calling the same
    // `tdbe::time::date_ms_to_epoch_ms(date, ms_of_day)` the Python
    // property and the `tdx_timestamp_ms` FFI route through — the single
    // DST-aware implementation. No binding reimplements the Eastern-Time
    // offset rules, so the epoch value is identical across a DST boundary
    // (verified at the core: `date_ms_to_epoch_ms_round_trips_edt_and_est`
    // pins 2026-01-15 09:30 EST -> 1768487400000 and the FFI/Python
    // bindings return the same value for the same inputs).
    const factorySrc = readFileSync(
      resolve(__dirname, '..', 'src', '_generated', 'tick_classes.rs'),
      'utf8'
    );
    const assignments = factorySrc.match(/\w*timestamp_ms:\s*tdbe::time::date_ms_to_epoch_ms\([^)]*\)\.map\(BigInt::from\)/g);
    assert.ok(assignments, 'expected generated timestamp_ms factory assignments');
    // 1 created + 1 last_trade + 19 timestamp_ms + 11 underlying + 1 quote
    // = 33 epoch-millisecond fields, one per (date, *_ms_of_day) pair.
    assert.equal(assignments.length, 33, `expected 33 epoch-ms factory assignments, found ${assignments.length}`);
  });
});

describe('subscription getters', () => {
  it('per-contract subscriptions expose the bound contract; full streams expose secType', () => {
    const option = addon.ContractRef.option('SPY', { expiration: '20260618', strike: 550, right: 'C' });
    const sub = option.quote();
    assert.equal(sub.isFull, false);
    assert.equal(sub.contract.symbol, 'SPY');
    assert.equal(sub.contract.strike, 550);
    assert.equal(sub.secType, null);

    const full = addon.SecType.option().fullTrades();
    assert.equal(full.isFull, true);
    assert.equal(full.contract, null);
    assert.equal(full.secType.name, 'OPTION');
  });
});
