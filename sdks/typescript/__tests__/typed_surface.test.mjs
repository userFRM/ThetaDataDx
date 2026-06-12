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

describe('strike is dollars everywhere', () => {
  it('fluent builder accepts number | string and reads dollars back', () => {
    for (const strike of [550, '550']) {
      const option = addon.ContractRef.option('SPY', '20260618', strike, 'C');
      assert.equal(option.strike, 550);
    }
    const cents = addon.ContractRef.option('SPX', '20260618', '5400.50', 'P');
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

describe('subscription getters', () => {
  it('per-contract subscriptions expose the bound contract; full streams expose secType', () => {
    const option = addon.ContractRef.option('SPY', '20260618', 550, 'C');
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
