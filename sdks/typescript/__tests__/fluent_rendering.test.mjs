// String rendering on the fluent value classes (L.1) and the TradeTick
// flag-word accessor fields (3.5) on the TypeScript surface.
//
//   * `toString()` on `ContractRef` / `Subscription` / `SecType` renders a
//     value instead of the opaque `ContractRef {}` Node prints for a napi
//     class whose getters do not surface on inspection. Mirrors the Python
//     `__repr__` / `__str__` surface.
//   * The TradeTick row carries precomputed boolean flag fields
//     (`isCancelled`, ...) decoded from the integer condition / flag
//     columns, declared on the `TradeTick` interface in `index.d.ts`, so a
//     caller never hand-decodes `conditionFlags` / `priceFlags`.

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
const { Contract, SecType } = addon;

describe('toString on the fluent value classes', () => {
  it('ContractRef renders symbol and option identity', () => {
    assert.equal(Contract.stock('AAPL').toString(), 'AAPL STOCK');
    assert.equal(
      Contract.option('SPY', { expiration: '20260620', strike: '550', right: 'C' }).toString(),
      'SPY OPTION 20260620 C 550000'
    );
  });

  it('SecType renders its symbolic name', () => {
    assert.equal(SecType.option().toString(), 'OPTION');
    assert.equal(SecType.stock().toString(), 'STOCK');
  });

  it('Subscription renders scope, kind, and contract', () => {
    const perContract = Contract.option('SPY', { expiration: '20260620', strike: '550', right: 'C' }).trade();
    assert.equal(perContract.toString(), 'Subscription(Trade, SPY OPTION 20260620 C 550000)');
    const marketValue = Contract.stock('AAPL').marketValue();
    assert.equal(marketValue.toString(), 'Subscription(MarketValue, AAPL STOCK)');
    const full = SecType.option().fullOpenInterest();
    assert.equal(full.toString(), 'Subscription(full OpenInterest, Option)');
  });

  it('the classes declare toString in index.d.ts', () => {
    // Slice each class body up to its `toString` declaration (a `[^}]*`
    // whole-body match would truncate at the `{}` inside a doc comment).
    for (const cls of ['ContractRef', 'Subscription', 'SecType']) {
      const start = dts.indexOf(`export declare class ${cls} {`);
      assert.notEqual(start, -1, `class ${cls} not found in index.d.ts`);
      const next = dts.indexOf('export declare class ', start + 1);
      const body = dts.slice(start, next === -1 ? undefined : next);
      assert.match(body, /toString\(\): string/, `${cls} must declare toString(): string`);
    }
  });
});

describe('TradeTick flag-word accessor fields', () => {
  it('declares the decoded boolean fields on the TradeTick interface', () => {
    const start = dts.indexOf('export interface TradeTick {');
    assert.notEqual(start, -1, 'TradeTick interface not found in index.d.ts');
    const end = dts.indexOf('}', start);
    const body = dts.slice(start, end);
    for (const field of [
      'isCancelled',
      'tradeConditionNoLast',
      'priceConditionSetLast',
      'isIncrementalVolume',
      'regularTradingHours',
      'isSeller',
    ]) {
      assert.match(
        body,
        new RegExp(`${field}: boolean`),
        `TradeTick must carry the precomputed boolean field ${field}`
      );
    }
  });
});
