// Decode-fed projected Arrow-IPC terminal (issue #1089).
//
// The full-schema `<tick>ToArrowIpc(rows)` serialises every column a hand-built
// row vector could carry. The decode-fed pair mirrors Python's projected
// `<TickName>List.to_arrow()`: resolve a response's wire headers to the columns
// it carried (`<tick>PresentColumns(headers)`), then serialise ONLY those
// columns (`<tick>ToArrowIpcProjected(rows, presentColumns, symbol?)`).
//
// A `stock_history_trade` response omits the four trade-flag columns and the
// contract-identity trio; the projected stream must omit them too while the
// full terminal keeps them. Offline: builds rows in-process and decodes the IPC
// with the bundled `apache-arrow` dev dependency.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { tableFromIPC } from 'apache-arrow';

const __dirname = dirname(fileURLToPath(import.meta.url));
const dts = readFileSync(resolve(__dirname, '..', 'index.d.ts'), 'utf8');
const require = createRequire(import.meta.url);
const addon = require('../index.js');

// The columns a `stock_history_trade` wire response carries: the ten trade
// execution columns plus `date`. No flag columns, no contract-id trio.
const STOCK_TRADE_HEADERS = [
  'ms_of_day',
  'sequence',
  'ext_condition1',
  'ext_condition2',
  'ext_condition3',
  'ext_condition4',
  'condition',
  'size',
  'exchange',
  'price',
  'date',
];

// Two rows; the flag / contract-id fields carry non-seed values so a projection
// bug (emitting them) would surface as real columns, not all-zero ones.
function sampleRows() {
  const base = {
    msOfDay: 34200000,
    sequence: 1,
    extCondition1: 0,
    extCondition2: 0,
    extCondition3: 0,
    extCondition4: 0,
    condition: 0,
    size: 100,
    exchange: 5,
    price: 12.5,
    conditionFlags: 7,
    priceFlags: 3,
    volumeType: 1,
    recordsBack: 2,
    date: 20260115,
    expiration: 20260117,
    strike: 100.0,
    right: 'C',
    // Flag accessors are output-only on the tick object but required on the
    // napi object shape; the reconstruct ignores them.
    isCancelled: false,
    tradeConditionNoLast: false,
    priceConditionSetLast: false,
    isIncrementalVolume: false,
    regularTradingHours: true,
    isSeller: false,
  };
  return [base, { ...base, sequence: 2, size: 200, price: 12.75, recordsBack: 1 }];
}

function ipcColumns(buf) {
  return tableFromIPC(buf).schema.fields.map((f) => f.name);
}

describe('projected Arrow-IPC terminal', () => {
  it('resolves wire headers to the columns the response carried', () => {
    const present = addon.tradeTickPresentColumns(STOCK_TRADE_HEADERS);
    assert.deepEqual(present, STOCK_TRADE_HEADERS);
  });

  it('projected export omits the flag and contract-id columns', () => {
    const present = addon.tradeTickPresentColumns(STOCK_TRADE_HEADERS);
    const buf = addon.tradeTickToArrowIpcProjected(sampleRows(), present, null);
    const cols = ipcColumns(buf);
    assert.deepEqual(cols, STOCK_TRADE_HEADERS, 'projected frame must carry only the wire columns');
    for (const absent of [
      'condition_flags',
      'price_flags',
      'volume_type',
      'records_back',
      'expiration',
      'strike',
      'right',
    ]) {
      assert.ok(!cols.includes(absent), `projected export leaked wire-absent column ${absent}`);
    }
  });

  it('broadcasts symbol as the leading projected column', () => {
    const optionHeaders = [
      'symbol',
      'expiration',
      'strike',
      'right',
      'ms_of_day',
      'sequence',
      'condition',
      'size',
      'exchange',
      'price',
    ];
    const present = addon.tradeTickPresentColumns(optionHeaders);
    const buf = addon.tradeTickToArrowIpcProjected(sampleRows(), present, 'SPY');
    const cols = ipcColumns(buf);
    assert.equal(cols[0], 'symbol', 'symbol must be the leading projected column');
    assert.ok(cols.includes('expiration'), 'option projection keeps the contract-id trio');
  });

  it('emits a per-row symbol column for a multi-symbol snapshot (#1100)', () => {
    // A multi-symbol snapshot's wire carries a per-row-varying `symbol`; the
    // `symbols` param (one value per row) drives a real per-row leading symbol
    // column so each row is attributable, instead of a single broadcast value.
    const rows = sampleRows();
    const perRow = ['AAPL', 'MSFT'];
    assert.equal(perRow.length, rows.length, 'one symbol per row');
    const present = addon.tradeTickPresentColumns(STOCK_TRADE_HEADERS);
    const buf = addon.tradeTickToArrowIpcProjected(rows, present, null, perRow);
    const table = tableFromIPC(buf);
    const cols = table.schema.fields.map((f) => f.name);
    assert.equal(cols[0], 'symbol', 'per-row symbol must be the leading projected column');
    const symCol = table.getChild('symbol');
    const got = rows.map((_, i) => symCol.get(i));
    assert.deepEqual(got, perRow, 'each row must carry its own symbol, not a broadcast');
  });

  it('an empty presence projects to zero columns without throwing', () => {
    // A response whose headers resolve to no schema column yields an empty
    // presence; the projected export must still produce a valid zero-column
    // stream rather than error. (The Rust builder pins the row count via
    // `with_row_count`; the apache-arrow JS reader reports 0 rows for a
    // zero-column batch, so the row-count invariant is asserted in the Rust
    // FFI projected test, not here.)
    const buf = addon.tradeTickToArrowIpcProjected(sampleRows(), [], null);
    const table = tableFromIPC(buf);
    assert.equal(table.schema.fields.length, 0, 'empty presence must project to zero columns');
  });

  it('the full terminal keeps every column for the same rows', () => {
    const cols = ipcColumns(addon.tradeTickToArrowIpc(sampleRows()));
    for (const kept of ['condition_flags', 'price_flags', 'volume_type', 'records_back', 'expiration', 'strike', 'right']) {
      assert.ok(cols.includes(kept), `full terminal dropped ${kept}`);
    }
  });

  it('declares the projected pair for several tick types in index.d.ts', () => {
    for (const tick of ['tradeTick', 'quoteTick', 'ohlcTick', 'eodTick']) {
      assert.match(
        dts,
        new RegExp(`export declare function ${tick}PresentColumns\\(headers: Array<string>\\): Array<string>`),
        `index.d.ts must declare ${tick}PresentColumns`
      );
      assert.match(
        dts,
        new RegExp(`export declare function ${tick}ToArrowIpcProjected\\(rows: Array<`),
        `index.d.ts must declare ${tick}ToArrowIpcProjected`
      );
    }
  });
});

// The `<method>WithColumns` live-call variant (#1098 TypeScript parity). The
// plain `<method>` returns `Array<Tick>` with no presence, so a live caller
// could only reach the full-schema `<tick>ToArrowIpc`. The variant returns
// `{ rows, presentColumns, symbol? }`, so the response's own column set drives
// `<tick>ToArrowIpcProjected` — the projected exit reachable from a live call.
//
// The prebuilt native addon in the tree predates this method, so the live call
// itself is exercised against the committed generated napi source (the shape
// napi lowers into index.d.ts, verified by the drift gate). The end-to-end
// drivability of the RETURN SHAPE is exercised for real: the object a live call
// yields is assembled and its `presentColumns` / `symbol` feed the real
// projected serialiser in the addon.
describe('withColumns live-call variant drives the projected export', () => {
  const generated = readFileSync(
    resolve(__dirname, '..', 'src', '_generated', 'historical_methods.rs'),
    'utf8'
  );

  // Emulate what `stockHistoryTradeWithColumns` returns for a stock response:
  // the converted rows plus the response's present columns (resolved from the
  // wire headers exactly as the method does from the core ColumnPresence) and
  // the broadcast symbol (absent on a stock response).
  function withColumnsReturn(headers, symbol) {
    return {
      rows: sampleRows(),
      presentColumns: addon.tradeTickPresentColumns(headers),
      symbol: symbol ?? undefined,
    };
  }

  it('projects a stock response to only its wire columns from the return object', () => {
    const page = withColumnsReturn(STOCK_TRADE_HEADERS, null);
    const buf = addon.tradeTickToArrowIpcProjected(page.rows, page.presentColumns, page.symbol ?? null);
    const cols = ipcColumns(buf);
    assert.deepEqual(cols, STOCK_TRADE_HEADERS, 'the return object must drive a terminal-exact frame');
    for (const absent of ['condition_flags', 'price_flags', 'volume_type', 'records_back', 'expiration', 'strike', 'right']) {
      assert.ok(!cols.includes(absent), `projected export leaked wire-absent column ${absent}`);
    }
  });

  it('broadcasts the return object symbol as the leading column', () => {
    const optionHeaders = ['expiration', 'strike', 'right', 'ms_of_day', 'sequence', 'condition', 'size', 'exchange', 'price'];
    const page = withColumnsReturn(optionHeaders, 'SPY');
    const buf = addon.tradeTickToArrowIpcProjected(page.rows, page.presentColumns, page.symbol ?? null);
    const cols = ipcColumns(buf);
    assert.equal(cols[0], 'symbol', 'the return object symbol must lead the projected frame');
  });

  it('generates the WithColumns variant on both historical classes', () => {
    // Present on the standalone HistoricalClient and the unified HistoricalView
    // (both impl blocks), returning the presence-carrying object, never the bare
    // row array.
    const jsName = (generated.match(/js_name = "(\w+WithColumns)"/g) || []).length;
    assert.ok(jsName >= 2, 'stockHistoryTradeWithColumns must exist on both historical impl blocks');
    assert.match(generated, /pub async fn stock_history_trade_with_columns\(/);
    assert.match(generated, /-> napi::Result<TradeTickWithColumns>/);
    assert.match(generated, /pub struct TradeTickWithColumns \{/);
    assert.match(generated, /pub present_columns: Vec<String>,/);
    // The plain buffered method is untouched (no breaking change).
    assert.match(generated, /pub async fn stock_history_trade\(\s*&self,[\s\S]*?-> napi::Result<Vec<TradeTick>>/);
  });
});
