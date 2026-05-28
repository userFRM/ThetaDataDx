// Pull-iter delivery lifecycle tests.
//
// Pins the contract that the JS shim:
//   * exposes the `EventIterator` napi class
//   * monkey-patches `[Symbol.asyncIterator]` onto its prototype so
//     `for await (const event of iter)` works
//   * `return()` (the early-break path) calls `iter.close()`
//
// The actual streaming round-trip is covered by the Rust soak tests
// + the Python live-credentials test; this test stays unit-scoped and
// runs without a live FPSS handshake.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

// CI build step is mandatory before `npm test`; fail loud if the wrapper
// (which depends on the napi addon) cannot be loaded so a broken build
// does not appear green.
let mod;
try {
  const imported = await import(wrapperImportPath);
  mod = imported.default ?? imported;
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('pull-iter EventIterator wrapper', () => {
  it('exposes EventIterator on the package surface', () => {
    assert.equal(
      typeof mod.EventIterator,
      'function',
      'EventIterator must be re-exported from the wrapper module',
    );
    assert.equal(
      typeof mod.ThetaDataDxClient.prototype.startStreamingIter,
      'function',
      'ThetaDataDxClient.startStreamingIter must be a napi-bound method',
    );
  });

  it('attaches Symbol.asyncIterator to EventIterator prototype', () => {
    assert.equal(
      typeof mod.EventIterator.prototype[Symbol.asyncIterator],
      'function',
      'Symbol.asyncIterator must be patched on EventIterator',
    );
  });

  it('next() resolves to null after close() — timeout vs closed disambiguation', async () => {
    // Earlier the napi iterator's `next()` spun forever once the
    // upstream client shut down because the Rust `next_timeout` call
    // returned `None` for both timeout and terminal close, so the
    // `spawn_blocking` loop never observed an exit condition. After
    // the `NextEvent` enum was wired through, `Closed` resolves the
    // promise to `null` and a `for await` loop exits cleanly.
    //
    // This unit-scoped variant uses a stand-in object whose `next`
    // resolves to `null` to mimic the Closed signal — the actual
    // Rust-side disambiguation is covered by the soak tests in
    // `crates/thetadatadx/src/fpss/streaming_soak_tests.rs`.
    const fakeIter = {
      async next() {
        return null;
      },
      close() {},
    };
    Object.setPrototypeOf(fakeIter, mod.EventIterator.prototype);

    const started = Date.now();
    let observed = 0;
    for await (const _evt of fakeIter) {
      observed += 1;
    }
    const elapsed = Date.now() - started;
    assert.equal(observed, 0, 'closed iterator must yield no events');
    assert.ok(
      elapsed < 1000,
      `for-await loop must exit promptly on Closed; took ${elapsed}ms`,
    );
  });

  it('Symbol.asyncIterator return() closes the underlying iterator', async () => {
    // Construct a stand-in object that exposes the `next` / `close`
    // contract the JS-side `[Symbol.asyncIterator]` uses, without
    // standing up a real FPSS connection. The shim's protocol shape
    // is what we are pinning here; the underlying napi class is
    // covered by the Rust soak tests.
    let closeCalls = 0;
    let nextCalls = 0;
    const fakeIter = {
      async next() {
        nextCalls += 1;
        // Yield one event then signal end so `for await` exits via
        // the normal path rather than `return()`.
        return nextCalls === 1 ? { kind: 'mock' } : null;
      },
      close() {
        closeCalls += 1;
      },
    };
    Object.setPrototypeOf(fakeIter, mod.EventIterator.prototype);

    let observed = 0;
    for await (const _evt of fakeIter) {
      observed += 1;
      // Early break exercises `return()` -> `close()`.
      break;
    }
    assert.equal(observed, 1);
    assert.equal(closeCalls, 1, 'early break must call close() via return()');
  });
});
