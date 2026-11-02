// The package's `startStreaming` forwarder (offline, stub native method).
//
// The native `startStreaming` hands events to the Node main thread in arrays.
// The forwarder the package installs must call the caller's callback once per
// event, in order, and a throwing callback must not cost the rest of the
// array: the throw is rethrown on the next tick instead.

import { it } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { wrapStartStreaming } = require('../record_batch_forwarder.js');

it('startStreaming calls the callback once per event, in order, past a throw', async () => {
  let deliver;
  const receiver = {};
  const startStreaming = wrapStartStreaming(async function nativeStart(handler) {
    assert.strictEqual(this, receiver, 'the receiver is preserved as `this`');
    deliver = handler;
  });
  const seen = [];
  const failure = new Error('callback failure');
  await startStreaming.call(receiver, (event) => {
    seen.push(event);
    if (event === 2) throw failure;
  });

  const deferred = [];
  const nextTick = process.nextTick;
  process.nextTick = (fn) => deferred.push(fn);
  try {
    deliver([1, 2, 3]);
    deliver([4]);
  } finally {
    process.nextTick = nextTick;
  }

  assert.deepEqual(seen, [1, 2, 3, 4]);
  assert.equal(deferred.length, 1, 'the throw is rethrown once, on the next tick');
  assert.throws(deferred[0], (err) => err === failure);
});
