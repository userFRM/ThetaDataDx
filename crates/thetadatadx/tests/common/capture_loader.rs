//! Shared capture-fixture loaders for the integration suites.
//!
//! Three integration tests (`test_decode_captures.rs`,
//! `test_eod_greeks_schema.rs`, `test_trade_greeks_schema.rs`)
//! historically carried hand-copied `load_response` helpers that
//! differed in subtle ways: the original `test_decode_captures.rs`
//! copy decoded the zstd-wrapped fixture format only, while the
//! newer trade-greeks and EOD-greek suites added a dual-format sniff
//! (`0x28 0xb5 0x2f 0xfd` magic vs raw `ResponseData` proto bytes).
//! The routing suite (`test_endpoint_routing.rs`) needed the same
//! logic for the gRPC mock and the divergence multiplied. This
//! module is the single source-of-truth: every suite includes it via
//! `#[path = "common/capture_loader.rs"] mod capture_loader;` and
//! calls [`load_response_data`] (raw `ResponseData` for the mock
//! transport tests) or [`load_data_table`] (decoded `DataTable` for
//! the per-parser regression suites).
//!
//! Adding a new fixture format? Update the `if bytes.starts_with(...)`
//! sniff here once; every test suite picks the change up via the
//! shared include.

#![allow(dead_code)] // Loaders are pub helpers; individual tests use a subset.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use prost::Message;

use thetadatadx::decode;
use thetadatadx::wire as proto;

/// Resolve the `tests/fixtures/captures/` directory.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("captures")
}

/// Load a capture fixture as the raw outer `proto::ResponseData`.
///
/// Auto-detects the fixture format from the file's first four bytes:
///
///   * `0x28 0xb5 0x2f 0xfd` — legacy fixtures (the `.pb.zst` suffix
///     is literal): outer zstd frame wraps the `ResponseData` bytes.
///   * Otherwise — raw `ResponseData` proto bytes; the inner
///     `compressed_data` field carries the zstd-wrapped `DataTable`
///     payload (sniffed and decompressed by `decode::decode_data_table`).
///
/// Decode / decompression errors panic with the fixture path — a
/// broken fixture is a test-infra bug, not a product bug.
pub fn load_response_data(endpoint: &str) -> proto::ResponseData {
    let path = fixtures_dir().join(format!("{endpoint}.pb.zst"));
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = zstd::Decoder::new(&bytes[..])
            .unwrap_or_else(|e| panic!("zstd::Decoder::new({}): {e}", path.display()));
        let mut inner = Vec::new();
        decoder
            .read_to_end(&mut inner)
            .unwrap_or_else(|e| panic!("zstd read_to_end {}: {e}", path.display()));
        proto::ResponseData::decode(inner.as_slice())
            .unwrap_or_else(|e| panic!("proto::ResponseData::decode {}: {e}", path.display()))
    } else {
        proto::ResponseData::decode(bytes.as_slice())
            .unwrap_or_else(|e| panic!("proto::ResponseData::decode {}: {e}", path.display()))
    }
}

/// Load a capture fixture and run the production decode pipeline
/// (`decode::decode_data_table`) to produce the columnar `DataTable`
/// the per-parser regression suites assert against. Composes
/// [`load_response_data`] with the in-crate decoder so individual
/// tests do not duplicate either step.
pub fn load_data_table(endpoint: &str) -> proto::DataTable {
    let mut response = load_response_data(endpoint);
    decode::decode_data_table(&mut response).expect("decode_data_table")
}
