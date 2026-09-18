//! Measure real stream throughput: messages per second and bytes per row.
//!
//! Runs against the dev replay cluster by default, which replays a full market
//! day at real rate whatever the wall clock says, so the numbers are open-of-
//! market numbers without waiting for the open.
//!
//! cargo run --release --example stream_rate -- <creds.txt> [seconds] [prod|dev] [option|stock] [interval]
//!
//! Every argument after the credentials is positional and optional, but
//! they are taken in that order: sampling length in seconds, which cluster,
//! which market, and how often to print a line. Defaults: 60 seconds, the dev
//! replay cluster, the option market, a line every 5 seconds.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thetadatadx::streaming::{SecTypeExt, StreamData, StreamEvent};
use thetadatadx::{Client, Credentials, DirectConfig, SecType, WaitMode};

static COUNT: AtomicU64 = AtomicU64::new(0);
static PER_SEC: Mutex<BTreeMap<u64, u64>> = Mutex::new(BTreeMap::new());
/// 100ms buckets. A ring overflows on a microburst, not on a mean, and a
/// per-second average cannot see one.
static PER_100MS: Mutex<BTreeMap<u128, u64>> = Mutex::new(BTreeMap::new());
/// First and last `ms_of_day` seen, to check the replay runs at real time.
static MS_SPAN: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// The exchange timestamp on a row, whatever shape it arrived in.
fn ms_of_day(data: &StreamData) -> Option<i32> {
    Some(match data {
        StreamData::Quote { ms_of_day, .. }
        | StreamData::Trade { ms_of_day, .. }
        | StreamData::OpenInterest { ms_of_day, .. } => *ms_of_day,
        // The vendor's bar is stamped at its session boundary, not when it
        // arrives: a stock feed carries bars marked 04:00:00.000 and
        // 20:00:00.000 all session, which would stretch the span to the
        // whole extended day whatever the run actually covered.
        _ => return None,
    })
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// UTC wall clock, so a long run's log lines locate themselves in time. UTC
/// and not a session offset: which venue is trading, and whether it keeps a
/// session at all, is not something this example should assume.
fn wall_clock() -> String {
    let s = now_s() % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn clock(ms: i32) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let creds_path = args
        .next()
        .expect("usage: stream_rate <creds.txt> [secs] [prod]");
    // Every argument is checked rather than defaulted: a typo that quietly
    // selected another cluster or another market would produce numbers that
    // look right and describe something else.
    let secs: u64 = match args.next() {
        None => 60,
        Some(a) => a
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("seconds must be a whole number of seconds, one or more")?,
    };
    let prod = match args.next().as_deref() {
        None | Some("dev") => false,
        Some("prod") => true,
        Some(other) => return Err(format!("cluster must be prod or dev, not {other}").into()),
    };
    let sec_type = match args.next().as_deref() {
        None | Some("option") => SecType::Option,
        Some("stock") => SecType::Stock,
        Some(other) => return Err(format!("market must be option or stock, not {other}").into()),
    };
    let interval: u64 = match args.next() {
        None => 5,
        Some(a) => a
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .ok_or("interval must be a whole number of seconds, one or more")?,
    }
    .min(secs);

    let creds = Credentials::from_file(&creds_path)?;
    let mut config = if prod {
        DirectConfig::production()
    } else {
        DirectConfig::dev()
    };
    // The consumer defaults to a spin wait, which holds a whole core for as
    // long as the stream is connected. Nothing here needs that: this counts
    // messages, it does not chase microseconds. Backoff spins while they
    // arrive and sleeps when they stop, so a busy market is measured at full
    // speed and a quiet one costs nothing. `dropped` is printed on every
    // line and is the check: if the consumer ever fell behind, it would
    // show there rather than quietly flattening the numbers.
    config.streaming.wait_mode = WaitMode::Backoff;
    let client = Client::connect(&creds, config).await?;

    client.stream().start_streaming(|event: &StreamEvent| {
        if let StreamEvent::Data(data) = event {
            COUNT.fetch_add(1, Ordering::Relaxed);
            *PER_SEC.lock().unwrap().entry(now_s()).or_insert(0) += 1;
            *PER_100MS.lock().unwrap().entry(now_ms() / 100).or_insert(0) += 1;
            if let Some(ms) = ms_of_day(data) {
                let mut span = MS_SPAN.lock().unwrap();
                match &mut *span {
                    Some((lo, hi)) => {
                        *lo = (*lo).min(ms);
                        *hi = (*hi).max(ms);
                    }
                    None => *span = Some((ms, ms)),
                }
            }
        }
    })?;

    // The whole option market: every print on every contract, the widest feed
    // the vendor sells.
    client.stream().subscribe(sec_type.full_trades())?;
    println!(
        "subscribed: full trades, {:?}, {} cluster. sampling {secs}s, every {interval}s...",
        sec_type,
        if prod { "prod" } else { "dev-replay" }
    );

    let start = Instant::now();
    let mut last = 0u64;
    let mut seen_buckets = 0usize;
    let deadline = Duration::from_secs(secs);
    while start.elapsed() < deadline {
        // Never sleep past the deadline: a short run would otherwise sample
        // for a whole interval and divide by the length that was asked for.
        let remaining = deadline.saturating_sub(start.elapsed());
        tokio::time::sleep(remaining.min(Duration::from_secs(interval))).await;
        let total = COUNT.load(Ordering::Relaxed);
        // Worst 100ms bucket since the previous line: the burst a ring must
        // survive, which a per-interval mean cannot show.
        let burst = {
            let b = PER_100MS.lock().unwrap();
            let fresh: Vec<u64> = b.values().skip(seen_buckets).copied().collect();
            seen_buckets = b.len();
            fresh.iter().copied().max().unwrap_or(0)
        };
        println!(
            "  {}  t+{:>5}s  {:>7}/s  burst={:>6}/s  total={:<11} dropped={}",
            wall_clock(),
            start.elapsed().as_secs(),
            (total - last) / interval,
            burst * 10,
            total,
            client.stream().dropped_event_count()
        );
        last = total;
    }

    let total = COUNT.load(Ordering::Relaxed);
    // Measured, not requested: the two differ by the last partial interval.
    let elapsed = start.elapsed().as_secs_f64().max(f64::MIN_POSITIVE);
    let buckets = PER_SEC.lock().unwrap().clone();
    // The first and last buckets are partial seconds; drop them.
    let mut rates: Vec<u64> = buckets.values().copied().collect();
    if rates.len() > 2 {
        rates.remove(0);
        rates.pop();
    }
    rates.sort_unstable();

    println!(
        "\n--- {elapsed:.0}s sample, full {sec_type:?} trade stream, {} ---",
        if prod { "PROD" } else { "dev-replay" }
    );
    println!("messages        : {total}");
    println!("mean msg/s      : {:.0}", total as f64 / elapsed);
    if !rates.is_empty() {
        println!("median msg/s    : {}", rates[rates.len() / 2]);
        println!("p95 msg/s       : {}", rates[rates.len() * 95 / 100]);
        println!("peak msg/s      : {}", rates[rates.len() - 1]);
    }
    if total > 0 {
        // The ring stores `StreamData` itself, so a row costs exactly this.
        let row = std::mem::size_of::<StreamData>() as f64;
        println!("bytes/row       : {row:.0}  (size_of::<StreamData>, what the ring holds)");
        println!(
            "peak MB/s       : {:.2}",
            rates.last().copied().unwrap_or(0) as f64 * row / 1e6
        );
        for mb in [8u64, 32, 128] {
            let rows = (mb as f64 * 1e6 / row) as u64;
            println!(
                "{mb:>4} MB holds   : {rows} rows = {:.1}s at peak",
                rows as f64 / rates.last().copied().unwrap_or(1).max(1) as f64
            );
        }
    }
    // Trim the partial buckets at each END OF THE RUN, then sort. Sorting
    // first would drop the largest bucket, which is the one this whole
    // harness exists to find.
    let mut bursts: Vec<u64> = PER_100MS.lock().unwrap().values().copied().collect();
    if bursts.len() > 2 {
        bursts.remove(0);
        bursts.pop();
        bursts.sort_unstable();
        println!(
            "\npeak 100ms      : {} rows  (= {} msg/s instantaneous)",
            bursts[bursts.len() - 1],
            bursts[bursts.len() - 1] * 10
        );
        println!("p99 100ms       : {} rows", bursts[bursts.len() * 99 / 100]);
    }
    if let Some((lo, hi)) = *MS_SPAN.lock().unwrap() {
        let span = (hi - lo) as f64 / 1000.0;
        println!(
            "exchange span   : {span:.0}s of market over {elapsed:.0}s wall  (x{:.2} real time)",
            span / elapsed
        );
        println!("market window   : {} -> {}", clock(lo), clock(hi));
    }
    println!(
        "feed dropped    : {}",
        client.stream().dropped_event_count()
    );
    Ok(())
}
