// The package's `startStreaming` forwarder (offline, stub native method).
//
// The native `startStreaming` hands events to the Node main thread in arrays.
// The forwarder the package installs must call the caller's callback once per
// event, in order. A throw that an `uncaughtException` handler will see must
// not cost the rest of the array (it is rethrown on the next tick); a throw
// nothing handles must end the process at that event.

import { it } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { wrapStartStreaming } = require('../record_batch_forwarder.js');

// The test runner installs its own `uncaughtException` handler, so this case
// covers the handled path.
it('startStreaming calls the callback once per event, in order, past a handled throw', async () => {
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

it('an unhandled throw from the callback ends the process at that event', () => {
  const forwarder = require.resolve('../record_batch_forwarder.js');
  const script = `
    const { forEachEvent } = require(${JSON.stringify(forwarder)});
    const seen = [];
    process.on('exit', () => process.stdout.write(JSON.stringify(seen)));
    forEachEvent((event) => {
      seen.push(event);
      if (event === 1) throw new Error('callback failure');
    })([1, 2, 3]);
  `;
  const child = spawnSync(process.execPath, ['-e', script], { encoding: 'utf8' });
  assert.notEqual(child.status, 0, 'the process exits with an error');
  assert.equal(child.stdout, '[1]', 'no event after the throwing one reaches the callback');
});
