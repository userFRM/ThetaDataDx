// Pins the real napi streaming-callback delivery boundary that issue #1107
// asks about: does the `stopStreaming() + awaitDrain()` disposer pairing
// guarantee no callback fires after it resolves?
//
// PROVEN ANSWER (offline, real tsfn path): no. The per-event callback is
// delivered to the Node main thread through a bounded threadsafe-function
// queue (`STREAMING_CALLBACK_QUEUE_DEPTH`) that sits DOWNSTREAM of the
// streaming consumer thread. `awaitDrain` waits on the consumer thread, not
// on that delivery queue, so already-queued events keep invoking the callback
// on later event-loop turns after the awaited completion resolves.
//
// `__benchFloodEvents(n, cb)` drives that exact production path (same
// `TsfnCallback` type, same bounded queue, same per-event marshal) with
// synthetic in-process events, so this runs with no FPSS handshake and no
// credentials. Its Promise resolves once all `n` events have been QUEUED (the
// worker is done) -- the same "consumer side finished" moment `awaitDrain`
// keys off. A large `n` (well over what napi's throttled per-turn delivery can
// flush before the Promise resolves) makes the assertion robust, not timing
// dependent.
//
// This test documents the guarantee the disposer docs now state. If a future
// change adds a hard delivery barrier (e.g. a terminal threadsafe-function
// abort that drops the queued tail), this test AND those docs must change
// together -- that is the point of pinning it.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const imported = await import('../streaming-session.js');
const mod = imported.default ?? imported;

const tick = () => new Promise((resolve) => setImmediate(resolve));

describe('streaming callback delivery barrier (#1107)', () => {
  it('exposes the real bench flood path', () => {
    assert.equal(typeof mod.__benchFloodEvents, 'function');
  });

  it('queued callbacks still fire after the awaited completion resolves', async () => {
    const N = 20000;
    let received = 0;
    let firedAfterAwait = 0;
    let awaitResolved = false;
    const cb = () => {
      received += 1;
      if (awaitResolved) firedAfterAwait += 1;
    };

    // Resolves once all N events are QUEUED onto the tsfn -- the consumer-side
    // "done" signal, the analogue of `awaitDrain` observing the consumer
    // thread's exit flag.
    await mod.__benchFloodEvents(N, cb);
    awaitResolved = true;
    const receivedAtResolve = received;

    // The awaited completion does NOT imply every callback has fired: napi
    // delivers the bounded backlog in throttled bursts across later event-loop
    // turns. With N this large only a small prefix can have been delivered.
    assert.ok(
      receivedAtResolve < N,
      `expected an undelivered backlog at await-resolve, but all ${N} had already fired`,
    );

    // Drain the remaining backlog off the event loop.
    for (let i = 0; i < 500 && received < N; i += 1) {
      // eslint-disable-next-line no-await-in-loop
      await tick();
    }
    await new Promise((resolve) => setTimeout(resolve, 100));

    // The tail is delivered, not dropped (no event loss on the healthy path)
    // and, crucially, it fired AFTER the awaited completion resolved -- which
    // is exactly the behavior the disposer docs now warn about.
    assert.equal(received, N, 'all queued events should eventually be delivered');
    assert.ok(
      firedAfterAwait > 0,
      'callbacks are expected to fire after the awaited drain resolves',
    );
    assert.equal(
      firedAfterAwait,
      N - receivedAtResolve,
      'every not-yet-delivered event fires after the awaited completion',
    );
  });
});
