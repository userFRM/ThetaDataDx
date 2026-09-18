//! Live-state views over the streaming feed.
//!
//! A model cannot read a feed: ticks arrive orders of magnitude faster than
//! tokens. What works is holding the subscription and answering questions
//! about it — what is the value now, what did the window look like, what
//! printed and where the market was around it.
//!
//! A buffer is named by its contract and kind, the same way the vendor names a
//! subscription. The first read opens it; a read every so often keeps it;
//! fifteen idle minutes or `live_stop` close it. There is no handle to
//! mint, pass back or lose. A whole-market buffer is named by its security
//! type alone, the way the vendor names a full-stream subscription.
//!
//! The feed answers a subscribe after accepting it, and can refuse it then.
//! The SDK's active set is the only record of what the feed kept, so every
//! read checks the buffer it is about against that set rather than trusting
//! the subscribe that opened it, and every close keeps the buffer until the
//! feed has let go.
//!
//! This layer stores what the feed sends and serves it back. Nothing is
//! aggregated and nothing is derived: a read returns the vendor's own
//! messages, how many of them the window held, and what it cannot vouch for.
//! Bar construction has condition, cancel and size rules that are the
//! caller's to choose, and a bar built on assumptions here would not
//! reconcile with one built anywhere else, so the vendor's own bar is served
//! as it arrived.
//!
//! The whole-market read is the exception, and only because it has to be: at
//! the rates that stream runs, handing back the rows is not something a
//! caller reading text can use, so it narrows and ranks on the vendor's own
//! fields and says what it selected from.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonic_rs::{json, JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, FullSubscriptionKind, OptionLeg, SecTypeExt, StreamControl, StreamData, StreamEvent,
    Subscription, SubscriptionKind,
};
use thetadatadx::{
    Client, ConnectionStatus, Error, SecType, StreamMsgType, StreamResponseType, SubscriptionTier,
};

use crate::{sanitize_error, ToolError};

/// Memory a buffer's ring may hold: 13,107 rows at the 80-byte row measured
/// on this build. Capacity in rows follows from the row size, so the
/// number says what a full buffer costs; what it covers in time depends on
/// the rate, and `covers_seconds` reports that at each read.
///
/// Measured on the production feed at the open of 2026-09-16
/// (`thetadatadx-rs/examples/stream_rate.rs`): the whole stock market
/// peaked at 672,350 msg/s over 100 ms and averaged 3,366 msg/s over the
/// session; the whole option market peaked at 121,630 msg/s and averaged
/// 1,113 msg/s. The peak was 41 times the mean of the minute that held
/// it. No single buffer sees more than its whole market, so this budget
/// covered at least 20 ms at that stock burst and about 4 s at that
/// session mean — two figures two hundred times apart, both true, which
/// is why nothing here promises a length of time.
const BUFFER_BUDGET: usize = 1 << 20;
const RING: usize = BUFFER_BUDGET / size_of::<StreamData>();
/// Memory a contract's prints may hold: 762 prints at the sizes measured
/// on this build. A print carries its trade and up to three quotes, one
/// inline and two on the heap. The rates above bound what it covers: a
/// contract prints no faster than its whole market trades.
const PRINTS_BUDGET: usize = 256 << 10;
/// Prints retained per contract.
const PRINTS: usize = PRINTS_BUDGET / (size_of::<Print>() + 2 * size_of::<StreamData>());
/// Clauses a selection may hold. Every clause runs on every print of a
/// selected market, under the lock the dispatcher records ticks on, so the
/// bound is on the dispatcher's time, not on what a caller may mean.
const MAX_CLAUSES: usize = 8;
/// How long to wait for a retired dispatcher to finish before a new
/// session replaces it. Long enough for a queue to drain, short enough that
/// a caller hears about it rather than waiting.
const DRAIN: Duration = Duration::from_secs(2);
/// How long a buffer survives without a read. Stated in the tool descriptions,
/// since that is where a model reads it.
const TTL: Duration = Duration::from_secs(900);
/// Newest rows served verbatim on a read, unless asked otherwise.
const TAIL: usize = 10;
/// Most rows a caller can take verbatim in one read.
///
/// The cap is not about response size. Rows are copied while the registry
/// lock is held, and the streaming dispatcher needs that same lock to record
/// a tick, so an unbounded tail lets one request stall the feed for as long
/// as it takes to clone the ring. Fifty rows is more than a model can use
/// and short enough that the critical section stays negligible.
const TAIL_MAX: usize = 50;

/// Rows to serve verbatim for a requested tail.
///
/// Pulled out of the request arm so the bound is reachable by a test: the
/// arm around it needs a live client, and a cap that only exists inside an
/// untestable branch is a cap nobody can prove is there.
fn tail_rows(requested: usize) -> usize {
    requested.min(TAIL_MAX)
}

/// Subscriptions to open or close on the feed, in the SDK's own type.
type Subs = Vec<Subscription>;
/// Per-contract and full-stream subscriptions in the two tuple shapes the
/// SDK's `restore_subscriptions` takes.
type Restore = (
    Vec<(SubscriptionKind, Contract)>,
    Vec<(SubscriptionKind, SecType)>,
);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// When the decoder saw this row, in milliseconds since the epoch.
///
/// Every variant carries `received_at_ns` stamped at decode, which is what
/// makes an honest age possible on a read.
/// When this row reached us, or `None` if that cannot be said.
///
/// The SDK stamps a row it could not read the host clock for with nought, so
/// nought is the absence of a stamp rather than the epoch. Reading it as a
/// time is how an age of fifty-five years, and a window claiming to cover
/// them, get served as though the clock had worked.
fn seen_ms(data: &StreamData) -> Option<u64> {
    let ns = match data {
        StreamData::Quote { received_at_ns, .. }
        | StreamData::Trade { received_at_ns, .. }
        | StreamData::OpenInterest { received_at_ns, .. }
        | StreamData::Ohlcvc { received_at_ns, .. }
        | StreamData::MarketValue { received_at_ns, .. } => *received_at_ns,
        _ => return None,
    };
    Some(ns / 1_000_000).filter(|ms| *ms > 0)
}

fn contract_of(data: &StreamData) -> Option<&Contract> {
    match data {
        StreamData::Quote { contract, .. }
        | StreamData::Trade { contract, .. }
        | StreamData::OpenInterest { contract, .. }
        | StreamData::Ohlcvc { contract, .. }
        | StreamData::MarketValue { contract, .. } => Some(contract),
        _ => None,
    }
}

/// The subscription code a stored row answers to. `StreamMsgType` is the
/// SDK's own discriminant for the shape, so nothing here re-derives it.
fn msg_type_of(data: &StreamData) -> Option<StreamMsgType> {
    Some(match data {
        StreamData::Quote { .. } => StreamMsgType::Quote,
        StreamData::Trade { .. } => StreamMsgType::Trade,
        StreamData::OpenInterest { .. } => StreamMsgType::OpenInterest,
        StreamData::MarketValue { .. } => StreamMsgType::MarketValue,
        _ => return None,
    })
}

fn date_of(data: &StreamData) -> Option<i32> {
    match data {
        StreamData::Quote { date, .. }
        | StreamData::Trade { date, .. }
        | StreamData::OpenInterest { date, .. }
        | StreamData::Ohlcvc { date, .. }
        | StreamData::MarketValue { date, .. } => Some(*date),
        _ => None,
    }
}

/// Every numeric column `fields` renders, plus `spread`, which is ask minus
/// bid on one quote message and the one name here the vendor does not send.
///
/// This is wider than what a selection can narrow or rank on, which is
/// [`PRINT_FIELDS`]. It exists so that a name belonging to some other row
/// can be refused as that rather than as no field at all: a caller ranking a
/// selection by `open_interest` is told it is not a field these rows carry,
/// instead of being handed the bare list and left to guess why.
const FIELDS: [&str; 18] = [
    "price",
    "size",
    "condition",
    "exchange",
    "sequence",
    "bid",
    "ask",
    "spread",
    "bid_size",
    "ask_size",
    "bid_exchange",
    "ask_exchange",
    "bid_condition",
    "ask_condition",
    "open_interest",
    "market_bid",
    "market_ask",
    "market_price",
];

/// The names a print carries: its trade's fields and its quote's. The full
/// trade stream sends no open-interest or market-value row, so offering
/// those to `live_market` would invite a selection nothing can ever pass.
const PRINT_FIELDS: [&str; 14] = [
    "price",
    "size",
    "condition",
    "exchange",
    "sequence",
    "bid",
    "ask",
    "spread",
    "bid_size",
    "ask_size",
    "bid_exchange",
    "ask_exchange",
    "bid_condition",
    "ask_condition",
];

/// A numeric field of a row by name; `None` when the row has no such field.
///
/// Reads a trade or the quote before it, which is what a print carries and
/// the only thing a selection ranks or narrows on. Runs on the dispatcher
/// thread for every print of a selected market, so it reads the enum and
/// allocates nothing.
fn field_of(data: &StreamData, name: &str) -> Option<f64> {
    let v = match data {
        StreamData::Quote {
            bid_size,
            bid_exchange,
            bid,
            bid_condition,
            ask_size,
            ask_exchange,
            ask,
            ask_condition,
            ..
        } => match name {
            "bid" => *bid,
            "ask" => *ask,
            "spread" => ask - bid,
            "bid_size" => f64::from(*bid_size),
            "ask_size" => f64::from(*ask_size),
            "bid_exchange" => f64::from(*bid_exchange),
            "ask_exchange" => f64::from(*ask_exchange),
            "bid_condition" => f64::from(*bid_condition),
            "ask_condition" => f64::from(*ask_condition),
            _ => return None,
        },
        StreamData::Trade {
            sequence,
            condition,
            size,
            exchange,
            price,
            ..
        } => match name {
            "price" => *price,
            "size" => f64::from(*size),
            "condition" => f64::from(*condition),
            "exchange" => f64::from(*exchange),
            "sequence" => f64::from(*sequence),
            _ => return None,
        },
        _ => return None,
    };
    Some(v)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

impl Op {
    const ALL: [Self; 6] = [Self::Gt, Self::Ge, Self::Lt, Self::Le, Self::Eq, Self::Ne];

    fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    fn holds(self, lhs: f64, rhs: f64) -> bool {
        match self {
            Self::Gt => lhs > rhs,
            Self::Ge => lhs >= rhs,
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Eq => lhs == rhs,
            Self::Ne => lhs != rhs,
        }
    }
}

/// The right-hand side of a comparison: a number, or another field of the
/// same row, which is how a crossed market (`bid >= ask`) is asked for.
#[derive(Clone, Debug, PartialEq)]
enum Rhs {
    Number(f64),
    Field(String),
}

/// One condition over a row's vendor fields. A row lacking the field does
/// not match; a predicate is every clause holding at once.
#[derive(Clone, Debug, PartialEq)]
enum Clause {
    Compare {
        field: String,
        op: Op,
        rhs: Rhs,
    },
    Range {
        field: String,
        lo: f64,
        hi: f64,
        inside: bool,
    },
}

impl Clause {
    fn holds(&self, get: &dyn Fn(&str) -> Option<f64>) -> bool {
        match self {
            Self::Compare { field, op, rhs } => {
                let (Some(l), Some(r)) = (
                    get(field),
                    match rhs {
                        Rhs::Number(n) => Some(*n),
                        Rhs::Field(f) => get(f),
                    },
                ) else {
                    return false;
                };
                op.holds(l, r)
            }
            Self::Range {
                field,
                lo,
                hi,
                inside,
            } => get(field).is_some_and(|v| (v >= *lo && v <= *hi) == *inside),
        }
    }
}

fn holds_all(clauses: &[Clause], get: &dyn Fn(&str) -> Option<f64>) -> bool {
    clauses.iter().all(|c| c.holds(get))
}

/// A field name from `fields`, the names the rows in question carry. A
/// name that exists on some other row is refused in words, so a caller
/// who asked a print for its open interest learns why nothing would ever
/// match rather than reading an empty answer forever.
fn field_name(v: Option<&Value>, what: &str, fields: &[&str]) -> Result<String, ToolError> {
    let name = v.and_then(|v: &Value| v.as_str()).unwrap_or_default();
    if fields.contains(&name) {
        Ok(name.to_string())
    } else if FIELDS.contains(&name) {
        Err(ToolError::InvalidParams(format!(
            "{name} is not a field these rows carry; {what} must be one of {}",
            fields.join(", ")
        )))
    } else {
        Err(ToolError::InvalidParams(format!(
            "{what} must be one of {}",
            fields.join(", ")
        )))
    }
}

/// Parse `[{field, op, value}, ...]` over `fields`. `value` is a number, a
/// field name for a comparison, or `[low, high]` for `inside` / `outside`.
fn parse_clauses(v: &Value, fields: &[&str]) -> Result<Vec<Clause>, ToolError> {
    let Some(items) = v.as_array() else {
        return Err(ToolError::InvalidParams(
            "a predicate is an array of {field, op, value} clauses".into(),
        ));
    };
    if items.len() > MAX_CLAUSES {
        return Err(ToolError::InvalidParams(format!(
            "a selection takes at most {MAX_CLAUSES} clauses, since every one runs on every \
             print of the market as it arrives; {} were given",
            items.len()
        )));
    }
    items
        .iter()
        .map(|item| {
            let field = field_name(item.get("field"), "field", fields)?;
            let op = item
                .get("op")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or_default();
            let value = item.get("value");
            if op == "inside" || op == "outside" {
                // Every entry must be a number. Dropping the ones that are
                // not would turn [90, "x", 110] into a valid [90, 110] and
                // install a predicate the caller never wrote.
                let bounds: Vec<f64> = value
                    .and_then(|v: &Value| v.as_array())
                    .map(|a| a.iter().map(|v| v.as_f64()).collect::<Option<Vec<_>>>())
                    .unwrap_or_default()
                    .unwrap_or_default();
                let [lo, hi] = bounds[..] else {
                    return Err(ToolError::InvalidParams(format!(
                        "{op} takes value [low, high]"
                    )));
                };
                return Ok(Clause::Range {
                    field,
                    lo,
                    hi,
                    inside: op == "inside",
                });
            }
            let Some(op) = Op::ALL.iter().find(|o| o.as_str() == op) else {
                return Err(ToolError::InvalidParams(
                    "op must be one of >, >=, <, <=, ==, !=, inside, outside".into(),
                ));
            };
            let rhs = match value.and_then(|v: &Value| v.as_f64()) {
                Some(n) => Rhs::Number(n),
                // Naming the fields alone would read as though a number were
                // not allowed, and a number is the common case.
                None => Rhs::Field(field_name(value, "value", fields).map_err(|_| {
                    ToolError::InvalidParams(format!(
                        "value must be a number, or one of {} to compare two fields",
                        fields.join(", ")
                    ))
                })?),
            };
            Ok(Clause::Compare {
                field,
                op: *op,
                rhs,
            })
        })
        .collect()
}

fn clause_json(c: &Clause) -> Value {
    match c {
        Clause::Compare { field, op, rhs } => json!({
            "field": field,
            "op": op.as_str(),
            "value": match rhs {
                Rhs::Number(n) => json!(*n),
                Rhs::Field(f) => json!(f),
            }
        }),
        Clause::Range {
            field,
            lo,
            hi,
            inside,
        } => json!({
            "field": field,
            "op": if *inside { "inside" } else { "outside" },
            "value": [lo, hi]
        }),
    }
}

/// A trade with the market around it.
///
/// The feed sends the quote before a print and the two after it as separate
/// messages. Nothing on the wire ties them together, so this does.
#[derive(Clone, Debug)]
pub struct Print {
    pub trade: StreamData,
    pub quote_before: Option<StreamData>,
    pub quotes_after: Vec<StreamData>,
}

/// A print's fields: the trade's, then the quote that stood before it.
fn print_field(trade: &StreamData, quote: Option<&StreamData>, name: &str) -> Option<f64> {
    field_of(trade, name).or_else(|| quote.and_then(|q| field_of(q, name)))
}

#[derive(Debug)]
struct Buffer {
    ring: VecDeque<StreamData>,
    received: u64,
    dropped: u64,
    opened_ms: u64,
    /// When this buffer was last READ: the floor of a default window, which
    /// is everything since the caller last looked at it.
    read_ms: u64,
    /// When this buffer was last USED for anything, which is what keeps it
    /// alive. `live_prints` needs the trade and quote buffers it reads from
    /// to survive, but it is not a read OF them: moving the window floor
    /// would shorten a later read's window without moving the cursor that
    /// decides which rows it returns, and the read would then report a
    /// window shorter than the rows it came back with.
    touched_ms: u64,
    /// Rows received as of the last read. The cursor is a count advanced
    /// under the lock that appends the rows, not a clock: a row stamped
    /// the same millisecond as a read, or decoded before it and dispatched
    /// after, is new exactly once.
    read_seq: u64,
    /// The SDK's count of events it discarded, as of the last read. Rows
    /// this buffer never saw are not in `received`, and the only record of
    /// them is that counter.
    feed_drops_at_read: Option<u64>,
    /// The feed was interrupted while this buffer was open, so rows it never
    /// received are missing from the ring with nothing else to record them:
    /// the SDK's discard count covers what it threw away, not what never
    /// arrived. Set on every interruption, cleared by the read that
    /// discloses it.
    /// Interruptions this buffer has seen, and how many the last answer
    /// disclosed. Counted rather than flagged: the feed can break between a
    /// read and the commit that settles it, and clearing a flag would
    /// discharge an interruption nobody was ever shown.
    gaps: u64,
    gaps_at_read: u64,
    /// The latest moment up to which this buffer is known to have lost rows.
    ///
    /// Dated to the read that learns of the loss, not to when the loss
    /// began: a feed that broke at t=10 and resumed at t=20 was not
    /// delivering for all of it, and nothing between the break and the read
    /// that notices is proven. A window reaching back over this moment is
    /// not whole however often it is asked, which is what makes a named
    /// window answer the same way twice. Zero means nothing has been lost.
    incomplete_at_ms: u64,
    /// The feed was interrupted and nothing has arrived since, so there is
    /// no proof it is delivering again. Until a row lands, no window ending
    /// now can be shown whole; the row that lands is the proof, and dates
    /// the end of the loss.
    awaiting_resume: bool,
}

impl Buffer {
    fn open(now: u64) -> Self {
        Self {
            ring: VecDeque::new(),
            received: 0,
            dropped: 0,
            opened_ms: now,
            read_ms: now,
            touched_ms: now,
            read_seq: 0,
            feed_drops_at_read: None,
            gaps: 0,
            gaps_at_read: 0,
            incomplete_at_ms: 0,
            awaiting_resume: false,
        }
    }
}

/// Everything held for one contract: a buffer per subscription kind, and the
/// print correlation that spans two of them. Living together, the correlation
/// cannot outlive the last buffer or be dropped while one remains.
#[derive(Debug, Default)]
struct ContractState {
    /// At most one buffer per kind. Four kinds exist, so a scan beats a hash
    /// and the kind is kept alongside for reopening on a fresh session.
    buffers: Vec<(SubscriptionKind, Buffer)>,
    last_quote: Option<StreamData>,
    prints: VecDeque<Print>,
    prints_dropped: u64,
    /// Prints received as of the last `live_prints` that was answered, the
    /// same cursor a buffer keeps. Advanced only once the call succeeds, so
    /// a call that failed leaves its prints new for the next one.
    prints_read: u64,
    /// See [`Buffer::feed_drops_at_read`].
    feed_drops_at_prints_read: Option<u64>,
    /// See [`Buffer::gaps`]; the print list loses rows the same way.
    /// Counted rather than flagged: a read discloses the interruptions it
    /// saw, and the commit that settles it must not clear one that arrived
    /// while the answer was being assembled.
    gaps: u64,
    /// Interruptions disclosed by the last answer the caller received.
    gaps_at_prints_read: u64,
    /// Prints received as of the last interruption, so a list still holding
    /// prints from before one is still incomplete however often it is read.
    /// The hole leaves when the prints on either side of it do.
    prints_holed_before: Option<u64>,
    /// Position, counted from the first print ever, before which no print
    /// takes another quote. Quotes seen while the quote buffer was away, or
    /// after it came back, belong to an interval the correlation did not
    /// observe; sealing what was open when the buffer went keeps a print
    /// from before the gap from claiming them.
    unsealed_from: u64,
    /// The vendor's own bar for this contract, as last sent. The feed sends
    /// one ahead of each trade; it is stored and served as is, never built,
    /// extended or reconciled here.
    ohlcvc: Option<StreamData>,
}

impl ContractState {
    /// The buffer for `kind`, opened now if absent. `true` when it was.
    fn open(&mut self, kind: SubscriptionKind, now: u64) -> (&mut Buffer, bool) {
        let pos = self.buffers.iter().position(|(k, _)| *k == kind);
        let first = pos.is_none();
        if first {
            self.buffers.push((kind, Buffer::open(now)));
        }
        let i = pos.unwrap_or(self.buffers.len() - 1);
        (&mut self.buffers[i].1, first)
    }

    /// Drop the buffers `gone` names, and close the print correlation while
    /// the quote buffer is not among those left. The sweep runs this on every
    /// call, so the call that reopens the quote buffer has sealed everything
    /// from before the reopening first.
    fn close(&mut self, mut gone: impl FnMut(SubscriptionKind, &Buffer) -> bool) {
        self.buffers.retain(|(k, b)| !gone(*k, b));
        if !self
            .buffers
            .iter()
            .any(|(k, _)| *k == SubscriptionKind::Quote)
        {
            self.seal();
        }
    }

    /// Every print held so far is complete as it stands, and no quote is
    /// waiting to go before the next trade.
    fn seal(&mut self) {
        self.unsealed_from = self.prints_dropped + self.prints.len() as u64;
        self.last_quote = None;
    }
}

/// Every trade on one security type, from the vendor's full-stream
/// subscription, each with the NBBO the vendor sends ahead of it.
///
/// Nothing is buffered. A whole market prints at a rate no buffer here
/// could hold for a stated length of time, so a read does not walk a
/// window; the caller's selection is applied to each print as it arrives
/// and what it keeps is what a read returns.
#[derive(Debug)]
struct Market {
    /// The standing selection; none while the buffer only holds a
    /// subscription the feed would not release.
    selection: Option<Selection>,
    received: u64,
    opened_ms: u64,
    read_ms: u64,
    /// Prints received as of the last read; see [`Buffer::read_seq`].
    read_seq: u64,
    /// See [`Buffer::touched_ms`]: what keeps this buffer alive, kept apart
    /// from the floor a default window reads from.
    touched_ms: u64,
    /// See [`Buffer::feed_drops_at_read`].
    feed_drops_at_read: Option<u64>,
    /// See [`Buffer::gaps`]. A selection has no ring to fall short,
    /// so without this its only loss signal is the SDK's discard count, and
    /// rows that never arrived are not in it.
    gaps: u64,
    gaps_at_read: u64,
    newest_ms: Option<u64>,
    /// The quote most recently sent on the stream. The vendor sends a
    /// contract's last NBBO and bar just before its trade, so the trade
    /// claims this when it is for the same contract and leaves it when
    /// another contract's messages came between.
    last_quote: Option<StreamData>,
}

impl Market {
    fn open(now: u64) -> Self {
        Self {
            selection: None,
            received: 0,
            opened_ms: now,
            read_ms: now,
            read_seq: 0,
            touched_ms: now,
            feed_drops_at_read: None,
            gaps: 0,
            gaps_at_read: 0,
            newest_ms: None,
            last_quote: None,
        }
    }

    fn ingest(&mut self, data: &StreamData) {
        // The dispatcher can deliver a row captured before this buffer was
        // opened: a per-contract subscription swept moments earlier still
        // has rows in flight, and they route here by security type. They
        // belong to a subscription this selection never had.
        // A zero stamp is the SDK's fallback when the clock misbehaves,
        // and means unknown rather than old: dropping live rows over a
        // clock hiccup would be far worse than counting a late one.
        if seen_ms(data).is_some_and(|seen| seen < self.opened_ms) {
            return;
        }
        match data {
            StreamData::Quote { .. } => self.last_quote = Some(data.clone()),
            StreamData::Trade { contract, .. } => {
                self.received += 1;
                self.newest_ms = seen_ms(data);
                let quote_before = self
                    .last_quote
                    .take()
                    .filter(|q| contract_of(q) == Some(contract));
                if let Some(selection) = &mut self.selection {
                    selection.offer(data, quote_before);
                }
            }
            _ => {}
        }
    }
}

/// A standing selection over a market: the filter and rank the caller
/// installed, applied to every print as it arrives, and what it has kept
/// since the last read. A read takes the kept rows and the counts and
/// starts them again, so it answers "since I last looked" over every
/// print the feed delivered, whatever the rate.
#[derive(Debug)]
struct Selection {
    query: MarketQuery,
    /// Prints seen, and prints that passed the filter, since the last
    /// read: the population the kept rows were chosen from.
    examined: u64,
    matched: u64,
    /// Matches without the rank field, which cannot be placed.
    unranked: u64,
    /// The top `limit` by the rank, best first; without a rank, the newest
    /// `limit`, oldest first.
    kept: Vec<Print>,
}

impl Selection {
    /// Take on what another selection caught while this one was set aside,
    /// so a read that failed loses nothing and double-counts nothing. The
    /// kept rows are whatever the two hold between them, cut to the limit
    /// the restored selection was installed with.
    fn absorb(&mut self, other: Self) {
        // `offer` counts what it is shown, and these rows were counted when
        // they first arrived. Re-offering them is how they find their place
        // in the ranking; the counters are restored afterwards so nothing
        // is counted twice.
        let (examined, matched, unranked) = (
            self.examined + other.examined,
            self.matched + other.matched,
            self.unranked + other.unranked,
        );
        for print in other.kept {
            self.offer(&print.trade, print.quote_before);
        }
        self.examined = examined;
        self.matched = matched;
        self.unranked = unranked;
    }

    fn new(query: MarketQuery) -> Self {
        Self {
            query,
            examined: 0,
            matched: 0,
            unranked: 0,
            kept: Vec::new(),
        }
    }

    /// Runs on the dispatcher thread for every print on the market. The
    /// print is built only when it is kept; `limit` is at least one, so
    /// the newest match always has a place.
    fn offer(&mut self, trade: &StreamData, quote_before: Option<StreamData>) {
        self.examined += 1;
        if !self.query.selects(trade, quote_before.as_ref()) {
            return;
        }
        self.matched += 1;
        let limit = self.query.limit;
        let pos =
            match self.query.rank_by.as_deref() {
                None => {
                    if self.kept.len() == limit {
                        self.kept.remove(0);
                    }
                    self.kept.len()
                }
                Some(field) => {
                    // partition_point needs the predicate to hold on a
                    // prefix and fail after it. Every comparison with a
                    // non-finite key is false, which puts it at the front
                    // and then lets the next row displace the real best, so
                    // a key that cannot be ordered is a key that cannot be
                    // ranked.
                    let key = match print_field(trade, quote_before.as_ref(), field) {
                        Some(k) if k.is_finite() => k,
                        _ => {
                            self.unranked += 1;
                            return;
                        }
                    };
                    let ascending = self.query.ascending;
                    // Kept rows are best first, and a row already kept holds
                    // its place on a tie.
                    let pos =
                        self.kept.partition_point(|k| {
                            print_field(&k.trade, k.quote_before.as_ref(), field)
                                .is_some_and(|kk| if ascending { kk <= key } else { kk >= key })
                        });
                    // Behind every kept row on a full selection: the
                    // truncate below would drop it, so it is not built at
                    // all. On a busy market that is most prints.
                    if pos == limit {
                        return;
                    }
                    pos
                }
            };
        self.kept.insert(
            pos,
            Print {
                trade: trade.clone(),
                quote_before,
                quotes_after: Vec::new(),
            },
        );
        self.kept.truncate(limit);
    }
}

#[derive(Debug, Default)]
struct Held {
    contracts: HashMap<Contract, ContractState>,
    /// At most one market buffer per security type, keyed the way the SDK
    /// keys its full-stream snapshot.
    markets: Vec<(SecType, Market)>,
    /// The feed's most recent refusal of a subscribe, with when it was
    /// seen. The wire answers by request id and nothing outside the SDK
    /// maps that back to a contract, so this is the nearest thing to a
    /// reason a dropped buffer can be given.
    last_rejection: Option<(StreamResponseType, u64)>,
}

impl Held {
    fn market(&mut self, sec: SecType) -> Option<&mut Market> {
        self.markets
            .iter_mut()
            .find(|(s, _)| *s == sec)
            .map(|(_, m)| m)
    }

    /// How many contracts of `sec` hold a trade or quote buffer.
    fn per_contract_feed_on(&self, sec: SecType) -> usize {
        self.contracts
            .iter()
            .filter(|(c, s)| c.sec_type == sec && s.buffers.iter().any(|(k, _)| overlaps(*k)))
            .count()
    }
}

/// Kinds the full trade stream also carries. The SDK documents that a
/// contract matching both a full-stream and a per-contract subscription is
/// delivered on each independently, with nothing de-duplicating across the
/// two, and a doubled row is a message the vendor did not send.
fn overlaps(kind: SubscriptionKind) -> bool {
    matches!(kind, SubscriptionKind::Trade | SubscriptionKind::Quote)
}

/// What a read observed and what settling it costs, handed back so the
/// call can settle only once its answer is going to reach the caller.
#[derive(Clone)]
struct Settle {
    received: u64,
    feed_drops: u64,
    /// Discards this answer reported, which date the loss on settling.
    feed_dropped: u64,
    /// Interruptions counted as this answer saw them, so one arriving
    /// afterwards is still owed to the next read.
    gaps: u64,
    gap: bool,
}

struct Reading {
    first: bool,
    settle: Settle,
    floor: u64,
    dropped: u64,
    /// Where coverage starts: the oldest row held once the ring has
    /// overflowed, else the moment the buffer opened. A quiet buffer has seen
    /// everything since it opened, rows or not.
    covered_since_ms: u64,
    newest_ms: Option<u64>,
    /// The window reaches back before coverage starts, so the result is not
    /// the whole window asked for — because the ring overflowed, or because
    /// the buffer is younger than the window.
    clipped: bool,
    new_since_last_read: u64,
    /// Rows the window held, which is not how many came back: the tail is
    /// capped. A caller reading ten rows needs to know whether that was all
    /// of them.
    count: u64,
    /// The oldest row in the window. It dates the window, and it is the one
    /// row the tail may not reach.
    oldest: Option<StreamData>,
    tail: Vec<StreamData>,
    ohlcvc: Option<StreamData>,
    /// Events the SDK discarded since the last read because this server
    /// fell behind. They belong to no buffer in particular, so every buffer
    /// reports them and counts its window clipped while they are not zero.
    feed_dropped_since_last_read: u64,
}

/// What a buffer knows about its own completeness at the moment of a read.
struct Coverage {
    /// Rows the ring no longer holds but the cursor still counts.
    new: u64,
    held: usize,
    dropped: u64,
    covered_since_ms: u64,
    /// Events the SDK discarded since this buffer last settled.
    feed_dropped: u64,
    /// The feed was interrupted since this buffer last settled.
    gap: bool,
    /// When this buffer last lost rows, or zero if it never has.
    incomplete_at_ms: u64,
    /// This read's clock, which dates a loss with no end yet proven.
    now: u64,
    /// See [`Buffer::awaiting_resume`].
    awaiting_resume: bool,
}

/// Whether a window is missing rows. Anything the feed discarded before
/// this server saw it may have belonged here. Otherwise, without a window,
/// rows arrived since the last read that the ring no longer holds. With
/// one, coverage starting after the floor — or on it while rows were
/// discarded: rows discarded ahead of the oldest held may share its stamp,
/// so a floor the oldest held row sits on is not proven covered.
fn clipped(window: Option<u64>, floor: u64, c: &Coverage) -> bool {
    match window {
        // Since your last read: anything lost in that interval counts, and
        // a loss an earlier read disclosed still sits inside this window
        // when it happened after that read.
        None => {
            c.gap
                || c.awaiting_resume
                || c.feed_dropped > 0
                || c.new > c.held as u64
                // Strictly after the last read: a loss dated at that read
                // is behind this window, and was disclosed by it.
                || c.incomplete_at_ms > floor
        }
        // A named window asks about an interval, so only a loss inside it
        // counts. A loss learned of now is dated now, since nothing between
        // the loss and this read is proven.
        Some(_) => {
            // Nothing has arrived since the interruption, so no window
            // ending now can be shown whole.
            let lost_at = if c.awaiting_resume || c.feed_dropped > 0 {
                c.now.max(c.incomplete_at_ms)
            } else {
                c.incomplete_at_ms
            };
            c.covered_since_ms > floor
                || (c.dropped > 0 && c.covered_since_ms == floor)
                || (lost_at > 0 && lost_at >= floor)
        }
    }
}

struct Prints {
    opened: Vec<SubscriptionKind>,
    rows: Vec<Print>,
    held: usize,
    dropped: u64,
    /// Prints received as of this read: the cursor to commit once the
    /// call has succeeded.
    received: u64,
    feed_dropped_since_last_read: u64,
    /// The feed was interrupted while these prints were held. Reported, and
    /// cleared only once the call has succeeded, so a failed read does not
    /// consume the disclosure.
    gap: bool,
    /// Prints from before a loss are still held. Unlike `gap` this is not
    /// spent by the read that reports it: the history stays holed until the
    /// prints around the hole are gone.
    holed: bool,
    /// Interruptions counted as this read saw them: the cursor to commit
    /// alongside `received`, so one that arrives afterwards still stands.
    gaps_seen: u64,
    /// The SDK's discard count as this read saw it: the cursor to commit
    /// alongside `received`.
    feed_drops_seen: u64,
    covered_since_ms: u64,
    newest_ms: Option<u64>,
    new_since_last_read: u64,
}

/// What a whole-market selection keeps: the contract attributes, the
/// predicate over vendor fields, and how to rank and cut what passes.
/// `limit` is at least one.
#[derive(Clone, Debug, PartialEq)]
struct MarketQuery {
    root: Option<String>,
    expiration: Option<i32>,
    is_call: Option<bool>,
    strike_min: Option<f64>,
    strike_max: Option<f64>,
    clauses: Vec<Clause>,
    rank_by: Option<String>,
    ascending: bool,
    limit: usize,
}

impl MarketQuery {
    fn selects(&self, trade: &StreamData, quote: Option<&StreamData>) -> bool {
        let Some(c) = contract_of(trade) else {
            return false;
        };
        if self
            .root
            .as_deref()
            .is_some_and(|r| !c.symbol.eq_ignore_ascii_case(r))
        {
            return false;
        }
        if self.expiration.is_some() && c.expiration != self.expiration {
            return false;
        }
        if self.is_call.is_some() && c.is_call != self.is_call {
            return false;
        }
        let strike = c.strike_dollars();
        if self
            .strike_min
            .is_some_and(|lo| strike.is_none_or(|s| s < lo))
            || self
                .strike_max
                .is_some_and(|hi| strike.is_none_or(|s| s > hi))
        {
            return false;
        }
        holds_all(&self.clauses, &|f| print_field(trade, quote, f))
    }
}

/// What settling a market read costs, so a call that fails on the feed
/// leaves the selection and its counts for the next one.
struct SettleMarket {
    received: u64,
    feed_drops: u64,
    /// Interruptions as this answer saw them, so one arriving before the
    /// commit is still owed to the next read.
    gaps: u64,
    /// The selection this read installs, restored if it never lands.
    outgoing: Option<Selection>,
}

struct MarketReading {
    first: bool,
    settle: SettleMarket,
    previous_ms: u64,
    received: u64,
    newest_ms: Option<u64>,
    new_since_last_read: u64,
    feed_dropped_since_last_read: u64,
    /// The feed was interrupted while the selection stood, so prints it
    /// never saw are missing and the discard count cannot show it: those
    /// rows never reached the SDK to be discarded.
    gap: bool,
    /// The populations each count covers: prints the selection saw since
    /// the last read, prints that passed it, matches it could not rank,
    /// and the rows kept.
    examined: u64,
    matched: u64,
    /// Present when the selection that kept the rows ranked them.
    unranked: Option<u64>,
    rows: Vec<Print>,
    /// The selection the rows were kept under, when this read replaced it.
    selected_by: Option<MarketQuery>,
}

struct Holding {
    sub: Subscription,
    label: String,
    received: u64,
    held: usize,
    dropped: u64,
    opened_ms: u64,
    /// When this buffer was last read, or `None` for one the sweep has marked
    /// idle so the next call hands its subscription back. Nought is that
    /// mark, not a moment, and reading it as one reports decades of idleness
    /// for a buffer put back a moment ago.
    read_ms: Option<u64>,
    newest_ms: Option<u64>,
}

/// The two shapes a subscription takes. `Subscription` is non-exhaustive
/// upstream, so every match on it needs a fallback; this is the one place
/// that fallback lives.
enum Shape<'a> {
    Contract(&'a Contract, SubscriptionKind),
    Full(SecType, FullSubscriptionKind),
}

fn shape(sub: &Subscription) -> Option<Shape<'_>> {
    match sub {
        Subscription::Contract { contract, kind } => Some(Shape::Contract(contract, *kind)),
        Subscription::Full { sec_type, kind } => Some(Shape::Full(*sec_type, *kind)),
        _ => None,
    }
}

fn subscription(kind: SubscriptionKind, contract: &Contract) -> Subscription {
    Subscription::Contract {
        contract: contract.clone(),
        kind,
    }
}

/// The full-stream subscription the SDK's snapshot tuple stands for, or
/// `None` for a kind the vendor has no full-stream form of.
fn full_subscription(kind: SubscriptionKind, sec: SecType) -> Option<Subscription> {
    Some(match kind {
        SubscriptionKind::Trade => sec.full_trades(),
        SubscriptionKind::OpenInterest => sec.full_open_interest(),
        _ => return None,
    })
}

/// A subscription in words, for an error or a listing.
fn label(sub: &Subscription) -> String {
    match shape(sub) {
        Some(Shape::Contract(c, k)) => format!("{} {c}", k.kind_str()),
        Some(Shape::Full(s, k)) => format!("{} {}", k.kind_str(), s.as_str()),
        None => format!("{sub:?}"),
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    inner: Mutex<Held>,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, Held> {
        // Recover rather than propagate: one panicking read must not disable
        // every stream tool for the life of the process.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Close buffers nobody has read inside the TTL and hand back their
    /// subscriptions. Every tool call runs this first and closes what it
    /// returns before doing anything else, so a buffer that outlived its idle
    /// window is buried before the read that would otherwise have revived
    /// it, a call the registry refuses cannot strand what the sweep freed,
    /// and a buffer put back because the feed would not let it go is in
    /// place before any conflict is judged.
    fn expire(&self, now: u64) -> Subs {
        let mut held = self.lock();
        let ttl = TTL.as_millis() as u64;
        let mut freed = Vec::new();
        held.contracts.retain(|contract, state| {
            state.close(|kind, buffer| {
                let dead = now.saturating_sub(buffer.touched_ms) > ttl;
                if dead {
                    freed.push(subscription(kind, contract));
                }
                dead
            });
            !state.buffers.is_empty()
        });
        held.markets.retain(|(sec, market)| {
            let live = now.saturating_sub(market.touched_ms) <= ttl;
            if !live {
                freed.push(sec.full_trades());
            }
            live
        });
        freed
    }

    /// Refuse a per-contract buffer the market buffer on its security type
    /// would double, and say what to close.
    fn without_market(
        held: &Held,
        contract: &Contract,
        kind: SubscriptionKind,
    ) -> Result<(), ToolError> {
        if overlaps(kind) && held.markets.iter().any(|(s, _)| *s == contract.sec_type) {
            return Err(ToolError::InvalidParams(format!(
                "the whole-market buffer on {} (live_market) already carries every {} on it, and \
                 the feed would deliver {contract} twice if both were held; live_stop with \
                 sec_type alone closes the market buffer",
                contract.sec_type.as_str(),
                kind.kind_str()
            )));
        }
        Ok(())
    }

    /// Read a buffer, opening it if this is the first look. `window` is a
    /// lookback in milliseconds; without one the window is everything since
    /// the previous read.
    /// `feed_drops` is the SDK's count of discarded events now.
    ///
    /// Every reader copies what it needs under the lock and releases it
    /// before serialising. The streaming dispatcher calls [`Self::ingest`] on
    /// the same lock, so a critical section held across JSON construction
    /// would stall the feed — the one thing this layer must never do. The
    /// the count is one pass over references; only the tail is cloned.
    fn read(
        &self,
        contract: &Contract,
        kind: SubscriptionKind,
        window: Option<u64>,
        tail: usize,
        feed_drops: u64,
        now: u64,
    ) -> Result<Reading, ToolError> {
        let mut held = self.lock();
        Self::without_market(&held, contract, kind)?;
        let state = held.contracts.entry(contract.clone()).or_default();
        let mut state_seal = false;
        let mut state_gap = false;
        // Reopening a leg is an interruption, whichever tool does it: the
        // feed stopped carrying this contract in between, and prints or
        // quotes from that interval are simply absent.
        let held_prints = !state.prints.is_empty();
        let first = !state.buffers.iter().any(|(k, _)| *k == kind);
        if first && kind == SubscriptionKind::Quote {
            state_seal = true;
        }
        if first && kind == SubscriptionKind::Trade && held_prints {
            state_gap = true;
        }
        if state_seal {
            state.seal();
        }
        if state_gap {
            state.gaps += 1;
            state.prints_holed_before = Some(state.prints_dropped + state.prints.len() as u64);
        }
        let inherited = state
            .feed_drops_at_prints_read
            .into_iter()
            .chain(
                state
                    .buffers
                    .iter()
                    .filter_map(|(_, b)| b.feed_drops_at_read),
            )
            .min();
        let holed_elsewhere = state.prints_holed_before.is_some();
        let (buffer, _) = state.open(kind, now);
        if buffer.feed_drops_at_read.is_none() {
            // Another view of this contract has been watching, so what the
            // feed discarded since is this buffer's to report too, and so is
            // the loss that number already stands for. With no other view,
            // this buffer starts counting from what this call was handed.
            buffer.feed_drops_at_read = Some(inherited.unwrap_or(feed_drops));
            if inherited.is_some() && holed_elsewhere {
                buffer.incomplete_at_ms = buffer.incomplete_at_ms.max(now);
            }
        }
        let previous = buffer.read_ms;
        // Using the buffer is what keeps it alive, and that is true even of a
        // call that goes on to fail. Everything else this read observes is
        // settled by `commit_read`, once the answer is going to reach the
        // caller.
        buffer.touched_ms = now;
        let new = buffer.received - buffer.read_seq;
        let received = buffer.received;
        let feed_dropped = buffer
            .feed_drops_at_read
            .map_or(0, |at| feed_drops.saturating_sub(at));
        let floor = window.map_or(previous, |w| now.saturating_sub(w));

        let mut count = 0u64;
        let mut rows = Vec::new();
        let mut oldest = None;
        // Newest first. Without a window the rows are the `new` newest;
        // with one they are those stamped at or after the floor. Either set
        // is a run from the back, so the walk stops at the first row outside
        // it and never touches the rest.
        for (i, d) in buffer.ring.iter().rev().enumerate() {
            let inside = match window {
                None => (i as u64) < new,
                // A zero stamp is the SDK's fallback for a clock it could
                // not read, and a clock can also step backwards, so a row
                // outside the window does not mean the rest are.
                Some(_) => seen_ms(d).is_none_or(|s| s >= floor && s <= now),
            };
            if !inside {
                // The default window counts arrivals, which are in order, so
                // the first row outside it ends the walk. A named window
                // reads stamps, which are not guaranteed to be, so it keeps
                // looking rather than hiding what sits behind one.
                if window.is_none() {
                    break;
                }
                continue;
            }
            count += 1;
            oldest = Some(d);
            if rows.len() < tail {
                rows.push(d.clone());
            }
        }
        let oldest = oldest.cloned();
        // The age this answer reports is of the newest row it hands back, not
        // of the newest the buffer holds nor the newest the window covers. An
        // answer returning nothing has no age: taking a held row's would date
        // it to whenever that row landed, and a window ending before it
        // renders nought, which reads as something having just arrived.
        let newest_ms = rows.first().and_then(seen_ms);
        rows.reverse();

        // Where coverage starts. A quiet buffer has seen everything since it
        // opened. Once rows have been evicted it starts at the oldest one
        // still held, and if that row carries no stamp there is nothing to
        // place it by, so nothing before this read is proven covered.
        let covered_since_ms = if buffer.dropped > 0 {
            buffer.ring.front().and_then(seen_ms).unwrap_or(now)
        } else {
            buffer.opened_ms
        };
        let dropped = buffer.dropped;
        // Everything the buffer has to say, read out before the borrow ends.
        let gap = buffer.gaps > buffer.gaps_at_read;
        let gaps_seen = buffer.gaps;
        let awaiting_resume = buffer.awaiting_resume;
        let incomplete_at = buffer.incomplete_at_ms;
        let held_rows = buffer.ring.len();
        let drops_at_read = buffer.feed_drops_at_read.unwrap_or(0);
        let reading = Reading {
            first,
            floor,
            dropped,
            covered_since_ms,
            newest_ms,
            clipped: clipped(
                window,
                floor,
                &Coverage {
                    new,
                    held: held_rows,
                    dropped,
                    covered_since_ms,
                    feed_dropped,
                    gap,
                    incomplete_at_ms: incomplete_at,
                    now,
                    awaiting_resume,
                },
            ),
            new_since_last_read: new,
            feed_dropped_since_last_read: feed_dropped,
            count,
            oldest,
            tail: rows,
            ohlcvc: state.ohlcvc.clone(),
            settle: Settle {
                received,
                feed_drops: feed_drops.max(drops_at_read),
                feed_dropped,
                gaps: gaps_seen,
                gap,
            },
        };
        Ok(reading)
    }

    /// The newest `count` prints, oldest first. Opens the trade and quote
    /// buffers a print is built from when they are not already held. The
    /// cursor is not advanced here: [`Self::commit_prints_read`] does that
    /// once the call has succeeded.
    fn prints(
        &self,
        contract: &Contract,
        count: usize,
        feed_drops: u64,
        now: u64,
    ) -> Result<Prints, ToolError> {
        let mut held = self.lock();
        Self::without_market(&held, contract, SubscriptionKind::Trade)?;
        let state = held.contracts.entry(contract.clone()).or_default();
        let mut opened = Vec::new();
        // Coverage starts where the trade leg opened: a print is a trade.
        let mut opened_ms = now;
        let held_prints = !state.prints.is_empty();
        let mut gaps_from_reopen = 0u64;
        let mut needs_seal = false;
        for kind in [SubscriptionKind::Trade, SubscriptionKind::Quote] {
            let (buffer, first) = state.open(kind, now);
            if first {
                opened.push(kind);
            }
            if kind == SubscriptionKind::Trade {
                opened_ms = buffer.opened_ms;
            }
            buffer.touched_ms = now;
            // A quote leg coming back has been away, and a print that
            // arrived while it was gone saw none of the quotes from that
            // interval. Sealing on the way out is not enough: a trade can
            // land between the sweep and this reopening.
            if first && kind == SubscriptionKind::Quote {
                needs_seal = true;
            }
            // Prints accrue only while the trade buffer exists, so a trade
            // buffer being created while prints from before it are still held
            // means it expired and the feed stopped carrying this contract
            // in between. Nothing else records that interval.
            if first && kind == SubscriptionKind::Trade && held_prints {
                gaps_from_reopen += 1;
            }
        }
        if needs_seal {
            state.seal();
        }
        if gaps_from_reopen > 0 {
            state.gaps += 1;
        }
        if gaps_from_reopen > 0 {
            // A leg reopened is an interruption, marked where it happened.
            state.prints_holed_before = Some(state.prints_dropped + state.prints.len() as u64);
        }
        // The hole is behind every print still held, so nothing it touches
        // is being served any more.
        if state
            .prints_holed_before
            .is_some_and(|at| at > 0 && state.prints_dropped >= at)
        {
            state.prints_holed_before = None;
        }
        let received = state.prints_dropped + state.prints.len() as u64;
        let new = received - state.prints_read;
        // Reported without moving the cursor: a call that fails still owes
        // these to the next one, the same as the gap mark and the prints
        // cursor beside it.
        // A contract already being read has a baseline; prints starting
        // afterwards inherit it rather than treating every discard since as
        // nobody's to report.
        if state.feed_drops_at_prints_read.is_none() {
            // Where another view of this contract already was, or where this
            // one starts. Either way it is set now rather than when an answer
            // first lands: a call that fails still opened the subscriptions,
            // and a discard after that is this contract's to report.
            state.feed_drops_at_prints_read = Some(
                state
                    .buffers
                    .iter()
                    .filter_map(|(_, b)| b.feed_drops_at_read)
                    .min()
                    .unwrap_or(feed_drops),
            );
            // The baseline carries what it has already counted. Taking the
            // number without the loss behind it would start these prints
            // from a clean history the buffer knows is holed.
            if state.buffers.iter().any(|(_, b)| b.incomplete_at_ms > 0)
                && state.prints_holed_before.is_none()
            {
                state.prints_holed_before = Some(state.prints_dropped + state.prints.len() as u64);
            }
        }
        let feed_dropped = state
            .feed_drops_at_prints_read
            .map_or(0, |at| feed_drops.saturating_sub(at));
        // A discard leaves the same hole an interruption does: the prints
        // on either side of it are served again and again.
        if feed_dropped > 0 {
            // The newest loss, not the first: clearing the marker when the
            // prints around an older hole are gone would declare a history
            // whole while it still spans a later one.
            state.prints_holed_before = Some(state.prints_dropped + state.prints.len() as u64);
        }
        let mut rows: Vec<Print> = state.prints.iter().rev().take(count).cloned().collect();
        rows.reverse();
        let prints = Prints {
            opened,
            held: state.prints.len(),
            dropped: state.prints_dropped,
            received,
            feed_dropped_since_last_read: feed_dropped,
            gap: state.gaps > state.gaps_at_prints_read,
            // A hole matters when the rows coming back sit either side of
            // it. Asking for only the newest prints, all of them after the
            // loss, is a complete answer to what was asked.
            holed: state.prints_holed_before.is_some_and(|at| {
                if at == 0 {
                    // The hole sits before every print held, so no returned
                    // row spans it. A caller asking for more prints than
                    // exist reaches back into it all the same.
                    return count > state.prints.len();
                }
                let oldest_returned =
                    state.prints_dropped + state.prints.len().saturating_sub(count) as u64;
                oldest_returned < at
            }),
            gaps_seen: state.gaps,
            feed_drops_seen: feed_drops,
            // Coverage starts at the oldest print held, or where the trade
            // buffer opened when none was dropped. A buffer reopened after the
            // prints it holds opened later than they arrived, and taking its
            // clock would claim coverage that starts after rows it is
            // already showing, so the oldest print wins whenever it is older.
            covered_since_ms: state
                .prints
                .front()
                .and_then(|p| seen_ms(&p.trade))
                .map(|oldest| {
                    if state.prints_dropped > 0 {
                        oldest
                    } else {
                        oldest.min(opened_ms)
                    }
                })
                .unwrap_or(opened_ms),
            // The newest print returned, which with a count of nought is
            // none: an age there would date a print the caller never saw.
            newest_ms: rows.last().and_then(|p| seen_ms(&p.trade)),
            new_since_last_read: new,
            rows,
        };
        Ok(prints)
    }

    /// Settle a buffer read: advance its cursors and discharge the
    /// interruption it disclosed. Runs only once the answer is going to
    /// reach the caller, so a call that fails on the feed leaves both for
    /// the next read.
    fn commit_read(&self, contract: &Contract, kind: SubscriptionKind, settle: Settle, now: u64) {
        let mut held = self.lock();
        let Some(state) = held.contracts.get_mut(contract) else {
            return;
        };
        let Some((_, buffer)) = state.buffers.iter_mut().find(|(k, _)| *k == kind) else {
            return;
        };
        buffer.read_ms = now;
        buffer.read_seq = settle.received;
        // A loss with no proof of resumption is dated here, because nothing
        // between it and this read is proven. One a row has already proven
        // the end of keeps that row's stamp: moving it forward would clip
        // windows that sit entirely after the feed came back.
        if (settle.gap && buffer.awaiting_resume) || settle.feed_dropped > 0 {
            buffer.incomplete_at_ms = buffer.incomplete_at_ms.max(now);
        }
        buffer.gaps_at_read = settle.gaps;
        buffer.feed_drops_at_read = Some(settle.feed_drops);
    }

    /// Settle a whole-market read: advance its cursors and discharge the
    /// interruption it disclosed. The replacement selection was installed
    /// as the answer was built, since the dispatcher must keep selecting
    /// against something; what this settles is everything else.
    fn commit_market(&self, sec: SecType, settle: &SettleMarket, now: u64) {
        let mut held = self.lock();
        let Some((_, market)) = held.markets.iter_mut().find(|(s, _)| *s == sec) else {
            return;
        };
        market.read_ms = now;
        market.touched_ms = now;
        market.read_seq = settle.received;
        market.feed_drops_at_read = Some(settle.feed_drops);
        market.gaps_at_read = settle.gaps;
    }

    /// Put back the selection a failed read took, so its rows and counts
    /// reach the next caller instead of being lost with the error.
    fn restore_market(&self, sec: SecType, outgoing: Option<Selection>) {
        let Some(outgoing) = outgoing else {
            return;
        };
        let mut held = self.lock();
        if let Some((_, market)) = held.markets.iter_mut().find(|(s, _)| *s == sec) {
            // Whatever the replacement caught while the call was failing
            // belongs to the same caller, so it is folded back in.
            let replacement = market.selection.replace(outgoing);
            if let (Some(kept), Some(back)) = (replacement, market.selection.as_mut()) {
                // Only when the replacement was asking the same question.
                // A different filter examined a different population and
                // kept rows this one would not have, and folding those in
                // would report them as its own.
                if kept.query == back.query {
                    back.absorb(kept);
                } else if kept.examined > 0 {
                    // A replacement asking something else examined prints
                    // this selection never saw. Folding its counts in would
                    // report another question's work as this one's, so they
                    // go; but a caller must not read the difference between
                    // received and examined as nothing having happened.
                    market.gaps += 1;
                }
            }
        }
    }

    /// Advance the prints cursor to what a successful `live_prints` served.
    ///
    /// Settle what the answer just delivered. The interruption count is
    /// the one that answer saw, not the one standing now: the feed can
    /// break between the read and this call, and an interruption the
    /// caller was never shown is still owed to the next read.
    fn commit_prints_read(&self, contract: &Contract, received: u64, feed_drops: u64, gaps: u64) {
        if let Some(state) = self.lock().contracts.get_mut(contract) {
            state.prints_read = received;
            state.gaps_at_prints_read = gaps;
            state.feed_drops_at_prints_read = Some(feed_drops);
        }
    }

    /// This server replaced the feed session. The SDK's discard count is
    /// per session and the new one starts at zero, so a cursor left at the
    /// old session's count would hide every discard until the new count
    /// passed it; everything the new session discards is new to every
    /// buffer, so the cursors start at zero too. Not for the SDK's own
    /// reconnects: the session and its count survive those.
    fn restarted(&self) {
        let mut held = self.lock();
        for state in held.contracts.values_mut() {
            state.feed_drops_at_prints_read = Some(0);
            for (_, buffer) in &mut state.buffers {
                buffer.feed_drops_at_read = Some(0);
            }
        }
        for (_, market) in &mut held.markets {
            market.feed_drops_at_read = Some(0);
        }
    }

    /// The feed was interrupted: whatever correlation was open spans an
    /// interval nobody observed, so every print is closed as it stands and
    /// no quote waits to go before the next trade, on every contract and
    /// every market.
    fn gap(&self, now: u64) {
        let mut held = self.lock();
        for state in held.contracts.values_mut() {
            state.seal();
            state.gaps += 1;
            state.prints_holed_before = Some(state.prints_dropped + state.prints.len() as u64);
            for (_, buffer) in &mut state.buffers {
                buffer.gaps += 1;
                buffer.incomplete_at_ms = now;
                buffer.awaiting_resume = true;
            }
        }
        for (_, market) in &mut held.markets {
            market.last_quote = None;
            market.gaps += 1;
        }
    }

    /// Read the whole-market buffer for `sec`, opening it on the first look,
    /// and install `q` as its selection from here on. What comes back was
    /// kept by the selection that stood until now.
    fn market(
        &self,
        sec: SecType,
        q: MarketQuery,
        feed_drops: u64,
        now: u64,
    ) -> Result<MarketReading, ToolError> {
        let mut held = self.lock();
        let doubled = held.per_contract_feed_on(sec);
        if doubled > 0 {
            return Err(ToolError::InvalidParams(format!(
                "{doubled} {} contract(s) hold a trade or quote buffer, and the feed would deliver \
                 them twice alongside a whole-market buffer; live_list shows them and live_stop \
                 closes them",
                sec.as_str()
            )));
        }
        let pos = held.markets.iter().position(|(s, _)| *s == sec);
        let first = pos.is_none();
        let i = pos.unwrap_or_else(|| {
            held.markets.push((sec, Market::open(now)));
            held.markets.len() - 1
        });
        let market = &mut held.markets[i].1;
        let previous = market.read_ms;
        market.touched_ms = now;
        // Observed here, settled by `commit_market` once the answer is
        // going to reach the caller. A call that fails on the feed after
        // this point leaves the selection and its counts for the next read.
        let new = market.received - market.read_seq;
        let received = market.received;
        let feed_dropped = market
            .feed_drops_at_read
            .map_or(0, |at| feed_drops.saturating_sub(at));

        // What stood until now comes out, reported under its own query
        // when this read changed it; the new selection starts empty.
        let outgoing = market.selection.replace(Selection::new(q.clone()));
        let (examined, matched, unranked, rows, selected_by, taken) = match outgoing {
            Some(s) => {
                // Disclosed by the selection that kept the rows: whether it
                // ranked decides whether it could have set anything aside.
                let unranked = s.query.rank_by.as_ref().map(|_| s.unranked);
                let query = s.query.clone();
                (
                    s.examined,
                    s.matched,
                    unranked,
                    s.kept.clone(),
                    (query != q).then_some(query),
                    Some(s),
                )
            }
            None => (0, 0, None, Vec::new(), None, None),
        };
        let reading = MarketReading {
            first,
            settle: SettleMarket {
                received,
                feed_drops: feed_drops.max(market.feed_drops_at_read.unwrap_or(0)),
                gaps: market.gaps,
                outgoing: taken,
            },
            previous_ms: previous,
            received: market.received,
            newest_ms: market.newest_ms,
            new_since_last_read: new,
            feed_dropped_since_last_read: feed_dropped,
            gap: market.gaps > market.gaps_at_read,
            examined,
            matched,
            unranked,
            rows,
            selected_by,
        };
        Ok(reading)
    }

    /// Every subscription a contract's buffers hold, for `live_stop` to close
    /// on the feed before the buffers go.
    fn held_for(&self, contract: &Contract) -> Subs {
        self.lock()
            .contracts
            .get(contract)
            .map_or_else(Vec::new, |s| {
                s.buffers
                    .iter()
                    .map(|(k, _)| subscription(*k, contract))
                    .collect()
            })
    }

    fn market_held(&self, sec: SecType) -> bool {
        self.lock().markets.iter().any(|(s, _)| *s == sec)
    }

    /// Drop a buffer the feed does not carry: one whose subscribe was refused
    /// after being accepted, or one whose unsubscribe landed. Leaving it
    /// would make the next read skip subscribing too.
    fn forget(&self, sub: &Subscription) {
        let mut held = self.lock();
        match shape(sub) {
            Some(Shape::Contract(contract, kind)) => {
                if let Some(state) = held.contracts.get_mut(contract) {
                    state.close(|k, _| k == kind);
                    if state.buffers.is_empty() {
                        held.contracts.remove(contract);
                    }
                }
            }
            Some(Shape::Full(sec, _)) => held.markets.retain(|(s, _)| *s != sec),
            None => {}
        }
    }

    /// Keep a reference to a subscription the feed still holds after its
    /// buffer was swept: an empty buffer already past its TTL, so the next
    /// sweep hands it back to be closed again. A buffer reopened meanwhile is
    /// left as it is.
    fn reinstate(&self, sub: &Subscription, now: u64) {
        let mut held = self.lock();
        match shape(sub) {
            Some(Shape::Contract(contract, kind)) => {
                let state = held.contracts.entry(contract.clone()).or_default();
                let had_prints = !state.prints.is_empty();
                let (buffer, first) = state.open(kind, now);
                if first {
                    // Idle as far as the sweep is concerned, so the next one
                    // hands the subscription back to be released again.
                    // `read_ms` is the floor of a default window and is left
                    // where it is: zeroing it would date the next read's
                    // window to the epoch.
                    buffer.touched_ms = 0;
                    // The buffer was gone while the feed kept delivering, and
                    // putting it back is not the same as never having lost
                    // it. A later read must not find an intact leg and
                    // conclude nothing was missed.
                    buffer.gaps += 1;
                    buffer.incomplete_at_ms = now;
                    buffer.awaiting_resume = true;
                    if kind == SubscriptionKind::Trade && had_prints {
                        state.gaps += 1;
                        state.prints_holed_before =
                            Some(state.prints_dropped + state.prints.len() as u64);
                    }
                    if kind == SubscriptionKind::Quote {
                        state.seal();
                    }
                }
            }
            Some(Shape::Full(sec, _)) if held.market(sec).is_none() => {
                let mut market = Market::open(now);
                // Idle to the sweep, without dating a read's since_seconds
                // to the epoch.
                market.touched_ms = 0;
                // The same absence the contract arm marks above. The buffer
                // was gone while the feed kept delivering every print on the
                // type, and it held no selection to offer them to, so the
                // next read must not find an intact market and conclude
                // nothing was missed.
                market.gaps += 1;
                held.markets.push((sec, market));
            }
            _ => {}
        }
    }

    fn note_rejection(&self, result: StreamResponseType, now: u64) {
        self.lock().last_rejection = Some((result, now));
    }

    fn last_rejection(&self) -> Option<(StreamResponseType, u64)> {
        self.lock().last_rejection
    }

    fn list(&self) -> Vec<Holding> {
        let held = self.lock();
        let mut rows: Vec<Holding> = held
            .contracts
            .iter()
            .flat_map(|(c, s)| {
                s.buffers.iter().map(move |(k, b)| Holding {
                    sub: subscription(*k, c),
                    label: c.to_string(),
                    received: b.received,
                    held: b.ring.len(),
                    dropped: b.dropped,
                    opened_ms: b.opened_ms,
                    read_ms: Some(b.touched_ms).filter(|t| *t > 0),
                    newest_ms: b.ring.back().and_then(seen_ms),
                })
            })
            .chain(held.markets.iter().map(|(sec, m)| Holding {
                sub: sec.full_trades(),
                label: sec.as_str().to_string(),
                received: m.received,
                held: m.selection.as_ref().map_or(0, |s| s.kept.len()),
                dropped: 0,
                opened_ms: m.opened_ms,
                read_ms: Some(m.touched_ms).filter(|t| *t > 0),
                newest_ms: m.newest_ms,
            }))
            .collect();
        rows.sort_by_key(|h| (h.label.clone(), label(&h.sub)));
        rows
    }

    /// Every subscription the buffers hold, once each, in the two shapes the
    /// SDK's restore takes: what a fresh session must reopen.
    fn subscriptions(&self) -> Restore {
        let held = self.lock();
        (
            held.contracts
                .iter()
                .flat_map(|(c, s)| s.buffers.iter().map(move |(k, _)| (*k, c.clone())))
                .collect(),
            held.markets
                .iter()
                .map(|(sec, _)| (SubscriptionKind::Trade, *sec))
                .collect(),
        )
    }

    /// Store a row. Unknown buffers are ignored rather than created: the feed
    /// can deliver a contract after its unsubscribe, and inventing a buffer for
    /// one would leak.
    pub fn ingest(&self, data: StreamData) {
        let Some(contract) = contract_of(&data) else {
            return;
        };
        let mut held = self.lock();
        if let Some(market) = held.market(contract.sec_type) {
            market.ingest(&data);
        }
        let Some(state) = held.contracts.get_mut(contract) else {
            return;
        };
        // The vendor's bar has no subscription of its own: it rides ahead of
        // the trade, and a held contract keeps the newest one.
        if let StreamData::Ohlcvc { .. } = data {
            // Through the same gate as every other row: a bar queued by a
            // subscription that has since closed is not this buffer's.
            //
            // Measured against the trade buffer where there is one, because a
            // bar rides ahead of a trade and that is the subscription that
            // produced it. Letting an older sibling vouch instead admits a
            // bar the previous trade subscription queued: a quote buffer open
            // since before the trade leg was dropped and reopened would date
            // the bar to itself and call it current.
            //
            // With no trade buffer there is nothing here it can belong to: the
            // subscription that produces bars is not held, so a bar in hand
            // was queued by one that has closed.
            let fresh = state
                .buffers
                .iter()
                .find(|(kind, _)| *kind == SubscriptionKind::Trade)
                .is_some_and(|(_, b)| seen_ms(&data).is_none_or(|seen| seen >= b.opened_ms));
            if fresh {
                state.ohlcvc = Some(data);
            }
            return;
        }
        let Some(msg) = msg_type_of(&data) else {
            return;
        };
        let Some((_, buffer)) = state
            .buffers
            .iter_mut()
            .find(|(k, _)| k.subscribe_code() == msg)
        else {
            return;
        };
        // A buffer reopened after an expiry or a stop can still be handed
        // rows the previous subscription queued. They belong to the feed
        // this buffer was not on. A zero stamp is the SDK's fallback for a
        // clock it could not read and means unknown, so those are kept.
        if seen_ms(&data).is_some_and(|seen| seen < buffer.opened_ms) {
            return;
        }
        buffer.received += 1;
        if buffer.awaiting_resume {
            // Delivery is proven again. A row whose clock could not be read
            // proves that much and dates nothing, so the loss keeps the
            // moment it already had rather than being moved to zero, which
            // would erase it.
            if let Some(seen) = seen_ms(&data) {
                buffer.incomplete_at_ms = seen;
                buffer.awaiting_resume = false;
            }
        }
        if buffer.ring.len() == RING {
            buffer.ring.pop_front();
            buffer.dropped += 1;
        }
        buffer.ring.push_back(data.clone());

        match &data {
            StreamData::Quote { .. } => {
                // Every unsealed print still short of its two quotes takes
                // this one. Filling only the newest starves an earlier print
                // whenever a second trade arrives before the first one's
                // quotes do, which on a liquid contract is most of them.
                // `unsealed_from` never exceeds the prints received, so the
                // range is within the deque.
                let unsealed = state.unsealed_from.saturating_sub(state.prints_dropped) as usize;
                for open in state
                    .prints
                    .range_mut(unsealed..)
                    .rev()
                    .take_while(|p| p.quotes_after.len() < 2)
                {
                    open.quotes_after.push(data.clone());
                }
                state.last_quote = Some(data);
            }
            StreamData::Trade { .. } => {
                state.prints.push_back(Print {
                    trade: data,
                    quote_before: state.last_quote.clone(),
                    quotes_after: Vec::new(),
                });
                while state.prints.len() > PRINTS {
                    state.prints.pop_front();
                    state.prints_dropped += 1;
                }
            }
            _ => {}
        }
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

pub const TOOL_NAMES: [&str; 5] = [
    "live_read",
    "live_prints",
    "live_market",
    "live_list",
    "live_stop",
];

/// Kinds the vendor offers for a security type, the default first.
///
/// Indices have no trade or quote stream — only price and market value — and
/// an index price arrives on the trade subscription in a trade-shaped
/// message, which is why `trade` is how it is asked for.
fn kinds_for(sec: SecType) -> &'static [SubscriptionKind] {
    match sec {
        SecType::Option => &[
            SubscriptionKind::Quote,
            SubscriptionKind::Trade,
            SubscriptionKind::MarketValue,
            SubscriptionKind::OpenInterest,
        ],
        // Open interest is a count of contracts outstanding, which a stock
        // does not have. Offering it opens a buffer that can never publish.
        SecType::Stock => &[
            SubscriptionKind::Quote,
            SubscriptionKind::Trade,
            SubscriptionKind::MarketValue,
        ],
        SecType::Index => &[SubscriptionKind::Trade, SubscriptionKind::MarketValue],
        _ => &[],
    }
}

fn resolve_kind(sec: SecType, raw: Option<&str>) -> Result<SubscriptionKind, ToolError> {
    let offered = kinds_for(sec);
    let Some(raw) = raw else {
        return offered.first().copied().ok_or_else(|| {
            ToolError::InvalidParams("sec_type must be option, stock or index".into())
        });
    };
    if let Some(kind) = offered.iter().find(|k| k.kind_str() == raw) {
        return Ok(*kind);
    }
    let names: Vec<&str> = offered.iter().map(|k| k.kind_str()).collect();
    Err(ToolError::InvalidParams(format!(
        "{raw} is not available for this security type; it offers {}",
        names.join(", ")
    )))
}

/// Whether the account may open the vendor's whole-market trade stream on
/// `sec`. The vendor gates it on the Pro tier for that asset class, and the
/// tier is read at authentication, so the refusal can say which tier the
/// account holds instead of leaving a subscription to be refused later.
fn pro_required(sec: SecType, tier: Option<SubscriptionTier>) -> Result<(), ToolError> {
    let class = match sec {
        SecType::Option => "Options",
        SecType::Stock => "Stocks",
        _ => {
            return Err(ToolError::InvalidParams(format!(
                "live_market covers option and stock: the vendor broadcasts every trade for \
                 those two only. Read {} per contract with live_read instead",
                sec.as_str().to_ascii_lowercase()
            )))
        }
    };
    if tier.is_some_and(|t| t >= SubscriptionTier::Pro) {
        return Ok(());
    }
    Err(ToolError::ServerError(format!(
        "live_market on {} needs a {class} Pro subscription: the vendor's whole-market trade \
         stream is Pro-only, and this account's {class} tier is {}. live_read and live_prints \
         on single {} contracts work at every tier",
        sec.as_str().to_ascii_lowercase(),
        tier.map_or("not reported".to_string(), |t| format!("{t:?}")),
        sec.as_str().to_ascii_lowercase()
    )))
}

/// The vendor's own definition of each subscribe response, from its
/// stream-verification page, so a dropped buffer can say what the code means.
fn rejection_meaning(code: StreamResponseType) -> &'static str {
    match code {
        StreamResponseType::Subscribed => "the request to subscribe was successful",
        StreamResponseType::Error => "an unknown error subscribing to the stream",
        StreamResponseType::MaxStreamsReached => {
            "streaming too many contracts; unsubscribe some (live_stop), upgrade the \
             subscription, or stop all streams"
        }
        StreamResponseType::InvalidPerms => {
            "no permission for the stream request; the subscription for this security type \
             may need upgrading"
        }
    }
}

/// The security types a tool can serve, which is not everything the vendor
/// has. A type offered and refused on every call is a call a model will
/// make and an answer it will never get, so this is the one list: the
/// schema advertises it and the call refuses against it.
fn sec_types_for(tool: &str) -> &'static [&'static str] {
    match tool {
        // A print is a trade with the quote that stood before it, and an
        // index has no quote stream. A whole-market stream is not offered
        // on indices at any tier.
        "live_prints" | "live_market" => &["option", "stock"],
        _ => &["option", "stock", "index"],
    }
}

/// The security type a public name stands for.
///
/// One mapping, read by the schema a caller composes against and by the call
/// that reads it, so the two cannot come to disagree about what a name means.
fn sec_named(name: &str) -> Option<SecType> {
    match name {
        "option" => Some(SecType::Option),
        "stock" => Some(SecType::Stock),
        "index" => Some(SecType::Index),
        _ => None,
    }
}

/// Say in the schema which kinds each security type actually takes.
///
/// A stock has no open interest and an index has neither a quote nor one, so
/// a flat enum over every kind lets a caller compose a request whose only
/// possible answer is a refusal. Both the enum and the per-type narrowing are
/// read out of [`kinds_for`], the list the call itself resolves against, so
/// the schema cannot advertise a kind the call would refuse.
fn with_kinds(mut schema: Value, sec_types: &[&str]) -> Value {
    let offered: Vec<(&str, Vec<&'static str>)> = sec_types
        .iter()
        .filter_map(|name| sec_named(name).map(|sec| (*name, sec)))
        .map(|(name, sec)| (name, kinds_for(sec).iter().map(|k| k.kind_str()).collect()))
        .collect();

    // The property's own enum is every kind some offered type takes; the
    // conditionals below cut it back per type.
    let mut union: Vec<&'static str> = Vec::new();
    for (_, kinds) in &offered {
        for k in kinds {
            if !union.contains(k) {
                union.push(k);
            }
        }
    }
    if let Some(kind) = schema
        .get_mut("properties")
        .and_then(|v: &mut Value| v.get_mut("kind"))
        .and_then(|v: &mut Value| v.as_object_mut())
    {
        kind.insert(&"enum", json!(union));
    }

    if let Some(all) = schema
        .get_mut("allOf")
        .and_then(|v: &mut Value| v.as_array_mut())
    {
        for (name, kinds) in offered {
            if kinds.len() == union.len() {
                continue;
            }
            all.push(json!({
                "if": {"properties": {"sec_type": {"const": name}}, "required": ["sec_type"]},
                "then": {"properties": {"kind": {"enum": kinds}}}
            }));
        }
    }
    schema
}

/// The contract properties shared by every tool that names one, plus
/// whatever the tool adds.
fn contract_schema(sec_types: &[&str], extra: Value) -> Value {
    let mut props = json!({
        "sec_type": {"type": "string", "enum": sec_types},
        "root": {"type": "string", "description": "Ticker or option root, e.g. AAPL."},
        "expiration": {"type": "integer", "minimum": 0, "maximum": 2147483647, "description": "YYYYMMDD. Options only."},
        "strike": {"type": "number", "description": "Strike in dollars. Options only."},
        "right": {"type": "string", "enum": ["C", "P"], "description": "Options only."}
    });
    if let (Some(props), Some(extra)) = (props.as_object_mut(), extra.as_object()) {
        for (k, v) in extra.iter() {
            props.insert(k, v.clone());
        }
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": props,
        "required": ["sec_type", "root"],
        // An option is one leg, and the three names that identify it are
        // required exactly when the type is option. Saying so here is what
        // stops a caller composing a request that can only be refused.
        "allOf": [{
            "if": {"properties": {"sec_type": {"const": "option"}}, "required": ["sec_type"]},
            "then": {"required": ["expiration", "strike", "right"]},
            "else": {"not": {"anyOf": [
                {"required": ["expiration"]},
                {"required": ["strike"]},
                {"required": ["right"]}
            ]}}
        }]
    })
}

/// The schema of a predicate: clauses over `fields`.
fn clauses_schema(description: &str, fields: &[&str]) -> Value {
    json!({
        "type": "array",
        "description": description,
        "maxItems": MAX_CLAUSES,
        "items": {
            "type": "object",
            "properties": {
                "field": {"type": "string", "enum": fields},
                "op": {"type": "string", "enum": [">", ">=", "<", "<=", "==", "!=", "inside", "outside"]},
                "value": {"description": "A number; a field name to compare against, as in bid >= ask; or [low, high] for inside and outside.", "anyOf": [
                    {"type": "number"},
                    {"type": "string", "enum": fields},
                    {"type": "array"}
                ]}
            },
            // A range takes two bounds and a comparison takes one value, so
            // the shape follows from the operator rather than being left to
            // a refusal.
            "allOf": [{
                "if": {"properties": {"op": {"enum": ["inside", "outside"]}}, "required": ["op"]},
                "then": {"properties": {"value": {
                    "type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2
                }}},
                "else": {"properties": {"value": {"not": {"type": "array"}}}}
            }],
            "required": ["field", "op", "value"]
        }
    })
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "live_read",
            "description": "What has happened to one contract since you last looked. \
                For what it is right now, use a snapshot instead: stock_snapshot_quote, \
                option_snapshot_trade and their siblings answer in one call, hold nothing, \
                and are the right tool for a price. This one holds a subscription on the \
                account and its first read returns nothing, so reaching for it to read a \
                single value costs two calls and leaves a buffer open for fifteen minutes. \
                Reach for it when the gap between two looks is the thing you care about: it \
                serves every row in that gap, and says so when it could not see them all, \
                which is what no snapshot can tell you. The first read opens the \
                subscription and returns nothing yet; read again a second or two later. After \
                that the window defaults to everything since your last read of this buffer; pass \
                seconds for a fixed lookback. age_ms is how old the newest row returned is, \
                never how long ago the feed last carried anything: an index reports \
                about once a second, so seconds of age are normal there and stale on an option \
                quote. Rows are held within a memory budget, not for a length of time: \
                covers_seconds is how far back the rows held reach right now. clipped means \
                this answer is not the whole of the window you asked for: either it reaches \
                further back than the rows held, which a liquid buffer can hit within seconds, \
                or the feed was interrupted, or events were discarded, inside it. Treat it as \
                the one field that says whether anything is missing, whatever the cause. rows_in_window is how many rows the window held, which is not how \
                many came back: the tail is capped, so a larger count means you are seeing the \
                newest of more. The tail is the vendor's messages as sent, condition and \
                exchange codes intact. vendor_ohlcvc is the vendor's own bar for the contract \
                as last sent, served as is; nothing here builds a bar from trades, because \
                condition, cancel and size rules are yours to choose, and nothing here \
                summarises them. feed_dropped_since_last_read counts events the SDK discarded \
                because this server fell behind; while it is not zero any buffer may be missing \
                rows, and clipped says so. kind \
                defaults to quote; an index has no quote stream, so it defaults to trade, which \
                carries the index price. market_value is a derived midpoint, not a quote. Times \
                are Eastern. A buffer goes 15 minutes unread and the next call to any of these \
                tools closes it; live_stop closes it now. A read that finds \
                the feed refused the subscription after accepting it says so and releases the \
                buffer; reading again re-subscribes. In the answer: window_from says whether the \
                window came from your seconds or from your last read, window_seconds is how far \
                back it reaches, columns names the fields of each tail row in order, and date is \
                the trading date they share, absent and carried on each row instead when they \
                span more than one.",
            "inputSchema": with_kinds(contract_schema(sec_types_for("live_read"), json!({
                "kind": {"type": "string",
                         "description": "Default quote, or trade for an index."},
                "seconds": {"type": "number", "minimum": 0, "description": "Fixed lookback. Default: since your last read."},
                "tail": {"type": "integer", "minimum": 0, "description": "Newest rows served verbatim. Default 10, capped at 50. Read more often rather than asking for more rows."}
            })), sec_types_for("live_read"))
        }),
        json!({
            "name": "live_prints",
            "description": "Trades on one contract since you last looked, newest last, each \
                with the quote that stood before it; the feed also sends the two quotes after \
                a print, and quotes_after returns them. For the last trade alone use a \
                snapshot, stock_snapshot_trade or option_snapshot_trade; for a past interval \
                use the trade-quote history endpoint. This pairs each trade with the quote \
                beside it as the feed delivers them, which is an association no snapshot \
                carries and no two snapshots can reconstruct. Opens the trade and quote \
                subscriptions if they are not already held. A print needs both: a contract nothing was watching has \
                none yet, so call again a second or two later. A contract already being read \
                for its trades has whatever printed since, without the quotes beside them, \
                because a print takes only the quotes that arrived while both were being \
                watched; prints from after this call have them. \
                new_since_last_read counts prints since your last live_prints on this \
                contract; live_read keeps its own count of trades. Prints are held within a memory budget, and clipped means older ones \
                were discarded before you asked. feed_interrupted means there was an interval \
                before this read that nothing was watching, so prints from it were never held: \
                the connection broke, or a leg expired and was reopened between your calls. Each print carries quote_before, the quote that stood when it \
                traded, and date when the prints span more than one trading date. A print is a \
                trade and the quote that stood \
                before it, and an index has no quote stream, so indices are not on this tool; \
                live_read with kind trade carries the index price. \
                Times are Eastern.",
            "inputSchema": contract_schema(sec_types_for("live_prints"), json!({
                "count": {"type": "integer", "minimum": 0, "description": "Newest prints. Default 20."},
                "quotes_after": {"type": "boolean", "description": "Include the two quotes after each print. Default false."}
            }))
        }),
        json!({
            "name": "live_market",
            "description": "Every trade across the whole option or stock market from one \
                subscription, selected as it arrives. State a selection: contract filters \
                (root, expiration, right, strike_min, strike_max), clauses over the vendor's \
                fields (where), and one field to rank by (rank_by, largest first unless \
                ascending), with limit rows kept (default 20, at most 50). The server applies \
                it to every print the feed delivers and keeps the top rows, or the newest \
                without rank_by, until your next read, which returns them and starts again. So \
                a read answers the largest prints since you last looked exactly, whatever the \
                rate: nothing here is windowed by a buffer. Nothing is computed: you select \
                and rank the vendor's own fields. examined and matched say how many prints the \
                selection saw and how many passed since your last read; returned is what you \
                got; unranked counts matches without the rank field, such as a quote field on \
                a print with no quote ahead of it. A print can go missing two ways and both \
                are reported: feed_dropped_since_last_read counts what the feed threw away, \
                and feed_interrupted means there was an interval before this read that no \
                selection was watching, so prints from it were never counted at all. The \
                connection breaking is one cause; the others are this market having been \
                released and reopened, and a replaced selection having examined prints the \
                one before it never saw. since_seconds is how long the selection stood before \
                this read took it. Sending different parameters replaces the \
                selection; the rows that come back were kept under the previous one, shown as \
                selected_by. Needs an Options Pro or Stocks Pro subscription; the error says \
                which when the account lacks it. The first call installs the selection, opens \
                the subscription and returns nothing yet; call again a second or two later. A \
                market buffer is not held alongside per-contract trade or quote buffers on the same \
                security type, because the feed would deliver those contracts twice; live_stop \
                one side. Once 15 minutes unread, the next call to any of these tools closes \
                it; live_stop with sec_type alone closes it now. Times are Eastern.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sec_type": {"type": "string", "enum": sec_types_for("live_market")},
                    "root": {"type": "string", "description": "Only this ticker or option root."},
                    "expiration": {"type": "integer", "description": "YYYYMMDD. Options only."},
                    "right": {"type": "string", "enum": ["C", "P"], "description": "Options only."},
                    "strike_min": {"type": "number", "description": "Dollars, inclusive. Options only."},
                    "strike_max": {"type": "number", "description": "Dollars, inclusive. Options only."},
                    "where": clauses_schema("Clauses that must all hold, over the trade's fields and the quote before it. Every field is the vendor's own except spread, which is ask minus bid. At most 8.", &PRINT_FIELDS),
                    "rank_by": {"type": "string", "enum": PRINT_FIELDS, "description": "Keep the top rows by this field of the trade or the quote before it. Default: the newest."},
                    "ascending": {"type": "boolean", "description": "Smallest first. Default false."},
                    "limit": {"type": "integer", "minimum": 1, "description": "Rows kept between reads. Default 20, capped at 50."}
                },
                "required": ["sec_type"],
                "additionalProperties": false,
                // The fields that describe an option belong to an option;
                // on anything else a selection on them matches nothing.
                "allOf": [{
                    "if": {"properties": {"sec_type": {"const": "option"}}, "required": ["sec_type"]},
                    "then": {},
                    "else": {"not": {"anyOf": [
                        {"required": ["expiration"]},
                        {"required": ["right"]},
                        {"required": ["strike_min"]},
                        {"required": ["strike_max"]}
                    ]}}
                }]
            }
        }),
        json!({
            "name": "live_list",
            "description": "Every buffer this server holds: rows received and held, the age of \
                the newest, how long since it was read and when it expires, plus the feed's \
                state. on_feed is whether the feed itself still carries the subscription; a \
                buffer the feed dropped is released on its next read. on_feed_not_held lists \
                subscriptions the feed carries for no buffer. last_rejection is the feed's most \
                recent refusal of a subscribe, with the vendor's meaning. feed_dropped_events is \
                the SDK's running count of events it discarded because this server fell behind. \
                An age that keeps \
                growing while the feed says Connected is a contract that has gone quiet, not a \
                fault. A feed that died or spent its reconnect budget is restarted by the \
                next live_read; one the SDK is still reconnecting on its own is left to it.",
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}}
        }),
        json!({
            "name": "live_stop",
            "description": "Close every buffer held for a contract and release its subscriptions; \
                with sec_type alone, close the whole-market buffer for that security type. A \
                subscription the feed did not release stays held; the buffer is kept so a \
                later stop, or the sweep once it is idle again, tries to release it. A buffer 15 minutes unread is closed by the next call to any of these \
                tools, so nothing is released while the server sits idle.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": contract_schema(sec_types_for("live_stop"), json!({}))
                    .get("properties")
                    .cloned()
                    .unwrap_or_default(),
                "required": ["sec_type"],
                // Naming a contract means naming all of it: an option root
                // without its leg identifies nothing, and a leg without a
                // root would close a whole market instead of one contract.
                // An index has no whole-market stream, so it needs a root.
                "allOf": [{
                    "if": {"properties": {"sec_type": {"const": "index"}}, "required": ["sec_type"]},
                    "then": {"required": ["root"]}
                }, {
                    "if": {
                        "properties": {"sec_type": {"const": "option"}},
                        "required": ["sec_type", "root"]
                    },
                    "then": {"required": ["expiration", "strike", "right"]},
                    "else": {"not": {"anyOf": [
                        {"required": ["expiration"]},
                        {"required": ["strike"]},
                        {"required": ["right"]}
                    ]}}
                }]
            }
        }),
    ]
}

/// `ms_of_day` as a clock time, Eastern. The vendor stamps every row this way.
fn clock(ms_of_day: i32) -> String {
    let ms = ms_of_day.max(0) as u32;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        ms % 1_000
    )
}

/// The columns of a row and their values, declared once so a tabular tail
/// and a keyed object cannot disagree about what a column means.
fn fields(data: &StreamData) -> Vec<(&'static str, Value)> {
    match data {
        StreamData::Quote {
            ms_of_day,
            bid_size,
            bid_exchange,
            bid,
            bid_condition,
            ask_size,
            ask_exchange,
            ask,
            ask_condition,
            ..
        } => vec![
            ("time", json!(clock(*ms_of_day))),
            ("bid_size", json!(*bid_size)),
            ("bid", json!(*bid)),
            ("ask", json!(*ask)),
            ("ask_size", json!(*ask_size)),
            ("bid_exchange", json!(*bid_exchange)),
            ("ask_exchange", json!(*ask_exchange)),
            ("bid_condition", json!(*bid_condition)),
            ("ask_condition", json!(*ask_condition)),
        ],
        StreamData::Trade {
            ms_of_day,
            sequence,
            condition,
            size,
            exchange,
            price,
            ..
        } => vec![
            ("time", json!(clock(*ms_of_day))),
            ("price", json!(*price)),
            ("size", json!(*size)),
            ("condition", json!(*condition)),
            ("exchange", json!(*exchange)),
            ("sequence", json!(*sequence)),
        ],
        StreamData::OpenInterest {
            ms_of_day,
            open_interest,
            ..
        } => vec![
            ("time", json!(clock(*ms_of_day))),
            ("open_interest", json!(*open_interest)),
        ],
        StreamData::MarketValue {
            ms_of_day,
            market_bid,
            market_ask,
            market_price,
            ..
        } => vec![
            ("time", json!(clock(*ms_of_day))),
            ("market_bid", json!(*market_bid)),
            ("market_ask", json!(*market_ask)),
            ("market_price", json!(*market_price)),
        ],
        StreamData::Ohlcvc {
            ms_of_day,
            open,
            high,
            low,
            close,
            volume,
            count,
            ..
        } => vec![
            ("time", json!(clock(*ms_of_day))),
            ("open", json!(*open)),
            ("high", json!(*high)),
            ("low", json!(*low)),
            ("close", json!(*close)),
            ("volume", json!(*volume)),
            ("count", json!(*count)),
        ],
        other => vec![("debug", json!(format!("{other:?}")))],
    }
}

fn object(data: &StreamData) -> Value {
    let mut out = json!({});
    if let Some(obj) = out.as_object_mut() {
        for (k, v) in fields(data) {
            obj.insert(k, v);
        }
    }
    out
}

/// A row with how long ago it arrived. `dated` also carries its trading
/// date, for a response whose rows do not share one and so cannot name a
/// date for the collection.
fn aged_object_dated(data: &StreamData, now: u64, dated: bool) -> Value {
    let mut out = object(data);
    if let Some(obj) = out.as_object_mut() {
        // Zero is the SDK's fallback for a clock it could not read. Aging
        // from it would report the time since the epoch and present an
        // unknown age as decades.
        if let Some(seen) = seen_ms(data) {
            obj.insert("age_ms", Value::from(now.saturating_sub(seen)));
        }
        if dated {
            if let Some(day) = date_of(data) {
                obj.insert("date", Value::from(day));
            }
        }
    }
    out
}

/// The single trading date every row shares, or `None` when they do not
/// share one.
///
/// A response names one date for the whole collection, which is both true
/// and cheap while the rows are from one session. Rows held across a
/// session boundary are not, and naming one of the two dates would silently
/// restamp the other. When they disagree the date travels on each row
/// instead, and this returns `None`.
fn one_date<'a>(rows: impl Iterator<Item = &'a StreamData>) -> Option<i32> {
    let mut seen = None;
    for d in rows {
        match (date_of(d), seen) {
            (Some(d), None) => seen = Some(d),
            (Some(d), Some(s)) if d != s => return None,
            _ => {}
        }
    }
    seen
}

/// How old the newest row a selection is returning is, which is not how old
/// the tape is: a narrow selection holds an old print while the market
/// carries on, and calling that fresh is the one thing this surface exists
/// not to do.
fn rows_age_ms(rows: &[Print], now: u64) -> Option<u64> {
    rows.iter()
        .filter_map(|p| seen_ms(&p.trade))
        .max()
        .map(|newest| now.saturating_sub(newest))
}

/// Where the window a read names begins.
///
/// A named window is the interval the caller asked for and nothing else.
/// The default window is "since you last looked", which has to stretch back
/// over a row decoded before that read and delivered after it.
fn window_start(window: Option<u64>, r: &Reading) -> u64 {
    if window.is_some() {
        return r.floor;
    }
    r.tail
        .iter()
        .chain(r.oldest.iter())
        .filter_map(seen_ms)
        .fold(r.floor, u64::min)
}

fn row(data: &StreamData) -> Value {
    fields(data)
        .into_iter()
        .map(|(_, v)| v)
        .collect::<Vec<_>>()
        .into()
}

fn seconds(ms: u64) -> f64 {
    ms as f64 / 1_000.0
}

fn stream_error(what: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::ServerError(format!("{what}: {}", sanitize_error(&e.to_string())))
}

fn feed_state(client: &Client) -> String {
    format!("{:?}", client.stream().connection_status())
}

/// Make sure the feed delivers into the registry.
///
/// The SDK reconnects on its own after a drop, and says so in its status
/// while it is trying. A budget it has given up on and a dispatcher fault
/// both read as terminal, and both need the dead session retired and a new
/// one started; a new session knows nothing of the buffers this process still
/// holds, so they are reopened from the registry rather than from what the
/// old session tracked.
fn ensure_streaming(client: &Client, reg: &'static Registry) -> Result<u64, ToolError> {
    let stream = client.stream();
    match stream.connection_status() {
        ConnectionStatus::NotStarted => {}
        // A dead session still occupies the slot until it is stopped.
        ConnectionStatus::Disconnected | ConnectionStatus::ReconnectsExhausted => {
            stream.stop_streaming();
        }
        _ => return Ok(stream.dropped_event_count()),
    }
    // Stopping is asynchronous: the retired dispatcher can still be
    // running, and a row it delivers after the reset would be recorded as
    // the new session's and would prove a resumption that has not happened.
    // Wait for it to finish before anything is reset.
    if !stream.await_drain(DRAIN) {
        return Err(ToolError::ServerError(format!(
            "the previous feed session was still delivering after {} s; nothing was reset, so \
             try again",
            DRAIN.as_secs()
        )));
    }
    // A session that died or was never started delivered nothing since the
    // buffers last saw the feed; the correlation across that interval closes,
    // and the new session's discard count starts from nothing.
    reg.gap(now_ms());
    reg.restarted();
    stream
        .start_streaming(move |event: &StreamEvent| match event {
            StreamEvent::Data(data) => reg.ingest(data.clone()),
            // The SDK reconnects on its own, and whatever the feed sent
            // between the drop and the new session was never seen.
            StreamEvent::Control(
                StreamControl::Disconnected { .. } | StreamControl::Reconnecting { .. },
            ) => reg.gap(now_ms()),
            StreamEvent::Control(StreamControl::ReqResponse { result, .. })
                if *result != StreamResponseType::Subscribed =>
            {
                reg.note_rejection(*result, now_ms());
            }
            _ => {}
        })
        .map_err(|e| stream_error("could not start streaming", e))?;
    let (per_contract, full) = reg.subscriptions();
    match stream.restore_subscriptions(&per_contract, &full) {
        Ok(()) => Ok(stream.dropped_event_count()),
        // The session is up and reads as Connected from here on, so nothing
        // later would notice a buffer the restore left behind. Release those
        // now and say which: the next read of each opens it afresh.
        Err(Error::PartialReconnect { failed }) => Err(ToolError::ServerError(format!(
            "the feed session restarted and could not reopen {}; released, and reading each \
             again re-subscribes",
            release_unrestored(reg, &failed).join(", ")
        ))),
        Err(e) => Err(stream_error("could not restart the feed session", e)),
    }
}

/// Drop the buffers whose subscriptions a restore could not reopen, and name
/// them. A full-stream failure comes back under the SDK's marker contract,
/// an empty symbol carrying only the security type.
fn release_unrestored(reg: &Registry, failed: &[(SubscriptionKind, Contract)]) -> Vec<String> {
    failed
        .iter()
        .filter_map(|(kind, contract)| {
            if contract.symbol.is_empty() {
                full_subscription(*kind, contract.sec_type)
            } else {
                Some(subscription(*kind, contract))
            }
        })
        .map(|sub| {
            reg.forget(&sub);
            label(&sub)
        })
        .collect()
}

/// Everything the feed carries, in the SDK's own accounting: the one record
/// of which subscribes it kept after answering them.
fn on_feed(client: &Client) -> Result<Subs, ToolError> {
    let stream = client.stream();
    let mut subs: Subs = stream
        .active_subscriptions()
        .map_err(|e| stream_error("could not read the feed's subscriptions", e))?
        .into_iter()
        .map(|(kind, contract)| subscription(kind, &contract))
        .collect();
    subs.extend(
        stream
            .active_full_subscriptions()
            .map_err(|e| stream_error("could not read the feed's subscriptions", e))?
            .into_iter()
            .filter_map(|(kind, sec)| full_subscription(kind, sec)),
    );
    Ok(subs)
}

/// Release a buffer the feed does not carry and say so in words, with the
/// feed's most recent refusal and what the vendor says it means.
fn dropped_by_feed(reg: &Registry, sub: &Subscription, now: u64) -> ToolError {
    reg.forget(sub);
    let why = reg.last_rejection().map_or_else(String::new, |(code, at)| {
        format!(
            " The feed's most recent rejection was {code}, {:.1} s ago: {}.",
            seconds(now.saturating_sub(at)),
            rejection_meaning(code)
        )
    });
    ToolError::ServerError(format!(
        "the feed accepted the {} subscription and then dropped it, so nothing has arrived \
         since; it has been released and reading again re-subscribes.{why} A subscribe is \
         refused with {} when {}, and with {} when {}.",
        label(sub),
        StreamResponseType::MaxStreamsReached,
        rejection_meaning(StreamResponseType::MaxStreamsReached),
        StreamResponseType::InvalidPerms,
        rejection_meaning(StreamResponseType::InvalidPerms),
    ))
}

/// Fail a read whose buffer the feed no longer carries. Runs on every read
/// after the first: the feed answers a subscribe after the call that sent
/// it returns, so the read that opened the buffer cannot know yet.
fn reconcile(
    client: &Client,
    reg: &Registry,
    subs: &[Subscription],
    now: u64,
) -> Result<(), ToolError> {
    if subs.is_empty() {
        return Ok(());
    }
    let live = on_feed(client)?;
    match dropped_of(subs, &live) {
        Some(sub) => Err(dropped_by_feed(reg, sub, now)),
        None => Ok(()),
    }
}

/// The first of `subs` the feed is no longer carrying, or `None` while it
/// carries them all.
///
/// Pulled out of the feed call so the decision is reachable by a test: the
/// call around it needs a live client, and a check that only exists inside an
/// untestable branch is a check nobody can prove runs the right way round.
fn dropped_of<'a>(subs: &'a [Subscription], live: &[Subscription]) -> Option<&'a Subscription> {
    subs.iter().find(|s| !live.contains(s))
}

fn parse_sec(args: &Value) -> Result<SecType, ToolError> {
    parse_sec_of(args, &["option", "stock", "index"])
}

/// `offered` is what the calling tool can serve. Recommending a type it
/// always refuses sends the caller straight into a second refusal.
fn parse_sec_of(args: &Value, offered: &[&str]) -> Result<SecType, ToolError> {
    let raw = arg(args, "sec_type", &offered.join(", "), |v| {
        v.as_str().map(str::to_string)
    })?;
    match raw
        .as_deref()
        .filter(|r| offered.contains(r))
        .and_then(sec_named)
    {
        Some(sec) => Ok(sec),
        None => Err(ToolError::InvalidParams(format!(
            "sec_type must be {}",
            offered.join(" or ")
        ))),
    }
}

/// The names that identify one option leg, which only an option has.
const OPTION_LEG: [&str; 3] = ["expiration", "strike", "right"];

/// The market a rootless `live_stop` names, or why it names none.
///
/// Pulled out of the request arm so the list it reads is reachable by a test:
/// the arm around it needs a live client, and a refusal that exists only
/// inside an untestable branch is a refusal nobody can prove is there. The
/// tool name is an argument so the test cannot supply the list itself and
/// call that an observation.
fn whole_market_target(name: &str, args: &Value) -> Result<SecType, ToolError> {
    let sec = parse_sec_of(args, sec_types_for(name))?;
    if sec == SecType::Index {
        return Err(ToolError::InvalidParams(
            "an index has no whole-market stream to close; live_read follows one index at a \
             time and live_stop with its root closes it"
                .into(),
        ));
    }
    // Without a root this closes the whole market. Contract identifiers say
    // the caller meant one contract, and closing every one of them instead is
    // not a smaller mistake for being silent.
    if let Some(named) = OPTION_LEG
        .iter()
        .find(|k| args.get(*k).is_some_and(|v: &Value| !v.is_null()))
    {
        return Err(ToolError::InvalidParams(format!(
            "{named} identifies one contract, and sec_type alone closes the whole \
             {} market; give root to close that contract, or drop {named}",
            sec.as_str().to_ascii_lowercase()
        )));
    }
    Ok(sec)
}

fn parse_contract(args: &Value) -> Result<(SecType, Contract), ToolError> {
    let sec = parse_sec(args)?;
    let root = arg(args, "root", "a ticker symbol", |v| {
        v.as_str().map(str::to_string)
    })?
    .ok_or_else(|| ToolError::InvalidParams("root is required".into()))?;
    let root = root.as_str();
    let contract = match sec {
        SecType::Stock | SecType::Index => {
            // Naming a leg on something that has none means the caller meant
            // a different contract than the one this would act on, and
            // acting on it anyway is the wrong answer to a clear question.
            if let Some(named) = OPTION_LEG
                .iter()
                .find(|k| args.get(*k).is_some_and(|v: &Value| !v.is_null()))
            {
                return Err(ToolError::InvalidParams(format!(
                    "{named} identifies an option leg, and {} is not an option; drop it, or ask \
                     for sec_type option",
                    sec.as_str().to_ascii_lowercase()
                )));
            }
            if sec == SecType::Stock {
                Contract::stock(root)
            } else {
                Contract::index(root)
            }
        }
        _ => {
            let exp = arg(args, "expiration", "a date as YYYYMMDD", Value::as_u64)?;
            let strike = arg(args, "strike", "a strike in dollars", Value::as_f64)?;
            let right = arg(args, "right", "C or P", |v| v.as_str().map(str::to_string))?;
            let (Some(exp), Some(strike), Some(right)) = (exp, strike, right.as_deref()) else {
                return Err(ToolError::InvalidParams(
                    "an option needs expiration, strike and right".into(),
                ));
            };
            Contract::option(
                root,
                OptionLeg {
                    expiration: &exp.to_string(),
                    strike: &strike.to_string(),
                    right,
                },
            )
            .map_err(|e| ToolError::InvalidParams(format!("contract: {e}")))?
        }
    };
    Ok((sec, contract))
}

fn parse_market_query(args: &Value, limit: usize) -> Result<MarketQuery, ToolError> {
    // A narrowing field that is present but unreadable is refused, never
    // dropped. Dropping one widens the selection: a caller asking for one
    // expiration would be handed every expiration and told nothing.
    fn narrowing<T>(
        args: &Value,
        key: &str,
        what: &str,
        read: impl Fn(&Value) -> Option<T>,
    ) -> Result<Option<T>, ToolError> {
        match args.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => read(v)
                .map(Some)
                .ok_or_else(|| ToolError::InvalidParams(format!("{key} must be {what}"))),
        }
    }
    let text = |v: &Value| v.as_str().map(str::to_string);
    let is_call = match narrowing(args, "right", "C or P", text)?.as_deref() {
        None => None,
        Some("C") => Some(true),
        Some("P") => Some(false),
        Some(_) => return Err(ToolError::InvalidParams("right must be C or P".into())),
    };
    Ok(MarketQuery {
        root: narrowing(args, "root", "a ticker symbol", text)?,
        expiration: narrowing(args, "expiration", "a date as YYYYMMDD", |v| {
            v.as_i64().and_then(|e| i32::try_from(e).ok())
        })?,
        is_call,
        strike_min: narrowing(args, "strike_min", "a number of dollars", Value::as_f64)?,
        strike_max: narrowing(args, "strike_max", "a number of dollars", Value::as_f64)?,
        clauses: args
            .get("where")
            .map_or(Ok(Vec::new()), |v| parse_clauses(v, &PRINT_FIELDS))?,
        rank_by: args
            .get("rank_by")
            .map(|v| field_name(Some(v), "rank_by", &PRINT_FIELDS))
            .transpose()?,
        ascending: arg(args, "ascending", "true or false", Value::as_bool)?.unwrap_or(false),
        // A selection that keeps nothing answers nothing, and quietly
        // keeping one instead answers a question the caller did not ask.
        limit: match limit {
            0 => {
                return Err(ToolError::InvalidParams(
                    "limit must keep at least one row; a selection of none has nothing to \
                     report"
                        .into(),
                ))
            }
            n => n,
        },
    })
}

/// A selection as the caller stated it, echoed so a read says what its
/// rows were kept under.
fn query_json(q: &MarketQuery) -> Value {
    json!({
        "root": q.root,
        "expiration": q.expiration,
        "right": q.is_call.map(|c| if c { "C" } else { "P" }),
        "strike_min": q.strike_min,
        "strike_max": q.strike_max,
        "where": q.clauses.iter().map(clause_json).collect::<Vec<_>>(),
        "rank_by": q.rank_by,
        "ascending": q.ascending,
        "limit": q.limit
    })
}

/// Close what the sweep freed, so a forgotten buffer cannot hold an allowance
/// the next caller needs. Nobody is waiting on these, so a failure is logged
/// rather than returned — and the buffer put back, expired, so the next sweep
/// tries the close again instead of leaving the feed holding it.
fn close_expired(client: &Client, reg: &Registry, expired: Subs, now: u64) {
    for sub in expired {
        if let Err(e) = client.stream().unsubscribe(sub.clone()) {
            tracing::warn!(sub = %label(&sub), error = %e, "expired buffer left its subscription open; will retry");
            reg.reinstate(&sub, now);
        }
    }
}

/// Open on the feed what a read just opened in the registry, rolling back
/// what did not get there if the feed refuses.
fn open_on_feed(client: &Client, reg: &Registry, subs: &[Subscription]) -> Result<(), ToolError> {
    for (i, sub) in subs.iter().enumerate() {
        if let Err(e) = client.stream().subscribe(sub.clone()) {
            roll_back(reg, subs, i);
            return Err(stream_error(
                &format!("could not subscribe {}", label(sub)),
                e,
            ));
        }
    }
    Ok(())
}

/// Drop the buffer whose subscribe failed and every one after it that was
/// never attempted. A buffer left recorded but never subscribed would look
/// held to the next call, which would skip subscribing it and read nothing.
fn roll_back(reg: &Registry, subs: &[Subscription], failed_at: usize) {
    for sub in &subs[failed_at..] {
        reg.forget(sub);
    }
}

/// Close subscriptions on the feed, dropping each buffer only once the feed
/// has let its subscription go. What the feed kept stays held and is named,
/// so the caller is not told an allowance came back while the feed holds it.
fn close_on_feed(client: &Client, reg: &Registry, subs: Subs) -> (usize, Vec<String>) {
    let mut done = 0;
    let mut failures = Vec::new();
    for sub in subs {
        match client.stream().unsubscribe(sub.clone()) {
            Ok(()) => {
                reg.forget(&sub);
                done += 1;
            }
            Err(e) => failures.push(format!(
                "{}: {}",
                label(&sub),
                sanitize_error(&e.to_string())
            )),
        }
    }
    (done, failures)
}

/// Dispatch for the stream tools; `None` when `name` is not one of them.
pub async fn try_execute(
    client: Option<&Client>,
    name: &str,
    args: &Value,
) -> Option<Result<Value, ToolError>> {
    if !TOOL_NAMES.contains(&name) {
        return None;
    }
    let Some(client) = client else {
        return Some(Err(ToolError::ServerError(
            "not connected to ThetaData yet; retry shortly".into(),
        )));
    };
    if let Err(e) = only_declared_arguments(name, args) {
        return Some(Err(e));
    }
    Some(execute(client, name, args))
}

/// Refuse an argument the tool does not declare.
///
/// An unknown name is a question the caller meant to ask and the server did
/// not hear. `strike` on a whole-market read is the shape of it: a caller
/// narrowing to one strike, ignored, handed every strike on the root, and
/// told nothing. The tool's own schema is the list, so the two cannot drift.
fn only_declared_arguments(name: &str, args: &Value) -> Result<(), ToolError> {
    let Some(supplied) = args.as_object() else {
        return Ok(());
    };
    let Some(schema) = tool_definitions().into_iter().find(|t| t["name"] == name) else {
        return Ok(());
    };
    let declared = &schema["inputSchema"]["properties"];
    let mut unknown: Vec<&str> = supplied
        .iter()
        .map(|(k, _)| k)
        .filter(|k| declared.get(*k).is_none())
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let mut known: Vec<&str> = declared
        .as_object()
        .map(|d| d.iter().map(|(k, _)| k).collect())
        .unwrap_or_default();
    known.sort_unstable();
    Err(ToolError::InvalidParams(format!(
        "{name} does not take {}; it takes {}",
        unknown.join(", "),
        known.join(", ")
    )))
}

/// An argument the caller supplied, or the default when absent.
///
/// A value that is present and cannot be read is refused, never replaced by
/// the default. Replacing it answers a different question from the one
/// asked, and says nothing: `ascending: "true"` would rank the wrong way,
/// `kind: 123` would read the wrong buffer, `quotes_after: "true"` would drop
/// the quotes the caller asked for, and a negative count would silently
/// become twenty.
fn arg<T>(
    args: &Value,
    key: &str,
    what: &str,
    read: impl Fn(&Value) -> Option<T>,
) -> Result<Option<T>, ToolError> {
    match args.get(key).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => read(v)
            .map(Some)
            .ok_or_else(|| ToolError::InvalidParams(format!("{key} must be {what}"))),
    }
}

/// A whole number of rows: present and negative, fractional or oversized is
/// refused rather than rounded into a default.
fn count_arg(args: &Value, key: &str, what: &str, default: usize) -> Result<usize, ToolError> {
    Ok(arg(args, key, what, |v| v.as_u64().map(|n| n as usize))?.unwrap_or(default))
}

/// What a close was asked to release: a whole market, or one contract.
///
/// The name the answer carries follows from this rather than from a literal
/// at each call site, so a market cannot come back under `contract`.
enum Closing<'a> {
    Market(SecType),
    Contract(&'a Contract),
}

/// The answer a close hands back.
///
/// Pulled out of the request arm so the count is reachable by a test. A
/// subscription the feed would not release is named in `failed_to_close` and
/// is not one that closed: counting it would report a leak as a clean close,
/// which is the number a caller decides on.
fn stop_response(what: Closing<'_>, closed: (usize, Vec<String>)) -> Value {
    let (done, failures) = closed;
    let (key, named) = match what {
        Closing::Market(sec) => ("sec_type", sec.as_str().to_ascii_lowercase()),
        Closing::Contract(c) => ("contract", c.to_string()),
    };
    let mut out = json!({
        "subscriptions_closed": done,
        "failed_to_close": failures,
    });
    if let Some(obj) = out.as_object_mut() {
        obj.insert(key, Value::from(named.as_str()));
    }
    out
}

/// The answer a whole-market read hands back.
///
/// Pulled out of the request arm so which value lands in which field is
/// reachable by a test. The age of the rows returned and the age of the
/// market are two different numbers a line apart, and a swap between them
/// was the thing this surface exists not to do, assigned where nothing could
/// check it.
fn market_response(
    sec: SecType,
    feed: String,
    q: &MarketQuery,
    m: &MarketReading,
    now: u64,
) -> Value {
    // One date for the whole selection while the prints agree; otherwise it
    // travels on each print.
    let market_date = one_date(
        m.rows
            .iter()
            .flat_map(|p| std::iter::once(&p.trade).chain(p.quote_before.iter())),
    );
    json!({
        "sec_type": sec.as_str().to_ascii_lowercase(),
        "feed": feed,
        "subscribed_now": m.first,
        "since_seconds": seconds(now.saturating_sub(m.previous_ms)),
        "received": m.received,
        "new_since_last_read": m.new_since_last_read,
        "feed_dropped_since_last_read": m.feed_dropped_since_last_read,
        "feed_interrupted": m.gap,
        // The age of what came back, not of the newest print on the market: a
        // narrow selection can hold an old row while the tape is busy, and
        // calling that fresh is the one thing this surface exists not to do.
        "age_ms": rows_age_ms(&m.rows, now),
        "feed_age_ms": m.newest_ms.map(|s| now.saturating_sub(s)),
        "examined": m.examined,
        "matched": m.matched,
        "unranked": m.unranked,
        "returned": m.rows.len(),
        "selection": query_json(q),
        "selected_by": m.selected_by.as_ref().map(query_json),
        "date": market_date,
        "prints": m.rows.iter().map(|p| json!({
            "date": market_date.is_none().then(|| date_of(&p.trade)).flatten(),
            "contract": contract_of(&p.trade).map(ToString::to_string),
            "trade": object(&p.trade),
            "quote_before": p.quote_before.as_ref().map(|q| {
                aged_object_dated(q, now, market_date.is_none())
            })
        })).collect::<Vec<_>>()
    })
}

/// The answer a listing hands back.
///
/// Pulled out of the request arm so which value lands in which field is
/// reachable by a test. A listing is what a caller reads to decide whether a
/// buffer is still on the feed and when it will be released, and both of those
/// were assigned where nothing could check them.
fn list_response(
    feed: String,
    feed_drops: u64,
    rejection: Option<(StreamResponseType, u64)>,
    rows: &[Holding],
    live: Option<&[Subscription]>,
    now: u64,
) -> Value {
    let unheld = live.map(|live| {
        live.iter()
            .filter(|s| !rows.iter().any(|h| h.sub == **s))
            .map(label)
            .collect::<Vec<_>>()
    });
    json!({
        "feed": feed,
        "feed_dropped_events": feed_drops,
        "last_rejection": rejection.map(|(code, at)| json!({
            "result": code.to_string(),
            "meaning": rejection_meaning(code),
            "seconds_ago": seconds(now.saturating_sub(at))
        })),
        "held": rows.iter().map(|h| json!({
            // A whole market is not a contract, and naming it under one hands
            // a caller a security type where a root belongs. `stop_response`
            // already refuses this; so does the listing.
            "contract": match shape(&h.sub) {
                Some(Shape::Full(..)) | None => Value::default(),
                Some(Shape::Contract(..)) => Value::from(h.label.as_str()),
            },
            "sec_type": match shape(&h.sub) {
                Some(Shape::Full(sec, _)) => Value::from(sec.as_str().to_ascii_lowercase().as_str()),
                _ => Value::default(),
            },
            "kind": match shape(&h.sub) {
                Some(Shape::Contract(_, k)) => k.kind_str(),
                Some(Shape::Full(_, k)) => k.kind_str(),
                None => "unknown",
            },
            "on_feed": live.map(|live| live.contains(&h.sub)),
            "received": h.received,
            "held": h.held,
            "dropped": h.dropped,
            "age_ms": h.newest_ms.map(|s| now.saturating_sub(s)),
            "open_for_seconds": seconds(now.saturating_sub(h.opened_ms)),
            // A buffer the sweep has marked has no idle time to report and is
            // due for release now, which is what the caller acts on.
            "idle_seconds": h.read_ms.map(|r| seconds(now.saturating_sub(r))),
            "expires_in_seconds": h.read_ms.map_or(0.0, |r| {
                seconds((TTL.as_millis() as u64).saturating_sub(now.saturating_sub(r)))
            })
        })).collect::<Vec<_>>(),
        "on_feed_not_held": unheld
    })
}

/// The answer a prints read hands back.
///
/// Pulled out of the request arm so which value lands in which field is
/// reachable by a test. The arm needs a live client, and a `clipped` that
/// could only be checked inside an untestable branch is a completeness claim
/// nobody can prove.
fn prints_response(
    contract: &Contract,
    feed: String,
    count: usize,
    with_quotes_after: bool,
    p: &Prints,
    now: u64,
) -> Value {
    // One date while the prints agree; otherwise it travels on each.
    let prints_date = one_date(p.rows.iter().flat_map(|p| {
        std::iter::once(&p.trade)
            .chain(p.quote_before.iter())
            .chain(p.quotes_after.iter())
    }));
    json!({
        "contract": contract.to_string(),
        "feed": feed,
        "subscribed_now": p.opened.iter().map(|k| k.kind_str()).collect::<Vec<_>>(),
        "count": p.rows.len(),
        "held": p.held,
        "feed_interrupted": p.gap,
        // A hole anywhere in the history, prints evicted before the caller
        // asked for them, or events the SDK discarded: any of the three means
        // the answer is not the whole of what happened.
        "clipped": p.holed
            || (p.held < count && p.dropped > 0)
            || p.feed_dropped_since_last_read > 0,
        "covers_seconds": seconds(now.saturating_sub(p.covered_since_ms)),
        "new_since_last_read": p.new_since_last_read,
        "feed_dropped_since_last_read": p.feed_dropped_since_last_read,
        // The age of the newest print returned, never of the feed.
        "age_ms": p.newest_ms.map(|s| now.saturating_sub(s)),
        "date": prints_date,
        "prints": p.rows.iter().map(|p| {
            let mut out = json!({
                "date": prints_date.is_none().then(|| date_of(&p.trade)).flatten(),
                "trade": object(&p.trade),
                "quote_before": p.quote_before.as_ref().map(|q| {
                    aged_object_dated(q, now, prints_date.is_none())
                })
            });
            if with_quotes_after {
                if let Some(obj) = out.as_object_mut() {
                    obj.insert("quotes_after", Value::from(p.quotes_after.iter()
                        .map(|q| aged_object_dated(q, now, prints_date.is_none()))
                        .collect::<Vec<_>>()));
                }
            }
            out
        }).collect::<Vec<_>>()
    })
}

/// The answer a buffer read hands back.
///
/// Pulled out of the request arm so which value lands in which field is
/// reachable by a test. The arm around it needs a live client, and an
/// `age_ms` that could only be checked inside an untestable branch is an age
/// nobody can prove describes the rows rather than the feed.
fn read_response(
    contract: &Contract,
    kind: SubscriptionKind,
    feed: String,
    window: Option<u64>,
    r: &Reading,
    now: u64,
) -> Value {
    let newest = r.tail.last();
    // One date for everything this response renders while they agree.
    // Otherwise it travels on each row: restamping one is not an option, and
    // the oldest row reaches further back than the tail.
    let shared_date = one_date(r.tail.iter().chain(r.oldest.iter()).chain(r.ohlcvc.iter()));
    json!({
        "contract": contract.to_string(),
        "kind": kind.kind_str(),
        "feed": feed,
        "subscribed_now": r.first,
        // A named window is the interval the caller asked for. Only the
        // default window, which is "since you last looked", stretches back
        // over a row decoded before that read and delivered after it.
        "window_seconds": seconds(now.saturating_sub(window_start(window, r))),
        "window_from": if window.is_some() { "request" } else { "last_read" },
        "covers_seconds": seconds(now.saturating_sub(r.covered_since_ms)),
        "clipped": r.clipped,
        "dropped": r.dropped,
        "new_since_last_read": r.new_since_last_read,
        "feed_dropped_since_last_read": r.feed_dropped_since_last_read,
        // The age of the rows returned, never of the feed: a quiet contract
        // and a dead feed look identical from a feed age alone.
        "age_ms": r.newest_ms.map(|s| now.saturating_sub(s)),
        "rows_in_window": r.count,
        "vendor_ohlcvc": r.ohlcvc
            .as_ref()
            .map(|bar| aged_object_dated(bar, now, shared_date.is_none())),
        "date": shared_date.map(Value::from),
        "columns": newest.map(|d| {
            let mut c: Vec<&str> = fields(d).into_iter().map(|(k, _)| k).collect();
            if shared_date.is_none() {
                c.push("date");
            }
            c
        }),
        "tail": r.tail.iter().map(|d| {
            let mut v = row(d);
            if shared_date.is_none() {
                if let (Some(a), Some(day)) = (v.as_array_mut(), date_of(d)) {
                    a.push(json!(day));
                }
            }
            v
        }).collect::<Vec<_>>()
    })
}

/// Tool calls arrive one at a time (the JSON-RPC loop awaits each before
/// reading the next), so a registry mutation and the feed call that follows
/// it are never interleaved with another tool call. That ordering is what
/// lets a read hand back subscriptions to open or close outside the lock.
fn execute(client: &Client, name: &str, args: &Value) -> Result<Value, ToolError> {
    let reg = registry();
    let now = now_ms();
    let num_of = |k: &str, d: usize| count_arg(args, k, "a whole number of rows, zero or more", d);
    let window = arg(args, "seconds", "a number of seconds, zero or more", |v| {
        v.as_f64().filter(|s| *s >= 0.0)
    })?
    .map(|s| (s * 1_000.0) as u64);
    // Before anything else: what the sweep frees is closed here, whatever
    // the call goes on to do or refuse.
    close_expired(client, reg, reg.expire(now), now);

    if name == "live_list" {
        let rows = reg.list();
        // Cumulative and display-only, so it needs no session of its own.
        let feed_drops = client.stream().dropped_event_count();
        // A listing is diagnostic, so a feed that cannot say what it carries
        // leaves the on_feed fields null rather than failing the call.
        let live = on_feed(client).ok();
        return Ok(list_response(
            feed_state(client),
            feed_drops,
            reg.last_rejection(),
            &rows,
            live.as_deref(),
            now,
        ));
    }

    if name == "live_market" {
        let sec = parse_sec_of(args, sec_types_for("live_market"))?;
        if sec != SecType::Option {
            if let Some(named) = ["expiration", "right", "strike_min", "strike_max"]
                .iter()
                .find(|k| args.get(*k).is_some_and(|v: &Value| !v.is_null()))
            {
                return Err(ToolError::InvalidParams(format!(
                    "{named} describes an option, and no {} has one; a selection on it would \
                     match nothing",
                    sec.as_str().to_ascii_lowercase()
                )));
            }
        }
        let hist = client.market_data();
        pro_required(
            sec,
            match sec {
                SecType::Option => hist.options_tier(),
                _ => hist.stock_tier(),
            },
        )?;
        let limit = count_arg(args, "limit", "a whole number of rows, one or more", 20)?;
        let q = parse_market_query(args, tail_rows(limit))?;
        // Sampled from the session this call guarantees, never before it:
        // a restart resets the SDK's counter, and a count taken ahead of one
        // belongs to the session that just died.
        // Re-read the clock: starting a session can take seconds, and a
        // window's end must be when the rows were read, not when the call
        // arrived, or it names a shorter window than the rows it returns.
        let feed_drops = ensure_streaming(client, reg)?;
        let now = now_ms();
        let mut m = reg.market(sec, q.clone(), feed_drops, now)?;
        let sub = sec.full_trades();
        let landed = if m.first {
            open_on_feed(client, reg, &[sub])
        } else {
            reconcile(client, reg, &[sub], now)
        };
        if let Err(e) = landed {
            // The caller receives nothing, so the selection this read took
            // goes back and its rows reach whoever asks next.
            reg.restore_market(sec, m.settle.outgoing.take());
            return Err(e);
        }
        reg.commit_market(sec, &m.settle, now);
        return Ok(market_response(sec, feed_state(client), &q, &m, now));
    }

    if name == "live_stop" && args.get("root").is_none() {
        let sec = whole_market_target(name, args)?;
        if !reg.market_held(sec) {
            return Err(ToolError::InvalidParams(format!(
                "no whole-market buffer is held on {}; live_list shows what is",
                sec.as_str().to_ascii_lowercase()
            )));
        }
        let (done, failures) = close_on_feed(client, reg, vec![sec.full_trades()]);
        return Ok(stop_response(Closing::Market(sec), (done, failures)));
    }

    // live_prints pairs a trade with the quote before it, and an index has
    // no quote stream, so it says which types it serves at the first refusal
    // rather than after a second call.
    if name == "live_prints" {
        parse_sec_of(args, sec_types_for(name))?;
    }
    let (sec, contract) = parse_contract(args)?;
    match name {
        "live_read" => {
            let kind = arg(args, "kind", "a subscription name", |v| {
                v.as_str().map(str::to_string)
            })?;
            let kind = resolve_kind(sec, kind.as_deref())?;
            let feed_drops = ensure_streaming(client, reg)?;
            let now = now_ms();
            let r = reg.read(
                &contract,
                kind,
                window,
                tail_rows(num_of("tail", TAIL)?),
                feed_drops,
                now,
            )?;
            let sub = subscription(kind, &contract);
            if r.first {
                open_on_feed(client, reg, &[sub])?;
            } else {
                reconcile(client, reg, &[sub], now)?;
            }
            // The answer is going to reach the caller, so the cursors move.
            // A call that failed above left all of it for the next read.
            reg.commit_read(&contract, kind, r.settle.clone(), now);
            Ok(read_response(
                &contract,
                kind,
                feed_state(client),
                window,
                &r,
                now,
            ))
        }
        "live_prints" => {
            // A print needs both legs; an index offers neither quote nor
            // print, and the refusal names what it does offer.
            resolve_kind(sec, Some("quote"))?;
            let with_quotes_after =
                arg(args, "quotes_after", "true or false", Value::as_bool)?.unwrap_or(false);
            let count = num_of("count", 20)?;
            let feed_drops = ensure_streaming(client, reg)?;
            let now = now_ms();
            let p = reg.prints(&contract, count, feed_drops, now)?;
            let (opened, kept): (Subs, Subs) = [SubscriptionKind::Trade, SubscriptionKind::Quote]
                .into_iter()
                .map(|k| subscription(k, &contract))
                .partition(|s| p.opened.iter().any(|k| subscription(*k, &contract) == *s));
            open_on_feed(client, reg, &opened)?;
            reconcile(client, reg, &kept, now)?;
            // Only an answer the caller receives consumes the cursor.
            reg.commit_prints_read(&contract, p.received, p.feed_drops_seen, p.gaps_seen);
            Ok(prints_response(
                &contract,
                feed_state(client),
                count,
                with_quotes_after,
                &p,
                now,
            ))
        }
        "live_stop" => {
            let held = reg.held_for(&contract);
            if held.is_empty() {
                return Err(ToolError::InvalidParams(format!(
                    "{contract} is not held; live_list shows what is"
                )));
            }
            let (done, failures) = close_on_feed(client, reg, held);
            Ok(stop_response(
                Closing::Contract(&contract),
                (done, failures),
            ))
        }
        other => Err(ToolError::InvalidParams(format!("unknown tool: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn stock(sym: &str) -> Contract {
        Contract::stock(sym)
    }

    fn option(strike: &str, right: &str) -> Contract {
        Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike,
                right,
            },
        )
        .expect("a valid contract")
    }

    fn trade_sized(
        c: &Contract,
        price: f64,
        size: i32,
        condition: i32,
        received_at_ns: u64,
    ) -> StreamData {
        StreamData::Trade {
            contract: Arc::new(c.clone()),
            ms_of_day: 34_200_000,
            sequence: 1,
            condition,
            size,
            exchange: 0,
            price,
            date: 20260915,
            received_at_ns,
        }
    }

    fn trade_with(c: &Contract, price: f64, condition: i32, received_at_ns: u64) -> StreamData {
        // Size differs from price so a swap between the two is visible. With
        // both rendering as one, `fields` could exchange them unseen.
        trade_sized(c, price, 7, condition, received_at_ns)
    }

    fn trade_on(c: &Contract, price: f64, date: i32, received_at_ns: u64) -> StreamData {
        match trade(c, price, received_at_ns) {
            StreamData::Trade {
                contract,
                ms_of_day,
                sequence,
                condition,
                size,
                exchange,
                price,
                received_at_ns,
                ..
            } => StreamData::Trade {
                contract,
                ms_of_day,
                sequence,
                condition,
                size,
                exchange,
                price,
                date,
                received_at_ns,
            },
            other => other,
        }
    }

    fn trade(c: &Contract, price: f64, received_at_ns: u64) -> StreamData {
        trade_with(c, price, 0, received_at_ns)
    }

    fn quote(c: &Contract, bid: f64, ask: f64) -> StreamData {
        StreamData::Quote {
            contract: Arc::new(c.clone()),
            ms_of_day: 1,
            bid_size: 1,
            bid_exchange: 0,
            bid,
            bid_condition: 0,
            ask_size: 1,
            ask_exchange: 0,
            ask,
            ask_condition: 0,
            date: 20260915,
            received_at_ns: 0,
        }
    }

    fn bar(c: &Contract, close: f64, received_at_ns: u64) -> StreamData {
        StreamData::Ohlcvc {
            contract: Arc::new(c.clone()),
            ms_of_day: 34_200_000,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close,
            volume: 10,
            count: 3,
            date: 20260915,
            received_at_ns,
        }
    }

    fn price(d: &StreamData) -> f64 {
        match d {
            StreamData::Trade { price, .. } => *price,
            other => panic!("not a trade: {other:?}"),
        }
    }

    fn clauses(v: Value) -> Vec<Clause> {
        parse_clauses(&v, &FIELDS).expect("a valid predicate")
    }

    fn query(limit: usize) -> MarketQuery {
        MarketQuery {
            root: None,
            expiration: None,
            is_call: None,
            strike_min: None,
            strike_max: None,
            clauses: Vec::new(),
            rank_by: None,
            ascending: false,
            limit,
        }
    }

    /// A read with no predicate, unwrapped: the conflict a read can refuse
    /// has its own test.
    fn read(
        reg: &Registry,
        c: &Contract,
        kind: SubscriptionKind,
        window: Option<u64>,
        tail: usize,
        now: u64,
    ) -> (Reading, Subs) {
        // A call that is answered settles its cursors, as the tool does.
        let expired = reg.expire(now);
        let reading = read_now(reg, c, kind, window, tail, 0, now).expect("nothing to refuse");
        reg.commit_read(c, kind, reading.settle.clone(), now);
        (reading, expired)
    }

    /// Read the way `execute` does: observe, then settle. A bare
    /// `Registry::read` leaves its cursors where they were, because the
    /// answer has not reached anyone yet.
    fn read_now(
        reg: &Registry,
        c: &Contract,
        kind: SubscriptionKind,
        window: Option<u64>,
        tail: usize,
        feed_drops: u64,
        now: u64,
    ) -> Result<Reading, ToolError> {
        let r = reg.read(c, kind, window, tail, feed_drops, now)?;
        reg.commit_read(c, kind, r.settle.clone(), now);
        Ok(r)
    }

    /// Read a market the way `execute` does: observe, then settle.
    fn market_now(
        reg: &Registry,
        sec: SecType,
        q: MarketQuery,
        feed_drops: u64,
        now: u64,
    ) -> Result<MarketReading, ToolError> {
        let m = reg.market(sec, q, feed_drops, now)?;
        reg.commit_market(sec, &m.settle, now);
        Ok(m)
    }

    fn prints(reg: &Registry, c: &Contract, count: usize, now: u64) -> (Prints, Subs) {
        // A call that is answered commits its cursor, as the tool does.
        let expired = reg.expire(now);
        let p = reg.prints(c, count, 0, now).expect("nothing to refuse");
        reg.commit_prints_read(c, p.received, p.feed_drops_seen, p.gaps_seen);
        (p, expired)
    }

    /// The refusal a call produced, in words.
    fn refused<T>(r: Result<T, ToolError>) -> String {
        match r {
            Err(e) => format!("{e:?}"),
            Ok(_) => panic!("not refused"),
        }
    }

    const MS: u64 = 1_000_000;
    const PAST_TTL: u64 = TTL.as_millis() as u64 + 1;

    #[test]
    fn the_first_read_opens_the_buffer_and_the_next_does_not() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (a, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1_000);
        let (b, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1_001);
        assert!(a.first, "the first read subscribes");
        assert!(!b.first, "the second must not re-subscribe");
        assert_eq!(
            reg.subscriptions(),
            (vec![(SubscriptionKind::Trade, c)], vec![]),
            "one buffer, one subscription"
        );
    }

    #[test]
    fn a_read_defaults_to_since_the_last_read_and_counts_what_is_new() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1_000);
        reg.ingest(trade(&c, 1_500.0, 1_500 * MS));
        reg.ingest(trade(&c, 2_500.0, 2_500 * MS));

        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 3_000);
        assert_eq!(
            r.floor, 1_000,
            "the default window starts at the previous read"
        );
        assert_eq!(
            r.tail.iter().map(price).collect::<Vec<_>>(),
            vec![1_500.0, 2_500.0],
            "rows since the last read, oldest first"
        );
        assert_eq!(r.new_since_last_read, 2);
        assert!(!r.clipped, "the buffer was open for the whole window");

        // A fixed lookback does not change what "new" means.
        reg.ingest(trade(&c, 3_500.0, 3_500 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(10_000), TAIL, 4_000);
        assert_eq!(r.count, 3, "the fixed window sees everything");
        assert_eq!(
            r.new_since_last_read, 1,
            "but only one row arrived since the read at 3000"
        );
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(600), 1, 4_000);
        assert_eq!(
            r.count, 1,
            "a 600 ms window at 4000 holds only the row at 3500"
        );
        assert_eq!(
            r.new_since_last_read, 0,
            "nothing arrived since the read a moment ago"
        );
    }

    #[test]
    fn a_row_is_new_exactly_once_whatever_its_stamp_says() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);

        // Stamped the same millisecond as the read that takes it: a clock
        // cursor with `>=` would serve it again on the next read.
        reg.ingest(trade(&c, 1.0, 100 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 100);
        assert_eq!((r.new_since_last_read, r.tail.len()), (1, 1));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 101);
        assert_eq!(
            (r.new_since_last_read, r.tail.len()),
            (0, 0),
            "not counted twice"
        );

        // Decoded before a read, dispatched after it: the stamp is older
        // than the cursor, and a clock cursor would never serve it.
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 200);
        assert_eq!(r.new_since_last_read, 0);
        reg.ingest(trade(&c, 2.0, 199 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 201);
        assert_eq!(
            (
                r.new_since_last_read,
                r.tail.iter().map(price).collect::<Vec<_>>()
            ),
            (1, vec![2.0]),
            "not lost"
        );

        // The same cursor on prints and on the market.
        prints(&reg, &c, 10, 300);
        reg.ingest(trade(&c, 3.0, 300 * MS));
        assert_eq!(prints(&reg, &c, 10, 300).0.new_since_last_read, 1);
        assert_eq!(prints(&reg, &c, 10, 301).0.new_since_last_read, 0);
        let call = option("550", "C");
        market_now(&reg, SecType::Option, query(10), 0, 400).expect("nothing to refuse");
        reg.ingest(trade(&call, 1.0, 400 * MS));
        let m = market_now(&reg, SecType::Option, query(10), 0, 400).expect("nothing to refuse");
        assert_eq!((m.new_since_last_read, m.examined), (1, 1));
        let m = market_now(&reg, SecType::Option, query(10), 0, 401).expect("nothing to refuse");
        assert_eq!((m.new_since_last_read, m.examined), (0, 0));
    }

    #[test]
    fn a_window_whose_oldest_held_row_sits_on_the_floor_is_not_proven_complete() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        // One more row than the ring holds, all stamped the same second.
        for i in 0..=RING {
            reg.ingest(trade(&c, i as f64, 1_000 * MS));
        }
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(1_000), TAIL, 2_000);
        assert_eq!((r.dropped, r.covered_since_ms), (1, 1_000));
        assert!(
            r.clipped,
            "the discarded row may share the oldest held row's stamp, so the window is not whole"
        );
        // Without a window, rows that arrived since the last read and fell
        // off before being served are the loss.
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 2_001);
        assert!(!r.clipped, "nothing new, nothing lost");
        for i in 0..=RING {
            reg.ingest(trade(&c, i as f64, 3_000 * MS));
        }
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 3_001);
        assert_eq!(r.new_since_last_read, RING as u64 + 1);
        assert_eq!(r.count, RING as u64, "the ring holds one fewer");
        assert!(r.clipped, "one new row was gone before this read");
    }

    #[test]
    fn a_quote_leg_that_went_away_closes_the_prints_it_left_open() {
        let reg = Registry::default();
        let c = stock("AAPL");
        prints(&reg, &c, 10, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.05, 0));

        // The quote leg expires while trades keep arriving and the trade
        // leg keeps being read.
        read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            TTL.as_millis() as u64,
        );
        let (_, expired) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert_eq!(expired, vec![c.quote()]);
        reg.ingest(trade(&c, 1.06, PAST_TTL * MS));

        // It comes back, and quotes flow again.
        let (p, _) = prints(&reg, &c, 10, PAST_TTL + 1);
        assert_eq!(p.opened, vec![SubscriptionKind::Quote]);
        reg.ingest(quote(&c, 2.00, 2.10));
        reg.ingest(quote(&c, 2.01, 2.11));
        reg.ingest(quote(&c, 2.02, 2.12));
        reg.ingest(trade(&c, 2.05, (PAST_TTL + 2) * MS));
        reg.ingest(quote(&c, 2.03, 2.13));
        reg.ingest(quote(&c, 2.04, 2.14));

        let (p, _) = prints(&reg, &c, 10, PAST_TTL + 3);
        let bid = |q: &StreamData| field_of(q, "bid");
        assert_eq!(p.rows.len(), 3);
        assert_eq!(
            p.rows[0].quote_before.as_ref().and_then(bid),
            Some(1.00),
            "the print before the gap keeps the quote it had"
        );
        assert!(
            p.rows[0].quotes_after.is_empty(),
            "but takes none from after the gap"
        );
        assert!(
            p.rows[1].quote_before.is_none(),
            "a print during the gap has no quote before it: the last one seen was stale"
        );
        assert!(p.rows[1].quotes_after.is_empty());
        assert_eq!(
            p.rows[2].quote_before.as_ref().and_then(bid),
            Some(2.02),
            "a print after the leg came back correlates as usual"
        );
        assert_eq!(
            p.rows[2].quotes_after.iter().map(bid).collect::<Vec<_>>(),
            vec![Some(2.03), Some(2.04)]
        );
    }

    #[test]
    fn a_leg_never_attempted_is_rolled_back_with_the_one_that_failed() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let legs = [c.trade(), c.quote()];
        prints(&reg, &c, 10, 0);

        // The first leg fails synchronously: the second was never sent.
        roll_back(&reg, &legs, 0);
        let (p, _) = prints(&reg, &c, 10, 1);
        assert_eq!(
            p.opened,
            vec![SubscriptionKind::Trade, SubscriptionKind::Quote],
            "both legs are opened again, so both are subscribed"
        );

        // The second leg fails: the first is on the feed and stays.
        roll_back(&reg, &legs, 1);
        let (p, _) = prints(&reg, &c, 10, 2);
        assert_eq!(p.opened, vec![SubscriptionKind::Quote]);
    }

    #[test]
    fn subscriptions_the_restore_could_not_reopen_are_released() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        market_now(&reg, SecType::Option, query(1), 0, 0).expect("nothing to refuse");

        let released = release_unrestored(
            &reg,
            &[
                (SubscriptionKind::Trade, c.clone()),
                (
                    SubscriptionKind::Trade,
                    Contract::full_type_marker(SecType::Option),
                ),
            ],
        );
        assert_eq!(released, vec!["trade AAPL STOCK", "full_trades OPTION"]);
        assert_eq!(
            reg.held_for(&c),
            vec![c.quote()],
            "the leg that restored is kept; the one that did not is gone"
        );
        assert!(
            !reg.market_held(SecType::Option),
            "a full-stream failure arrives as the marker contract and releases the market buffer"
        );
    }

    #[test]
    fn a_refused_call_still_hands_back_what_the_sweep_freed() {
        // The sweep is its own step, run and closed before any call that
        // the registry may refuse, so a refusal cannot strand a freed
        // subscription.
        let reg = Registry::default();
        let aapl = stock("AAPL");
        read(&reg, &aapl, SubscriptionKind::Trade, None, TAIL, 0);
        market_now(&reg, SecType::Option, query(1), 0, PAST_TTL - 1).expect("nothing to refuse");

        let expired = reg.expire(PAST_TTL);
        assert_eq!(
            expired,
            vec![aapl.trade()],
            "the idle stock buffer is freed"
        );
        let why = refused(read_now(
            &reg,
            &option("550", "C"),
            SubscriptionKind::Quote,
            None,
            TAIL,
            0,
            PAST_TTL,
        ));
        assert!(
            why.contains("live_market"),
            "and the call is refused: {why}"
        );
        assert_eq!(
            reg.subscriptions().0,
            vec![],
            "the freed subscription is nowhere in the registry: only the caller of expire holds it"
        );
    }

    #[test]
    fn a_buffer_the_feed_would_not_release_still_bars_the_market() {
        let reg = Registry::default();
        let c = option("550", "C");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        assert_eq!(reg.expire(PAST_TTL), vec![c.trade()]);
        // The close failed, so the buffer is put back before any conflict is
        // judged; the feed still delivers that contract.
        reg.reinstate(&c.trade(), PAST_TTL);
        let why = refused(market_now(&reg, SecType::Option, query(1), 0, PAST_TTL));
        assert!(why.contains("1 OPTION contract"), "{why}");
    }

    #[test]
    fn a_feed_gap_closes_every_open_correlation() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let call = option("550", "C");
        prints(&reg, &c, 10, 0);
        market_now(&reg, SecType::Option, query(10), 0, 0).expect("nothing to refuse");
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.05, 0));
        reg.ingest(quote(&call, 3.00, 3.10));

        // The connection drops; whatever printed meanwhile was never seen.
        reg.gap(1);
        reg.ingest(quote(&c, 2.00, 2.10));
        reg.ingest(quote(&c, 2.01, 2.11));
        reg.ingest(trade(&c, 2.05, 1));
        reg.ingest(trade(&call, 3.05, 1));

        let (p, _) = prints(&reg, &c, 10, 2);
        assert!(
            p.rows[0].quotes_after.is_empty(),
            "the print from before the gap takes no quote from after it"
        );
        assert_eq!(
            p.rows[1]
                .quote_before
                .as_ref()
                .and_then(|q| field_of(q, "bid")),
            Some(2.01),
            "a print after the gap correlates with what came after it"
        );
        let m = market_now(&reg, SecType::Option, query(10), 0, 3).expect("nothing to refuse");
        assert!(
            m.rows[0].quote_before.is_none(),
            "the market's waiting quote is from before the gap and is not this trade's"
        );
    }

    #[test]
    fn events_the_feed_discarded_count_against_every_window() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let call = option("550", "C");
        // The SDK's counter is cumulative; a first read starts from it.
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 5, 0)
            .expect("nothing to refuse");
        assert_eq!((r.feed_dropped_since_last_read, r.clipped), (0, false));
        reg.ingest(trade(&c, 1.0, MS));
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 6, 1)
            .expect("nothing to refuse");
        assert_eq!(
            r.feed_dropped_since_last_read, 1,
            "one event lost since the last read"
        );
        assert!(
            r.clipped,
            "it may have been this buffer's, so the window is not whole"
        );
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 6, 2)
            .expect("nothing to refuse");
        assert_eq!((r.feed_dropped_since_last_read, r.clipped), (0, false));

        // The cursor moves where the prints cursor moves: on the commit that
        // follows a call the caller actually received.
        let first = reg.prints(&c, 10, 6, 3).expect("nothing to refuse");
        reg.commit_prints_read(&c, first.received, first.feed_drops_seen, first.gaps_seen);
        let p = reg.prints(&c, 10, 9, 4).expect("nothing to refuse");
        assert_eq!(p.feed_dropped_since_last_read, 3);
        // Uncommitted, so the same three are still owed.
        let again = reg.prints(&c, 10, 9, 5).expect("nothing to refuse");
        assert_eq!(
            again.feed_dropped_since_last_read, 3,
            "a call the caller never received does not spend the count"
        );
        reg.commit_prints_read(&c, again.received, again.feed_drops_seen, again.gaps_seen);
        let settled = reg.prints(&c, 10, 9, 6).expect("nothing to refuse");
        assert_eq!(settled.feed_dropped_since_last_read, 0);
        market_now(&reg, SecType::Option, query(1), 9, 5).expect("nothing to refuse");
        reg.ingest(trade(&call, 1.0, 5 * MS));
        let m = market_now(&reg, SecType::Option, query(1), 10, 6).expect("nothing to refuse");
        assert_eq!(m.feed_dropped_since_last_read, 1);
    }

    #[test]
    fn a_feed_gap_is_reported_on_a_market_selection() {
        // The selection has no ring to fall short, so without this the only
        // loss signal is the discard count, and rows that never reached the
        // SDK are not in it.
        let reg = Registry::default();
        let call = option("550000", "C");
        market_now(&reg, SecType::Option, query(5), 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&call, 1.0, 0));
        reg.gap(1);
        let m = market_now(&reg, SecType::Option, query(5), 0, 1).expect("nothing to refuse");
        assert_eq!(
            (m.feed_dropped_since_last_read, m.gap),
            (0, true),
            "the feed discarded nothing; the prints simply never came"
        );
        let after = market_now(&reg, SecType::Option, query(5), 0, 2).expect("nothing to refuse");
        assert!(!after.gap, "disclosed once, not for ever");
    }

    #[test]
    fn a_second_interruption_survives_the_commit_that_settles_the_first() {
        // The answer disclosed one interruption; another arrived while it
        // was in flight. Settling the first must not discharge the second.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 0));
        reg.gap(1);
        let p = reg.prints(&c, 10, 0, 1).expect("nothing to refuse");
        assert!(p.gap, "the first interruption is disclosed");
        reg.gap(1);
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        let next = reg.prints(&c, 10, 0, 2).expect("nothing to refuse");
        assert!(next.gap, "the second was never shown, so it is still owed");
        reg.commit_prints_read(&c, next.received, next.feed_drops_seen, next.gaps_seen);
        let settled = reg.prints(&c, 10, 0, 3).expect("nothing to refuse");
        assert!(!settled.gap, "both disclosed and both settled");
    }

    #[test]
    fn reading_prints_keeps_a_buffer_alive_without_moving_its_window() {
        // live_prints needs the trade and quote buffers to survive, but it is
        // not a read of them: moving the window floor would shorten a later
        // read's window without moving the cursor that picks its rows, and
        // the read would report a window shorter than what it returned.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, MS));
        reg.prints(&c, 10, 0, 2 * MS).expect("nothing to refuse");
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3 * MS)
            .expect("nothing to refuse");
        assert_eq!(r.new_since_last_read, 1, "the trade is new to this read");
        assert_eq!(
            r.floor, 0,
            "the window still starts at the last read of this buffer, not at the prints call"
        );
    }

    #[test]
    fn rows_from_two_sessions_each_carry_their_own_date() {
        // One date for the collection is true and cheap while the rows agree.
        // Naming one of two would restamp the other.
        let a = trade_on(&stock("AAPL"), 1.0, 20260915, 0);
        let b = trade_on(&stock("AAPL"), 2.0, 20260916, MS);
        assert_eq!(one_date([&a, &b].into_iter()), None, "they disagree");
        assert_eq!(
            one_date([&a].into_iter()),
            Some(20260915),
            "one session, one date"
        );
        assert_eq!(one_date(std::iter::empty()), None, "nothing to name");

        // And what the name claims: with no date for the collection, each row
        // carries its own and the column header says so. Asserting only the
        // aggregate leaves the carriage itself unproven.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade_on(&c, 1.0, 20260915, 2_000 * MS));
        reg.ingest(trade_on(&c, 2.0, 20260916, 3_000 * MS));
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 4_000)
            .expect("nothing to refuse");
        let v = read_response(
            &c,
            SubscriptionKind::Trade,
            "Connected".into(),
            None,
            &r,
            4_000,
        );
        assert!(v["date"].is_null(), "the rows do not share one");
        let cols: Vec<&str> = v["columns"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            cols.last(),
            Some(&"date"),
            "so the header names a date column"
        );
        let rows = v["tail"].as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), 2, "both rows come back");
        let dates: Vec<i64> = rows
            .iter()
            .filter_map(|row| {
                row.as_array()
                    .and_then(|a| a.last())
                    .and_then(|d| d.as_i64())
            })
            .collect();
        assert_eq!(dates, vec![20260915, 20260916], "each carrying its own");
    }

    #[test]
    fn a_read_that_is_never_answered_leaves_everything_for_the_next_one() {
        // The feed steps run after the registry has produced the answer. If
        // one of them fails the caller gets an error and no rows, so nothing
        // the read observed may have been spent.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, MS));
        reg.gap(1);

        // An uncommitted read: the answer never reached anyone.
        let lost = reg
            .read(&c, SubscriptionKind::Trade, None, TAIL, 7, 2 * MS)
            .expect("nothing to refuse");
        assert_eq!(lost.new_since_last_read, 1);
        assert_eq!(lost.feed_dropped_since_last_read, 7);

        // Every one of them is still owed.
        let next = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 7, 3 * MS)
            .expect("nothing to refuse");
        assert_eq!(next.new_since_last_read, 1, "the row is still new");
        assert_eq!(next.feed_dropped_since_last_read, 7, "nor the discards");
        assert!(next.clipped, "nor the interruption");

        // Once answered, they are settled. A row first, since nothing else
        // shows the feed came back from the interruption above.
        reg.ingest(trade(&c, 2.0, 4 * MS));
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 7, 5 * MS)
            .expect("nothing to refuse");
        let after = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 7, 6 * MS)
            .expect("nothing to refuse");
        assert_eq!(
            (
                after.new_since_last_read,
                after.feed_dropped_since_last_read,
                after.clipped
            ),
            (0, 0, false),
            "settled: nothing is owed to the next read"
        );
    }

    #[test]
    fn a_buffer_the_sweep_has_marked_reports_no_idle_time() {
        // `reinstate` marks a buffer idle with nought so the next sweep hands
        // its subscription back. Nought is that mark, not a moment, and read
        // as one it reports decades of idleness for a buffer put back now.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        reg.reinstate(&subscription(SubscriptionKind::Trade, &c), 2_000);

        let held = reg.list();
        let h = held.first().expect("the buffer was put back");
        assert_eq!(
            h.read_ms, None,
            "a marked buffer has no read time to report"
        );

        // And one that has been read carries its own.
        let reg = Registry::default();
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 5_000)
            .expect("nothing to refuse");
        assert_eq!(
            reg.list().first().expect("held").read_ms,
            Some(5_000),
            "a buffer that was read reports when"
        );
    }

    #[test]
    fn closing_a_whole_market_refuses_a_contract_identifier() {
        // Without a root this closes every contract on the type. A leg says
        // the caller meant one, and closing all of them instead is not a
        // smaller mistake for being silent.
        for named in OPTION_LEG {
            let args = json!({"sec_type": "option", named: 100});
            let why = refused(whole_market_target("live_stop", &args));
            assert!(
                why.contains(named) && why.contains("give root"),
                "{named} is named, with what to do instead: {why}"
            );
        }
        assert_eq!(
            whole_market_target("live_stop", &json!({"sec_type": "option"})).ok(),
            Some(SecType::Option),
            "and the whole market closes when nothing identifies one contract"
        );
    }

    #[test]
    fn an_index_is_refused_before_it_is_told_to_drop_a_leg() {
        // Told to drop `strike` so that sec_type alone closes the whole index
        // market, a caller who does so is then told there is no such stream.
        let why = refused(whole_market_target(
            "live_stop",
            &json!({"sec_type": "index", "strike": 100}),
        ));
        assert!(
            why.contains("no whole-market stream to close"),
            "the refusal a caller can act on comes first: {why}"
        );
    }

    #[test]
    fn a_capped_tail_keeps_the_newest_rows() {
        // The tail is the newest rows served verbatim, and `rows_in_window`
        // says how many it was cut from. Keeping the oldest instead serves
        // stale prints as current, with a count beside them that reads as
        // though they were the newest of more.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        for (price, at) in [(100.0, 10), (95.0, 20), (105.0, 30), (101.0, 40)] {
            reg.ingest(trade(&c, price, at * MS));
        }
        let r =
            read_now(&reg, &c, SubscriptionKind::Trade, None, 2, 0, 50).expect("nothing to refuse");
        assert_eq!(r.count, 4, "the window held every row");
        let prices: Vec<f64> = r
            .tail
            .iter()
            .map(|d| match d {
                StreamData::Trade { price, .. } => *price,
                other => panic!("expected a trade, got {other:?}"),
            })
            .collect();
        assert_eq!(prices, vec![105.0, 101.0], "the newest two, oldest first");
    }

    #[test]
    fn a_default_window_reaches_a_late_row_the_tail_does_not_hold() {
        // A row decoded before the last read and dispatched after it is new
        // to this one, and the window has to reach back far enough to hold
        // it. Once more than `tail` rows arrive behind it, the only thing
        // still carrying its stamp is the oldest row in the window.
        let reg = Registry::default();
        let c = stock("AAPL");
        // Opened well before, so the late row is not one the previous
        // subscription queued, which ingest drops.
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 10_000)
            .expect("nothing to refuse");
        // Stamped before that read, delivered after it.
        reg.ingest(trade(&c, 1.0, 5_000 * MS));
        for i in 0..12 {
            reg.ingest(trade(&c, 2.0, (11_000 + i * 1_000) * MS));
        }
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 23_000)
            .expect("nothing to refuse");
        assert_eq!(
            r.count, 13,
            "every row since the last read is in the window"
        );
        assert_eq!(r.tail.len(), TAIL, "and the tail is capped below that");
        assert_eq!(
            seen_ms(r.oldest.as_ref().expect("the window has an oldest row")),
            Some(5_000),
            "the oldest row in the window is the late one"
        );
        assert_eq!(
            window_start(None, &r),
            5_000,
            "so the window stretches back over it rather than to the last read"
        );
    }

    #[test]
    fn a_read_answers_about_its_own_rows_and_not_about_the_feed() {
        // Which value lands in which field. Every one of these was assigned
        // inside a branch no test reached, so swapping two of them, or
        // hardcoding a disclosure, changed nothing any test could see.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        // More rows than the tail will carry, so a count that quietly became
        // the tail's length would be wrong; and an interruption, so a clipped
        // flag that quietly became false would be wrong too. A fixture where
        // they agree cannot tell either apart.
        reg.gap(2_000);
        for (price, at) in [(1.0, 4_000), (2.0, 5_000), (3.0, 6_000), (4.0, 7_000)] {
            reg.ingest(trade(&c, price, at * MS));
        }
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, 2, 0, 9_000)
            .expect("nothing to refuse");

        let v = read_response(
            &c,
            SubscriptionKind::Trade,
            "Connected".into(),
            None,
            &r,
            9_000,
        );
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();

        // The age is of the newest row returned, stamped 7_000, not of
        // anything the feed did. This is what this surface exists not to get
        // wrong.
        assert_eq!(
            get("age_ms").as_u64(),
            Some(2_000),
            "age of the newest row, 7_000"
        );
        assert_eq!(
            get("rows_in_window").as_u64(),
            Some(4),
            "the population the tail was cut from, not the tail"
        );
        assert_eq!(
            get("tail").as_array().map(|a| a.len()),
            Some(2),
            "and the tail is smaller"
        );
        assert_eq!(get("dropped").as_u64(), Some(0));
        assert_eq!(get("new_since_last_read").as_u64(), Some(4));
        assert_eq!(get("feed_dropped_since_last_read").as_u64(), Some(0));
        assert_eq!(
            get("clipped").as_bool(),
            Some(true),
            "the feed was interrupted"
        );
        assert_eq!(get("window_from").as_str(), Some("last_read"));
        assert_eq!(get("kind").as_str(), Some("trade"));
        assert_eq!(get("feed").as_str(), Some("Connected"));
        // The default window starts at the last read, 1_000, and stretches
        // further only for a row older than it. These rows are newer.
        assert_eq!(
            get("window_seconds").as_f64(),
            Some(8.0),
            "back to the last read"
        );
        assert_eq!(
            get("covers_seconds").as_f64(),
            Some(8.0),
            "back to when the buffer opened"
        );

        // A named window is labelled as the caller's and measured from the
        // floor that read carried, which is the interval they asked for.
        // One more row, and a feed that discarded three events, so the four
        // counts below are four different numbers: under the default window
        // a count of the window and a count since the last read are the same
        // rows, and a swap between them is invisible.
        reg.ingest(trade(&c, 5.0, 8_500 * MS));
        let n = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(6_000),
            TAIL,
            3,
            10_000,
        )
        .expect("nothing to refuse");
        let named = read_response(
            &c,
            SubscriptionKind::Trade,
            "Connected".into(),
            Some(6_000),
            &n,
            10_000,
        );
        assert_eq!(
            named.get("window_from").and_then(|v| v.as_str()),
            Some("request")
        );
        assert_eq!(
            named.get("window_seconds").and_then(|v| v.as_f64()),
            Some(6.0)
        );
        let named_get = |k: &str| named.get(k).cloned().unwrap_or_default();
        assert_eq!(
            named_get("rows_in_window").as_u64(),
            Some(5),
            "every row stamped at or after the floor"
        );
        assert_eq!(
            named_get("new_since_last_read").as_u64(),
            Some(1),
            "of which one arrived since the read at 9_000"
        );
        assert_eq!(
            named_get("dropped").as_u64(),
            Some(0),
            "the ring evicted nothing"
        );
        assert_eq!(
            named_get("feed_dropped_since_last_read").as_u64(),
            Some(3),
            "while the feed discarded three, which is a different loss"
        );
        // Coverage is how far back the rows held reach, and it is not the
        // window: this buffer has been open nine seconds, so a two-second
        // window covers less than the buffer does. Asserting the two equal
        // proves neither, and computing one from the other would then be a
        // buffer two seconds old claiming an hour of coverage.
        assert_eq!(
            named.get("covers_seconds").and_then(|v| v.as_f64()),
            Some(9.0),
            "back to when the buffer opened, whatever window was asked for"
        );
    }

    #[test]
    fn a_close_does_not_count_what_it_could_not_release() {
        // A subscription the feed would not let go is named, and it is not
        // one that closed. Counting it would report a leak as a clean close,
        // and the count is the number a caller decides on.
        let c = stock("AAPL");
        let clean = stop_response(Closing::Contract(&c), (3, Vec::new()));
        assert_eq!(clean["subscriptions_closed"].as_u64(), Some(3));
        assert_eq!(
            clean["failed_to_close"].as_array().map(|a| a.len()),
            Some(0)
        );
        assert_eq!(clean["contract"].as_str(), Some(c.to_string().as_str()));
        assert!(
            clean.get("sec_type").is_none(),
            "a contract is not a market"
        );

        let leaked = stop_response(
            Closing::Market(SecType::Option),
            (1, vec!["quote AAPL".into(), "trade AAPL".into()]),
        );
        assert_eq!(
            leaked["subscriptions_closed"].as_u64(),
            Some(1),
            "only the one that actually closed"
        );
        assert_eq!(
            leaked["failed_to_close"].as_array().map(|a| a.len()),
            Some(2),
            "and the two the feed still holds are named"
        );
        assert_eq!(leaked["sec_type"].as_str(), Some("option"));
        assert!(
            leaked.get("contract").is_none(),
            "a whole market has no contract"
        );
    }

    #[test]
    fn every_tool_a_description_sends_a_model_to_exists() {
        // These descriptions route a caller to a snapshot for the current
        // value, because holding a subscription to read one costs two calls
        // and leaves a buffer open. A name that no longer exists sends the
        // model nowhere and it comes back here, which is the failure the
        // routing was added to prevent.
        use thetadatadx::ENDPOINTS;
        let served: Vec<&str> = ENDPOINTS.iter().map(|e| e.name).chain(TOOL_NAMES).collect();
        let mut named = 0;
        for t in tool_definitions() {
            let tool = t["name"].as_str().unwrap_or_default().to_string();
            let text = t["description"].as_str().unwrap_or_default().to_string();
            for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if !word.contains('_') || word == tool {
                    continue;
                }
                // A word shaped like a tool name is one, or it is prose that
                // reads like one, which is just as misleading to a model.
                if word.starts_with("live_")
                    || word.starts_with("stock_")
                    || word.starts_with("option_")
                    || word.starts_with("index_")
                {
                    assert!(
                        served.contains(&word),
                        "{tool} sends a model to {word}, which this server does not serve"
                    );
                    named += 1;
                }
            }
        }
        assert!(named > 0, "the descriptions do name other tools");
    }

    #[test]
    fn a_market_answer_ages_its_rows_and_not_the_market() {
        // The age of what came back and the age of the market are two
        // numbers a line apart. A narrow selection holding an old print
        // while the tape stays busy is the case that separates them, and
        // both were assigned where no test reached.
        let reg = Registry::default();
        let mine = stock("AAPL");
        let other = stock("MSFT");
        // A limit of one, with two prints matching, so the rows handed back
        // and the prints that passed are different numbers.
        let mut q = query(1);
        q.root = Some("AAPL".into());
        market_now(&reg, SecType::Stock, q.clone(), 0, 1_000).expect("nothing to refuse");
        reg.ingest(quote(&mine, 10.0, 11.0));
        reg.ingest(trade(&mine, 1.0, 1_050 * MS));
        reg.ingest(quote(&mine, 12.0, 13.0));
        reg.ingest(trade(&mine, 1.0, 1_100 * MS));
        // The tape carries on, none of it this selection's.
        reg.ingest(trade(&other, 2.0, 5_000 * MS));
        let m = market_now(&reg, SecType::Stock, q.clone(), 0, 6_000).expect("nothing to refuse");

        let v = market_response(SecType::Stock, "Connected".into(), &q, &m, 6_000);
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        assert_eq!(
            get("age_ms").as_u64(),
            Some(4_900),
            "the print returned was stamped 1_100, so it is 4.9 s old"
        );
        assert_eq!(
            get("feed_age_ms").as_u64(),
            Some(1_000),
            "while the market itself last printed 1 s ago"
        );
        assert_eq!(get("sec_type").as_str(), Some("stock"));
        assert_eq!(
            get("returned").as_u64(),
            Some(1),
            "the one row the limit kept"
        );
        assert_eq!(
            get("examined").as_u64(),
            Some(3),
            "every print the selection saw"
        );
        assert_eq!(
            get("matched").as_u64(),
            Some(2),
            "of which two passed, more than came back"
        );
        assert_eq!(get("prints").as_array().map(|a| a.len()), Some(1));
        assert_eq!(
            v["prints"][0]["contract"].as_str(),
            Some(mine.to_string().as_str()),
            "and it is the contract the selection named"
        );
        assert_eq!(
            v["prints"][0]["trade"]["price"].as_f64(),
            Some(1.0),
            "the trade"
        );
        assert_eq!(
            v["prints"][0]["quote_before"]["bid"].as_f64(),
            Some(12.0),
            "paired with the quote that stood before it, not an earlier one"
        );
        // The most important disclosure on this arm, read through the
        // response rather than off the reading. Both ways round: a fixture
        // that never interrupts proves only that a negation is caught.
        assert_eq!(get("feed_interrupted").as_bool(), Some(false));
        reg.gap(6_100);
        let after =
            market_now(&reg, SecType::Stock, q.clone(), 0, 6_200).expect("nothing to refuse");
        let after_v = market_response(SecType::Stock, "Connected".into(), &q, &after, 6_200);
        assert_eq!(
            after_v["feed_interrupted"].as_bool(),
            Some(true),
            "and an interruption is reported"
        );
        assert_eq!(
            after_v["received"].as_u64(),
            Some(3),
            "every stock print the market took since it opened"
        );
        assert_eq!(
            after_v["new_since_last_read"].as_u64(),
            Some(0),
            "none of them since the read a moment ago, which is the other number"
        );
    }

    #[test]
    fn a_prints_read_answers_about_its_own_prints() {
        // Every field below was assigned where no test reached it. The
        // fixture is built so each assertion can fail: the counts differ from
        // each other, prints were lost, and the feed was interrupted, so a
        // swap or a hardcoded disclosure is visible.
        let reg = Registry::default();
        let c = stock("AAPL");
        // The first call opens both legs, which is the only read that has a
        // subscribed_now to report; a later one opens nothing.
        let opening = reg.prints(&c, 20, 0, 1_000).expect("nothing to refuse");
        let first = prints_response(&c, "Connected".into(), 20, false, &opening, 1_000);
        let opened: Vec<&str> = first["subscribed_now"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(opened, vec!["trade", "quote"], "a print needs both legs");
        reg.gap(2_000);
        for at in [4_000u64, 5_000, 6_000] {
            // A different quote before each trade, so a print carries both
            // and both which quote it took and the trade slot rendering a
            // quote instead are visible. Identical quotes say neither.
            reg.ingest(quote(&c, at as f64 / 1_000.0, 11.0));
            reg.ingest(trade(&c, 1.0, at * MS));
        }
        // Ask for fewer than are held, so `count` and `held` cannot agree.
        let p = reg.prints(&c, 2, 7, 9_000).expect("nothing to refuse");

        let v = prints_response(&c, "Connected".into(), 2, false, &p, 9_000);
        let get = |k: &str| v.get(k).cloned().unwrap_or_default();
        assert_eq!(get("feed").as_str(), Some("Connected"));
        assert_eq!(get("count").as_u64(), Some(2), "the prints returned");
        assert_eq!(
            get("held").as_u64(),
            Some(3),
            "the prints held, which is more"
        );
        assert_eq!(get("feed_interrupted").as_bool(), Some(true));
        assert_eq!(
            get("clipped").as_bool(),
            Some(true),
            "the feed discarded events"
        );
        assert_eq!(
            get("age_ms").as_u64(),
            Some(3_000),
            "age of the newest print, 6_000"
        );
        assert_eq!(
            get("feed_dropped_since_last_read").as_u64(),
            Some(7),
            "what the SDK discarded, not what arrived"
        );
        assert_eq!(get("prints").as_array().map(|a| a.len()), Some(2));
        // The trade slot carries the executed price, not the quote beside it.
        assert_eq!(
            v["prints"][0]["trade"]["price"].as_f64(),
            Some(1.0),
            "the trade"
        );
        assert_eq!(
            v["prints"][0]["quote_before"]["bid"].as_f64(),
            Some(5.0),
            "paired with the quote that stood before it, not an earlier one"
        );
        assert!(
            get("prints")[0].get("quotes_after").is_none(),
            "not asked for"
        );
        let with = prints_response(&c, "Connected".into(), 2, true, &p, 9_000);
        assert!(with["prints"][0].get("quotes_after").is_some(), "asked for");

        // Each reason to clip on its own, since a fixture that trips two of
        // them cannot tell which one the answer is reading. Here the history
        // has a hole and nothing else is wrong: no discards, nothing evicted,
        // and room for every print held.
        let reg = Registry::default();
        let c = stock("TSLA");
        reg.prints(&c, 20, 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        reg.prints(&c, 20, 0, 3_000).expect("nothing to refuse");
        reg.gap(4_000);
        reg.ingest(trade(&c, 2.0, 5_000 * MS));
        let holed = reg.prints(&c, 20, 0, 6_000).expect("nothing to refuse");
        assert_eq!(
            holed.feed_dropped_since_last_read, 0,
            "nothing was discarded"
        );
        assert_eq!(holed.dropped, 0, "and nothing was evicted");
        let v = prints_response(&c, "Connected".into(), 20, false, &holed, 6_000);
        assert_eq!(
            v["clipped"].as_bool(),
            Some(true),
            "a hole in the history clips on its own"
        );

        // A discard also clips. The schema allows count 0, which is a
        // caller asking whether anything was lost without wanting rows
        // back, and that is where the discard term of `clipped` is the only
        // one that can answer: with no row returned, none straddles the
        // hole, so `holed` is false and the history reads as whole.
        let reg = Registry::default();
        let c = stock("AMD");
        reg.prints(&c, 20, 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        let none_wanted = reg.prints(&c, 0, 5, 3_000).expect("nothing to refuse");
        assert!(!none_wanted.holed, "no returned row straddles the hole");
        assert_eq!(none_wanted.dropped, 0, "and nothing was evicted");
        assert_eq!(
            prints_response(&c, "Connected".into(), 0, false, &none_wanted, 3_000)["clipped"]
                .as_bool(),
            Some(true),
            "but five events were discarded, so this is not the whole of it"
        );

        let reg = Registry::default();
        let c = stock("NVDA");
        reg.prints(&c, 20, 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        let dropped = reg.prints(&c, 20, 5, 3_000).expect("nothing to refuse");
        assert_eq!(dropped.dropped, 0, "nothing was evicted");
        assert_eq!(
            dropped.feed_dropped_since_last_read, 5,
            "the SDK discarded five"
        );
        let v = prints_response(&c, "Connected".into(), 20, false, &dropped, 3_000);
        assert_eq!(
            v["clipped"].as_bool(),
            Some(true),
            "so the answer is not the whole of what happened"
        );
    }

    #[test]
    fn a_listing_says_which_buffers_are_still_on_the_feed() {
        // `on_feed` and `expires_in_seconds` are what a caller acts on, and
        // both were assigned where nothing could check them.
        let reg = Registry::default();
        let held = stock("AAPL");
        let gone = stock("MSFT");
        read_now(&reg, &held, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        read_now(&reg, &gone, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        // Read again later, so the moment it opened and the moment it was
        // read differ and the countdown can only come from one of them.
        read_now(&reg, &held, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        // Counters that differ from one another, so a swap between any two is
        // visible. All nought proves nothing.
        for _ in 0..(RING + 2) {
            reg.ingest(trade(&held, 1.0, 2_000 * MS));
        }
        // The feed kept one of them and dropped the other.
        let on_feed = vec![subscription(SubscriptionKind::Trade, &held)];
        let rows = reg.list();
        let v = list_response("Connected".into(), 9, None, &rows, Some(&on_feed), 5_000);

        let listed = v["held"].as_array().cloned().unwrap_or_default();
        assert_eq!(listed.len(), 2, "both buffers are listed");
        let flag = |label: &str| {
            listed
                .iter()
                .find(|h| h["contract"].as_str() == Some(label))
                .and_then(|h| h["on_feed"].as_bool())
        };
        assert_eq!(
            flag(&held.to_string()),
            Some(true),
            "this one the feed kept"
        );
        assert_eq!(flag(&gone.to_string()), Some(false), "this one it did not");
        assert_eq!(v["feed_dropped_events"].as_u64(), Some(9));
        let row = listed
            .iter()
            .find(|h| h["contract"].as_str() == Some(held.to_string().as_str()))
            .expect("the held buffer is listed");
        assert_eq!(
            row["received"].as_u64(),
            Some(RING as u64 + 2),
            "every row it took"
        );
        assert_eq!(
            row["held"].as_u64(),
            Some(RING as u64),
            "what the ring still holds"
        );
        assert_eq!(row["dropped"].as_u64(), Some(2), "and what it pushed out");
        assert!(row["sec_type"].is_null(), "a contract is not a market");
        assert_eq!(
            row["expires_in_seconds"].as_f64(),
            Some(seconds(TTL.as_millis() as u64 - 2_000)),
            "counted down from the last read at 3_000, not the open at 1_000"
        );
        assert_eq!(
            v["on_feed_not_held"].as_array().map(|a| a.len()),
            Some(0),
            "nothing on the feed that is not held"
        );
    }

    #[test]
    fn opening_a_second_kind_on_a_held_contract_is_a_first_read_of_that_kind() {
        // `first` decides whether the caller subscribes or reconciles. Read
        // as false for a kind never opened, the subscribe is never sent, the
        // leg is then absent from the feed's own list, and the answer says
        // the feed accepted and dropped it while releasing the buffer, so
        // every retry repeats.
        let reg = Registry::default();
        let c = stock("AAPL");
        let q = read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        assert!(q.first, "the quote leg was not held");
        let t = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 2_000)
            .expect("nothing to refuse");
        assert!(
            t.first,
            "the trade leg is new to this contract, though the contract is held"
        );
        let again = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        assert!(!again.first, "and the second read of it is not");
    }

    #[test]
    fn a_row_lands_in_the_buffer_for_its_own_subscription() {
        // Filed by whichever buffer comes first, a quote reaches the trade
        // buffer and is served under the trade's column header: a row of
        // bids and asks beneath `price, size, condition`.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(quote(&c, 10.0, 11.0));

        let t = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 2_000)
            .expect("nothing to refuse");
        assert_eq!(t.count, 0, "a quote is not a row on the trade buffer");
        let q = read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 2_000)
            .expect("nothing to refuse");
        assert_eq!(q.count, 1, "it belongs to the quote buffer");
        assert!(
            matches!(q.tail.first(), Some(StreamData::Quote { bid, .. }) if *bid == 10.0),
            "and it is the quote that was sent: {:?}",
            q.tail.first()
        );
    }

    #[test]
    fn prints_evicted_before_the_caller_asked_clip_the_answer() {
        // Asking for more prints than are held, when older ones were pushed
        // out, is an answer that does not reach as far back as it was asked
        // to. Both halves matter: evicted and short of the count.
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 20, 0, 1_000).expect("nothing to refuse");
        for i in 0..(PRINTS + 5) {
            reg.ingest(trade(&c, 1.0, (2_000 + i as u64) * MS));
        }
        let p = reg
            .prints(&c, PRINTS + 5, 0, 9_000)
            .expect("nothing to refuse");
        assert_eq!(p.held, PRINTS, "the ring holds its budget and no more");
        assert!(p.dropped > 0, "and pushed the older ones out");
        assert!(!p.holed, "with no hole and nothing discarded");
        assert_eq!(
            prints_response(&c, "Connected".into(), PRINTS + 5, false, &p, 9_000)["clipped"]
                .as_bool(),
            Some(true),
            "so the answer does not reach as far back as it was asked to"
        );
        // Asking for exactly what is held is a complete answer, though rows
        // were evicted before it. That count is the only one separating
        // "fewer than asked for" from "no more than asked for".
        let exact = reg.prints(&c, PRINTS, 0, 9_100).expect("nothing to refuse");
        assert_eq!(exact.held, PRINTS, "every print held comes back");
        assert!(exact.dropped > 0, "rows were still evicted at some point");
        assert_eq!(
            prints_response(&c, "Connected".into(), PRINTS, false, &exact, 9_100)["clipped"]
                .as_bool(),
            Some(false),
            "but none of them is missing from this answer"
        );

        // And asking for more than a quiet contract has ever produced is a
        // complete answer too: fewer rows than asked for only clips when
        // rows were evicted to make it so.
        let quiet = stock("MSFT");
        reg.prints(&quiet, 20, 0, 1_000).expect("nothing to refuse");
        for at in [2_000u64, 3_000, 4_000] {
            reg.ingest(trade(&quiet, 1.0, at * MS));
        }
        let few = reg.prints(&quiet, 20, 0, 5_000).expect("nothing to refuse");
        assert_eq!(few.held, 3, "three prints, and twenty asked for");
        assert_eq!(few.dropped, 0, "with nothing evicted");
        assert_eq!(
            prints_response(&quiet, "Connected".into(), 20, false, &few, 5_000)["clipped"]
                .as_bool(),
            Some(false),
            "so the answer is everything there is"
        );
    }

    #[test]
    fn reading_prints_keeps_its_buffers_from_being_swept() {
        // A caller polling only `live_prints` touches the trade and quote
        // legs through it. Without that, both are swept at the TTL and every
        // call re-subscribes and discloses a gap it created itself.
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        // Read just short of the TTL. Without the touch the legs still carry
        // the moment they opened, and the sweep a millisecond later takes
        // them; with it they are a millisecond old and stay.
        reg.prints(&c, 10, 0, PAST_TTL - 1)
            .expect("nothing to refuse");
        let expired = reg.expire(PAST_TTL);
        assert!(
            expired.is_empty(),
            "reading the prints kept both legs alive: {expired:?}"
        );
    }

    #[test]
    fn a_contract_buffer_takes_rows_while_a_market_buffer_is_held() {
        // Open interest and market value do not overlap the whole-market
        // stream, so a contract may hold one beside a market buffer. Filing
        // the row only with the market leaves that buffer empty for ever on
        // a live subscription.
        let reg = Registry::default();
        let c = option("550", "C");
        market_now(&reg, SecType::Option, query(5), 0, 1_000).expect("nothing to refuse");
        read_now(
            &reg,
            &c,
            SubscriptionKind::OpenInterest,
            None,
            TAIL,
            0,
            1_000,
        )
        .expect("nothing to refuse");
        reg.ingest(StreamData::OpenInterest {
            contract: Arc::new(c.clone()),
            ms_of_day: 1,
            open_interest: 42,
            date: 20260918,
            received_at_ns: 2_000 * MS,
        });
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::OpenInterest,
            None,
            TAIL,
            0,
            3_000,
        )
        .expect("nothing to refuse");
        assert_eq!(r.count, 1, "the row reached the contract's own buffer");
    }

    #[test]
    fn an_answer_is_as_old_as_the_newest_row_it_covers() {
        // Aged by the newest row the buffer holds rather than the newest the
        // window covers, an answer that returns nothing reports the age of a
        // row it did not return, and a window ending before that row renders
        // nought, which reads as something having just arrived.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));

        // A window ending before the only row held.
        let empty = read_now(&reg, &c, SubscriptionKind::Trade, Some(0), TAIL, 0, 1_500)
            .expect("nothing to refuse");
        assert_eq!(empty.count, 0, "the window covers nothing");
        assert_eq!(
            empty.newest_ms, None,
            "so the answer has no age, rather than the age of a row it did not return"
        );
        assert_eq!(
            read_response(
                &c,
                SubscriptionKind::Trade,
                "Connected".into(),
                Some(0),
                &empty,
                1_500
            )["age_ms"]
                .as_u64(),
            None,
            "and it does not render as nought, which reads as just arrived"
        );

        // A clock that steps backwards: the newest row held is behind the
        // newest row this window covers.
        let reg = Registry::default();
        let c = stock("MSFT");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 3_000 * MS));
        reg.ingest(trade(&c, 2.0, 2_000 * MS));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            3_500,
        )
        .expect("nothing to refuse");
        assert_eq!(
            r.count, 1,
            "only the row stamped 3_000 is inside the window"
        );
        assert_eq!(
            r.newest_ms,
            Some(3_000),
            "and the age is of that row, not of the one behind it in the ring"
        );
    }

    #[test]
    fn a_selection_ages_itself_by_the_newest_row_it_returns() {
        // With one row returned the oldest and the newest are the same, so a
        // fold over them cannot say which end it took.
        let reg = Registry::default();
        let c = stock("AAPL");
        let mut q = query(5);
        q.root = Some("AAPL".into());
        market_now(&reg, SecType::Stock, q.clone(), 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        reg.ingest(trade(&c, 2.0, 5_000 * MS));
        let m = market_now(&reg, SecType::Stock, q.clone(), 0, 6_000).expect("nothing to refuse");
        assert_eq!(m.rows.len(), 2, "two prints, stamped apart");
        assert_eq!(
            market_response(SecType::Stock, "Connected".into(), &q, &m, 6_000)["age_ms"].as_u64(),
            Some(1_000),
            "the newest of them is 1 s old; the oldest is 4 s and is not the answer"
        );
    }

    #[test]
    fn every_comparison_a_selection_offers_means_what_it_says() {
        // Each operator on its own boundary, where it differs from its
        // neighbour. Tried away from the boundary, `>` and `>=` agree and
        // neither is proven.
        let row = |bid: f64, ask: f64| quote(&stock("AAPL"), bid, ask);
        let holds = |op: &str, value: f64, bid: f64, ask: f64| {
            let c = parse_clauses(
                &json!([{"field": "spread", "op": op, "value": value}]),
                &FIELDS,
            )
            .expect("a clause");
            let d = row(bid, ask);
            holds_all(&c, &|f| field_of(&d, f))
        };
        // Spread sits exactly on the value, which is the only input that
        // separates each pair. A quarter is exact in binary; a tenth is not,
        // and `1.10 - 1.0` lands just above it, which would make `>` hold.
        assert!(!holds(">", 0.25, 1.0, 1.25), "greater than excludes equal");
        assert!(holds(">=", 0.25, 1.0, 1.25), "at least includes it");
        assert!(!holds("<", 0.25, 1.0, 1.25), "less than excludes equal");
        assert!(holds("<=", 0.25, 1.0, 1.25), "at most includes it");
        assert!(holds("==", 0.25, 1.0, 1.25), "equal holds");
        assert!(!holds("!=", 0.25, 1.0, 1.25), "and unequal does not");
        // A range includes both of its ends.
        let inside = parse_clauses(
            &json!([{"field": "spread", "op": "inside", "value": [0.25, 0.5]}]),
            &FIELDS,
        )
        .expect("a clause");
        let lo = row(1.0, 1.25);
        let hi = row(1.0, 1.5);
        assert!(
            holds_all(&inside, &|f| field_of(&lo, f)),
            "the low end is in"
        );
        assert!(
            holds_all(&inside, &|f| field_of(&hi, f)),
            "and the high end"
        );
        // And the complement excludes both, which is where an endpoint that
        // quietly became exclusive would show.
        let outside = parse_clauses(
            &json!([{"field": "spread", "op": "outside", "value": [0.25, 0.5]}]),
            &FIELDS,
        )
        .expect("a clause");
        assert!(
            !holds_all(&outside, &|f| field_of(&lo, f)),
            "the low end is not outside"
        );
        assert!(
            !holds_all(&outside, &|f| field_of(&hi, f)),
            "nor is the high end"
        );
        assert!(
            holds_all(&outside, &|f| field_of(&row(1.0, 1.6), f)),
            "past the high end is"
        );
    }

    #[test]
    fn an_answer_that_returns_no_rows_reports_no_age() {
        // An age dates the rows handed back. Asking for none is a caller
        // wanting the counts and the disclosures without the rows, and an
        // age there would date a row they were never given.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));

        let none = read_now(&reg, &c, SubscriptionKind::Trade, None, 0, 0, 3_000)
            .expect("nothing to refuse");
        assert_eq!(none.count, 1, "the window covers the row");
        assert!(none.tail.is_empty(), "and hands none of it back");
        assert_eq!(none.newest_ms, None, "so there is no age to report");

        // With a tail the age is of the newest row in it.
        reg.ingest(trade(&c, 2.0, 4_000 * MS));
        let some = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 5_000)
            .expect("nothing to refuse");
        assert_eq!(some.newest_ms, Some(4_000), "the newest row handed back");

        // Prints answer the same way at the same boundary.
        let reg = Registry::default();
        let c = stock("MSFT");
        reg.prints(&c, 10, 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        let empty = reg.prints(&c, 0, 0, 3_000).expect("nothing to refuse");
        assert_eq!(empty.held, 1, "the print is held");
        assert!(empty.rows.is_empty(), "and none is returned");
        assert_eq!(empty.newest_ms, None, "so there is no age to report");
        let one = reg.prints(&c, 10, 0, 3_000).expect("nothing to refuse");
        assert_eq!(
            one.newest_ms,
            Some(2_000),
            "and with a print returned, its age"
        );
    }

    #[test]
    fn a_subscription_the_feed_no_longer_carries_is_the_one_reported() {
        // Inverted, this reports a subscription the feed kept and misses the
        // one it dropped, so a read answers that a live leg was dropped and
        // releases it. Every read after the first routes through here.
        let c = stock("AAPL");
        let trade = subscription(SubscriptionKind::Trade, &c);
        let quote = subscription(SubscriptionKind::Quote, &c);
        let both = [trade.clone(), quote.clone()];

        assert_eq!(
            dropped_of(&both, &[trade.clone(), quote.clone()]),
            None,
            "the feed carries both, so nothing is reported"
        );
        assert_eq!(
            dropped_of(&both, std::slice::from_ref(&trade)),
            Some(&quote),
            "the quote leg is the one it stopped carrying"
        );
        assert_eq!(
            dropped_of(&both, std::slice::from_ref(&quote)),
            Some(&trade),
            "and the trade leg when that is the one"
        );
        assert_eq!(
            dropped_of(&both, &[]),
            Some(&trade),
            "with neither carried, the first asked about"
        );
        assert_eq!(
            dropped_of(&[], &[]),
            None,
            "nothing asked, nothing reported"
        );
    }

    #[test]
    fn a_row_already_kept_holds_its_place_on_a_tie() {
        // Two prints of equal rank and room for one. The comment says the row
        // already kept stays, and nothing said which one a caller gets.
        let reg = Registry::default();
        let c = stock("AAPL");
        let mut q = query(1);
        q.root = Some("AAPL".into());
        q.rank_by = Some("size".into());
        market_now(&reg, SecType::Stock, q.clone(), 0, 1_000).expect("nothing to refuse");
        // Same size, so they tie; different prices, so they are told apart.
        reg.ingest(trade_sized(&c, 10.0, 5, 0, 2_000 * MS));
        reg.ingest(trade_sized(&c, 20.0, 5, 0, 3_000 * MS));
        let m = market_now(&reg, SecType::Stock, q, 0, 4_000).expect("nothing to refuse");
        assert_eq!(m.rows.len(), 1, "room for one");
        assert!(
            matches!(&m.rows[0].trade, StreamData::Trade { price, .. } if *price == 10.0),
            "the first to arrive holds its place: {:?}",
            m.rows[0].trade
        );
    }

    #[test]
    fn reopening_a_leg_for_a_read_discloses_what_it_missed() {
        // Reopening a leg is an interruption whichever tool does it: the feed
        // stopped carrying that contract in between, and what it sent then is
        // simply absent. None of this path was exercised through `read`.

        // A quote leg coming back seals the prints it left open: a print
        // cannot take a quote from an interval nobody was watching.
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 10, 0, 1_000).expect("nothing to refuse");
        // The quote leg goes while the trade leg stays, then a trade lands.
        // That print is open and was never sealed by the leg going, so only
        // the reopening can seal it.
        reg.forget(&subscription(SubscriptionKind::Quote, &c));
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        reg.ingest(quote(&c, 10.0, 11.0));
        let p = reg.prints(&c, 10, 0, 4_000).expect("nothing to refuse");
        assert_eq!(p.rows.len(), 1, "the print is still held");
        assert!(
            p.rows[0].quotes_after.is_empty(),
            "and takes no quote from across the interval it was not watching"
        );

        // A trade leg coming back while prints are held holes the history.
        let reg = Registry::default();
        let c = stock("MSFT");
        reg.prints(&c, 10, 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        reg.prints(&c, 10, 0, 2_500).expect("nothing to refuse");
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        let after = reg.prints(&c, 10, 0, 4_000).expect("nothing to refuse");
        assert!(
            after.holed,
            "the trades from while the leg was gone are missing, and it says so"
        );

        // A second view inherits the discard count the first was already
        // carrying, so it reports what the feed threw away since then and not
        // since this call.
        let reg = Registry::default();
        let c = stock("NVDA");
        read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        let second = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 9, 2_000)
            .expect("nothing to refuse");
        assert_eq!(
            second.feed_dropped_since_last_read, 9,
            "the nine the feed discarded since the first view opened, not nought"
        );
    }

    #[test]
    fn a_market_put_back_after_a_failed_release_is_not_an_unbroken_one() {
        // The sweep frees the market buffer, the release fails, and it goes
        // back. The feed carried every print on the type throughout, and the
        // buffer that went back held no selection to offer them to, so the
        // next read has not seen that interval and must say so.
        let reg = Registry::default();
        market_now(&reg, SecType::Stock, query(5), 0, 0).expect("nothing to refuse");
        let expired = reg.expire(PAST_TTL);
        assert_eq!(expired.len(), 1, "the idle market is freed");
        reg.reinstate(&SecType::Stock.full_trades(), PAST_TTL);

        let back = market_now(&reg, SecType::Stock, query(5), 0, PAST_TTL + 1_000)
            .expect("nothing to refuse");
        assert!(
            back.gap,
            "the interval it did not observe is disclosed, as it is for a contract"
        );
    }

    #[test]
    fn a_listing_names_a_market_as_a_market() {
        // A whole market has no contract, and naming it under one hands a
        // caller a security type where a root belongs. The close answer
        // already refuses this.
        let reg = Registry::default();
        market_now(&reg, SecType::Stock, query(5), 0, 1_000).expect("nothing to refuse");
        let v = list_response("Connected".into(), 0, None, &reg.list(), Some(&[]), 2_000);
        let held = v["held"].as_array().cloned().unwrap_or_default();
        let m = held.first().expect("the market is listed");
        assert!(m["contract"].is_null(), "a market is not a contract");
        assert_eq!(
            m["sec_type"].as_str(),
            Some("stock"),
            "it is named by the type it covers"
        );
        assert_eq!(
            m["kind"].as_str(),
            Some("full_trades"),
            "and the stream it is on"
        );
    }

    #[test]
    fn a_tool_offers_only_the_security_types_it_can_serve() {
        // A type offered in the schema and refused on every call is a call
        // a model will make and an answer it will never get.
        let types = |tool: &str| -> Vec<String> {
            tool_definitions()
                .into_iter()
                .find(|t| t["name"] == tool)
                .and_then(|t| {
                    t["inputSchema"]["properties"]["sec_type"]["enum"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                })
                .unwrap_or_default()
        };
        // A print is a trade with the quote that stood before it, and an
        // index has no quote stream at any tier.
        // Stated outright, not compared against the same list the code
        // reads: a test that asks whether a value equals itself passes
        // whatever the value becomes.
        assert_eq!(sec_types_for("live_prints"), ["option", "stock"]);
        assert_eq!(sec_types_for("live_market"), ["option", "stock"]);
        assert_eq!(sec_types_for("live_read"), ["option", "stock", "index"]);
        assert_eq!(sec_types_for("live_stop"), ["option", "stock", "index"]);
        assert_eq!(types("live_prints"), ["option", "stock"]);
        assert_eq!(types("live_market"), ["option", "stock"]);
        // An index price arrives on the trade subscription, so a read and a
        // stop both work.
        assert_eq!(types("live_read"), ["option", "stock", "index"]);
        assert_eq!(types("live_stop"), ["option", "stock", "index"]);
    }

    #[test]
    fn a_discard_clips_every_window_that_reaches_back_over_it() {
        // A discard is a loss like an interruption, and a named window
        // asking about the same interval must answer the same way twice.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        let first = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(60_000),
            TAIL,
            4,
            1_000,
        )
        .expect("nothing to refuse");
        assert!(first.clipped, "the discard is inside the window");
        let again = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(60_000),
            TAIL,
            4,
            2_000,
        )
        .expect("nothing to refuse");
        assert!(
            again.clipped,
            "the same window still reaches back over the same loss"
        );
        let moved_on = read_now(&reg, &c, SubscriptionKind::Trade, Some(1), TAIL, 4, 600_000)
            .expect("nothing to refuse");
        assert!(!moved_on.clipped, "the loss is behind this window");
    }

    #[test]
    fn a_window_entirely_after_an_interruption_is_whole() {
        // The interruption is disclosed once on the default window. A named
        // window that does not reach back to it was never affected.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.gap(10_000);
        // The feed comes back and delivers, which is the proof that it did.
        reg.ingest(trade(&c, 1.0, 11_000 * MS));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            600_000,
        )
        .expect("nothing to refuse");
        assert!(
            !r.clipped,
            "this second of tape is nowhere near the interruption"
        );
    }

    #[test]
    fn a_window_after_the_feed_came_back_stays_whole_however_often_it_is_read() {
        // The loss ended where the first row after it landed. Dating it at
        // the read that noticed would clip windows sitting entirely after
        // the feed returned, and clip them again on every later read.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.gap(10_000);
        reg.ingest(trade(&c, 1.0, 11_000 * MS));
        let first = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            20_000,
        )
        .expect("nothing to refuse");
        assert!(!first.clipped, "this second sits after the feed came back");
        let again = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            20_100,
        )
        .expect("nothing to refuse");
        assert!(!again.clipped, "and it is still after it on the next read");
    }

    #[test]
    fn a_schema_never_offers_a_kind_the_call_would_refuse() {
        // A caller composing against the schema should not be able to write
        // a request whose only possible answer is a refusal. The schema's
        // enum, narrowed by whatever conditional applies to the type, has to
        // be exactly what `resolve_kind` accepts for that type.
        let t = tool_definitions()
            .into_iter()
            .find(|t| t["name"] == "live_read")
            .expect("live_read is advertised");
        let schema = &t["inputSchema"];
        let flat: Vec<String> = schema["properties"]["kind"]["enum"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        assert!(!flat.is_empty(), "live_read advertises a kind enum");
        let rules = schema["allOf"].as_array().cloned().unwrap_or_default();

        for name in sec_types_for("live_read") {
            let narrowed: Option<Vec<String>> = rules
                .iter()
                .find(|r| r["if"]["properties"]["sec_type"]["const"] == *name)
                .and_then(|r| r["then"]["properties"]["kind"]["enum"].as_array().cloned())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_string)
                        .collect()
                });
            let advertised = narrowed.unwrap_or_else(|| flat.clone());

            // Stated, not derived: read out of `kinds_for` this would
            // compare the schema to the source it is built from.
            let expected: &[&str] = match *name {
                "option" => &["quote", "trade", "market_value", "open_interest"],
                // A stock has no open interest; an index has neither that nor
                // a quote, and its price arrives on the trade subscription.
                "stock" => &["quote", "trade", "market_value"],
                "index" => &["trade", "market_value"],
                other => panic!("{other} is offered and this test does not say what it takes"),
            };
            assert_eq!(
                advertised, expected,
                "{name} is advertised exactly the kinds the call takes"
            );
        }
    }

    #[test]
    fn closing_a_whole_market_reads_the_types_its_own_schema_offers() {
        // Read against another tool's list, an index could never be named
        // here, so the refusal that says what to do instead was unreachable
        // and the caller was told the type was not served at all.
        let why = refused(whole_market_target(
            "live_stop",
            &json!({"sec_type": "index"}),
        ));
        assert!(
            why.contains("no whole-market stream to close"),
            "an index is named, and told where its stream lives: {why}"
        );
        assert!(
            !why.contains("sec_type must be"),
            "not refused as a type this tool does not serve: {why}"
        );
        assert_eq!(
            whole_market_target("live_stop", &json!({"sec_type": "stock"})).ok(),
            Some(SecType::Stock),
            "a type with a whole-market stream is named"
        );
    }

    #[test]
    fn every_advertised_tool_is_one_the_server_answers() {
        // Two hand-kept lists: the names the server routes on, and the names
        // in the definitions it publishes. A name in one and not the other is
        // either a tool nobody can call or one advertised and unimplemented.
        let mut advertised: Vec<String> = tool_definitions()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect();
        advertised.sort();
        let mut routed: Vec<String> = TOOL_NAMES.iter().map(|n| (*n).to_string()).collect();
        routed.sort();
        assert_eq!(advertised, routed, "every advertised tool is a routed one");
    }

    #[test]
    fn a_schema_says_when_an_option_leg_is_required() {
        // A caller composing against the schema should not be able to write
        // a request whose only possible answer is a refusal.
        for tool in ["live_read", "live_prints", "live_stop"] {
            let t = tool_definitions()
                .into_iter()
                .find(|t| t["name"] == tool)
                .unwrap_or_else(|| panic!("{tool} is advertised"));
            // Found by what it says, not by where it sits: other rules
            // share the list.
            let rules = t["inputSchema"]["allOf"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let rule = rules
                .iter()
                .find(|r| r["if"]["properties"]["sec_type"]["const"] == "option")
                .unwrap_or_else(|| panic!("{tool} has a rule for an option leg"));
            assert_eq!(
                rules
                    .iter()
                    .filter(|r| r["if"]["properties"]["sec_type"]["const"] == "option")
                    .count(),
                1,
                "{tool} says it once, so two rules cannot disagree about the leg"
            );
            let needs = rule["then"]["required"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            assert_eq!(needs, OPTION_LEG.to_vec(), "{tool} names the whole leg");
        }
    }

    #[test]
    fn a_discarded_replacement_does_not_leave_prints_unaccounted_for() {
        // Its counts cannot be folded in: it was asking something else. But
        // the prints it saw are still prints this contract received, and the
        // gap between received and examined must not read as nothing having
        // happened.
        let reg = Registry::default();
        let spy = option("550000", "C");
        let mut aapl = query(5);
        aapl.root = Some("AAPL".into());
        market_now(&reg, SecType::Option, aapl.clone(), 0, 0).expect("nothing to refuse");

        let mut spy_q = query(5);
        spy_q.root = Some("SPY".into());
        let lost = reg
            .market(SecType::Option, spy_q, 0, 1)
            .expect("nothing to refuse");
        reg.ingest(trade(&spy, 1.0, 2 * MS));
        reg.restore_market(SecType::Option, lost.settle.outgoing);

        let next = market_now(&reg, SecType::Option, aapl, 0, 3).expect("nothing to refuse");
        assert_eq!(next.examined, 0, "an SPY print is not this selection's");
        assert_eq!(next.new_since_last_read, 1, "but it did arrive");
        assert!(
            next.gap,
            "and the answer says its view of the tape was broken"
        );
    }

    #[test]
    fn a_view_that_failed_to_open_still_counts_what_followed_it() {
        // Opening the subscriptions starts the count, not the first answer
        // that lands. A call that fails still opened them, so a discard
        // after that belongs to this contract rather than being swallowed.
        let reg = Registry::default();
        let c = stock("AAPL");
        // The feed has already discarded five before this contract was ever
        // looked at; those were never this view's to report.
        reg.prints(&c, 10, 5, 0).expect("nothing to refuse");
        // One more is discarded, then a retry succeeds.
        let retry = reg.prints(&c, 10, 6, MS).expect("nothing to refuse");
        assert_eq!(
            retry.feed_dropped_since_last_read, 1,
            "the one discarded since the subscriptions opened, not all six"
        );
    }

    #[test]
    fn a_reopened_trade_leg_does_not_take_a_bar_the_closed_one_queued() {
        // An older quote buffer keeps the contract alive across the drop, so
        // measuring the bar against the earliest buffer this contract holds
        // dates it to that quote buffer and calls a bar from the closed trade
        // subscription current.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 0).expect("nothing to refuse");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        // The feed drops the trade leg; the quote buffer keeps the contract.
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_400)
            .expect("nothing to refuse");

        // Decoded at 2_500, under the subscription that has since closed.
        reg.ingest(bar(&c, 1.5, 2_500 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 4_000);
        assert!(
            r.ohlcvc.is_none(),
            "a bar the closed trade subscription queued is not the reopened one's: {:?}",
            r.ohlcvc
        );

        // And one produced after the reopening is kept.
        reg.ingest(bar(&c, 1.7, 3_500 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 4_100);
        assert!(matches!(r.ohlcvc, Some(StreamData::Ohlcvc { close, .. }) if close == 1.7));
    }

    #[test]
    fn a_bar_with_no_buffer_to_measure_it_against_is_not_kept() {
        // A bar queued by a subscription that has closed is not the next
        // one's. With the trade buffer gone it has nothing to be measured
        // against except the buffers that remain.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        // A fresh quote-only view, opened long after that bar was decoded.
        read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 10_000)
            .expect("nothing to refuse");
        reg.ingest(bar(&c, 1.0, MS));
        let r = read_now(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0, 11_000)
            .expect("nothing to refuse");
        assert!(
            r.ohlcvc.is_none(),
            "the previous subscription's bar is not this buffer's"
        );
    }

    #[test]
    fn a_restored_selection_does_not_take_counts_from_a_different_question() {
        // A replacement asking something else examined a different
        // population and kept rows this one would have refused. Folding
        // those in would report another question's work as its own.
        let reg = Registry::default();
        let spy = option("550000", "C");
        let mut aapl = query(5);
        aapl.root = Some("AAPL".into());
        market_now(&reg, SecType::Option, aapl.clone(), 0, 0).expect("nothing to refuse");

        // The read that changes the question fails on the feed, so the AAPL
        // selection goes back; meanwhile the replacement caught an SPY row.
        let mut spy_q = query(5);
        spy_q.root = Some("SPY".into());
        let lost = reg
            .market(SecType::Option, spy_q, 0, 1)
            .expect("nothing to refuse");
        reg.ingest(trade(&spy, 1.0, MS));
        reg.restore_market(SecType::Option, lost.settle.outgoing);

        let next = market_now(&reg, SecType::Option, aapl, 0, 2).expect("nothing to refuse");
        assert_eq!(
            (next.examined, next.matched, next.rows.len()),
            (0, 0, 0),
            "an SPY print is not this selection's to count"
        );
    }

    #[test]
    fn no_window_is_whole_while_the_feed_is_unproven() {
        // The interruption is disclosed once, but until something arrives
        // there is no evidence the feed is delivering, so neither shape of
        // window can be called whole.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.gap(1_000);
        let disclosed = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 2_000)
            .expect("nothing to refuse");
        assert!(disclosed.clipped, "the read that notices");
        let still = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        assert!(
            still.clipped,
            "the interruption is spent, but nothing has arrived to show the feed is back"
        );
        // A row is that evidence.
        reg.ingest(trade(&c, 1.0, 4_000 * MS));
        let proven = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 5_000)
            .expect("nothing to refuse");
        assert!(
            proven.clipped,
            "this window still reaches back over where the loss ended"
        );
        let after = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 6_000)
            .expect("nothing to refuse");
        assert!(!after.clipped, "and this one begins after it");
    }

    #[test]
    fn a_discard_holes_a_print_history_for_as_long_as_it_shows() {
        // The discard is disclosed once. The prints on either side of the
        // missing one are served again and again, and the history between
        // them has a hole in it the whole time.
        let reg = Registry::default();
        let c = stock("AAPL");
        let p = reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        reg.ingest(trade(&c, 1.0, MS));
        // The feed discarded one between these two.
        reg.ingest(trade(&c, 3.0, 3 * MS));
        let first = reg.prints(&c, 10, 2, 4).expect("nothing to refuse");
        assert!(first.holed, "the read that learns of the discard");
        reg.commit_prints_read(&c, first.received, first.feed_drops_seen, first.gaps_seen);
        let again = reg.prints(&c, 10, 2, 5).expect("nothing to refuse");
        assert_eq!(again.feed_dropped_since_last_read, 0, "disclosed once");
        assert!(again.holed, "but the same prints with the same hole");
    }

    #[test]
    fn a_buffer_put_back_and_read_at_once_does_not_date_its_window_to_the_epoch() {
        // Reinstating marks a buffer idle so the next sweep hands its
        // subscription back. The floor a default window reads from is a
        // different thing, and zeroing it dates the window to 1970.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        reg.reinstate(&subscription(SubscriptionKind::Trade, &c), 2_000);
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        // Stated as the moment the buffer was put back, not as a bound that
        // `saturating_sub` would satisfy for a floor of nought as readily as
        // for the right one.
        assert_eq!(
            r.floor, 2_000,
            "the window starts when the buffer was put back, not at the epoch"
        );
    }

    #[test]
    fn asking_for_more_prints_than_exist_reaches_into_the_hole_before_them() {
        // The loss happened before any print was held, so no returned row
        // spans it. A caller asking for more than exist reaches back into
        // it regardless, and the answer says so.
        let reg = Registry::default();
        let c = stock("AAPL");
        let p = reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        // The feed breaks before anything printed, then one arrives.
        reg.gap(1);
        reg.ingest(trade(&c, 1.0, 2 * MS));
        let first = reg.prints(&c, 2, 0, 3).expect("nothing to refuse");
        reg.commit_prints_read(&c, first.received, first.feed_drops_seen, first.gaps_seen);
        let again = reg.prints(&c, 2, 0, 4).expect("nothing to refuse");
        assert_eq!(again.rows.len(), 1, "only one print exists");
        assert!(
            again.holed,
            "and the two that were asked for reach back over the break"
        );
        let enough = reg.prints(&c, 1, 0, 5).expect("nothing to refuse");
        assert!(!enough.holed, "asking only for what is there is whole");
    }

    #[test]
    fn a_buffer_put_back_after_a_failed_release_is_not_an_unbroken_one() {
        // The feed would not let the subscription go, so the buffer is kept
        // to try again. It was gone while the feed kept delivering, and a
        // later read must not find an intact leg and conclude nothing was
        // missed.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, MS));
        reg.forget(&subscription(SubscriptionKind::Trade, &c));
        reg.reinstate(&subscription(SubscriptionKind::Trade, &c), 2);
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3)
            .expect("nothing to refuse");
        assert!(
            r.clipped,
            "the leg was absent while the feed carried on without it"
        );
    }

    #[test]
    fn the_newest_hole_is_the_one_a_history_is_measured_against() {
        // Two losses, the second later than the first. Once the prints from
        // before the first are gone the history still spans the second, and
        // a marker left at the first would declare it whole.
        let reg = Registry::default();
        let c = stock("AAPL");
        let p = reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);

        reg.ingest(trade(&c, 1.0, MS));
        let first = reg.prints(&c, 10, 1, 2).expect("nothing to refuse");
        assert!(first.holed, "one print held, one hole before it");
        reg.commit_prints_read(&c, first.received, first.feed_drops_seen, first.gaps_seen);

        // Two more, then a second loss, all while nothing has been evicted.
        reg.ingest(trade(&c, 2.0, 2 * MS));
        reg.ingest(trade(&c, 3.0, 3 * MS));
        let second = reg.prints(&c, 10, 2, 4).expect("nothing to refuse");
        assert!(second.holed, "and a second hole after those");
        reg.commit_prints_read(
            &c,
            second.received,
            second.feed_drops_seen,
            second.gaps_seen,
        );

        // Evict past the first hole, but not past the second: three prints
        // are held, so one more than the bound drops exactly the first.
        for i in 0..PRINTS - 2 {
            reg.ingest(trade(&c, 9.0, (10 + i as u64) * MS));
        }
        let again = reg.prints(&c, PRINTS, 2, 5).expect("nothing to refuse");
        assert_eq!(again.feed_dropped_since_last_read, 0, "disclosed already");
        assert_eq!(
            again.dropped, 1,
            "the print before the first hole, and only it"
        );
        assert!(
            again.holed,
            "the prints held still sit either side of the second hole"
        );
    }

    #[test]
    fn a_loss_inside_a_default_window_clips_it_even_once_disclosed() {
        // The interruption is disclosed once, but a window that begins
        // before the loss ended still reaches over it.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.gap(2_000);
        let disclosed = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 3_000)
            .expect("nothing to refuse");
        assert!(disclosed.clipped, "the read that notices");
        // Delivery resumes after that read, so the loss ended inside the
        // window the next one covers.
        reg.ingest(trade(&c, 1.0, 4_000 * MS));
        let spanning = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 5_000)
            .expect("nothing to refuse");
        assert!(spanning.clipped, "this window reaches back over it");
        let after = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 6_000)
            .expect("nothing to refuse");
        assert!(!after.clipped, "and this one begins after it");
    }

    #[test]
    fn a_hole_the_rows_returned_do_not_span_is_not_theirs() {
        // Asking for only the newest prints, all of them after a loss, is a
        // complete answer to what was asked.
        let reg = Registry::default();
        let c = stock("AAPL");
        let p = reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        reg.ingest(trade(&c, 1.0, MS));
        reg.gap(2);
        reg.ingest(trade(&c, 2.0, 3 * MS));
        let both = reg.prints(&c, 10, 0, 4).expect("nothing to refuse");
        assert!(both.holed, "these rows sit either side of it");
        reg.commit_prints_read(&c, both.received, both.feed_drops_seen, both.gaps_seen);
        let newest = reg.prints(&c, 1, 0, 5).expect("nothing to refuse");
        assert_eq!(newest.rows.len(), 1, "only the newest was asked for");
        assert!(!newest.holed, "and it is entirely after the loss");
    }

    #[test]
    fn a_resumption_row_with_no_clock_does_not_erase_the_loss() {
        // The row proves the feed is delivering again. Its stamp is the
        // SDK's fallback for a clock it could not read, so it dates
        // nothing, and dating the loss at zero would erase it.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.gap(10_000);
        // Delivery resumes, but with an unreadable clock.
        reg.ingest(trade(&c, 1.0, 0));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            60_000,
        )
        .expect("nothing to refuse");
        assert!(
            r.clipped,
            "the interruption still stands; nothing has dated its end"
        );
    }

    #[test]
    fn prints_inherit_the_losses_behind_the_baseline_they_take() {
        // A buffer that already counted a discard hands on its number. Taking
        // it without the loss behind it would start these prints from a
        // clean history the buffer knows is holed.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, MS));
        // The buffer learns of a discard and advances its own baseline.
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 3, 2 * MS)
            .expect("nothing to refuse");
        assert_eq!(r.feed_dropped_since_last_read, 3, "the buffer counted them");
        reg.ingest(trade(&c, 2.0, 3 * MS));
        // The first prints call inherits that baseline.
        let p = reg.prints(&c, 10, 3, 4 * MS).expect("nothing to refuse");
        assert_eq!(p.feed_dropped_since_last_read, 0, "already counted once");
        assert!(
            p.holed,
            "but the history these prints come from has the hole in it"
        );
    }

    #[test]
    fn a_stamp_the_clock_could_not_read_does_not_end_a_window() {
        // Zero is the SDK's fallback for a clock it could not read. Taking
        // it as older than the floor ends the reverse walk and hides every
        // row behind it that does belong in the window.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 1_500 * MS));
        reg.ingest(trade(&c, 2.0, 0));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            2_000,
        )
        .expect("nothing to refuse");
        assert_eq!(r.count, 2, "both rows are in the window");
        assert_eq!(r.tail.len(), 2, "and both come back");
    }

    #[test]
    fn an_unknown_clock_has_no_age_rather_than_a_very_old_one() {
        // Zero is the SDK's fallback for a clock it could not read. Aging
        // from it reports the time since the epoch, which presents an
        // unknown age as decades.
        let c = stock("AAPL");
        let unknown = aged_object_dated(&trade(&c, 1.0, 0), 1_700_000_000_000, false);
        assert!(
            unknown.get("age_ms").is_none(),
            "no age at all: {unknown:?}"
        );
        let known = aged_object_dated(&trade(&c, 1.0, 5 * MS), 10, false);
        assert_eq!(known.get("age_ms").and_then(|v| v.as_u64()), Some(5));
    }

    #[test]
    fn a_named_window_does_not_reach_past_now() {
        // A row can land between the clock being read and the registry
        // being locked. It is newer than the window, not inside it.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 2_000 * MS));
        let r = read_now(&reg, &c, SubscriptionKind::Trade, Some(0), TAIL, 0, 1_500)
            .expect("nothing to refuse");
        assert_eq!(
            r.count, 0,
            "a zero-length window ending now holds nothing stamped after it"
        );
    }

    #[test]
    fn a_clock_that_steps_backwards_does_not_hide_the_rows_behind_it() {
        // Arrivals are in order; the stamps on them are not guaranteed to
        // be. A row outside the window does not mean the rest are, so the
        // walk keeps looking instead of stopping at the first one.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1_000)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 3_000 * MS));
        // The clock steps back, so this one is stamped before the last.
        reg.ingest(trade(&c, 2.0, 2_000 * MS));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            3_500,
        )
        .expect("nothing to refuse");
        assert_eq!(
            r.count, 1,
            "the row inside the window, found behind the one outside it"
        );
        assert_eq!(r.tail.len(), 1, "and it comes back");
    }

    #[test]
    fn a_named_window_is_the_interval_that_was_asked_for() {
        // Only the default window stretches to cover a row decoded before
        // the last read. A fixed lookback means what it says.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, MS));
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(1_000),
            TAIL,
            0,
            60_000,
        )
        .expect("nothing to refuse");
        assert_eq!(r.floor, 59_000, "sixty seconds in, one second back");
        assert_eq!(
            window_start(Some(1_000), &r),
            59_000,
            "the window named, not the one the rows would widen it to"
        );
    }

    #[test]
    fn a_default_window_stretches_back_over_a_row_delivered_late() {
        // A row decoded before the previous read and dispatched after it is
        // new to this one, so the span named beside it has to hold it. A
        // named window is the interval asked for and does not move.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 5_000 * MS));
        let mut r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 20_000)
            .expect("nothing to refuse");
        assert_eq!(r.tail.len(), 1, "the row came back");
        // Its stamp sits before the floor a later read would use.
        r.floor = 10_000;
        assert_eq!(
            window_start(None, &r),
            5_000,
            "the default span reaches back over the row it returns"
        );
        assert_eq!(
            window_start(Some(1_000), &r),
            10_000,
            "a named window is the interval asked for"
        );
    }

    #[test]
    fn a_row_captured_before_a_buffer_reopened_is_not_its_own() {
        // A buffer reopened after an expiry can still be handed rows the
        // previous subscription queued. They belong to a feed this buffer was
        // not on, and counting them makes a fresh buffer look busy.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.expire(PAST_TTL);
        // Reopened long after the first buffer went.
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            0,
            PAST_TTL + 1,
        )
        .expect("nothing to refuse");
        assert!(r.first, "a fresh buffer");
        // Queued by the subscription that closed.
        reg.ingest(trade(&c, 1.0, MS));
        // Delivered by the one that opened.
        reg.ingest(trade(&c, 2.0, (PAST_TTL + 2) * MS));
        let next = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            0,
            PAST_TTL + 3,
        )
        .expect("nothing to refuse");
        assert_eq!(
            next.new_since_last_read, 1,
            "only the row this buffer was open for"
        );
    }

    #[test]
    fn a_market_read_ages_the_rows_it_returns_not_the_tape() {
        // A narrow selection holds an old row while the tape stays busy.
        // Reporting the tape's age as the row's is the one thing this
        // surface exists not to do.
        let reg = Registry::default();
        let mine = stock("AAPL");
        let other = stock("MSFT");
        let mut q = query(5);
        q.root = Some("AAPL".into());
        market_now(&reg, SecType::Stock, q.clone(), 0, 1_000).expect("nothing to refuse");
        reg.ingest(trade(&mine, 1.0, 1_100 * MS));
        // The tape carries on, none of it this selection's.
        reg.ingest(trade(&other, 2.0, 5_000 * MS));
        let m = market_now(&reg, SecType::Stock, q, 0, 6_000).expect("nothing to refuse");
        assert_eq!(m.rows.len(), 1, "the one print that matched");
        let returned = m
            .rows
            .iter()
            .filter_map(|p| seen_ms(&p.trade))
            .max()
            .expect("a stamp");
        assert_eq!(returned, 1_100, "the matching print, not the newest trade");
        assert_eq!(
            m.newest_ms,
            Some(5_000),
            "the tape moved on without this selection"
        );
        assert_eq!(
            rows_age_ms(&m.rows, 6_000),
            Some(4_900),
            "the age of the print returned, not of the tape"
        );
    }

    #[test]
    fn a_stock_is_not_offered_a_count_of_contracts_outstanding() {
        // Open interest is a number of option contracts. A stock has none,
        // and the buffer would never publish.
        assert!(!kinds_for(SecType::Stock).contains(&SubscriptionKind::OpenInterest));
        assert!(kinds_for(SecType::Option).contains(&SubscriptionKind::OpenInterest));
        let why = refused(resolve_kind(SecType::Stock, Some("open_interest")));
        assert!(why.contains("not available"), "{why}");
    }

    #[test]
    fn a_row_captured_before_a_market_buffer_existed_is_not_its_own() {
        // A per-contract subscription swept a moment earlier still has rows
        // in flight, and they route to a market buffer by security type. They
        // belong to a subscription this selection never had.
        let reg = Registry::default();
        let call = option("550000", "C");
        market_now(&reg, SecType::Option, query(5), 0, 10_000).expect("nothing to refuse");
        // Captured before the buffer opened.
        reg.ingest(trade(&call, 1.0, 5_000 * MS));
        // Captured after it.
        reg.ingest(trade(&call, 2.0, 11_000 * MS));
        let m = market_now(&reg, SecType::Option, query(5), 0, 12_000).expect("nothing to refuse");
        assert_eq!(
            (m.examined, m.rows.len()),
            (1, 1),
            "only the print this selection was open for"
        );
    }

    #[test]
    fn a_second_view_of_a_contract_inherits_what_the_first_was_counting() {
        // Discards belong to the contract, not to whichever tool looked
        // first. A view opened afterwards must not report a clean history
        // the other one already knows has a hole in it.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        // The feed discards three while only the trade buffer is watching.
        let p = reg.prints(&c, 10, 3, MS).expect("nothing to refuse");
        assert_eq!(
            p.feed_dropped_since_last_read, 3,
            "prints start where the buffer already was, not from nothing"
        );
    }

    #[test]
    fn a_history_with_a_hole_in_it_says_so_every_time() {
        // The interruption is disclosed once, but the prints on either side
        // of the missing ones are served again and again, and the history
        // they come from has a hole in it for as long as they are held.
        let reg = Registry::default();
        let c = stock("AAPL");
        let p = reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        reg.ingest(trade(&c, 1.0, MS));
        reg.gap(2);
        reg.ingest(trade(&c, 3.0, 3 * MS));
        let first = reg.prints(&c, 10, 0, 4).expect("nothing to refuse");
        assert!(first.gap && first.holed, "both, on the read that notices");
        reg.commit_prints_read(&c, first.received, first.feed_drops_seen, first.gaps_seen);
        let again = reg.prints(&c, 10, 0, 5).expect("nothing to refuse");
        assert!(!again.gap, "the interruption is disclosed once");
        assert!(
            again.holed,
            "but these are the same prints, with the same hole between them"
        );
    }

    #[test]
    fn a_quote_leg_coming_back_does_not_pair_across_the_interval_it_missed() {
        // A trade can land between the sweep that drops the quote leg and
        // the call that brings it back. Quotes after that are from a
        // different interval than the one the print traded in.
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        // The quote leg goes idle while the trade leg is kept alive.
        read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            0,
            PAST_TTL - 1,
        )
        .expect("nothing to refuse");
        reg.expire(PAST_TTL);
        // A print arrives while nothing is watching the quotes.
        reg.ingest(trade(&c, 1.0, PAST_TTL * MS));
        // The quote leg comes back, and quotes start flowing again.
        let p = reg
            .prints(&c, 10, 0, PAST_TTL + 1)
            .expect("nothing to refuse");
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        reg.ingest(quote(&c, 1.0, 2.0));
        reg.ingest(quote(&c, 1.1, 2.1));
        let after = reg
            .prints(&c, 10, 0, PAST_TTL + 2)
            .expect("nothing to refuse");
        let orphan = after
            .rows
            .iter()
            .find(|r| price(&r.trade) == 1.0)
            .expect("the print survived");
        assert!(
            orphan.quotes_after.is_empty(),
            "those quotes are from after an interval nobody watched"
        );
    }

    #[test]
    fn a_market_read_that_is_never_answered_gives_its_rows_back() {
        // The feed steps run after the selection has been taken. A call
        // that fails there returns no prints, so the prints it took must
        // reach whoever asks next, and the population they were chosen
        // from must survive with them.
        let reg = Registry::default();
        let call = option("550000", "C");
        // A selection that keeps one row of however many it sees, so the
        // rows kept and the rows counted are different numbers.
        market_now(&reg, SecType::Option, query(1), 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&call, 1.0, MS));
        reg.ingest(trade(&call, 2.0, 2 * MS));

        // Taken, then the call fails: nothing reached the caller.
        let lost = reg
            .market(SecType::Option, query(1), 0, 3)
            .expect("nothing to refuse");
        assert_eq!((lost.rows.len(), lost.examined), (1, 2));
        // Three more arrive against the replacement while the call fails.
        for (i, price) in [3.0, 4.0, 5.0].into_iter().enumerate() {
            reg.ingest(trade(&call, price, (4 + i as u64) * MS));
        }
        reg.restore_market(SecType::Option, lost.settle.outgoing);

        let next = market_now(&reg, SecType::Option, query(1), 0, 9).expect("nothing to refuse");
        assert_eq!(
            next.rows.len(),
            1,
            "one row, as the selection was asked for"
        );
        assert_eq!(
            next.examined, 5,
            "every print since the last answered read, counted once each"
        );
        let settled =
            market_now(&reg, SecType::Option, query(1), 0, 10).expect("nothing to refuse");
        assert_eq!((settled.rows.len(), settled.examined), (0, 0));
    }

    #[test]
    fn a_row_the_clock_could_not_stamp_has_no_age_and_vouches_for_no_coverage() {
        // The SDK stamps a row it could not read the host clock for with
        // nought. Read as a time that is the epoch, so the row reports an age
        // of fifty-five years and a window claiming to have covered them.
        let reg = Registry::default();
        let c = stock("AAPL");
        let now = 100 * MS / 1_000;

        // Open the buffers first: ingest before that has nowhere to land.
        reg.prints(&c, 10, 0, now).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 0));
        let p = reg.prints(&c, 10, 0, now).expect("nothing to refuse");
        assert_eq!(p.rows.len(), 1, "the row is still served");
        assert_eq!(
            rows_age_ms(&p.rows, now),
            None,
            "an unstamped row has no age, rather than one measured from the epoch"
        );
        assert_eq!(
            p.covered_since_ms, now,
            "coverage starts when the buffer opened, not at the epoch"
        );

        // And on a buffer read, where an epoch coverage start also unsets
        // `clipped`: nought is below every floor, so every test for a loss
        // inside the window answers no and the window reads as whole.
        //
        // Coverage only consults the oldest held row once the ring has
        // overflowed, so the ring is pushed past it with unstamped rows.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 1).expect("nothing to refuse");
        for _ in 0..=RING {
            reg.ingest(trade(&c, 1.0, 0));
        }
        let r = read_now(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(60_000),
            TAIL,
            0,
            now,
        )
        .expect("nothing to refuse");
        assert!(r.dropped > 0, "the ring overflowed, so rows were lost");
        assert_eq!(
            r.covered_since_ms, now,
            "rows were evicted and no held row can be placed, so nothing before \
             this read is proven covered"
        );
        assert!(
            r.clipped,
            "a window reaching back before the buffer opened is not whole"
        );
    }

    #[test]
    fn prints_coverage_never_starts_after_a_print_it_returns() {
        // A trade buffer reopened after the prints it holds opened later than
        // they arrived. Taking its clock would claim coverage beginning
        // after rows the same answer is showing.
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 10, 0, 0).expect("nothing to refuse");
        // Stamped: an unstamped row has no arrival time to compare coverage
        // against, so the property this test states would be vacuous.
        reg.ingest(trade(&c, 1.0, MS));
        // The quote leg is kept alive while the trade leg goes idle, so the
        // trade buffer expires on its own and the print it produced survives.
        read_now(
            &reg,
            &c,
            SubscriptionKind::Quote,
            None,
            TAIL,
            0,
            PAST_TTL - 1,
        )
        .expect("nothing to refuse");
        reg.expire(PAST_TTL);
        // A later prints call reopens the trade buffer, long after the print.
        let p = reg
            .prints(&c, 10, 0, PAST_TTL + 1)
            .expect("nothing to refuse");
        assert_eq!(p.rows.len(), 1, "the print survived");
        let oldest = seen_ms(&p.rows[0].trade).expect("a stamp");
        assert!(
            p.covered_since_ms <= oldest,
            "coverage starts at or before the oldest row it returns: {} vs {oldest}",
            p.covered_since_ms
        );
    }

    #[test]
    fn an_argument_the_tool_does_not_take_is_refused() {
        // An unknown name is a question the caller meant to ask and the
        // server did not hear. `strike` on a whole-market read is the shape
        // of it: narrowing to one strike, ignored, handed every strike.
        let why = refused(only_declared_arguments(
            "live_market",
            &json!({"sec_type": "option", "root": "SPY", "strike": 550}),
        ));
        assert!(
            why.contains("does not take strike") && why.contains("strike_min"),
            "names it and what it does take: {why}"
        );
        // Every name a tool's own schema declares is one a caller may send.
        // Stated as literals: built from the schema, the argument object
        // would be checked against the map it came from.
        for (tool, names) in [
            (
                "live_read",
                &["sec_type", "root", "kind", "seconds", "tail"][..],
            ),
            (
                "live_prints",
                &["sec_type", "root", "count", "quotes_after"][..],
            ),
            ("live_list", &[][..]),
        ] {
            let mut args = json!({});
            if let Some(o) = args.as_object_mut() {
                for k in names {
                    o.insert(k, Value::from(1));
                }
            }
            only_declared_arguments(tool, &args)
                .unwrap_or_else(|e| panic!("{tool} takes the names it declares: {e:?}"));
        }
    }

    #[test]
    fn a_tool_recommends_only_a_security_type_it_serves() {
        // Recommending one it always refuses sends the caller into a second
        // refusal.
        let why = refused(parse_sec_of(
            &json!({"sec_type": "bogus"}),
            &["option", "stock"],
        ));
        assert!(
            !why.contains("index"),
            "does not offer what it refuses: {why}"
        );
        assert!(why.contains("option or stock"), "{why}");
        assert_eq!(
            parse_sec_of(&json!({"sec_type": "index"}), &["option", "stock", "index"])
                .expect("served here"),
            SecType::Index
        );
        let why = refused(parse_sec_of(
            &json!({"sec_type": "index"}),
            &["option", "stock"],
        ));
        assert!(why.contains("option or stock"), "{why}");
    }

    #[test]
    fn naming_an_option_leg_on_something_that_has_none_is_refused() {
        // Acting on the stock instead would close a buffer the caller never
        // named, which is the wrong answer to a clear question.
        for key in OPTION_LEG {
            let why = refused(parse_contract(
                &json!({"sec_type": "stock", "root": "AAPL", key: 1}),
            ));
            assert!(
                why.contains(key) && why.contains("not an option"),
                "names the leg it refused: {why}"
            );
        }
        // A stock on its own still parses, and so does a whole option leg.
        parse_contract(&json!({"sec_type": "stock", "root": "AAPL"})).expect("a stock");
        parse_contract(&json!({
            "sec_type": "option", "root": "SPY",
            "expiration": 20260620, "strike": 550, "right": "C"
        }))
        .expect("an option");
    }

    #[test]
    fn a_clause_value_refusal_says_a_number_is_allowed() {
        // Naming only the fields reads as though a number were not allowed,
        // and a number is the common case.
        let why = refused(parse_clauses(
            &json!([{"field": "price", "op": ">", "value": true}]),
            &FIELDS,
        ));
        assert!(
            why.contains("must be a number"),
            "does not exclude the common case: {why}"
        );
    }

    #[test]
    fn an_argument_that_cannot_be_read_is_refused_not_defaulted() {
        // Replacing it with the default answers a different question from
        // the one asked, and says nothing about having done so.
        for (key, bad, what) in [
            ("seconds", json!("60"), "a number of seconds"),
            ("seconds", json!(-60), "a number of seconds"),
            ("tail", json!(-1), "a whole number of rows"),
            ("count", json!(1.5), "a whole number of rows"),
            ("quotes_after", json!("true"), "true or false"),
            ("ascending", json!("true"), "true or false"),
        ] {
            let args = json!({ key: bad });
            let got = match key {
                "seconds" => arg(&args, "seconds", "a number of seconds, zero or more", |v| {
                    v.as_f64().filter(|s| *s >= 0.0)
                })
                .err(),
                "quotes_after" | "ascending" => {
                    arg(&args, key, "true or false", Value::as_bool).err()
                }
                _ => count_arg(&args, key, "a whole number of rows, zero or more", 20).err(),
            };
            let why = format!("{:?}", got.expect("refused"));
            assert!(why.contains(key) && why.contains(what), "{why}");
        }
        // Absent and null still mean the default.
        assert_eq!(
            count_arg(&json!({}), "tail", "rows", 10).expect("default"),
            10
        );
        assert_eq!(
            count_arg(&json!({"tail": null}), "tail", "rows", 10).expect("default"),
            10
        );
        assert_eq!(
            count_arg(&json!({"tail": 3}), "tail", "rows", 10).expect("given"),
            3
        );
    }

    #[test]
    fn a_narrowing_filter_that_cannot_be_read_is_refused_not_dropped() {
        // Dropping one widens the selection: a caller asking for a single
        // expiration would be handed every expiration and told nothing.
        for (key, bad) in [
            ("expiration", json!(4_294_967_296i64)),
            ("expiration", json!("20260620")),
            ("strike_min", json!("550")),
            ("strike_max", json!(false)),
            // The two that widen the most: a root that is not a string
            // selects every root, and a right that is not a string selects
            // both.
            ("root", json!(123)),
            ("right", json!(true)),
        ] {
            let why = refused(parse_market_query(&json!({key: bad}), 10));
            assert!(why.contains(key), "names the field it refused: {why}");
        }
        // Absent and explicitly null both mean no filter.
        let q = parse_market_query(&json!({"expiration": null}), 10).expect("no filter");
        assert_eq!(q.expiration, None);
        let q = parse_market_query(&json!({"expiration": 20260620}), 10).expect("a date");
        assert_eq!(q.expiration, Some(20260620));
    }

    #[test]
    fn a_gap_arriving_after_a_read_is_not_buried_by_its_commit() {
        // The feed can break between the answer and the commit that settles
        // it. Clearing a mark the caller never saw would lose it entirely.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 0));
        let p = reg.prints(&c, 10, 0, 1).expect("nothing to refuse");
        assert!(!p.gap, "nothing had been interrupted when this was read");
        reg.gap(1);
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        let next = reg.prints(&c, 10, 0, 2).expect("nothing to refuse");
        assert!(
            next.gap,
            "the interruption the answer predates still stands"
        );
    }

    #[test]
    fn a_rank_key_that_cannot_be_ordered_is_not_ranked() {
        // partition_point needs the predicate to hold on a prefix. Every
        // comparison with a non-finite key is false, so it lands at the
        // front and the next row displaces the real best.
        let reg = Registry::default();
        let call = option("550000", "C");
        let mut q = query(1);
        q.rank_by = Some("price".into());
        market_now(&reg, SecType::Option, q.clone(), 0, 0).expect("nothing to refuse");
        reg.ingest(trade(&call, 10.0, 0));
        reg.ingest(trade(&call, f64::NAN, 1));
        reg.ingest(trade(&call, 5.0, 2));
        let m = market_now(&reg, SecType::Option, q, 0, 3).expect("nothing to refuse");
        assert_eq!(m.unranked, Some(1), "the unorderable one is set aside");
        let kept: Vec<f64> = m
            .rows
            .iter()
            .filter_map(|p| print_field(&p.trade, None, "price"))
            .collect();
        assert_eq!(kept, vec![10.0], "the largest finite price still wins");
    }

    #[test]
    fn a_feed_gap_clips_a_window_the_rows_no_longer_cover() {
        // A reconnect loses rows that never reached the SDK at all, so its
        // discard count stays at zero and the ring simply has a hole. The
        // window is not whole and must not read as if it were.
        let reg = Registry::default();
        let c = stock("AAPL");
        read_now(&reg, &c, SubscriptionKind::Trade, Some(60_000), TAIL, 0, 0)
            .expect("nothing to refuse");
        reg.ingest(trade(&c, 1.0, 0));
        reg.prints(&c, 10, 0, 1).expect("nothing to refuse");
        reg.commit_prints_read(&c, 1, 0, 0);

        reg.gap(1);

        let r = read_now(&reg, &c, SubscriptionKind::Trade, Some(60_000), TAIL, 0, 2)
            .expect("nothing to refuse");
        assert_eq!(
            (r.feed_dropped_since_last_read, r.clipped),
            (0, true),
            "the feed discarded nothing; the rows are missing because they never came"
        );
        // Asked again for the same window, the answer is the same: it still
        // reaches back over the interval the rows are missing from.
        let again = read_now(&reg, &c, SubscriptionKind::Trade, Some(60_000), TAIL, 0, 3)
            .expect("nothing to refuse");
        assert!(
            again.clipped,
            "a named window answers the same way every time it is asked"
        );
        // Nothing has arrived since the interruption, so no window ending
        // now is proven whole, however short.
        let unproven = read_now(&reg, &c, SubscriptionKind::Trade, Some(1), TAIL, 0, 30_000)
            .expect("nothing to refuse");
        assert!(
            unproven.clipped,
            "no row since the interruption, so nothing shows the feed is back"
        );
        // A row is that proof, and dates the end of the loss. Once the
        // window no longer reaches back that far, it is whole.
        reg.ingest(trade(&c, 1.0, 40_000 * MS));
        let moved_on = read_now(&reg, &c, SubscriptionKind::Trade, Some(1), TAIL, 0, 60_000)
            .expect("nothing to refuse");
        assert!(!moved_on.clipped, "the loss is behind this window");

        // The print list loses rows the same way, and a read that never
        // reached the caller must not consume the disclosure.
        reg.gap(1);
        let p = reg.prints(&c, 10, 0, 4).expect("nothing to refuse");
        assert!(p.gap, "the interruption is reported");
        let again = reg.prints(&c, 10, 0, 5).expect("nothing to refuse");
        assert!(again.gap, "uncommitted, so it is still owed");
        reg.commit_prints_read(&c, again.received, again.feed_drops_seen, again.gaps_seen);
        let settled = reg.prints(&c, 10, 0, 6).expect("nothing to refuse");
        assert!(!settled.gap, "committed, so it is discharged");
    }

    #[test]
    fn a_replaced_session_counts_its_discards_from_nothing() {
        // The SDK's discard count belongs to one session; a new one starts
        // at zero. A cursor left at the old count would read clean until
        // the new count passed it, on the one path where the feed was just
        // unhealthy.
        let reg = Registry::default();
        let c = stock("AAPL");
        let call = option("550", "C");
        read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 5_000, 0)
            .expect("nothing to refuse");
        reg.prints(&c, 10, 5_000, 1).expect("nothing to refuse");
        market_now(&reg, SecType::Option, query(1), 5_000, 2).expect("nothing to refuse");

        reg.restarted();
        let r = read_now(&reg, &c, SubscriptionKind::Trade, None, TAIL, 300, 3)
            .expect("nothing to refuse");
        assert_eq!(
            (r.feed_dropped_since_last_read, r.clipped),
            (300, true),
            "everything the new session discarded is new to the buffer"
        );
        assert_eq!(
            reg.prints(&c, 10, 300, 4)
                .expect("nothing to refuse")
                .feed_dropped_since_last_read,
            300
        );
        reg.ingest(trade(&call, 1.0, 4 * MS));
        let m = market_now(&reg, SecType::Option, query(1), 300, 5).expect("nothing to refuse");
        assert_eq!(m.feed_dropped_since_last_read, 300);
    }

    #[test]
    fn a_selection_keeps_at_least_one_row() {
        // `Selection::offer` evicts the oldest kept row once `limit` are
        // held, so zero would evict from nothing. Quietly keeping one
        // instead would answer a question the caller did not ask, so the
        // request is refused and says why.
        let why = refused(parse_market_query(&json!({"limit": 0}), 0));
        assert!(
            why.contains("at least one row"),
            "says what it needs: {why}"
        );
        assert_eq!(
            parse_market_query(&json!({}), 7)
                .expect("a valid query")
                .limit,
            7
        );
    }

    #[test]
    fn a_range_with_a_non_numeric_bound_is_refused_not_repaired() {
        // Dropping the bad entry would leave [90, 110]: a valid predicate the
        // caller never asked for, installed silently over the one they did.
        for bad in [
            json!([90, "ignored", 110]),
            json!([90, null, 110]),
            json!(["90", "110"]),
        ] {
            let why = refused(parse_clauses(
                &json!([{"field": "price", "op": "inside", "value": bad}]),
                &FIELDS,
            ));
            assert!(
                why.contains("[low, high]"),
                "refused and says the shape: {why}"
            );
        }
        // The well-formed pair still parses.
        assert!(parse_clauses(
            &json!([{"field": "price", "op": "inside", "value": [90, 110]}]),
            &FIELDS,
        )
        .is_ok());
    }

    #[test]
    fn a_predicate_is_bounded() {
        let clause = json!({"field": "size", "op": ">", "value": 1});
        let of = |n: usize| Value::from(vec![clause.clone(); n]);
        assert!(parse_clauses(&of(MAX_CLAUSES), &FIELDS).is_ok());
        let why = refused(parse_clauses(&of(MAX_CLAUSES + 1), &FIELDS));
        assert!(
            why.contains(&format!("at most {MAX_CLAUSES}")),
            "names the bound: {why}"
        );
    }

    #[test]
    fn a_failed_prints_call_leaves_its_prints_new() {
        let reg = Registry::default();
        let c = stock("AAPL");
        prints(&reg, &c, 10, 0);
        for i in 1..=3 {
            reg.ingest(trade(&c, i as f64, i * MS));
        }
        // A call that fails after the read commits nothing, so the prints
        // it saw are new again for the next one.
        let p = reg.prints(&c, 10, 0, 1).expect("nothing to refuse");
        assert_eq!(p.new_since_last_read, 3);
        let again = reg.prints(&c, 10, 0, 2).expect("nothing to refuse");
        assert_eq!(
            again.new_since_last_read, 3,
            "not consumed by a call that failed"
        );
        reg.commit_prints_read(&c, p.received, p.feed_drops_seen, p.gaps_seen);
        let (after, _) = prints(&reg, &c, 10, 3);
        assert_eq!(
            after.new_since_last_read, 0,
            "consumed by the call that was answered"
        );
    }

    #[test]
    fn coverage_is_the_oldest_row_held_and_clipped_says_when_the_window_reaches_past_it() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        // One row per millisecond, ten more than the ring holds.
        for ms in 1..=(RING as u64 + 10) {
            reg.ingest(trade(&c, ms as f64, ms * MS));
        }
        let now = RING as u64 + 20;
        let oldest_held = 11;

        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(now), TAIL, now);
        assert_eq!(r.dropped, 10);
        assert_eq!(
            r.covered_since_ms, oldest_held,
            "once rows fell off, coverage starts at the oldest row held, not at the subscription"
        );
        assert!(
            r.clipped,
            "the window asked for the whole life; the ring does not reach it"
        );
        assert_eq!(r.count, RING as u64, "the summary saw the whole ring");

        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(20), TAIL, now);
        assert_eq!(
            r.count, 11,
            "rows from 4096 to 4106 lie inside the last 20 ms"
        );
        assert!(!r.clipped, "a window inside coverage is whole");

        // A buffer younger than the window is clipped too, with nothing dropped.
        let young = stock("MSFT");
        read(&reg, &young, SubscriptionKind::Trade, None, TAIL, now);
        reg.ingest(trade(&young, 1.0, (now + 1) * MS));
        let (r, _) = read(
            &reg,
            &young,
            SubscriptionKind::Trade,
            Some(60_000),
            TAIL,
            now + 2,
        );
        assert_eq!(r.dropped, 0);
        assert_eq!(
            r.covered_since_ms, now,
            "nothing dropped, so coverage starts where the buffer opened, not at its first row"
        );
        assert!(r.clipped, "two milliseconds of life cannot cover a minute");
    }

    #[test]
    fn a_print_takes_the_quote_before_it_and_exactly_the_two_after() {
        let reg = Registry::default();
        let c = stock("AAPL");
        // A prints call opens both legs; the correlation is per contract.
        let (p, _) = prints(&reg, &c, 10, 0);
        assert_eq!(
            p.opened,
            vec![SubscriptionKind::Trade, SubscriptionKind::Quote],
            "a print needs both legs open"
        );
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 0));
        reg.ingest(quote(&c, 1.02, 1.12));
        reg.ingest(quote(&c, 1.03, 1.13));
        reg.ingest(quote(&c, 1.04, 1.14));

        let (p, _) = prints(&reg, &c, 10, 1);
        assert!(p.opened.is_empty(), "already held");
        assert_eq!(p.rows.len(), 1);
        assert!(p.rows[0].quote_before.is_some());
        assert_eq!(
            p.rows[0].quotes_after.len(),
            2,
            "the feed sends two after a print; a third belongs to the next one"
        );
    }

    #[test]
    fn a_second_trade_does_not_starve_the_first_of_its_quotes() {
        // Q0 T1 Q1 T2 Q2 Q3. Each trade takes the next two quotes, so Q2
        // belongs to both tails. Filling only the newest print leaves T1 with
        // one quote forever, which on a liquid contract is most prints.
        let reg = Registry::default();
        let c = stock("AAPL");
        prints(&reg, &c, 10, 0);

        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 1));
        reg.ingest(quote(&c, 1.01, 1.11));
        reg.ingest(trade(&c, 1.09, 2));
        reg.ingest(quote(&c, 1.02, 1.12));
        reg.ingest(quote(&c, 1.03, 1.13));

        let (p, _) = prints(&reg, &c, 10, 1);
        assert_eq!(p.rows.len(), 2);

        let bids = |p: &Print| {
            p.quotes_after
                .iter()
                .map(|q| match q {
                    StreamData::Quote { bid, .. } => *bid,
                    _ => f64::NAN,
                })
                .collect::<Vec<_>>()
        };
        // Identity, not count: duplicating one quote twice would satisfy a
        // length check and be wrong.
        assert_eq!(bids(&p.rows[0]), vec![1.01, 1.02], "first print");
        assert_eq!(bids(&p.rows[1]), vec![1.02, 1.03], "second print");
    }

    #[test]
    fn prints_count_what_is_new_and_say_when_older_ones_were_discarded() {
        let reg = Registry::default();
        let c = stock("AAPL");
        prints(&reg, &c, 10, 1_000);
        for i in 1..=(PRINTS as u64 + 3) {
            reg.ingest(trade(&c, i as f64, (1_000 + i) * MS));
        }
        let (p, _) = prints(&reg, &c, 5, 2_000);
        assert_eq!(p.rows.len(), 5);
        assert_eq!(p.held, PRINTS);
        assert_eq!(p.dropped, 3, "three fell off the front");
        assert_eq!(
            p.covered_since_ms, 1_004,
            "so coverage starts at the oldest print held"
        );
        assert_eq!(
            p.new_since_last_read,
            PRINTS as u64 + 3,
            "every print arrived after the read at 1000, the three discarded included"
        );
        assert_eq!(
            p.rows.last().map(|p| price(&p.trade)),
            Some(PRINTS as f64 + 3.0),
            "newest last"
        );
        let (p, _) = prints(&reg, &c, 5, 3_000);
        assert_eq!(
            p.new_since_last_read, 0,
            "nothing printed since the read at 2000"
        );
    }

    #[test]
    fn an_expired_quote_leg_does_not_erase_the_prints_the_trade_leg_still_serves() {
        let reg = Registry::default();
        let c = stock("AAPL");
        prints(&reg, &c, 10, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 0));

        // The trade leg is read inside its window; the quote leg is not.
        let (r, expired) = read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            TTL.as_millis() as u64,
        );
        assert!(
            !r.first && expired.is_empty(),
            "both still inside the window"
        );
        let (_, expired) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert_eq!(
            expired,
            vec![c.quote()],
            "the idle quote leg is closed and handed back to unsubscribe"
        );

        let (p, _) = prints(&reg, &c, 10, PAST_TTL + 1);
        assert_eq!(
            p.opened,
            vec![SubscriptionKind::Quote],
            "reopens only the missing leg"
        );
        assert_eq!(
            p.rows.len(),
            1,
            "history survives the loss of the other leg"
        );
    }

    #[test]
    fn an_idle_buffer_is_buried_before_the_read_and_reopened() {
        // The sweep runs first, so a buffer that outlived its window is not
        // revived by the read that noticed: the read gets a fresh one and the
        // old subscription comes back to be closed.
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&c, 1.0, 0));

        let (r, expired) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert!(r.first, "a fresh buffer");
        assert_eq!(expired, vec![c.trade()]);
        assert_eq!(r.count, 0, "and nothing carried over");
    }

    #[test]
    fn stopping_a_contract_closes_each_kind_on_the_feed_before_the_buffer_goes() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        read(&reg, &stock("MSFT"), SubscriptionKind::Trade, None, TAIL, 0);

        let held = reg.held_for(&c);
        assert_eq!(held.len(), 2, "both legs come back to be unsubscribed");
        // The feed lets one go and keeps the other: only the first is dropped.
        reg.forget(&held[0]);
        assert_eq!(
            reg.held_for(&c),
            vec![held[1].clone()],
            "the leg the feed kept is still held, for the next stop to retry"
        );
        reg.forget(&held[1]);
        let rows = reg.list();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].sub,
            stock("MSFT").trade(),
            "the other contract is untouched"
        );
        assert!(reg.held_for(&c).is_empty(), "stopping again finds nothing");
    }

    #[test]
    fn a_failed_subscribe_is_forgotten_so_the_next_read_tries_again() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.forget(&c.trade());
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1);
        assert!(r.first, "a buffer the feed refused must not look held");
    }

    #[test]
    fn a_buffer_the_feed_dropped_is_released_with_the_feeds_last_word_on_why() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        reg.note_rejection(StreamResponseType::MaxStreamsReached, 500);

        let why = format!("{:?}", dropped_by_feed(&reg, &c.quote(), 1_700));
        assert!(
            why.contains("MAX_STREAMS_REACHED, 1.2 s ago"),
            "names the code and how long ago: {why}"
        );
        assert!(
            why.contains("too many contracts") && why.contains("INVALID_PERMS"),
            "carries the vendor's meaning and the other refusal it could be: {why}"
        );
        let (r, _) = read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 1_701);
        assert!(
            r.first,
            "the buffer is released, so the next read re-subscribes"
        );

        let fresh = Registry::default();
        read(&fresh, &c, SubscriptionKind::Quote, None, TAIL, 0);
        let why = format!("{:?}", dropped_by_feed(&fresh, &c.quote(), 1));
        assert!(
            !why.contains("most recent rejection"),
            "no rejection seen, none claimed: {why}"
        );
    }

    #[test]
    fn a_subscription_the_feed_kept_after_its_sweep_is_handed_back_to_close_again() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        let (_, expired) = read(
            &reg,
            &stock("MSFT"),
            SubscriptionKind::Trade,
            None,
            TAIL,
            PAST_TTL,
        );
        assert_eq!(expired, vec![c.trade()], "swept");

        // The unsubscribe did not land, so the reference is kept...
        reg.reinstate(&c.trade(), PAST_TTL);
        let (per_contract, _) = reg.subscriptions();
        assert!(
            per_contract.contains(&(SubscriptionKind::Trade, c.clone())),
            "a fresh session would still have to close it"
        );
        // ...and the very next sweep, whenever it runs, offers it again.
        let (_, expired) = read(
            &reg,
            &stock("MSFT"),
            SubscriptionKind::Trade,
            None,
            TAIL,
            PAST_TTL + 1,
        );
        assert_eq!(expired, vec![c.trade()], "retried");

        // A buffer someone reopened meanwhile is not reset to expired.
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, PAST_TTL + 2);
        reg.reinstate(&c.trade(), PAST_TTL + 3);
        let (_, expired) = read(
            &reg,
            &stock("MSFT"),
            SubscriptionKind::Trade,
            None,
            TAIL,
            PAST_TTL + 4,
        );
        assert!(expired.is_empty(), "the live buffer keeps its read time");

        // The same for a market buffer.
        reg.reinstate(&SecType::Option.full_trades(), PAST_TTL + 5);
        let (_, expired) = read(
            &reg,
            &stock("MSFT"),
            SubscriptionKind::Trade,
            None,
            TAIL,
            PAST_TTL + 6,
        );
        assert_eq!(expired, vec![SecType::Option.full_trades()]);
    }

    #[test]
    fn a_fresh_session_reopens_each_held_subscription_once() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        market_now(&reg, SecType::Option, query(1), 0, 0).expect("nothing to refuse");

        let (mut subs, full) = reg.subscriptions();
        subs.sort_by_key(|(k, _)| k.kind_str());
        assert_eq!(
            subs,
            vec![
                (SubscriptionKind::Quote, c.clone()),
                (SubscriptionKind::Trade, c)
            ],
            "two reads share one subscription; replaying it twice would double-subscribe"
        );
        assert_eq!(
            full,
            vec![(SubscriptionKind::Trade, SecType::Option)],
            "the market buffer restores as the SDK's full-stream tuple"
        );
    }

    #[test]
    fn an_index_has_no_quote_stream_and_defaults_to_its_price() {
        // The vendor offers indices price (on the trade subscription) and
        // market value. Offering a quote would invent a product.
        assert!(resolve_kind(SecType::Index, Some("quote")).is_err());
        assert_eq!(
            resolve_kind(SecType::Index, None).ok(),
            Some(SubscriptionKind::Trade)
        );
        assert_eq!(
            resolve_kind(SecType::Stock, None).ok(),
            Some(SubscriptionKind::Quote)
        );
        assert!(resolve_kind(SecType::Index, Some("market_value")).is_ok());

        let why = format!(
            "{:?}",
            resolve_kind(SecType::Index, Some("quote")).unwrap_err()
        );
        assert!(
            why.contains("trade"),
            "the refusal names what is on offer: {why}"
        );
    }

    #[test]
    fn a_tail_larger_than_the_cap_is_clamped() {
        // Rows are copied while the registry lock is held and the dispatcher
        // needs that lock to record a tick, so an uncapped tail lets one
        // request stall the feed for as long as cloning the ring takes.
        let reg = Registry::default();
        let c = stock("AAPL");
        // Open the buffer first: ingest drops a tick for a contract nobody
        // holds, so ingesting before the read would leave an empty tail and
        // an assertion that cannot fail.
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        for i in 0..(TAIL_MAX + 40) {
            reg.ingest(trade(&c, 1.0 + i as f64, i as u64));
        }

        let asked = TAIL_MAX + 40;
        let (uncapped, _) = read(&reg, &c, SubscriptionKind::Trade, Some(600_000), asked, 1);
        assert_eq!(
            uncapped.tail.len(),
            asked,
            "the registry serves what it is asked for; the bound belongs to the tool"
        );

        assert_eq!(tail_rows(asked), TAIL_MAX, "the tool clamps it");
        assert_eq!(tail_rows(3), 3, "and leaves a modest request alone");

        let (capped, _) = read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            Some(600_000),
            tail_rows(asked),
            2,
        );
        assert_eq!(capped.tail.len(), TAIL_MAX);
    }

    #[test]
    fn the_vendors_bar_is_kept_for_a_held_contract_and_served_as_sent() {
        let reg = Registry::default();
        let c = stock("AAPL");
        // The trade leg is what the vendor broadcasts a bar ahead of, so it
        // is held here; the point of the test is that a quote read still gets
        // the bar.
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        reg.ingest(bar(&c, 1.5, 10 * MS));
        reg.ingest(bar(&c, 1.7, 20 * MS));
        reg.ingest(bar(&stock("MSFT"), 9.0, 20 * MS));

        let (r, _) = read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 30);
        let close = match &r.ohlcvc {
            Some(StreamData::Ohlcvc { close, .. }) => *close,
            other => panic!("expected the vendor's bar, got {other:?}"),
        };
        assert_eq!(
            close, 1.7,
            "the newest bar the vendor sent, whichever kind is read"
        );
        assert_eq!(r.count, 0, "a bar is not a row on the quote buffer");
        assert!(
            !reg.lock().contracts.contains_key(&stock("MSFT")),
            "a bar for an unheld contract creates nothing"
        );
        let cols: Vec<&str> = fields(&bar(&c, 1.0, 0)).iter().map(|(k, _)| *k).collect();
        assert_eq!(
            cols,
            ["time", "open", "high", "low", "close", "volume", "count"],
            "served with the vendor's own fields"
        );
    }

    #[test]
    fn a_row_for_an_unheld_buffer_creates_nothing() {
        let reg = Registry::default();
        read(&reg, &stock("AAPL"), SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&stock("MSFT"), 5.0, 0));
        reg.ingest(quote(&stock("AAPL"), 1.0, 1.1));
        let held = reg.lock();
        assert_eq!(held.contracts.len(), 1, "no buffer invented for MSFT");
        assert_eq!(
            held.contracts[&stock("AAPL")].buffers.len(),
            1,
            "nor for an unread kind"
        );
    }

    #[test]
    fn a_row_renders_its_declared_columns_and_a_clock_time() {
        let d = trade_with(&stock("AAPL"), 150.25, 37, 0);
        let cols: Vec<&str> = fields(&d).iter().map(|(k, _)| *k).collect();
        assert_eq!(
            cols,
            ["time", "price", "size", "condition", "exchange", "sequence"]
        );
        // `columns` and the positional row are both projections of `fields`,
        // so they cannot disagree and nothing here would prove it if they
        // could. What can be wrong is the order `fields` renders, which the
        // literal above states.
        assert_eq!(clock(34_200_000), "09:30:00.000");
        assert_eq!(clock(57_600_123), "16:00:00.123");
        assert_eq!(
            object(&d).get("time").and_then(|v: &Value| v.as_str()),
            Some("09:30:00.000")
        );
    }

    #[test]
    fn every_column_a_print_renders_is_one_a_selection_can_read() {
        // A column the model sees in a row must be one it can narrow or rank
        // on, and a name the schema offers must read something on the rows it
        // is offered for. A selection reads prints, and a print is a trade
        // and the quote before it.
        let c = stock("AAPL");
        let (t, q) = (trade(&c, 1.0, 0), quote(&c, 1.0, 1.1));
        for d in [&t, &q] {
            for (col, _) in fields(d) {
                if col == "time" {
                    // The one rendered column a selection cannot read: a
                    // clock string, not a number. Skipped with its reason
                    // asserted, so the day it becomes selectable this fails
                    // rather than quietly passing over it.
                    assert!(
                        !PRINT_FIELDS.contains(&col),
                        "time is excluded on purpose; the schema now offers it"
                    );
                    assert!(
                        print_field(&t, Some(&q), col).is_none(),
                        "and it reads no number off a print"
                    );
                    continue;
                }
                assert!(
                    PRINT_FIELDS.contains(&col),
                    "{col} is rendered on a print but cannot be selected on"
                );
                assert!(
                    print_field(&t, Some(&q), col).is_some(),
                    "{col} reads nothing on the print it is rendered from"
                );
            }
        }
        for name in PRINT_FIELDS {
            assert!(
                print_field(&t, Some(&q), name).is_some(),
                "{name} is offered and reads nothing"
            );
        }
        assert_eq!(
            field_of(&quote(&c, 1.0, 1.25), "spread"),
            Some(0.25),
            "spread is ask minus bid"
        );

        // FIELDS is wider than that on purpose: a name belonging to some
        // other row is refused as that rather than as no field at all.
        let why = refused(parse_market_query(&json!({"rank_by": "open_interest"}), 1));
        assert!(
            why.contains("open_interest is not a field these rows carry"),
            "{why}"
        );
        assert!(
            FIELDS.contains(&"open_interest") && !PRINT_FIELDS.contains(&"open_interest"),
            "which is what that wording depends on"
        );
        let plain = refused(parse_market_query(&json!({"rank_by": "nonsense"}), 1));
        assert!(
            !plain.contains("not a field these rows carry") && plain.contains("rank_by must be"),
            "a name that is no field anywhere just names the list: {plain}"
        );
        assert!(parse_market_query(
            &json!({"where": [{"field": "market_price", "op": ">", "value": 1}]}),
            1
        )
        .is_err());
    }

    #[test]
    fn a_clause_compares_a_field_to_a_number_a_field_or_a_range() {
        let c = stock("AAPL");
        fn get(q: &StreamData) -> impl Fn(&str) -> Option<f64> + '_ {
            move |f| field_of(q, f)
        }

        let wide = clauses(json!([{"field": "spread", "op": ">", "value": 0.1}]));
        assert!(holds_all(&wide, &get(&quote(&c, 1.0, 1.2))));
        assert!(!holds_all(&wide, &get(&quote(&c, 1.0, 1.05))));

        let crossed = clauses(json!([{"field": "bid", "op": ">=", "value": "ask"}]));
        assert!(holds_all(&crossed, &get(&quote(&c, 1.1, 1.0))));
        assert!(!holds_all(&crossed, &get(&quote(&c, 1.0, 1.1))));

        let outside = clauses(json!([{"field": "price", "op": "outside", "value": [90, 110]}]));
        assert!(holds_all(&outside, &get(&trade(&c, 120.0, 0))));
        assert!(!holds_all(&outside, &get(&trade(&c, 100.0, 0))));
        let inside = clauses(json!([{"field": "price", "op": "inside", "value": [90, 110]}]));
        assert!(
            holds_all(&inside, &get(&trade(&c, 110.0, 0))),
            "bounds are inclusive"
        );

        // Every clause must hold, and a row without the field never does.
        let both = clauses(json!([
            {"field": "size", "op": ">", "value": 0},
            {"field": "price", "op": "!=", "value": 5}
        ]));
        assert!(holds_all(&both, &get(&trade(&c, 6.0, 0))));
        assert!(!holds_all(&both, &get(&trade(&c, 5.0, 0))));
        assert!(
            !holds_all(&wide, &get(&trade(&c, 6.0, 0))),
            "a trade has no spread"
        );

        for bad in [
            json!([{"field": "vega", "op": ">", "value": 1}]),
            json!([{"field": "bid", "op": "~", "value": 1}]),
            json!([{"field": "bid", "op": ">", "value": "vega"}]),
            json!([{"field": "bid", "op": "inside", "value": [1]}]),
            json!({"field": "bid"}),
        ] {
            assert!(parse_clauses(&bad, &FIELDS).is_err(), "accepted {bad}");
        }
        let round_trip: Vec<Value> = [&wide[0], &crossed[0], &outside[0]]
            .into_iter()
            .map(clause_json)
            .collect();
        assert_eq!(
            round_trip,
            vec![
                json!({"field": "spread", "op": ">", "value": 0.1}),
                json!({"field": "bid", "op": ">=", "value": "ask"}),
                json!({"field": "price", "op": "outside", "value": [90.0, 110.0]})
            ],
            "a clause echoes back as it was given"
        );
    }

    #[test]
    fn the_market_buffer_takes_every_trade_on_its_security_type_with_the_quote_sent_ahead() {
        let reg = Registry::default();
        let call = option("550", "C");
        let put = option("540", "P");
        let m = market_now(&reg, SecType::Option, query(10), 0, 0).expect("nothing to refuse");
        assert!(m.first, "the first look opens it");

        // The vendor sends the contract's NBBO and bar, then its trade.
        reg.ingest(quote(&call, 1.0, 1.1));
        reg.ingest(bar(&call, 1.05, MS));
        reg.ingest(trade(&call, 1.05, MS));
        // Another contract's quote in between: the trade does not claim it.
        reg.ingest(quote(&put, 2.0, 2.1));
        reg.ingest(trade(&call, 1.06, 2 * MS));
        // A stock trade belongs to a market buffer this registry does not hold.
        reg.ingest(trade(&stock("AAPL"), 150.0, 3 * MS));

        let m = market_now(&reg, SecType::Option, query(10), 0, 10).expect("nothing to refuse");
        assert!(!m.first);
        assert_eq!(m.received, 2, "two option trades, no stock");
        assert_eq!(m.new_since_last_read, 2);
        assert_eq!((m.examined, m.matched, m.rows.len()), (2, 2, 2));
        assert_eq!(
            m.rows.iter().map(|p| price(&p.trade)).collect::<Vec<_>>(),
            vec![1.05, 1.06],
            "oldest first without a rank"
        );
        assert_eq!(
            m.rows[0]
                .quote_before
                .as_ref()
                .and_then(|q| field_of(q, "bid")),
            Some(1.0),
            "the NBBO sent ahead of the trade"
        );
        assert!(
            m.rows[1].quote_before.is_none(),
            "a quote for another contract is not this trade's"
        );
        assert!(
            reg.lock().contracts.is_empty(),
            "a market row opens no per-contract buffer"
        );

        // A read takes what was kept: the next has nothing until more prints.
        let m = market_now(&reg, SecType::Option, query(10), 0, 11).expect("nothing to refuse");
        assert_eq!((m.examined, m.matched, m.rows.len()), (0, 0, 0));
        assert_eq!(
            (m.received, m.new_since_last_read),
            (2, 0),
            "received counts every print since the market opened; new only those since the last read"
        );

        // Only `limit` rows are kept, the newest, and the counts still say
        // how many there were.
        market_now(&reg, SecType::Option, query(2), 0, 12).expect("nothing to refuse");
        for i in 0..5 {
            reg.ingest(trade(&call, i as f64, (20 + i) * MS));
        }
        let m = market_now(&reg, SecType::Option, query(2), 0, 30).expect("nothing to refuse");
        assert_eq!((m.examined, m.matched), (5, 5));
        assert_eq!(
            m.rows.iter().map(|p| price(&p.trade)).collect::<Vec<_>>(),
            vec![3.0, 4.0]
        );
    }

    #[test]
    fn a_market_selection_filters_and_ranks_every_print_as_it_arrives() {
        let reg = Registry::default();
        let c540 = option("540", "C");
        let c550 = option("550", "C");
        let p550 = option("550", "P");
        let qqq = Contract::option(
            "QQQ",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "C",
            },
        )
        .expect("a valid contract");
        // Rows arrive after the selection is installed, as they do on a
        // feed: a buffer cannot be shown prints from before it existed.
        let feed = |reg: &Registry, t: u64| {
            reg.ingest(quote(&c550, 1.0, 1.5));
            reg.ingest(trade_sized(&c550, 1.2, 10, 0, (t + 1) * MS));
            reg.ingest(trade_sized(&c540, 5.0, 300, 0, (t + 2) * MS));
            reg.ingest(trade_sized(&p550, 0.9, 200, 0, (t + 3) * MS));
            reg.ingest(trade_sized(&c550, 1.3, 50, 0, (t + 4) * MS));
            reg.ingest(trade_sized(&qqq, 2.0, 999, 0, (t + 5) * MS));
        };
        // Install a selection, let the five prints through it, read it back.
        let run = |q: MarketQuery, t: u64| {
            market_now(&reg, SecType::Option, q.clone(), 0, t).expect("nothing to refuse");
            feed(&reg, t);
            market_now(&reg, SecType::Option, q, 0, t + 1).expect("nothing to refuse")
        };
        let sizes = |m: &MarketReading| {
            m.rows
                .iter()
                .map(|p| field_of(&p.trade, "size"))
                .collect::<Vec<_>>()
        };

        // Contract attributes: root, right, strike range.
        let mut q = query(10);
        q.root = Some("spy".into());
        q.is_call = Some(true);
        q.strike_min = Some(545.0);
        q.strike_max = Some(555.0);
        let m = run(q, 10);
        assert_eq!(
            (m.examined, m.matched),
            (5, 2),
            "population, then selection"
        );
        assert_eq!(
            sizes(&m),
            vec![Some(10.0), Some(50.0)],
            "SPY 550 calls, oldest first"
        );

        // Vendor fields, on the trade and on the quote before it.
        let mut q = query(10);
        q.clauses = clauses(json!([{"field": "spread", "op": ">=", "value": 0.5}]));
        let m = run(q, 20);
        assert_eq!(
            sizes(&m),
            vec![Some(10.0)],
            "only the print with a quote ahead of it has a spread"
        );

        // Rank, direction, cut.
        let mut q = query(2);
        q.rank_by = Some("size".into());
        let m = run(q.clone(), 30);
        assert_eq!(
            (m.matched, m.rows.len()),
            (5, 2),
            "matched says what the cut hid"
        );
        assert_eq!(sizes(&m), vec![Some(999.0), Some(300.0)], "largest first");
        q.ascending = true;
        let m = run(q, 40);
        assert_eq!(sizes(&m), vec![Some(10.0), Some(50.0)]);

        // A match without the rank field cannot be placed, and is counted.
        let mut q = query(5);
        q.rank_by = Some("bid".into());
        let m = run(q.clone(), 50);
        assert_eq!((m.matched, m.unranked), (5, Some(4)));
        assert_eq!(sizes(&m), vec![Some(10.0)], "the one print with a bid");
        // Disclosed by the selection that set them aside, even when the read
        // that collects them no longer ranks.
        market_now(&reg, SecType::Option, q, 0, 52).expect("nothing to refuse");
        feed(&reg, 52);
        let m = market_now(&reg, SecType::Option, query(5), 0, 53).expect("nothing to refuse");
        assert_eq!((m.unranked, m.selected_by.is_some()), (Some(4), true));
        let m = market_now(&reg, SecType::Option, query(5), 0, 54).expect("nothing to refuse");
        assert_eq!(m.unranked, None, "an unranked selection sets nothing aside");

        let mut q = query(10);
        q.expiration = Some(20260620);
        assert_eq!(run(q.clone(), 60).matched, 5);
        q.expiration = Some(20260621);
        assert_eq!(run(q, 70).matched, 0);

        // Changing the selection: what comes back was kept by the old one,
        // and says so.
        let mut spy = query(10);
        spy.root = Some("SPY".into());
        let mut nasdaq = query(10);
        nasdaq.root = Some("QQQ".into());
        market_now(&reg, SecType::Option, spy.clone(), 0, 80).expect("nothing to refuse");
        feed(&reg, 80);
        let m =
            market_now(&reg, SecType::Option, nasdaq.clone(), 0, 81).expect("nothing to refuse");
        assert_eq!(
            m.rows.len(),
            4,
            "the SPY prints, kept under the SPY selection"
        );
        assert_eq!(m.selected_by, Some(spy));
        feed(&reg, 81);
        let m = market_now(&reg, SecType::Option, nasdaq, 0, 82).expect("nothing to refuse");
        assert_eq!((m.rows.len(), m.selected_by), (1, None));
    }

    #[test]
    fn a_market_buffer_and_a_per_contract_buffer_on_the_same_class_are_not_held_together() {
        let reg = Registry::default();
        let c = option("550", "C");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        let why = refused(market_now(&reg, SecType::Option, query(1), 0, 1));
        assert!(
            why.contains("1 OPTION contract"),
            "names the conflict: {why}"
        );
        assert!(!reg.market_held(SecType::Option), "and opens nothing");
        market_now(&reg, SecType::Stock, query(1), 0, 2).expect("another class is no conflict");

        reg.forget(&c.trade());
        market_now(&reg, SecType::Option, query(1), 0, 3).expect("nothing to refuse");
        let why = refused(read_now(
            &reg,
            &c,
            SubscriptionKind::Quote,
            None,
            TAIL,
            0,
            4,
        ));
        assert!(why.contains("live_market"), "names the other side: {why}");
        assert!(
            reg.prints(&c, 1, 0, 5).is_err(),
            "a print needs both doubled legs"
        );
        read(&reg, &c, SubscriptionKind::OpenInterest, None, TAIL, 6);
        read(
            &reg,
            &Contract::index("SPX"),
            SubscriptionKind::Trade,
            None,
            TAIL,
            7,
        );
        assert!(
            reg.lock()
                .contracts
                .get(&c)
                .is_some_and(|s| s.buffers.len() == 1),
            "kinds the full stream does not carry are held alongside it, as is another class"
        );
    }

    #[test]
    fn an_idle_market_buffer_is_swept_and_stopped_by_its_class() {
        let reg = Registry::default();
        market_now(&reg, SecType::Stock, query(1), 0, 0).expect("nothing to refuse");
        let spx = Contract::index("SPX");
        let (_, expired) = read(&reg, &spx, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert_eq!(expired, vec![SecType::Stock.full_trades()]);
        assert!(!reg.market_held(SecType::Stock));

        market_now(&reg, SecType::Stock, query(1), 0, PAST_TTL).expect("nothing to refuse");
        assert!(reg.market_held(SecType::Stock));
        reg.forget(&SecType::Stock.full_trades());
        assert!(!reg.market_held(SecType::Stock), "stopped");
    }

    #[test]
    fn the_market_stream_is_refused_in_words_below_the_pro_tier() {
        assert!(pro_required(SecType::Option, Some(SubscriptionTier::Pro)).is_ok());
        let why = refused(pro_required(
            SecType::Option,
            Some(SubscriptionTier::Standard),
        ));
        assert!(
            why.contains("Options Pro")
                && why.contains("tier is Standard")
                && why.contains("live_read"),
            "names the tier needed, the tier held and what still works: {why}"
        );
        let why = refused(pro_required(SecType::Stock, None));
        assert!(
            why.contains("Stocks Pro") && why.contains("not reported"),
            "{why}"
        );
        let why = refused(pro_required(SecType::Index, Some(SubscriptionTier::Pro)));
        assert!(
            why.contains("per contract"),
            "an index has no whole-market stream at any tier: {why}"
        );
    }
}
