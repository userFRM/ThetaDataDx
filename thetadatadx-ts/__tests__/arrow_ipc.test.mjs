// Arrow-IPC terminal on the history result rows (M.2).
//
// Mirrors the FlatFiles `FlatFileRowList.toArrowIpc()` exit for the typed
// history rows: `<tick>ToArrowIpc(rows)` serialises an `Array<Tick>` to an
// Arrow IPC stream so a TypeScript caller can hand the bytes to
// `apache-arrow` — the same columnar exit Python exposes via
// `<TickName>List.to_arrow()`. Offline: builds tick arrays in-process and
// checks the serialiser returns a well-formed IPC stream (schema header)
// for populated and empty inputs, without needing apache-arrow installed.

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

// Arrow IPC streams open with the 0xFFFFFFFF continuation marker.
function looksLikeArrowIpcStream(buf) {
  return buf.length >= 8 && buf[0] === 0xff && buf[1] === 0xff && buf[2] === 0xff && buf[3] === 0xff;
}

describe('Arrow IPC terminal on history rows', () => {
  it('serialises a populated EodTick array', () => {
    const rows = [
      {
        createdMsOfDay: 0,
        lastTradeMsOfDay: 0,
        open: 1.0,
        high: 2.0,
        low: 0.5,
        close: 1.5,
        volume: 1000n,
        count: 10n,
        bidSize: 0,
        bidExchange: 0,
        bid: 0,
        bidCondition: 0,
        askSize: 0,
        askExchange: 0,
        ask: 0,
        askCondition: 0,
        date: 20260115,
      },
    ];
    const buf = addon.eodTickToArrowIpc(rows);
    assert.ok(looksLikeArrowIpcStream(buf), 'expected an Arrow IPC stream');
  });

  it('an empty array still yields a valid schema-only stream', () => {
    const buf = addon.tradeTickToArrowIpc([]);
    assert.ok(looksLikeArrowIpcStream(buf), 'empty input must produce a valid schema stream, not throw');
  });

  it('declares a ToArrowIpc terminal for several history tick types in index.d.ts', () => {
    for (const fn of [
      'eodTickToArrowIpc',
      'ohlcTickToArrowIpc',
      'tradeTickToArrowIpc',
      'quoteTickToArrowIpc',
      'greeksAllTickToArrowIpc',
      'interestRateTickToArrowIpc',
      'calendarDayToArrowIpc',
    ]) {
      assert.match(
        dts,
        new RegExp(`export declare function ${fn}\\(rows: Array<`),
        `index.d.ts must declare ${fn}`
      );
    }
  });
});
