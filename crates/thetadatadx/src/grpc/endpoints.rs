//! Endpoint methods for MDDS historical queries.
//!
//! The hand-written `stock_list_symbols` helper here is the
//! foundation example. The full RPC surface is emitted into
//! `crate::proto::beta_theta_terminal` by the codegen in
//! `build_support/grpc/`, with one async function per
//! server-streaming method in `mdds.proto`.

use std::collections::HashMap;

use crate::util::stream_ext::StreamNextExt;

use crate::decode;
use crate::error::Error;
use crate::proto;

use super::channel::{Channel, ChannelError};

/// `client = "terminal"` is the only static entry the wire
/// `query_parameters` map carries; the macro-driven [`crate::mdds`]
/// endpoints set the same value (see `MddsClient::query_info`).
const CLIENT_PARAMETER_VALUE: &str = "terminal";

/// Issue `BetaThetaTerminal::GetStockListSymbols` over `channel` and
/// return the list of stock symbols.
///
/// `session_uuid` and `client_type` thread through the
/// `QueryInfo.auth_token.session_uuid` and `QueryInfo.client_type`
/// wire fields — see [`crate::mdds::MddsClient`] for how they are
/// obtained from the Nexus auth response.
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

    let stream = proto::beta_theta_terminal::get_stock_list_symbols(channel, request)
        .await
        .map_err(map_channel_error)?;

    let table = collect_stream(stream).await?;
    Ok(decode::extract_text_column(&table, "symbol")
        .into_iter()
        .flatten()
        .collect())
}

/// Drain `stream` into a single merged `DataTable`. Mirrors the
/// `collect_stream` helper on [`crate::mdds::MddsClient`] but operates
/// on the in-house [`crate::grpc::ServerStreaming`] adapter rather than
/// `tonic::Streaming`.
///
/// When the stream's source [`crate::grpc::Channel`] carries a
/// [`crate::grpc::DecoderHandle`], each chunk's zstd + protobuf
/// decode runs on a dedicated decoder thread; otherwise the work
/// runs inline on the caller's tokio task.
pub async fn collect_stream(
    mut stream: crate::grpc::ServerStreaming<proto::ResponseData>,
) -> Result<proto::DataTable, Error> {
    let mut all_rows: Vec<proto::DataValueList> = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut chunk_index: usize = 0;
    let decoder = stream.decoder().cloned();
    let max_message_size = stream.max_message_size();

    while let Some(response) = stream.next().await {
        let response = response.map_err(map_channel_error)?;

        if all_rows.is_empty() && response.original_size > 0 {
            // R1: reserve hint must also respect the channel's
            // `max_message_size` so a hostile peer that claims
            // `original_size = i32::MAX` cannot inflate `all_rows`'
            // capacity to 33 M slots (≈ 32 MiB of `DataValueList`
            // header overhead) ahead of the decompression-layer
            // rejection that follows.
            let hint = usize::try_from(response.original_size).unwrap_or(0);
            let bounded = hint.min(max_message_size);
            // ~64 bytes per row keeps the row vec from reallocating
            // mid-stream.
            all_rows.reserve(bounded / 64);
        }

        let table = decode_chunk(decoder.as_ref(), response, max_message_size).await?;
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

/// Route a single chunk through the channel's decoder pool (when
/// attached) so zstd + `DataTable::decode` runs off-reactor. Falls
/// back to inline decode when no decoder is attached — keeps the
/// helper usable from the unit-test channels that construct a
/// [`crate::grpc::Channel`] without a pool wired up.
async fn decode_chunk(
    decoder: Option<&crate::grpc::DecoderHandle>,
    response: proto::ResponseData,
    max_message_size: usize,
) -> Result<proto::DataTable, Error> {
    if let Some(handle) = decoder {
        // `submit` short-circuits with `DecoderSubmitError::Poisoned`
        // when a prior worker-thread panic has flipped the pool's
        // poison flag — surface as a transport-level failure so
        // higher layers can decide on retry vs. rebuild.
        let rx = handle
            .submit(response, max_message_size)
            .map_err(|err| Error::Transport {
                kind: crate::error::TransportErrorKind::DecoderPoisoned,
                message: format!("mdds decoder pool rejected submission: {err}"),
            })?;
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(Error::Transport {
                kind: crate::error::TransportErrorKind::DecoderReplyDropped,
                message: "mdds decoder pool dropped its reply channel".to_string(),
            }),
        }
    } else {
        let mut response = response;
        decode::decode_data_table_with_max(&mut response, max_message_size)
    }
}

/// Bench-only helpers that issue representative MDDS RPCs through the
/// in-house transport without going through the macro-generated
/// `MddsClient` surface. Exists so `benches/grpc_channel.rs` can A/B
/// 2–3 endpoints without re-implementing the full `MddsClient::connect`
/// auth handshake just to time an RPC.
///
/// # Errors
///
/// Returns an [`Error`] when the RPC fails or the response cannot be
/// decoded.
pub mod bench_support {
    use super::{map_channel_error, Channel, Error, HashMap, CLIENT_PARAMETER_VALUE};
    use crate::proto;

    fn make_query_info(session_uuid: String, client_type: String) -> proto::QueryInfo {
        let mut query_parameters = HashMap::with_capacity(1);
        query_parameters.insert("client".to_string(), CLIENT_PARAMETER_VALUE.to_string());
        proto::QueryInfo {
            auth_token: Some(proto::AuthToken { session_uuid }),
            query_parameters,
            client_type,
            terminal_git_commit: String::new(),
            terminal_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Issue `BetaThetaTerminal::GetStockHistoryEod` and return the
    /// merged response table. Used by the criterion bench only.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the RPC fails or the response cannot
    /// be decoded.
    pub async fn stock_history_eod(
        channel: &Channel,
        session_uuid: String,
        client_type: String,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<proto::DataTable, Error> {
        let request = proto::StockHistoryEodRequest {
            query_info: Some(make_query_info(session_uuid, client_type)),
            params: Some(proto::StockHistoryEodRequestQuery {
                symbol: symbol.to_string(),
                start_date: start_date.to_string(),
                end_date: end_date.to_string(),
            }),
        };
        let stream = proto::beta_theta_terminal::get_stock_history_eod(channel, request)
            .await
            .map_err(map_channel_error)?;
        super::collect_stream(stream).await
    }

    /// Issue `BetaThetaTerminal::GetOptionHistoryQuote` and return the
    /// merged response table. Used by the criterion bench only.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when the RPC fails or the response cannot
    /// be decoded.
    pub async fn option_history_quote(
        channel: &Channel,
        session_uuid: String,
        client_type: String,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> Result<proto::DataTable, Error> {
        let request = proto::OptionHistoryQuoteRequest {
            query_info: Some(make_query_info(session_uuid, client_type)),
            params: Some(proto::OptionHistoryQuoteRequestQuery {
                contract_spec: Some(proto::ContractSpec {
                    symbol: symbol.to_string(),
                    expiration: expiration.to_string(),
                    strike: Some(strike.to_string()),
                    right: Some(right.to_string()),
                }),
                date: Some(date.to_string()),
                expiration: expiration.to_string(),
                start_time: Some("09:30:00".to_string()),
                end_time: Some("16:00:00".to_string()),
                interval: "1s".to_string(),
                max_dte: None,
                strike_range: None,
                start_date: None,
                end_date: None,
            }),
        };
        let stream = proto::beta_theta_terminal::get_option_history_quote(channel, request)
            .await
            .map_err(map_channel_error)?;
        super::collect_stream(stream).await
    }
}

/// Lift a [`ChannelError`] into the crate's umbrella [`Error`] type.
///
/// Delegates to the canonical `From<ChannelError> for Error` impl in
/// [`crate::error`], which preserves the structured taxonomy:
/// `Rpc` becomes `Error::Grpc { kind: GrpcStatusKind::*, .. }`,
/// `DeadlineExceeded` becomes `Error::Timeout`, and every transport
/// fault folds into `Error::Transport { kind: TransportErrorKind::*, .. }`.
/// The retry classifier in [`crate::mdds::macros`] then dispatches on
/// the typed `kind` rather than parsing `Display` strings.
fn map_channel_error(err: ChannelError) -> Error {
    Error::from(err)
}
