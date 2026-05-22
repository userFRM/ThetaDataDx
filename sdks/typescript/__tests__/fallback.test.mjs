// REST-fallback surface tests.
//
// Offline tests pin the factory + getter contracts on `FallbackPolicy`
// and `Config.withRestFallback`. Live tests behind
// `THETADX_LIVE_PATCHED_TERMINAL` exercise the actual h2-cascade
// recovery path against a patched local Terminal -- mirrors the gating
// on the Python `tests/test_rest_fallback.py` live suite.
import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.log('SKIP: native addon not built (run `npm run build` first)');
  process.exit(0);
}

const { FallbackPolicy, Config, ThetaDataDxClient, DEFAULT_REST_BASE_URL } = mod;

describe('FallbackPolicy factories', () => {
  it('exports the FallbackPolicy class with the four named factories', () => {
    assert.ok(FallbackPolicy, 'FallbackPolicy should be exported');
    assert.equal(typeof FallbackPolicy.disabled, 'function');
    assert.equal(typeof FallbackPolicy.restOnH2Disconnect, 'function');
    assert.equal(typeof FallbackPolicy.restAlwaysForDateRange, 'function');
    assert.equal(typeof FallbackPolicy.restAlways, 'function');
  });

  it('exports DEFAULT_REST_BASE_URL as the local Terminal default', () => {
    assert.equal(typeof DEFAULT_REST_BASE_URL, 'string');
    assert.match(DEFAULT_REST_BASE_URL, /^http:\/\/127\.0\.0\.1:25503/);
  });

  it('disabled() carries the Disabled variant + no base URL', () => {
    const p = FallbackPolicy.disabled();
    assert.equal(p.variant, 'Disabled');
    assert.equal(p.baseUrl, null);
  });

  it('restOnH2Disconnect() carries the supplied base URL', () => {
    const p = FallbackPolicy.restOnH2Disconnect(DEFAULT_REST_BASE_URL);
    assert.equal(p.variant, 'RestOnH2Disconnect');
    assert.equal(p.baseUrl, DEFAULT_REST_BASE_URL);
  });

  it('restAlwaysForDateRange() round-trips the YYYYMMDD cutoff', () => {
    const p = FallbackPolicy.restAlwaysForDateRange(DEFAULT_REST_BASE_URL, 20230101);
    assert.equal(p.variant, 'RestAlwaysForDateRange');
    assert.equal(p.baseUrl, DEFAULT_REST_BASE_URL);
  });

  it('restAlways() carries the supplied base URL', () => {
    const p = FallbackPolicy.restAlways(DEFAULT_REST_BASE_URL);
    assert.equal(p.variant, 'RestAlways');
    assert.equal(p.baseUrl, DEFAULT_REST_BASE_URL);
  });
});

describe('Config.withRestFallback', () => {
  it('exports the Config class with the three factories + withRestFallback', () => {
    assert.ok(Config, 'Config should be exported');
    assert.equal(typeof Config.production, 'function');
    assert.equal(typeof Config.dev, 'function');
    assert.equal(typeof Config.stage, 'function');
    const cfg = Config.production();
    assert.equal(typeof cfg.withRestFallback, 'function');
    assert.equal(cfg.fallbackVariant, 'Disabled');
  });

  it('installs Disabled / RestOnH2Disconnect / RestAlwaysForDateRange / RestAlways', () => {
    const cfg = Config.production();
    cfg.withRestFallback(FallbackPolicy.disabled());
    assert.equal(cfg.fallbackVariant, 'Disabled');

    cfg.withRestFallback(FallbackPolicy.restOnH2Disconnect(DEFAULT_REST_BASE_URL));
    assert.equal(cfg.fallbackVariant, 'RestOnH2Disconnect');

    cfg.withRestFallback(FallbackPolicy.restAlwaysForDateRange(DEFAULT_REST_BASE_URL, 20230101));
    assert.equal(cfg.fallbackVariant, 'RestAlwaysForDateRange');

    cfg.withRestFallback(FallbackPolicy.restAlways(DEFAULT_REST_BASE_URL));
    assert.equal(cfg.fallbackVariant, 'RestAlways');
  });
});

describe('ThetaDataDxClient _with_fallback method surface', () => {
  it('exports the connectWithConfig + connectFromFileWithConfig factories', () => {
    assert.equal(typeof ThetaDataDxClient.connectWithConfig, 'function');
    assert.equal(typeof ThetaDataDxClient.connectFromFileWithConfig, 'function');
  });

  it('exports all four optionHistory*WithFallback methods on the prototype', () => {
    const proto = ThetaDataDxClient.prototype;
    assert.equal(typeof proto.optionHistoryQuoteWithFallback, 'function');
    assert.equal(typeof proto.optionHistoryTradeQuoteWithFallback, 'function');
    assert.equal(typeof proto.optionHistoryGreeksImpliedVolatilityWithFallback, 'function');
    assert.equal(typeof proto.optionHistoryGreeksFirstOrderWithFallback, 'function');
  });
});

// Live end-to-end suite against the patched local Terminal. Mirrors
// `sdks/python/tests/test_rest_fallback.py::test_option_history_quote_with_fallback_live`.
const liveEnabled = !!process.env.THETADX_LIVE_PATCHED_TERMINAL;
describe('FallbackPolicy live (gated on THETADX_LIVE_PATCHED_TERMINAL)', () => {
  if (!liveEnabled) {
    it('skipped (set THETADX_LIVE_PATCHED_TERMINAL=1 to enable)', () => {});
    return;
  }
  it('optionHistoryQuoteWithFallback recovers from h2 cascade on 2022-era rows', async () => {
    const email = process.env.THETADX_EMAIL;
    const password = process.env.THETADX_PASSWORD;
    if (!email || !password) {
      throw new Error('THETADX_EMAIL / THETADX_PASSWORD must be set for live tests');
    }
    const cfg = Config.production();
    cfg.withRestFallback(FallbackPolicy.restAlways(DEFAULT_REST_BASE_URL));
    const tdx = ThetaDataDxClient.connectWithConfig(email, password, cfg);
    const ticks = await tdx.optionHistoryQuoteWithFallback(
      'QQQ',
      '20220422',
      '20220414',
      undefined,
      '305',
      'P',
      '60000',
    );
    assert.ok(Array.isArray(ticks), 'fallback path should return a tick array');
  });
});
