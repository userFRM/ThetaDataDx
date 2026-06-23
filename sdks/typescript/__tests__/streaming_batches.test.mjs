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
//   - `.dropped` and `.schema` proxy to the handle,
//   - the package-installed `StreamView.prototype.batches` patch forwards
//     the caller's single options object straight through to the native
//     method (no positional explosion) and re-wraps the returned native
//     handle in a `RecordBatchStream`. This exercises the real
//     wrapper -> native call shape, so a drift between the options-object
//     surface and the native method signature fails here.
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

  it('StreamView.batches forwards the options object to the native method and re-wraps', async () => {
    // The real wrapper -> native call shape, driven through the SAME
    // forwarder the package installs onto `StreamView.prototype.batches`.
    // A live server is not needed: a stub native `batches` stands in for the
    // napi method, records what it was handed, and returns a fake handle.
    //
    // This is the regression guard for the options-object surface. The
    // native `batches(options?: BatchesOptions)` takes ONE object argument;
    // an earlier positional `batches(batchSize?, lingerMs?, ...)` shape threw
    // a coercion error when called with the documented options object. The
    // synthetic-handle tests below never touch this forwarding seam, so they
    // could not have caught that drift — this test does, by asserting the
    // caller's single options object reaches the native method verbatim
    // (no positional explosion) and the returned handle is re-wrapped in a
    // `RecordBatchStream`.
    const calls = [];
    const handle = fakeHandle(1);
    async function nativeBatches(...args) {
      // `this` is the StreamView receiver the forwarder applies onto.
      calls.push({ thisValue: this, args });
      return handle;
    }
    const forwarder = pkg.wrapStreamViewBatches(nativeBatches);

    const options = { batchSize: 256, lingerMs: 5, backpressure: 'dropOldest', capacity: 8 };
    const receiver = { tag: 'streamView' };
    const reader = await forwarder.call(receiver, options);

    assert.equal(calls.length, 1, 'native batches must be called exactly once');
    assert.equal(calls[0].args.length, 1, 'exactly one argument forwarded (the options object, no positional explosion)');
    assert.strictEqual(calls[0].args[0], options, 'the caller options object is forwarded verbatim');
    assert.strictEqual(calls[0].thisValue, receiver, 'the StreamView receiver is preserved as `this`');
    assert.ok(reader instanceof pkg.RecordBatchStream, 'the native handle is re-wrapped in a RecordBatchStream');
    // The re-wrapped reader drives the very handle the native method returned.
    reader.close();
    assert.equal(handle.state.closed, true, 'closing the reader closes the underlying native handle');
  });

  it('StreamView.batches forwards a no-argument call (options omitted)', async () => {
    // `batches()` with no options must forward zero arguments — the native
    // method defaults every knob. Proves the forwarder does not synthesise a
    // spurious `undefined` positional that an arity-sensitive napi method
    // could reject.
    const calls = [];
    async function nativeBatches(...args) {
      calls.push(args);
      return fakeHandle(0);
    }
    const forwarder = pkg.wrapStreamViewBatches(nativeBatches);
    const reader = await forwarder.call({}, undefined);
    // Calling with an explicit `undefined` still forwards one arg; the
    // documented no-arg form forwards none. Cover the no-arg form here.
    calls.length = 0;
    await forwarder.call({});
    assert.equal(calls.length, 1);
    assert.equal(calls[0].length, 0, 'no options -> no forwarded arguments');
    assert.ok(reader instanceof pkg.RecordBatchStream);
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
    // Drive the disposal path the way `await using reader = ...` lowers it:
    // invoke the well-known `[Symbol.asyncDispose]()` method on scope exit.
    // Spelled explicitly (not via the `await using` declaration) so the test
    // parses on the `engines.node >= 20` floor, where explicit
    // resource management is not yet available and `await using` is a
    // SyntaxError at module load.
    const handle = fakeHandle(5);
    const reader = new pkg.RecordBatchStream(handle);
    assert.equal(typeof reader[Symbol.asyncDispose], 'function');
    await reader[Symbol.asyncDispose]();
    assert.equal(handle.state.closed, true, 'asyncDispose must dispose the reader');
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
