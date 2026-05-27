//! `RestClient` -- talks HTTP to the local Terminal's `/v3/...` paths.
//!
//! Mirrors the gRPC builder shape so call sites can switch transports
//! with a receiver swap; see the module docstring on [`super`] for the
//! incident this exists to escape.

use std::time::Duration;

use reqwest::Client as ReqwestClient;
use tdbe::types::tick::{GreeksFirstOrderTick, IvTick, QuoteTick, TradeQuoteTick};

use super::csv::Table;
use super::error::RestError;

// The tick re-imports above (`GreeksFirstOrderTick` / `IvTick` /
// `QuoteTick` / `TradeQuoteTick`) feed the `decode_*_csv` decoders that
// live at the bottom of this file; the generated builder module in
// `super::_generated` consumes the decoders via `pub` re-export and
// re-imports the tick types from `tdbe` directly.

/// Default Terminal base URL (`http://127.0.0.1:25503`).
pub const DEFAULT_TERMINAL_BASE_URL: &str = "http://127.0.0.1:25503";

/// Default maximum response body size (256 MiB).
///
/// The Terminal can in principle return arbitrarily large CSVs (a
/// year-wide `start_date` / `end_date` window on a busy underlying
/// crosses 100M rows); the default cap protects against accidental
/// OOM when a caller forgets to bound the date range. Raise the cap
/// via [`RestClient::with_max_response_bytes`] for legitimate
/// large-window queries.
pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 256 * 1024 * 1024;

/// HTTP REST transport against the local ThetaTerminal.
///
/// `RestClient` is cheap to construct (no network round-trip on
/// `new()`); the underlying [`reqwest::Client`] holds the connection
/// pool. Clone is `O(1)` -- the `reqwest::Client` is `Arc`-backed and
/// the cap is a `u64` -- so handing the client across worker tasks
/// does not duplicate connections.
#[derive(Debug, Clone)]
pub struct RestClient {
    base_url: String,
    http: ReqwestClient,
    max_response_bytes: u64,
}

impl RestClient {
    /// Build a REST client pointing at `base_url` (e.g.
    /// `"http://127.0.0.1:25503"`).
    ///
    /// `connect_timeout`, `request_timeout` are set to sensible defaults
    /// (5 s / 60 s); override via [`Self::with_http_client`] when the
    /// caller wants a tuned `reqwest::Client`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying `reqwest::ClientBuilder`
    /// fails to assemble (rustls platform-verifier init failure,
    /// invalid timeout config). Hot in practice.
    pub fn new(base_url: impl Into<String>) -> Result<Self, RestError> {
        let http = ReqwestClient::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            base_url: base_url.into(),
            http,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        })
    }

    /// Build a REST client with a caller-supplied [`reqwest::Client`].
    ///
    /// Use when the application already runs a tuned HTTP client
    /// (custom timeouts, connection pool size, middleware) and wants
    /// the REST transport to share it.
    #[must_use]
    pub fn with_http_client(base_url: impl Into<String>, http: ReqwestClient) -> Self {
        Self {
            base_url: base_url.into(),
            http,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// Override the maximum response body size (in bytes) the client
    /// accepts. Defaults to [`DEFAULT_MAX_RESPONSE_BYTES`] (256 MiB).
    ///
    /// The cap is checked against `Content-Length` when the server
    /// emits one and against the streamed byte count as the body
    /// arrives -- either route surfaces
    /// [`RestError::ResponseTooLarge`] before the buffer reaches the
    /// caller's allocator.
    ///
    /// Pass `u64::MAX` to effectively disable the cap.
    #[must_use]
    pub fn with_max_response_bytes(mut self, max_response_bytes: u64) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    /// Base URL the client targets.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Configured maximum response body size (in bytes). See
    /// [`Self::with_max_response_bytes`].
    #[must_use]
    pub fn max_response_bytes(&self) -> u64 {
        self.max_response_bytes
    }
}

// The four `option_history_*` builder constructors + builder structs +
// `impl` blocks (`OptionHistoryQuoteRestBuilder`,
// `OptionHistoryTradeQuoteRestBuilder`, `OptionHistoryGreeksIvRestBuilder`,
// `OptionHistoryGreeksFirstOrderRestBuilder`) are emitted by
// `build_support_bin/endpoints/sdk_render/rest_builder.rs` from
// `endpoint_surface.toml` and live in `super::_generated::rest_endpoints`.
// Adding a new REST endpoint is now one TOML row.

// ─── Transport helper ────────────────────────────────────────────────

/// Issue the GET request and return the body as `String`. Empty
/// string-valued query params are dropped so the Terminal sees a
/// clean URL.
///
/// `pub(crate)` so the generated REST builder module in
/// `super::_generated` can route every endpoint's `execute()` method
/// through this single transport helper.
pub(crate) async fn fetch_csv(
    client: &RestClient,
    path: &str,
    params: &[(&str, &str)],
) -> Result<String, RestError> {
    let url = format!("{}{}", client.base_url, path);
    // Drop empty params so we don't send e.g. `strike=` to the
    // Terminal -- the upstream parser treats empty as "missing", but
    // some endpoints emit a 400 on an empty-valued required param.
    let kept: Vec<(&str, &str)> = params
        .iter()
        .copied()
        .filter(|(_, v)| !v.is_empty())
        .collect();

    tracing::debug!(target: "thetadatadx::rest", path, ?kept, "REST GET");

    let resp = client.http.get(&url).query(&kept).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        let truncated: String = body.chars().take(4096).collect();
        return Err(RestError::HttpStatus {
            status: status.as_u16(),
            body: truncated,
        });
    }

    let limit = client.max_response_bytes;

    // Pre-flight on Content-Length when the server emits one. Cheap
    // O(1) reject before we touch the network buffer.
    let advertised_len = resp.content_length();
    if let Some(cl) = advertised_len {
        if cl > limit {
            return Err(RestError::ResponseTooLarge { size: cl, limit });
        }
    }

    // Stream the body in chunks. The cap is checked on every chunk so a
    // server that under-reports `Content-Length` (or omits it entirely
    // when transfer-encoding=chunked) cannot smuggle past the limit.
    // Seed the accumulator with the advertised content-length when the
    // server provided one and it fits the cap — saves the
    // double-and-copy growth pattern on the typical historical-quote
    // response (multi-MB bodies, single-shot consumption). On the
    // chunked path (`Content-Length` absent) seed with a 64 KiB floor
    // so the first DATA frame doesn't trip a per-chunk realloc.
    const CHUNKED_INITIAL_CAPACITY: usize = 64 * 1024;
    let mut buf: Vec<u8> = match advertised_len {
        Some(cl) if cl <= limit => {
            // Floor the seed at the chunked initial capacity so a
            // `Content-Length: 0` head paired with a chunked body
            // (e.g. a misconfigured proxy that forwards both
            // framings) does not trip the same per-chunk realloc the
            // pure-chunked path avoids.
            let advertised = usize::try_from(cl).unwrap_or(CHUNKED_INITIAL_CAPACITY);
            Vec::with_capacity(std::cmp::max(advertised, CHUNKED_INITIAL_CAPACITY))
        }
        _ => Vec::with_capacity(CHUNKED_INITIAL_CAPACITY),
    };
    let mut stream = resp;
    while let Some(chunk) = stream.chunk().await? {
        let new_len = (buf.len() as u64).saturating_add(chunk.len() as u64);
        if new_len > limit {
            return Err(RestError::ResponseTooLarge {
                size: new_len,
                limit,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).map_err(|e| RestError::CsvDecode {
        reason: format!("response body is not valid UTF-8: {e}"),
        row: usize::MAX,
    })
}

// ─── Per-tick CSV decoders ───────────────────────────────────────────
//
// Each decoder mirrors the gRPC `parse_<tick>_ticks` function: resolve
// every column index up front, then iterate rows. Absent columns
// zero-fill via `cell_i32_or_zero` / `cell_f64_or_zero` so the
// decoder tolerates subset NBBO layouts the upstream may emit for
// older storage tiers.

/// Decode the `option_history_quote` CSV body into `Vec<QuoteTick>`.
///
/// Accepts both the full 11-field and the 6-field subset header
/// layouts; absent columns default to 0, mirroring the gRPC
/// decoder's `opt_number(row, None) -> 0` contract.
///
/// # Errors
///
/// Returns [`RestError`] when the body has no header, when one of the
/// hard-required columns (`ms_of_day`, `date`) is missing on a
/// non-empty response, or when a numeric cell fails to parse.
pub fn decode_quote_csv(body: &str) -> Result<Vec<QuoteTick>, RestError> {
    let table = Table::parse(body)?;
    let ms_idx = table.column_index("ms_of_day");
    let bid_size_idx = table.column_index("bid_size");
    let bid_exg_idx = table.column_index("bid_exchange");
    let bid_idx = table.column_index("bid");
    let bid_cond_idx = table.column_index("bid_condition");
    let ask_size_idx = table.column_index("ask_size");
    let ask_exg_idx = table.column_index("ask_exchange");
    let ask_idx = table.column_index("ask");
    let ask_cond_idx = table.column_index("ask_condition");
    let date_idx = table.column_index("date");

    if table.rows.is_empty() {
        // Empty response -- legitimate "no data today". No header
        // validation, mirrors the gRPC path's empty-table guard.
        return Ok(vec![]);
    }

    // N3: validate required columns BEFORE allocating the row buffer.
    // A response missing `ms_of_day` / `date` is a wire-format failure;
    // surfacing it before `Vec::with_capacity(rows.len())` saves a
    // potentially-large allocation on a malformed million-row body.
    if ms_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "ms_of_day",
            available: table.headers.join(","),
        });
    }
    if date_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "date",
            available: table.headers.join(","),
        });
    }

    let mut out: Vec<QuoteTick> = Vec::with_capacity(table.rows.len());
    for (row_idx, _) in table.rows.iter().enumerate() {
        let ms_of_day = table.cell_i32_required(row_idx, ms_idx, "ms_of_day")?;
        let date = table.cell_i32_required(row_idx, date_idx, "date")?;
        let bid = table.cell_f64_or_zero(row_idx, bid_idx)?;
        let ask = table.cell_f64_or_zero(row_idx, ask_idx)?;
        let midpoint = (bid + ask) / 2.0;
        // QuoteTick has contract-id fields (expiration / strike /
        // right). The REST path doesn't currently carry them in the
        // CSV -- they're per-request constants, not per-row. Zero /
        // empty placeholders keep the struct populated; callers who
        // need contract identity in the result attach it at the
        // request layer.
        out.push(QuoteTick {
            ms_of_day,
            bid_size: table.cell_i32_or_zero(row_idx, bid_size_idx)?,
            bid_exchange: table.cell_i32_or_zero(row_idx, bid_exg_idx)?,
            bid,
            bid_condition: table.cell_i32_or_zero(row_idx, bid_cond_idx)?,
            ask_size: table.cell_i32_or_zero(row_idx, ask_size_idx)?,
            ask_exchange: table.cell_i32_or_zero(row_idx, ask_exg_idx)?,
            ask,
            ask_condition: table.cell_i32_or_zero(row_idx, ask_cond_idx)?,
            date,
            midpoint,
            expiration: 0,
            strike: 0.0,
            right: 0,
        });
    }
    Ok(out)
}

/// Decode `option_history_trade_quote` CSV into `Vec<TradeQuoteTick>`.
///
/// Combined trade+quote rows carry the quote-side columns the legacy
/// rollover affects (`bid_exchange`, `bid_condition`,
/// `ask_exchange`, `ask_condition`) -- those default to 0 when
/// absent, same contract as the standalone quote path.
///
/// # Errors
///
/// See [`decode_quote_csv`].
pub fn decode_trade_quote_csv(body: &str) -> Result<Vec<TradeQuoteTick>, RestError> {
    let table = Table::parse(body)?;
    if table.rows.is_empty() {
        return Ok(vec![]);
    }
    let ms_idx = table.column_index("ms_of_day");
    let price_idx = table.column_index("price");
    let size_idx = table.column_index("size");
    let exchange_idx = table.column_index("exchange");
    let bid_size_idx = table.column_index("bid_size");
    let bid_exg_idx = table.column_index("bid_exchange");
    let bid_idx = table.column_index("bid");
    let bid_cond_idx = table.column_index("bid_condition");
    let ask_size_idx = table.column_index("ask_size");
    let ask_exg_idx = table.column_index("ask_exchange");
    let ask_idx = table.column_index("ask");
    let ask_cond_idx = table.column_index("ask_condition");
    let date_idx = table.column_index("date");

    // N3: required columns validated before the row-buffer allocation.
    if ms_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "ms_of_day",
            available: table.headers.join(","),
        });
    }
    if date_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "date",
            available: table.headers.join(","),
        });
    }

    let mut out: Vec<TradeQuoteTick> = Vec::with_capacity(table.rows.len());
    for row_idx in 0..table.rows.len() {
        let ms_of_day = table.cell_i32_required(row_idx, ms_idx, "ms_of_day")?;
        let date = table.cell_i32_required(row_idx, date_idx, "date")?;
        let price = table.cell_f64_or_zero(row_idx, price_idx)?;
        let bid = table.cell_f64_or_zero(row_idx, bid_idx)?;
        let ask = table.cell_f64_or_zero(row_idx, ask_idx)?;
        out.push(TradeQuoteTick {
            ms_of_day,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: table.cell_i32_or_zero(row_idx, size_idx)?,
            exchange: table.cell_i32_or_zero(row_idx, exchange_idx)?,
            price,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            quote_ms_of_day: ms_of_day,
            bid_size: table.cell_i32_or_zero(row_idx, bid_size_idx)?,
            bid_exchange: table.cell_i32_or_zero(row_idx, bid_exg_idx)?,
            bid,
            bid_condition: table.cell_i32_or_zero(row_idx, bid_cond_idx)?,
            ask_size: table.cell_i32_or_zero(row_idx, ask_size_idx)?,
            ask_exchange: table.cell_i32_or_zero(row_idx, ask_exg_idx)?,
            ask,
            ask_condition: table.cell_i32_or_zero(row_idx, ask_cond_idx)?,
            date,
            expiration: 0,
            strike: 0.0,
            right: 0,
        });
    }
    Ok(out)
}

/// Decode `option_history_greeks_implied_volatility` CSV into
/// `Vec<IvTick>`.
///
/// # Errors
///
/// See [`decode_quote_csv`].
pub fn decode_iv_csv(body: &str) -> Result<Vec<IvTick>, RestError> {
    let table = Table::parse(body)?;
    if table.rows.is_empty() {
        return Ok(vec![]);
    }
    let ms_idx = table.column_index("ms_of_day");
    let bid_idx = table.column_index("bid");
    let bid_iv_idx = table
        .column_index("bid_implied_vol")
        .or_else(|| table.column_index("bid_implied_volatility"));
    let midpoint_idx = table.column_index("midpoint");
    // Accept both wire (`implied_vol`) and schema (`implied_volatility`)
    // header forms — the REST decoder is the user-facing csv parser and
    // production callers may pre-rewrite the header to match the public
    // schema field name.
    let iv_idx = table
        .column_index("implied_vol")
        .or_else(|| table.column_index("implied_volatility"));
    let ask_idx = table.column_index("ask");
    let ask_iv_idx = table
        .column_index("ask_implied_vol")
        .or_else(|| table.column_index("ask_implied_volatility"));
    let iv_err_idx = table.column_index("iv_error");
    let und_ms_idx = table
        .column_index("underlying_timestamp")
        .or_else(|| table.column_index("underlying_ms_of_day"));
    let und_price_idx = table.column_index("underlying_price");
    let date_idx = table.column_index("date");

    // N3: required columns validated before the row-buffer allocation.
    if ms_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "ms_of_day",
            available: table.headers.join(","),
        });
    }
    if date_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "date",
            available: table.headers.join(","),
        });
    }

    let mut out: Vec<IvTick> = Vec::with_capacity(table.rows.len());
    for row_idx in 0..table.rows.len() {
        let ms_of_day = table.cell_i32_required(row_idx, ms_idx, "ms_of_day")?;
        let date = table.cell_i32_required(row_idx, date_idx, "date")?;
        out.push(IvTick {
            ms_of_day,
            bid: table.cell_f64_or_zero(row_idx, bid_idx)?,
            bid_implied_volatility: table.cell_f64_or_zero(row_idx, bid_iv_idx)?,
            midpoint: table.cell_f64_or_zero(row_idx, midpoint_idx)?,
            implied_volatility: table.cell_f64_or_zero(row_idx, iv_idx)?,
            ask: table.cell_f64_or_zero(row_idx, ask_idx)?,
            ask_implied_volatility: table.cell_f64_or_zero(row_idx, ask_iv_idx)?,
            iv_error: table.cell_f64_or_zero(row_idx, iv_err_idx)?,
            underlying_ms_of_day: table.cell_i32_or_zero(row_idx, und_ms_idx)?,
            underlying_price: table.cell_f64_or_zero(row_idx, und_price_idx)?,
            date,
            expiration: 0,
            strike: 0.0,
            right: 0,
        });
    }
    Ok(out)
}

/// Decode `option_history_greeks_first_order` CSV into
/// `Vec<GreeksFirstOrderTick>`.
///
/// # Errors
///
/// See [`decode_quote_csv`].
pub fn decode_greeks_first_order_csv(body: &str) -> Result<Vec<GreeksFirstOrderTick>, RestError> {
    let table = Table::parse(body)?;
    if table.rows.is_empty() {
        return Ok(vec![]);
    }
    // Resolve every column index up front, mirroring `decode_quote_csv`.
    // The pre-M2 implementation called `column_index(...)` inline on each
    // row; for the 13-column Greeks layout that was 13 * `rows.len()`
    // header-scan lookups per response.
    let ms_idx = table.column_index("ms_of_day");
    let date_idx = table.column_index("date");
    let bid_idx = table.column_index("bid");
    let ask_idx = table.column_index("ask");
    let delta_idx = table.column_index("delta");
    let theta_idx = table.column_index("theta");
    let vega_idx = table.column_index("vega");
    let rho_idx = table.column_index("rho");
    let epsilon_idx = table.column_index("epsilon");
    let lambda_idx = table.column_index("lambda");
    let iv_idx = table.column_index("implied_volatility");
    let iv_err_idx = table.column_index("iv_error");
    let und_ms_idx = table.column_index("underlying_ms_of_day");
    let und_price_idx = table.column_index("underlying_price");

    // N3: required columns validated before the row-buffer allocation.
    if ms_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "ms_of_day",
            available: table.headers.join(","),
        });
    }
    if date_idx.is_none() {
        return Err(RestError::MissingColumn {
            column: "date",
            available: table.headers.join(","),
        });
    }

    let mut out: Vec<GreeksFirstOrderTick> = Vec::with_capacity(table.rows.len());
    for row_idx in 0..table.rows.len() {
        let ms_of_day = table.cell_i32_required(row_idx, ms_idx, "ms_of_day")?;
        let date = table.cell_i32_required(row_idx, date_idx, "date")?;
        out.push(GreeksFirstOrderTick {
            ms_of_day,
            bid: table.cell_f64_or_zero(row_idx, bid_idx)?,
            ask: table.cell_f64_or_zero(row_idx, ask_idx)?,
            delta: table.cell_f64_or_zero(row_idx, delta_idx)?,
            theta: table.cell_f64_or_zero(row_idx, theta_idx)?,
            vega: table.cell_f64_or_zero(row_idx, vega_idx)?,
            rho: table.cell_f64_or_zero(row_idx, rho_idx)?,
            epsilon: table.cell_f64_or_zero(row_idx, epsilon_idx)?,
            lambda: table.cell_f64_or_zero(row_idx, lambda_idx)?,
            implied_volatility: table.cell_f64_or_zero(row_idx, iv_idx)?,
            iv_error: table.cell_f64_or_zero(row_idx, iv_err_idx)?,
            underlying_ms_of_day: table.cell_i32_or_zero(row_idx, und_ms_idx)?,
            underlying_price: table.cell_f64_or_zero(row_idx, und_price_idx)?,
            date,
            expiration: 0,
            strike: 0.0,
            right: 0,
        });
    }
    Ok(out)
}
