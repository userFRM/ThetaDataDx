//! Whole-universe FLATFILES download CLI.
//!
//! Pulls one full vendor flat file for `(sec_type, data_type, date)` and
//! writes it in the requested format.
//!
//! Usage:
//!     cargo run --release --example flatfile_demo -- \
//!         <sec> <data_type> <date> <out_path> <format>
//!
//! Args:
//!   sec        OPTION | STOCK | INDEX
//!   data_type  EOD | QUOTE | TRADE | TRADE_QUOTE | OPEN_INTEREST | OHLC
//!   date       YYYYMMDD (e.g. 20260428)
//!   out_path   destination path; the format extension is appended if absent
//!   format     CSV | PARQUET | JSONL
//!
//! Credentials are loaded from `$CREDS` (default `./creds.txt`).

use std::path::PathBuf;
use std::process::ExitCode;

use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

fn parse_sec(s: &str) -> Result<SecType, String> {
    match s.to_ascii_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(format!("unknown sec_type {other:?}")),
    }
}

fn parse_req(s: &str) -> Result<ReqType, String> {
    match s.to_ascii_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" => Ok(ReqType::TradeQuote),
        "OPEN_INTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        other => Err(format!("unknown data_type {other:?}")),
    }
}

fn parse_format(s: &str) -> Result<FlatFileFormat, String> {
    match s.to_ascii_uppercase().as_str() {
        "CSV" => Ok(FlatFileFormat::Csv),
        "PARQUET" => Ok(FlatFileFormat::Parquet),
        "JSONL" => Ok(FlatFileFormat::Jsonl),
        other => Err(format!("unknown format {other:?}")),
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    // Workspace pulls multiple rustls crypto-provider candidates (ring via
    // `rustls = ["ring"]` and aws-lc-rs through some transitive dep). Pick
    // one explicitly so rustls' process-default resolver doesn't panic at
    // first TLS connect. ring is what `rustls`'s feature flag in this
    // workspace selects.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!(
            "usage: {} <sec> <data_type> <date> <out_path> <format>",
            args.first().map(String::as_str).unwrap_or("flatfile_demo")
        );
        eprintln!("       sec: OPTION | STOCK | INDEX");
        eprintln!("       data_type: EOD | QUOTE | TRADE | TRADE_QUOTE | OPEN_INTEREST | OHLC");
        eprintln!("       format: CSV | PARQUET | JSONL");
        return ExitCode::from(2);
    }
    let sec = match parse_sec(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let req = match parse_req(&args[2]) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let date = &args[3];
    let out: PathBuf = PathBuf::from(&args[4]);
    let format = match parse_format(&args[5]) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let creds_path: PathBuf = std::env::var("CREDS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("creds.txt"));
    let creds = match Credentials::from_file(&creds_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("creds load failed ({}): {e}", creds_path.display());
            return ExitCode::from(1);
        }
    };

    let started = std::time::Instant::now();
    eprintln!(
        "flatfile {sec} {req:?} {date} -> {} ({format})",
        out.display()
    );
    match thetadatadx::flatfile_request(&creds, sec, req, date, &out, format).await {
        Ok(p) => {
            eprintln!(
                "OK {:.1}s -> {} ({} bytes)",
                started.elapsed().as_secs_f64(),
                p.display(),
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("FAILED {:.1}s: {e}", started.elapsed().as_secs_f64());
            ExitCode::FAILURE
        }
    }
}
