// Config per-channel environment readback getters - TypeScript binding smoke test.
//
// The production / stage / dev presets select each channel's target cluster;
// the `marketDataEnvironment` / `streamingEnvironment` getters read those
// selections back as `"PROD"` / `"STAGE"` / `"DEV"` strings, mirroring the
// `marketDataType` / `streamingType` selectors the inline `Client.connectWith`
// factory accepts. The two channels are selected independently.
import { spawnSync } from 'node:child_process';
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

test('a bad environment selector throws from each preset instead of aborting', () => {
  // Run in a child process: before the fix the preset panicked across the
  // native boundary and aborted the whole process, test runner included.
  const index = new URL('../index.js', import.meta.url).href;
  const script = `
    const { Config } = await import(${JSON.stringify(index)});
    for (const preset of ['production', 'stage', 'dev']) {
      try {
        Config[preset]();
        console.log(preset + ': returned');
      } catch (e) {
        console.log(preset + ': ' + e.message);
      }
    }`;
  const child = spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    env: { ...process.env, THETADATA_MARKET_DATA_TYPE: 'production' },
    encoding: 'utf8',
  });
  assert.equal(child.status, 0, child.stderr);
  for (const preset of ['production', 'stage', 'dev']) {
    assert.match(child.stdout, new RegExp(`^${preset}: .*THETADATA_MARKET_DATA_TYPE`, 'm'));
  }
});
