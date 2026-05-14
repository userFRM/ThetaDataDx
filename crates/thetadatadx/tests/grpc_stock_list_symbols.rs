//! End-to-end test of [`thetadatadx::grpc::stock_list_symbols`] against
//! the mock h2 server in `grpc_mock_server.rs`.
//!
//! Builds a `ResponseData` whose `compressed_data` field carries a
//! zstd-compressed `DataTable` with one `symbol` column. The in-house
//! endpoint runs the same decode pipeline tonic-backed callers do
//! (`decode_data_table` → `extract_text_column`), so a green result
//! verifies the full chain end-to-end:
//!
//!   `Channel::connect_h2c` → `Channel::server_streaming` →
//!   `ServerStreaming` → `Codec::decode` → `decode_data_table` →
//!   `extract_text_column`.

use std::io::Write;
use std::time::Duration;

use prost::Message;

use thetadatadx::grpc::{stock_list_symbols, Channel};
use thetadatadx::wire::{
    data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
    ResponseData,
};

#[path = "grpc_mock_server.rs"]
mod mock;

/// Build a `ResponseData` carrying a single-column `DataTable` with
/// the given symbols, zstd-compressed exactly the way the upstream
/// MDDS server does it.
fn make_symbols_response(symbols: &[&str]) -> ResponseData {
    let rows: Vec<DataValueList> = symbols
        .iter()
        .map(|s| DataValueList {
            values: vec![DataValue {
                data_type: Some(data_value::DataType::Text((*s).to_string())),
            }],
        })
        .collect();

    let table = DataTable {
        headers: vec!["symbol".to_string()],
        data_table: rows,
    };
    let inner = table.encode_to_vec();

    // zstd-compress with level 3, the same default tier the upstream
    // server selects. `decode_data_table` reads the compression algo
    // off `compression_description` and routes to the right path.
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).expect("zstd encoder");
    encoder.write_all(&inner).expect("zstd write");
    let compressed = encoder.finish().expect("zstd finalize");

    ResponseData {
        compressed_data: compressed,
        compression_description: Some(CompressionDescription {
            algo: i32::from(CompressionAlgo::Zstd),
            ..CompressionDescription::default()
        }),
        original_size: i32::try_from(inner.len()).unwrap_or(0),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_house_stock_list_symbols_returns_decoded_symbols() {
    let server = mock::MockServer::spawn(
        vec![make_symbols_response(&["AAPL", "MSFT", "SPY", "QQQ"])],
        0,
    )
    .await;

    let channel = tokio::time::timeout(
        Duration::from_secs(5),
        Channel::connect_h2c("127.0.0.1", server.addr.port()),
    )
    .await
    .expect("connect did not hang")
    .expect("h2c connect");

    let symbols = stock_list_symbols(
        &channel,
        "00000000-0000-0000-0000-000000000000".to_string(),
        "rust-thetadatadx-grpc".to_string(),
    )
    .await
    .expect("rpc completes ok");

    assert_eq!(symbols, vec!["AAPL", "MSFT", "SPY", "QQQ"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_house_stock_list_symbols_merges_two_chunks() {
    let server = mock::MockServer::spawn(
        vec![
            make_symbols_response(&["AAPL", "MSFT"]),
            make_symbols_response(&["SPY", "QQQ"]),
        ],
        0,
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect");

    let symbols = stock_list_symbols(
        &channel,
        "00000000-0000-0000-0000-000000000000".to_string(),
        "rust-thetadatadx-grpc".to_string(),
    )
    .await
    .expect("rpc completes ok");

    assert_eq!(symbols, vec!["AAPL", "MSFT", "SPY", "QQQ"]);
}
