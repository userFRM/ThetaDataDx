// Full-chain option quote streaming benchmark -- ported from
// `thetadatadx-rs/examples/chain_quote_bench.rs` (see that file for the
// semantics this mirrors).
//
// Pulls an entire option chain's quote history over the streaming
// market-data endpoint and reports TTFB, throughput, and an approximate
// in-memory decoded volume so the effect of the h2 flow-control window
// sizes can be measured against the live backend.
//
// Run with:
//     npx tsx examples/chain_quote_bench.ts [symbol] [expiration] [date] [interval]
//
// Args (all optional):
//   symbol      option root (default SPXW)
//   expiration  contract expiration YYYYMMDD (default 20260710)
//   date        history date YYYYMMDD (default = expiration, i.e. a 0DTE pull)
//   interval    tick | 1s | 1m | ... (default tick)
//
// The h2 windows are set on the config before connect via
// STREAM_WINDOW_SIZE_KB / CONNECTION_WINDOW_SIZE_KB below -- edit those
// constants and rerun to benchmark different values (both are clamped into
// [64, 2_097_151] KB at connect). This binding has no post-connect
// (validated) config read-back, so the constants themselves are printed as
// the "effective" values every run.
//
// Credentials are loaded from $CREDS (default ./creds.txt).

import { Config, Credentials, MarketDataClient } from "thetadatadx-ts";

const USAGE = `usage: chain_quote_bench.ts [symbol] [expiration] [date] [interval]
       defaults: SPXW 20260710 <expiration> tick
       date defaults to <expiration> (a 0DTE full-chain pull)
       h2 windows come from the STREAM_WINDOW_SIZE_KB / CONNECTION_WINDOW_SIZE_KB
       constants in this file; edit and rerun to test different values
       credentials from $CREDS (default ./creds.txt)`;

// h2 flow-control windows applied to Config's `setMarketDataStreamWindowSizeKb`
// / `setMarketDataConnectionWindowSizeKb` before connect. Edit and rerun to
// benchmark different values; validate/connect clamps both into
// [64, 2_097_151] KB.
const STREAM_WINDOW_SIZE_KB = 8_192;
// const STREAM_WINDOW_SIZE_KB = 64;
const CONNECTION_WINDOW_SIZE_KB = 32_768;
// const CONNECTION_WINDOW_SIZE_KB = 64;

const MIB = 1024.0 * 1024.0;

// Decoded in-memory size of a single parsed quote row. The streaming
// callback only exposes `QuoteTick[]`, so decoded volume is approximated as
// `rows * QUOTE_TICK_SIZE` -- an in-memory figure, not wire bytes. Mirrors
// `std::mem::size_of::<QuoteTick>()` in the Rust example (measured 128
// bytes: `#[repr(C, align(64))]`, 13 fields) so figures are comparable
// across bindings.
const QUOTE_TICK_SIZE = 128;

function argOr(args: string[], idx: number, fallback: string): string {
  return args[idx] ?? fallback;
}

function humanBytes(n: number): string {
  const gib = MIB * 1024.0;
  if (n >= gib) {
    return `${(n / gib).toFixed(2)} GiB`;
  }
  return `${(n / MIB).toFixed(2)} MiB`;
}

function nowSecs(): bigint {
  return process.hrtime.bigint();
}

function secsSince(start: bigint): number {
  return Number(nowSecs() - start) / 1e9;
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  if (args.some((a) => a === "-h" || a === "--help")) {
    console.log(USAGE);
    return;
  }
  if (args.length > 4) {
    console.error(USAGE);
    process.exit(2);
  }

  const symbol = argOr(args, 0, "SPXW");
  const expiration = argOr(args, 1, "20260710");
  const date = argOr(args, 2, expiration);
  const interval = argOr(args, 3, "tick");

  const credsPath = process.env.CREDS ?? "creds.txt";
  let creds: Credentials;
  try {
    creds = Credentials.fromFile(credsPath);
  } catch (e) {
    console.error(`creds load failed (${credsPath}): ${e}`);
    process.exit(1);
  }

  // production() supplies the defaults; the benchmark constants override the
  // h2 window knobs before connect, which clamps the applied values into
  // [64, 2_097_151] KB via validate.
  const config = Config.production();
  config.setMarketDataStreamWindowSizeKb(BigInt(STREAM_WINDOW_SIZE_KB));
  config.setMarketDataConnectionWindowSizeKb(BigInt(CONNECTION_WINDOW_SIZE_KB));

  const connectStart = nowSecs();
  let client: MarketDataClient;
  try {
    client = await MarketDataClient.connect(creds, config);
  } catch (e) {
    console.error(`connect failed: ${e}`);
    process.exit(1);
  }
  const connectAuthSecs = secsSince(connectStart);

  // Effective h2 window sizes, so every run is self-documenting. No
  // post-connect (validated) config read-back exists on this binding, so
  // the constants are printed as-is; validate clamps both into
  // [64, 2_097_151] KB at connect.
  const streamWindowSizeKb = STREAM_WINDOW_SIZE_KB;
  const connectionWindowSizeKb = CONNECTION_WINDOW_SIZE_KB;
  console.error(
    `[bench] effective h2 windows: stream=${streamWindowSizeKb} KB, ` +
      `connection=${connectionWindowSizeKb} KB`,
  );
  console.error(
    `[bench] streaming option_history_quote ${symbol} exp=${expiration} date=${date} ` +
      `interval=${interval} strike=* right=both (no deadline)`,
  );

  let rows = 0;
  let chunks = 0;
  let ttfbSecs: number | null = null;
  let lastLog = nowSecs();

  // Dispatch clock: started immediately before dispatching the stream call
  // so TTFB excludes connect/auth and measures backend-to-first-chunk latency.
  const dispatch = nowSecs();

  // A full-day 0DTE pull can run 6-15 minutes; the config default
  // requestTimeoutSecs (300 s) would kill it, so opt out of any deadline
  // with timeoutMs: 0 (documented as "no deadline").
  try {
    await client.optionHistoryQuoteStream(
      symbol,
      expiration,
      { strike: "*", right: "both", date, interval, timeoutMs: 0 },
      (chunk) => {
        const now = nowSecs();
        if (ttfbSecs === null) {
          ttfbSecs = secsSince(dispatch);
        }
        rows += chunk.length;
        chunks += 1;
        // Lightweight liveness so a 10-minute pull is not silent.
        if (secsSince(lastLog) >= 10.0) {
          lastLog = now;
          const secs = Math.max(secsSince(dispatch), Number.EPSILON);
          const approxMib = (rows * QUOTE_TICK_SIZE) / MIB;
          console.error(
            `[bench] +${secs.toFixed(0).padStart(6, " ")}s rows=${rows} chunks=${chunks} ` +
              `~${approxMib.toFixed(2)} MiB in-mem (${(approxMib / secs).toFixed(2)} MiB/s)`,
          );
        }
      },
    );
  } catch (e) {
    const total = secsSince(dispatch);
    console.error(`stream failed after ${total.toFixed(1)}s: ${e}`);
    process.exit(1);
  }

  const total = secsSince(dispatch);
  const secs = Math.max(total, Number.EPSILON);
  const finalTtfbSecs = ttfbSecs ?? 0.0;
  // Approximate decoded VOLUME (in-memory, not wire): the streaming callback
  // exposes only parsed rows, so multiply the row count by the decoded row
  // size. This is a lower bound on RSS (ignores per-row heap) and is
  // unrelated to the compressed bytes that crossed the h2 window.
  const approxDecoded = rows * QUOTE_TICK_SIZE;

  // Greppable key=value block on stdout; progress/logs stay on stderr.
  console.log(`symbol=${symbol}`);
  console.log(`expiration=${expiration}`);
  console.log(`date=${date}`);
  console.log(`interval=${interval}`);
  console.log(`stream_window_size_kb=${streamWindowSizeKb}`);
  console.log(`connection_window_size_kb=${connectionWindowSizeKb}`);
  console.log(`connect_auth_secs=${connectAuthSecs.toFixed(3)}`);
  console.log(`ttfb_secs=${finalTtfbSecs.toFixed(3)}`);
  console.log(`total_secs=${secs.toFixed(3)}`);
  console.log(`rows=${rows}`);
  console.log(`chunks=${chunks}`);
  console.log(`rows_per_sec=${(rows / secs).toFixed(1)}`);
  console.log(`quote_tick_size_bytes=${QUOTE_TICK_SIZE}`);
  console.log(`approx_decoded_bytes=${approxDecoded}`);
  console.log(`approx_decoded=${humanBytes(approxDecoded)}`);
  console.log(`approx_decoded_bytes_per_sec=${(approxDecoded / secs).toFixed(0)}`);
  console.log(`approx_rate_mib_per_sec=${(approxDecoded / secs / MIB).toFixed(2)}`);
  console.log(
    `# approx_decoded* is in-memory volume = rows x QUOTE_TICK_SIZE ` +
      `(${QUOTE_TICK_SIZE} B); not wire bytes`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
