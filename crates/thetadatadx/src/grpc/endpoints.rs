//! Endpoint methods routed through [`crate::grpc::Channel`].
//!
//! Each function here is the in-house counterpart of one tonic-backed
//! method on [`crate::mdds::MddsClient`]. The two paths share the
//! `mdds.proto` types and the `decode` helpers; only the transport
//! layer differs — `tonic::Streaming<ResponseData>` becomes
//! [`crate::grpc::ServerStreaming<ResponseData>`].

use std::collections::HashMap;

use futures_core::Stream;
use tokio_stream::StreamExt;

use crate::decode;
use crate::error::Error;
use crate::proto;

use super::channel::{Channel, ChannelError};

/// gRPC method path for `BetaThetaTerminal::GetStockListSymbols`.
const STOCK_LIST_SYMBOLS_METHOD: &str = "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols";

/// `client = "terminal"` is the only static entry the wire `query_parameters`
/// carries. The tonic path builds the same map per call (see
/// `mdds::client::MddsClient::query_info`).
const CLIENT_PARAMETER_VALUE: &str = "terminal";

/// Issue `BetaThetaTerminal::GetStockListSymbols` over `channel` and
/// return the list of stock symbols.
///
/// `session_uuid` and `client_type` are the same auth values the tonic
/// path threads through `QueryInfo.auth_token.session_uuid` and
/// `QueryInfo.client_type` — see [`crate::mdds::client::MddsClient`]
/// for how they are obtained from the Nexus auth response.
///
/// # Errors
///
/// Returns an [`Error`] when the RPC fails, the response cannot be
/// decoded, or the schema does not carry the `symbol` column.
pub async fn stock_list_symbols(
    channel: &Channel,
    session_uuid: String,
    client_type: String,
) -> Result<Vec<String>, Error> {
    let mut query_parameters = HashMap::with_capacity(1);
    query_parameters.insert("client".to_string(), CLIENT_PARAMETER_VALUE.to_string());

    let request = proto::StockListSymbolsRequest {
        query_info: Some(proto::QueryInfo {
            auth_token: Some(proto::AuthToken { session_uuid }),
            query_parameters,
            client_type,
            terminal_git_commit: String::new(),
            terminal_version: env!("CARGO_PKG_VERSION").to_string(),
        }),
        params: Some(proto::StockListSymbolsRequestQuery {}),
    };

    let stream = channel
        .server_streaming::<proto::StockListSymbolsRequest, proto::ResponseData>(
            STOCK_LIST_SYMBOLS_METHOD,
            request,
        )
        .await
        .map_err(map_channel_error)?;

    let table = collect_stream(stream).await?;
    Ok(decode::extract_text_column(&table, "symbol")
        .into_iter()
        .flatten()
        .collect())
}

/// Drain `stream` into a single merged `DataTable`. Mirrors
/// [`crate::mdds::MddsClient::collect_stream`] but operates on the
/// in-house [`crate::grpc::ServerStreaming`] adapter rather than
/// `tonic::Streaming`.
pub async fn collect_stream<S>(mut stream: S) -> Result<proto::DataTable, Error>
where
    S: Stream<Item = Result<proto::ResponseData, ChannelError>> + Unpin,
{
    let mut all_rows: Vec<proto::DataValueList> = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut chunk_index: usize = 0;

    while let Some(response) = stream.next().await {
        let response = response.map_err(map_channel_error)?;

        if all_rows.is_empty() && response.original_size > 0 {
            // Same pre-allocation hint the tonic path uses: ~64 bytes
            // per row keeps the row vec from reallocating mid-stream.
            all_rows.reserve(usize::try_from(response.original_size).unwrap_or(0) / 64);
        }

        let table = decode::decode_data_table(&response)?;
        if headers.is_empty() {
            headers = table.headers;
        } else if !table.headers.is_empty() && table.headers != headers {
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index,
                first: headers.join(","),
                chunk: table.headers.join(","),
            }
            .into());
        }
        all_rows.extend(table.data_table);
        chunk_index += 1;
    }

    Ok(proto::DataTable {
        headers,
        data_table: all_rows,
    })
}

/// Run `BetaThetaTerminal::GetStockListSymbols` over a `tonic` channel
/// using the exact same request shape the in-house path sends. Exists
/// so the criterion bench can A/B the two transports against the same
/// upstream wire bytes without leaking generated `proto` types onto
/// the public surface.
///
/// # Errors
///
/// Returns an [`Error`] when the RPC fails, the response cannot be
/// decoded, or the schema does not carry the `symbol` column.
pub async fn stock_list_symbols_via_tonic(
    channel: tonic::transport::Channel,
    session_uuid: String,
    client_type: String,
) -> Result<Vec<String>, Error> {
    let mut query_parameters = HashMap::with_capacity(1);
    query_parameters.insert("client".to_string(), CLIENT_PARAMETER_VALUE.to_string());

    let request = tonic::Request::new(proto::StockListSymbolsRequest {
        query_info: Some(proto::QueryInfo {
            auth_token: Some(proto::AuthToken { session_uuid }),
            query_parameters,
            client_type,
            terminal_git_commit: String::new(),
            terminal_version: env!("CARGO_PKG_VERSION").to_string(),
        }),
        params: Some(proto::StockListSymbolsRequestQuery {}),
    });

    let mut stub = proto::beta_theta_terminal_client::BetaThetaTerminalClient::new(channel);
    let response = stub
        .get_stock_list_symbols(request)
        .await
        .map_err(|e| Error::config_internal(format!("tonic rpc: {e}")))?;
    let stream = response.into_inner();
    let table = collect_tonic_stream(stream).await?;
    Ok(decode::extract_text_column(&table, "symbol")
        .into_iter()
        .flatten()
        .collect())
}

async fn collect_tonic_stream(
    mut stream: tonic::Streaming<proto::ResponseData>,
) -> Result<proto::DataTable, Error> {
    let mut all_rows: Vec<proto::DataValueList> = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut chunk_index: usize = 0;
    while let Some(response) = tokio_stream::StreamExt::next(&mut stream).await {
        let response =
            response.map_err(|e| Error::config_internal(format!("tonic stream: {e}")))?;
        if all_rows.is_empty() && response.original_size > 0 {
            all_rows.reserve(usize::try_from(response.original_size).unwrap_or(0) / 64);
        }
        let table = decode::decode_data_table(&response)?;
        if headers.is_empty() {
            headers = table.headers;
        } else if !table.headers.is_empty() && table.headers != headers {
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index,
                first: headers.join(","),
                chunk: table.headers.join(","),
            }
            .into());
        }
        all_rows.extend(table.data_table);
        chunk_index += 1;
    }
    Ok(proto::DataTable {
        headers,
        data_table: all_rows,
    })
}

/// Lift a [`ChannelError`] into the crate's umbrella [`Error`] type.
///
/// Wire-level failures (`Rpc`, `Codec`, `H2Stream`, ...) flow through
/// [`Error::config_internal`] with the underlying message preserved in
/// the carried `String`. Callers that need structured discrimination
/// match on the [`ChannelError`] directly via [`Channel::server_streaming`].
fn map_channel_error(err: ChannelError) -> Error {
    match err {
        ChannelError::Rpc { status } => Error::config_internal(format!(
            "in-house grpc rpc returned non-ok status: {status}"
        )),
        other => Error::config_internal(format!("in-house grpc transport: {other}")),
    }
}
