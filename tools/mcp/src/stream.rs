//! Live-state views over the streaming feed.
//!
//! A model cannot read a feed: ticks arrive orders of magnitude faster than
//! tokens. What works is holding the subscription and answering questions
//! about it — what is the value now, what did the window look like, what
//! printed and where the market was around it.
//!
//! A book is named by its contract and kind, the same way the vendor names a
//! subscription. The first read opens it; a read every so often keeps it;
//! fifteen idle minutes or `stream_stop` close it. There is no handle to
//! mint, pass back or lose.
//!
//! This layer stores what the feed sends and serves it back. The summary on
//! a read counts and sorts the rows it saw and says which population it saw
//! them in; it does not aggregate. Bar construction has condition, cancel and
//! size rules that are the caller's to choose, and a bar built on assumptions
//! here would not reconcile with one built anywhere else.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonic_rs::{json, JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, OptionLeg, StreamControl, StreamData, StreamEvent, Subscription, SubscriptionKind,
};
use thetadatadx::{Client, ConnectionStatus, SecType, StreamMsgType};

use crate::{sanitize_error, ToolError};

/// Rows retained per book. A busy contract prints far more than this in a
/// session; the tail is what a model can act on and the rest is weight.
const RING: usize = 4_096;
/// Prints retained per contract. Fewer than rows: each one holds up to four
/// messages.
const PRINTS: usize = 256;
/// How long a book survives without a read. Stated in the tool descriptions,
/// since that is where a model reads it.
const TTL: Duration = Duration::from_secs(900);
/// Newest rows served verbatim on a read, unless asked otherwise.
const TAIL: usize = 10;

/// Subscriptions to open or close on the feed, in the SDK's own tuple order.
type Subs = Vec<(SubscriptionKind, Contract)>;

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

#[derive(Debug)]
struct Book {
    ring: VecDeque<StreamData>,
    received: u64,
    dropped: u64,
    opened_ms: u64,
    read_ms: u64,
}

impl Book {
    fn open(now: u64) -> Self {
        Self {
            ring: VecDeque::new(),
            received: 0,
            dropped: 0,
            opened_ms: now,
            read_ms: now,
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

type Held = HashMap<Contract, ContractState>;

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

struct Holding {
    contract: Contract,
    kind: SubscriptionKind,
    received: u64,
    held: usize,
    dropped: u64,
    opened_ms: u64,
    read_ms: u64,
    newest_ms: Option<u64>,
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
        held.retain(|contract, state| {
            state.books.retain(|(kind, book)| {
                let live = now.saturating_sub(book.read_ms) <= ttl;
                if !live {
                    freed.push((*kind, contract.clone()));
                }
                live
            });
            !state.books.is_empty()
        });
        freed
    }

    /// Read a book, opening it if this is the first look. `window` is a
    /// lookback in milliseconds; without one the window is everything since
    /// the previous read. Returns what the sweep freed alongside.
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
        now: u64,
    ) -> (Reading, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let (book, first) = held.entry(contract.clone()).or_default().open(kind, now);
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

        let covered_since_ms = book
            .ring
            .front()
            .and_then(seen_ms)
            .filter(|_| book.dropped > 0)
            .unwrap_or(book.opened_ms);
        let reading = Reading {
            first,
            floor,
            dropped: book.dropped,
            covered_since_ms,
            newest_ms: book.ring.back().and_then(seen_ms),
            clipped: covered_since_ms > floor,
            new_since_last_read,
            summary,
            tail: rows,
        };
        (reading, expired)
    }

    /// The newest `count` prints, oldest first. Opens the trade and quote
    /// books a print is built from when they are not already held.
    fn prints(&self, contract: &Contract, count: usize, now: u64) -> (Prints, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let state = held.entry(contract.clone()).or_default();
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
        (prints, expired)
    }

    /// Close every book held for a contract. Returns their subscriptions,
    /// then what the sweep freed.
    fn stop(&self, contract: &Contract, now: u64) -> (Subs, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let closed = held.remove(contract).map_or_else(Vec::new, |s| {
            s.books
                .into_iter()
                .map(|(k, _)| (k, contract.clone()))
                .collect()
        });
        (closed, expired)
    }

    /// Drop a book whose subscription could not be opened. Leaving it would
    /// make the next read on this contract skip subscribing too.
    fn forget(&self, contract: &Contract, kind: SubscriptionKind) {
        let mut held = self.lock();
        if let Some(state) = held.get_mut(contract) {
            state.books.retain(|(k, _)| *k != kind);
            if state.books.is_empty() {
                held.remove(contract);
            }
        }
    }

    fn list(&self, now: u64) -> (Vec<Holding>, Subs) {
        let mut held = self.lock();
        let expired = Self::sweep(&mut held, now);
        let mut rows: Vec<Holding> = held
            .iter()
            .flat_map(|(c, s)| {
                s.books.iter().map(move |(k, b)| Holding {
                    contract: c.clone(),
                    kind: *k,
                    received: b.received,
                    held: b.ring.len(),
                    dropped: b.dropped,
                    opened_ms: b.opened_ms,
                    read_ms: b.read_ms,
                    newest_ms: b.ring.back().and_then(seen_ms),
                })
            })
            .collect();
        rows.sort_by_key(|h| (h.contract.to_string(), h.kind.kind_str()));
        (rows, expired)
    }

    /// Every subscription the books hold, once each: what a fresh session
    /// must reopen.
    fn subscriptions(&self) -> Subs {
        self.lock()
            .iter()
            .flat_map(|(c, s)| s.books.iter().map(move |(k, _)| (*k, c.clone())))
            .collect()
    }

    /// Store a row. Unknown books are ignored rather than created: the feed
    /// can deliver a contract after its unsubscribe, and inventing a book for
    /// one would leak.
    pub fn ingest(&self, data: StreamData) {
        let (Some(contract), Some(msg)) = (contract_of(&data), msg_type_of(&data)) else {
            return;
        };
        let mut held = self.lock();
        let Some(state) = held.get_mut(contract) else {
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

pub const TOOL_NAMES: [&str; 4] = ["stream_read", "stream_prints", "stream_list", "stream_stop"];

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

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "stream_read",
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
                messages as sent, condition and exchange codes intact; nothing is built into \
                bars, because condition, cancel and size rules are yours to choose. kind \
                defaults to quote; an index has no quote stream, so it defaults to trade, which \
                carries the index price. market_value is a derived midpoint, not a quote. Times \
                are Eastern. A book unread for 15 minutes closes on its own.",
            "inputSchema": contract_schema(json!({
                "kind": {"type": "string", "enum": ["quote", "trade", "market_value", "open_interest"],
                         "description": "Default quote, or trade for an index."},
                "seconds": {"type": "number", "description": "Fixed lookback. Default: since your last read."},
                "tail": {"type": "integer", "description": "Newest rows served verbatim. Default 10."}
            }))
        }),
        json!({
            "name": "stream_prints",
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
            "name": "stream_list",
            "description": "Every book this server holds: rows received and held, the age of \
                the newest, how long since it was read and when it expires, plus the feed's \
                state. An age that keeps growing while the feed says Connected is a contract \
                that has gone quiet, not a fault; anything else and the next stream_read \
                restarts the feed.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "stream_stop",
            "description": "Close every book held for a contract and release its subscriptions. \
                Books also close on their own after 15 minutes without a read.",
            "inputSchema": contract_schema(json!({}))
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
            _ => {}
        })
        .map_err(|e| stream_error("could not start streaming", e))?;
    stream
        .restore_subscriptions(&reg.subscriptions(), &[])
        .map_err(|e| stream_error("could not reopen every subscription", e))
}

fn parse_contract(args: &Value) -> Result<(SecType, Contract), ToolError> {
    let str_of = |k: &str| args.get(k).and_then(|v: &Value| v.as_str());
    let sec = match str_of("sec_type") {
        Some("option") => SecType::Option,
        Some("stock") => SecType::Stock,
        Some("index") => SecType::Index,
        _ => {
            return Err(ToolError::InvalidParams(
                "sec_type must be option, stock or index".into(),
            ))
        }
    };
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

fn subscription(kind: SubscriptionKind, contract: &Contract) -> Subscription {
    Subscription::Contract {
        contract: contract.clone(),
        kind,
    }
}

/// Close what the sweep freed, so a forgotten book cannot hold an allowance
/// the next caller needs. Nobody is waiting on these, so a failure is logged
/// rather than returned.
fn close_expired(client: &Client, expired: Subs) {
    for (kind, contract) in expired {
        if let Err(e) = client.stream().unsubscribe(subscription(kind, &contract)) {
            tracing::warn!(%contract, ?kind, error = %e, "expired book left its subscription open");
        }
    }
}

/// Open on the feed what a read just opened in the registry, rolling the
/// book back if the feed refuses.
fn open_on_feed(
    client: &Client,
    reg: &Registry,
    contract: &Contract,
    kinds: &[SubscriptionKind],
) -> Result<(), ToolError> {
    for kind in kinds {
        if let Err(e) = client.stream().subscribe(subscription(*kind, contract)) {
            reg.forget(contract, *kind);
            return Err(stream_error(
                &format!("could not subscribe {}", kind.kind_str()),
                e,
            ));
        }
    }
    Ok(())
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

    if name == "stream_list" {
        let (rows, expired) = reg.list(now);
        close_expired(client, expired);
        return Ok(json!({
            "feed": feed_state(client, reg),
            "held": rows.iter().map(|h| json!({
                "contract": h.contract.to_string(),
                "kind": h.kind.kind_str(),
                "received": h.received,
                "held": h.held,
                "dropped": h.dropped,
                "age_ms": h.newest_ms.map(|s| now.saturating_sub(s)),
                "open_for_seconds": seconds(now.saturating_sub(h.opened_ms)),
                "idle_seconds": seconds(now.saturating_sub(h.read_ms)),
                "expires_in_seconds": seconds((TTL.as_millis() as u64).saturating_sub(now.saturating_sub(h.read_ms)))
            })).collect::<Vec<_>>()
        }));
    }

    let (sec, contract) = parse_contract(args)?;
    match name {
        "stream_read" => {
            let kind = resolve_kind(sec, args.get("kind").and_then(|v: &Value| v.as_str()))?;
            let window = args
                .get("seconds")
                .and_then(|v: &Value| v.as_f64())
                .map(|s| (s.max(0.0) * 1_000.0) as u64);
            ensure_streaming(client, reg)?;
            let (r, expired) = reg.read(&contract, kind, window, num_of("tail", TAIL), now);
            close_expired(client, expired);
            if r.first {
                open_on_feed(client, reg, &contract, &[kind])?;
            }
            let newest = r.tail.last();
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
                "date": newest.and_then(date_of),
                "columns": newest.map(|d| fields(d).into_iter().map(|(k, _)| k).collect::<Vec<_>>()),
                "tail": r.tail.iter().map(row).collect::<Vec<_>>()
            }))
        }
        "stream_prints" => {
            // A print needs both legs; an index offers neither quote nor
            // print, and the refusal names what it does offer.
            resolve_kind(sec, Some("quote"))?;
            let with_quotes_after = args
                .get("quotes_after")
                .and_then(|v: &Value| v.as_bool())
                .unwrap_or(false);
            let count = num_of("count", 20);
            ensure_streaming(client, reg)?;
            let (p, expired) = reg.prints(&contract, count, now);
            close_expired(client, expired);
            open_on_feed(client, reg, &contract, &p.opened)?;
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
        "stream_stop" => {
            let (closed, expired) = reg.stop(&contract, now);
            close_expired(client, expired);
            if closed.is_empty() {
                return Err(ToolError::InvalidParams(format!(
                    "{contract} is not held; stream_list shows what is"
                )));
            }
            let mut done = 0usize;
            let mut failures = Vec::new();
            for (kind, contract) in closed {
                match client.stream().unsubscribe(subscription(kind, &contract)) {
                    Ok(()) => done += 1,
                    // Report it. Claiming a close that did not happen tells the
                    // caller an allowance came back while the feed still holds it.
                    Err(e) => failures.push(format!(
                        "{}: {}",
                        kind.kind_str(),
                        sanitize_error(&e.to_string())
                    )),
                }
            }
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

    fn trade_with(c: &Contract, price: f64, condition: i32, received_at_ns: u64) -> StreamData {
        StreamData::Trade {
            contract: Arc::new(c.clone()),
            ms_of_day: 34_200_000,
            sequence: 1,
            condition,
            size: 1,
            exchange: 0,
            price,
            date: 20260915,
            received_at_ns,
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

    const MS: u64 = 1_000_000;
    const PAST_TTL: u64 = TTL.as_millis() as u64 + 1;

    #[test]
    fn the_first_read_opens_the_book_and_the_next_does_not() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (a, _) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, 1_000);
        let (b, _) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, 1_001);
        assert!(a.first, "the first read subscribes");
        assert!(!b.first, "the second must not re-subscribe");
        assert_eq!(
            reg.subscriptions(),
            vec![(SubscriptionKind::Trade, c)],
            "one book, one subscription"
        );
    }

    #[test]
    fn a_read_defaults_to_since_the_last_read_and_counts_what_is_new() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 1_000);
        reg.ingest(trade(&c, 1_500.0, 1_500 * MS));
        reg.ingest(trade(&c, 2_500.0, 2_500 * MS));

        let (r, _) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, 3_000);
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
        let (r, _) = reg.read(&c, SubscriptionKind::Trade, Some(10_000), TAIL, 4_000);
        assert_eq!(r.summary.count, 3, "the fixed window sees everything");
        assert_eq!(
            r.new_since_last_read, 1,
            "but only one row arrived since the read at 3000"
        );
        let (r, _) = reg.read(&c, SubscriptionKind::Trade, Some(600), 1, 4_000);
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
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade_with(&c, 100.0, 0, 10 * MS));
        reg.ingest(trade_with(&c, 95.0, 37, 20 * MS));
        reg.ingest(trade_with(&c, 105.0, 12, 30 * MS));
        reg.ingest(trade_with(&c, 101.0, 0, 40 * MS));

        let (r, _) = reg.read(&c, SubscriptionKind::Trade, None, 2, 50);
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
        reg.read(&c, SubscriptionKind::Quote, None, TAIL, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(quote(&c, 0.98, 1.12));
        reg.ingest(quote(&c, 1.01, 1.09));

        let (r, _) = reg.read(&c, SubscriptionKind::Quote, Some(1), TAIL, 1);
        assert_eq!(r.summary.bid, Some((0.98, 1.01)));
        assert_eq!(r.summary.ask, Some((1.09, 1.12)));
        assert!(r.summary.low.is_none(), "a quote has no trade extremes");
    }

    #[test]
    fn coverage_is_the_oldest_row_held_and_clipped_says_when_the_window_reaches_past_it() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        // One row per millisecond, ten more than the ring holds.
        for ms in 1..=(RING as u64 + 10) {
            reg.ingest(trade(&c, ms as f64, ms * MS));
        }
        let now = RING as u64 + 20;
        let oldest_held = 11;

        let (r, _) = reg.read(&c, SubscriptionKind::Trade, Some(now), TAIL, now);
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

        let (r, _) = reg.read(&c, SubscriptionKind::Trade, Some(20), TAIL, now);
        assert_eq!(
            r.summary.count, 11,
            "rows from 4096 to 4106 lie inside the last 20 ms"
        );
        assert!(!r.clipped, "a window inside coverage is whole");

        // A book younger than the window is clipped too, with nothing dropped.
        let young = stock("MSFT");
        reg.read(&young, SubscriptionKind::Trade, None, TAIL, now);
        reg.ingest(trade(&young, 1.0, (now + 1) * MS));
        let (r, _) = reg.read(&young, SubscriptionKind::Trade, Some(60_000), TAIL, now + 2);
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
        let (p, _) = reg.prints(&c, 10, 0);
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

        let (p, _) = reg.prints(&c, 10, 1);
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
        reg.prints(&c, 10, 0);

        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 1));
        reg.ingest(quote(&c, 1.01, 1.11));
        reg.ingest(trade(&c, 1.09, 2));
        reg.ingest(quote(&c, 1.02, 1.12));
        reg.ingest(quote(&c, 1.03, 1.13));

        let (p, _) = reg.prints(&c, 10, 1);
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
        reg.prints(&c, 10, 1_000);
        for i in 1..=(PRINTS as u64 + 3) {
            reg.ingest(trade(&c, i as f64, (1_000 + i) * MS));
        }
        let (p, _) = reg.prints(&c, 5, 2_000);
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
        let (p, _) = reg.prints(&c, 5, 3_000);
        assert_eq!(
            p.new_since_last_read, 0,
            "nothing printed since the read at 2000"
        );
    }

    #[test]
    fn an_expired_quote_leg_does_not_erase_the_prints_the_trade_leg_still_serves() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.prints(&c, 10, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 0));

        // The trade leg is read inside its window; the quote leg is not.
        let (r, expired) = reg.read(
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
        let (_, expired) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert_eq!(
            expired,
            vec![(SubscriptionKind::Quote, c.clone())],
            "the idle quote leg is closed and handed back to unsubscribe"
        );

        let (p, _) = reg.prints(&c, 10, PAST_TTL + 1);
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
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&c, 1.0, 0));

        let (r, expired) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, PAST_TTL);
        assert!(r.first, "a fresh book");
        assert_eq!(expired, vec![(SubscriptionKind::Trade, c)]);
        assert_eq!(r.summary.count, 0, "and nothing carried over");
    }

    #[test]
    fn stopping_a_contract_frees_every_kind_it_holds() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.read(&c, SubscriptionKind::Quote, None, TAIL, 0);
        reg.read(&stock("MSFT"), SubscriptionKind::Trade, None, TAIL, 0);

        let (closed, _) = reg.stop(&c, 1);
        assert_eq!(closed.len(), 2, "both legs come back to be unsubscribed");
        let (rows, _) = reg.list(2);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].contract,
            stock("MSFT"),
            "the other contract is untouched"
        );
        assert!(reg.stop(&c, 3).0.is_empty(), "stopping again finds nothing");
    }

    #[test]
    fn a_failed_subscribe_is_forgotten_so_the_next_read_tries_again() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.forget(&c, SubscriptionKind::Trade);
        let (r, _) = reg.read(&c, SubscriptionKind::Trade, None, TAIL, 1);
        assert!(r.first, "a book the feed refused must not look held");
    }

    #[test]
    fn a_fresh_session_reopens_each_held_subscription_once() {
        let reg = Registry::default();
        let c = stock("AAPL");
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.read(&c, SubscriptionKind::Trade, None, TAIL, 0);
        reg.read(&c, SubscriptionKind::Quote, None, TAIL, 0);

        let mut subs = reg.subscriptions();
        subs.sort_by_key(|(k, _)| k.kind_str());
        assert_eq!(
            subs,
            vec![
                (SubscriptionKind::Quote, c.clone()),
                (SubscriptionKind::Trade, c)
            ],
            "two reads share one subscription; replaying it twice would double-subscribe"
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
    fn a_row_for_an_unheld_book_creates_nothing() {
        let reg = Registry::default();
        reg.read(&stock("AAPL"), SubscriptionKind::Trade, None, TAIL, 0);
        reg.ingest(trade(&stock("MSFT"), 5.0, 0));
        reg.ingest(quote(&stock("AAPL"), 1.0, 1.1));
        let held = reg.lock();
        assert_eq!(held.len(), 1, "no book invented for MSFT");
        assert_eq!(
            held[&stock("AAPL")].books.len(),
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
}
