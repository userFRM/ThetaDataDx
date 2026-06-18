// API-key credential construction + redaction — TypeScript binding smoke test.
//
// Mirrors the email + password constructor coverage: build credentials
// through every API-key factory and confirm `toString` never exposes the
// key or the email.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// CI build step is mandatory before `npm test`; fail loud if the addon
// is missing so a broken build does not appear green.
let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('Credentials API-key factories', () => {
  it('builds credentials from an API key', () => {
    const creds = mod.Credentials.fromApiKey('super-secret-key');
    assert.ok(creds, 'fromApiKey should return a Credentials handle');
  });

  it('builds credentials from an API key paired with an email', () => {
    const creds = mod.Credentials.fromApiKeyWithEmail(
      'user@example.com',
      'super-secret-key',
    );
    assert.ok(creds, 'fromApiKeyWithEmail should return a Credentials handle');
  });

  it('sources credentials from the environment, falling back to a file', () => {
    process.env.THETADATA_API_KEY = 'env-sourced-key';
    try {
      const creds = mod.Credentials.fromEnvOrFile('/nonexistent/creds.txt');
      assert.ok(creds, 'fromEnvOrFile should source the API key from the env');
    } finally {
      delete process.env.THETADATA_API_KEY;
    }
  });

  it('falls back to the file when the env is unset', () => {
    delete process.env.THETADATA_API_KEY;
    // No fallback file exists, so the file path must surface an error
    // rather than silently building a handle.
    assert.throws(
      () => mod.Credentials.fromEnvOrFile('/nonexistent/creds.txt'),
      'fromEnvOrFile should throw when neither the env nor the file is available',
    );
  });

  it('redacts the API key in toString', () => {
    const creds = mod.Credentials.fromApiKey('super-secret-key');
    const rendered = creds.toString();
    assert.ok(
      !rendered.includes('super-secret-key'),
      `toString leaked the API key: ${rendered}`,
    );
    assert.ok(
      rendered.includes('<redacted>'),
      `toString missing the redaction marker: ${rendered}`,
    );
  });

  it('redacts both the email and the API key in toString', () => {
    const creds = mod.Credentials.fromApiKeyWithEmail(
      'user@example.com',
      'super-secret-key',
    );
    const rendered = creds.toString();
    assert.ok(!rendered.includes('super-secret-key'), 'toString leaked the API key');
    assert.ok(!rendered.includes('user@example.com'), 'toString leaked the email');
    assert.ok(rendered.includes('<redacted>'));
  });
});
