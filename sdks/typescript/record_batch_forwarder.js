'use strict';

// Internal forwarder for `StreamView.prototype.batches`. Not part of the
// public API: `streaming-session.js` installs it onto the native prototype
// and the test suite imports it from here to drive the real
// options-object -> native call shape against a stub native method (no live
// server). `RecordBatchStream` is injected rather than required to keep this
// free of a circular dependency on `streaming-session.js`.
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

module.exports = { wrapStreamViewBatches };
