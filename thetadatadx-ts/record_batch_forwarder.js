'use strict';

// Internal forwarders for `StreamView.prototype.batches` and the streaming
// callback. Not part of the public API: `streaming-session.js` installs them
// onto the native prototypes and the test suite imports them from here to
// drive the real call shapes against stub native methods (no live server).
// `RecordBatchStream` is injected rather than required to keep this free of
// a circular dependency on `streaming-session.js`.
//
// The native `batches(options?)` takes ONE object argument; the wrap forwards
// the caller's single options object straight through (no positional
// explosion) and re-wraps the returned native handle in a `RecordBatchStream`.
function wrapStreamViewBatches(nativeBatches, RecordBatchStream) {
  return async function batches(...args) {
    const handle = await nativeBatches.apply(this, args);
    return new RecordBatchStream(handle);
  };
}

// The native `startStreaming(callback)` hands events to the Node main thread
// in arrays, one hop per array, so a burst pays the hop cost once per array
// rather than once per event. `forEachEvent` turns the caller's per-event
// callback into the array handler the native method registers: it calls the
// callback once per event, in order. A throw from the callback is rethrown on
// the next tick, so it still reaches Node's `uncaughtException` while the rest
// of the array is delivered.
function forEachEvent(callback) {
  return (events) => {
    for (const event of events) {
      try {
        callback(event);
      } catch (err) {
        process.nextTick(() => {
          throw err;
        });
      }
    }
  };
}

// Forwarder for `StreamView.prototype.startStreaming` and
// `StreamingClient.prototype.startStreaming`. A non-function goes straight
// through so the native argument check reports it.
function wrapStartStreaming(nativeStart) {
  return function startStreaming(callback) {
    return nativeStart.call(this, typeof callback === 'function' ? forEachEvent(callback) : callback);
  };
}

module.exports = { wrapStreamViewBatches, forEachEvent, wrapStartStreaming };
