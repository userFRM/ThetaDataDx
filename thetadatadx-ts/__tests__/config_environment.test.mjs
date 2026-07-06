// Config per-channel environment readback getters - TypeScript binding smoke test.
//
// The production / stage / dev presets select each channel's target cluster;
// the `marketDataEnvironment` / `streamingEnvironment` getters read those
// selections back as `"PROD"` / `"STAGE"` / `"DEV"` strings, mirroring the
// `marketDataType` / `streamingType` selectors the inline `Client.connectWith`
// factory accepts. The two channels are selected independently.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Config } from '../index.js';

test('production() reads back PROD on both channels', () => {
  const cfg = Config.production();
  assert.equal(cfg.marketDataEnvironment, 'PROD');
  assert.equal(cfg.streamingEnvironment, 'PROD');
});

test('stage() selects market-data STAGE and leaves streaming on PROD', () => {
  const cfg = Config.stage();
  assert.equal(cfg.marketDataEnvironment, 'STAGE');
  assert.equal(cfg.streamingEnvironment, 'PROD');
});

test('dev() selects streaming DEV and leaves market-data on PROD', () => {
  const cfg = Config.dev();
  assert.equal(cfg.marketDataEnvironment, 'PROD');
  assert.equal(cfg.streamingEnvironment, 'DEV');
});
