// Smoke test: verify the native addon loads and exports the expected class.
// Does NOT connect to ThetaData — just checks the binding surface.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// CI build step is mandatory before `npm test`; fail loud if the addon
// is missing so a broken build is not silently green.
let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

describe('ThetaDataDxClient native addon', () => {
  it('exports ThetaDataDxClient class with connect factory', () => {
    assert.ok(mod.ThetaDataDxClient, 'ThetaDataDxClient should be exported');
    assert.equal(typeof mod.ThetaDataDxClient.connect, 'function', 'connect should be a static method');
    assert.equal(typeof mod.ThetaDataDxClient.connectFromFile, 'function', 'connectFromFile should be a static method');
  });
});
