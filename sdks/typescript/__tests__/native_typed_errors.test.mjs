// Native regression tests for the typed-error contract on every call
// shape, exercised against the real built addon (not a synthetic stub).
//
// The typed-error contract requires both halves to hold for a typed
// error to reach the caller: the Rust method tags its error reason with
// a `[ClassName] ...` prefix, AND the JS entrypoint is wrapped so the
// shim's `rethrowTyped` runs. Instance methods were wrapped already;
// these tests pin the previously-unwrapped shapes:
//
//   * static / factory methods (`ThetaDataDxClient.connectFromFile`,
//     `Contract.option`) — wrapped on the constructor object itself;
//   * exported free functions (`calendarDayToArrowIpc`,
//     `eodTickToArrowIpc`) — wrapped directly on the native binding.
//
// They also pin the Arrow-IPC enum-validation parity with Python: an
// invalid logical `status` / `right` must be REJECTED as
// `InvalidParameterError`, never silently coerced to a neighbouring
// variant (the prior `unwrap_or(Weekend)` / `chars().next()` truncation
// corrupted data instead of failing loudly).
//
// Loads the wrapped surface (`../streaming-session.js`), the package
// `main`, so the assertions observe exactly what a consumer sees. When
// the native addon cannot be dlopen'd (no `.node` built) the suite
// skips, matching the offline-tolerance of the sibling test files.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

async function loadWrapped() {
  try {
    const imported = await import(wrapperImportPath);
    return imported.default ?? imported;
  } catch (err) {
    if (err.code === 'ERR_DLOPEN_FAILED' || err.code === 'MODULE_NOT_FOUND') {
      return null;
    }
    throw err;
  }
}

// Arrow IPC streams open with the 0xFFFFFFFF continuation marker.
function looksLikeArrowIpcStream(buf) {
  return buf.length >= 8 && buf[0] === 0xff && buf[1] === 0xff && buf[2] === 0xff && buf[3] === 0xff;
}

// A valid single-row CalendarDay (used for the happy-path control and
// as the base the invalid-status case mutates).
function validCalendarDay(status) {
  return {
    date: 20260102,
    isOpen: true,
    openTime: 34200000,
    closeTime: 57600000,
    status,
  };
}

// A valid single-row EodTick (used for the happy-path control and as
// the base the invalid-right case mutates).
function validEodTick(right) {
  return {
    createdMsOfDay: 0,
    lastTradeMsOfDay: 0,
    open: 1,
    high: 1,
    low: 1,
    close: 1,
    volume: 1n,
    count: 1n,
    bidSize: 0,
    bidExchange: 0,
    bid: 0,
    bidCondition: 0,
    askSize: 0,
    askExchange: 0,
    ask: 0,
    askCondition: 0,
    date: 20260115,
    expiration: 20260117,
    strike: 550,
    right,
  };
}

describe('typed errors on static / factory entrypoints (native)', () => {
  it('ThetaDataDxClient.connectFromFile on a missing path throws ThetaDataError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    assert.throws(
      () => mod.ThetaDataDxClient.connectFromFile('/definitely/missing-creds.txt'),
      (err) => err instanceof mod.ThetaDataError,
      'a static factory failure must reclassify to the typed hierarchy, not a plain Error',
    );
  });

  it('Contract.option with a bad expiration throws InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    assert.throws(
      () => mod.Contract.option('SPY', { expiration: 'bad', strike: '550', right: 'C' }),
      (err) =>
        err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an invalid option leg must reclassify to InvalidParameterError',
    );
  });
});

describe('Arrow-IPC logical-enum validation parity (native)', () => {
  it('calendarDayToArrowIpc rejects an invalid status as InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    assert.throws(
      () => mod.calendarDayToArrowIpc([validCalendarDay('bogus')]),
      (err) => err instanceof mod.InvalidParameterError,
      'an out-of-vocabulary status must be rejected, not coerced to Weekend',
    );
  });

  it('calendarDayToArrowIpc accepts a valid status and returns an Arrow IPC Buffer', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    const buf = mod.calendarDayToArrowIpc([validCalendarDay('full_close')]);
    assert.ok(Buffer.isBuffer(buf), 'a valid status must serialise to a Buffer');
    assert.ok(looksLikeArrowIpcStream(buf), 'expected an Arrow IPC stream');
  });

  it('eodTickToArrowIpc rejects an invalid right as InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    assert.throws(
      () => mod.eodTickToArrowIpc([validEodTick('XYZ')]),
      (err) => err instanceof mod.InvalidParameterError,
      'an out-of-vocabulary right must be rejected, not truncated to its first char',
    );
  });

  it('eodTickToArrowIpc accepts a valid right and returns an Arrow IPC Buffer', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    const buf = mod.eodTickToArrowIpc([validEodTick('C')]);
    assert.ok(Buffer.isBuffer(buf), 'a valid right must serialise to a Buffer');
    assert.ok(looksLikeArrowIpcStream(buf), 'expected an Arrow IPC stream');
  });
});
