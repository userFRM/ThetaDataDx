// `await using session = await tdx.streamingIter()` lifecycle tests.
//
// Pins the contract that the JS shim:
//   * exposes `StreamingIterSession` on the package surface
//   * patches `streamingIter()` onto `ThetaDataDxClient.prototype`
//   * the session implements `[Symbol.asyncIterator]` so user code
//     can `for await (const event of session)` directly
//   * the session implements `[Symbol.asyncDispose]` so `await using`
//     pairs the iterator close + `stopStreaming` + `awaitDrain`
//
// Network round-trip is out of scope here — the goal is the type-
// level surface and the dispose protocol. Live FPSS coverage lives
// in the Rust soak tests + the Python live-credentials test.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

describe('pull-iter StreamingIterSession wrapper', () => {
  it('exposes StreamingIterSession on the package surface', async () => {
    let mod;
    try {
      const imported = await import(wrapperImportPath);
      mod = imported.default ?? imported;
    } catch (err) {
      // Native binary may be missing on machines without a built FFI;
      // skip rather than fail to keep the lint matrix green.
      if (err.code === 'ERR_DLOPEN_FAILED') {
        return;
      }
      throw err;
    }
    assert.equal(
      typeof mod.StreamingIterSession,
      'function',
      'StreamingIterSession must be exported',
    );
    assert.equal(
      typeof mod.StreamingIterSession.prototype[Symbol.asyncDispose],
      'function',
      'session must implement Symbol.asyncDispose',
    );
    assert.equal(
      typeof mod.StreamingIterSession.prototype[Symbol.asyncIterator],
      'function',
      'session must implement Symbol.asyncIterator',
    );
  });

  it('patches streamingIter() onto ThetaDataDxClient.prototype', async () => {
    let mod;
    try {
      const imported = await import(wrapperImportPath);
      mod = imported.default ?? imported;
    } catch (err) {
      if (err.code === 'ERR_DLOPEN_FAILED') return;
      throw err;
    }
    assert.equal(
      typeof mod.ThetaDataDxClient.prototype.streamingIter,
      'function',
      'ThetaDataDxClient.prototype.streamingIter must be patched',
    );
  });

  it('iterates and disposes a mock iterator', async () => {
    let mod;
    try {
      const imported = await import(wrapperImportPath);
      mod = imported.default ?? imported;
    } catch (err) {
      if (err.code === 'ERR_DLOPEN_FAILED') return;
      throw err;
    }

    // Mock just enough of the napi surface to drive the session
    // through one event + the disposal protocol without a live
    // streaming connection.
    let stopCalls = 0;
    let drainCalls = 0;
    let closeCalls = 0;
    const events = [{ kind: 0 }];
    const mockIter = {
      next: async () => events.shift() ?? null,
      close: () => {
        closeCalls += 1;
      },
    };
    const mockTdx = {
      stopStreaming: () => {
        stopCalls += 1;
      },
      awaitDrain: async () => {
        drainCalls += 1;
        return true;
      },
    };

    const session = new mod.StreamingIterSession(mockTdx, mockIter);
    const seen = [];
    for await (const event of session) {
      seen.push(event);
    }
    assert.equal(seen.length, 1, 'one event yielded by the async iterator');

    await session[Symbol.asyncDispose]();
    assert.equal(stopCalls, 1, 'stopStreaming invoked exactly once on dispose');
    assert.equal(drainCalls, 1, 'awaitDrain invoked exactly once on dispose');
    assert.equal(closeCalls, 1, 'iterator close invoked on dispose');
  });
});
