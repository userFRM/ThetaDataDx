//! Byte-match the SDK's CSV output against the vendor jar's
//! whole-universe CSV for the same `(sec, req, date)`.
//!
//! The vendor reference files used here were produced by the legacy
//! ThetaTerminal jar at `~/ThetaData/ThetaTerminal/downloads/`. CI does
//! not check the fixtures into the repo (they are gigabyte-scale
//! option-day CSVs); the test gates on a single env var:
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

// Test gate sits on each #[test] via `cfg_attr(not(feature="live-tests"),
// ignore)`. Without `--features live-tests`, the live-MDDS integration
// case shows up as `ignored` in `cargo test` output instead of running.

use std::path::{Path, PathBuf};

use thetadatadx_engine::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx_engine::Credentials;

/// Fixture directory env var. Single source of truth for both byte-
/// match tests — point at the directory holding the vendor reference
/// CSVs, or leave unset to skip both tests.
const FIXTURES_PATH_ENV: &str = "THETADATADX_FLATFILE_FIXTURES_PATH";

const REFERENCE_CSV_FILENAME: &str = "OPTION-OPEN_INTEREST-20260428.csv";
const REFERENCE_EOD_CSV_FILENAME: &str = "OPTION-EOD-20260428.csv";
const TEST_DATE: &str = "20260428";

/// Result of resolving a named fixture under [`FIXTURES_PATH_ENV`].
#[derive(Debug, PartialEq, Eq)]
enum FixtureResolution {
    /// Env var unset or directory absent — test passes as skipped.
    Skipped(String),
    /// Opt-in dir exists and the named fixture is present.
    Provisioned(PathBuf),
    /// Opt-in dir exists but the named fixture is missing inside —
    /// hard failure.
    MissingInOptInDir {
        dir: PathBuf,
        missing: PathBuf,
        env_var: &'static str,
    },
}

/// Locate a named fixture under [`FIXTURES_PATH_ENV`]. Pure (no panic,
/// no `eprintln`) so the regression tests can drive every branch.
fn resolve_fixture_inner(filename: &str, env_var: &'static str) -> FixtureResolution {
    let Ok(dir) = std::env::var(env_var) else {
        return FixtureResolution::Skipped(format!(
            "{env_var} unset (set it to a directory containing \
             {REFERENCE_CSV_FILENAME} / {REFERENCE_EOD_CSV_FILENAME} \
             to enable byte-match validation)",
        ));
    };
    let dir_path = PathBuf::from(&dir);
    if !dir_path.exists() {
        return FixtureResolution::Skipped(format!(
            "{env_var}={dir} does not exist on disk \
             (provision the vendor fixtures and rerun to enable byte-match validation)",
        ));
    }
    let fixture = dir_path.join(filename);
    if !fixture.exists() {
        return FixtureResolution::MissingInOptInDir {
            dir: dir_path,
            missing: fixture,
            env_var,
        };
    }
    FixtureResolution::Provisioned(fixture)
}

/// Convert a [`FixtureResolution`] to the caller-facing
/// `Option<PathBuf>`. Panics when the operator opted in but a named
/// fixture is missing inside the opt-in directory.
fn resolution_to_option(result: FixtureResolution) -> Option<PathBuf> {
    match result {
        FixtureResolution::Skipped(reason) => {
            eprintln!("skipping flatfile_byte_match: {reason}");
            None
        }
        FixtureResolution::Provisioned(path) => Some(path),
        FixtureResolution::MissingInOptInDir {
            dir,
            missing,
            env_var,
        } => {
            panic!(
                "flatfile_byte_match: opt-in fixture {} is missing from {env_var}={} — \
                 provision it or unset {env_var} to skip",
                missing.display(),
                Path::new(&dir).display(),
            );
        }
    }
}

/// Resolve a fixture, mapping `Skipped` -> `None` and panicking on a
/// missing fixture inside an opt-in directory.
fn resolve_fixture(filename: &str) -> Option<PathBuf> {
    resolution_to_option(resolve_fixture_inner(filename, FIXTURES_PATH_ENV))
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
    let raw_path = thetadatadx_engine::flatfile_request_raw(
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
    thetadatadx_engine::flatfiles::decoded_decode_to_file_for_test(
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
    thetadatadx_engine::flatfiles::decoded_decode_to_file_for_test(
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
    let raw_path = thetadatadx_engine::flatfile_request_raw(
        &creds,
        SecType::Option,
        ReqType::Eod,
        TEST_DATE,
        &raw,
    )
    .await
    .expect("raw flatfile pull must succeed");
    assert_eq!(raw_path, raw);

    // Decode CSV from the saved blob.
    let csv_path = out;
    thetadatadx_engine::flatfiles::decoded_decode_to_file_for_test(
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
    thetadatadx_engine::flatfiles::decoded_decode_to_file_for_test(
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

// Fixture resolution unit tests. Each uses a unique env var name so
// parallel runs do not see one another's state.

#[cfg(test)]
mod fixture_resolution_tests {
    use super::{resolve_fixture_inner, FixtureResolution};
    use std::path::PathBuf;

    #[test]
    fn unset_env_var_is_skipped() {
        // Per-test env var name — never set, so the helper sees the
        // unset branch unconditionally.
        const ENV: &str = "THETADATADX_TEST_FIXTURES_UNSET_001";
        // Make sure no prior test polluted it.
        std::env::remove_var(ENV);
        let result = resolve_fixture_inner("any.csv", ENV);
        match result {
            FixtureResolution::Skipped(_) => {}
            other => panic!("expected Skipped on unset env var, got {other:?}"),
        }
    }

    #[test]
    fn nonexistent_directory_is_skipped() {
        const ENV: &str = "THETADATADX_TEST_FIXTURES_NONEXISTENT_002";
        // Pick a path guaranteed not to exist.
        std::env::set_var(
            ENV,
            "/nonexistent-path-for-thetadatadx-byte-match-test-c52f1e",
        );
        let result = resolve_fixture_inner("any.csv", ENV);
        std::env::remove_var(ENV);
        match result {
            FixtureResolution::Skipped(_) => {}
            other => panic!("expected Skipped on missing directory, got {other:?}"),
        }
    }

    #[test]
    fn opt_in_with_missing_fixture_is_hard_failure_variant() {
        // Env var set to an existing directory, fixture missing inside.
        const ENV: &str = "THETADATADX_TEST_FIXTURES_OPTED_IN_003";
        let tmp = std::env::temp_dir().join("thetadatadx-byte-match-opt-in-test-003");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        // Sanity: directory exists but the named CSV does not.
        std::env::set_var(ENV, &tmp);
        let result = resolve_fixture_inner("missing-fixture.csv", ENV);
        std::env::remove_var(ENV);
        let _ = std::fs::remove_dir_all(&tmp);

        match result {
            FixtureResolution::MissingInOptInDir {
                dir,
                missing,
                env_var,
            } => {
                assert_eq!(dir, tmp);
                assert_eq!(missing, tmp.join("missing-fixture.csv"));
                assert_eq!(env_var, ENV);
            }
            other => {
                panic!("expected MissingInOptInDir on opt-in dir without fixture, got {other:?}")
            }
        }
    }

    #[test]
    fn opt_in_with_present_fixture_is_provisioned() {
        const ENV: &str = "THETADATADX_TEST_FIXTURES_OPTED_IN_004";
        let tmp = std::env::temp_dir().join("thetadatadx-byte-match-opt-in-test-004");
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let fixture_path = tmp.join("provisioned.csv");
        std::fs::write(&fixture_path, b"header\n").expect("write stub fixture");
        std::env::set_var(ENV, &tmp);
        let result = resolve_fixture_inner("provisioned.csv", ENV);
        std::env::remove_var(ENV);
        let _ = std::fs::remove_dir_all(&tmp);

        match result {
            FixtureResolution::Provisioned(path) => {
                assert_eq!(path, fixture_path);
            }
            other => panic!("expected Provisioned when fixture exists, got {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "opt-in fixture")]
    fn resolution_to_option_panics_on_missing_in_opt_in_dir() {
        // Drive the wrapper's panic branch via a stubbed
        // `MissingInOptInDir` value rather than touching the
        // production env var. Set_var-on-production races other
        // tests that read it; the structured-resolution split lets
        // us cover the panic path deterministically.
        let missing = PathBuf::from("/opt-in-dir/missing-fixture.csv");
        let dir = PathBuf::from("/opt-in-dir");
        let stubbed = FixtureResolution::MissingInOptInDir {
            dir,
            missing,
            env_var: "THETADATADX_TEST_FIXTURES_PANIC_005",
        };
        let _ = super::resolution_to_option(stubbed);
    }

    #[test]
    fn resolution_to_option_returns_none_on_skipped() {
        let result = super::resolution_to_option(FixtureResolution::Skipped("unset".to_string()));
        assert!(
            result.is_none(),
            "resolution_to_option must return None on Skipped"
        );
    }

    #[test]
    fn resolution_to_option_returns_some_on_provisioned() {
        let path = PathBuf::from("/some/fixture.csv");
        let result = super::resolution_to_option(FixtureResolution::Provisioned(path.clone()));
        assert_eq!(result, Some(path));
    }

    #[test]
    fn nonexistent_directory_skip_reason_names_the_directory() {
        // Sanity: the diagnostic captures the offending dir so an
        // operator chasing CI output can grep it back to the env var
        // they set.
        const ENV: &str = "THETADATADX_TEST_FIXTURES_NONEXISTENT_006";
        let bad = PathBuf::from("/nope-thetadatadx-006");
        std::env::set_var(ENV, &bad);
        let result = resolve_fixture_inner("x.csv", ENV);
        std::env::remove_var(ENV);

        if let FixtureResolution::Skipped(reason) = result {
            assert!(
                reason.contains("/nope-thetadatadx-006"),
                "skip reason should mention the offending dir, got {reason:?}"
            );
        } else {
            panic!("expected Skipped variant, got {result:?}");
        }
    }
}
