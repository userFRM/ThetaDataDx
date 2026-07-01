// Connection-resilience knobs on `Config` — TypeScript binding parity
// with Python / C++ / FFI. Pins defaults and setter/getter round-trips
// for the reconnect cadence ladder, jitter, budgets + wall-clock
// envelope, replay pacing, the streaming transport knobs, the historical
// retry envelope, the flatfile jitter toggle, and the custom reconnect
// callback registration.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Config } from "../index.js";

test("reconnect ladder defaults and round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.reconnectWaitMs, 250n);
  assert.equal(cfg.reconnectWaitMaxMs, 30_000n);
  assert.equal(cfg.reconnectWaitRateLimitedMs, 130_000n);
  assert.equal(cfg.reconnectWaitServerRestartMs, 5_000n);
  cfg.setReconnectWaitMs(100n);
  cfg.setReconnectWaitMaxMs(60_000n);
  cfg.setReconnectWaitServerRestartMs(2_500n);
  assert.equal(cfg.reconnectWaitMs, 100n);
  assert.equal(cfg.reconnectWaitMaxMs, 60_000n);
  assert.equal(cfg.reconnectWaitServerRestartMs, 2_500n);
});

test("reconnect jitter round-trips and rejects unknown", () => {
  const cfg = Config.production();
  assert.equal(cfg.reconnectJitter, "full");
  for (const mode of ["equal", "DECORRELATED", "none", "Full"]) {
    cfg.setReconnectJitter(mode);
    assert.equal(cfg.reconnectJitter, mode.toLowerCase());
  }
  assert.throws(() => cfg.setReconnectJitter("gaussian"), /full/);
});

test("reconnect budgets and envelope round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.reconnectPolicy, "auto");
  assert.equal(cfg.reconnectMaxAttempts, 30);
  assert.equal(cfg.reconnectMaxRateLimitedAttempts, 100);
  assert.equal(cfg.reconnectMaxServerRestartAttempts, 60);
  assert.equal(cfg.reconnectMaxElapsedSecs, 300n);
  assert.equal(cfg.reconnectStableWindowSecs, 60n);
  cfg.setReconnectMaxServerRestartAttempts(5);
  assert.equal(cfg.reconnectMaxServerRestartAttempts, 5);
  cfg.setReconnectMaxElapsedSecs(0n); // disables the envelope
  assert.equal(cfg.reconnectMaxElapsedSecs, 0n);
});

test("reconnect replay pacing round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.reconnectReplayBurstSize, 50);
  assert.equal(cfg.reconnectReplayPaceMs, 5n);
  cfg.setReconnectReplayBurstSize(200);
  cfg.setReconnectReplayPaceMs(0n);
  assert.equal(cfg.reconnectReplayBurstSize, 200);
  assert.equal(cfg.reconnectReplayPaceMs, 0n);
});

test("reconnect callback registration switches policy", () => {
  const cfg = Config.production();
  assert.equal(cfg.reconnectPolicy, "auto");
  cfg.setReconnectCallback(() => 1_000);
  assert.equal(cfg.reconnectPolicy, "custom");
  cfg.setReconnectCallback(null);
  assert.equal(cfg.reconnectPolicy, "auto");
});

test("streaming transport defaults and round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.streamingTimeoutMs, 3_000n);
  assert.equal(cfg.streamingConnectTimeoutMs, 2_000n);
  assert.equal(cfg.streamingPingIntervalMs, 250n);
  assert.equal(cfg.streamingRingSize, 131_072n);
  assert.equal(cfg.streamingIoReadSliceMs, 25n);
  assert.equal(cfg.streamingKeepaliveIdleSecs, 5n);
  assert.equal(cfg.streamingKeepaliveIntervalSecs, 2n);
  assert.equal(cfg.streamingKeepaliveRetries, 2);
  cfg.setStreamingTimeoutMs(10_000n);
  cfg.setStreamingKeepaliveIdleSecs(10n);
  assert.equal(cfg.streamingTimeoutMs, 10_000n);
  assert.equal(cfg.streamingKeepaliveIdleSecs, 10n);
});

test("streaming ring size round-trips via BigInt and rejects non-power-of-two", () => {
  const cfg = Config.production();
  cfg.setStreamingRingSize(8_192n);
  assert.equal(cfg.streamingRingSize, 8_192n);
  // A slot count beyond the legacy 32-bit width round-trips losslessly.
  cfg.setStreamingRingSize(4_294_967_296n);
  assert.equal(cfg.streamingRingSize, 4_294_967_296n);
  assert.throws(() => cfg.setStreamingRingSize(5_000n), /power of two/);
  assert.equal(cfg.streamingRingSize, 4_294_967_296n, "rejected value leaves config unchanged");
});

test("streaming host selection round-trips and rejects unknown", () => {
  const cfg = Config.production();
  assert.equal(cfg.streamingHostSelection, "shuffled");
  cfg.setStreamingHostSelection("fixed_order");
  assert.equal(cfg.streamingHostSelection, "fixed_order");
  cfg.setStreamingHostSelection("SHUFFLED");
  assert.equal(cfg.streamingHostSelection, "shuffled");
  assert.throws(() => cfg.setStreamingHostSelection("round_robin"), /shuffled/);
});

test("streaming host shuffle seed round-trips the null sentinel", () => {
  const cfg = Config.production();
  assert.equal(cfg.streamingHostShuffleSeed, null);
  cfg.setStreamingHostShuffleSeed(42n);
  assert.equal(cfg.streamingHostShuffleSeed, 42n);
  cfg.setStreamingHostShuffleSeed(null);
  assert.equal(cfg.streamingHostShuffleSeed, null);
});

test("retry envelope defaults and round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.retryMaxAttempts, 20);
  assert.equal(cfg.retryMaxElapsedSecs, 300n);
  cfg.setRetryMaxElapsedSecs(0n); // disables the envelope
  assert.equal(cfg.retryMaxElapsedSecs, 0n);
});

test("flatfiles budget defaults and jitter round-trip", () => {
  const cfg = Config.production();
  assert.equal(cfg.flatfilesMaxAttempts, 10);
  assert.equal(cfg.flatfilesMaxBackoffSecs, 30n);
  assert.equal(cfg.flatfilesJitter, true);
  cfg.setFlatfilesJitter(false);
  assert.equal(cfg.flatfilesJitter, false);
});
