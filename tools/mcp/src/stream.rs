//! Live-state views over the streaming feed.
//!
//! A model cannot read a feed. Ticks arrive orders of magnitude faster than
//! tokens, so the useful shape is not "forward the stream" but "hold the
//! stream and answer questions about it": what is the value now, what has the
//! last few minutes looked like, what do the bars say.
//!
//! The protocol has no session, so a subscription is named by a handle this
//! module mints and the caller passes back as an ordinary tool argument.
//!
//! Everything here is pure with respect to the network: [`Registry::ingest`]
//! takes a decoded tick and the readers take a handle, so the whole surface is
//! exercisable in tests without a connection.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Ticks retained per book. A busy option chain prints millions of rows a
/// day; the server keeps the tail, never the tape. Oldest is dropped first and
/// the drop is counted, so a reader can tell a complete window from a clipped
/// one instead of assuming.
pub const DEFAULT_RING: usize = 4_096;

/// How long a handle survives without a read before it is collected. Stated in
/// the creation tool's description, because that is where a model reads it
/// when deciding whether to create state.
pub const DEFAULT_TTL: Duration = Duration::from_secs(900);

/// Bars are only meaningful for trades: a quote has no traded size, so
/// aggregating one would invent volume that never happened.
const BAR_KINDS: [Kind; 1] = [Kind::Trade];

/// What a watch covers. The shapes are the vendor's, not ours: the bulk
/// full-trade stream and the per-contract streams are different products with
/// different tiers, and indices have neither a trade nor a quote stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// One option contract.
    Contract,
    /// One stock symbol.
    Equity,
    /// One index symbol. Price and market value only.
    Index,
}

impl Scope {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "contract" => Some(Self::Contract),
            "equity" => Some(Self::Equity),
            "index" => Some(Self::Index),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Equity => "equity",
            Self::Index => "index",
        }
    }

    /// Kinds this scope can actually carry.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Self::Contract | Self::Equity => &[Kind::Trade, Kind::Quote, Kind::MarketValue],
            // No index trade or quote stream exists. Price is its own kind and
            // arrives in a trade-shaped message despite not being trade data.
            Self::Index => &[Kind::Price, Kind::MarketValue],
        }
    }

    /// Why a kind is refused, phrased for a model that has to choose again.
    pub fn reject(self, kind: Kind) -> Option<String> {
        if self.kinds().contains(&kind) {
            return None;
        }
        let offered = self
            .kinds()
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Some(match (self, kind) {
            (Self::Index, Kind::Trade | Kind::Quote) => format!(
                "indices have no {} stream; this scope offers {offered}",
                kind.as_str()
            ),
            _ => format!("{} scope offers {offered}", self.as_str()),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Quote,
    Trade,
    /// Derived midpoint, not a quote. Never present it as NBBO.
    MarketValue,
    /// Index price changes. Reported about once a second, and only the price
    /// moves between reports, so staleness reads differently here than on a
    /// quote.
    Price,
}

impl Kind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "quote" => Some(Self::Quote),
            "trade" => Some(Self::Trade),
            "market_value" => Some(Self::MarketValue),
            "price" => Some(Self::Price),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Trade => "trade",
            Self::MarketValue => "market_value",
            Self::Price => "price",
        }
    }
}

/// One tick, flattened to what a reader needs. Prices stay `f64` as the feed
/// decodes them; the millisecond fields stay integers rather than becoming
/// timestamps, because every reader compares them and none of them format.
#[derive(Clone, Debug, PartialEq)]
pub enum Tick {
    Quote {
        ms_of_day: i32,
        date: i32,
        bid: f64,
        bid_size: i32,
        ask: f64,
        ask_size: i32,
    },
    Trade {
        ms_of_day: i32,
        date: i32,
        price: f64,
        size: i32,
        condition: i32,
        sequence: i32,
    },
    MarketValue {
        ms_of_day: i32,
        date: i32,
        /// Absent for an index, which reports a market price with no book.
        bid: Option<f64>,
        ask: Option<f64>,
        price: f64,
    },
}

impl Tick {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Quote { .. } => Kind::Quote,
            Self::Trade { .. } => Kind::Trade,
            Self::MarketValue { .. } => Kind::MarketValue,
        }
    }

    pub fn ms_of_day(&self) -> i32 {
        match *self {
            Self::Quote { ms_of_day, .. }
            | Self::Trade { ms_of_day, .. }
            | Self::MarketValue { ms_of_day, .. } => ms_of_day,
        }
    }
}

/// A tick with the wall-clock instant the server saw it, which is what makes
/// `age_ms` possible on a read.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamped {
    pub tick: Tick,
    pub seen_ms: u64,
}

/// A print with the market around it.
///
/// The bulk full-trade stream sends, for every trade: the last quote before
/// it and an OHLC for the contract, then the trade, then the next two quotes.
/// They arrive as separate messages, so the value only exists once something
/// correlates them. That is this.
#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    pub trade: Tick,
    /// The NBBO as it stood immediately before the print.
    pub quote_before: Option<Tick>,
    /// The next two NBBO updates after it. The second does not always arrive.
    pub quotes_after: Vec<Tick>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub start_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    pub trades: u64,
}

/// Enough of a contract to rebuild the subscription and to name the book.
///
/// The book index is a string because every read is a lookup by name, but a
/// subscription needs the parts back, so the parts are what is stored and the
/// name is derived from them.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Spec {
    pub sec_type: SecType,
    pub root: String,
    /// Option legs only.
    pub expiration: Option<u32>,
    /// Option legs only, in thousandths of a dollar, as `Contract` carries it
    /// (a $550 strike is 550_000).
    pub strike: Option<i64>,
    /// Option legs only: 'C' or 'P'.
    pub right: Option<char>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SecType {
    Option,
    Stock,
    Index,
}

impl Spec {
    pub fn equity(root: &str) -> Self {
        Self {
            sec_type: SecType::Stock,
            root: root.to_owned(),
            expiration: None,
            strike: None,
            right: None,
        }
    }

    pub fn index(root: &str) -> Self {
        Self {
            sec_type: SecType::Index,
            root: root.to_owned(),
            expiration: None,
            strike: None,
            right: None,
        }
    }

    pub fn option(root: &str, expiration: u32, strike: i64, right: char) -> Self {
        Self {
            sec_type: SecType::Option,
            root: root.to_owned(),
            expiration: Some(expiration),
            strike: Some(strike),
            right: Some(right),
        }
    }

    /// The book name. Stable, and the same string the ingest side derives from
    /// an incoming contract, which is what makes the two sides meet.
    pub fn symbol(&self) -> String {
        match (self.expiration, self.strike, self.right) {
            (Some(exp), Some(strike), Some(right)) => {
                format!("{} {exp} {right} {strike}", self.root)
            }
            _ => self.root.clone(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BookKey {
    pub symbol: String,
    pub kind: Kind,
}

#[derive(Debug)]
struct Book {
    /// How many live handles want this subscription. One feed subscription
    /// per key regardless of how many handles name it, so an agent that
    /// forgets to release does not burn the account's stream allowance.
    refs: usize,
    ring: VecDeque<Stamped>,
    bars: BTreeMap<i64, Bar>,
    dropped: u64,
    received: u64,
}

impl Book {
    fn new() -> Self {
        Self {
            refs: 0,
            ring: VecDeque::new(),
            bars: BTreeMap::new(),
            dropped: 0,
            received: 0,
        }
    }
}

#[derive(Debug)]
struct Watch {
    keys: Vec<BookKey>,
    /// What to subscribe and unsubscribe on the feed. The book index cannot
    /// carry this: it is a name, and a subscription needs the parts.
    specs: Vec<(Spec, Kind)>,
    scope: String,
    created_ms: u64,
    last_read_ms: u64,
}

#[derive(Debug, PartialEq)]
pub enum WatchError {
    /// The handle is unknown, or expired and was collected. Either way the
    /// caller's recovery is the same: create a new one.
    UnknownHandle,
    /// The handle exists but does not cover the requested symbol.
    NotWatched,
}

impl WatchError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::UnknownHandle => {
                "unknown or expired stream handle; call stream_watch to create a new one"
            }
            Self::NotWatched => "this handle does not cover that symbol",
        }
    }
}

/// Per-contract correlation state. Trades and quotes arrive as separate
/// messages in separate books, so the print-with-context view has to be
/// assembled across them, which means it cannot live on either book.
#[derive(Debug, Default)]
struct Correlation {
    last_quote: Option<Tick>,
    prints: VecDeque<Print>,
}

#[derive(Debug, Default)]
pub struct Inner {
    watches: HashMap<String, Watch>,
    books: HashMap<BookKey, Book>,
    correlation: HashMap<String, Correlation>,
    minted: u64,
}

/// The live-state store behind the stream tools.
#[derive(Debug)]
pub struct Registry {
    inner: Mutex<Inner>,
    ring_capacity: usize,
    ttl_ms: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(DEFAULT_RING, DEFAULT_TTL)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

impl Registry {
    pub fn new(ring_capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            ring_capacity: ring_capacity.max(1),
            ttl_ms: ttl.as_millis() as u64,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A panic while holding this lock would poison it for every later
        // call. Recovering the guard keeps one bad read from disabling every
        // stream tool for the life of the process.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a watch over `keys` and return its handle.
    ///
    /// Keys already held by another handle are shared, not re-subscribed. The
    /// returned list is the subset that is new to this process, which is
    /// exactly what the caller must subscribe on the feed.
    pub fn watch(
        &self,
        scope: &str,
        specs: Vec<(Spec, Kind)>,
        now: u64,
    ) -> (String, Vec<(Spec, Kind)>) {
        let keys: Vec<BookKey> = specs
            .iter()
            .map(|(spec, kind)| BookKey {
                symbol: spec.symbol(),
                kind: *kind,
            })
            .collect();
        let mut inner = self.lock();
        self.collect_expired(&mut inner, now);

        inner.minted += 1;
        let handle = format!("sw_{:012x}{:04x}", now, inner.minted & 0xffff);

        let mut fresh = Vec::new();
        for (idx, key) in keys.iter().enumerate() {
            let book = inner.books.entry(key.clone()).or_insert_with(Book::new);
            if book.refs == 0 {
                fresh.push(specs[idx].clone());
            }
            book.refs += 1;
        }

        inner.watches.insert(
            handle.clone(),
            Watch {
                keys,
                specs,
                scope: scope.to_owned(),
                created_ms: now,
                last_read_ms: now,
            },
        );
        (handle, fresh)
    }

    /// Drop a watch. Returns the keys whose last reference just went away —
    /// the ones the caller should unsubscribe on the feed.
    pub fn release(&self, handle: &str, now: u64) -> Result<Vec<(Spec, Kind)>, WatchError> {
        let mut inner = self.lock();
        let watch = inner
            .watches
            .remove(handle)
            .ok_or(WatchError::UnknownHandle)?;
        let freed = Self::deref_keys(&mut inner, &watch.keys);
        self.collect_expired(&mut inner, now);
        // Only the specs whose last reference just went away.
        Ok(watch
            .specs
            .into_iter()
            .filter(|(spec, kind)| {
                freed
                    .iter()
                    .any(|k| k.symbol == spec.symbol() && k.kind == *kind)
            })
            .collect())
    }

    fn deref_keys(inner: &mut Inner, keys: &[BookKey]) -> Vec<BookKey> {
        let mut freed = Vec::new();
        for key in keys {
            let Some(book) = inner.books.get_mut(key) else {
                continue;
            };
            book.refs = book.refs.saturating_sub(1);
            if book.refs == 0 {
                freed.push(key.clone());
            }
        }
        for key in &freed {
            inner.books.remove(key);
        }
        freed
    }

    /// Collect handles that have gone untouched for longer than the TTL.
    ///
    /// Called on every mutation rather than from a timer: a handle nobody
    /// reads costs nothing until someone else asks for one, and a sweep that
    /// only runs when the map changes cannot drift from the map.
    fn collect_expired(&self, inner: &mut Inner, now: u64) -> Vec<BookKey> {
        let stale: Vec<String> = inner
            .watches
            .iter()
            .filter(|(_, w)| now.saturating_sub(w.last_read_ms) > self.ttl_ms)
            .map(|(h, _)| h.clone())
            .collect();

        let mut freed = Vec::new();
        for handle in stale {
            if let Some(watch) = inner.watches.remove(&handle) {
                freed.extend(Self::deref_keys(inner, &watch.keys));
            }
        }
        freed
    }

    /// Record a tick against `symbol`. Unknown books are ignored rather than
    /// created: the feed can deliver a contract nobody is watching after an
    /// unsubscribe, and inventing a book for it would leak.
    pub fn ingest(&self, symbol: &str, tick: Tick, now: u64, bar_interval_ms: i64) {
        let key = BookKey {
            symbol: symbol.to_owned(),
            kind: tick.kind(),
        };
        let mut inner = self.lock();
        let capacity = self.ring_capacity;
        if !inner.books.contains_key(&key) {
            return;
        }
        {
            let book = inner.books.get_mut(&key).expect("checked above");
            book.received += 1;
            if BAR_KINDS.contains(&tick.kind()) {
                Self::fold_bar(book, &tick, bar_interval_ms);
            }
            if book.ring.len() == capacity {
                book.ring.pop_front();
                book.dropped += 1;
            }
            book.ring.push_back(Stamped {
                tick: tick.clone(),
                seen_ms: now,
            });
        }
        Self::correlate(&mut inner, symbol, &tick, capacity);
    }

    /// Fold a tick into the per-contract print view.
    ///
    /// A quote either completes the two-quote tail of the most recent print or
    /// becomes the standing pre-trade quote for the next one. A trade opens a
    /// new print carrying whatever quote stood before it.
    fn correlate(inner: &mut Inner, symbol: &str, tick: &Tick, capacity: usize) {
        let state = inner.correlation.entry(symbol.to_owned()).or_default();
        match tick {
            Tick::Quote { .. } => {
                if let Some(open) = state.prints.back_mut() {
                    if open.quotes_after.len() < 2 {
                        open.quotes_after.push(tick.clone());
                    }
                }
                state.last_quote = Some(tick.clone());
            }
            Tick::Trade { .. } => {
                state.prints.push_back(Print {
                    trade: tick.clone(),
                    quote_before: state.last_quote.clone(),
                    quotes_after: Vec::new(),
                });
                // Prints are the expensive view; keep far fewer than ticks.
                while state.prints.len() > capacity / 8 + 1 {
                    state.prints.pop_front();
                }
            }
            _ => {}
        }
    }

    /// Recent prints with the market around each one.
    pub fn prints(
        &self,
        handle: &str,
        symbol: Option<&str>,
        count: usize,
        now: u64,
    ) -> Result<Vec<(String, Vec<Print>)>, WatchError> {
        let mut inner = self.lock();
        let keys = Self::touch(&mut inner, handle, now)?;
        let mut symbols: Vec<String> = match symbol {
            Some(want) => {
                if !keys.iter().any(|k| k.symbol == want) {
                    return Err(WatchError::NotWatched);
                }
                vec![want.to_owned()]
            }
            None => {
                let mut all: Vec<String> = keys.iter().map(|k| k.symbol.clone()).collect();
                all.sort();
                all.dedup();
                all
            }
        };
        symbols.truncate(64);

        Ok(symbols
            .into_iter()
            .map(|sym| {
                let mut rows: Vec<Print> = inner
                    .correlation
                    .get(&sym)
                    .map(|c| c.prints.iter().cloned().collect())
                    .unwrap_or_default();
                if rows.len() > count {
                    rows.drain(..rows.len() - count);
                }
                (sym, rows)
            })
            .collect())
    }

    fn fold_bar(book: &mut Book, tick: &Tick, interval_ms: i64) {
        let Tick::Trade { price, size, .. } = *tick else {
            return;
        };
        let interval = interval_ms.max(1);
        let bucket = i64::from(tick.ms_of_day()) / interval * interval;
        let bar = book.bars.entry(bucket).or_insert(Bar {
            start_ms: bucket,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 0,
            trades: 0,
        });
        bar.high = bar.high.max(price);
        bar.low = bar.low.min(price);
        bar.close = price;
        bar.volume += i64::from(size);
        bar.trades += 1;

        // Bars are cheap next to the tick ring, but not free. Keep a bounded
        // history so a session left open overnight cannot grow without limit.
        while book.bars.len() > 1_440 {
            let Some(oldest) = book.bars.keys().next().copied() else {
                break;
            };
            book.bars.remove(&oldest);
        }
    }

    fn touch(inner: &mut Inner, handle: &str, now: u64) -> Result<Vec<BookKey>, WatchError> {
        let watch = inner
            .watches
            .get_mut(handle)
            .ok_or(WatchError::UnknownHandle)?;
        watch.last_read_ms = now;
        Ok(watch.keys.clone())
    }

    fn resolve<'a>(keys: &'a [BookKey], symbol: Option<&str>) -> Vec<&'a BookKey> {
        match symbol {
            Some(want) => keys.iter().filter(|k| k.symbol == want).collect(),
            None => keys.iter().collect(),
        }
    }

    /// Most recent tick per book, with how old it is.
    pub fn latest(
        &self,
        handle: &str,
        symbol: Option<&str>,
        now: u64,
    ) -> Result<Vec<(BookKey, Stamped, u64)>, WatchError> {
        let mut inner = self.lock();
        let keys = Self::touch(&mut inner, handle, now)?;
        let wanted = Self::resolve(&keys, symbol);
        if wanted.is_empty() {
            return Err(WatchError::NotWatched);
        }
        let mut out = Vec::new();
        for key in wanted {
            if let Some(last) = inner.books.get(key).and_then(|b| b.ring.back()) {
                out.push((key.clone(), last.clone(), now.saturating_sub(last.seen_ms)));
            }
        }
        Ok(out)
    }

    /// Ticks seen within the last `window_ms`, oldest first, newest-capped at
    /// `limit`.
    pub fn window(
        &self,
        handle: &str,
        symbol: Option<&str>,
        window_ms: u64,
        limit: usize,
        now: u64,
    ) -> Result<Vec<(BookKey, Vec<Stamped>)>, WatchError> {
        let mut inner = self.lock();
        let keys = Self::touch(&mut inner, handle, now)?;
        let wanted = Self::resolve(&keys, symbol);
        if wanted.is_empty() {
            return Err(WatchError::NotWatched);
        }
        let floor = now.saturating_sub(window_ms);
        let mut out = Vec::new();
        for key in wanted {
            let Some(book) = inner.books.get(key) else {
                continue;
            };
            let mut rows: Vec<Stamped> = book
                .ring
                .iter()
                .filter(|s| s.seen_ms >= floor)
                .cloned()
                .collect();
            if rows.len() > limit {
                rows.drain(..rows.len() - limit);
            }
            out.push((key.clone(), rows));
        }
        Ok(out)
    }

    /// The most recent `count` bars per book, oldest first.
    pub fn bars(
        &self,
        handle: &str,
        symbol: Option<&str>,
        count: usize,
        now: u64,
    ) -> Result<Vec<(BookKey, Vec<Bar>)>, WatchError> {
        let mut inner = self.lock();
        let keys = Self::touch(&mut inner, handle, now)?;
        let wanted = Self::resolve(&keys, symbol);
        if wanted.is_empty() {
            return Err(WatchError::NotWatched);
        }
        let mut out = Vec::new();
        for key in wanted {
            let Some(book) = inner.books.get(key) else {
                continue;
            };
            let mut rows: Vec<Bar> = book.bars.values().copied().collect();
            if rows.len() > count {
                rows.drain(..rows.len() - count);
            }
            out.push((key.clone(), rows));
        }
        Ok(out)
    }

    /// What the handle covers and how healthy each book is.
    pub fn status(&self, handle: &str, now: u64) -> Result<WatchStatus, WatchError> {
        let mut inner = self.lock();
        let keys = Self::touch(&mut inner, handle, now)?;
        let watch = inner.watches.get(handle).ok_or(WatchError::UnknownHandle)?;
        let scope = watch.scope.clone();
        let created_ms = watch.created_ms;

        let books = keys
            .iter()
            .map(|key| {
                let (received, dropped, held, age) = inner
                    .books
                    .get(key)
                    .map(|b| {
                        (
                            b.received,
                            b.dropped,
                            b.ring.len(),
                            b.ring.back().map(|s| now.saturating_sub(s.seen_ms)),
                        )
                    })
                    .unwrap_or((0, 0, 0, None));
                BookStatus {
                    key: key.clone(),
                    received,
                    dropped,
                    held,
                    age_ms: age,
                }
            })
            .collect();

        Ok(WatchStatus {
            scope,
            created_ms,
            expires_in_ms: self.ttl_ms,
            books,
        })
    }
}

#[derive(Debug)]
pub struct BookStatus {
    pub key: BookKey,
    pub received: u64,
    pub dropped: u64,
    pub held: usize,
    pub age_ms: Option<u64>,
}

#[derive(Debug)]
pub struct WatchStatus {
    pub scope: String,
    pub created_ms: u64,
    pub expires_in_ms: u64,
    pub books: Vec<BookStatus>,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool surface
// ═══════════════════════════════════════════════════════════════════════════

use std::sync::OnceLock;

use sonic_rs::{json, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, OptionLeg, StreamData, StreamEvent, Subscription, SubscriptionKind,
};
use thetadatadx::Client;

use crate::ToolError;

/// One registry per process. The handles it mints are only meaningful against
/// the subscriptions this process holds, so there is nothing to share wider.
static REGISTRY: OnceLock<Registry> = OnceLock::new();
/// Set once the event handler is installed, so a second watch does not install
/// a second one.
static HANDLER: OnceLock<()> = OnceLock::new();

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

pub const TOOL_NAMES: [&str; 7] = [
    "stream_watch",
    "stream_latest",
    "stream_window",
    "stream_bars",
    "stream_prints",
    "stream_status",
    "stream_release",
];

fn handle_arg() -> Value {
    json!({
        "type": "string",
        "description": "Handle returned by stream_watch."
    })
}

fn symbol_arg() -> Value {
    json!({
        "type": "string",
        "description": "Restrict to one book. Omit to read every book the handle covers."
    })
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "stream_watch",
            "description": "Open a live subscription and return a handle to read it with. \
                A snapshot is a round trip and has already moved by the time you read it; \
                a handle is read locally and every read tells you how old the value is. \
                Scopes: 'contract' (one option), 'equity' (one stock), 'index' (one index). \
                Indices have no trade or quote stream, only price and market value. \
                A handle is collected after 15 minutes without a read; call stream_release when \
                finished rather than waiting for that.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["contract", "equity", "index"]},
                    "kind": {
                        "type": "string",
                        "enum": ["trade", "quote", "market_value", "price"],
                        "description": "trade and quote are per-contract; market_value is a derived midpoint, not a quote; price is indices only."
                    },
                    "root": {"type": "string", "description": "Ticker or option root, e.g. AAPL or QQQ."},
                    "expiration": {"type": "integer", "description": "YYYYMMDD. Option contract and chain scopes."},
                    "strike": {"type": "number", "description": "Strike in dollars. Contract scope only."},
                    "right": {"type": "string", "enum": ["C", "P"], "description": "Contract scope only."},
                },
                "required": ["scope", "kind"]
            }
        }),
        json!({
            "name": "stream_latest",
            "description": "The most recent value on each book, with age_ms: how long ago the \
                server saw it. An index reports about once a second, so seconds of age are \
                normal there and stale on an option quote.",
            "inputSchema": {
                "type": "object",
                "properties": {"handle": handle_arg(), "symbol": symbol_arg()},
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_window",
            "description": "Ticks seen in the last N seconds, oldest first. Use stream_bars \
                instead when you want shape rather than every print.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "handle": handle_arg(),
                    "symbol": symbol_arg(),
                    "seconds": {"type": "integer", "description": "Window length. Default 60."},
                    "limit": {"type": "integer", "description": "Newest N rows. Default 200."}
                },
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_bars",
            "description": "OHLCV bars built from the trades seen since the watch opened. \
                Quotes produce no bars: a quote has no traded size, so a bar from one would \
                report volume that never happened.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "handle": handle_arg(),
                    "symbol": symbol_arg(),
                    "count": {"type": "integer", "description": "Most recent N bars. Default 30."}
                },
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_prints",
            "description": "Recent trades, each with the quote that stood immediately before it \
                and the next two quote updates after it. This is the market around a print \
                rather than the print alone. Richest on a feed watch, which is where the \
                vendor sends that context.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "handle": handle_arg(),
                    "symbol": symbol_arg(),
                    "count": {"type": "integer", "description": "Most recent N prints. Default 20."}
                },
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_status",
            "description": "What the handle covers and how healthy it is: messages received, \
                messages dropped because the buffer was full, and how old each book's newest \
                value is. A non-zero drop count means a window is clipped, not complete.",
            "inputSchema": {
                "type": "object",
                "properties": {"handle": handle_arg()},
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_release",
            "description": "Close a watch. Subscriptions shared with another handle stay open; \
                only the last holder unsubscribes.",
            "inputSchema": {
                "type": "object",
                "properties": {"handle": handle_arg()},
                "required": ["handle"]
            }
        }),
    ]
}

fn str_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(|v: &Value| v.as_str())
}

fn usize_arg(args: &Value, name: &str, default: usize) -> usize {
    args.get(name)
        .and_then(|v: &Value| v.as_u64())
        .map_or(default, |v| v as usize)
}

/// Book name for a contract off the wire. Must agree with [`Spec::symbol`] —
/// the two are the only thing joining a subscription to the ticks it produces.
fn contract_symbol(contract: &Contract) -> String {
    match (
        contract.expiration,
        contract.is_call,
        contract.strike_thousandths,
    ) {
        (Some(exp), Some(is_call), Some(strike)) => {
            let right = if is_call { 'C' } else { 'P' };
            format!("{} {exp} {right} {strike}", contract.symbol)
        }
        _ => contract.symbol.to_string(),
    }
}

fn tick_from(data: &StreamData) -> Option<(String, Tick)> {
    match data {
        StreamData::Quote {
            contract,
            ms_of_day,
            bid,
            bid_size,
            ask,
            ask_size,
            date,
            ..
        } => Some((
            contract_symbol(contract),
            Tick::Quote {
                ms_of_day: *ms_of_day,
                date: *date,
                bid: *bid,
                bid_size: *bid_size,
                ask: *ask,
                ask_size: *ask_size,
            },
        )),
        StreamData::Trade {
            contract,
            ms_of_day,
            price,
            size,
            condition,
            sequence,
            date,
            ..
        } => Some((
            contract_symbol(contract),
            Tick::Trade {
                ms_of_day: *ms_of_day,
                date: *date,
                price: *price,
                size: *size,
                condition: *condition,
                sequence: *sequence,
            },
        )),
        StreamData::MarketValue {
            contract,
            ms_of_day,
            market_bid,
            market_ask,
            market_price,
            date,
            ..
        } => Some((
            contract_symbol(contract),
            Tick::MarketValue {
                ms_of_day: *ms_of_day,
                date: *date,
                bid: Some(*market_bid),
                ask: Some(*market_ask),
                price: *market_price,
            },
        )),
        _ => None,
    }
}

fn install_handler(client: &Client) -> Result<(), ToolError> {
    if HANDLER.get().is_some() {
        return Ok(());
    }
    client
        .stream()
        .start_streaming(move |event: &StreamEvent| {
            if let StreamEvent::Data(data) = event {
                if let Some((symbol, tick)) = tick_from(data) {
                    registry().ingest(&symbol, tick, now_ms(), 60_000);
                }
            }
        })
        .map_err(|e| ToolError::ServerError(format!("could not start streaming: {e}")))?;
    let _ = HANDLER.set(());
    Ok(())
}

fn wire_kind(kind: Kind) -> SubscriptionKind {
    match kind {
        Kind::Quote => SubscriptionKind::Quote,
        Kind::MarketValue => SubscriptionKind::MarketValue,
        // An index price arrives on the trade subscription, in a trade-shaped
        // message. That is the vendor's shape, not a mapping we chose.
        Kind::Trade | Kind::Price => SubscriptionKind::Trade,
    }
}

fn build_contract(spec: &Spec) -> Result<Contract, ToolError> {
    match spec.sec_type {
        SecType::Stock => Ok(Contract::stock(&spec.root)),
        SecType::Index => Ok(Contract::index(&spec.root)),
        SecType::Option => {
            let (Some(exp), Some(strike), Some(right)) = (spec.expiration, spec.strike, spec.right)
            else {
                return Err(ToolError::InvalidParams(
                    "an option needs expiration, strike and right".into(),
                ));
            };
            // `OptionLeg` takes dollars; the spec holds thousandths of one.
            let dollars = format!("{:.3}", strike as f64 / 1000.0);
            Contract::option(
                &spec.root,
                OptionLeg {
                    expiration: &exp.to_string(),
                    strike: &dollars,
                    right: &right.to_string(),
                },
            )
            .map_err(|e| ToolError::InvalidParams(format!("contract: {e}")))
        }
    }
}

fn specs_for(args: &Value, scope: Scope, kind: Kind) -> Result<Vec<(Spec, Kind)>, ToolError> {
    let root = str_arg(args, "root");
    let need_root = |s: Option<&str>| {
        s.map(str::to_owned)
            .ok_or_else(|| ToolError::InvalidParams("root is required for this scope".into()))
    };

    match scope {
        Scope::Equity => Ok(vec![(Spec::equity(&need_root(root)?), kind)]),
        Scope::Index => Ok(vec![(Spec::index(&need_root(root)?), kind)]),
        Scope::Contract => {
            let root = need_root(root)?;
            let expiration = args
                .get("expiration")
                .and_then(|v: &Value| v.as_u64())
                .ok_or_else(|| ToolError::InvalidParams("expiration is required".into()))?;
            let strike = args
                .get("strike")
                .and_then(|v: &Value| v.as_f64())
                .ok_or_else(|| ToolError::InvalidParams("strike is required".into()))?;
            let right = str_arg(args, "right")
                .and_then(|r| r.chars().next())
                .ok_or_else(|| ToolError::InvalidParams("right must be C or P".into()))?;
            Ok(vec![(
                Spec::option(
                    &root,
                    expiration as u32,
                    (strike * 1000.0).round() as i64,
                    right.to_ascii_uppercase(),
                ),
                kind,
            )])
        }
    }
}

fn tick_json(tick: &Tick) -> Value {
    match tick {
        Tick::Quote {
            ms_of_day,
            date,
            bid,
            bid_size,
            ask,
            ask_size,
        } => json!({
            "type": "quote", "ms_of_day": ms_of_day, "date": date,
            "bid": bid, "bid_size": bid_size, "ask": ask, "ask_size": ask_size
        }),
        Tick::Trade {
            ms_of_day,
            date,
            price,
            size,
            condition,
            sequence,
        } => json!({
            "type": "trade", "ms_of_day": ms_of_day, "date": date,
            "price": price, "size": size, "condition": condition, "sequence": sequence
        }),
        Tick::MarketValue {
            ms_of_day,
            date,
            bid,
            ask,
            price,
        } => json!({
            "type": "market_value", "ms_of_day": ms_of_day, "date": date,
            "market_bid": bid, "market_ask": ask, "market_price": price,
            "note": "a derived midpoint, not a quote"
        }),
    }
}

fn err_json(e: &WatchError) -> ToolError {
    ToolError::InvalidParams(e.message().to_owned())
}

/// Dispatch for the stream tools. Returns `None` when `name` is not one of
/// them, so the caller can fall through to the endpoint registry.
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
            "not connected to ThetaData yet; retry in a moment".into(),
        )));
    };
    Some(execute(client, name, args).await)
}

async fn execute(client: &Client, name: &str, args: &Value) -> Result<Value, ToolError> {
    let reg = registry();
    let now = now_ms();

    if name == "stream_watch" {
        let scope = str_arg(args, "scope")
            .and_then(Scope::parse)
            .ok_or_else(|| ToolError::InvalidParams("unknown scope".into()))?;
        let kind = str_arg(args, "kind")
            .and_then(Kind::parse)
            .ok_or_else(|| ToolError::InvalidParams("unknown kind".into()))?;
        if let Some(why) = scope.reject(kind) {
            return Err(ToolError::InvalidParams(why));
        }

        let specs = specs_for(args, scope, kind)?;
        install_handler(client)?;
        let (handle, fresh) = reg.watch(scope.as_str(), specs, now);

        for (spec, kind) in &fresh {
            client
                .stream()
                .subscribe(Subscription::Contract {
                    contract: build_contract(spec)?,
                    kind: wire_kind(*kind),
                })
                .map_err(|e| ToolError::ServerError(format!("subscribe failed: {e}")))?;
        }

        return Ok(json!({
            "handle": handle,
            "scope": scope.as_str(),
            "kind": kind.as_str(),
            "subscriptions_opened": fresh.len(),
            "expires_after_seconds": DEFAULT_TTL.as_secs(),
            "note": "reads carry age_ms. Call stream_release when finished."
        }));
    }

    let handle = str_arg(args, "handle")
        .ok_or_else(|| ToolError::InvalidParams("handle is required".into()))?;
    let symbol = str_arg(args, "symbol");

    match name {
        "stream_latest" => {
            let rows = reg.latest(handle, symbol, now).map_err(|e| err_json(&e))?;
            Ok(json!({
                "books": rows.iter().map(|(key, stamped, age)| json!({
                    "symbol": key.symbol,
                    "kind": key.kind.as_str(),
                    "age_ms": age,
                    "tick": tick_json(&stamped.tick),
                })).collect::<Vec<_>>()
            }))
        }
        "stream_window" => {
            let seconds = usize_arg(args, "seconds", 60) as u64;
            let limit = usize_arg(args, "limit", 200);
            let rows = reg
                .window(handle, symbol, seconds * 1_000, limit, now)
                .map_err(|e| err_json(&e))?;
            Ok(json!({
                "window_seconds": seconds,
                "books": rows.iter().map(|(key, ticks)| json!({
                    "symbol": key.symbol,
                    "kind": key.kind.as_str(),
                    "count": ticks.len(),
                    "ticks": ticks.iter().map(|s| tick_json(&s.tick)).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()
            }))
        }
        "stream_bars" => {
            let count = usize_arg(args, "count", 30);
            let rows = reg
                .bars(handle, symbol, count, now)
                .map_err(|e| err_json(&e))?;
            Ok(json!({
                "interval_seconds": 60,
                "books": rows.iter().map(|(key, bars)| json!({
                    "symbol": key.symbol,
                    "kind": key.kind.as_str(),
                    "bars": bars.iter().map(|b| json!({
                        "start_ms_of_day": b.start_ms, "open": b.open, "high": b.high,
                        "low": b.low, "close": b.close, "volume": b.volume, "trades": b.trades
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()
            }))
        }
        "stream_prints" => {
            let count = usize_arg(args, "count", 20);
            let rows = reg
                .prints(handle, symbol, count, now)
                .map_err(|e| err_json(&e))?;
            Ok(json!({
                "books": rows.iter().map(|(sym, prints)| json!({
                    "symbol": sym,
                    "prints": prints.iter().map(|p| json!({
                        "trade": tick_json(&p.trade),
                        "quote_before": p.quote_before.as_ref().map(tick_json),
                        "quotes_after": p.quotes_after.iter().map(tick_json).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()
            }))
        }
        "stream_status" => {
            let st = reg.status(handle, now).map_err(|e| err_json(&e))?;
            Ok(json!({
                "scope": st.scope,
                "open_for_ms": now.saturating_sub(st.created_ms),
                "expires_after_ms_idle": st.expires_in_ms,
                "books": st.books.iter().map(|b| json!({
                    "symbol": b.key.symbol,
                    "kind": b.key.kind.as_str(),
                    "received": b.received,
                    "dropped": b.dropped,
                    "held": b.held,
                    "age_ms": b.age_ms,
                })).collect::<Vec<_>>()
            }))
        }
        "stream_release" => {
            let freed = reg.release(handle, now).map_err(|e| err_json(&e))?;
            for (spec, kind) in &freed {
                let _ = client.stream().unsubscribe(Subscription::Contract {
                    contract: build_contract(spec)?,
                    kind: wire_kind(*kind),
                });
            }
            Ok(json!({ "released": true, "subscriptions_closed": freed.len() }))
        }
        other => Err(ToolError::InvalidParams(format!("unknown tool: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(symbol: &str, kind: Kind) -> (Spec, Kind) {
        (Spec::equity(symbol), kind)
    }

    fn trade(ms: i32, price: f64, size: i32) -> Tick {
        Tick::Trade {
            ms_of_day: ms,
            date: 20260915,
            price,
            size,
            condition: 0,
            sequence: ms,
        }
    }

    fn quote(ms: i32, bid: f64, ask: f64) -> Tick {
        Tick::Quote {
            ms_of_day: ms,
            date: 20260915,
            bid,
            bid_size: 10,
            ask,
            ask_size: 10,
        }
    }

    #[test]
    fn indices_have_no_trade_or_quote_stream() {
        // The vendor publishes only a price stream and a market-value stream
        // for indices. Offering either of the others would invent a product.
        assert!(Scope::Index.reject(Kind::Trade).is_some());
        assert!(Scope::Index.reject(Kind::Quote).is_some());
        assert!(Scope::Index.reject(Kind::Price).is_none());
        assert!(Scope::Index.reject(Kind::MarketValue).is_none());

        let why = Scope::Index.reject(Kind::Quote).expect("a reason");
        assert!(
            why.contains("price"),
            "the refusal must name what is on offer: {why}"
        );
    }

    #[test]
    fn contract_scope_carries_the_per_contract_streams() {
        for kind in [Kind::Trade, Kind::Quote, Kind::MarketValue] {
            assert!(Scope::Contract.reject(kind).is_none(), "{kind:?}");
            assert!(Scope::Equity.reject(kind).is_none(), "{kind:?}");
        }
        assert!(Scope::Contract.reject(Kind::Price).is_some());
    }

    #[test]
    fn a_print_carries_the_quote_before_it_and_the_two_after() {
        let reg = Registry::default();
        let keys = vec![spec("QQQ", Kind::Trade), spec("QQQ", Kind::Quote)];
        let (h, _) = reg.watch("feed", keys, 0);

        // Exactly the order the bulk stream sends: the standing quote, the
        // print, then the next two quotes.
        reg.ingest("QQQ", quote(10, 1.00, 1.10), 1, 60_000);
        reg.ingest("QQQ", trade(11, 1.08, 3), 2, 60_000);
        reg.ingest("QQQ", quote(12, 1.02, 1.12), 3, 60_000);
        reg.ingest("QQQ", quote(13, 1.03, 1.13), 4, 60_000);
        // A third quote must not be swept into the same print.
        reg.ingest("QQQ", quote(14, 1.04, 1.14), 5, 60_000);

        let out = reg.prints(&h, Some("QQQ"), 10, 6).expect("prints");
        let prints = &out[0].1;
        assert_eq!(prints.len(), 1);
        let p = &prints[0];
        assert_eq!(p.trade, trade(11, 1.08, 3));
        assert_eq!(p.quote_before, Some(quote(10, 1.00, 1.10)));
        assert_eq!(
            p.quotes_after,
            vec![quote(12, 1.02, 1.12), quote(13, 1.03, 1.13)],
            "the stream sends two, and only two, belong to this print"
        );
    }

    #[test]
    fn a_print_with_no_prior_quote_says_so_rather_than_guessing() {
        let reg = Registry::default();
        let (h, _) = reg.watch("feed", vec![spec("QQQ", Kind::Trade)], 0);
        reg.ingest("QQQ", trade(11, 1.08, 3), 1, 60_000);

        let out = reg.prints(&h, Some("QQQ"), 10, 2).expect("prints");
        let p = &out[0].1[0];
        assert_eq!(p.quote_before, None);
        assert!(p.quotes_after.is_empty());
    }

    #[test]
    fn a_second_watch_on_the_same_key_shares_one_subscription() {
        let reg = Registry::default();
        let (_a, fresh_a) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 1_000);
        let (b, fresh_b) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 1_001);

        assert_eq!(fresh_a.len(), 1, "the first watch must subscribe");
        assert!(
            fresh_b.is_empty(),
            "the second must not: one feed subscription per key, however many handles name it"
        );

        // Releasing one of two holders must not unsubscribe the other's data.
        let freed = reg.release(&b, 1_002).expect("release");
        assert!(freed.is_empty(), "still referenced, so nothing is freed");
    }

    #[test]
    fn the_last_release_frees_the_subscription() {
        let reg = Registry::default();
        let (a, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 1_000);
        let (b, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 1_000);
        assert!(reg.release(&a, 1_001).expect("release a").is_empty());
        assert_eq!(
            reg.release(&b, 1_002).expect("release b"),
            vec![spec("AAPL", Kind::Trade)]
        );
        assert_eq!(reg.release(&b, 1_003), Err(WatchError::UnknownHandle));
    }

    #[test]
    fn the_ring_is_bounded_and_says_so() {
        let reg = Registry::new(3, DEFAULT_TTL);
        let (h, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);
        for i in 0..10 {
            reg.ingest(
                "AAPL",
                trade(i, 100.0 + f64::from(i), 1),
                u64::from(i as u32),
                60_000,
            );
        }
        let status = reg.status(&h, 100).expect("status");
        let book = &status.books[0];
        assert_eq!(book.received, 10);
        assert_eq!(book.held, 3, "ring holds its capacity, not the tape");
        assert_eq!(
            book.dropped, 7,
            "a reader must be able to tell a clipped window from a whole one"
        );
    }

    #[test]
    fn latest_reports_how_stale_it_is() {
        let reg = Registry::default();
        let (h, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);
        reg.ingest("AAPL", trade(1, 101.5, 4), 5_000, 60_000);

        let rows = reg.latest(&h, None, 7_500).expect("latest");
        assert_eq!(rows.len(), 1);
        let (_, stamped, age_ms) = &rows[0];
        assert_eq!(*age_ms, 2_500, "the age is the whole point over a snapshot");
        assert_eq!(stamped.tick, trade(1, 101.5, 4));
    }

    #[test]
    fn bars_aggregate_trades_and_only_trades() {
        let reg = Registry::default();
        let keys = vec![spec("AAPL", Kind::Trade), spec("AAPL", Kind::Quote)];
        let (h, _) = reg.watch("contract", keys, 0);

        reg.ingest("AAPL", trade(0, 100.0, 10), 1, 60_000);
        reg.ingest("AAPL", trade(30_000, 102.0, 5), 2, 60_000);
        reg.ingest("AAPL", trade(61_000, 99.0, 7), 3, 60_000);
        reg.ingest(
            "AAPL",
            Tick::Quote {
                ms_of_day: 10,
                date: 20260915,
                bid: 1.0,
                bid_size: 1,
                ask: 2.0,
                ask_size: 1,
            },
            4,
            60_000,
        );

        let out = reg.bars(&h, Some("AAPL"), 10, 5).expect("bars");
        let trade_bars = &out
            .iter()
            .find(|(k, _)| k.kind == Kind::Trade)
            .expect("trade book")
            .1;
        assert_eq!(trade_bars.len(), 2, "two one-minute buckets");
        assert_eq!(trade_bars[0].open, 100.0);
        assert_eq!(trade_bars[0].high, 102.0);
        assert_eq!(trade_bars[0].low, 100.0);
        assert_eq!(trade_bars[0].close, 102.0);
        assert_eq!(trade_bars[0].volume, 15);
        assert_eq!(trade_bars[0].trades, 2);
        assert_eq!(trade_bars[1].open, 99.0);

        let quote_bars = &out
            .iter()
            .find(|(k, _)| k.kind == Kind::Quote)
            .expect("quote book")
            .1;
        assert!(
            quote_bars.is_empty(),
            "a quote has no traded size; aggregating one would invent volume"
        );
    }

    #[test]
    fn a_window_excludes_what_fell_outside_it() {
        let reg = Registry::default();
        let (h, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);
        reg.ingest("AAPL", trade(1, 10.0, 1), 1_000, 60_000);
        reg.ingest("AAPL", trade(2, 11.0, 1), 9_000, 60_000);

        let out = reg.window(&h, None, 5_000, 100, 10_000).expect("window");
        assert_eq!(out[0].1.len(), 1, "only the tick inside the window");
        assert_eq!(out[0].1[0].tick, trade(2, 11.0, 1));
    }

    #[test]
    fn an_idle_handle_expires_and_frees_its_subscription() {
        let reg = Registry::new(DEFAULT_RING, Duration::from_millis(100));
        let (stale, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);

        // A later watch is what triggers the sweep; nothing runs on a timer.
        let (_fresh, fresh_keys) = reg.watch("contract", vec![spec("MSFT", Kind::Trade)], 10_000);
        assert_eq!(fresh_keys.len(), 1);

        assert_eq!(
            reg.latest(&stale, None, 10_001),
            Err(WatchError::UnknownHandle),
            "an expired handle must say so, so the model creates a new one"
        );
    }

    #[test]
    fn a_symbol_the_handle_does_not_cover_is_refused() {
        let reg = Registry::default();
        let (h, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);
        assert_eq!(reg.latest(&h, Some("MSFT"), 1), Err(WatchError::NotWatched));
    }

    #[test]
    fn a_tick_for_an_unwatched_book_is_dropped_rather_than_creating_one() {
        let reg = Registry::default();
        let (h, _) = reg.watch("contract", vec![spec("AAPL", Kind::Trade)], 0);
        reg.ingest("MSFT", trade(1, 5.0, 1), 1, 60_000);
        let status = reg.status(&h, 2).expect("status");
        assert_eq!(status.books.len(), 1, "no book invented for MSFT");
    }
}
