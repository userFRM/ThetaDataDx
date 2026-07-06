// Config.fromDotenv — TypeScript binding smoke test.
//
// Sources the staging selection from a `.env` file and confirms the
// resulting config points the historical channel at the staging cluster,
// distinct from the production host a prod / api-key-only `.env` yields.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

function withDotenv(name, body, fn) {
  const dir = mkdtempSync(join(tmpdir(), 'tddx-cfg-dotenv-'));
  const path = join(dir, name);
  writeFileSync(path, body);
  try {
    return fn(path);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

describe('Config.fromDotenv', () => {
  it('selects the staging environment from a .env file', () => {
    withDotenv('stage.env', '# select staging\nTHETADATA_MARKET_DATA_TYPE=STAGE\n', (path) => {
      const cfg = mod.Config.fromDotenv(path);
      assert.ok(cfg, 'fromDotenv should return a Config handle');
      assert.equal(cfg.marketDataHost, 'mdds-stage.thetadata.us');
    });
  });

  it('keeps the production host when the .env carries only an API key', () => {
    withDotenv('apikey.env', 'THETADATA_API_KEY=td_example_key\n', (path) => {
      const cfg = mod.Config.fromDotenv(path);
      assert.equal(cfg.marketDataHost, 'mdds-01.thetadata.us');
    });
  });

  it('yields a different host for a stage .env than for a prod .env', () => {
    withDotenv('prod.env', 'THETADATA_MARKET_DATA_TYPE=PROD\n', (prodPath) => {
      withDotenv('stage2.env', 'THETADATA_MARKET_DATA_TYPE=STAGE\n', (stagePath) => {
        const prod = mod.Config.fromDotenv(prodPath);
        const stage = mod.Config.fromDotenv(stagePath);
        assert.notEqual(prod.marketDataHost, stage.marketDataHost);
      });
    });
  });
});
