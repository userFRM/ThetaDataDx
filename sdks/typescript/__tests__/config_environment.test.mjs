// Config.environment readback getter - TypeScript binding smoke test.
//
// The production / stage presets select the target cluster as a unit; the
// `environment` getter reads that selection back as a `"PROD"` / `"STAGE"`
// string, mirroring the `historicalType` selector the inline `Client.connectWith`
// factory accepts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Config } from '../index.js';

test('production() reads back PROD', () => {
  const cfg = Config.production();
  assert.equal(cfg.environment, 'PROD');
});

test('stage() reads back STAGE', () => {
  const cfg = Config.stage();
  assert.equal(cfg.environment, 'STAGE');
});
