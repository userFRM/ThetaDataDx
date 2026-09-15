//! Live-state views over the streaming feed.
//!
//! A model cannot read a feed: ticks arrive orders of magnitude faster than
//! tokens. What works is holding the subscription and answering questions
//! about it — what is the value now, what did the window look like, what
//! printed and where the market was around it.
//!
//! A book is named by its contract and kind, the same way the vendor names a
//! subscription. The first read opens it; a read every so often keeps it;
//! fifteen idle minutes or `tape_stop` close it. There is no handle to
//! mint, pass back or lose. A whole-market book is named by its security
//! type alone, the way the vendor names a full-stream subscription.
//!
//! The feed answers a subscribe after accepting it, and can refuse it then.
//! The SDK's active set is the only record of what the feed kept, so every
//! read checks the book it is about against that set rather than trusting
//! the subscribe that opened it, and every close keeps the book until the
//! feed has let go.
//!
//! This layer stores what the feed sends and serves it back. The summary on
//! a read counts and sorts the rows it saw and says which population it saw
//! them in; it does not aggregate. Bar construction has condition, cancel and
//! size rules that are the caller's to choose, and a bar built on assumptions
//! here would not reconcile with one built anywhere else. A predicate left
//! on a book compares the vendor's own fields and keeps the rows that pass,
//! so what crosses a line between two reads is not lost with the ring.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonic_rs::{json, JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, FullSubscriptionKind, OptionLeg, SecTypeExt, StreamControl, StreamData, StreamEvent,
    Subscription, SubscriptionKind,
};
use thetadatadx::{
    Client, ConnectionStatus, SecType, StreamMsgType, StreamResponseType, SubscriptionTier,
};

use crate::{sanitize_error, ToolError};

/// Rows retained per book. A busy contract prints far more than this in a
/// session; the tail is what a model can act on and the rest is weight.
const RING: usize = 4_096;
/// Prints retained per contract. Fewer than rows: each one holds up to four
/// messages.
const PRINTS: usize = 256;
/// Prints retained per whole-market book. Every trade on the security type
/// lands here, so the window is short in time however deep it is in rows;
/// `covers_seconds` says how far back it reaches at the moment of a read.
const MARKET_RING: usize = 16_384;
/// Rows a predicate may keep on a book. A predicate that matches more than
/// this between two reads is a line the market crossed for good, and the
/// newest matches say so as well as a thousand would.
const WATCHED: usize = 64;
/// How long a book survives without a read. Stated in the tool descriptions,
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
fn seen_ms(data: &StreamData) -> Option<u64> {
    let ns = match data {
        StreamData::Quote { received_at_ns, .. }
        | StreamData::Trade { received_at_ns, .. }
        | StreamData::OpenInterest { received_at_ns, .. }
        | StreamData::Ohlcvc { received_at_ns, .. }
        | StreamData::MarketValue { received_at_ns, .. } => *received_at_ns,
        _ => return None,
    };
    Some(ns / 1_000_000)
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

/// The names a predicate or a rank may use: every numeric column `fields`
/// renders, so what a model reads in a tail is what it can compare, plus
/// `spread`. Spread is ask minus bid on one quote message — the comparison a
/// quote predicate is nearly always about, and the one name here that is
/// not a column the vendor sends.
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

/// A numeric field of a row by name; `None` when the row has no such field.
///
/// Runs on the dispatcher thread for a watched book, so it reads the enum
/// and allocates nothing.
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
        StreamData::OpenInterest { open_interest, .. } => match name {
            "open_interest" => f64::from(*open_interest),
            _ => return None,
        },
        StreamData::MarketValue {
            market_bid,
            market_ask,
            market_price,
            ..
        } => match name {
            "market_bid" => *market_bid,
            "market_ask" => *market_ask,
            "market_price" => *market_price,
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
/// same row, which is how a crossed book (`bid >= ask`) is asked for.
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

fn field_name(v: Option<&Value>, what: &str) -> Result<String, ToolError> {
    let name = v.and_then(|v: &Value| v.as_str()).unwrap_or_default();
    if FIELDS.contains(&name) {
        Ok(name.to_string())
    } else {
        Err(ToolError::InvalidParams(format!(
            "{what} must be one of {}",
            FIELDS.join(", ")
        )))
    }
}

/// Parse `[{field, op, value}, ...]`. `value` is a number, a field name
/// for a comparison, or `[low, high]` for `inside` / `outside`.
fn parse_clauses(v: &Value) -> Result<Vec<Clause>, ToolError> {
    let Some(items) = v.as_array() else {
        return Err(ToolError::InvalidParams(
            "a predicate is an array of {field, op, value} clauses".into(),
        ));
    };
    items
        .iter()
        .map(|item| {
            let field = field_name(item.get("field"), "field")?;
            let op = item
                .get("op")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or_default();
            let value = item.get("value");
            if op == "inside" || op == "outside" {
                let bounds: Vec<f64> = value
                    .and_then(|v: &Value| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
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
                None => Rhs::Field(field_name(value, "value")?),
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
fn print_field(p: &Print, name: &str) -> Option<f64> {
    field_of(&p.trade, name).or_else(|| p.quote_before.as_ref().and_then(|q| field_of(q, name)))
}

#[derive(Debug)]
struct Book {
    ring: VecDeque<StreamData>,
    received: u64,
    dropped: u64,
    opened_ms: u64,
    read_ms: u64,
    /// The caller's predicate; empty when none. Evaluated as rows arrive,
    /// because the rows it exists for are the ones the ring loses between
    /// two reads.
    watch: Vec<Clause>,
    retained: VecDeque<StreamData>,
    retained_dropped: u64,
    /// Rows the predicate examined since the last read: the population a
    /// list of matches covers.
    checked: u64,
}

impl Book {
    fn open(now: u64) -> Self {
        Self {
            ring: VecDeque::new(),
            received: 0,
            dropped: 0,
            opened_ms: now,
            read_ms: now,
            watch: Vec::new(),
            retained: VecDeque::new(),
            retained_dropped: 0,
            checked: 0,
        }
    }
}

/// Everything held for one contract: a book per subscription kind, and the
/// print correlation that spans two of them. Living together, the correlation
/// cannot outlive the last book or be dropped while one remains.
#[derive(Debug, Default)]
struct ContractState {
    /// At most one book per kind. Four kinds exist, so a scan beats a hash
    /// and the kind is kept alongside for reopening on a fresh session.
    books: Vec<(SubscriptionKind, Book)>,
    last_quote: Option<StreamData>,
    prints: VecDeque<Print>,
    prints_dropped: u64,
    /// The vendor's own bar for this contract, as last sent. The feed sends
    /// one ahead of each trade; it is stored and served as is, never built,
    /// extended or reconciled here.
    ohlcvc: Option<StreamData>,
}

impl ContractState {
    /// The book for `kind`, opened now if absent. `true` when it was.
    fn open(&mut self, kind: SubscriptionKind, now: u64) -> (&mut Book, bool) {
        let pos = self.books.iter().position(|(k, _)| *k == kind);
        let first = pos.is_none();
        let i = pos.unwrap_or_else(|| {
            self.books.push((kind, Book::open(now)));
            self.books.len() - 1
        });
        (&mut self.books[i].1, first)
    }
}

/// Every trade on one security type, from the vendor's full-stream
/// subscription, each with the NBBO the vendor sends ahead of it.
#[derive(Debug)]
struct Market {
    ring: VecDeque<Print>,
    received: u64,
    dropped: u64,
    opened_ms: u64,
    read_ms: u64,
    /// The quote most recently sent on the stream. The vendor sends a
    /// contract's last NBBO and bar just before its trade, so the trade
    /// claims this when it is for the same contract and leaves it when
    /// another contract's messages came between.
    last_quote: Option<StreamData>,
}

impl Market {
    fn open(now: u64) -> Self {
        Self {
            ring: VecDeque::new(),
            received: 0,
            dropped: 0,
            opened_ms: now,
            read_ms: now,
            last_quote: None,
        }
    }

    fn ingest(&mut self, data: &StreamData) {
        match data {
            StreamData::Quote { .. } => self.last_quote = Some(data.clone()),
            StreamData::Trade { contract, .. } => {
                self.received += 1;
                let quote_before = self
                    .last_quote
                    .take()
                    .filter(|q| contract_of(q) == Some(contract));
                if self.ring.len() == MARKET_RING {
                    self.ring.pop_front();
                    self.dropped += 1;
                }
                self.ring.push_back(Print {
                    trade: data.clone(),
                    quote_before,
                    quotes_after: Vec::new(),
                });
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct Held {
    contracts: HashMap<Contract, ContractState>,
    /// At most one market book per security type, keyed the way the SDK
    /// keys its full-stream snapshot.
    markets: Vec<(SecType, Market)>,
    /// The feed's most recent refusal of a subscribe, with when it was
    /// seen. The wire answers by request id and nothing outside the SDK
    /// maps that back to a contract, so this is the nearest thing to a
    /// reason a dropped book can be given.
    last_rejection: Option<(StreamResponseType, u64)>,
}

impl Held {
    fn market(&mut self, sec: SecType) -> Option<&mut Market> {
        self.markets
            .iter_mut()
            .find(|(s, _)| *s == sec)
            .map(|(_, m)| m)
    }

    /// How many contracts of `sec` hold a trade or quote book.
    fn per_contract_feed_on(&self, sec: SecType) -> usize {
        self.contracts
            .iter()
            .filter(|(c, s)| c.sec_type == sec && s.books.iter().any(|(k, _)| overlaps(*k)))
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

/// What one read saw, copied out from under the lock.
struct Reading {
    first: bool,
    floor: u64,
    dropped: u64,
    /// Where coverage starts: the oldest row held once the ring has
    /// overflowed, else the moment the book opened. A quiet book has seen
    /// everything since it opened, rows or not.
    covered_since_ms: u64,
    newest_ms: Option<u64>,
    /// The window reaches back before coverage starts, so the result is not
    /// the whole window asked for — because the ring overflowed, or because
    /// the book is younger than the window.
    clipped: bool,
    new_since_last_read: u64,
    summary: Summary,
    tail: Vec<StreamData>,
    ohlcvc: Option<StreamData>,
    /// The predicate in force after this read, and what the previous one
    /// kept since the last read.
    watch: Vec<Clause>,
    watched: Vec<StreamData>,
    watched_dropped: u64,
    checked: u64,
}

/// Counts and extremes over the rows in a window. Every row is the vendor's
/// own message; nothing is derived from one.
#[derive(Default)]
struct Summary {
    count: u64,
    first: Option<StreamData>,
    last: Option<StreamData>,
    /// Lowest and highest trade price, as the whole message so each carries
    /// its condition code.
    low: Option<StreamData>,
    high: Option<StreamData>,
    /// Trade condition codes seen, with how often. Sorted by code.
    conditions: BTreeMap<i32, u64>,
    bid: Option<(f64, f64)>,
    ask: Option<(f64, f64)>,
}

fn span(acc: Option<(f64, f64)>, v: f64) -> Option<(f64, f64)> {
    Some(acc.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))))
}

struct Prints {
    opened: Vec<SubscriptionKind>,
    rows: Vec<Print>,
    held: usize,
    dropped: u64,
    covered_since_ms: u64,
    newest_ms: Option<u64>,
    new_since_last_read: u64,
}

/// What a whole-market read selects: the contract attributes, the
/// predicate over vendor fields, and how to rank and cut what passes.
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
    fn selects(&self, p: &Print) -> bool {
        let Some(c) = contract_of(&p.trade) else {
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
        holds_all(&self.clauses, &|f| print_field(p, f))
    }
}

struct MarketReading {
    first: bool,
    floor: u64,
    received: u64,
    held: usize,
    dropped: u64,
    covered_since_ms: u64,
    newest_ms: Option<u64>,
    clipped: bool,
    new_since_last_read: u64,
    /// The populations each count covers: prints inside the window, prints
    /// the query selected from those, and the rows served.
    in_window: u64,
    matched: u64,
    rows: Vec<Print>,
}

struct Holding {
    sub: Subscription,
    label: String,
    received: u64,
    held: usize,
    dropped: u64,
    opened_ms: u64,
    read_ms: u64,
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
    /// Set when the feed reports its reconnect budget spent. The SDK then
    /// reads as `Reconnecting` for good, so this is the only signal that the
    /// next read must restart the session rather than wait for it.
    reconnects_exhausted: AtomicBool,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, Held> {
        // Recover rather than propagate: one panicking read must not disable
        // every stream tool for the life of the process.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Close books nobody has read inside the TTL. Runs first in every call,
    /// so a book that outlived its idle window is buried before the read
    /// that would otherwise have revived it, and a sweep tied to the map
    /// cannot drift from it.
    fn sweep(held: &mut Held, now: u64) -> Subs {
        let ttl = TTL.as_millis() as u64;
        let mut freed = Vec::new();
        held.contracts.retain(|contract, state| {
            state.books.retain(|(kind, book)| {
                let live = now.saturating_sub(book.read_ms) <= ttl;
                if !live {
                    freed.push(subscription(*kind, contract));
                }
                live
            });
            !state.books.is_empty()
        });
        held.markets.retain(|(sec, market)| {
            let live = now.saturating_sub(market.read_ms) <= ttl;
            if !live {
                freed.push(sec.full_trades());
            }
            live
        });
        freed
    }

    /// Refuse a per-contract book the market book on its security type
    /// would double, and say what to close.
    fn without_market(
        held: &Held,
        contract: &Contract,
        kind: SubscriptionKind,
    ) -> Result<(), ToolError> {
        if overlaps(kind) && held.markets.iter().any(|(s, _)| *s == contract.sec_type) {
            return Err(ToolError::InvalidParams(format!(
                "the whole-market book on {} (tape_market) already carries every {} on it, and \
                 the feed would deliver {contract} twice if both were held; tape_stop with \
                 sec_type alone closes the market book",
                contract.sec_type.as_str(),
                kind.kind_str()
            )));
        }
        Ok(())
    }

    /// Read a book, opening it if this is the first look. `window` is a
    /// lookback in milliseconds; without one the window is everything since
    /// the previous read. `watch` replaces the book's predicate when given.
    /// Returns what the sweep freed alongside.
    ///
    /// Every reader copies what it needs under the lock and releases it
    /// before serialising. The streaming dispatcher calls [`Self::ingest`] on
    /// the same lock, so a critical section held across JSON construction
    /// would stall the feed — the one thing this layer must never do. The
    /// summary is one pass over references; only the tail is cloned.
    fn read(
        &self,
        contract: &Contract,
        kind: SubscriptionKind,
        window: Option<u64>,
        tail: usize,
        watch: Option<Vec<Clause>>,
        now: u64,
    ) -> Result<(Reading, Subs), ToolError> {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        Self::without_market(&held, contract, kind)?;
        let state = held.contracts.entry(contract.clone()).or_default();
        let (book, first) = state.open(kind, now);
        let previous = book.read_ms;
        book.read_ms = now;
        let floor = window.map_or(previous, |w| now.saturating_sub(w));

        let mut summary = Summary::default();
        let mut rows = Vec::new();
        let mut new_since_last_read = 0;
        let mut oldest = None;
        let mut low: Option<(f64, &StreamData)> = None;
        let mut high: Option<(f64, &StreamData)> = None;
        // Newest first, so the walk stops at whichever of the window and the
        // previous read reaches further back and never touches the rest.
        for d in book.ring.iter().rev() {
            let seen = seen_ms(d);
            if seen.is_some_and(|s| s < floor.min(previous)) {
                break;
            }
            if seen.is_none_or(|s| s >= previous) {
                new_since_last_read += 1;
            }
            if seen.is_some_and(|s| s < floor) {
                continue;
            }
            summary.count += 1;
            if summary.last.is_none() {
                summary.last = Some(d.clone());
            }
            oldest = Some(d);
            match d {
                StreamData::Trade {
                    price, condition, ..
                } => {
                    *summary.conditions.entry(*condition).or_default() += 1;
                    if low.is_none_or(|(p, _)| *price < p) {
                        low = Some((*price, d));
                    }
                    if high.is_none_or(|(p, _)| *price > p) {
                        high = Some((*price, d));
                    }
                }
                StreamData::Quote { bid, ask, .. } => {
                    summary.bid = span(summary.bid, *bid);
                    summary.ask = span(summary.ask, *ask);
                }
                _ => {}
            }
            if rows.len() < tail {
                rows.push(d.clone());
            }
        }
        summary.first = oldest.cloned();
        summary.low = low.map(|(_, d)| d.clone());
        summary.high = high.map(|(_, d)| d.clone());
        rows.reverse();

        // What the standing predicate kept comes out before a new one goes
        // in: it matched the line that stood when it arrived.
        let watched = std::mem::take(&mut book.retained).into();
        let watched_dropped = std::mem::take(&mut book.retained_dropped);
        let checked = std::mem::take(&mut book.checked);
        if let Some(w) = watch {
            book.watch = w;
        }

        let covered_since_ms = book
            .ring
            .front()
            .and_then(seen_ms)
            .filter(|_| book.dropped > 0)
            .unwrap_or(book.opened_ms);
        let dropped = book.dropped;
        let newest_ms = book.ring.back().and_then(seen_ms);
        let watch = book.watch.clone();
        let reading = Reading {
            first,
            floor,
            dropped,
            covered_since_ms,
            newest_ms,
            clipped: covered_since_ms > floor,
            new_since_last_read,
            summary,
            tail: rows,
            ohlcvc: state.ohlcvc.clone(),
            watch,
            watched,
            watched_dropped,
            checked,
        };
        Ok((reading, expired))
    }

    /// The newest `count` prints, oldest first. Opens the trade and quote
    /// books a print is built from when they are not already held.
    fn prints(
        &self,
        contract: &Contract,
        count: usize,
        now: u64,
    ) -> Result<(Prints, Subs), ToolError> {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        Self::without_market(&held, contract, SubscriptionKind::Trade)?;
        let state = held.contracts.entry(contract.clone()).or_default();
        let mut opened = Vec::new();
        // Prints are read against the trade leg: its last read is what "new"
        // means, and its opening is where coverage starts.
        let mut previous = now;
        let mut opened_ms = now;
        for kind in [SubscriptionKind::Trade, SubscriptionKind::Quote] {
            let (book, first) = state.open(kind, now);
            if first {
                opened.push(kind);
            }
            if kind == SubscriptionKind::Trade {
                previous = book.read_ms;
                opened_ms = book.opened_ms;
            }
            book.read_ms = now;
        }
        let mut rows: Vec<Print> = state.prints.iter().rev().take(count).cloned().collect();
        rows.reverse();
        let prints = Prints {
            opened,
            held: state.prints.len(),
            dropped: state.prints_dropped,
            covered_since_ms: state
                .prints
                .front()
                .and_then(|p| seen_ms(&p.trade))
                .filter(|_| state.prints_dropped > 0)
                .unwrap_or(opened_ms),
            newest_ms: state.prints.back().and_then(|p| seen_ms(&p.trade)),
            new_since_last_read: state
                .prints
                .iter()
                .rev()
                .take_while(|p| seen_ms(&p.trade).is_none_or(|s| s >= previous))
                .count() as u64,
            rows,
        };
        Ok((prints, expired))
    }

    /// Read the whole-market book for `sec`, opening it on the first look.
    /// The walk is one pass over references; only the rows served are
    /// cloned, and there are at most `limit` of those.
    fn market(
        &self,
        sec: SecType,
        q: &MarketQuery,
        window: Option<u64>,
        now: u64,
    ) -> Result<(MarketReading, Subs), ToolError> {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let doubled = held.per_contract_feed_on(sec);
        if doubled > 0 {
            return Err(ToolError::InvalidParams(format!(
                "{doubled} {} contract(s) hold a trade or quote book, and the feed would deliver \
                 them twice alongside a whole-market book; tape_list shows them and tape_stop \
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
        market.read_ms = now;
        let floor = window.map_or(previous, |w| now.saturating_sub(w));

        let mut new_since_last_read = 0;
        let mut in_window = 0;
        let mut picked: Vec<(Option<f64>, &Print)> = Vec::new();
        for p in market.ring.iter().rev() {
            let seen = seen_ms(&p.trade);
            if seen.is_some_and(|s| s < floor.min(previous)) {
                break;
            }
            if seen.is_none_or(|s| s >= previous) {
                new_since_last_read += 1;
            }
            if seen.is_some_and(|s| s < floor) {
                continue;
            }
            in_window += 1;
            if q.selects(p) {
                let key = q.rank_by.as_deref().and_then(|f| print_field(p, f));
                picked.push((key, p));
            }
        }
        let matched = picked.len() as u64;
        if q.rank_by.is_some() {
            // A row without the rank field sorts last either way; the sort
            // is stable, so ties keep the newest first.
            picked.sort_by(|(a, _), (b, _)| match (a, b) {
                (Some(x), Some(y)) if q.ascending => x.total_cmp(y),
                (Some(x), Some(y)) => y.total_cmp(x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
        }
        picked.truncate(q.limit);
        let mut rows: Vec<Print> = picked.into_iter().map(|(_, p)| p.clone()).collect();
        if q.rank_by.is_none() {
            rows.reverse();
        }

        let covered_since_ms = market
            .ring
            .front()
            .and_then(|p| seen_ms(&p.trade))
            .filter(|_| market.dropped > 0)
            .unwrap_or(market.opened_ms);
        let reading = MarketReading {
            first,
            floor,
            received: market.received,
            held: market.ring.len(),
            dropped: market.dropped,
            covered_since_ms,
            newest_ms: market.ring.back().and_then(|p| seen_ms(&p.trade)),
            clipped: covered_since_ms > floor,
            new_since_last_read,
            in_window,
            matched,
            rows,
        };
        Ok((reading, expired))
    }

    /// Every subscription a contract's books hold, for `tape_stop` to close
    /// on the feed before the books go. Returns what the sweep freed
    /// alongside.
    fn held_for(&self, contract: &Contract, now: u64) -> (Subs, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let subs = held.contracts.get(contract).map_or_else(Vec::new, |s| {
            s.books
                .iter()
                .map(|(k, _)| subscription(*k, contract))
                .collect()
        });
        (subs, expired)
    }

    fn market_held(&self, sec: SecType) -> bool {
        self.lock().markets.iter().any(|(s, _)| *s == sec)
    }

    /// Drop a book the feed does not carry: one whose subscribe was refused
    /// after being accepted, or one whose unsubscribe landed. Leaving it
    /// would make the next read skip subscribing too.
    fn forget(&self, sub: &Subscription) {
        let mut held = self.lock();
        match shape(sub) {
            Some(Shape::Contract(contract, kind)) => {
                if let Some(state) = held.contracts.get_mut(contract) {
                    state.books.retain(|(k, _)| *k != kind);
                    if state.books.is_empty() {
                        held.contracts.remove(contract);
                    }
                }
            }
            Some(Shape::Full(sec, _)) => held.markets.retain(|(s, _)| *s != sec),
            None => {}
        }
    }

    /// Keep a reference to a subscription the feed still holds after its
    /// book was swept: an empty book already past its TTL, so the next
    /// sweep hands it back to be closed again. A book reopened meanwhile is
    /// left as it is.
    fn reinstate(&self, sub: &Subscription, now: u64) {
        let mut held = self.lock();
        match shape(sub) {
            Some(Shape::Contract(contract, kind)) => {
                let state = held.contracts.entry(contract.clone()).or_default();
                let (book, first) = state.open(kind, now);
                if first {
                    book.read_ms = 0;
                }
            }
            Some(Shape::Full(sec, _)) if held.market(sec).is_none() => {
                let mut market = Market::open(now);
                market.read_ms = 0;
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

    fn list(&self, now: u64) -> (Vec<Holding>, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let mut rows: Vec<Holding> = held
            .contracts
            .iter()
            .flat_map(|(c, s)| {
                s.books.iter().map(move |(k, b)| Holding {
                    sub: subscription(*k, c),
                    label: c.to_string(),
                    received: b.received,
                    held: b.ring.len(),
                    dropped: b.dropped,
                    opened_ms: b.opened_ms,
                    read_ms: b.read_ms,
                    newest_ms: b.ring.back().and_then(seen_ms),
                })
            })
            .chain(held.markets.iter().map(|(sec, m)| Holding {
                sub: sec.full_trades(),
                label: sec.as_str().to_string(),
                received: m.received,
                held: m.ring.len(),
                dropped: m.dropped,
                opened_ms: m.opened_ms,
                read_ms: m.read_ms,
                newest_ms: m.ring.back().and_then(|p| seen_ms(&p.trade)),
            }))
            .collect();
        rows.sort_by_key(|h| (h.label.clone(), label(&h.sub)));
        (rows, expired)
    }

    /// Every subscription the books hold, once each, in the two shapes the
    /// SDK's restore takes: what a fresh session must reopen.
    fn subscriptions(&self) -> Restore {
        let held = self.lock();
        (
            held.contracts
                .iter()
                .flat_map(|(c, s)| s.books.iter().map(move |(k, _)| (*k, c.clone())))
                .collect(),
            held.markets
                .iter()
                .map(|(sec, _)| (SubscriptionKind::Trade, *sec))
                .collect(),
        )
    }

    /// Store a row. Unknown books are ignored rather than created: the feed
    /// can deliver a contract after its unsubscribe, and inventing a book for
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
            state.ohlcvc = Some(data);
            return;
        }
        let Some(msg) = msg_type_of(&data) else {
            return;
        };
        let Some((_, book)) = state
            .books
            .iter_mut()
            .find(|(k, _)| k.subscribe_code() == msg)
        else {
            return;
        };
        book.received += 1;
        if book.ring.len() == RING {
            book.ring.pop_front();
            book.dropped += 1;
        }
        book.ring.push_back(data.clone());
        if !book.watch.is_empty() {
            book.checked += 1;
            if holds_all(&book.watch, &|f| field_of(&data, f)) {
                if book.retained.len() == WATCHED {
                    book.retained.pop_front();
                    book.retained_dropped += 1;
                }
                book.retained.push_back(data.clone());
            }
        }

        match &data {
            StreamData::Quote { .. } => {
                // Every print still short of its two quotes takes this one.
                // Filling only the newest starves an earlier print whenever a
                // second trade arrives before the first one's quotes do, which
                // on a liquid contract is most of them.
                for open in state
                    .prints
                    .iter_mut()
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
    "tape_read",
    "tape_prints",
    "tape_market",
    "tape_list",
    "tape_stop",
];

/// Kinds the vendor offers for a security type, the default first.
///
/// Indices have no trade or quote stream — only price and market value — and
/// an index price arrives on the trade subscription in a trade-shaped
/// message, which is why `trade` is how it is asked for.
fn kinds_for(sec: SecType) -> &'static [SubscriptionKind] {
    match sec {
        SecType::Option | SecType::Stock => &[
            SubscriptionKind::Quote,
            SubscriptionKind::Trade,
            SubscriptionKind::MarketValue,
            SubscriptionKind::OpenInterest,
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
                "tape_market covers option and stock: the vendor broadcasts every trade for \
                 those two only. Read {} per contract with tape_read instead",
                sec.as_str().to_ascii_lowercase()
            )))
        }
    };
    if tier.is_some_and(|t| t >= SubscriptionTier::Pro) {
        return Ok(());
    }
    Err(ToolError::ServerError(format!(
        "tape_market on {} needs a {class} Pro subscription: the vendor's whole-market trade \
         stream is Pro-only, and this account's {class} tier is {}. tape_read and tape_prints \
         on single {} contracts work at every tier",
        sec.as_str().to_ascii_lowercase(),
        tier.map_or("not reported".to_string(), |t| format!("{t:?}")),
        sec.as_str().to_ascii_lowercase()
    )))
}

/// The vendor's own definition of each subscribe response, from its
/// stream-verification page, so a dropped book can say what the code means.
fn rejection_meaning(code: StreamResponseType) -> &'static str {
    match code {
        StreamResponseType::Subscribed => "the request to subscribe was successful",
        StreamResponseType::Error => "an unknown error subscribing to the stream",
        StreamResponseType::MaxStreamsReached => {
            "streaming too many contracts; unsubscribe some (tape_stop), upgrade the \
             subscription, or stop all streams"
        }
        StreamResponseType::InvalidPerms => {
            "no permission for the stream request; the subscription for this security type \
             may need upgrading"
        }
    }
}

/// The contract properties shared by every tool that names one, plus
/// whatever the tool adds.
fn contract_schema(extra: Value) -> Value {
    let mut props = json!({
        "sec_type": {"type": "string", "enum": ["option", "stock", "index"]},
        "root": {"type": "string", "description": "Ticker or option root, e.g. AAPL."},
        "expiration": {"type": "integer", "description": "YYYYMMDD. Options only."},
        "strike": {"type": "number", "description": "Strike in dollars. Options only."},
        "right": {"type": "string", "enum": ["C", "P"], "description": "Options only."}
    });
    if let (Some(props), Some(extra)) = (props.as_object_mut(), extra.as_object()) {
        for (k, v) in extra.iter() {
            props.insert(k, v.clone());
        }
    }
    json!({"type": "object", "properties": props, "required": ["sec_type", "root"]})
}

/// The schema of a predicate: clauses over the vendor's fields.
fn clauses_schema(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": {
            "type": "object",
            "properties": {
                "field": {"type": "string", "enum": FIELDS},
                "op": {"type": "string", "enum": [">", ">=", "<", "<=", "==", "!=", "inside", "outside"]},
                "value": {"description": "A number; a field name to compare against, as in bid >= ask; or [low, high] for inside and outside."}
            },
            "required": ["field", "op", "value"]
        }
    })
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "tape_read",
            "description": "Live data for one contract in one call. The first read opens the \
                subscription and returns nothing yet; read again a second or two later. After \
                that each read summarises a window and serves its newest rows verbatim. The \
                window defaults to everything since your last read of this book; pass seconds \
                for a fixed lookback. A snapshot is a round trip and has already moved by the \
                time you read it, so age_ms says how old the newest row is: an index reports \
                about once a second, so seconds of age are normal there and stale on an option \
                quote. covers_seconds is how far back the rows held reach and clipped means the \
                window you asked for reaches further, which a liquid quote book hits inside a \
                minute: 4096 rows are held per book. The summary counts and sorts the rows it \
                saw, each trade extreme with its condition code, and the tail is the vendor's \
                messages as sent, condition and exchange codes intact. vendor_ohlcvc is the \
                vendor's own bar for the contract as last sent, served as is; nothing here \
                builds a bar from trades, because condition, cancel and size rules are yours \
                to choose. watch leaves a predicate on the book: rows that match it are kept \
                past the ring and served at the top of your next read under watch.matched, \
                with their times, so what crosses a line while you are not reading is not \
                lost; watch.checked says how many rows it examined. kind \
                defaults to quote; an index has no quote stream, so it defaults to trade, which \
                carries the index price. market_value is a derived midpoint, not a quote. Times \
                are Eastern. A book unread for 15 minutes closes on its own. A read that finds \
                the feed refused the subscription after accepting it says so and releases the \
                book; reading again re-subscribes.",
            "inputSchema": contract_schema(json!({
                "kind": {"type": "string", "enum": ["quote", "trade", "market_value", "open_interest"],
                         "description": "Default quote, or trade for an index."},
                "seconds": {"type": "number", "description": "Fixed lookback. Default: since your last read."},
                "tail": {"type": "integer", "description": "Newest rows served verbatim. Default 10, capped at 50. Ask for a summary over a longer window rather than more rows."},
                "watch": clauses_schema("Clauses that must all hold for a row to be kept, over the vendor's fields; spread is ask minus bid. Stays in force until replaced; [] clears it. Spread wider than 0.10: {field: spread, op: >, value: 0.10}. Crossed book: {field: bid, op: >=, value: ask}. Price outside a range: {field: price, op: outside, value: [lo, hi]}. 64 matches are kept between reads.")
            }))
        }),
        json!({
            "name": "tape_prints",
            "description": "Recent trades on one contract, newest last, each with the quote that \
                stood before it; the feed also sends the two quotes after a print, and \
                quotes_after returns them. Opens the trade and quote subscriptions on the first \
                call and returns nothing yet; call again a second or two later. \
                new_since_last_read counts prints since you last looked at this contract's \
                trades. clipped means older prints were discarded: 256 are held per contract. \
                Times are Eastern.",
            "inputSchema": contract_schema(json!({
                "count": {"type": "integer", "description": "Newest prints. Default 20."},
                "quotes_after": {"type": "boolean", "description": "Include the two quotes after each print. Default false."}
            }))
        }),
        json!({
            "name": "tape_market",
            "description": "Every trade across the whole option or stock market from one \
                subscription, held as a rolling window of 16384 prints, each with the NBBO the \
                vendor sent ahead of it. Needs an Options Pro or Stocks Pro subscription; the \
                error says which when the account lacks it. Nothing is computed: you select and \
                rank. Filter on the contract (root, expiration, right, strike_min, strike_max) \
                and on the vendor's fields (where), rank by one field (rank_by, largest first \
                unless ascending), and take at most limit rows, 50 at most. Without rank_by the \
                newest matches come back, oldest first. in_window, matched and returned say how \
                many prints the window held, how many passed your selection and how many you \
                got; held, dropped and covers_seconds say what the ring reaches. The window \
                defaults to since your last read; seconds fixes a lookback. The first call opens \
                the subscription and returns nothing yet; call again a second or two later. A \
                market book is not held alongside per-contract trade or quote books on the same \
                security type, because the feed would deliver those contracts twice; tape_stop \
                one side. Unread for 15 minutes it closes; tape_stop with sec_type alone closes \
                it. Times are Eastern.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sec_type": {"type": "string", "enum": ["option", "stock"]},
                    "root": {"type": "string", "description": "Only this ticker or option root."},
                    "expiration": {"type": "integer", "description": "YYYYMMDD. Options only."},
                    "right": {"type": "string", "enum": ["C", "P"], "description": "Options only."},
                    "strike_min": {"type": "number", "description": "Dollars, inclusive. Options only."},
                    "strike_max": {"type": "number", "description": "Dollars, inclusive. Options only."},
                    "where": clauses_schema("Clauses that must all hold, over the trade's fields and the quote before it; spread is ask minus bid."),
                    "rank_by": {"type": "string", "enum": FIELDS, "description": "Rank the selection by this field. Default: newest."},
                    "ascending": {"type": "boolean", "description": "Smallest first. Default false."},
                    "limit": {"type": "integer", "description": "Rows returned. Default 20, capped at 50."},
                    "seconds": {"type": "number", "description": "Fixed lookback. Default: since your last read."}
                },
                "required": ["sec_type"]
            }
        }),
        json!({
            "name": "tape_list",
            "description": "Every book this server holds: rows received and held, the age of \
                the newest, how long since it was read and when it expires, plus the feed's \
                state. on_feed is whether the feed itself still carries the subscription; a \
                book the feed dropped is released on its next read. on_feed_not_held lists \
                subscriptions the feed carries for no book. last_rejection is the feed's most \
                recent refusal of a subscribe, with the vendor's meaning. An age that keeps \
                growing while the feed says Connected is a contract that has gone quiet, not a \
                fault; anything else and the next tape_read restarts the feed.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "tape_stop",
            "description": "Close every book held for a contract and release its subscriptions; \
                with sec_type alone, close the whole-market book for that security type. A \
                subscription the feed did not release stays held, and the next sweep or stop \
                tries again. Books also close on their own after 15 minutes without a read.",
            "inputSchema": {
                "type": "object",
                "properties": contract_schema(json!({})).get("properties").cloned().unwrap_or_default(),
                "required": ["sec_type"]
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

/// A row as an object with how old it is, for rows served out of time
/// order where the tail's implicit ordering does not say.
fn aged_object(data: &StreamData, now: u64) -> Value {
    let mut out = object(data);
    if let (Some(obj), Some(seen)) = (out.as_object_mut(), seen_ms(data)) {
        obj.insert("age_ms", Value::from(now.saturating_sub(seen)));
    }
    out
}

fn row(data: &StreamData) -> Value {
    fields(data)
        .into_iter()
        .map(|(_, v)| v)
        .collect::<Vec<_>>()
        .into()
}

fn summary_json(s: &Summary) -> Value {
    let mut out = json!({
        "count": s.count,
        "first": s.first.as_ref().map(object),
        "last": s.last.as_ref().map(object),
    });
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    if let (Some(low), Some(high)) = (&s.low, &s.high) {
        obj.insert("low", object(low));
        obj.insert("high", object(high));
        let mut conditions = json!({});
        if let Some(c) = conditions.as_object_mut() {
            for (code, n) in &s.conditions {
                c.insert(&code.to_string(), Value::from(*n));
            }
        }
        obj.insert("conditions", conditions);
    }
    if let (Some((bid_lo, bid_hi)), Some((ask_lo, ask_hi))) = (s.bid, s.ask) {
        obj.insert("bid", json!({"min": bid_lo, "max": bid_hi}));
        obj.insert("ask", json!({"min": ask_lo, "max": ask_hi}));
    }
    out
}

fn seconds(ms: u64) -> f64 {
    ms as f64 / 1_000.0
}

fn stream_error(what: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::ServerError(format!("{what}: {}", sanitize_error(&e.to_string())))
}

fn feed_state(client: &Client, reg: &Registry) -> String {
    if reg.reconnects_exhausted.load(Ordering::Relaxed) {
        "ReconnectsExhausted".to_string()
    } else {
        format!("{:?}", client.stream().connection_status())
    }
}

/// Make sure the feed delivers into the registry.
///
/// The SDK reconnects on its own after a drop. When it gives up it says so
/// once and then reads as `Reconnecting` indefinitely; a dispatcher fault
/// reads as `Disconnected`. Both need the dead session retired and a new one
/// started, and a new session knows nothing of the books this process still
/// holds, so they are reopened from the registry rather than from what the
/// old session tracked.
fn ensure_streaming(client: &Client, reg: &'static Registry) -> Result<(), ToolError> {
    let stream = client.stream();
    let exhausted = reg.reconnects_exhausted.swap(false, Ordering::Relaxed);
    match stream.connection_status() {
        ConnectionStatus::NotStarted => {}
        // A dead session still occupies the slot until it is stopped.
        ConnectionStatus::Disconnected => stream.stop_streaming(),
        _ if exhausted => stream.stop_streaming(),
        _ => return Ok(()),
    }
    stream
        .start_streaming(move |event: &StreamEvent| match event {
            StreamEvent::Data(data) => reg.ingest(data.clone()),
            StreamEvent::Control(StreamControl::ReconnectsExhausted { .. }) => {
                reg.reconnects_exhausted.store(true, Ordering::Relaxed);
            }
            StreamEvent::Control(StreamControl::ReqResponse { result, .. })
                if *result != StreamResponseType::Subscribed =>
            {
                reg.note_rejection(*result, now_ms());
            }
            _ => {}
        })
        .map_err(|e| stream_error("could not start streaming", e))?;
    let (per_contract, full) = reg.subscriptions();
    stream
        .restore_subscriptions(&per_contract, &full)
        .map_err(|e| stream_error("could not reopen every subscription", e))
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

/// Release a book the feed does not carry and say so in words, with the
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
        "the feed accepted the {} subscription and then dropped it, so this book received \
         nothing; it has been released and reading again re-subscribes.{why} A subscribe is \
         refused with {} when {}, and with {} when {}.",
        label(sub),
        StreamResponseType::MaxStreamsReached,
        rejection_meaning(StreamResponseType::MaxStreamsReached),
        StreamResponseType::InvalidPerms,
        rejection_meaning(StreamResponseType::InvalidPerms),
    ))
}

/// Fail a read whose book the feed no longer carries. Runs on every read
/// after the first: the feed answers a subscribe after the call that sent
/// it returns, so the read that opened the book cannot know yet.
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
    match subs.iter().find(|s| !live.contains(s)) {
        Some(sub) => Err(dropped_by_feed(reg, sub, now)),
        None => Ok(()),
    }
}

fn parse_sec(args: &Value) -> Result<SecType, ToolError> {
    match args.get("sec_type").and_then(|v: &Value| v.as_str()) {
        Some("option") => Ok(SecType::Option),
        Some("stock") => Ok(SecType::Stock),
        Some("index") => Ok(SecType::Index),
        _ => Err(ToolError::InvalidParams(
            "sec_type must be option, stock or index".into(),
        )),
    }
}

fn parse_contract(args: &Value) -> Result<(SecType, Contract), ToolError> {
    let str_of = |k: &str| args.get(k).and_then(|v: &Value| v.as_str());
    let sec = parse_sec(args)?;
    let root = str_of("root").ok_or_else(|| ToolError::InvalidParams("root is required".into()))?;
    let contract = match sec {
        SecType::Stock => Contract::stock(root),
        SecType::Index => Contract::index(root),
        _ => {
            let exp = args.get("expiration").and_then(|v: &Value| v.as_u64());
            let strike = args.get("strike").and_then(|v: &Value| v.as_f64());
            let (Some(exp), Some(strike), Some(right)) = (exp, strike, str_of("right")) else {
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
    let str_of = |k: &str| args.get(k).and_then(|v: &Value| v.as_str());
    let num_of = |k: &str| args.get(k).and_then(|v: &Value| v.as_f64());
    let is_call = match str_of("right") {
        None => None,
        Some("C") => Some(true),
        Some("P") => Some(false),
        Some(_) => return Err(ToolError::InvalidParams("right must be C or P".into())),
    };
    Ok(MarketQuery {
        root: str_of("root").map(str::to_string),
        expiration: args
            .get("expiration")
            .and_then(|v: &Value| v.as_i64())
            .and_then(|e| i32::try_from(e).ok()),
        is_call,
        strike_min: num_of("strike_min"),
        strike_max: num_of("strike_max"),
        clauses: args.get("where").map_or(Ok(Vec::new()), parse_clauses)?,
        rank_by: args
            .get("rank_by")
            .map(|v| field_name(Some(v), "rank_by"))
            .transpose()?,
        ascending: args
            .get("ascending")
            .and_then(|v: &Value| v.as_bool())
            .unwrap_or(false),
        limit,
    })
}

/// Close what the sweep freed, so a forgotten book cannot hold an allowance
/// the next caller needs. Nobody is waiting on these, so a failure is logged
/// rather than returned — and the book put back, expired, so the next sweep
/// tries the close again instead of leaving the feed holding it.
fn close_expired(client: &Client, reg: &Registry, expired: Subs, now: u64) {
    for sub in expired {
        if let Err(e) = client.stream().unsubscribe(sub.clone()) {
            tracing::warn!(sub = %label(&sub), error = %e, "expired book left its subscription open; will retry");
            reg.reinstate(&sub, now);
        }
    }
}

/// Open on the feed what a read just opened in the registry, rolling the
/// book back if the feed refuses.
fn open_on_feed(client: &Client, reg: &Registry, subs: &[Subscription]) -> Result<(), ToolError> {
    for sub in subs {
        if let Err(e) = client.stream().subscribe(sub.clone()) {
            reg.forget(sub);
            return Err(stream_error(
                &format!("could not subscribe {}", label(sub)),
                e,
            ));
        }
    }
    Ok(())
}

/// Close subscriptions on the feed, dropping each book only once the feed
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
    Some(execute(client, name, args))
}

/// Tool calls arrive one at a time (the JSON-RPC loop awaits each before
/// reading the next), so a registry mutation and the feed call that follows
/// it are never interleaved with another tool call. That ordering is what
/// lets a read hand back subscriptions to open or close outside the lock.
fn execute(client: &Client, name: &str, args: &Value) -> Result<Value, ToolError> {
    let reg = registry();
    let now = now_ms();
    let num_of = |k: &str, d: usize| {
        args.get(k)
            .and_then(|v: &Value| v.as_u64())
            .map_or(d, |v| v as usize)
    };
    let window = args
        .get("seconds")
        .and_then(|v: &Value| v.as_f64())
        .map(|s| (s.max(0.0) * 1_000.0) as u64);

    if name == "tape_list" {
        let (rows, expired) = reg.list(now);
        close_expired(client, reg, expired, now);
        // A listing is diagnostic, so a feed that cannot say what it carries
        // leaves the on_feed fields null rather than failing the call.
        let live = on_feed(client).ok();
        let unheld = live.as_ref().map(|live| {
            live.iter()
                .filter(|s| !rows.iter().any(|h| h.sub == **s))
                .map(label)
                .collect::<Vec<_>>()
        });
        return Ok(json!({
            "feed": feed_state(client, reg),
            "last_rejection": reg.last_rejection().map(|(code, at)| json!({
                "result": code.to_string(),
                "meaning": rejection_meaning(code),
                "seconds_ago": seconds(now.saturating_sub(at))
            })),
            "held": rows.iter().map(|h| json!({
                "contract": h.label,
                "kind": match shape(&h.sub) {
                    Some(Shape::Contract(_, k)) => k.kind_str(),
                    Some(Shape::Full(_, k)) => k.kind_str(),
                    None => "unknown",
                },
                "on_feed": live.as_ref().map(|live| live.contains(&h.sub)),
                "received": h.received,
                "held": h.held,
                "dropped": h.dropped,
                "age_ms": h.newest_ms.map(|s| now.saturating_sub(s)),
                "open_for_seconds": seconds(now.saturating_sub(h.opened_ms)),
                "idle_seconds": seconds(now.saturating_sub(h.read_ms)),
                "expires_in_seconds": seconds((TTL.as_millis() as u64).saturating_sub(now.saturating_sub(h.read_ms)))
            })).collect::<Vec<_>>(),
            "on_feed_not_held": unheld
        }));
    }

    if name == "tape_market" {
        let sec = parse_sec(args)?;
        let hist = client.market_data();
        pro_required(
            sec,
            match sec {
                SecType::Option => hist.options_tier(),
                _ => hist.stock_tier(),
            },
        )?;
        let q = parse_market_query(args, tail_rows(num_of("limit", 20)))?;
        ensure_streaming(client, reg)?;
        let (m, expired) = reg.market(sec, &q, window, now)?;
        close_expired(client, reg, expired, now);
        let sub = sec.full_trades();
        if m.first {
            open_on_feed(client, reg, &[sub])?;
        } else {
            reconcile(client, reg, &[sub], now)?;
        }
        return Ok(json!({
            "sec_type": sec.as_str().to_ascii_lowercase(),
            "feed": feed_state(client, reg),
            "subscribed_now": m.first,
            "window_seconds": seconds(now.saturating_sub(m.floor)),
            "window_from": if window.is_some() { "request" } else { "last_read" },
            "covers_seconds": seconds(now.saturating_sub(m.covered_since_ms)),
            "clipped": m.clipped,
            "received": m.received,
            "held": m.held,
            "dropped": m.dropped,
            "new_since_last_read": m.new_since_last_read,
            "age_ms": m.newest_ms.map(|s| now.saturating_sub(s)),
            "in_window": m.in_window,
            "matched": m.matched,
            "returned": m.rows.len(),
            "ranked_by": q.rank_by,
            "date": m.rows.last().and_then(|p| date_of(&p.trade)),
            "prints": m.rows.iter().map(|p| json!({
                "contract": contract_of(&p.trade).map(ToString::to_string),
                "trade": object(&p.trade),
                "quote_before": p.quote_before.as_ref().map(object)
            })).collect::<Vec<_>>()
        }));
    }

    if name == "tape_stop" && args.get("root").is_none() {
        let sec = parse_sec(args)?;
        if !reg.market_held(sec) {
            return Err(ToolError::InvalidParams(format!(
                "no whole-market book is held on {}; tape_list shows what is",
                sec.as_str().to_ascii_lowercase()
            )));
        }
        let (done, failures) = close_on_feed(client, reg, vec![sec.full_trades()]);
        return Ok(json!({
            "sec_type": sec.as_str().to_ascii_lowercase(),
            "subscriptions_closed": done,
            "failed_to_close": failures
        }));
    }

    let (sec, contract) = parse_contract(args)?;
    match name {
        "tape_read" => {
            let kind = resolve_kind(sec, args.get("kind").and_then(|v: &Value| v.as_str()))?;
            let watch = args.get("watch").map(parse_clauses).transpose()?;
            ensure_streaming(client, reg)?;
            let (r, expired) = reg.read(
                &contract,
                kind,
                window,
                tail_rows(num_of("tail", TAIL)),
                watch,
                now,
            )?;
            close_expired(client, reg, expired, now);
            let sub = subscription(kind, &contract);
            if r.first {
                open_on_feed(client, reg, &[sub])?;
            } else {
                reconcile(client, reg, &[sub], now)?;
            }
            let newest = r.tail.last();
            let watch = (!r.watch.is_empty() || !r.watched.is_empty()).then(|| {
                json!({
                    "clauses": r.watch.iter().map(clause_json).collect::<Vec<_>>(),
                    "checked": r.checked,
                    "matched": r.watched.iter().map(|d| aged_object(d, now)).collect::<Vec<_>>(),
                    "dropped": r.watched_dropped
                })
            });
            Ok(json!({
                "contract": contract.to_string(),
                "kind": kind.kind_str(),
                "feed": feed_state(client, reg),
                "subscribed_now": r.first,
                "window_seconds": seconds(now.saturating_sub(r.floor)),
                "window_from": if window.is_some() { "request" } else { "last_read" },
                "covers_seconds": seconds(now.saturating_sub(r.covered_since_ms)),
                "clipped": r.clipped,
                "dropped": r.dropped,
                "new_since_last_read": r.new_since_last_read,
                "age_ms": r.newest_ms.map(|s| now.saturating_sub(s)),
                "summary": summary_json(&r.summary),
                "watch": watch,
                "vendor_ohlcvc": r.ohlcvc.as_ref().map(|bar| aged_object(bar, now)),
                "date": newest.and_then(date_of),
                "columns": newest.map(|d| fields(d).into_iter().map(|(k, _)| k).collect::<Vec<_>>()),
                "tail": r.tail.iter().map(row).collect::<Vec<_>>()
            }))
        }
        "tape_prints" => {
            // A print needs both legs; an index offers neither quote nor
            // print, and the refusal names what it does offer.
            resolve_kind(sec, Some("quote"))?;
            let with_quotes_after = args
                .get("quotes_after")
                .and_then(|v: &Value| v.as_bool())
                .unwrap_or(false);
            let count = num_of("count", 20);
            ensure_streaming(client, reg)?;
            let (p, expired) = reg.prints(&contract, count, now)?;
            close_expired(client, reg, expired, now);
            let (opened, kept): (Subs, Subs) = [SubscriptionKind::Trade, SubscriptionKind::Quote]
                .into_iter()
                .map(|k| subscription(k, &contract))
                .partition(|s| p.opened.iter().any(|k| subscription(*k, &contract) == *s));
            open_on_feed(client, reg, &opened)?;
            reconcile(client, reg, &kept, now)?;
            Ok(json!({
                "contract": contract.to_string(),
                "feed": feed_state(client, reg),
                "subscribed_now": p.opened.iter().map(|k| k.kind_str()).collect::<Vec<_>>(),
                "count": p.rows.len(),
                "held": p.held,
                "clipped": p.held < count && p.dropped > 0,
                "covers_seconds": seconds(now.saturating_sub(p.covered_since_ms)),
                "new_since_last_read": p.new_since_last_read,
                "age_ms": p.newest_ms.map(|s| now.saturating_sub(s)),
                "date": p.rows.last().and_then(|p| date_of(&p.trade)),
                "prints": p.rows.iter().map(|p| {
                    let mut out = json!({
                        "trade": object(&p.trade),
                        "quote_before": p.quote_before.as_ref().map(object)
                    });
                    if with_quotes_after {
                        if let Some(obj) = out.as_object_mut() {
                            obj.insert("quotes_after", Value::from(p.quotes_after.iter().map(object).collect::<Vec<_>>()));
                        }
                    }
                    out
                }).collect::<Vec<_>>()
            }))
        }
        "tape_stop" => {
            let (held, expired) = reg.held_for(&contract, now);
            close_expired(client, reg, expired, now);
            if held.is_empty() {
                return Err(ToolError::InvalidParams(format!(
                    "{contract} is not held; tape_list shows what is"
                )));
            }
            let (done, failures) = close_on_feed(client, reg, held);
            Ok(json!({
                "contract": contract.to_string(),
                "subscriptions_closed": done,
                "failed_to_close": failures
            }))
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
        trade_sized(c, price, 1, condition, received_at_ns)
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

    fn condition(d: &StreamData) -> i32 {
        match d {
            StreamData::Trade { condition, .. } => *condition,
            other => panic!("not a trade: {other:?}"),
        }
    }

    fn clauses(v: Value) -> Vec<Clause> {
        parse_clauses(&v).expect("a valid predicate")
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
        reg.read(c, kind, window, tail, None, now)
            .expect("nothing to refuse")
    }

    fn prints(reg: &Registry, c: &Contract, count: usize, now: u64) -> (Prints, Subs) {
        reg.prints(c, count, now).expect("nothing to refuse")
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
    fn the_first_read_opens_the_book_and_the_next_does_not() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (a, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1_000);
        let (b, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1_001);
        assert!(a.first, "the first read subscribes");
        assert!(!b.first, "the second must not re-subscribe");
        assert_eq!(
            reg.subscriptions(),
            (vec![(SubscriptionKind::Trade, c)], vec![]),
            "one book, one subscription"
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
        assert!(!r.clipped, "the book was open for the whole window");

        // A fixed lookback does not change what "new" means.
        reg.ingest(trade(&c, 3_500.0, 3_500 * MS));
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(10_000), TAIL, 4_000);
        assert_eq!(r.summary.count, 3, "the fixed window sees everything");
        assert_eq!(
            r.new_since_last_read, 1,
            "but only one row arrived since the read at 3000"
        );
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(600), 1, 4_000);
        assert_eq!(
            r.summary.count, 1,
            "a 600 ms window at 4000 holds only the row at 3500"
        );
        assert_eq!(
            r.new_since_last_read, 0,
            "nothing arrived since the read a moment ago"
        );
    }

    #[test]
    fn the_summary_tags_each_trade_extreme_with_its_condition_and_counts_by_code() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade_with(&c, 100.0, 0, 10 * MS));
        reg.ingest(trade_with(&c, 95.0, 37, 20 * MS));
        reg.ingest(trade_with(&c, 105.0, 12, 30 * MS));
        reg.ingest(trade_with(&c, 101.0, 0, 40 * MS));

        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, 2, 50);
        let s = &r.summary;
        assert_eq!(s.count, 4);
        assert_eq!(s.first.as_ref().map(price), Some(100.0));
        assert_eq!(s.last.as_ref().map(price), Some(101.0));
        assert_eq!(
            (s.low.as_ref().map(price), s.low.as_ref().map(condition)),
            (Some(95.0), Some(37)),
            "the low carries the condition it printed under"
        );
        assert_eq!(
            (s.high.as_ref().map(price), s.high.as_ref().map(condition)),
            (Some(105.0), Some(12))
        );
        assert_eq!(
            s.conditions,
            BTreeMap::from([(0, 2), (12, 1), (37, 1)]),
            "every code seen, with its count"
        );
        assert_eq!(
            r.tail.iter().map(price).collect::<Vec<_>>(),
            vec![105.0, 101.0],
            "the tail is the newest rows, oldest first, and the summary still saw all four"
        );
    }

    #[test]
    fn the_summary_spans_bid_and_ask_on_a_quote_book() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(quote(&c, 0.98, 1.12));
        reg.ingest(quote(&c, 1.01, 1.09));

        let (r, _) = read(&reg, &c, SubscriptionKind::Quote, Some(1), TAIL, 1);
        assert_eq!(r.summary.bid, Some((0.98, 1.01)));
        assert_eq!(r.summary.ask, Some((1.09, 1.12)));
        assert!(r.summary.low.is_none(), "a quote has no trade extremes");
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
        assert_eq!(
            r.summary.count, RING as u64,
            "the summary saw the whole ring"
        );

        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, Some(20), TAIL, now);
        assert_eq!(
            r.summary.count, 11,
            "rows from 4096 to 4106 lie inside the last 20 ms"
        );
        assert!(!r.clipped, "a window inside coverage is whole");

        // A book younger than the window is clipped too, with nothing dropped.
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
            "nothing dropped, so coverage starts where the book opened, not at its first row"
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
            p.new_since_last_read, PRINTS as u64,
            "every held print arrived after the read at 1000"
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
    fn an_idle_book_is_buried_before_the_read_and_reopened() {
        // The sweep runs first, so a book that outlived its window is not
        // revived by the read that noticed: the read gets a fresh one and the
        // old subscription comes back to be closed.
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&c, 1.0, 0));

        let (r, expired) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert!(r.first, "a fresh book");
        assert_eq!(expired, vec![c.trade()]);
        assert_eq!(r.summary.count, 0, "and nothing carried over");
    }

    #[test]
    fn stopping_a_contract_closes_each_kind_on_the_feed_before_the_book_goes() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        read(&reg, &c, SubscriptionKind::Quote, None, TAIL, 0);
        read(&reg, &stock("MSFT"), SubscriptionKind::Trade, None, TAIL, 0);

        let (held, _) = reg.held_for(&c, 1);
        assert_eq!(held.len(), 2, "both legs come back to be unsubscribed");
        // The feed lets one go and keeps the other: only the first is dropped.
        reg.forget(&held[0]);
        assert_eq!(
            reg.held_for(&c, 1).0,
            vec![held[1].clone()],
            "the leg the feed kept is still held, for the next stop to retry"
        );
        reg.forget(&held[1]);
        let (rows, _) = reg.list(2);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].sub,
            stock("MSFT").trade(),
            "the other contract is untouched"
        );
        assert!(
            reg.held_for(&c, 3).0.is_empty(),
            "stopping again finds nothing"
        );
    }

    #[test]
    fn a_failed_subscribe_is_forgotten_so_the_next_read_tries_again() {
        let reg = Registry::default();
        let c = stock("AAPL");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.forget(&c.trade());
        let (r, _) = read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 1);
        assert!(r.first, "a book the feed refused must not look held");
    }

    #[test]
    fn a_book_the_feed_dropped_is_released_with_the_feeds_last_word_on_why() {
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
            "the book is released, so the next read re-subscribes"
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

        // A book someone reopened meanwhile is not reset to expired.
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
        assert!(expired.is_empty(), "the live book keeps its read time");

        // The same for a market book.
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
        reg.market(SecType::Option, &query(1), None, 0)
            .expect("nothing to refuse");

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
            "the market book restores as the SDK's full-stream tuple"
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
        // Open the book first: ingest drops a tick for a contract nobody
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
        assert_eq!(r.summary.count, 0, "a bar is not a row on the quote book");
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
    fn a_row_for_an_unheld_book_creates_nothing() {
        let reg = Registry::default();
        read(&reg, &stock("AAPL"), SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&stock("MSFT"), 5.0, 0));
        reg.ingest(quote(&stock("AAPL"), 1.0, 1.1));
        let held = reg.lock();
        assert_eq!(held.contracts.len(), 1, "no book invented for MSFT");
        assert_eq!(
            held.contracts[&stock("AAPL")].books.len(),
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
        assert_eq!(row(&d).as_array().map(|a| a.len()), Some(cols.len()));
        assert_eq!(clock(34_200_000), "09:30:00.000");
        assert_eq!(clock(57_600_123), "16:00:00.123");
        assert_eq!(
            object(&d).get("time").and_then(|v: &Value| v.as_str()),
            Some("09:30:00.000")
        );
    }

    #[test]
    fn every_column_a_row_renders_is_a_field_a_predicate_can_read() {
        // A column the model sees in a tail must be one it can compare on,
        // and a name the schema offers must read something on the row it
        // belongs to. The bar is left out: it is never a ring row, so no
        // predicate runs on it.
        let c = stock("AAPL");
        let rows = [
            quote(&c, 1.0, 1.1),
            trade(&c, 1.0, 0),
            StreamData::OpenInterest {
                contract: Arc::new(c.clone()),
                ms_of_day: 0,
                open_interest: 5,
                date: 20260915,
                received_at_ns: 0,
            },
            StreamData::MarketValue {
                contract: Arc::new(c.clone()),
                ms_of_day: 0,
                market_bid: 1.0,
                market_ask: 1.1,
                market_price: 1.05,
                date: 20260915,
                received_at_ns: 0,
            },
        ];
        let mut readable = Vec::new();
        for d in &rows {
            for (col, _) in fields(d) {
                if col == "time" {
                    continue;
                }
                assert!(
                    FIELDS.contains(&col),
                    "{col} is rendered but cannot be compared on"
                );
                assert!(
                    field_of(d, col).is_some(),
                    "{col} reads nothing on its own row"
                );
                readable.push(col);
            }
        }
        readable.push("spread");
        for name in FIELDS {
            assert!(
                readable.contains(&name),
                "{name} is offered but reads nothing anywhere"
            );
        }
        assert_eq!(
            field_of(&quote(&c, 1.0, 1.25), "spread"),
            Some(0.25),
            "spread is ask minus bid"
        );
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
            assert!(parse_clauses(&bad).is_err(), "accepted {bad}");
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
    fn a_watched_row_is_kept_past_the_ring_and_served_on_the_next_read() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let big = clauses(json!([{"field": "size", "op": ">", "value": 100}]));
        let (r, _) = reg
            .read(
                &c,
                SubscriptionKind::Trade,
                None,
                TAIL,
                Some(big.clone()),
                0,
            )
            .expect("nothing to refuse");
        assert_eq!(r.watch, big, "the predicate is in force from this read");
        assert!(r.watched.is_empty() && r.checked == 0);

        // One large print, then enough small ones to push it off the ring.
        reg.ingest(trade_sized(&c, 10.0, 500, 0, MS));
        for i in 2..=(RING as u64 + 1) {
            reg.ingest(trade_sized(&c, 10.0, 1, 0, i * MS));
        }
        let (r, _) = read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            RING as u64 + 2,
        );
        assert_eq!(r.dropped, 1, "the large print is gone from the ring");
        assert_eq!(
            r.watched
                .iter()
                .map(|d| field_of(d, "size"))
                .collect::<Vec<_>>(),
            vec![Some(500.0)],
            "and is served anyway, because it matched when it arrived"
        );
        assert_eq!(r.checked, RING as u64 + 1, "every row was examined");
        assert_eq!(
            r.watch, big,
            "a read without watch leaves the predicate alone"
        );

        let (r, _) = read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            RING as u64 + 3,
        );
        assert!(r.watched.is_empty(), "served once");
        assert_eq!(r.checked, 0, "nothing arrived since");

        // More matches than are kept: the newest survive, the loss is counted.
        for i in 0..(WATCHED as u64 + 3) {
            reg.ingest(trade_sized(
                &c,
                i as f64,
                500,
                0,
                (RING as u64 + 10 + i) * MS,
            ));
        }
        let (r, _) = reg
            .read(
                &c,
                SubscriptionKind::Trade,
                None,
                TAIL,
                Some(Vec::new()),
                RING as u64 + 100,
            )
            .expect("nothing to refuse");
        assert_eq!(r.watched.len(), WATCHED);
        assert_eq!(r.watched_dropped, 3);
        assert_eq!(
            r.watched.first().map(price),
            Some(3.0),
            "the oldest kept is the fourth"
        );
        assert!(r.watch.is_empty(), "[] clears the predicate");
        reg.ingest(trade_sized(&c, 1.0, 500, 0, (RING as u64 + 200) * MS));
        let (r, _) = read(
            &reg,
            &c,
            SubscriptionKind::Trade,
            None,
            TAIL,
            RING as u64 + 201,
        );
        assert!(
            r.watched.is_empty() && r.checked == 0,
            "nothing watched, nothing kept"
        );
    }

    #[test]
    fn the_market_book_takes_every_trade_on_its_security_type_with_the_quote_sent_ahead() {
        let reg = Registry::default();
        let call = option("550", "C");
        let put = option("540", "P");
        let (m, _) = reg
            .market(SecType::Option, &query(10), None, 0)
            .expect("nothing to refuse");
        assert!(m.first, "the first look opens it");

        // The vendor sends the contract's NBBO and bar, then its trade.
        reg.ingest(quote(&call, 1.0, 1.1));
        reg.ingest(bar(&call, 1.05, MS));
        reg.ingest(trade(&call, 1.05, MS));
        // Another contract's quote in between: the trade does not claim it.
        reg.ingest(quote(&put, 2.0, 2.1));
        reg.ingest(trade(&call, 1.06, 2 * MS));
        // A stock trade belongs to a market book this registry does not hold.
        reg.ingest(trade(&stock("AAPL"), 150.0, 3 * MS));

        let (m, _) = reg
            .market(SecType::Option, &query(10), None, 10)
            .expect("nothing to refuse");
        assert!(!m.first);
        assert_eq!(m.received, 2, "two option trades, no stock");
        assert_eq!(m.new_since_last_read, 2);
        assert_eq!((m.in_window, m.matched, m.rows.len()), (2, 2, 2));
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
            "a market row opens no per-contract book"
        );

        // The ring is bounded and says what it lost.
        for i in 0..(MARKET_RING as u64 + 5) {
            reg.ingest(trade(&call, 1.0, (100 + i) * MS));
        }
        let (m, _) = reg
            .market(SecType::Option, &query(1), Some(1_000_000), 100_000)
            .expect("nothing to refuse");
        assert_eq!((m.held, m.dropped), (MARKET_RING, 7));
        assert_eq!(
            m.covered_since_ms, 105,
            "coverage starts at the oldest print held"
        );
        assert!(m.clipped);
    }

    #[test]
    fn a_market_read_selects_on_the_contract_and_the_fields_and_ranks_what_passes() {
        let reg = Registry::default();
        reg.market(SecType::Option, &query(10), None, 0)
            .expect("nothing to refuse");
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
        reg.ingest(quote(&c550, 1.0, 1.5));
        reg.ingest(trade_sized(&c550, 1.2, 10, 0, MS));
        reg.ingest(trade_sized(&c540, 5.0, 300, 0, 2 * MS));
        reg.ingest(trade_sized(&p550, 0.9, 200, 0, 3 * MS));
        reg.ingest(trade_sized(&c550, 1.3, 50, 0, 4 * MS));
        reg.ingest(trade_sized(&qqq, 2.0, 999, 0, 5 * MS));

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
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 10)
            .expect("nothing to refuse");
        assert_eq!(
            (m.in_window, m.matched),
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
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 11)
            .expect("nothing to refuse");
        assert_eq!(
            sizes(&m),
            vec![Some(10.0)],
            "only the print with a quote ahead of it has a spread"
        );

        // Rank, direction, cut — and a rank field the row lacks sorts last.
        let mut q = query(2);
        q.rank_by = Some("size".into());
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 12)
            .expect("nothing to refuse");
        assert_eq!(
            (m.matched, m.rows.len()),
            (5, 2),
            "matched says what the cut hid"
        );
        assert_eq!(sizes(&m), vec![Some(999.0), Some(300.0)], "largest first");
        q.ascending = true;
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 13)
            .expect("nothing to refuse");
        assert_eq!(sizes(&m), vec![Some(10.0), Some(50.0)]);
        let mut q = query(5);
        q.rank_by = Some("bid".into());
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 14)
            .expect("nothing to refuse");
        assert_eq!(
            sizes(&m)[0],
            Some(10.0),
            "the one print with a bid ranks ahead of those without"
        );

        // The window is the population.
        let (m, _) = reg
            .market(SecType::Option, &query(10), Some(2), 6)
            .expect("nothing to refuse");
        assert_eq!(
            (m.in_window, m.matched),
            (2, 2),
            "two prints inside the last 2 ms"
        );
        let mut q = query(10);
        q.expiration = Some(20260620);
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 15)
            .expect("nothing to refuse");
        assert_eq!(m.matched, 5);
        q.expiration = Some(20260621);
        let (m, _) = reg
            .market(SecType::Option, &q, Some(100), 16)
            .expect("nothing to refuse");
        assert_eq!(m.matched, 0);
    }

    #[test]
    fn a_market_book_and_a_per_contract_book_on_the_same_class_are_not_held_together() {
        let reg = Registry::default();
        let c = option("550", "C");
        read(&reg, &c, SubscriptionKind::Trade, None, TAIL, 0);
        let why = refused(reg.market(SecType::Option, &query(1), None, 1));
        assert!(
            why.contains("1 OPTION contract"),
            "names the conflict: {why}"
        );
        assert!(!reg.market_held(SecType::Option), "and opens nothing");
        reg.market(SecType::Stock, &query(1), None, 2)
            .expect("another class is no conflict");

        reg.forget(&c.trade());
        reg.market(SecType::Option, &query(1), None, 3)
            .expect("nothing to refuse");
        let why = refused(reg.read(&c, SubscriptionKind::Quote, None, TAIL, None, 4));
        assert!(why.contains("tape_market"), "names the other side: {why}");
        assert!(
            reg.prints(&c, 1, 5).is_err(),
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
                .is_some_and(|s| s.books.len() == 1),
            "kinds the full stream does not carry are held alongside it, as is another class"
        );
    }

    #[test]
    fn an_idle_market_book_is_swept_and_stopped_by_its_class() {
        let reg = Registry::default();
        reg.market(SecType::Stock, &query(1), None, 0)
            .expect("nothing to refuse");
        let spx = Contract::index("SPX");
        let (_, expired) = read(&reg, &spx, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert_eq!(expired, vec![SecType::Stock.full_trades()]);
        assert!(!reg.market_held(SecType::Stock));

        reg.market(SecType::Stock, &query(1), None, PAST_TTL)
            .expect("nothing to refuse");
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
                && why.contains("tape_read"),
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
