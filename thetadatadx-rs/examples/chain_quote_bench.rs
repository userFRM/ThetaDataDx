//! Full-chain option quote streaming benchmark.
//!
//! Pulls an entire option chain's quote history over the streaming market-data
//! endpoint and reports TTFB, throughput, and an approximate in-memory decoded
//! volume so the effect of the h2 flow-control window sizes can be measured
//! against the live backend.
//!
//! Usage:
//!     cargo run --release --example chain_quote_bench -- \
//!         [symbol] [expiration] [date] [interval]
//!
//! Args (all optional):
//!   symbol      option root (default SPXW)
//!   expiration  contract expiration YYYYMMDD (default 20260710)
//!   date        history date YYYYMMDD (default = expiration, i.e. a 0DTE pull)
//!   interval    tick | 1s | 1m | ... (default tick)
//!
//! The h2 windows are set on the config before connect via
//! [`STREAM_WINDOW_SIZE_KB`] / [`CONNECTION_WINDOW_SIZE_KB`] — edit those
//! constants and rebuild to benchmark different values (both are clamped
//! into `[64, 2_097_151]` KB by `DirectConfig::validate`, which
//! `MarketDataClient::connect` runs). The effective (validated) values are
//! printed on every run.
//!
//! Credentials are loaded from `$CREDS` (default `./creds.txt`).

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use thetadatadx::{Credentials, DirectConfig, MarketDataClient, QuoteTick};

const USAGE: &str = "\
usage: chain_quote_bench [symbol] [expiration] [date] [interval]
       defaults: SPXW 20260710 <expiration> tick
       date defaults to <expiration> (a 0DTE full-chain pull)
       h2 windows come from the STREAM_WINDOW_SIZE_KB / CONNECTION_WINDOW_SIZE_KB
       constants in this file; edit and rebuild to test different values
       credentials from $CREDS (default ./creds.txt)";

/// h2 flow-control windows applied to `market_data.stream_window_size_kb` /
/// `.connection_window_size_kb` before connect. Edit and rebuild to benchmark
/// different values; `DirectConfig::validate` clamps both into
/// `[64, 2_097_151]` KB at connect.
const STREAM_WINDOW_SIZE_KB: usize = 8_192;
// const STREAM_WINDOW_SIZE_KB: usize = 64;
const CONNECTION_WINDOW_SIZE_KB: usize = 32_768;
// const CONNECTION_WINDOW_SIZE_KB: usize = 64;

const MIB: f64 = 1024.0 * 1024.0;

/// Decoded in-memory size of a single parsed quote row. The `.stream()`
/// callback only exposes `&[QuoteTick]`, so decoded volume is approximated as
/// `rows * size_of::<QuoteTick>()` — an in-memory figure, not wire bytes.
const QUOTE_TICK_SIZE: usize = std::mem::size_of::<QuoteTick>();

/// Per-chunk state shared between the stream handler and the post-run report.
/// The handler is invoked once per chunk under the crate's internal mutex, so
/// a single `Mutex` here is uncontended.
struct Progress {
    rows: u64,
    chunks: u64,
    ttfb: Option<Duration>,
    last_log: Instant,
}

fn arg_or(args: &[String], idx: usize, default: &str) -> String {
    args.get(idx)
        .map(String::as_str)
        .unwrap_or(default)
        .to_string()
}

fn human_bytes(n: u64) -> String {
    let f = n as f64;
    const GIB: f64 = MIB * 1024.0;
    if f >= GIB {
        format!("{:.2} GiB", f / GIB)
    } else {
        format!("{:.2} MiB", f / MIB)
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    // Pin ring as the single rustls `CryptoProvider` via the standard
    // `install_default` path — the workspace compiles rustls with only the
    // `ring` provider, so this seats it before the first TLS handshake.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.len() > 5 {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let symbol = arg_or(&args, 1, "SPXW");
    let expiration = arg_or(&args, 2, "20260710");
    let date = arg_or(&args, 3, &expiration);
    let interval = arg_or(&args, 4, "tick");

    let creds_path = std::env::var("CREDS").unwrap_or_else(|_| "creds.txt".to_string());
    let creds = match Credentials::from_file(&creds_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("creds load failed ({creds_path}): {e}");
            return ExitCode::from(1);
        }
    };

    // `production()` supplies the defaults; the benchmark constants override
    // the h2 window knobs before connect, which clamps the applied values
    // into [64, 2_097_151] KB via `validate`.
    let mut config = DirectConfig::production();
    config.market_data.stream_window_size_kb = STREAM_WINDOW_SIZE_KB;
    config.market_data.connection_window_size_kb = CONNECTION_WINDOW_SIZE_KB;

    let connect_start = Instant::now();
    let client = match MarketDataClient::connect(&creds, config).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connect failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let connect_elapsed = connect_start.elapsed();

    // Effective (validated) h2 window sizes, so every run is self-documenting.
    let stream_window_size_kb = client.config().market_data.stream_window_size_kb;
    let connection_window_size_kb = client.config().market_data.connection_window_size_kb;
    eprintln!(
        "[bench] effective h2 windows: stream={stream_window_size_kb} KB, connection={connection_window_size_kb} KB"
    );
    eprintln!(
        "[bench] streaming option_history_quote {symbol} exp={expiration} date={date} \
         interval={interval} strike=* right=both (no deadline)"
    );

    let progress = Arc::new(Mutex::new(Progress {
        rows: 0,
        chunks: 0,
        ttfb: None,
        last_log: Instant::now(),
    }));

    // Dispatch clock: started immediately before awaiting the stream so TTFB
    // excludes connect/auth and measures backend-to-first-chunk latency.
    let dispatch = Instant::now();
    let handler_progress = progress.clone();
    // A full-day 0DTE pull can run 6-15 minutes; the config default
    // `request_timeout_secs` (300 s) would kill it, so opt out of any deadline
    // with `Duration::ZERO` (normalized to "no deadline" by `effective_deadline`).
    let stream_result = client
        .option_history_quote_stream(&symbol, &expiration)
        .date(&date)
        .strike("*")
        .right("both")
        .interval(&interval)
        .with_deadline(Duration::ZERO)
        .stream(move |ticks| {
            let mut p = handler_progress.lock().expect("progress mutex poisoned");
            if p.ttfb.is_none() {
                p.ttfb = Some(dispatch.elapsed());
            }
            p.rows += ticks.len() as u64;
            p.chunks += 1;
            // Lightweight liveness so a 10-minute pull is not silent.
            if p.last_log.elapsed() >= Duration::from_secs(10) {
                p.last_log = Instant::now();
                let secs = dispatch.elapsed().as_secs_f64().max(f64::EPSILON);
                let approx_mib = (p.rows * QUOTE_TICK_SIZE as u64) as f64 / MIB;
                eprintln!(
                    "[bench] +{secs:6.0}s rows={} chunks={} ~{approx_mib:.2} MiB in-mem ({:.2} MiB/s)",
                    p.rows,
                    p.chunks,
                    approx_mib / secs,
                );
            }
        })
        .await;
    let total = dispatch.elapsed();

    if let Err(e) = stream_result {
        eprintln!("stream failed after {:.1}s: {e}", total.as_secs_f64());
        return ExitCode::FAILURE;
    }

    let (rows, chunks, ttfb) = {
        let p = progress.lock().expect("progress mutex poisoned");
        (p.rows, p.chunks, p.ttfb.unwrap_or_default())
    };
    let secs = total.as_secs_f64().max(f64::EPSILON);
    // Approximate decoded VOLUME (in-memory, not wire): the public `.stream()`
    // callback exposes only parsed `&[QuoteTick]`, so multiply the row count by
    // the decoded row size. This is a lower bound on RSS (ignores per-row heap)
    // and is unrelated to the compressed bytes that crossed the h2 window.
    let approx_decoded = rows * QUOTE_TICK_SIZE as u64;

    // Greppable key=value block on stdout; progress/logs stay on stderr.
    println!("symbol={symbol}");
    println!("expiration={expiration}");
    println!("date={date}");
    println!("interval={interval}");
    println!("stream_window_size_kb={stream_window_size_kb}");
    println!("connection_window_size_kb={connection_window_size_kb}");
    println!("connect_auth_secs={:.3}", connect_elapsed.as_secs_f64());
    println!("ttfb_secs={:.3}", ttfb.as_secs_f64());
    println!("total_secs={secs:.3}");
    println!("rows={rows}");
    println!("chunks={chunks}");
    println!("rows_per_sec={:.1}", rows as f64 / secs);
    println!("quote_tick_size_bytes={QUOTE_TICK_SIZE}");
    println!("approx_decoded_bytes={approx_decoded}");
    println!("approx_decoded={}", human_bytes(approx_decoded));
    println!(
        "approx_decoded_bytes_per_sec={:.0}",
        approx_decoded as f64 / secs
    );
    println!(
        "approx_rate_mib_per_sec={:.2}",
        approx_decoded as f64 / secs / MIB
    );
    println!(
        "# approx_decoded* is in-memory volume = rows x size_of::<QuoteTick>() ({QUOTE_TICK_SIZE} B); not wire bytes"
    );

    ExitCode::SUCCESS
}
