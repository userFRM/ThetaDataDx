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
async fn mid_stream_unauthenticated_classifies_as_grpc_unauthenticated() {
    // Mock emits one valid DATA chunk, then closes the stream with
    // `grpc-status: 16 (Unauthenticated)` in the trailers — the exact
    // wire shape MDDS produces when an upstream session token expires
    // mid-stream. The streaming retry / refresh shell in the generated
    // builders dispatches on `Error::Grpc { kind: Unauthenticated }`,
    // so this test pins the wire-to-classifier handoff: the mid-
    // stream trailer status surfaces as `ChannelError::Rpc {
    // status.code() == 16 }`, and the umbrella conversion
    // `From<ChannelError> for Error` then folds it to
    // `Error::Grpc { kind: Unauthenticated }` along the streaming-
    // builder code path.
    use futures::StreamExt as _;
    use thetadatadx::grpc::{ChannelError, Status};
    use thetadatadx::wire::{DataValueList, ResponseData};

    let server = mock::MockServer::spawn_with_message(
        vec![make_symbols_response(&["AAPL", "MSFT"])],
        16,
        "session expired".to_string(),
    )
    .await;

    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect");

    let mut stream = channel
        .server_streaming::<DataValueList, ResponseData>(
            "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
            DataValueList::default(),
        )
        .await
        .expect("rpc opens");

    // The mid-stream error may surface on the very first poll (when
    // the response head and trailers are flushed together) or after
    // one or more successful DATA chunks. Either shape is correct on
    // the wire; the assertion is that the failure is `Rpc { 16 }`.
    let final_err = loop {
        match stream.next().await {
            Some(Ok(_)) => continue,
            Some(Err(e)) => break e,
            None => panic!("stream closed cleanly; expected Unauthenticated trailers"),
        }
    };

    match final_err {
        ChannelError::Rpc { status } => {
            assert_eq!(
                status.code(),
                16,
                "trailing grpc-status preserved (Unauthenticated)"
            );
        }
        other => panic!("expected ChannelError::Rpc(Unauthenticated), got {other:?}"),
    }

    // The umbrella conversion is what the streaming builder uses to
    // hand the classifier a typed `Error::Grpc { Unauthenticated }`,
    // which the macro-driven retry shell then routes to NeedsRefresh.
    // Pin the wire-status → typed-Error mapping here so a future
    // refactor of `From<ChannelError> for Error` cannot silently
    // re-route `16` to `Transport(_)` (which would skip refresh).
    let thetadatadx_err: thetadatadx::Error = ChannelError::Rpc {
        status: Status::new(16, "session expired"),
    }
    .into();
    match thetadatadx_err {
        thetadatadx::Error::Grpc {
            kind: thetadatadx::error::GrpcStatusKind::Unauthenticated,
            ..
        } => {}
        other => panic!("ChannelError → Error conversion drift: got {other:?}"),
    }
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
