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
//!   sec        OPTION | STOCK
//!   data_type  EOD | TRADE_QUOTE | OPEN_INTEREST (OPEN_INTEREST is OPTION only)
//!   date       YYYYMMDD (e.g. 20260428)
//!   out_path   destination path; the format extension is appended if absent
//!   format     CSV | JSON | JSONL | HTML
//!
//! Credentials are loaded from `$CREDS` (default `./creds.txt`).

use std::path::PathBuf;
use std::process::ExitCode;

use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    // Pin ring as the single rustls `CryptoProvider` via the standard
    // `install_default` path. The workspace builds rustls/tokio-rustls
    // with `default-features = false, features = ["ring", ...]`, so ring
    // is the only provider compiled in; this call seats it as the
    // process-wide default before the first TLS handshake.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!(
            "usage: {} <sec> <data_type> <date> <out_path> <format>",
            args.first().map(String::as_str).unwrap_or("flatfile_demo")
        );
        eprintln!("       sec: OPTION | STOCK");
        eprintln!(
            "       data_type: EOD | TRADE_QUOTE | OPEN_INTEREST (OPEN_INTEREST is OPTION only)"
        );
        eprintln!("       format: CSV | JSON | JSONL | HTML");
        return ExitCode::from(2);
    }
    let sec = match args[1].parse::<SecType>() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let req = match args[2].parse::<ReqType>() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let date = &args[3];
    let out: PathBuf = PathBuf::from(&args[4]);
    let format = match args[5].parse::<FlatFileFormat>() {
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
