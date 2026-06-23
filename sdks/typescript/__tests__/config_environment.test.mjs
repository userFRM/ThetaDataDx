// Config per-channel environment readback getters - TypeScript binding smoke test.
//
// The production / stage / dev presets select each channel's target cluster;
// the `historicalEnvironment` / `streamingEnvironment` getters read those
// selections back as `"PROD"` / `"STAGE"` / `"DEV"` strings, mirroring the
// `historicalType` / `streamingType` selectors the inline `Client.connectWith`
// factory accepts. The two channels are selected independently.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Config } from '../index.js';

test('production() reads back PROD on both channels', () => {
  const cfg = Config.production();
  assert.equal(cfg.historicalEnvironment, 'PROD');
  assert.equal(cfg.streamingEnvironment, 'PROD');
});

test('stage() selects historical STAGE and leaves streaming on PROD', () => {
  const cfg = Config.stage();
  assert.equal(cfg.historicalEnvironment, 'STAGE');
  assert.equal(cfg.streamingEnvironment, 'PROD');
});

test('dev() selects streaming DEV and leaves historical on PROD', () => {
  const cfg = Config.dev();
  assert.equal(cfg.historicalEnvironment, 'PROD');
  assert.equal(cfg.streamingEnvironment, 'DEV');
});
