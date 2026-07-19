//! Automatic sharding of large history pulls.
//!
//! One logical query becomes N disjoint sub-requests along a filter
//! axis (date band, time band), dispatched concurrently across the
//! tier's channel pool. Bands are cut from the request shape alone —
//! equal wall-clock duration on the time axis, equal day count on the
//! date axis — never from a sizing query. Balance is equal span, not
//! equal rows: a deliberate trade that costs zero extra round-trips and
//! zero per-endpoint tuning, and the concurrency win dominates the
//! residual row imbalance between bands. The buffered `.await` path merges the shard
//! responses back into exactly the rows the single-stream response would
//! have carried — in the exact server order for single-contract / stock /
//! index pulls, in a deterministic canonical contract order for chains
//! (see `merge_typed_in_order`). The chunk-streaming `.stream*` path forwards
//! each band's chunks to the user handler as they arrive — no merge, no
//! materialize — so chunks from different bands interleave in arrival
//! order (see `join_streaming_shards`). The server's per-account send
//! rate caps a single stream well below the tier's aggregate budget, so
//! the fan-out multiplies bulk throughput without exceeding the tier's
//! concurrent-request ceiling.
//!
//! # Flow (buffered `.await` path)
//!
//! 1. The generated builder assembles its wire parameters once and
//!    projects them into a [`ShardQuery`].
//! 2. `auto_plan` applies the [`BulkFetchPolicy`], the shardable-endpoint
//!    gate, axis selection, and the equal-span band cut — pure
//!    computation, no request issued. Anything that does not clearly
//!    benefit resolves to `None` and runs on today's single stream,
//!    byte-identical.
//! 3. Each [`ShardBand`] is applied onto a clone of the original wire
//!    parameters — every shard is a normal terminal-parity query differing
//!    only in its band fields.
//! 4. Shards run as independent top-level requests (each takes its own
//!    request-semaphore permit), parsing each response chunk into typed
//!    rows as it arrives (`collect_stream_typed` in `mdds/stream.rs` —
//!    the proto rows are never buffered), a band the server reports
//!    empty (`NotFound`) contributes zero rows, and `merge_typed_in_order`
//!    reassembles the output (see its order note).
//!
//! The streaming `.stream*` path shares steps 1–3 verbatim; step 4
//! becomes `join_streaming_shards`, which drives the per-band streams
//! concurrently and forwards chunks instead of joining tables.
//!
//! # Semaphore safety
//!
//! The joining driver never holds a request permit: each shard acquires
//! its own permit inside its spawned task. Since the semaphore is sized
//! to the channel pool (== tier cap) and a plan never exceeds the pool
//! size, N shards make progress even at `pool_size == 1` (they simply
//! serialize).

use std::sync::Arc;

use crate::auth::SessionToken;
use crate::columns::{Ticks, WireColumns};
use crate::config::{BulkFetchPolicy, RetryPolicy};
use crate::error::{Error, GrpcStatusKind};
use crate::grpc::{ChannelLease, ChannelPool};
use crate::proto;

use super::client::MarketDataClient;
use super::decode;
use super::decode::headers::find_header;

// ─── Date-range split math (shared with the Python binding) ─────────────

/// Failure modes of the date-range split.
#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    /// A boundary string is not a valid `YYYYMMDD` Gregorian date. The
    /// first field is the offending input, the second a human-readable
    /// reason.
    #[error("invalid YYYYMMDD date '{0}': {1}")]
    InvalidDate(String, String),
    /// The requested range has its end before its start.
    #[error("end date {end} is before start date {start}")]
    EndBeforeStart {
        /// Requested (inclusive) start of the range.
        start: String,
        /// Requested (inclusive) end of the range.
        end: String,
    },
}

/// Validate a decomposed Gregorian date against the actual calendar.
///
/// Rejects impossible combinations — month 0, month > 12, day 0, day > the
/// month's length, and Feb 29 in non-leap years — returning `false` for any
/// such input. The leap-year rule is the proleptic Gregorian: divisible by 4,
/// except centuries, except quadricentennials.
fn is_valid_ymd(year: i32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap {
                29
            } else {
                28
            }
        }
        _ => unreachable!(),
    };
    day <= days_in_month
}

/// A single (inclusive) day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ymd {
    year: u32,
    month: u32,
    day: u32,
}

impl Ymd {
    fn from_yyyymmdd(s: &str) -> Result<Self, ChunkError> {
        if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ChunkError::InvalidDate(
                s.to_string(),
                "must be 8 ASCII digits".into(),
            ));
        }
        let year = s[0..4]
            .parse::<i32>()
            .map_err(|e| ChunkError::InvalidDate(s.to_string(), format!("year: {e}")))?;
        let month = s[4..6]
            .parse::<u32>()
            .map_err(|e| ChunkError::InvalidDate(s.to_string(), format!("month: {e}")))?;
        let day = s[6..8]
            .parse::<u32>()
            .map_err(|e| ChunkError::InvalidDate(s.to_string(), format!("day: {e}")))?;
        if !(0..=9999).contains(&year) {
            return Err(ChunkError::InvalidDate(
                s.to_string(),
                format!("year {year} out of YYYY range"),
            ));
        }
        if !is_valid_ymd(year, month, day) {
            return Err(ChunkError::InvalidDate(
                s.to_string(),
                format!("{year:04}-{month:02}-{day:02} is not a valid Gregorian date"),
            ));
        }
        Ok(Ymd {
            year: year as u32,
            month,
            day,
        })
    }

    fn to_yyyymmdd(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// Days from 0001-01-01 (Gregorian). Simple proleptic calculation —
    /// accurate for any reasonable market-data range (the server only
    /// has post-1990 data anyway).
    fn to_ord(self) -> i64 {
        // Rata Die ordinal from Howard Hinnant's date algorithms.
        let y = if self.month <= 2 {
            i64::from(self.year) - 1
        } else {
            i64::from(self.year)
        };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64;
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = (yoe as i64) * 365 + (yoe as i64) / 4 - (yoe as i64) / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_ord(z: i64) -> Self {
        // Inverse Rata Die (Howard Hinnant).
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp.wrapping_sub(9) } as u32;
        let y = if m <= 2 { y + 1 } else { y };
        Ymd {
            year: y as u32,
            month: m,
            day: d,
        }
    }
}

/// Maximum span accepted by the server (inclusive). The server caps a
/// single request at 365 days.
const MAX_SPAN_DAYS: i64 = 365;

/// Split `(start, end)` (YYYYMMDD strings, inclusive on both ends) into
/// chunks that each span at most 365 days. The returned chunks are
/// contiguous and cover the full range exactly once.
///
/// The ThetaData server rejects history ranges exceeding 365 calendar days
/// with a raw gRPC `InvalidArgument`; callers can turn an arbitrary
/// multi-year range into a series of server-accepted requests and
/// concatenate the results. Pure date arithmetic — no tokio, no client.
/// Also re-exported to Python as `thetadatadx.split_date_range`.
///
/// # Errors
///
/// Returns [`ChunkError::InvalidDate`] when either boundary is not a
/// valid `YYYYMMDD` Gregorian date, and [`ChunkError::EndBeforeStart`]
/// when `end` precedes `start`.
pub fn split_date_range(start: &str, end: &str) -> Result<Vec<(String, String)>, ChunkError> {
    let start_ord = Ymd::from_yyyymmdd(start)?.to_ord();
    let end_ord = Ymd::from_yyyymmdd(end)?.to_ord();
    if end_ord < start_ord {
        return Err(ChunkError::EndBeforeStart {
            start: start.into(),
            end: end.into(),
        });
    }
    if end_ord - start_ord < MAX_SPAN_DAYS {
        return Ok(vec![(start.to_string(), end.to_string())]);
    }

    let mut chunks = Vec::new();
    let mut cursor = start_ord;
    while cursor <= end_ord {
        let chunk_end = (cursor + MAX_SPAN_DAYS - 1).min(end_ord);
        chunks.push((
            Ymd::from_ord(cursor).to_yyyymmdd(),
            Ymd::from_ord(chunk_end).to_yyyymmdd(),
        ));
        cursor = chunk_end + 1;
    }
    Ok(chunks)
}

// ─── Public plan types ───────────────────────────────────────────────────

/// The filter axis a shard plan cuts along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShardAxis {
    /// Contiguous inclusive `start_date ..= end_date` sub-ranges.
    Date,
    /// Intraday `start_time ..= end_time` bands within one trading day.
    Time,
}

/// One shard's filter override. Applied onto a clone of the original wire
/// parameters; every other parameter is forwarded verbatim, so each shard
/// is a normal terminal-parity query.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShardBand {
    /// Inclusive date sub-range (`YYYYMMDD`, both ends), matching the
    /// server's inclusive `start_date` / `end_date` semantics.
    Date {
        /// Inclusive band start (`YYYYMMDD`).
        start_date: String,
        /// Inclusive band end (`YYYYMMDD`).
        end_date: String,
    },
    /// Inclusive intraday window (`HH:MM:SS.mmm`, both ends), matching the
    /// server's inclusive `start_time` / `end_time` semantics at
    /// millisecond resolution. Adjacent bands abut at `end + 1 ms`.
    Time {
        /// Inclusive band start (`HH:MM:SS.mmm`).
        start_time: String,
        /// Inclusive band end (`HH:MM:SS.mmm`).
        end_time: String,
    },
}

/// An equal-span fan-out plan for one logical history query: the
/// per-shard band overrides, in output order.
///
/// Power users running their own concurrency can request the plan through
/// [`MarketDataClient::bulk_fetch_plan`] and apply each band to a clone of
/// their builder call (bands only override the band fields; every other
/// parameter stays as issued). Treat a band that errors `NotFound` as
/// empty — the union of bands can still hold data. Concatenating the
/// per-band responses in band order reproduces the single-stream rows
/// exactly; chain responses land in a deterministic canonical contract
/// order rather than the server's own enumeration (see the order note on
/// `merge_typed_in_order` in this module's source).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShardPlan {
    /// Disjoint, exhaustive band overrides in output order.
    pub bands: Vec<ShardBand>,
}

/// Wire-level view of a history query, as consumed by the shard planner.
///
/// Field values are the normalized wire strings the request carries
/// (`YYYYMMDD` dates, `HH:MM:SS[.mmm]` times, `call`/`put`/`both` rights).
/// The generated builders project their parameters into this shape
/// automatically; manual-mode callers fill the fields that mirror their
/// builder call and leave the rest `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShardQuery {
    /// Underlying symbol / root.
    pub symbol: Option<String>,
    /// Option expiration (`YYYYMMDD` or `*`), when the query has one.
    pub expiration: Option<String>,
    /// Option strike (wire form, `*` for the chain), when present.
    pub strike: Option<String>,
    /// Option right (`call` / `put` / `both`), when present.
    pub right: Option<String>,
    /// Single trading date (`YYYYMMDD`), when the query names one.
    pub date: Option<String>,
    /// Inclusive range start (`YYYYMMDD`), when the query names a range.
    pub start_date: Option<String>,
    /// Inclusive range end (`YYYYMMDD`), when the query names a range.
    pub end_date: Option<String>,
    /// Intraday window start (`HH:MM:SS[.mmm]` or ms-of-day digits).
    pub start_time: Option<String>,
    /// Intraday window end (`HH:MM:SS[.mmm]` or ms-of-day digits).
    pub end_time: Option<String>,
    /// Bar / snapshot interval as issued (`tick`, `1s`, `1m`, ...).
    pub interval: Option<String>,
}

// ─── Wire-field projection helpers (used by the endpoint macros) ─────────

/// Request fields the shard machinery reads or writes come in two proto
/// spellings — `string` and `optional string` — depending on the endpoint.
/// This trait erases that difference for the generated projection macros.
pub(crate) trait WireParam {
    fn opt_str(&self) -> Option<String>;
    fn set_str(&mut self, v: &str);
}

impl WireParam for String {
    fn opt_str(&self) -> Option<String> {
        if self.is_empty() {
            None
        } else {
            Some(self.clone())
        }
    }
    fn set_str(&mut self, v: &str) {
        *self = v.to_string();
    }
}

impl WireParam for Option<String> {
    fn opt_str(&self) -> Option<String> {
        self.as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    fn set_str(&mut self, v: &str) {
        *self = Some(v.to_string());
    }
}

/// Multi-symbol snapshot fields. Snapshot endpoints are never shardable,
/// so the projection reads nothing from them.
impl WireParam for Vec<String> {
    fn opt_str(&self) -> Option<String> {
        None
    }
    fn set_str(&mut self, _v: &str) {}
}

/// Read a wire field into its `ShardQuery` slot (macro shim).
pub(crate) fn opt_wire_str<T: WireParam>(v: &T) -> Option<String> {
    v.opt_str()
}

/// Write a band value onto a wire field (macro shim).
pub(crate) fn set_wire_str<T: WireParam>(field: &mut T, v: &str) {
    field.set_str(v);
}

// ─── Shardable endpoints ─────────────────────────────────────────────────

/// Whether the generated endpoint method named `endpoint` may shard.
///
/// Only intraday (tick / bar / greeks) bulk history endpoints qualify:
/// snapshots, lists, at-time queries, and the daily-only EOD /
/// open-interest / greeks-EOD families return bounded responses that a
/// fan-out cannot help. Endpoints not listed here always run on the
/// single-stream path.
fn is_shardable_history_endpoint(endpoint: &str) -> bool {
    matches!(
        endpoint,
        "option_history_trade"
            | "option_history_ohlc"
            | "option_history_trade_quote"
            | "option_history_quote"
            | "option_history_trade_greeks_all"
            | "option_history_trade_greeks_first_order"
            | "option_history_trade_greeks_second_order"
            | "option_history_trade_greeks_third_order"
            | "option_history_trade_greeks_implied_volatility"
            | "option_history_greeks_all"
            | "option_history_greeks_first_order"
            | "option_history_greeks_second_order"
            | "option_history_greeks_third_order"
            | "option_history_greeks_implied_volatility"
            | "stock_history_trade"
            | "stock_history_trade_quote"
            | "stock_history_ohlc"
            | "stock_history_quote"
            | "index_history_price"
            | "index_history_ohlc"
    )
}

// ─── Pure planning math ──────────────────────────────────────────────────

/// Bar-grid ceiling below which a bounded-interval query stays on the
/// single stream. A single-contract (or stock / index) query at a
/// bounded bar interval cannot return more than one row per grid slot;
/// when that provable ceiling is this small, the single stream finishes
/// within about one server prepare interval of a fan-out, so sharding
/// cannot win. Chain cross-products and tick-interval pulls have no
/// such ceiling and shard on the window alone.
const SHARD_MIN_GRID_ROWS: u64 = 10_000_000;

/// Minimum wall-clock span of one time band (five minutes). The server
/// spends a roughly fixed prepare interval per request before the first
/// byte, so a band much narrower than this pays fan-out overhead without
/// meaningful work to parallelize; a window that cannot yield at least
/// two such bands stays on the single stream.
const MIN_SHARD_BAND_MS: i64 = 5 * 60_000;

/// Regular-session window (09:30–16:00 ET): the per-day bar-grid span
/// the small-pull gate assumes when the query itself carries no
/// parsable window.
const RTH_WINDOW_MS: i64 = 23_400_000;

/// Parse a wire time-of-day (`HH:MM`, `HH:MM:SS`, `HH:MM:SS.mmm`, or bare
/// ms-of-day digits) into milliseconds since midnight ET.
fn parse_ms_of_day(s: &str) -> Option<i64> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        let ms = s.parse::<i64>().ok()?;
        return (ms < 86_400_000).then_some(ms);
    }
    let mut parts = s.split(':');
    let hours = parts.next()?.parse::<i64>().ok()?;
    let minutes = parts.next()?.parse::<i64>().ok()?;
    let (seconds, millis) = match parts.next() {
        None => (0, 0),
        Some(sec) => match sec.split_once('.') {
            None => (sec.parse::<i64>().ok()?, 0),
            Some((whole, frac)) => {
                let millis = match frac.len() {
                    1 => frac.parse::<i64>().ok()? * 100,
                    2 => frac.parse::<i64>().ok()? * 10,
                    3 => frac.parse::<i64>().ok()?,
                    _ => return None,
                };
                (whole.parse::<i64>().ok()?, millis)
            }
        },
    };
    if parts.next().is_some()
        || !(0..24).contains(&hours)
        || !(0..60).contains(&minutes)
        || !(0..60).contains(&seconds)
        || !(0..1_000).contains(&millis)
    {
        return None;
    }
    Some(((hours * 60 + minutes) * 60 + seconds) * 1_000 + millis)
}

/// Format milliseconds since midnight as the wire-canonical
/// `HH:MM:SS.mmm`.
fn format_hms(ms: i64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms % 3_600_000) / 60_000,
        (ms % 60_000) / 1_000,
        ms % 1_000
    )
}

/// Interval width in ms. `None` for `tick` (or anything unrecognised,
/// which errs toward treating the pull as unbounded tick density).
fn interval_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("tick") {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        // Legacy numeric spelling: interval in milliseconds, forwarded
        // verbatim on the wire.
        return s.parse::<i64>().ok().filter(|&n| n > 0);
    }
    let (digits, unit_ms) = if let Some(p) = s.strip_suffix("ms") {
        (p, 1)
    } else if let Some(p) = s.strip_suffix('s') {
        (p, 1_000)
    } else if let Some(p) = s.strip_suffix('m') {
        (p, 60_000)
    } else {
        (s.strip_suffix('h')?, 3_600_000)
    };
    digits
        .parse::<i64>()
        .ok()
        .filter(|&n| n > 0)
        .and_then(|n| n.checked_mul(unit_ms))
}

/// Inclusive-day span of the query's `start_date ..= end_date` range, when
/// both bounds parse. `None` when the range is absent or malformed.
fn date_span_days(q: &ShardQuery) -> Option<i64> {
    let s = Ymd::from_yyyymmdd(q.start_date.as_deref()?).ok()?.to_ord();
    let e = Ymd::from_yyyymmdd(q.end_date.as_deref()?).ok()?.to_ord();
    (e >= s).then_some(e - s + 1)
}

/// Whether the query spans more than one contract (an option-chain
/// cross-product). Only chain pulls have an a-priori-unbounded row count
/// per grid slot; a concrete contract — and every stock / index query —
/// is capped at one row per bar for bounded intervals, which the
/// small-pull gates in `plan_query` exploit.
fn chain_cross_product(q: &ShardQuery) -> bool {
    q.expiration.is_some()
        && (q.expiration.as_deref() == Some("*")
            || q.strike.as_deref().is_none_or(|s| s == "*")
            || q.right.as_deref() != Some("call") && q.right.as_deref() != Some("put"))
}

/// Pick the shard axis for a query shape, if any.
///
/// Priority mirrors the balance the axes deliver: a multi-day range cuts
/// into date bands; a single trading day with an intraday window cuts into
/// time bands (the headline case — dates cannot split one day).
fn select_axis(q: &ShardQuery) -> Option<ShardAxis> {
    let span = date_span_days(q);
    if q.date.is_none() && span.is_some_and(|d| d >= 2) {
        return Some(ShardAxis::Date);
    }
    // A single trading day, spelled either as `date` or as a degenerate
    // one-day range. A query carrying BOTH a `date` and a range is left
    // unsharded: the server's precedence between the two spellings is its
    // own, and a shard plan must never guess at it.
    let single_day = (q.date.is_some() && q.start_date.is_none() && q.end_date.is_none())
        || (q.date.is_none() && span == Some(1));
    if !single_day {
        return None;
    }
    if q.start_time.is_some() && q.end_time.is_some() {
        return Some(ShardAxis::Time);
    }
    None
}

/// Materialize `n` equal-duration time bands over the inclusive window
/// `[start_ms, end_ms]`. Caller guarantees `1 <= n <= window_ms`.
///
/// Bands are inclusive `[start, end]` wire windows at millisecond
/// resolution: band `k` ends 1 ms before band `k + 1` starts, so every
/// tick lands in exactly one band. The server treats `start_time` /
/// `end_time` as inclusive at millisecond resolution — verified live:
/// adjacent inclusive bands built this way reproduce the single
/// stream's row count exactly on full-day chain pulls. The first band
/// starts at the query's own `start_time` and the last ends at its
/// `end_time`; band durations differ by at most 1 ms.
fn time_bands(start_ms: i64, end_ms: i64, n: i64) -> Vec<ShardBand> {
    let window = end_ms - start_ms + 1;
    (0..n)
        .map(|k| ShardBand::Time {
            start_time: format_hms(start_ms + window * k / n),
            end_time: format_hms(start_ms + window * (k + 1) / n - 1),
        })
        .collect()
}

/// Materialize `n` equal-day-count date bands over the inclusive day
/// ordinals `[start_ord, end_ord]`. Caller guarantees `1 <= n <= days`.
/// Bands are contiguous inclusive `[start_date, end_date]` sub-ranges
/// covering the query's range exactly once, matching the server's
/// inclusive date semantics; band day counts differ by at most one.
fn date_bands(start_ord: i64, end_ord: i64, n: i64) -> Vec<ShardBand> {
    let days = end_ord - start_ord + 1;
    (0..n)
        .map(|k| ShardBand::Date {
            start_date: Ymd::from_ord(start_ord + days * k / n).to_yyyymmdd(),
            end_date: Ymd::from_ord(start_ord + days * (k + 1) / n - 1).to_yyyymmdd(),
        })
        .collect()
}

// ─── Dispatch handle (owned, task-portable) ──────────────────────────────

/// Owned, `'static` snapshot of everything one top-level MDDS dispatch
/// needs: the tier semaphore, the shared session token, the channel pool,
/// the retry policy, and the `QueryInfo` template. All handles are
/// `Arc`-backed clones of the client's own — a `ShardDispatch` moved into
/// a spawned shard task dispatches exactly like the client itself.
#[derive(Clone)]
pub(crate) struct ShardDispatch {
    pub(crate) semaphore: Arc<tokio::sync::Semaphore>,
    pub(crate) session: SessionToken,
    channels: ChannelPool,
    pub(crate) retry: RetryPolicy,
    query_info_template: proto::QueryInfo,
}

impl ShardDispatch {
    pub(crate) fn new(
        semaphore: Arc<tokio::sync::Semaphore>,
        session: SessionToken,
        channels: ChannelPool,
        retry: RetryPolicy,
        query_info_template: proto::QueryInfo,
    ) -> Self {
        Self {
            semaphore,
            session,
            channels,
            retry,
            query_info_template,
        }
    }

    /// `QueryInfo` for one attempt, pinned to the attempt's session UUID.
    pub(crate) fn query_info(&self, uuid: String) -> proto::QueryInfo {
        let mut qi = self.query_info_template.clone();
        qi.auth_token = Some(proto::AuthToken { session_uuid: uuid });
        qi
    }

    /// Pick the next pooled channel (same least-loaded lease semantics as
    /// [`MarketDataClient::channel`]).
    pub(crate) fn channel(&self) -> ChannelLease<'_> {
        self.channels.next()
    }
}

// ─── Plan construction ───────────────────────────────────────────────────

/// Resolved shard width: the configured `shard_concurrency` clamped into
/// `[1, pool_size]`. The pool size is the tier's server-enforced
/// concurrent-request ceiling, so a plan can never spread wider than the
/// account is allowed to run.
fn shard_width(config_width: Option<u32>, pool_size: usize) -> usize {
    let pool = pool_size.max(1);
    match config_width {
        Some(n) => usize::try_from(n).unwrap_or(pool).clamp(1, pool),
        None => pool,
    }
}

/// Policy-gated plan for the automatic paths. Any reason not to shard —
/// policy `Off`, an unlisted endpoint, no usable axis, or a provably
/// small or too-narrow pull — resolves to `None`, and the caller runs
/// today's single stream.
pub(crate) fn auto_plan(
    client: &MarketDataClient,
    endpoint: &'static str,
    q: &ShardQuery,
) -> Option<ShardPlan> {
    if client.config().market_data.bulk_fetch == BulkFetchPolicy::Off {
        return None;
    }
    let width = shard_width(
        client.config().market_data.shard_concurrency,
        client.pool_size(),
    );
    let plan = plan_query(endpoint, q, width);
    if let Some(p) = plan.as_ref() {
        let axis = match p.bands.first() {
            Some(ShardBand::Time { .. }) => "time",
            Some(ShardBand::Date { .. }) => "date",
            None => "none",
        };
        tracing::debug!(
            endpoint,
            axis,
            shards = p.bands.len(),
            "bulk fetch sharded across the request pool"
        );
    }
    plan
}

/// Build the shard plan for `endpoint` and `q` at fan-out width `width`,
/// from the request shape alone — pure computation, no request issued.
///
/// Sharding splits a large history pull into equal wall-clock bands —
/// equal duration along the time axis, equal day count along the date
/// axis — run concurrently; it never issues a sizing query. Balance is
/// equal span, not equal rows: a deliberate trade that costs zero extra
/// round-trips and zero per-endpoint tuning, and the concurrency win
/// dominates the residual row imbalance between bands.
///
/// Shared by [`MarketDataClient::bulk_fetch_plan`] (manual mode) and
/// [`auto_plan`]. Returns `None` when the query should stay on a single
/// stream: an endpoint outside the shardable set, a shape with no cut
/// axis, `width < 2`, a window too narrow for two bands of
/// [`MIN_SHARD_BAND_MS`], or a bounded-interval single-contract pull
/// whose bar grid provably stays under [`SHARD_MIN_GRID_ROWS`].
fn plan_query(endpoint: &str, q: &ShardQuery, width: usize) -> Option<ShardPlan> {
    if !is_shardable_history_endpoint(endpoint) {
        return None;
    }
    let axis = select_axis(q)?;
    if width < 2 {
        return None;
    }
    let width = i64::try_from(width).unwrap_or(i64::MAX);
    match axis {
        ShardAxis::Time => {
            let start_ms = parse_ms_of_day(q.start_time.as_deref()?)?;
            let end_ms = parse_ms_of_day(q.end_time.as_deref()?)?;
            if end_ms <= start_ms {
                return None;
            }
            // Request-shape small-pull gate: a single-contract (or stock
            // / index) query at a bounded bar interval cannot exceed one
            // row per grid slot, so a provably-small pull never pays a
            // fan-out. Chain cross-products and tick-interval pulls are
            // unbounded a priori and shard on the window alone.
            if !chain_cross_product(q) {
                if let Some(ivl) = q.interval.as_deref().and_then(interval_ms) {
                    let grid_rows =
                        u64::try_from((end_ms - start_ms) / ivl.max(1) + 1).unwrap_or(0);
                    if grid_rows < SHARD_MIN_GRID_ROWS {
                        return None;
                    }
                }
            }
            // Equal-duration bands, capped so each spans at least
            // `MIN_SHARD_BAND_MS`; a window too narrow for two such
            // bands stays on the single stream.
            let n = ((end_ms - start_ms + 1) / MIN_SHARD_BAND_MS).min(width);
            (n >= 2).then(|| ShardPlan {
                bands: time_bands(start_ms, end_ms, n),
            })
        }
        ShardAxis::Date => {
            let start_ord = Ymd::from_yyyymmdd(q.start_date.as_deref()?).ok()?.to_ord();
            let end_ord = Ymd::from_yyyymmdd(q.end_date.as_deref()?).ok()?.to_ord();
            // Same small-pull gate as the time axis, with per-day rows
            // capped by the query's own intraday window (the regular
            // session when it carries none).
            if !chain_cross_product(q) {
                if let Some(ivl) = q.interval.as_deref().and_then(interval_ms) {
                    let window_ms = match (
                        q.start_time.as_deref().and_then(parse_ms_of_day),
                        q.end_time.as_deref().and_then(parse_ms_of_day),
                    ) {
                        (Some(s), Some(e)) if e > s => e - s,
                        _ => RTH_WINDOW_MS,
                    };
                    let days = u64::try_from(end_ord - start_ord + 1).unwrap_or(u64::MAX);
                    let per_day = u64::try_from(window_ms / ivl.max(1) + 1).unwrap_or(0);
                    if days.saturating_mul(per_day) < SHARD_MIN_GRID_ROWS {
                        return None;
                    }
                }
            }
            // Equal-day-count bands, at most one band per day.
            let n = (end_ord - start_ord + 1).min(width);
            (n >= 2).then(|| ShardPlan {
                bands: date_bands(start_ord, end_ord, n),
            })
        }
    }
}

impl MarketDataClient {
    /// Compute the shard plan the automatic bulk-fetch path would use for
    /// one history query, without running the pull.
    ///
    /// Manual-mode entry point: apply each returned [`ShardBand`] to a
    /// clone of the same builder call (band fields override, everything
    /// else stays as issued) and run the sub-requests under your own
    /// concurrency. Ignores the configured [`BulkFetchPolicy`], so the
    /// plan stays available with `bulk_fetch = Off`. `endpoint` is the
    /// builder method name (for example `"option_history_quote"`).
    ///
    /// Pure computation on the request shape — no request is issued.
    /// Returns `None` when the query should stay on a single stream: the
    /// endpoint is not a bulk history endpoint, the shape offers no cut
    /// axis, or the pull is provably too small (or its window too
    /// narrow) for a fan-out to help.
    #[must_use]
    pub fn bulk_fetch_plan(&self, endpoint: &str, query: &ShardQuery) -> Option<ShardPlan> {
        let width = shard_width(
            self.config().market_data.shard_concurrency,
            self.pool_size(),
        );
        plan_query(endpoint, query, width)
    }
}

// ─── Concurrent driver ───────────────────────────────────────────────────

/// A spawned shard request whose task is aborted when the handle drops.
///
/// The buffered call's deadline works by dropping the in-flight future
/// (`run_with_optional_deadline`); spawned tasks would outlive that drop,
/// so the abort-on-drop guard extends the exact same cancellation contract
/// to every shard: dropping the driver aborts the tasks, each task's
/// semaphore permit releases, and its `ServerStreaming` drops
/// (RST_STREAM).
pub(crate) struct ShardTask<R>(tokio::task::JoinHandle<Result<R, Error>>);

impl<R> Drop for ShardTask<R> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawn one shard as an independent top-level request. The future must
/// acquire its own request-semaphore permit; the caller (the joining
/// driver) holds none, which is what makes the fan-out deadlock-free
/// against the tier-sized semaphore.
pub(crate) fn spawn_shard<R, F>(fut: F) -> ShardTask<R>
where
    R: Send + 'static,
    F: std::future::Future<Output = Result<R, Error>> + Send + 'static,
{
    ShardTask(tokio::spawn(fut))
}

/// Await every shard in band order and return their typed bands in that
/// order.
///
/// A band the server holds no rows for comes back as gRPC `NotFound`
/// ("no data") — the same status a single stream returns for an empty
/// query — so a `NotFound` shard folds to an empty contribution instead
/// of failing the pull: the bands partition the query, and the union can
/// have data even when one band is empty. Only when EVERY shard reports
/// `NotFound` is the first such error propagated, matching what a single
/// stream returns for a genuinely empty query. Any other shard error (or
/// panic) fails the whole logical query — the same contract as a
/// single-stream error — and dropping the remaining [`ShardTask`] guards
/// aborts their in-flight requests.
pub(crate) async fn join_shards<T>(
    tasks: Vec<ShardTask<TypedBand<T>>>,
) -> Result<Vec<TypedBand<T>>, Error> {
    let mut bands = Vec::with_capacity(tasks.len());
    let mut first_not_found: Option<Error> = None;
    for mut task in tasks {
        match (&mut task.0).await {
            Ok(Ok(band)) => bands.push(band),
            Ok(Err(
                err @ Error::Grpc {
                    kind: GrpcStatusKind::NotFound,
                    ..
                },
            )) => {
                tracing::debug!("empty shard band (NotFound) folded to zero rows");
                first_not_found.get_or_insert(err);
            }
            Ok(Err(err)) => return Err(err),
            Err(join_err) => {
                return Err(Error::config_internal(format!(
                    "shard task terminated abnormally: {join_err}"
                )))
            }
        }
    }
    match first_not_found {
        // Every band was empty — NotFound-folded, or opened with zero rows —
        // so the whole query is empty and the caller gets the single-stream
        // NotFound status for it, not a spurious empty frame.
        Some(err) if bands.iter().all(|band| band.rows.is_empty()) => Err(err),
        _ => Ok(bands),
    }
}

// ─── Concurrent streaming driver ─────────────────────────────────────────

/// One in-line per-band streaming shard, driven by
/// [`join_streaming_shards`]. Resolves to whether the band delivered any
/// non-empty chunk to the shared handler.
///
/// Streaming shards are plain futures rather than spawned [`ShardTask`]s
/// because they call the user's chunk handler directly: the `.stream*`
/// handlers are `FnMut + Send` with no `'static` bound (they may borrow
/// the caller's stack), so they cannot move into `tokio::spawn`. Driving
/// the futures in-line preserves the exact cancellation contract anyway —
/// dropping the join future drops every band future, which drops each
/// band's `ServerStreaming` (RST_STREAM) and releases its semaphore
/// permit.
pub(crate) type ShardStreamFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, Error>> + Send + 'a>>;

/// Drive per-band streaming shards concurrently to completion.
///
/// All bands are polled on the caller's task, so chunks reach the shared
/// handler in arrival order across bands; the caller (the fan-out arm in
/// `parsed_endpoint!`) holds no request permit while this runs — each
/// band future acquires its own, which is what keeps the fan-out
/// deadlock-free at any pool size (worst case the bands serialize).
/// Every completion event re-polls the remaining set; the set is capped
/// by the tier pool, so the sweep is noise next to a chunk decode.
///
/// Mirrors [`join_shards`]: a band the server holds no rows for errors
/// gRPC `NotFound`, which folds to an empty contribution; only when
/// every band is empty — NotFound-folded or completed without
/// delivering a chunk — is the first `NotFound` propagated, matching
/// what a single stream returns for a genuinely empty query. The fold
/// relies on MDDS issuing `NotFound` only as a pre-stream verdict: a band
/// that delivered chunks and then terminated `NotFound` would be folded
/// rather than surfaced, but the server never returns `NotFound` after
/// data. Any other band error fails the whole logical stream as soon as
/// it lands
/// (chunks other bands already delivered stay delivered, like any
/// mid-stream error), and returning drops the remaining band futures,
/// aborting their in-flight requests.
///
/// # Errors
///
/// Returns the first non-`NotFound` band error unchanged, or the first
/// `NotFound` when every band was empty.
pub(crate) async fn join_streaming_shards(
    mut shards: Vec<ShardStreamFuture<'_>>,
) -> Result<(), Error> {
    let mut first_not_found: Option<Error> = None;
    let mut any_rows = false;
    std::future::poll_fn(|cx| {
        let mut i = 0;
        while i < shards.len() {
            match shards[i].as_mut().poll(cx) {
                std::task::Poll::Ready(Ok(delivered)) => {
                    any_rows |= delivered;
                    drop(shards.swap_remove(i));
                }
                std::task::Poll::Ready(Err(err)) => {
                    if matches!(
                        err,
                        Error::Grpc {
                            kind: GrpcStatusKind::NotFound,
                            ..
                        }
                    ) {
                        tracing::debug!("empty shard band (NotFound) folded to zero chunks");
                        first_not_found.get_or_insert(err);
                        drop(shards.swap_remove(i));
                    } else {
                        return std::task::Poll::Ready(Err(err));
                    }
                }
                std::task::Poll::Pending => i += 1,
            }
        }
        if shards.is_empty() {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    })
    .await?;
    match first_not_found {
        // Every band was empty — NotFound-folded, or completed without a
        // delivered chunk — so the whole query is empty and the caller
        // gets the single-stream NotFound status for it.
        Some(err) if !any_rows => Err(err),
        _ => Ok(()),
    }
}

// ─── Ordered merge ───────────────────────────────────────────────────────

/// One shard band's collected buffered response: rows parsed to the
/// endpoint's typed ticks chunk-by-chunk as they arrived (see
/// `collect_stream_typed` in `mdds/stream.rs`), plus the response schema
/// and the band's `root` (symbol) column summary — everything
/// [`merge_typed_in_order`] needs to reassemble the exact frame the
/// single-stream buffered path emits, without the band ever
/// materializing its proto table.
#[derive(Debug)]
pub(crate) struct TypedBand<T> {
    /// The band's response schema: its first non-empty chunk headers.
    /// Empty when the stream carried none — which, for a band with rows,
    /// fails the parse (every tick schema declares required headers)
    /// before this struct is built.
    pub(crate) headers: Vec<String>,
    /// Typed rows in the band's server order.
    pub(crate) rows: Vec<T>,
    /// The band's `root` (symbol) column cells, folded chunk by chunk.
    pub(crate) root: RootColumn,
}

/// One band's `root` (symbol) column summary, folded chunk by chunk while
/// each chunk is decode-hot.
///
/// The single-stream buffered path classifies the merged table's root
/// column in one pass (`decode::extract::response_symbol`): absent,
/// constant across every row, or per-row varying. The shard path folds
/// the same classification incrementally: the constant column the
/// single-underlying history endpoints broadcast stays one `Uniform`
/// value per band, and per-row values only materialize on the first
/// observed divergence — the uniform prefix expands then — so the final
/// classification and its per-row values match the one-pass result
/// exactly.
#[derive(Debug, Default, PartialEq)]
pub(crate) enum RootColumn {
    /// No cells observed: the band has no rows, or its schema carries no
    /// root column.
    #[default]
    Unobserved,
    /// Every observed row's cell decodes to this value (`None` = a
    /// non-`Text` / null cell, matching `response_symbol`).
    Uniform(Option<Box<str>>),
    /// Per-row cell values, aligned with the band's rows.
    PerRow(Vec<Option<Box<str>>>),
}

impl RootColumn {
    /// Fold one non-empty chunk's root cells. `rows_before` is the number
    /// of band rows accumulated before this chunk — the length the
    /// uniform prefix expands to if this chunk breaks constancy.
    pub(crate) fn observe_chunk<'a, I>(&mut self, rows_before: usize, cells: I)
    where
        I: Iterator<Item = Option<&'a str>> + Clone,
    {
        let Some(first) = cells.clone().next() else {
            return;
        };
        let uniform = cells.clone().all(|cell| cell == first);
        match self {
            Self::Unobserved if uniform && rows_before == 0 => {
                *self = Self::Uniform(first.map(Into::into));
            }
            Self::Uniform(value) if uniform && value.as_deref() == first => {}
            _ => {
                let mut per_row = match std::mem::take(self) {
                    Self::PerRow(cells) => cells,
                    Self::Uniform(value) => vec![value; rows_before],
                    Self::Unobserved => vec![None; rows_before],
                };
                per_row.extend(cells.map(|cell| cell.map(Into::into)));
                *self = Self::PerRow(per_row);
            }
        }
    }

    /// The row's cell value under the final classification (`None` = a
    /// non-`Text` / null cell).
    fn cell(&self, index: usize) -> Option<&str> {
        match self {
            Self::Unobserved => None,
            Self::Uniform(value) => value.as_deref(),
            Self::PerRow(cells) => cells.get(index).and_then(|cell| cell.as_deref()),
        }
    }
}

/// A typed tick's contract-identity merge key, pre-ranked so the derived
/// `Ord` compares exactly like the buffered merge ranks wire key cells:
/// by `expiration`, then `strike`, then `right`, ascending with
/// `call < put`, and the parser's absent-cell seeds ranked LAST within
/// their column — the rank wire-null key cells sort to. The seeds are
/// unambiguous on a successful parse: `0` is not a valid `YYYYMMDD`
/// expiration, `'\0'` is not a decodable right, and a `0.0` strike is
/// not a listable contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ChainKey {
    /// `1` for the absent-cell seed (`expiration == 0`), ranking it last
    /// within the column.
    expiration_seeded: u8,
    expiration: i32,
    /// `1` for the absent-cell seed (`strike == 0.0`), ranking it last
    /// within the column.
    strike_seeded: u8,
    /// The strike under an order-preserving unsigned image of
    /// `f64::total_cmp`, keeping the derived `Ord` total.
    strike_bits: u64,
    /// `0` call, `1` put, `2` the absent-cell seed (`'\0'`) — the wire
    /// ranking `call < put < null`.
    right_rank: u8,
}

impl ChainKey {
    pub(crate) fn new(expiration: i32, strike: f64, right: char) -> Self {
        let bits = strike.to_bits();
        // Sign-magnitude → monotone unsigned: negatives invert (descending
        // magnitude becomes ascending), positives set the bit above every
        // negative. Identical ordering to `f64::total_cmp`.
        let strike_bits = if bits >> 63 == 0 {
            bits | (1 << 63)
        } else {
            !bits
        };
        Self {
            expiration_seeded: u8::from(expiration == 0),
            expiration,
            strike_seeded: u8::from(strike == 0.0),
            strike_bits,
            right_rank: match right {
                'C' => 0,
                'P' => 1,
                _ => 2,
            },
        }
    }
}

/// The contract-identity key a typed tick sorts chain responses by.
///
/// Implemented (generated) for every tick type from `tick_schema.toml`
/// next to its parser: `Some` for the chain-capable types
/// (`contract_id = true`, the injected `expiration` / `strike` / `right`
/// trio), `None` for every other type — whose sharded pulls concatenate
/// in band order.
pub(crate) trait ChainSortKey {
    /// This row's ranked merge key, `None` when the tick type carries no
    /// contract identity.
    fn chain_sort_key(&self) -> Option<ChainKey>;
}

/// One maximal run of equal-key rows within one band. The merge sorts
/// these descriptors — a few per contract per band — never the rows.
struct ChainRun {
    band: usize,
    start: usize,
    len: usize,
    key: Option<ChainKey>,
}

/// Merge per-shard typed bands (in band order) into the output row order,
/// producing the exact `Ticks` frame the single-stream buffered path
/// emits: same rows, same order, same column presence and symbol.
///
/// # NOTE: output order
///
/// * Concrete (single-contract / stock / index) responses: the server
///   sends rows ascending by `(date, ms_of_day)` — trading time. Bands
///   partition exactly that key (date bands partition `date`, time bands
///   partition `ms_of_day` within one date), so concatenating band
///   responses in band order IS the single-stream order, byte-exact.
///   These rows carry no distinguishing contract key — the tick type has
///   none (`chain_sort_key() == None`), or the injected trio is constant
///   or seeded — so every band folds to runs of one key, the stable
///   descriptor sort is an identity, and the gather is that
///   concatenation.
/// * Bulk chain responses: the server groups rows per contract — rights
///   `C` before `P` within a contract, `(date, ms_of_day)` ascending
///   within each `(contract, right)` — but enumerates the contract
///   groups in an internal order (observed live on a full SPXW chain:
///   strikes 4200, 8400, 7590, 7185, ...) that no client-visible key
///   reproduces, so the exact single-stream row order is not recoverable
///   from shard responses. The merge instead produces a DETERMINISTIC
///   canonical order: ascending `(expiration, strike, right)` with
///   `call < put` ([`ChainKey`]), stability carrying the time axis — for
///   one contract, rows from band k precede band k+1 (bands ascend in
///   time) and stay server-ordered within a band, so the within-contract
///   `(date, ms_of_day)` order is preserved without ever comparing
///   timestamps. The result is row-exact with the single stream
///   (live-verified: `Auto` and `Off` both return 1,593,184 rows on a
///   full SPXW quote chain) but canonically reordered; `bulk_fetch =
///   Off` yields the server's own enumeration.
///
/// # Row-order parity with the wire-cell comparator
///
/// This merge replaced a shape that concatenated the raw proto bands,
/// stable-sorted every row with a wire-cell comparator, and parsed the
/// merged table once. Sorting run descriptors of already-typed rows is
/// order-identical because, within one successful parse, each key
/// column's cells decode order-preservingly onto the typed key:
/// `expiration` ISO text and `YYYYMMDD` numbers order the same,
/// same-scale `Price` mantissas (i32, exact in `f64`) order as their
/// `f64`, `right` maps onto `'C'` < `'P'`, and wire-NULL key cells —
/// ranked after real values by the wire comparator — decode to the
/// parser seeds, which [`ChainKey`] also ranks last. A successful parse
/// admits no other cell shape. The divergences a hostile server could
/// manufacture — a key column mixing `Number` and `Text` encodings,
/// within one response or across shard responses (the wire comparator
/// ranked encoding before value), or `Number` strikes beyond `f64`'s
/// 2^53 integer range — do not occur on the wire: MDDS encodes each
/// field with one wire variant fixed by the endpoint schema, so every
/// row of every shard response of one query carries the same variant,
/// and strikes are dollar-scale. (The cross-band header check pins only
/// column names; variant constancy is that per-endpoint encoding, the
/// same property the single-stream path already relies on.) Were a field
/// to mix variants across shards regardless, the typed key would order
/// it by decoded value — chronological for `expiration`, numeric for
/// `strike` — a more meaningful order than the wire comparator's
/// encoding-first grouping, so the fallback is benign, not a regression.
///
/// Peak memory is the typed bands plus the gathered output — the merged
/// proto table (roughly an order of magnitude larger per row) is never
/// materialized, and the sort touches descriptors instead of shuffling
/// ~row-count proto cells.
///
/// # Errors
///
/// Returns a decode error when band headers disagree — the same schema
/// contract `collect_stream` enforces across chunks of one stream.
pub(crate) fn merge_typed_in_order<T>(bands: Vec<TypedBand<T>>) -> Result<Ticks<T>, Error>
where
    T: ChainSortKey + WireColumns + Clone,
{
    // Cross-band schema agreement — the same contract `collect_stream`
    // enforces across chunks of one stream, here across bands of one
    // logical query.
    let mut headers: &[String] = &[];
    for (band_index, band) in bands.iter().enumerate() {
        if headers.is_empty() {
            headers = &band.headers;
        } else if !band.headers.is_empty() && band.headers != headers {
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index: band_index,
                first: headers.join(","),
                chunk: band.headers.join(","),
            }
            .into());
        }
    }

    // One linear pass per band: maximal runs of rows with equal contract
    // key. A non-chain tick type (`chain_sort_key() == None`, uniform per
    // type) folds each band into one run, so the sort below degenerates
    // to band-order concatenation.
    let total_rows: usize = bands.iter().map(|band| band.rows.len()).sum();
    let mut runs: Vec<ChainRun> = Vec::new();
    for (band_index, band) in bands.iter().enumerate() {
        let Some(first) = band.rows.first() else {
            continue;
        };
        let mut key = first.chain_sort_key();
        if key.is_none() {
            runs.push(ChainRun {
                band: band_index,
                start: 0,
                len: band.rows.len(),
                key: None,
            });
            continue;
        }
        let mut start = 0;
        for (index, row) in band.rows.iter().enumerate().skip(1) {
            let row_key = row.chain_sort_key();
            if row_key != key {
                runs.push(ChainRun {
                    band: band_index,
                    start,
                    len: index - start,
                    key,
                });
                start = index;
                key = row_key;
            }
        }
        runs.push(ChainRun {
            band: band_index,
            start,
            len: band.rows.len() - start,
            key,
        });
    }

    // Stable sort of the descriptors on the ranked key; equal keys keep
    // band order, then server order within a band — the stability carrier
    // for within-contract time order. Keys are uniform per tick type
    // (`Some` for chain-capable, `None` otherwise), so the `None` arm
    // never mixes with `Some`.
    runs.sort_by(|a, b| match (&a.key, &b.key) {
        (Some(x), Some(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });

    // Gather: one allocation, then run-sized copies in descriptor order.
    let mut rows: Vec<T> = Vec::with_capacity(total_rows);
    for run in &runs {
        rows.extend_from_slice(&bands[run.band].rows[run.start..run.start + run.len]);
    }

    // Column presence + response symbol, exactly as the single-stream
    // path derives them from the merged table. A tick that owns a
    // `symbol` column emits it from its own field; otherwise classify
    // the response root column (constant broadcast / per-row / absent),
    // matching `response_symbol`.
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let mut columns = T::present_columns(&header_refs);
    if !columns.contains("symbol") && find_header(&header_refs, "root").is_some() {
        columns = match fold_root_columns(&bands) {
            RootFold::Empty => columns.with_symbol(""),
            RootFold::Constant(symbol) => columns.with_symbol(symbol),
            RootFold::Absent => columns,
            RootFold::Varying => {
                // One value per row, permuted by the same descriptors as
                // the rows so each row keeps its own cell.
                let mut symbols: Vec<Box<str>> = Vec::with_capacity(total_rows);
                for run in &runs {
                    let root = &bands[run.band].root;
                    symbols.extend(
                        (run.start..run.start + run.len)
                            .map(|index| Box::<str>::from(root.cell(index).unwrap_or(""))),
                    );
                }
                columns.with_symbols(symbols)
            }
        };
    }
    Ok(Ticks::new(rows, columns))
}

/// Merged classification of the bands' root columns — the cross-band
/// fold of [`RootColumn`], mirroring `response_symbol`'s row-level pass
/// over the merged table.
enum RootFold {
    /// Header present, zero rows in every band: `Constant("")`.
    Empty,
    /// Every row's cell is this `Text` value (broadcast).
    Constant(Box<str>),
    /// Every row's cell is non-`Text` / null.
    Absent,
    /// Cells vary across rows, within or across bands.
    Varying,
}

fn fold_root_columns<T>(bands: &[TypedBand<T>]) -> RootFold {
    let mut uniform: Option<Option<&str>> = None;
    for band in bands {
        if band.rows.is_empty() {
            continue;
        }
        let band_value = match &band.root {
            RootColumn::PerRow(_) => return RootFold::Varying,
            RootColumn::Uniform(value) => value.as_deref(),
            // A band with rows always classified its cells (its schema is
            // the shared band schema); `None` keeps the fold total anyway.
            RootColumn::Unobserved => None,
        };
        match uniform {
            None => uniform = Some(band_value),
            Some(seen) if seen == band_value => {}
            Some(_) => return RootFold::Varying,
        }
    }
    match uniform {
        None => RootFold::Empty,
        Some(Some(symbol)) => RootFold::Constant(symbol.into()),
        Some(None) => RootFold::Absent,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── date-range split (lifted from the Python binding, tests intact) ──

    #[test]
    fn single_day_stays_single_chunk() {
        let chunks = split_date_range("20240101", "20240101").unwrap();
        assert_eq!(chunks, vec![("20240101".into(), "20240101".into())]);
    }

    #[test]
    fn span_of_exactly_365_days_is_one_chunk() {
        // 20240101 → 20241230 is 365 days inclusive. Should not split.
        let chunks = split_date_range("20240101", "20241230").unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn span_of_366_days_splits_into_two_chunks() {
        // 20240101 → 20241231 is 366 days inclusive.
        let chunks = split_date_range("20240101", "20241231").unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0, "20240101");
        assert_eq!(chunks[0].1, "20241230");
        assert_eq!(chunks[1].0, "20241231");
        assert_eq!(chunks[1].1, "20241231");
    }

    #[test]
    fn invalid_yyyymmdd_returns_err() {
        let err = split_date_range("2024-01-01", "20241231").unwrap_err();
        assert!(matches!(err, ChunkError::InvalidDate(_, _)));
    }

    #[test]
    fn end_before_start_returns_err() {
        let err = split_date_range("20241231", "20240101").unwrap_err();
        assert!(matches!(err, ChunkError::EndBeforeStart { .. }));
    }

    #[test]
    fn ymd_round_trip_preserves_date() {
        for s in &["20240101", "19900315", "20261231", "20200229"] {
            let parsed = Ymd::from_yyyymmdd(s).unwrap();
            assert_eq!(&parsed.to_yyyymmdd(), *s, "round-trip failed for {s}");
            let reparsed = Ymd::from_ord(parsed.to_ord());
            assert_eq!(reparsed, parsed, "ordinal round-trip failed for {s}");
        }
    }

    #[test]
    fn impossible_gregorian_dates_are_rejected() {
        // Feb 29 outside a leap year, Feb 31, Apr 31, month 0/13, day 0,
        // century non-leap (1900) — the classic misvalidations.
        for s in [
            "20230229", "20240231", "20240431", "20240001", "20241301", "20240100", "19000229",
        ] {
            assert!(
                Ymd::from_yyyymmdd(s).is_err(),
                "expected {s} to be rejected"
            );
        }
        // 2000 is a quadricentennial leap year.
        assert!(Ymd::from_yyyymmdd("20000229").is_ok());
    }

    #[test]
    fn chunks_cover_exactly_the_requested_range() {
        let (start, end) = ("20200101", "20261231");
        let chunks = split_date_range(start, end).unwrap();
        let s = Ymd::from_yyyymmdd(start).unwrap().to_ord();
        let e = Ymd::from_yyyymmdd(end).unwrap().to_ord();
        let mut covered = 0;
        for (cs, ce) in &chunks {
            let c_start = Ymd::from_yyyymmdd(cs).unwrap().to_ord();
            let c_end = Ymd::from_yyyymmdd(ce).unwrap().to_ord();
            assert!(c_end - c_start < MAX_SPAN_DAYS);
            covered += c_end - c_start + 1;
        }
        assert_eq!(
            covered,
            e - s + 1,
            "chunks must cover the range exactly once"
        );
        assert_eq!(chunks.first().unwrap().0, start);
        assert_eq!(chunks.last().unwrap().1, end);
        for w in chunks.windows(2) {
            let prev_end = Ymd::from_yyyymmdd(&w[0].1).unwrap().to_ord();
            let next_start = Ymd::from_yyyymmdd(&w[1].0).unwrap().to_ord();
            assert_eq!(next_start, prev_end + 1, "chunks must be contiguous");
        }
    }

    // ── plan construction ──

    /// Full-chain single-day tick query over the regular session — the
    /// headline sharding shape.
    fn chain_day_query() -> ShardQuery {
        ShardQuery {
            symbol: Some("SPXW".into()),
            expiration: Some("*".into()),
            strike: Some("*".into()),
            right: Some("both".into()),
            date: Some("20260710".into()),
            start_time: Some("09:30:00".into()),
            end_time: Some("16:00:00".into()),
            interval: Some("tick".into()),
            ..ShardQuery::default()
        }
    }

    #[test]
    fn plan_cuts_a_chain_day_into_equal_time_bands() {
        let plan = plan_query("option_history_quote", &chain_day_query(), 8)
            .expect("full-day chain must shard");
        assert_eq!(plan.bands.len(), 8);
        let windows: Vec<(i64, i64)> = plan.bands.iter().map(band_window_ms).collect();
        assert_eq!(windows[0].0, parse_ms_of_day("09:30:00").unwrap());
        assert_eq!(
            windows.last().unwrap().1,
            parse_ms_of_day("16:00:00").unwrap()
        );
        for w in windows.windows(2) {
            assert_eq!(w[1].0, w[0].1 + 1, "bands must abut at +1 ms");
        }
    }

    #[test]
    fn narrow_window_reduces_band_count_or_declines() {
        // Too narrow for two minimum-width bands: single stream.
        let mut query = chain_day_query();
        query.end_time = Some("09:36:00".into());
        assert!(plan_query("option_history_quote", &query, 8).is_none());
        // Wide enough for exactly two: two bands, not the full width.
        let mut query = chain_day_query();
        query.end_time = Some("09:41:00".into());
        let plan = plan_query("option_history_quote", &query, 8).expect("two bands fit");
        assert_eq!(plan.bands.len(), 2);
    }

    #[test]
    fn width_under_two_declines() {
        assert!(plan_query("option_history_quote", &chain_day_query(), 1).is_none());
        assert!(plan_query("option_history_quote", &chain_day_query(), 0).is_none());
    }

    #[test]
    fn unlisted_endpoint_declines() {
        assert!(plan_query("option_history_eod", &chain_day_query(), 8).is_none());
        assert!(plan_query("stock_snapshot_quote", &chain_day_query(), 8).is_none());
    }

    #[test]
    fn bounded_interval_gates_concrete_contracts_not_chains() {
        // A concrete contract at a bounded bar interval is capped at the
        // bar grid — provably small over one session — so it stays on
        // the single stream.
        let mut concrete = chain_day_query();
        concrete.expiration = Some("20260710".into());
        concrete.strike = Some("6000".into());
        concrete.right = Some("call".into());
        concrete.interval = Some("1s".into());
        assert!(plan_query("option_history_quote", &concrete, 8).is_none());
        // The same contract at tick interval has no grid ceiling.
        concrete.interval = Some("tick".into());
        assert!(plan_query("option_history_quote", &concrete, 8).is_some());
        // A chain cross-product at the same bounded interval is unbounded
        // per grid slot and still shards.
        let mut chain = chain_day_query();
        chain.interval = Some("1s".into());
        assert!(plan_query("option_history_quote", &chain, 8).is_some());
    }

    #[test]
    fn multi_day_range_splits_into_equal_date_bands() {
        let query = ShardQuery {
            symbol: Some("AAPL".into()),
            start_date: Some("20240101".into()),
            end_date: Some("20240131".into()),
            interval: Some("tick".into()),
            ..ShardQuery::default()
        };
        let plan = plan_query("stock_history_trade", &query, 4).expect("31 days must shard");
        assert_eq!(plan.bands.len(), 4);
        // Bands are contiguous, cover the range exactly once, and their
        // day counts differ by at most one.
        let ords: Vec<(i64, i64)> = plan
            .bands
            .iter()
            .map(|b| match b {
                ShardBand::Date {
                    start_date,
                    end_date,
                } => (
                    Ymd::from_yyyymmdd(start_date).unwrap().to_ord(),
                    Ymd::from_yyyymmdd(end_date).unwrap().to_ord(),
                ),
                ShardBand::Time { .. } => panic!("expected date bands"),
            })
            .collect();
        assert_eq!(ords[0].0, Ymd::from_yyyymmdd("20240101").unwrap().to_ord());
        assert_eq!(
            ords.last().unwrap().1,
            Ymd::from_yyyymmdd("20240131").unwrap().to_ord()
        );
        for w in ords.windows(2) {
            assert_eq!(w[1].0, w[0].1 + 1, "date bands must be contiguous");
        }
        let days: Vec<i64> = ords.iter().map(|(s, e)| e - s + 1).collect();
        let (min, max) = (days.iter().min().unwrap(), days.iter().max().unwrap());
        assert!(max - min <= 1, "unequal band day counts: {days:?}");
        assert_eq!(days.iter().sum::<i64>(), 31);
    }

    #[test]
    fn date_bands_cap_at_one_band_per_day() {
        let query = ShardQuery {
            symbol: Some("AAPL".into()),
            start_date: Some("20240101".into()),
            end_date: Some("20240102".into()),
            interval: Some("tick".into()),
            ..ShardQuery::default()
        };
        let plan = plan_query("stock_history_trade", &query, 8).expect("two days shard");
        assert_eq!(plan.bands.len(), 2);
    }

    // ── axis selection ──

    fn q() -> ShardQuery {
        ShardQuery::default()
    }

    #[test]
    fn multi_day_range_selects_date_axis() {
        let mut query = q();
        query.start_date = Some("20240101".into());
        query.end_date = Some("20240301".into());
        assert_eq!(select_axis(&query), Some(ShardAxis::Date));
    }

    #[test]
    fn single_day_with_window_selects_time_axis() {
        let mut query = q();
        query.date = Some("20260710".into());
        query.start_time = Some("09:30:00".into());
        query.end_time = Some("16:00:00".into());
        assert_eq!(select_axis(&query), Some(ShardAxis::Time));
    }

    #[test]
    fn degenerate_one_day_range_selects_time_axis() {
        let mut query = q();
        query.start_date = Some("20260710".into());
        query.end_date = Some("20260710".into());
        query.start_time = Some("09:30:00".into());
        query.end_time = Some("16:00:00".into());
        assert_eq!(select_axis(&query), Some(ShardAxis::Time));
    }

    #[test]
    fn single_day_chain_without_window_is_not_sharded() {
        // A single-day option query with no intraday window has no usable
        // cut axis (dates cannot split one day, and no time window exists
        // to band), so it stays on the single stream.
        let mut query = q();
        query.date = Some("20260710".into());
        query.expiration = Some("20260710".into());
        query.right = Some("both".into());
        assert_eq!(select_axis(&query), None);
    }

    #[test]
    fn ambiguous_date_plus_range_is_not_sharded() {
        // Both a single `date` and a range: server precedence is its own;
        // never guess.
        let mut query = q();
        query.date = Some("20260710".into());
        query.start_date = Some("20260701".into());
        query.end_date = Some("20260731".into());
        query.start_time = Some("09:30:00".into());
        query.end_time = Some("16:00:00".into());
        assert_eq!(select_axis(&query), None);
    }

    #[test]
    fn no_axis_fields_is_not_sharded() {
        assert_eq!(select_axis(&q()), None);
    }

    // ── band construction ──

    /// Parsed inclusive `[start_ms, end_ms]` window of a time band.
    fn band_window_ms(b: &ShardBand) -> (i64, i64) {
        match b {
            ShardBand::Time {
                start_time,
                end_time,
            } => (
                parse_ms_of_day(start_time).unwrap(),
                parse_ms_of_day(end_time).unwrap(),
            ),
            ShardBand::Date { .. } => panic!("expected time bands"),
        }
    }

    #[test]
    fn time_bands_partition_the_window_exactly() {
        let start = parse_ms_of_day("09:30:00").unwrap();
        let end = parse_ms_of_day("16:00:00").unwrap();
        let bands = time_bands(start, end, 4);
        assert_eq!(bands.len(), 4);
        // First band starts at the query start, last ends at the query
        // end, and adjacent inclusive bands abut at exactly +1 ms so every
        // tick lands in one band.
        assert_eq!(band_window_ms(&bands[0]).0, start);
        assert_eq!(band_window_ms(bands.last().unwrap()).1, end);
        for w in bands.windows(2) {
            assert_eq!(band_window_ms(&w[1]).0, band_window_ms(&w[0]).1 + 1);
        }
        // Equal wall-clock spans, within the 1 ms division remainder.
        let spans: Vec<i64> = bands
            .iter()
            .map(|b| {
                let (s, e) = band_window_ms(b);
                e - s + 1
            })
            .collect();
        let (min, max) = (spans.iter().min().unwrap(), spans.iter().max().unwrap());
        assert!(max - min <= 1, "unequal band spans: {spans:?}");
    }

    #[test]
    fn time_bands_format_wire_canonical_times() {
        let start = parse_ms_of_day("09:30:00").unwrap();
        let end = parse_ms_of_day("16:00:00").unwrap();
        let bands = time_bands(start, end, 2);
        assert_eq!(
            bands,
            vec![
                ShardBand::Time {
                    start_time: "09:30:00.000".into(),
                    end_time: "12:44:59.999".into(),
                },
                ShardBand::Time {
                    start_time: "12:45:00.000".into(),
                    end_time: "16:00:00.000".into(),
                },
            ]
        );
    }

    #[test]
    fn date_bands_partition_the_range_exactly() {
        let s = Ymd::from_yyyymmdd("20240101").unwrap().to_ord();
        let e = Ymd::from_yyyymmdd("20240131").unwrap().to_ord();
        let bands = date_bands(s, e, 3);
        assert_eq!(
            bands,
            vec![
                ShardBand::Date {
                    start_date: "20240101".into(),
                    end_date: "20240110".into(),
                },
                ShardBand::Date {
                    start_date: "20240111".into(),
                    end_date: "20240120".into(),
                },
                ShardBand::Date {
                    start_date: "20240121".into(),
                    end_date: "20240131".into(),
                },
            ]
        );
    }

    // ── time / interval parsing ──

    #[test]
    fn parse_ms_of_day_accepts_wire_forms() {
        assert_eq!(parse_ms_of_day("09:30:00"), Some(34_200_000));
        assert_eq!(parse_ms_of_day("09:30"), Some(34_200_000));
        assert_eq!(parse_ms_of_day("16:00:00.000"), Some(57_600_000));
        assert_eq!(parse_ms_of_day("09:30:00.5"), Some(34_200_500));
        assert_eq!(parse_ms_of_day("34200000"), Some(34_200_000));
        assert_eq!(parse_ms_of_day("86400000"), None);
        assert_eq!(parse_ms_of_day("09:61"), None);
        assert_eq!(parse_ms_of_day("garbage"), None);
        // Negative components are rejected (keeps the range guard a total order).
        assert_eq!(parse_ms_of_day("-1:30:00"), None);
        assert_eq!(parse_ms_of_day("09:-5:00"), None);
    }

    #[test]
    fn chain_key_ranks_match_the_wire_comparator() {
        let key = ChainKey::new;
        // Column precedence: expiration, then strike, then right; call
        // before put.
        assert!(key(20260709, 999.0, 'P') < key(20260710, 1.0, 'C'));
        assert!(key(20260710, 50.0, 'P') < key(20260710, 100.0, 'C'));
        assert!(key(20260710, 100.0, 'C') < key(20260710, 100.0, 'P'));
        // The parser's absent-cell seeds rank LAST within their column —
        // the rank wire-null key cells sorted to.
        assert!(key(20991231, 1.0, 'C') < key(0, 1.0, 'C'));
        assert!(key(20260710, 100.0, 'P') < key(20260710, 0.0, 'C'));
        assert!(key(20260710, 100.0, 'P') < key(20260710, 100.0, '\0'));
        // Strikes order numerically across sign and fraction (total order
        // via the f64 bit map).
        assert!(key(20260710, -1.5, 'C') < key(20260710, 1.5, 'C'));
        assert!(key(20260710, 1.25, 'C') < key(20260710, 1.5, 'C'));
        // Equality is exact per column — run detection folds only rows of
        // one contract.
        assert_eq!(key(20260710, 100.0, 'C'), key(20260710, 100.0, 'C'));
        assert_ne!(key(20260710, 100.0, 'C'), key(20260710, 100.5, 'C'));
        assert_ne!(key(20260710, 100.0, 'C'), key(20260710, 100.0, 'P'));
    }

    #[test]
    fn interval_ms_understands_wire_spellings() {
        assert_eq!(interval_ms("tick"), None);
        assert_eq!(interval_ms("1s"), Some(1_000));
        assert_eq!(interval_ms("5m"), Some(300_000));
        assert_eq!(interval_ms("1h"), Some(3_600_000));
        assert_eq!(interval_ms("100ms"), Some(100));
        assert_eq!(interval_ms("60000"), Some(60_000));
        assert_eq!(interval_ms("weird"), None);
        // Oversized value overflows i64 * unit: must be None, never a panic.
        assert_eq!(interval_ms("9223372036854775h"), None);
    }

    // ── plan gates ──

    #[test]
    fn chain_detection_separates_wildcards_from_concrete_contracts() {
        // Full chain: wildcard strike + both rights.
        let mut chain = q();
        chain.expiration = Some("20260710".into());
        chain.strike = Some("*".into());
        chain.right = Some("both".into());
        assert!(chain_cross_product(&chain));
        // Wildcard expiration alone widens the universe.
        let mut exp_wild = q();
        exp_wild.expiration = Some("*".into());
        exp_wild.strike = Some("6000".into());
        exp_wild.right = Some("call".into());
        assert!(chain_cross_product(&exp_wild));
        // A fully concrete contract is not a chain.
        let mut concrete = q();
        concrete.expiration = Some("20260710".into());
        concrete.strike = Some("6000".into());
        concrete.right = Some("put".into());
        assert!(!chain_cross_product(&concrete));
        // Stock / index queries carry no expiration and are never chains.
        let mut stock = q();
        stock.symbol = Some("AAPL".into());
        assert!(!chain_cross_product(&stock));
    }

    #[test]
    fn shard_width_clamps_into_pool() {
        assert_eq!(shard_width(None, 8), 8);
        assert_eq!(shard_width(Some(4), 8), 4);
        assert_eq!(shard_width(Some(99), 8), 8);
        assert_eq!(shard_width(Some(1), 8), 1);
        assert_eq!(shard_width(Some(0), 8), 1); // validate floors this too
        assert_eq!(shard_width(None, 0), 1);
    }

    #[test]
    fn shardable_endpoints_cover_history_families_only() {
        assert!(is_shardable_history_endpoint("option_history_quote"));
        assert!(is_shardable_history_endpoint("stock_history_trade"));
        assert!(is_shardable_history_endpoint("index_history_price"));
        // Daily-only history (EOD / open interest / greeks-EOD) is
        // bounded at one row per day, so it is not shardable and
        // stays on the single stream, like snapshots, lists, and
        // at-time queries.
        assert!(!is_shardable_history_endpoint("option_history_eod"));
        assert!(!is_shardable_history_endpoint(
            "option_history_open_interest"
        ));
        assert!(!is_shardable_history_endpoint("stock_history_eod"));
        assert!(!is_shardable_history_endpoint("index_history_eod"));
        assert!(!is_shardable_history_endpoint("stock_snapshot_quote"));
        assert!(!is_shardable_history_endpoint("option_list_contracts"));
        assert!(!is_shardable_history_endpoint("stock_at_time_trade"));
    }

    // ── ordered merge ──

    use crate::columns::{present_columns_from, ColumnPresence, WireColumns};

    /// Typed stand-in for a chain-capable tick (`contract_id = true`):
    /// the injected identity trio plus a payload column that pins row
    /// identity and time order in the assertions.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct ChainTestTick {
        expiration: i32,
        strike: f64,
        right: char,
        ms_of_day: i64,
    }

    impl ChainSortKey for ChainTestTick {
        fn chain_sort_key(&self) -> Option<ChainKey> {
            Some(ChainKey::new(self.expiration, self.strike, self.right))
        }
    }

    impl WireColumns for ChainTestTick {
        fn present_columns(headers: &[&str]) -> ColumnPresence {
            present_columns_from(headers, &[("ms_of_day", "ms_of_day")], true)
        }
        fn all_columns() -> ColumnPresence {
            ColumnPresence::from_names(["ms_of_day", "expiration", "strike", "right"])
        }
    }

    /// Typed stand-in for a non-chain tick (the stock / index shapes,
    /// `chain_sort_key() == None`).
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct FlatTestTick {
        ms_of_day: i64,
    }

    impl ChainSortKey for FlatTestTick {
        fn chain_sort_key(&self) -> Option<ChainKey> {
            None
        }
    }

    impl WireColumns for FlatTestTick {
        fn present_columns(headers: &[&str]) -> ColumnPresence {
            present_columns_from(headers, &[("ms_of_day", "ms_of_day")], false)
        }
        fn all_columns() -> ColumnPresence {
            ColumnPresence::from_names(["ms_of_day"])
        }
    }

    fn tick(exp: i32, strike: f64, right: char, ms: i64) -> ChainTestTick {
        ChainTestTick {
            expiration: exp,
            strike,
            right,
            ms_of_day: ms,
        }
    }

    fn chain_headers() -> Vec<String> {
        ["expiration", "strike", "right", "ms_of_day"]
            .map(String::from)
            .to_vec()
    }

    fn chain_band(rows: Vec<ChainTestTick>) -> TypedBand<ChainTestTick> {
        TypedBand {
            headers: chain_headers(),
            rows,
            root: RootColumn::Unobserved,
        }
    }

    /// Band whose schema also carries the broadcast `root` column, with
    /// the given per-band fold state.
    fn rooted_band(rows: Vec<ChainTestTick>, root: RootColumn) -> TypedBand<ChainTestTick> {
        let mut headers = chain_headers();
        headers.push("root".to_string());
        TypedBand {
            headers,
            rows,
            root,
        }
    }

    #[test]
    fn merge_restores_contract_grouped_order_from_time_bands() {
        // Single-stream canonical order for two contracts A(=strike 100)
        // and B(=strike 200): all of A time-ascending, then all of B.
        // Time bands slice ACROSS contracts; each band arrives
        // contract-grouped within itself. The merge must reassemble the
        // canonical whole.
        let band1 = chain_band(vec![
            tick(20260710, 100.0, 'C', 1000),
            tick(20260710, 100.0, 'C', 2000),
            tick(20260710, 200.0, 'P', 1500),
        ]);
        let band2 = chain_band(vec![
            tick(20260710, 100.0, 'C', 5000),
            tick(20260710, 200.0, 'P', 4000),
            tick(20260710, 200.0, 'P', 6000),
        ]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let expected = [
            tick(20260710, 100.0, 'C', 1000),
            tick(20260710, 100.0, 'C', 2000),
            tick(20260710, 100.0, 'C', 5000),
            tick(20260710, 200.0, 'P', 1500),
            tick(20260710, 200.0, 'P', 4000),
            tick(20260710, 200.0, 'P', 6000),
        ];
        assert_eq!(merged.as_slice(), expected.as_slice());
    }

    #[test]
    fn merge_orders_call_before_put_within_a_strike() {
        // Rights interleave WITHIN (expiration, strike): C before P for
        // each pair, not all Cs before all Ps.
        let band1 = chain_band(vec![
            tick(20260710, 100.0, 'P', 1000),
            tick(20260710, 200.0, 'C', 1000),
        ]);
        let band2 = chain_band(vec![
            tick(20260710, 100.0, 'C', 4000),
            tick(20260710, 200.0, 'P', 4000),
        ]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let expected = [
            tick(20260710, 100.0, 'C', 4000),
            tick(20260710, 100.0, 'P', 1000),
            tick(20260710, 200.0, 'C', 1000),
            tick(20260710, 200.0, 'P', 4000),
        ];
        assert_eq!(merged.as_slice(), expected.as_slice());
    }

    #[test]
    fn merge_orders_expirations_before_strikes() {
        // Expiration is the leading key column: a lower expiration's
        // high strike precedes a higher expiration's low strike.
        let band1 = chain_band(vec![tick(20260724, 100.0, 'C', 1)]);
        let band2 = chain_band(vec![
            tick(20260710, 200.0, 'C', 2),
            tick(20260724, 50.0, 'C', 3),
        ]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let expected = [
            tick(20260710, 200.0, 'C', 2),
            tick(20260724, 50.0, 'C', 3),
            tick(20260724, 100.0, 'C', 1),
        ];
        assert_eq!(merged.as_slice(), expected.as_slice());
    }

    #[test]
    fn merge_is_stable_within_a_contract() {
        // Equal keys must keep band order (band k precedes band k+1), and
        // server order within a band — this is what carries time order
        // without comparing timestamps.
        let band1 = chain_band(vec![
            tick(20260710, 100.0, 'C', 1),
            tick(20260710, 100.0, 'C', 2),
        ]);
        let band2 = chain_band(vec![
            tick(20260710, 100.0, 'C', 3),
            tick(20260710, 100.0, 'C', 4),
        ]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let ms: Vec<i64> = merged.iter().map(|t| t.ms_of_day).collect();
        assert_eq!(ms, vec![1, 2, 3, 4]);
    }

    #[test]
    fn merge_ranks_seeded_null_keys_last() {
        // Wire-null key cells decode to the parser seeds (0 / 0.0 /
        // '\0'); the merge must rank them where the wire comparator
        // ranked the null cells — after every real value in that column —
        // with band order preserved among equal seeded keys.
        let band1 = chain_band(vec![
            tick(20260710, 100.0, '\0', 3), // C: null right, real exp+strike
            tick(0, 100.0, 'C', 5),         // E: null expiration
        ]);
        let band2 = chain_band(vec![
            tick(20260710, 0.0, 'C', 4),   // D: null strike
            tick(20260710, 100.0, 'C', 1), // A
            tick(20260710, 100.0, 'P', 2), // B
            tick(0, 100.0, 'C', 6),        // F: equal seeded key as E, later band
        ]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let ms: Vec<i64> = merged.iter().map(|t| t.ms_of_day).collect();
        assert_eq!(ms, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn merge_without_contract_key_concatenates_in_band_order() {
        // Non-chain tick types (stock / index responses): no contract
        // key, bands already partition trading time — concatenation IS
        // canonical.
        let make = |ms: &[i64]| TypedBand {
            headers: vec!["ms_of_day".to_string(), "price".to_string()],
            rows: ms.iter().map(|&m| FlatTestTick { ms_of_day: m }).collect(),
            root: RootColumn::Unobserved,
        };
        let merged = merge_typed_in_order(vec![make(&[1, 2, 3]), make(&[4, 5])]).unwrap();
        let ms: Vec<i64> = merged.iter().map(|t| t.ms_of_day).collect();
        assert_eq!(ms, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn merge_with_constant_key_concatenates_in_band_order() {
        // A single-contract chain-typed pull: the injected trio is
        // constant, so every band is one run and band order (trading
        // time) is the output order — the wire comparator's stable-sort
        // fixpoint.
        let band1 = chain_band(vec![
            tick(20260710, 100.0, 'C', 1),
            tick(20260710, 100.0, 'C', 2),
        ]);
        let band2 = chain_band(vec![tick(20260710, 100.0, 'C', 3)]);
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        let ms: Vec<i64> = merged.iter().map(|t| t.ms_of_day).collect();
        assert_eq!(ms, vec![1, 2, 3]);
    }

    #[test]
    fn merge_tolerates_empty_bands_and_takes_first_headers() {
        let empty = TypedBand {
            headers: Vec::new(),
            rows: Vec::new(),
            root: RootColumn::Unobserved,
        };
        let band2 = chain_band(vec![tick(20260710, 100.0, 'C', 1)]);
        let merged = merge_typed_in_order(vec![empty, band2]).unwrap();
        assert_eq!(merged.len(), 1);
        // Presence comes from the first non-empty band schema, exactly
        // as the single-stream path derives it from the merged headers.
        assert!(merged.columns().contains("ms_of_day"));
        assert!(merged.columns().contains("expiration"));
        assert!(merged.columns().contains("strike"));
        assert!(merged.columns().contains("right"));
    }

    #[test]
    fn merge_rejects_band_header_drift() {
        let band1 = chain_band(vec![tick(20260710, 100.0, 'C', 1)]);
        let band2 = TypedBand {
            headers: vec!["something".to_string(), "else".to_string()],
            rows: vec![tick(20260710, 100.0, 'C', 2)],
            root: RootColumn::Unobserved,
        };
        let err = merge_typed_in_order(vec![band1, band2]).unwrap_err();
        assert!(
            matches!(err, Error::Decode { .. }),
            "expected a decode-class schema error, got {err:?}"
        );
    }

    #[test]
    fn merge_broadcasts_the_constant_root_symbol() {
        // Every shardable history endpoint sends one constant root; the
        // merged frame must carry it exactly as the single-stream
        // `response_symbol` pass does.
        let band1 = rooted_band(
            vec![tick(20260710, 100.0, 'C', 1)],
            RootColumn::Uniform(Some("SPXW".into())),
        );
        let band2 = rooted_band(
            vec![tick(20260710, 200.0, 'C', 2)],
            RootColumn::Uniform(Some("SPXW".into())),
        );
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        assert_eq!(merged.columns().symbol(), Some("SPXW"));
        assert_eq!(merged.columns().symbols(), None);
    }

    #[test]
    fn merge_gathers_varying_root_symbols_in_merged_row_order() {
        // Divergent root cells fall back to per-row symbols — permuted by
        // the same run descriptors as the rows, so each row keeps its own
        // value. Band 2's row sorts FIRST (lower strike), so its symbol
        // must lead.
        let band1 = rooted_band(
            vec![tick(20260710, 200.0, 'C', 1)],
            RootColumn::Uniform(Some("AAA".into())),
        );
        let band2 = rooted_band(
            vec![tick(20260710, 100.0, 'C', 2)],
            RootColumn::Uniform(Some("BBB".into())),
        );
        let merged = merge_typed_in_order(vec![band1, band2]).unwrap();
        assert_eq!(merged.columns().symbol(), None);
        let symbols: Vec<&str> = merged
            .columns()
            .symbols()
            .expect("varying root must attach per-row symbols")
            .iter()
            .map(AsRef::as_ref)
            .collect();
        assert_eq!(symbols, vec!["BBB", "AAA"]);
    }

    #[test]
    fn merge_fills_empty_string_for_non_text_root_cells_when_varying() {
        // A per-row fold carries `None` for non-Text/null cells; the
        // emitted per-row symbol is "" for those rows, matching
        // `response_symbol`.
        let band = rooted_band(
            vec![tick(20260710, 100.0, 'C', 1), tick(20260710, 100.0, 'C', 2)],
            RootColumn::PerRow(vec![Some("SPXW".into()), None]),
        );
        let merged = merge_typed_in_order(vec![band]).unwrap();
        let symbols: Vec<&str> = merged
            .columns()
            .symbols()
            .expect("per-row root must attach per-row symbols")
            .iter()
            .map(AsRef::as_ref)
            .collect();
        assert_eq!(symbols, vec!["SPXW", ""]);
    }

    #[test]
    fn merge_root_classification_edge_shapes_match_response_symbol() {
        // All-null root cells → no symbol (Absent).
        let band = rooted_band(
            vec![tick(20260710, 100.0, 'C', 1)],
            RootColumn::Uniform(None),
        );
        let merged = merge_typed_in_order(vec![band]).unwrap();
        assert_eq!(merged.columns().symbol(), None);
        assert_eq!(merged.columns().symbols(), None);

        // Root header present but zero rows in every band → Constant("")
        // so the column set stays keyed on the header.
        let merged =
            merge_typed_in_order(vec![rooted_band(vec![], RootColumn::Unobserved)]).unwrap();
        assert_eq!(merged.columns().symbol(), Some(""));

        // No root header at all → Absent, whatever the fold says.
        let band = chain_band(vec![tick(20260710, 100.0, 'C', 1)]);
        let merged = merge_typed_in_order(vec![band]).unwrap();
        assert_eq!(merged.columns().symbol(), None);
    }

    #[test]
    fn root_column_fold_materializes_per_row_only_on_divergence() {
        // Uniform chunks fold to one value...
        let mut root = RootColumn::Unobserved;
        root.observe_chunk(0, [Some("SPXW"), Some("SPXW")].into_iter());
        root.observe_chunk(2, [Some("SPXW")].into_iter());
        assert_eq!(root, RootColumn::Uniform(Some("SPXW".into())));

        // ...and the first divergent chunk expands the uniform prefix so
        // per-row values stay row-aligned.
        root.observe_chunk(3, [Some("QQQ"), None].into_iter());
        assert_eq!(
            root,
            RootColumn::PerRow(vec![
                Some("SPXW".into()),
                Some("SPXW".into()),
                Some("SPXW".into()),
                Some("QQQ".into()),
                None,
            ])
        );

        // An internally-varying first chunk materializes immediately.
        let mut root = RootColumn::Unobserved;
        root.observe_chunk(0, [Some("A"), Some("B")].into_iter());
        assert_eq!(
            root,
            RootColumn::PerRow(vec![Some("A".into()), Some("B".into())])
        );

        // Non-Text cells fold as `None` without breaking constancy.
        let mut root = RootColumn::Unobserved;
        root.observe_chunk(0, [None, None].into_iter());
        root.observe_chunk(2, [None].into_iter());
        assert_eq!(root, RootColumn::Uniform(None));
    }

    #[test]
    fn generated_chain_sort_key_covers_contract_id_types() {
        // Pin the codegen: a `contract_id = true` tick yields the ranked
        // key of its injected trio; a non-chain tick yields `None` (the
        // concat path).
        let trade = crate::TradeTick {
            ms_of_day: 1,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 0,
            exchange: 0,
            price: 0.0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20260710,
            expiration: 20260717,
            strike: 6000.0,
            right: 'P',
        };
        assert_eq!(
            trade.chain_sort_key(),
            Some(ChainKey::new(20260717, 6000.0, 'P'))
        );
        let day = crate::tdbe::types::tick::CalendarDay {
            date: 20260710,
            open_time: 0,
            close_time: 0,
            status: crate::tdbe::CalendarStatus::Open,
        };
        assert_eq!(day.chain_sort_key(), None);
    }

    // ── shard join: empty-band (NotFound) folding ──

    fn not_found() -> Error {
        Error::Grpc {
            kind: GrpcStatusKind::NotFound,
            message: "No data found for your request".into(),
            retry_after: None,
        }
    }

    #[tokio::test]
    async fn join_folds_not_found_shard_to_empty() {
        // Bands partition the query; one band can be empty (the server
        // answers NotFound) while the union has data. The empty band
        // must contribute zero rows, not fail the pull.
        let with_data = chain_band(vec![tick(20260710, 100.0, 'C', 1)]);
        let tasks = vec![
            spawn_shard(async move { Ok(with_data) }),
            spawn_shard(async move { Err::<TypedBand<ChainTestTick>, _>(not_found()) }),
        ];
        let bands = join_shards(tasks).await.expect("sibling band has data");
        let merged = merge_typed_in_order(bands).unwrap();
        assert_eq!(merged.as_slice(), &[tick(20260710, 100.0, 'C', 1)]);
    }

    #[tokio::test]
    async fn join_propagates_not_found_when_every_shard_is_empty() {
        // All bands empty means the whole query is empty: surface the
        // same NotFound a single stream returns for it.
        let tasks = vec![
            spawn_shard(async move { Err::<TypedBand<ChainTestTick>, _>(not_found()) }),
            spawn_shard(async move { Err::<TypedBand<ChainTestTick>, _>(not_found()) }),
        ];
        let err = join_shards(tasks).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Grpc {
                    kind: GrpcStatusKind::NotFound,
                    ..
                }
            ),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn join_all_empty_including_zero_row_shard_is_not_found() {
        // A NotFound band folds to empty; a sibling that opens but yields
        // zero rows is also empty. The union is empty, so surface NotFound
        // rather than a spurious empty frame.
        let empty = chain_band(vec![]);
        let tasks = vec![
            spawn_shard(async move { Err::<TypedBand<ChainTestTick>, _>(not_found()) }),
            spawn_shard(async move { Ok(empty) }),
        ];
        let err = join_shards(tasks).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Grpc {
                    kind: GrpcStatusKind::NotFound,
                    ..
                }
            ),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn join_propagates_real_errors_unchanged() {
        // Only the empty-band status folds; any other error fails the
        // pull even when a sibling shard has data.
        let with_data = chain_band(vec![tick(20260710, 100.0, 'C', 1)]);
        let tasks = vec![
            spawn_shard(async move { Ok(with_data) }),
            spawn_shard(async move {
                Err::<TypedBand<ChainTestTick>, _>(Error::Grpc {
                    kind: GrpcStatusKind::PermissionDenied,
                    message: String::new(),
                    retry_after: None,
                })
            }),
        ];
        let err = join_shards(tasks).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Grpc {
                    kind: GrpcStatusKind::PermissionDenied,
                    ..
                }
            ),
            "expected PermissionDenied, got {err:?}"
        );
    }

    // ── streaming shard join: concurrent forward, abort, NotFound ──

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// Sets its flag on drop — stands in for the in-flight state a band
    /// future owns (`ServerStreaming`, semaphore permit): if the join
    /// drops the future, the "stream" was aborted.
    struct AbortFlag(Arc<AtomicBool>);

    impl Drop for AbortFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// A band future that never completes until dropped.
    fn pending_band(aborted: Arc<AtomicBool>) -> ShardStreamFuture<'static> {
        Box::pin(async move {
            let _guard = AbortFlag(aborted);
            std::future::pending().await
        })
    }

    /// A band future shaped like the macro's shard body: acquire an own
    /// permit, then hand each chunk to the shared handler under its lock,
    /// yielding between chunks so sibling bands interleave.
    fn chunk_band(
        semaphore: Arc<tokio::sync::Semaphore>,
        handler: Arc<Mutex<Vec<u32>>>,
        chunks: Vec<Vec<u32>>,
    ) -> ShardStreamFuture<'static> {
        Box::pin(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|_| Error::config_internal("request semaphore closed"))?;
            let mut delivered = false;
            for chunk in chunks {
                tokio::task::yield_now().await;
                let mut h = handler.lock().unwrap();
                delivered |= !chunk.is_empty();
                h.extend(chunk);
            }
            Ok(delivered)
        })
    }

    #[tokio::test]
    async fn streaming_join_delivers_every_chunk_exactly_once() {
        // Three bands partition the logical stream; the union of their
        // chunks must equal the single-stream row set — every row once,
        // arrival order across bands unconstrained.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let handler = Arc::new(Mutex::new(Vec::new()));
        let shards = vec![
            chunk_band(
                Arc::clone(&semaphore),
                Arc::clone(&handler),
                vec![vec![1, 2], vec![3]],
            ),
            chunk_band(Arc::clone(&semaphore), Arc::clone(&handler), vec![vec![4]]),
            chunk_band(
                Arc::clone(&semaphore),
                Arc::clone(&handler),
                vec![vec![5], vec![6, 7]],
            ),
        ];
        join_streaming_shards(shards).await.expect("all bands ok");
        let mut rows = std::mem::take(&mut *handler.lock().unwrap());
        rows.sort_unstable();
        assert_eq!(rows, vec![1, 2, 3, 4, 5, 6, 7]);
    }

    #[tokio::test]
    async fn streaming_join_serializes_through_a_one_permit_pool() {
        // The joining driver holds no permit; each band takes its own.
        // At pool size 1 the bands must serialize and complete rather
        // than deadlock.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let handler = Arc::new(Mutex::new(Vec::new()));
        let shards = (0..3u32)
            .map(|band| {
                chunk_band(
                    Arc::clone(&semaphore),
                    Arc::clone(&handler),
                    vec![vec![band]],
                )
            })
            .collect();
        join_streaming_shards(shards)
            .await
            .expect("bands serialize");
        assert_eq!(handler.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn streaming_join_first_error_aborts_sibling_bands() {
        // A real band error fails the whole logical stream as soon as it
        // lands; the still-pending sibling must be dropped (its stream
        // aborted) rather than awaited.
        let aborted = Arc::new(AtomicBool::new(false));
        let shards = vec![
            pending_band(Arc::clone(&aborted)),
            Box::pin(async move {
                tokio::task::yield_now().await;
                Err(Error::Grpc {
                    kind: GrpcStatusKind::PermissionDenied,
                    message: String::new(),
                    retry_after: None,
                })
            }) as ShardStreamFuture<'static>,
        ];
        let err = join_streaming_shards(shards).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Grpc {
                    kind: GrpcStatusKind::PermissionDenied,
                    ..
                }
            ),
            "expected PermissionDenied, got {err:?}"
        );
        assert!(
            aborted.load(Ordering::Relaxed),
            "sibling band must be dropped (aborted) on the first error"
        );
    }

    #[tokio::test]
    async fn streaming_join_folds_not_found_band_to_zero_chunks() {
        // Bands partition the query; one band can be empty (NotFound)
        // while the union has data. The empty band contributes nothing
        // and must not fail the pull.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
        let handler = Arc::new(Mutex::new(Vec::new()));
        let shards = vec![
            chunk_band(
                Arc::clone(&semaphore),
                Arc::clone(&handler),
                vec![vec![1, 2]],
            ),
            Box::pin(async move { Err(not_found()) }) as ShardStreamFuture<'static>,
        ];
        join_streaming_shards(shards)
            .await
            .expect("sibling band has data");
        assert_eq!(*handler.lock().unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn streaming_join_all_empty_bands_propagate_not_found() {
        // All bands empty — NotFound-folded, or completed without
        // delivering a chunk — means the whole query is empty: surface
        // the same NotFound a single stream returns for it.
        let shards = vec![
            Box::pin(async move { Err(not_found()) }) as ShardStreamFuture<'static>,
            Box::pin(async move { Ok(false) }) as ShardStreamFuture<'static>,
        ];
        let err = join_streaming_shards(shards).await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Grpc {
                    kind: GrpcStatusKind::NotFound,
                    ..
                }
            ),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn dropping_the_streaming_join_aborts_in_flight_bands() {
        // The deadline contract: `run_with_optional_deadline` cancels by
        // dropping the in-flight future, so dropping the join future must
        // drop (abort) every band stream.
        let aborted = Arc::new(AtomicBool::new(false));
        let mut fut = Box::pin(join_streaming_shards(vec![pending_band(Arc::clone(
            &aborted,
        ))]));
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        assert!(std::future::Future::poll(fut.as_mut(), &mut cx).is_pending());
        assert!(!aborted.load(Ordering::Relaxed), "band is in flight");
        drop(fut);
        assert!(
            aborted.load(Ordering::Relaxed),
            "dropping the join must abort the in-flight band"
        );
    }

    // ── wire-field projection ──

    #[test]
    fn wire_param_projection_covers_both_proto_spellings() {
        let plain = "20240101".to_string();
        assert_eq!(opt_wire_str(&plain), Some("20240101".to_string()));
        assert_eq!(opt_wire_str(&String::new()), None);
        let optional: Option<String> = Some("09:30:00".into());
        assert_eq!(opt_wire_str(&optional), Some("09:30:00".to_string()));
        assert_eq!(opt_wire_str(&Option::<String>::None), None);
        let multi = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert_eq!(opt_wire_str(&multi), None);

        let mut plain_dst = String::new();
        set_wire_str(&mut plain_dst, "20240102");
        assert_eq!(plain_dst, "20240102");
        let mut opt_dst: Option<String> = None;
        set_wire_str(&mut opt_dst, "10:00:00.000");
        assert_eq!(opt_dst.as_deref(), Some("10:00:00.000"));
    }
}
