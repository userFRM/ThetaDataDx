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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Quote,
    Trade,
    OpenInterest,
    MarketValue,
}

impl Kind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "quote" => Some(Self::Quote),
            "trade" => Some(Self::Trade),
            "open_interest" => Some(Self::OpenInterest),
            "market_value" => Some(Self::MarketValue),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Trade => "trade",
            Self::OpenInterest => "open_interest",
            Self::MarketValue => "market_value",
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
    OpenInterest {
        ms_of_day: i32,
        date: i32,
        open_interest: i32,
    },
    MarketValue {
        ms_of_day: i32,
        date: i32,
        bid: f64,
        ask: f64,
        price: f64,
    },
}

impl Tick {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Quote { .. } => Kind::Quote,
            Self::Trade { .. } => Kind::Trade,
            Self::OpenInterest { .. } => Kind::OpenInterest,
            Self::MarketValue { .. } => Kind::MarketValue,
        }
    }

    pub fn ms_of_day(&self) -> i32 {
        match *self {
            Self::Quote { ms_of_day, .. }
            | Self::Trade { ms_of_day, .. }
            | Self::OpenInterest { ms_of_day, .. }
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

#[derive(Debug, Default)]
pub struct Inner {
    watches: HashMap<String, Watch>,
    books: HashMap<BookKey, Book>,
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
    pub fn watch(&self, scope: &str, keys: Vec<BookKey>, now: u64) -> (String, Vec<BookKey>) {
        let mut inner = self.lock();
        self.collect_expired(&mut inner, now);

        inner.minted += 1;
        let handle = format!("sw_{:012x}{:04x}", now, inner.minted & 0xffff);

        let mut fresh = Vec::new();
        for key in &keys {
            let book = inner.books.entry(key.clone()).or_insert_with(Book::new);
            if book.refs == 0 {
                fresh.push(key.clone());
            }
            book.refs += 1;
        }

        inner.watches.insert(
            handle.clone(),
            Watch {
                keys,
                scope: scope.to_owned(),
                created_ms: now,
                last_read_ms: now,
            },
        );
        (handle, fresh)
    }

    /// Drop a watch. Returns the keys whose last reference just went away —
    /// the ones the caller should unsubscribe on the feed.
    pub fn release(&self, handle: &str, now: u64) -> Result<Vec<BookKey>, WatchError> {
        let mut inner = self.lock();
        let watch = inner
            .watches
            .remove(handle)
            .ok_or(WatchError::UnknownHandle)?;
        let freed = Self::deref_keys(&mut inner, &watch.keys);
        self.collect_expired(&mut inner, now);
        Ok(freed)
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
        let Some(book) = inner.books.get_mut(&key) else {
            return;
        };

        book.received += 1;
        if BAR_KINDS.contains(&tick.kind()) {
            Self::fold_bar(book, &tick, bar_interval_ms);
        }
        if book.ring.len() == capacity {
            book.ring.pop_front();
            book.dropped += 1;
        }
        book.ring.push_back(Stamped {
            tick,
            seen_ms: now,
        });
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
        let watch = inner
            .watches
            .get(handle)
            .ok_or(WatchError::UnknownHandle)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(symbol: &str, kind: Kind) -> BookKey {
        BookKey {
            symbol: symbol.to_owned(),
            kind,
        }
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

    #[test]
    fn a_second_watch_on_the_same_key_shares_one_subscription() {
        let reg = Registry::default();
        let (_a, fresh_a) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 1_000);
        let (b, fresh_b) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 1_001);

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
        let (a, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 1_000);
        let (b, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 1_000);
        assert!(reg.release(&a, 1_001).expect("release a").is_empty());
        assert_eq!(
            reg.release(&b, 1_002).expect("release b"),
            vec![key("AAPL", Kind::Trade)]
        );
        assert_eq!(reg.release(&b, 1_003), Err(WatchError::UnknownHandle));
    }

    #[test]
    fn the_ring_is_bounded_and_says_so() {
        let reg = Registry::new(3, DEFAULT_TTL);
        let (h, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);
        for i in 0..10 {
            reg.ingest("AAPL", trade(i, 100.0 + f64::from(i), 1), u64::from(i as u32), 60_000);
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
        let (h, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);
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
        let keys = vec![key("AAPL", Kind::Trade), key("AAPL", Kind::Quote)];
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
        let (h, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);
        reg.ingest("AAPL", trade(1, 10.0, 1), 1_000, 60_000);
        reg.ingest("AAPL", trade(2, 11.0, 1), 9_000, 60_000);

        let out = reg.window(&h, None, 5_000, 100, 10_000).expect("window");
        assert_eq!(out[0].1.len(), 1, "only the tick inside the window");
        assert_eq!(out[0].1[0].tick, trade(2, 11.0, 1));
    }

    #[test]
    fn an_idle_handle_expires_and_frees_its_subscription() {
        let reg = Registry::new(DEFAULT_RING, Duration::from_millis(100));
        let (stale, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);

        // A later watch is what triggers the sweep; nothing runs on a timer.
        let (_fresh, fresh_keys) = reg.watch("contract", vec![key("MSFT", Kind::Trade)], 10_000);
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
        let (h, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);
        assert_eq!(reg.latest(&h, Some("MSFT"), 1), Err(WatchError::NotWatched));
    }

    #[test]
    fn a_tick_for_an_unwatched_book_is_dropped_rather_than_creating_one() {
        let reg = Registry::default();
        let (h, _) = reg.watch("contract", vec![key("AAPL", Kind::Trade)], 0);
        reg.ingest("MSFT", trade(1, 5.0, 1), 1, 60_000);
        let status = reg.status(&h, 2).expect("status");
        assert_eq!(status.books.len(), 1, "no book invented for MSFT");
    }
}
