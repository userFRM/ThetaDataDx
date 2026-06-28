// `await using session = await client.streaming(callback)` lifecycle
// tests. Pins the contract that the wrapper:
//   * exists on the package's CJS entry point
//   * defines a `Symbol.asyncDispose` slot
//   * pairs `stopStreaming()` + `awaitDrain(5000)` on dispose
//   * fires `console.warn` when `awaitDrain` returns `false`
//   * proxies subscription methods to the underlying `Client`
//     via the `Proxy` SSOT path
//
// Runs without a live FPSS handshake by stubbing the `Client`
// instance the wrapper drives. The native binding is loaded only to
// verify the static factory shape; the streaming lifecycle itself is
// exercised against an in-memory mock so the test stays unit-scoped
// and stable across CI runs without credentials.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

// CI build step is mandatory before `npm test`; fail loud if the wrapper
// (which depends on the napi addon) cannot be loaded so a broken build
// does not appear green.
let mod;
try {
  const imported = await import(wrapperImportPath);
  // CJS modules show up under `default` when imported from ESM in
  // Node's interop layer; the wrapper sets `module.exports = {...}`
  // so we get the namespace via `default`.
  mod = imported.default ?? imported;
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('streaming-session wrapper', () => {
  it('exports StreamingSession constructor', () => {
    assert.equal(typeof mod.StreamingSession, 'function');
    assert.equal(typeof mod.Client, 'function');
    // streaming() is monkey-patched onto the prototype on require.
    assert.equal(typeof mod.Client.prototype.streaming, 'function');
  });

  it('Symbol.asyncDispose pairs stopStreaming with awaitDrain', async () => {
    const calls = [];
    // The unified client's streaming surface lives on the `client.stream`
    // sub-namespace view, so the session resolves these through `_client.stream`.
    const fakeClient = {
      stream: {
        stopStreaming() { calls.push('stopStreaming'); },
        async awaitDrain(timeoutMs) {
          calls.push(['awaitDrain', timeoutMs]);
          return true;
        },
        subscribe(sub) { calls.push(['subscribe', sub]); },
      },
    };
    const session = new mod.StreamingSession(fakeClient);

    // Proxy SSOT: `subscribe(sub)` proxies through to the wrapped client.
    const fakeSub = { kind: 'quote', isFull: false };
    session.subscribe(fakeSub);
    assert.deepEqual(calls.shift(), ['subscribe', fakeSub]);

    // asyncDispose should call stop then awaitDrain in that order.
    await session[Symbol.asyncDispose]();
    assert.deepEqual(calls, ['stopStreaming', ['awaitDrain', 5000]]);
  });

  it('warns to console when awaitDrain returns false', async () => {
    const fakeClient = {
      stream: {
        stopStreaming() {},
        async awaitDrain() { return false; },
      },
    };
    const session = new mod.StreamingSession(fakeClient);

    const originalWarn = console.warn;
    const warned = [];
    console.warn = (msg) => warned.push(msg);
    try {
      await session[Symbol.asyncDispose]();
    } finally {
      console.warn = originalWarn;
    }
    assert.equal(warned.length, 1, 'console.warn should fire exactly once on drain timeout');
    assert.match(warned[0], /drain timed out/, 'warning text should mention drain timeout');
    assert.match(warned[0], /5000ms/, 'warning text should include the 5000ms timeout');
  });

  it('Proxy forwards arbitrary method calls to the underlying client', () => {
    // Method that does NOT exist on StreamingSession itself but should
    // proxy through to the client. This is the SSOT property: adding a new
    // method to the napi binding makes it reachable on the session
    // automatically without a wrapper-side mirror.
    const fakeClient = {
      stream: {
        stopStreaming() {},
        async awaitDrain() { return true; },
        activeSubscriptions() { return [{ kind: 'Trade', contract: 'AAPL' }]; },
        droppedEventCount() { return 42n; },
      },
    };
    const session = new mod.StreamingSession(fakeClient);
    assert.deepEqual(session.activeSubscriptions(), [{ kind: 'Trade', contract: 'AAPL' }]);
    assert.equal(session.droppedEventCount(), 42n);
  });
});
