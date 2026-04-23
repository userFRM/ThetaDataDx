//! End-to-end latency benchmark for the pure-Rust FPSS path.
//!
//! Measures the time from `received_at_ns` (captured in the I/O thread at
//! frame decode) to the moment the user's callback sees the event on the
//! Disruptor consumer thread. This is the floor — no Python, no numpy, no
//! extra FFI hop — so it sets the upper bound on throughput the SDK can
//! deliver per connection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tdbe::types::enums::SecType;
use thetadatadx::auth::Credentials;
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::ThetaDataDx;

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| {
            d.as_secs()
                .saturating_mul(1_000_000_000)
                .saturating_add(u64::from(d.subsec_nanos()))
        })
        .unwrap_or(0)
}

fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let creds_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "creds.txt".into());
    let duration_secs: u64 = std::env::var("DUR")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let sec_type_str = std::env::var("SEC").unwrap_or_else(|_| "OPTION".into());
    let sec = match sec_type_str.to_uppercase().as_str() {
        "STOCK" => SecType::Stock,
        "OPTION" => SecType::Option,
        "INDEX" => SecType::Index,
        _ => SecType::Option,
    };

    let creds = Credentials::from_file(std::path::Path::new(&creds_path))?;
    let cfg = DirectConfig::dev();

    let total = Arc::new(AtomicU64::new(0));
    let trades = Arc::new(AtomicU64::new(0));
    // Sample latencies for data events only.
    let lat_samples: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::with_capacity(1_000_000)));

    let t2 = Arc::clone(&total);
    let tr2 = Arc::clone(&trades);
    let ls2 = Arc::clone(&lat_samples);

    let tdx = ThetaDataDx::connect(&creds, cfg).await?;
    tdx.start_streaming(move |ev: &FpssEvent| {
        let observed_ns = now_ns();
        t2.fetch_add(1, Ordering::Relaxed);
        match ev {
            FpssEvent::Data(FpssData::Quote { received_at_ns, .. })
            | FpssEvent::Data(FpssData::Trade { received_at_ns, .. })
            | FpssEvent::Data(FpssData::Ohlcvc { received_at_ns, .. })
            | FpssEvent::Data(FpssData::OpenInterest { received_at_ns, .. }) => {
                if matches!(ev, FpssEvent::Data(FpssData::Trade { .. })) {
                    tr2.fetch_add(1, Ordering::Relaxed);
                }
                let lat = observed_ns.saturating_sub(*received_at_ns);
                if let Ok(mut v) = ls2.lock() {
                    if v.len() < v.capacity() {
                        v.push(lat);
                    }
                }
            }
            _ => {}
        }
    })?;

    tdx.subscribe_full_trades(sec)?;

    let start = Instant::now();
    let mut last_print = start;
    let mut last_n = 0u64;
    loop {
        let el = start.elapsed();
        if el.as_secs() >= duration_secs {
            break;
        }
        std::thread::sleep(Duration::from_millis(1000));
        let now = Instant::now();
        let n = total.load(Ordering::Relaxed);
        let t = trades.load(Ordering::Relaxed);
        let dt = (now - last_print).as_secs_f64();
        let inst_rate = (n - last_n) as f64 / dt;
        let avg_rate = n as f64 / el.as_secs_f64();
        println!(
            "  t={:3}s total={:>10} trades={:>9}  instant={:>7.0}/s  avg={:>7.0}/s",
            el.as_secs(),
            n,
            t,
            inst_rate,
            avg_rate
        );
        last_print = now;
        last_n = n;
    }

    let el = start.elapsed().as_secs_f64();
    let n = total.load(Ordering::Relaxed);
    let t = trades.load(Ordering::Relaxed);

    let mut samples = {
        let v = lat_samples.lock().unwrap();
        v.clone()
    };
    samples.sort_unstable();

    println!();
    println!("=== END-TO-END LATENCY (received_at_ns -> callback observed) ===");
    println!("  samples:  {}", samples.len());
    if !samples.is_empty() {
        let fmt = |ns: u64| {
            if ns < 10_000 {
                format!("{ns} ns")
            } else if ns < 10_000_000 {
                format!("{:.1} us", ns as f64 / 1_000.0)
            } else {
                format!("{:.2} ms", ns as f64 / 1_000_000.0)
            }
        };
        println!("  min:      {}", fmt(samples[0]));
        println!("  p50:      {}", fmt(percentile(&samples, 0.50)));
        println!("  p90:      {}", fmt(percentile(&samples, 0.90)));
        println!("  p99:      {}", fmt(percentile(&samples, 0.99)));
        println!("  p99.9:    {}", fmt(percentile(&samples, 0.999)));
        println!("  max:      {}", fmt(samples[samples.len() - 1]));
    }
    println!();
    println!("=== THROUGHPUT ===");
    println!(
        "  events={n} trades={t} in {el:.1}s  rate={:.0}/s",
        n as f64 / el
    );

    tdx.stop_streaming();
    Ok(())
}
