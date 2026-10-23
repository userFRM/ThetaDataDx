//! Byte-match the SDK's CSV output against the JVM terminal's
//! whole-universe CSV for the same `(sec, req, date)`.
//!
//! The reference files used here were captured from the JVM terminal
//! (Theta Terminal) for the same query. CI does not check the fixtures
//! into the repo (they are gigabyte-scale option-day CSVs); the test
//! gates on a single env var:
//!
//! - Set `THETADATADX_FLATFILE_FIXTURES_PATH` to a directory containing
//!   `OPTION-OPEN_INTEREST-20260428.csv` and `OPTION-EOD-20260428.csv`
//!   to run the full byte-match.
//! - Leave it unset, or point it at a path that doesn't exist, to skip
//!   (the test passes as skipped).
//!
//! Once the env var points at an existing directory, every named
//! fixture inside must exist — a missing CSV at that point is a hard
//! failure. The JSONL sink re-encodes the same logical rows and is
//! row-count smoke-tested.
//!
//! The comparison is column-for-column rather than a raw byte compare:
//! the SDK emits option strikes in dollars on every client-facing
//! surface, while the vendor reference renders the raw scaled-integer
//! wire value (1000 wire units per dollar). That one column is compared
//! on a common scale; every other byte must still match exactly.

// Test gate sits on each #[test] via `cfg_attr(not(feature="live-tests"),
// ignore)`. Without `--features live-tests`, the live-MDDS integration
// case shows up as `ignored` in `cargo test` output instead of running.

use std::path::PathBuf;

use thetadatadx::Credentials;
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};

/// Fixture directory env var. Single source of truth for both byte-
/// match tests — point at the directory holding the vendor reference
/// CSVs, or leave unset to skip both tests.
const FIXTURES_PATH_ENV: &str = "THETADATADX_FLATFILE_FIXTURES_PATH";

const REFERENCE_CSV_FILENAME: &str = "OPTION-OPEN_INTEREST-20260428.csv";
const REFERENCE_EOD_CSV_FILENAME: &str = "OPTION-EOD-20260428.csv";
const TEST_DATE: &str = "20260428";

/// Locate a named fixture under [`FIXTURES_PATH_ENV`]. `None` (the test
/// skips) when the variable is unset or names a directory that does not
/// exist; panics when the directory exists but the fixture is missing.
fn resolve_fixture(filename: &str) -> Option<PathBuf> {
    let Ok(dir) = std::env::var(FIXTURES_PATH_ENV) else {
        eprintln!(
            "skipping flatfile_byte_match: {FIXTURES_PATH_ENV} unset (set it to a directory \
             containing {REFERENCE_CSV_FILENAME} / {REFERENCE_EOD_CSV_FILENAME} to enable \
             byte-match validation)",
        );
        return None;
    };
    let dir = PathBuf::from(dir);
    if !dir.exists() {
        eprintln!(
            "skipping flatfile_byte_match: {FIXTURES_PATH_ENV}={} does not exist on disk \
             (provision the vendor fixtures and rerun to enable byte-match validation)",
            dir.display(),
        );
        return None;
    }
    let fixture = dir.join(filename);
    assert!(
        fixture.exists(),
        "flatfile_byte_match: opt-in fixture {} is missing from {FIXTURES_PATH_ENV}={} \
         (provision it or unset {FIXTURES_PATH_ENV} to skip)",
        fixture.display(),
        dir.display(),
    );
    Some(fixture)
}

/// Zero-based index of the `strike` column in the option CSV layout
/// (`symbol,expiration,strike,right,...`). The contract prefix the
/// vendor prepends to every option row shares this layout.
const OPTION_STRIKE_COLUMN: usize = 2;

/// Assert the SDK CSV matches the vendor CSV column-for-column, treating
/// the strike column as semantically equal across the two scales.
///
/// The strike column is the one cell that legitimately differs: the SDK
/// emits strikes in dollars (`$210.00`), while the vendor reference
/// renders the raw wire integer (1000 wire units per dollar, so `210000`
/// for a $210 strike). The two encode the same strike at different
/// scales, so a byte-equal comparison of that column would always fail.
/// Every other byte must still match exactly, so each row is compared
/// field-by-field with the strike cell converted to a common scale
/// (vendor wire units / 1000 == SDK dollars) and all remaining cells
/// compared verbatim. The header row matches byte-for-byte and is
/// compared whole.
fn assert_option_csv_matches_vendor(ours: &[u8], theirs: &[u8], context: &str) {
    let ours = std::str::from_utf8(ours).expect("SDK CSV is UTF-8");
    let theirs = std::str::from_utf8(theirs).expect("vendor CSV is UTF-8");
    let our_lines: Vec<&str> = ours.lines().collect();
    let their_lines: Vec<&str> = theirs.lines().collect();
    assert_eq!(
        our_lines.len(),
        their_lines.len(),
        "{context}: row count mismatch SDK={} vendor={}",
        our_lines.len(),
        their_lines.len()
    );

    for (row_idx, (our_line, their_line)) in our_lines.iter().zip(&their_lines).enumerate() {
        // Row 0 is the header; both sides write the identical column
        // names, so compare it whole.
        if row_idx == 0 {
            assert_eq!(our_line, their_line, "{context}: header row differs");
            continue;
        }
        let our_cells: Vec<&str> = our_line.split(',').collect();
        let their_cells: Vec<&str> = their_line.split(',').collect();
        assert_eq!(
            our_cells.len(),
            their_cells.len(),
            "{context}: column count mismatch at row {row_idx}: SDK={our_line:?} vendor={their_line:?}"
        );
        for (col, (our_cell, their_cell)) in our_cells.iter().zip(&their_cells).enumerate() {
            if col == OPTION_STRIKE_COLUMN {
                let our_strike: f64 = our_cell
                    .parse()
                    .unwrap_or_else(|_| panic!("{context}: SDK strike {our_cell:?} not numeric"));
                let their_wire: f64 = their_cell.parse().unwrap_or_else(|_| {
                    panic!("{context}: vendor strike {their_cell:?} not numeric")
                });
                // 1000 wire units per dollar; SDK dollars are wire /
                // 1000. Compare on the dollar scale.
                let their_dollars = their_wire / 1_000.0;
                assert!(
                    (our_strike - their_dollars).abs() < 1e-9,
                    "{context}: strike mismatch at row {row_idx}: SDK={our_strike} vendor={their_wire} (={their_dollars} dollars)"
                );
            } else {
                assert_eq!(
                    our_cell, their_cell,
                    "{context}: cell mismatch at row {row_idx} col {col}: SDK={our_cell:?} vendor={their_cell:?}"
                );
            }
        }
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

    let Some(reference_path) = resolve_fixture(REFERENCE_CSV_FILENAME) else {
        // Skip path is documented inside `resolve_fixture` via a
        // single `eprintln!`. The test passes as skipped — matching
        // the workflow contract that fixtures are an opt-in input.
        return;
    };
    let creds = Credentials::from_file("creds.txt")
        .or_else(|_| Credentials::from_file("../creds.txt"))
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
    assert_option_csv_matches_vendor(&ours, &theirs, "OPTION/OPEN_INTEREST");

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
/// the fixture: capture the JVM terminal's daily-flatfile download for
/// the same `(SecType, ReqType, date)` and drop the CSV into the
/// directory pointed to by `THETADATADX_FLATFILE_FIXTURES_PATH`.
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
        .or_else(|_| Credentials::from_file("../creds.txt"))
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
    assert_option_csv_matches_vendor(&ours, &theirs, "OPTION/EOD");

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
