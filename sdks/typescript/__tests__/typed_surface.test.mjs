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
