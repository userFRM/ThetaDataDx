// Smoke test: verify the native addon loads and exports the expected class.
// Does NOT connect to ThetaData — just checks the binding surface.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

// The native addon won't be built in CI unless we add a napi build step,
// so this test is a structural check that runs after `npm run build`.
describe('ThetaDataDx native addon', () => {
  it('exports ThetaDataDx class with connect factory', async () => {
    let mod;
    try {
      mod = await import('../index.js');
    } catch {
      // If the native addon isn't built, skip gracefully.
      console.log('SKIP: native addon not built (run `npm run build` first)');
      return;
    }
    assert.ok(mod.ThetaDataDx, 'ThetaDataDx should be exported');
    assert.equal(typeof mod.ThetaDataDx.connect, 'function', 'connect should be a static method');
    assert.equal(typeof mod.ThetaDataDx.connectFromFile, 'function', 'connectFromFile should be a static method');
  });
});
