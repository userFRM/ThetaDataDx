//! Byte-match the SDK's CSV output against the vendor jar's
//! whole-universe CSV for the same `(sec, req, date)`.
//!
//! The vendor reference files used here were produced by the legacy
//! ThetaTerminal jar at `~/ThetaData/ThetaTerminal/downloads/`. CI's
//! steady state does not check those fixtures into the repo (they're
//! gigabyte-scale option-day CSVs), so the test gates the whole
//! byte-match contract on a single env var:
//!
//! - Set `THETADATADX_FLATFILE_FIXTURES_PATH` to a directory containing
//!   `OPTION-OPEN_INTEREST-20260428.csv` and `OPTION-EOD-20260428.csv`
//!   to run the full byte-match.
//! - Leave it unset, or point it at a path that doesn't exist, to skip
//!   (test passes as skipped). One `eprintln!` documents the skip so
//!   CI operators can find the contract on demand.
//!
//! Wave-6 closure: the prior gate panicked when the default fixture
//! filename was absent under `--features live-tests`, which is the CI
//! steady state — the test was therefore a hard failure rather than a
//! conditional gate. The single-env-var path now makes the contract
//! consistent: either provide fixtures and validate end-to-end, or
//! skip.
//!
//! This is the load-bearing decoder regression: when fixtures are
//! provisioned, byte-matching our CSV against the vendor's output for
//! every contract on a 1.8 M-row whole-universe day verifies row-by-
//! row decode end-to-end. The JSONL sink re-encodes the same logical
//! rows and is row-count smoke-tested.

// Test gate sits on each #[test] via `cfg_attr(not(feature="live-tests"),
// ignore)`. Without `--features live-tests`, the live-MDDS integration
// case shows up as `ignored` in `cargo test` output instead of running.

use std::path::PathBuf;

use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

/// Fixture directory env var. Single source of truth for both byte-
/// match tests — point at the directory holding the vendor reference
/// CSVs, or leave unset to skip both tests.
const FIXTURES_PATH_ENV: &str = "THETADATADX_FLATFILE_FIXTURES_PATH";

const REFERENCE_CSV_FILENAME: &str = "OPTION-OPEN_INTEREST-20260428.csv";
const REFERENCE_EOD_CSV_FILENAME: &str = "OPTION-EOD-20260428.csv";
const TEST_DATE: &str = "20260428";

/// Resolved reference fixture for one of the byte-match tests.
///
/// Returns `None` (and prints a single skip diagnostic) when
/// [`FIXTURES_PATH_ENV`] is unset, the directory doesn't exist, or the
/// expected fixture filename is absent inside it. Callers treat a
/// `None` result as the test passing as skipped.
fn resolve_fixture(filename: &str) -> Option<PathBuf> {
    let dir = match std::env::var(FIXTURES_PATH_ENV) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "skipping flatfile_byte_match: {FIXTURES_PATH_ENV} unset \
                 (set it to a directory containing {REFERENCE_CSV_FILENAME} / \
                 {REFERENCE_EOD_CSV_FILENAME} to enable byte-match validation)"
            );
            return None;
        }
    };
    let dir_path = PathBuf::from(&dir);
    if !dir_path.exists() {
        eprintln!(
            "skipping flatfile_byte_match: {FIXTURES_PATH_ENV}={dir} does not exist on disk \
             (provision the vendor fixtures and rerun to enable byte-match validation)"
        );
        return None;
    }
    let fixture = dir_path.join(filename);
    if !fixture.exists() {
        eprintln!(
            "skipping flatfile_byte_match: fixture {} missing from {FIXTURES_PATH_ENV}={dir} \
             (provision it to enable byte-match validation)",
            fixture.display(),
        );
        return None;
    }
    Some(fixture)
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

    let Some(reference_path) = resolve_fixture(REFERENCE_CSV_FILENAME) else {
        // Skip path is documented inside `resolve_fixture` via a
        // single `eprintln!`. The test passes as skipped — matching
        // the workflow contract that fixtures are an opt-in input.
        return;
    };
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
    let theirs = std::fs::read(&reference_path).expect("read vendor CSV");
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
/// When the reference CSV is missing, this test skips. To regenerate
/// the fixture: run the legacy ThetaTerminal jar's daily-flatfile
/// download for the same `(SecType, ReqType, date)` and drop the CSV
/// into the directory pointed to by
/// `THETADATADX_FLATFILE_FIXTURES_PATH`.
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

    let Some(reference_path) = resolve_fixture(REFERENCE_EOD_CSV_FILENAME) else {
        // Skip path is documented inside `resolve_fixture`. The test
        // passes as skipped when the EOD fixture is absent, matching
        // the open-interest gate.
        return;
    };
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
    let theirs = std::fs::read(&reference_path).expect("read vendor CSV");
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
