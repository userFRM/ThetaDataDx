// Client.connectWith({ ... }) options contract - TypeScript binding guard.
//
// Mirrors the Python `tests/test_inline_client_construction.py` pattern:
// the connectWith factory resolves the authentication fields (and the
// environment) and rejects a conflicting / absent / unparseable option set
// with a `[ConfigError]` BEFORE any network round-trip, so every field can
// be pinned by name offline. A field that was renamed or dropped on the
// `ClientConnectOptions` napi object would be silently ignored by napi
// (the value arrives as `undefined`), which changes the rejection reason,
// so each assertion below fails loud on option-contract drift rather than
// silently passing.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

const { Client } = mod;

// The "no authentication field set" reason fires only when the auth-field
// count resolves to zero. If a named auth field is wired and parsed, the
// resolver enters that field's branch and fails for a DIFFERENT reason
// (a missing env var / missing file), so the absence of this exact reason
// is the proof the field reached the resolver under its declared name.
const NO_AUTH_RE = /no authentication field set/;

async function rejectionMessage(promise) {
  try {
    await promise;
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
  throw new assert.AssertionError({
    message: 'expected connectWith(...) to reject, but it resolved',
  });
}

describe('Client.connectWith options contract', () => {
  it('rejects an empty option set with no auth field', async () => {
    const message = await rejectionMessage(Client.connectWith({}));
    assert.match(message, /\[ConfigError\]/);
    assert.match(message, NO_AUTH_RE);
  });

  it('wires apiKey + email + password (conflict rejects)', async () => {
    // apiKey alongside the email/password pair is a conflict, so all three
    // fields must reach the resolver by name for the conflict to be seen.
    // A dropped field would drop the conflict and change the outcome.
    const message = await rejectionMessage(
      Client.connectWith({ apiKey: 'td1_dummy_key', email: 'you@example.com', password: 'secret' }),
    );
    assert.match(message, /\[ConfigError\]/);
    assert.match(message, /conflicting authentication fields/);
  });

  it('wires email without password (incomplete pair rejects)', async () => {
    // email alone (no password) is the email/password method with a
    // missing half, proving both `email` and `password` are wired: a
    // dropped `email` would instead surface "no authentication field set".
    const message = await rejectionMessage(
      Client.connectWith({ email: 'you@example.com' }),
    );
    assert.match(message, /\[ConfigError\]/);
    assert.doesNotMatch(message, NO_AUTH_RE);
    assert.match(message, /email\/password/);
  });

  it('wires and parses historicalType (bad selector rejects)', async () => {
    // A valid single auth field plus a bogus historicalType reaches the env
    // parse step, so the rejection is the historicalType parse error, proving
    // historicalType is both wired and parsed. A dropped historicalType would let the
    // call past validation into a network-class failure instead.
    const message = await rejectionMessage(
      Client.connectWith({ apiKey: 'td1_dummy_key', historicalType: 'BOGUS' }),
    );
    assert.match(message, /\[ConfigError\]/);
    assert.match(message, /historicalType/);
  });

  it('accepts apiKeyFromEnv by name (strict env source)', async () => {
    // With THETADATA_API_KEY unset, apiKeyFromEnv reaches the strict env
    // source and fails because the var is absent, NOT "no authentication
    // field set". A dropped/renamed field would be ignored by napi and the
    // option set would resolve to zero auth fields, flipping the message.
    const prev = process.env.THETADATA_API_KEY;
    delete process.env.THETADATA_API_KEY;
    try {
      const message = await rejectionMessage(
        Client.connectWith({ apiKeyFromEnv: true }),
      );
      assert.doesNotMatch(message, NO_AUTH_RE);
    } finally {
      if (prev !== undefined) process.env.THETADATA_API_KEY = prev;
    }
  });

  it('accepts apiKeyFromDotenv by name (missing file rejects, not "no auth")', async () => {
    const message = await rejectionMessage(
      Client.connectWith({ apiKeyFromDotenv: '/nonexistent/connect-with.env' }),
    );
    assert.doesNotMatch(message, NO_AUTH_RE);
  });

  it('accepts credentialsFile by name (missing file rejects, not "no auth")', async () => {
    const message = await rejectionMessage(
      Client.connectWith({ credentialsFile: '/nonexistent/creds.txt' }),
    );
    assert.doesNotMatch(message, NO_AUTH_RE);
  });
});
