// Base-client lifecycle tests (issue #1069): the deterministic `close()`
// plus the TC39 `[Symbol.dispose]` / `[Symbol.asyncDispose]` disposers that
// `using` / `await using` invoke on scope exit.
//
// Pins that:
//   * `Client` and `HistoricalClient` expose `close()` (napi-generated) and
//     the two disposer symbols (patched on at require time);
//   * the sync disposer routes through `close()`;
//   * the async disposer on the unified client pairs `stopStreaming()` with
//     `awaitDrain(5000)` and warns (never throws) on drain timeout;
//   * the async disposer on the historical-only surface (no `.stream`) falls
//     back to `close()`.
//
// Runs without a live FPSS handshake: the disposer bodies read `this.stream`
// / `this.close`, so they are invoked against in-memory fakes via `.call()`,
// keeping the test unit-scoped and credential-free. The native binding is
// loaded only to confirm the methods are wired onto the real prototypes.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

let mod;
try {
  const imported = await import(wrapperImportPath);
  mod = imported.default ?? imported;
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('base-client lifecycle surface', () => {
  it('Client exposes close() and both disposer symbols', () => {
    assert.equal(typeof mod.Client, 'function');
    assert.equal(typeof mod.Client.prototype.close, 'function', 'Client.close()');
    assert.equal(
      typeof mod.Client.prototype[Symbol.dispose],
      'function',
      'Client[Symbol.dispose]',
    );
    assert.equal(
      typeof mod.Client.prototype[Symbol.asyncDispose],
      'function',
      'Client[Symbol.asyncDispose]',
    );
  });

  it('HistoricalClient exposes close() and both disposer symbols', () => {
    assert.equal(typeof mod.HistoricalClient, 'function');
    assert.equal(typeof mod.HistoricalClient.prototype.close, 'function');
    assert.equal(typeof mod.HistoricalClient.prototype[Symbol.dispose], 'function');
    assert.equal(typeof mod.HistoricalClient.prototype[Symbol.asyncDispose], 'function');
  });

  it('[Symbol.dispose] routes through close()', () => {
    let closed = 0;
    const fake = { close() { closed += 1; } };
    mod.Client.prototype[Symbol.dispose].call(fake);
    assert.equal(closed, 1, 'sync dispose must call close() exactly once');
  });

  it('[Symbol.asyncDispose] pairs stopStreaming with awaitDrain on the unified client', async () => {
    const calls = [];
    const fake = {
      stream: {
        stopStreaming() { calls.push('stopStreaming'); },
        async awaitDrain(timeoutMs) {
          calls.push(['awaitDrain', timeoutMs]);
          return true;
        },
      },
      close() { calls.push('close'); },
    };
    await mod.Client.prototype[Symbol.asyncDispose].call(fake);
    // Streaming surface present → stop + drain, never the close() fallback.
    assert.deepEqual(calls, ['stopStreaming', ['awaitDrain', 5000]]);
  });

  it('[Symbol.asyncDispose] warns (does not throw) when the drain times out', async () => {
    const fake = {
      stream: {
        stopStreaming() {},
        async awaitDrain() { return false; },
      },
      close() {},
    };
    const originalWarn = console.warn;
    const warned = [];
    console.warn = (msg) => warned.push(msg);
    try {
      await mod.Client.prototype[Symbol.asyncDispose].call(fake);
    } finally {
      console.warn = originalWarn;
    }
    assert.equal(warned.length, 1, 'exactly one warning on drain timeout');
    assert.match(warned[0], /drain timed out/);
    assert.match(warned[0], /5000ms/);
  });

  it('[Symbol.asyncDispose] falls back to close() with no streaming surface', async () => {
    let closed = 0;
    // Historical-only shape: no `.stream`. The async disposer must not throw
    // reaching for `stopStreaming`; it closes instead.
    const fake = { stream: undefined, close() { closed += 1; } };
    await mod.HistoricalClient.prototype[Symbol.asyncDispose].call(fake);
    assert.equal(closed, 1, 'historical async dispose must call close() exactly once');
  });
});
