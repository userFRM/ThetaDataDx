// Pull-based Arrow RecordBatch reader (`client.stream.batches(...)`).
//
// Offline: no live server. These tests prove the JS-side wrapper logic
// without a connection by driving the wrapper with a SYNTHETIC handle that
// returns known Arrow IPC buffers (built with the SDK's own
// `tradeTicksToArrowIpc` export). They verify:
//   - the package exports the `RecordBatchStream` wrapper and the
//     `StreamView.batches` entry is declared,
//   - `for await` yields decoded apache-arrow RecordBatch values,
//   - the iterator stops cleanly at end of stream,
//   - `close()` / `Symbol.asyncDispose` release the handle,
//   - `.dropped` and `.schema` proxy to the handle.
//
// The end-to-end batching / linger / backpressure behaviour is proven
// offline in the Rust core's `fpss::batch_reader` tests.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { createRequire } from 'node:module';
import {
  tableToIPC,
  tableFromArrays,
} from 'apache-arrow';

const __dirname = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const pkg = require('../streaming-session.js');
const dts = readFileSync(resolve(__dirname, '..', 'streaming-session.d.ts'), 'utf8');

// A synthetic single-batch Arrow IPC stream, built with apache-arrow so the
// wrapper decodes a genuine, well-formed stream (the same `tableFromIPC`
// path the wrapper runs). One column, two rows.
function syntheticBatchIpc() {
  const table = tableFromArrays({
    event_type: Int32Array.from([0, 1]),
  });
  return Buffer.from(tableToIPC(table, 'stream'));
}

// A fake `RecordBatchStreamHandle`: hands back `count` IPC batches, then
// null (end of stream). Records whether `close()` was called.
function fakeHandle(count) {
  let remaining = count;
  const state = { closed: false };
  return {
    state,
    async nextIpc() {
      if (state.closed || remaining <= 0) return null;
      remaining -= 1;
      return syntheticBatchIpc();
    },
    schemaIpc() {
      return syntheticBatchIpc();
    },
    get dropped() {
      return 7;
    },
    close() {
      state.closed = true;
    },
  };
}

describe('streaming RecordBatch reader', () => {
  it('exports the RecordBatchStream wrapper', () => {
    assert.equal(typeof pkg.RecordBatchStream, 'function');
  });

  it('declares StreamView.batches in the type surface', () => {
    assert.match(dts, /interface StreamView\b/);
    assert.match(dts, /batches\s*\(\s*options\?\s*:\s*BatchesOptions\s*\)/);
    assert.match(dts, /interface RecordBatchStream\b/);
  });

  it('for await yields decoded apache-arrow RecordBatch values', async () => {
    const reader = new pkg.RecordBatchStream(fakeHandle(3));
    let n = 0;
    for await (const batch of reader) {
      // apache-arrow RecordBatch exposes numRows and a schema.
      assert.equal(typeof batch.numRows, 'number');
      assert.ok(batch.schema);
      n += 1;
    }
    assert.equal(n, 3, 'should yield exactly the produced batches');
  });

  it('stops cleanly at end of stream and closes the handle', async () => {
    const handle = fakeHandle(2);
    const reader = new pkg.RecordBatchStream(handle);
    // eslint-disable-next-line no-unused-vars
    for await (const _batch of reader) {
      // drain
    }
    assert.equal(handle.state.closed, true, 'iterator end must close the handle');
  });

  it('close() releases the handle', () => {
    const handle = fakeHandle(5);
    const reader = new pkg.RecordBatchStream(handle);
    reader.close();
    assert.equal(handle.state.closed, true);
  });

  it('Symbol.asyncDispose releases the handle', async () => {
    const handle = fakeHandle(5);
    {
      // eslint-disable-next-line no-undef
      await using reader = new pkg.RecordBatchStream(handle);
      assert.ok(reader);
    }
    assert.equal(handle.state.closed, true, 'await using must dispose the reader');
  });

  it('.dropped and .schema proxy to the handle', () => {
    const reader = new pkg.RecordBatchStream(fakeHandle(1));
    assert.equal(reader.dropped, 7);
    // schema decodes the schema-only IPC buffer into an apache-arrow Schema.
    assert.ok(reader.schema);
    assert.ok(Array.isArray(reader.schema.fields));
  });

  it('breaking out of for await still closes the handle', async () => {
    const handle = fakeHandle(10);
    const reader = new pkg.RecordBatchStream(handle);
    for await (const batch of reader) {
      assert.ok(batch);
      break; // early exit -> finally must close
    }
    assert.equal(handle.state.closed, true, 'early break must close the handle');
  });
});
