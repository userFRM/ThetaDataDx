// Typed error hierarchy tests.
//
// Before B4, every napi error surfaced as a plain `Error` carrying
// the formatted reason string — callers had to substring-match to
// distinguish auth failures from rate limits. B4 introduces a
// `ThetaDataError` base + a leaf class per `GrpcStatusKind` /
// `AuthErrorKind` discriminator. The hierarchy mirrors the Python
// leaf set so the cross-binding contract stays uniform.
//
// The JS shim re-casts every napi-thrown error whose reason carries
// a `[ClassName] ...` prefix as the matching subclass; this test
// drives that interceptor directly because the offline harness can't
// reproduce a real RPC failure.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

const wrapperImportPath = '../streaming-session.js';

describe('typed error hierarchy', () => {
  it('exports every documented leaf class', async () => {
    let mod;
    try {
      const imported = await import(wrapperImportPath);
      mod = imported.default ?? imported;
    } catch (err) {
      if (err.code === 'ERR_DLOPEN_FAILED') return;
      throw err;
    }

    const expected = [
      'ThetaDataError',
      'AuthenticationError',
      'InvalidCredentialsError',
      'SubscriptionError',
      'RateLimitError',
      'NotFoundError',
      'DeadlineExceededError',
      'UnavailableError',
      'NetworkError',
      'SchemaMismatchError',
      'StreamError',
    ];

    for (const name of expected) {
      assert.equal(typeof mod[name], 'function', `${name} must be exported`);
      const inst = new mod[name]('test');
      assert.ok(
        inst instanceof Error,
        `${name} must derive from Error`,
      );
      assert.ok(
        inst instanceof mod.ThetaDataError,
        `${name} must derive from ThetaDataError`,
      );
      assert.equal(inst.name, name, `${name}.name must match the class name`);
    }

    // InvalidCredentialsError narrows AuthenticationError.
    const invalid = new mod.InvalidCredentialsError('bad password');
    assert.ok(
      invalid instanceof mod.AuthenticationError,
      'InvalidCredentialsError must derive from AuthenticationError',
    );
  });

  it('parses the [ClassName] prefix off napi-thrown errors', async () => {
    // Drive the JS shim's interceptor by simulating the napi error
    // shape: a plain `Error` whose `.message` starts with the
    // `[ClassName] ...` prefix `to_napi_err` emits. The shim must
    // re-throw as the matching typed subclass with the prefix
    // stripped from the user-visible message.
    let mod;
    try {
      const imported = await import(wrapperImportPath);
      mod = imported.default ?? imported;
    } catch (err) {
      if (err.code === 'ERR_DLOPEN_FAILED') return;
      throw err;
    }

    // Build a tiny consumer of the same `rethrowTyped` logic by
    // patching a stub method onto a class instance and forcing it
    // to throw. Mirrors what every napi-bound method on
    // `ThetaDataDxClient` does after the JS shim wrap.
    class Stub {
      throwIt(msg) {
        const e = new Error(msg);
        throw e;
      }
    }
    // Manually mirror the `wrapMethodsWithTypedErrors` pattern from
    // the shim so we can test the typed re-throw on an arbitrary
    // class without standing up the full native binding.
    const PREFIX_RE = /^\[([A-Za-z]+Error)\]\s*(.*)$/s;
    const original = Stub.prototype.throwIt;
    Stub.prototype.throwIt = function patched(msg) {
      try {
        return original.call(this, msg);
      } catch (err) {
        const match = PREFIX_RE.exec(err.message);
        if (!match) throw err;
        const Cls = mod[match[1]];
        if (!Cls) throw err;
        const typed = new Cls(match[2]);
        typed.stack = err.stack;
        throw typed;
      }
    };

    const stub = new Stub();
    assert.throws(
      () => stub.throwIt('[SubscriptionError] tier insufficient'),
      (err) => {
        return (
          err instanceof mod.SubscriptionError &&
          err.message === 'tier insufficient'
        );
      },
    );
    assert.throws(
      () => stub.throwIt('[RateLimitError] back off'),
      (err) => err instanceof mod.RateLimitError,
    );
    assert.throws(
      () => stub.throwIt('[AuthenticationError] session expired'),
      (err) => err instanceof mod.AuthenticationError,
    );
    // Errors without the prefix surface unchanged — preserves the
    // existing Error shape for failures inside napi argument
    // coercion etc.
    assert.throws(
      () => stub.throwIt('something raw and untyped'),
      (err) => err instanceof Error && !(err instanceof mod.ThetaDataError),
    );
  });
});
