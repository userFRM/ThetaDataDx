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
//! day, the format-pluggable writer is verified for all three formats
//! (Parquet / JSONL re-encode the same logical rows).

// Test gate sits on each #[test] via `cfg_attr(not(feature="live-tests"),
// ignore)`. Without `--features live-tests`, the live-MDDS integration
// case shows up as `ignored` in `cargo test` output instead of running.

use std::path::PathBuf;

use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

const REFERENCE_CSV: &str =
    "/home/theta-gamma/ThetaData/ThetaTerminal/downloads/OPTION-OPEN_INTEREST-20260428.csv";
const TEST_DATE: &str = "20260428";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg_attr(not(feature = "live-tests"), ignore = "live MDDS — opt in with --features live-tests")]
async fn option_open_interest_csv_byte_matches_vendor() {
    let reference = PathBuf::from(REFERENCE_CSV);
    if !reference.exists() {
        eprintln!(
            "reference vendor CSV not present at {} — skipping byte-match test",
            reference.display()
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

    let path = thetadatadx::flatfile_request(
        &creds,
        SecType::Option,
        ReqType::OpenInterest,
        TEST_DATE,
        &out,
        FlatFileFormat::Csv,
    )
    .await
    .expect("flatfile_request must succeed against live MDDS");
    assert_eq!(path, out);

    let ours = std::fs::read(&out).expect("read SDK CSV");
    let theirs = std::fs::read(&reference).expect("read vendor CSV");
    assert_eq!(
        ours.len(),
        theirs.len(),
        "byte length mismatch: SDK={} vendor={}",
        ours.len(),
        theirs.len()
    );
    assert_eq!(ours, theirs, "CSV content does not byte-match vendor");
}
