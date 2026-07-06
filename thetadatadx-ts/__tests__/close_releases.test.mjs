// `close()` deterministic-release lifecycle tests (#1071 follow-up).
//
// Pins the contract that `close()` RELEASES the core client handle (not just
// stops streaming), so a closed client is unusable: every surface getter
// (`marketData` / `stream` / `flatFiles`) throws a clear "client is closed"
// error, and a second close is a no-op. Also pins that the explicit-resource
// disposers (`Symbol.dispose` / `Symbol.asyncDispose`) route through the real
// `close()` and tolerate an already-closed client.
//
// The disposer-shape tests run offline against crafted receivers; the
// behavioural close-then-use tests are gated on THETADATADX_TEST_CREDS because
// constructing a `Client` needs a live handshake.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// The wrapper patches the disposers onto the native `Client` /
// `MarketDataClient` prototypes at import time and re-exports the native
// classes, so importing it is enough to exercise both.
let mod;
try {
  const imported = await import('../streaming-session.js');
  mod = imported.default ?? imported;
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('base-client disposers route through close()', () => {
  it('Symbol.dispose calls close()', () => {
    const disposer = mod.Client.prototype[Symbol.dispose];
    assert.equal(typeof disposer, 'function', 'Client must define [Symbol.dispose]');
    let closed = 0;
    disposer.call({ close() { closed++; } });
    assert.equal(closed, 1, 'the sync disposer must invoke the real close()');
  });

  it('Symbol.asyncDispose still closes an already-closed client (stream getter throws)', async () => {
    const disposer = mod.Client.prototype[Symbol.asyncDispose];
    assert.equal(typeof disposer, 'function', 'Client must define [Symbol.asyncDispose]');
    let closed = 0;
    const alreadyClosed = {
      // A closed client throws "client is closed" from the `stream` getter.
      get stream() { throw new Error('client is closed'); },
      close() { closed++; },
    };
    // Must not reject: the disposer treats a throwing `stream` getter as
    // nothing-to-drain and still runs close().
    await disposer.call(alreadyClosed);
    assert.equal(closed, 1, 'asyncDispose must still run close() on a closed client');
  });

  it('Symbol.asyncDispose drains then closes a live client', async () => {
    const disposer = mod.Client.prototype[Symbol.asyncDispose];
    const order = [];
    const live = {
      get stream() {
        return {
          stopStreaming() { order.push('stopStreaming'); },
          async awaitDrain() { order.push('awaitDrain'); return true; },
        };
      },
      close() { order.push('close'); },
    };
    await disposer.call(live);
    assert.deepEqual(order, ['stopStreaming', 'awaitDrain', 'close']);
  });
});

describe('close() releases the handle and makes the client unusable', () => {
  it('unified Client: surface getters throw after close (live)', async () => {
    const credsPath = process.env.THETADATADX_TEST_CREDS;
    if (!credsPath) {
      console.log('SKIP: set THETADATADX_TEST_CREDS=/path/to/creds.txt to enable this live test');
      return;
    }
    const client = await mod.Client.connectFromFile(credsPath);
    client.close();
    assert.throws(() => client.marketData, /closed/, 'marketData must throw after close');
    assert.throws(() => client.stream, /closed/, 'stream must throw after close');
    assert.throws(() => client.flatFiles, /closed/, 'flatFiles must throw after close');
    // Idempotent.
    assert.doesNotThrow(() => client.close());
  });

  it('standalone MarketDataClient: endpoint call rejects after close (live)', async () => {
    const credsPath = process.env.THETADATADX_TEST_CREDS;
    if (!credsPath) {
      console.log('SKIP: set THETADATADX_TEST_CREDS=/path/to/creds.txt to enable this live test');
      return;
    }
    const hist = await mod.MarketDataClient.connectFromFile(credsPath);
    hist.close();
    await assert.rejects(
      hist.stockHistoryEOD('AAPL', '20240101', '20240301'),
      /closed/,
      'a market-data call after close must reject with the closed error',
    );
    assert.doesNotThrow(() => hist.close());
  });
});
