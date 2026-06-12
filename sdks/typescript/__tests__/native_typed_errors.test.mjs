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
// And they pin the user-input validation parity on the `Config` setters
// and the FLATFILES enum parsers: an invalid enum / out-of-range value
// rejected by a hand-written guard before the call reaches the core
// client must surface as `InvalidParameterError`, the same class the
// Python binding raises (`ValueError`) for the identical bad input — so
// a caller's `catch (e) { if (e instanceof InvalidParameterError) }`
// branch ports across bindings. The `Config` cases are reachable
// without a connection; the FLATFILES parsers sit on a connected
// namespace, so that block builds a client from a credentials file and
// skips when none is available (set `THETADATADX_TEST_CREDS` to point at
// one), exercising only the synchronous enum parse that runs before any
// network round-trip.
//
// Loads the wrapped surface (`../streaming-session.js`), the package
// `main`, so the assertions observe exactly what a consumer sees. When
// the native addon cannot be dlopen'd (no `.node` built) the suite
// skips, matching the offline-tolerance of the sibling test files.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';

const wrapperImportPath = '../streaming-session.js';

// Optional credentials file for the FLATFILES enum-parse block. The
// parsers live on `tdx.flat_files` / `tdx.flatFileToPath`, which require
// a built client; the enum parse itself is synchronous and runs before
// any network I/O, so a connected client is enough to exercise it. When
// no credentials file is present the block skips, keeping the suite
// runnable offline and on CI.
const TEST_CREDS_PATH = process.env.THETADATADX_TEST_CREDS ?? '/home/theta-gamma/thetadx/creds.txt';

// Build a client from the credentials file, or return `null` when none
// is available / the connect fails — the caller skips in that case.
function tryConnect(mod) {
  if (!existsSync(TEST_CREDS_PATH)) return null;
  try {
    return mod.ThetaDataDxClient.connectFromFile(TEST_CREDS_PATH);
  } catch {
    return null;
  }
}

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

describe('Config setter input-validation parity (native)', () => {
  it('setReconnectPolicy with an unknown policy throws InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    const cfg = mod.Config.production();
    assert.throws(
      () => cfg.setReconnectPolicy('bogus'),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an unknown reconnect policy must reclassify to InvalidParameterError, matching the Python ValueError',
    );
  });

  it('setReconnectPolicy accepts a valid policy', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    const cfg = mod.Config.production();
    assert.doesNotThrow(
      () => cfg.setReconnectPolicy('manual'),
      'a valid reconnect policy must be accepted',
    );
  });

  it('setFpssRingSize rejects a non-power-of-two as InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    // The setter rejects eagerly with the typed class; the core's
    // connect-time validation of `fpss.ring_size` is unchanged.
    const cfg = mod.Config.production();
    assert.throws(
      () => cfg.setFpssRingSize(100),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'a non-power-of-two ring size must reclassify to InvalidParameterError',
    );
  });

  it('setFpssRingSize accepts a valid power-of-two >= 64', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    const cfg = mod.Config.production();
    assert.doesNotThrow(
      () => cfg.setFpssRingSize(65536),
      'a valid power-of-two ring size must be accepted',
    );
  });

  it('setReconnectWaitMs keeps an integer-domain overflow as a plain Error', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    // A value too large for u64 is a representation overflow, not an
    // invalid parameter: Python raises the built-in OverflowError here,
    // so TypeScript intentionally leaves it a plain Error rather than
    // inventing a typed class the parity binding does not raise.
    const cfg = mod.Config.production();
    assert.throws(
      () => cfg.setReconnectWaitMs(2n ** 70n),
      (err) => err instanceof Error && !(err instanceof mod.InvalidParameterError),
      'an integer-domain overflow must stay a plain Error, not reclassify',
    );
  });
});

describe('Sequence-util wire-range input-validation parity (native)', () => {
  it('sequenceUnsignedToSigned rejects an above-wire-range value as InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    // 2^32 fits the BigInt parameter but is outside the unsigned wire
    // range (0 ..= 2^32 - 1). It is a rejected value, not a silent
    // reinterpret, so through the wrapped surface it reclassifies to
    // InvalidParameterError — matching the Python ValueError / C++
    // InvalidParameterError for the same input. Before the fix this
    // returned 0.
    assert.throws(
      () => mod.Util.sequenceUnsignedToSigned(4294967296n),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an above-wire-range sequence value must reclassify to InvalidParameterError',
    );
  });

  it('sequenceSignedToUnsigned rejects an out-of-i32-range value as InvalidParameterError', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    assert.throws(
      () => mod.Util.sequenceSignedToUnsigned(2147483648n),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an out-of-i32-range sequence value must reclassify to InvalidParameterError',
    );
  });

  it('sequence converters round-trip in-wire-range values', async () => {
    const mod = await loadWrapped();
    if (!mod) return;

    for (const signed of [-2147483648n, -1n, 0n, 1n, 2147483647n]) {
      const unsigned = mod.Util.sequenceSignedToUnsigned(signed);
      assert.equal(mod.Util.sequenceUnsignedToSigned(unsigned), signed);
    }
  });
});

describe('FLATFILES enum-parse input-validation parity (native)', () => {
  it('flatFiles.request with an unknown sec_type throws InvalidParameterError', async (t) => {
    const mod = await loadWrapped();
    if (!mod) return;
    const tdx = tryConnect(mod);
    if (!tdx) {
      t.skip('no credentials available to build a client for the FLATFILES surface');
      return;
    }

    assert.throws(
      () => tdx.flatFiles.request('BOGUS', 'QUOTE', '20260102'),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an unknown flat-file sec_type must reclassify to InvalidParameterError, matching the Python ValueError',
    );
  });

  it('flatFileToPath with an unknown req_type throws InvalidParameterError', async (t) => {
    const mod = await loadWrapped();
    if (!mod) return;
    const tdx = tryConnect(mod);
    if (!tdx) {
      t.skip('no credentials available to build a client for the FLATFILES surface');
      return;
    }

    assert.throws(
      () => tdx.flatFileToPath('OPTION', 'BOGUS', '20260102', '/tmp/thetadatadx-test.csv'),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'an unknown flat-file req_type must reclassify to InvalidParameterError',
    );
  });
});

describe('timeoutMs input-validation parity (native)', () => {
  // `timeoutMs` rides in the per-endpoint options object as a JS `number`
  // (an IEEE-754 double). The integer-typed bindings (Python / C++ / C
  // ABI) take a `u64`, which cannot represent a non-finite, negative, or
  // fractional deadline; this binding rejects the same inputs rather than
  // coercing them. A bare `as u64` cast would silently rewrite `NaN` and a
  // negative value to `0` (an instant deadline), `Infinity` to `u64::MAX`
  // (a multi-century deadline), and a fractional value to its truncation —
  // each the opposite of the caller's intent. The reject must surface as
  // `InvalidParameterError`, the same class the Python binding raises
  // (`ValueError`) for the identical bad input, so a caller's
  // `catch (e instanceof InvalidParameterError)` branch ports across
  // bindings. Validation runs synchronously before the request task is
  // spawned, so the Promise rejects regardless of network state.
  //
  // Both endpoint shapes are covered: a builder-backed endpoint
  // (`stockSnapshotQuote`) and a string-list endpoint (`stockListSymbols`),
  // since each consumes the deadline at a separate generated cast site.
  const badValues = [
    ['NaN', Number.NaN],
    ['Infinity', Number.POSITIVE_INFINITY],
    ['-Infinity', Number.NEGATIVE_INFINITY],
    ['a negative value', -1],
    ['a fractional value', 1.9],
  ];

  for (const [label, value] of badValues) {
    it(`stockSnapshotQuote rejects ${label} timeoutMs as InvalidParameterError`, async (t) => {
      const mod = await loadWrapped();
      if (!mod) return;
      const tdx = tryConnect(mod);
      if (!tdx) {
        t.skip('no credentials available to build a client for the endpoint surface');
        return;
      }

      await assert.rejects(
        () => tdx.stockSnapshotQuote('AAPL', { timeoutMs: value }),
        (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
        `a ${label} timeoutMs must reject as InvalidParameterError, not coerce`,
      );
    });
  }

  it('stockListSymbols rejects a negative timeoutMs on the string-list path', async (t) => {
    const mod = await loadWrapped();
    if (!mod) return;
    const tdx = tryConnect(mod);
    if (!tdx) {
      t.skip('no credentials available to build a client for the endpoint surface');
      return;
    }

    await assert.rejects(
      () => tdx.stockListSymbols({ timeoutMs: -1 }),
      (err) => err instanceof mod.InvalidParameterError && err instanceof mod.ThetaDataError,
      'a negative timeoutMs must reject on the string-list path too',
    );
  });

  it('stockSnapshotQuote accepts a valid whole-millisecond timeoutMs', async (t) => {
    const mod = await loadWrapped();
    if (!mod) return;
    const tdx = tryConnect(mod);
    if (!tdx) {
      t.skip('no credentials available to build a client for the endpoint surface');
      return;
    }

    // A valid integer deadline must pass validation and reach the core
    // client. The round-trip may surface a data/network error, but it must
    // never be an InvalidParameterError from the deadline guard.
    try {
      await tdx.stockSnapshotQuote('AAPL', { timeoutMs: 5000 });
    } catch (err) {
      assert.ok(
        !(err instanceof mod.InvalidParameterError),
        `a valid integer timeoutMs must not be rejected by validation: ${err.message}`,
      );
    }
  });
});
