//! Byte-match the SDK's CSV output against the vendor jar's
//! whole-universe CSV for the same `(sec, req, date)`.
//!
//! The vendor reference file used here was produced by the legacy
//! ThetaTerminal jar at `~/ThetaData/ThetaTerminal/downloads/`. If the
//! file is absent the test is skipped — environments without the
//! vendor terminal locally can still run the rest of the test suite.
//!
//! This is the load-bearing decoder regression: if our CSV byte-matches
//! the vendor's output for every contract on a 1.8 M-row whole-universe
//! day, the row-by-row decode is verified end-to-end. The JSONL sink
//! re-encodes the same logical rows and is row-count smoke-tested.

// Test gate sits on each #[test] via `cfg_attr(not(feature="live-tests"),
// ignore)`. Without `--features live-tests`, the live-MDDS integration
// case shows up as `ignored` in `cargo test` output instead of running.

use std::path::PathBuf;

use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

// Reference vendor CSV path. Override with THETADATADX_REFERENCE_CSV when
// running on a different machine or vendor-jar layout.
const DEFAULT_REFERENCE_CSV: &str = "OPTION-OPEN_INTEREST-20260428.csv";
const DEFAULT_REFERENCE_EOD_CSV: &str = "OPTION-EOD-20260428.csv";
const TEST_DATE: &str = "20260428";

/// Resolved reference path plus a flag telling the test whether the
/// caller explicitly set the env var. When the env var is set but the
/// file is missing, the test must FAIL — silently skipping would let
/// CI report a green check while validating nothing.
struct ReferencePath {
    path: PathBuf,
    explicit: bool,
}

fn reference_csv_path() -> ReferencePath {
    if let Ok(p) = std::env::var("THETADATADX_REFERENCE_CSV") {
        return ReferencePath {
            path: PathBuf::from(p),
            explicit: true,
        };
    }
    ReferencePath {
        path: PathBuf::from(DEFAULT_REFERENCE_CSV),
        explicit: false,
    }
}

fn reference_eod_csv_path() -> ReferencePath {
    if let Ok(p) = std::env::var("THETADATADX_REFERENCE_EOD_CSV") {
        return ReferencePath {
            path: PathBuf::from(p),
            explicit: true,
        };
    }
    ReferencePath {
        path: PathBuf::from(DEFAULT_REFERENCE_EOD_CSV),
        explicit: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(
    not(feature = "live-tests"),
    ignore = "live MDDS — opt in with --features live-tests"
)]
async fn option_open_interest_csv_byte_matches_vendor() {
    // Install rustls' ring CryptoProvider exactly once. Tests build their
    // own binary so the provider isn't installed yet; idempotent failure
    // (already-installed) is fine and quietly ignored.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let reference = reference_csv_path();
    if !reference.path.exists() {
        if reference.explicit {
            panic!(
                "THETADATADX_REFERENCE_CSV={} is set but the file is missing. \
                 Provision the vendor reference CSV at that path or unset the \
                 env var so the test skips silently.",
                reference.path.display()
            );
        }
        eprintln!(
            "reference vendor CSV not present at {} — skipping byte-match test \
             (set THETADATADX_REFERENCE_CSV to provision)",
            reference.path.display()
        );
        return;
    }
    let creds = Credentials::from_file("creds.txt")
        .or_else(|_| Credentials::from_file("../../creds.txt"))
        .expect("creds.txt must be reachable");
    let out_dir = std::env::temp_dir().join("thetadatadx-flatfiles-test");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("OPTION-OPEN_INTEREST-20260428.csv");
    if out.exists() {
        let _ = std::fs::remove_file(&out);
    }

    // Pull once via raw, then decode the same blob into both formats.
    // A single live FLAT_FILE call covers the CSV byte-match plus the
    // JSONL row-count smoke test.
    let raw = out_dir.join("OPTION-OPEN_INTEREST-20260428.bin");
    if raw.exists() {
        let _ = std::fs::remove_file(&raw);
    }
    let raw_path = thetadatadx::flatfile_request_raw(
        &creds,
        SecType::Option,
        ReqType::OpenInterest,
        TEST_DATE,
        &raw,
    )
    .await
    .expect("raw flatfile pull must succeed");
    assert_eq!(raw_path, raw);

    // Decode CSV from the saved blob.
    let csv_path = out;
    thetadatadx::flatfiles::decoded_decode_to_file_for_test(
        &raw,
        SecType::Option,
        &csv_path,
        FlatFileFormat::Csv,
    )
    .expect("CSV decode");
    let ours = std::fs::read(&csv_path).expect("read SDK CSV");
    let theirs = std::fs::read(&reference.path).expect("read vendor CSV");
    assert_eq!(
        ours.len(),
        theirs.len(),
        "byte length mismatch: SDK={} vendor={}",
        ours.len(),
        theirs.len()
    );
    assert_eq!(ours, theirs, "CSV content does not byte-match vendor");

    // Smoke-test JSONL on the same blob; assert row count equals CSV
    // rows minus the header.
    let csv_rows = ours
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        .saturating_sub(1);

    let jsonl_path = out_dir.join("OPTION-OPEN_INTEREST-20260428.jsonl");
    thetadatadx::flatfiles::decoded_decode_to_file_for_test(
        &raw,
        SecType::Option,
        &jsonl_path,
        FlatFileFormat::Jsonl,
    )
    .expect("JSONL decode");
    let jsonl_bytes = std::fs::read(&jsonl_path).expect("read jsonl");
    let jsonl_rows = jsonl_bytes.iter().filter(|&&b| b == b'\n').count();
    assert_eq!(
        jsonl_rows, csv_rows,
        "JSONL row count {jsonl_rows} != CSV row count {csv_rows}"
    );

    eprintln!(
        "byte-match OK: {csv_rows} rows, csv={} bytes, jsonl={} bytes",
        ours.len(),
        jsonl_bytes.len(),
    );

    // Cleanup raw blob — it's large.
    let _ = std::fs::remove_file(&raw);
}

/// Price-bearing byte-match for OPTION/EOD on a known business day.
///
/// EOD rows carry `bid` / `ask` / `price` columns, so this exercises the
/// CSV price formatter end-to-end against the vendor's reference output —
/// the OPEN_INTEREST byte-match test does not (open-interest is integer-
/// only).
///
/// When the reference CSV is missing, this test skips. To regenerate the
/// fixture: run the legacy ThetaTerminal jar's daily-flatfile download for
/// the same `(SecType, ReqType, date)` and copy the CSV to the path
/// pointed to by `THETADATADX_REFERENCE_EOD_CSV`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(
    not(feature = "live-tests"),
    ignore = "live MDDS — opt in with --features live-tests"
)]
async fn option_eod_csv_byte_matches_vendor() {
    // Install rustls' ring CryptoProvider exactly once. Tests build their
    // own binary so the provider isn't installed yet; idempotent failure
    // (already-installed) is fine and quietly ignored.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let reference = reference_eod_csv_path();
    if !reference.path.exists() {
        if reference.explicit {
            panic!(
                "THETADATADX_REFERENCE_EOD_CSV={} is set but the file is missing. \
                 Provision the vendor reference EOD CSV at that path or unset \
                 the env var so the test skips silently.",
                reference.path.display()
            );
        }
        eprintln!(
            "reference vendor EOD CSV not present at {} — skipping byte-match test \
             (set THETADATADX_REFERENCE_EOD_CSV to provision)",
            reference.path.display()
        );
        return;
    }
    let creds = Credentials::from_file("creds.txt")
        .or_else(|_| Credentials::from_file("../../creds.txt"))
        .expect("creds.txt must be reachable");
    let out_dir = std::env::temp_dir().join("thetadatadx-flatfiles-test");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("OPTION-EOD-20260428.csv");
    if out.exists() {
        let _ = std::fs::remove_file(&out);
    }

    // Pull once via raw, then decode the same blob into both formats.
    // A single live FLAT_FILE call covers the CSV byte-match plus the
    // JSONL row-count smoke test.
    let raw = out_dir.join("OPTION-EOD-20260428.bin");
    if raw.exists() {
        let _ = std::fs::remove_file(&raw);
    }
    let raw_path =
        thetadatadx::flatfile_request_raw(&creds, SecType::Option, ReqType::Eod, TEST_DATE, &raw)
            .await
            .expect("raw flatfile pull must succeed");
    assert_eq!(raw_path, raw);

    // Decode CSV from the saved blob.
    let csv_path = out;
    thetadatadx::flatfiles::decoded_decode_to_file_for_test(
        &raw,
        SecType::Option,
        &csv_path,
        FlatFileFormat::Csv,
    )
    .expect("CSV decode");
    let ours = std::fs::read(&csv_path).expect("read SDK CSV");
    let theirs = std::fs::read(&reference.path).expect("read vendor CSV");
    assert_eq!(
        ours.len(),
        theirs.len(),
        "byte length mismatch: SDK={} vendor={}",
        ours.len(),
        theirs.len()
    );
    assert_eq!(ours, theirs, "EOD CSV content does not byte-match vendor");

    // Smoke-test JSONL on the same blob; assert row count equals CSV
    // rows minus the header.
    let csv_rows = ours
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        .saturating_sub(1);

    let jsonl_path = out_dir.join("OPTION-EOD-20260428.jsonl");
    thetadatadx::flatfiles::decoded_decode_to_file_for_test(
        &raw,
        SecType::Option,
        &jsonl_path,
        FlatFileFormat::Jsonl,
    )
    .expect("JSONL decode");
    let jsonl_bytes = std::fs::read(&jsonl_path).expect("read jsonl");
    let jsonl_rows = jsonl_bytes.iter().filter(|&&b| b == b'\n').count();
    assert_eq!(
        jsonl_rows, csv_rows,
        "JSONL row count {jsonl_rows} != CSV row count {csv_rows}"
    );

    eprintln!(
        "byte-match OK (EOD): {csv_rows} rows, csv={} bytes, jsonl={} bytes",
        ours.len(),
        jsonl_bytes.len(),
    );

    // Cleanup raw blob — it's large.
    let _ = std::fs::remove_file(&raw);
}
