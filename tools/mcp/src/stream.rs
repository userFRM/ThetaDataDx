//! Live-state views over the streaming feed.
//!
//! A model cannot read a feed: ticks arrive orders of magnitude faster than
//! tokens. What works is holding the subscription and answering questions
//! about it — what is the value now, what did the last minute look like, what
//! printed and where the market was around it.
//!
//! The protocol has no session, so a subscription is named by a handle minted
//! here and passed back as an ordinary tool argument.
//!
//! This layer stores what the feed sends and serves it back. It does not
//! aggregate: bar construction has condition, cancel and size rules that are
//! the caller's to choose, and a bar built on assumptions here would not
//! reconcile with one built anywhere else.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonic_rs::{json, JsonValueMutTrait, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, OptionLeg, StreamControl, StreamData, StreamEvent, Subscription, SubscriptionKind,
};
use thetadatadx::{Client, ConnectionStatus, SecType, StreamMsgType};

use crate::{sanitize_error, ToolError};

/// Ticks retained per book. A busy contract prints far more than this in a
/// session; the tail is what a model can act on and the rest is weight.
const RING: usize = 4_096;
/// Prints retained per contract. Fewer than ticks: each one holds three
/// messages.
const PRINTS: usize = 256;
/// How long a handle survives without a read. Stated in the creating tool's
/// description, since that is where a model reads it.
const TTL: Duration = Duration::from_secs(900);

/// Subscriptions to open or close on the feed, in the SDK's own tuple order.
/// A handle owns exactly one; a set appears when the idle sweep frees several
/// at once, or when a fresh session must reopen everything still held.
type Subs = Vec<(SubscriptionKind, Contract)>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// When the decoder saw this tick, in milliseconds since the epoch.
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

/// The subscription code a stored message answers to. `StreamMsgType` is the
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

#[derive(Debug, Default)]
struct Book {
    ring: VecDeque<StreamData>,
    received: u64,
    dropped: u64,
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
}

impl ContractState {
    fn book(&self, kind: SubscriptionKind) -> Option<&Book> {
        self.books.iter().find(|(k, _)| *k == kind).map(|(_, b)| b)
    }
}

#[derive(Debug, Clone)]
struct Watch {
    contract: Contract,
    kind: SubscriptionKind,
    opened_ms: u64,
    read_ms: u64,
}

#[derive(Debug, Default)]
struct Inner {
    watches: HashMap<String, Watch>,
    contracts: HashMap<Contract, ContractState>,
    minted: u64,
}

/// Unknown, or expired and collected. The caller's recovery is the same
/// either way: open a new one.
#[derive(Debug, PartialEq)]
pub struct UnknownHandle;

#[derive(Debug, Default)]
pub struct Registry {
    inner: Mutex<Inner>,
    /// Set when the feed reports its reconnect budget spent. The SDK then
    /// reads as `Reconnecting` for good, so this is the only signal that the
    /// next watch must restart the session rather than wait for it.
    reconnects_exhausted: AtomicBool,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Recover rather than propagate: one panicking read must not disable
        // every stream tool for the life of the process.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a watch. Returns its handle, whether this process must open the
    /// subscription on the feed, and what the idle sweep freed.
    fn watch(&self, contract: Contract, kind: SubscriptionKind, now: u64) -> (String, bool, Subs) {
        let mut inner = self.lock();
        // Two lists, because they need opposite actions: what the idle sweep
        // freed must be closed on the feed, what this watch is first to want
        // must be opened.
        let expired = Self::collect_expired(&mut inner, now);
        inner.minted += 1;
        let handle = format!("sw_{now:012x}{:04x}", inner.minted & 0xffff);

        // A book exists exactly while some watch wants it, so its absence is
        // the whole reference count.
        let books = &mut inner.contracts.entry(contract.clone()).or_default().books;
        let first = !books.iter().any(|(k, _)| *k == kind);
        if first {
            books.push((kind, Book::default()));
        }

        inner.watches.insert(
            handle.clone(),
            Watch {
                contract,
                kind,
                opened_ms: now,
                read_ms: now,
            },
        );
        (handle, first, expired)
    }

    /// Close a watch. Returns the subscriptions whose last holder just left.
    fn release(&self, handle: &str, now: u64) -> Result<Subs, UnknownHandle> {
        let mut inner = self.lock();
        let watch = inner.watches.remove(handle).ok_or(UnknownHandle)?;
        let mut freed: Subs = Self::close(&mut inner, watch).into_iter().collect();
        // A handle that timed out while this one was open still holds a live
        // subscription until someone closes it.
        freed.extend(Self::collect_expired(&mut inner, now));
        Ok(freed)
    }

    /// Drop the book behind a removed watch once no other watch wants it.
    /// Returns the subscription to close on the feed when that was the last.
    fn close(inner: &mut Inner, watch: Watch) -> Option<(SubscriptionKind, Contract)> {
        let Watch { contract, kind, .. } = watch;
        if inner
            .watches
            .values()
            .any(|w| w.kind == kind && w.contract == contract)
        {
            return None;
        }
        if let Some(state) = inner.contracts.get_mut(&contract) {
            state.books.retain(|(k, _)| *k != kind);
            if state.books.is_empty() {
                inner.contracts.remove(&contract);
            }
        }
        Some((kind, contract))
    }

    /// Drop handles nobody has read inside the TTL.
    ///
    /// Swept on mutation rather than from a timer: an untouched handle costs
    /// nothing until someone else opens one, and a sweep tied to the map
    /// cannot drift from it.
    fn collect_expired(inner: &mut Inner, now: u64) -> Subs {
        let ttl = TTL.as_millis() as u64;
        let stale: Vec<Watch> = inner
            .watches
            .extract_if(|_, w| now.saturating_sub(w.read_ms) > ttl)
            .map(|(_, w)| w)
            .collect();
        stale
            .into_iter()
            .filter_map(|w| Self::close(inner, w))
            .collect()
    }

    /// Every subscription the books hold, once each: what a fresh session
    /// must reopen.
    fn subscriptions(&self) -> Subs {
        self.lock()
            .contracts
            .iter()
            .flat_map(|(c, s)| s.books.iter().map(move |(k, _)| (*k, c.clone())))
            .collect()
    }

    /// Store a tick. Unknown books are ignored rather than created: the feed
    /// can deliver a contract after its unsubscribe, and inventing a book for
    /// one would leak.
    pub fn ingest(&self, data: StreamData) {
        let (Some(contract), Some(msg)) = (contract_of(&data), msg_type_of(&data)) else {
            return;
        };
        let mut inner = self.lock();
        let Some(state) = inner.contracts.get_mut(contract) else {
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
                if let Some(open) = state.prints.back_mut() {
                    if open.quotes_after.len() < 2 {
                        open.quotes_after.push(data.clone());
                    }
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
                }
            }
            _ => {}
        }
    }

    /// Mark a handle read and return what it covers.
    ///
    /// Every reader below copies what it needs under the lock and releases it
    /// before serialising. The streaming dispatcher calls [`Self::ingest`] on
    /// the same lock, so a critical section held across JSON construction
    /// would stall the feed — the one thing this layer must never do.
    fn touch(inner: &mut Inner, handle: &str, now: u64) -> Result<Watch, UnknownHandle> {
        let watch = inner.watches.get_mut(handle).ok_or(UnknownHandle)?;
        watch.read_ms = now;
        Ok(watch.clone())
    }

    fn book<'a>(inner: &'a Inner, watch: &Watch) -> Option<&'a Book> {
        inner.contracts.get(&watch.contract)?.book(watch.kind)
    }

    fn latest(
        &self,
        handle: &str,
        now: u64,
    ) -> Result<(Contract, Option<StreamData>), UnknownHandle> {
        let mut inner = self.lock();
        let watch = Self::touch(&mut inner, handle, now)?;
        let last = Self::book(&inner, &watch).and_then(|b| b.ring.back().cloned());
        Ok((watch.contract, last))
    }

    /// Messages at or after `floor`, newest `limit` of them, oldest first.
    fn window(
        &self,
        handle: &str,
        floor: u64,
        limit: usize,
        now: u64,
    ) -> Result<(Contract, Vec<StreamData>), UnknownHandle> {
        let mut inner = self.lock();
        let watch = Self::touch(&mut inner, handle, now)?;
        // Walk back from the newest and stop at the floor or the limit, so the
        // critical section copies what will be served and not the whole ring.
        let mut rows: Vec<StreamData> = Self::book(&inner, &watch)
            .map(|b| {
                b.ring
                    .iter()
                    .rev()
                    .take_while(|d| seen_ms(d).is_none_or(|s| s >= floor))
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        rows.reverse();
        Ok((watch.contract, rows))
    }

    fn prints(
        &self,
        handle: &str,
        count: usize,
        now: u64,
    ) -> Result<(Contract, Vec<Print>), UnknownHandle> {
        let mut inner = self.lock();
        let watch = Self::touch(&mut inner, handle, now)?;
        let mut rows: Vec<Print> = inner
            .contracts
            .get(&watch.contract)
            .map(|c| c.prints.iter().rev().take(count).cloned().collect())
            .unwrap_or_default();
        rows.reverse();
        Ok((watch.contract, rows))
    }

    fn status(&self, handle: &str, now: u64) -> Result<Health, UnknownHandle> {
        let mut inner = self.lock();
        let watch = Self::touch(&mut inner, handle, now)?;
        let book = Self::book(&inner, &watch);
        Ok(Health {
            received: book.map_or(0, |b| b.received),
            dropped: book.map_or(0, |b| b.dropped),
            held: book.map_or(0, |b| b.ring.len()),
            age_ms: book
                .and_then(|b| b.ring.back())
                .and_then(seen_ms)
                .map(|s| now.saturating_sub(s)),
            opened_ms: watch.opened_ms,
            contract: watch.contract,
        })
    }
}

/// Counters copied out from under the lock, ready to serialise.
struct Health {
    contract: Contract,
    opened_ms: u64,
    received: u64,
    dropped: u64,
    held: usize,
    age_ms: Option<u64>,
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

pub const TOOL_NAMES: [&str; 6] = [
    "stream_watch",
    "stream_latest",
    "stream_window",
    "stream_prints",
    "stream_status",
    "stream_release",
];

/// Kinds the vendor offers for a security type.
///
/// Indices have no trade or quote stream — only price and market value — and
/// an index price arrives on the trade subscription in a trade-shaped
/// message, which is why `trade` is how it is asked for.
fn kinds_for(sec: SecType) -> &'static [SubscriptionKind] {
    match sec {
        SecType::Option | SecType::Stock => &[
            SubscriptionKind::Trade,
            SubscriptionKind::Quote,
            SubscriptionKind::MarketValue,
            SubscriptionKind::OpenInterest,
        ],
        SecType::Index => &[SubscriptionKind::Trade, SubscriptionKind::MarketValue],
        _ => &[],
    }
}

fn resolve_kind(sec: SecType, raw: &str) -> Result<SubscriptionKind, ToolError> {
    let offered = kinds_for(sec);
    if let Some(kind) = offered.iter().find(|k| k.kind_str() == raw) {
        return Ok(*kind);
    }
    let names: Vec<&str> = offered.iter().map(|k| k.kind_str()).collect();
    Err(ToolError::InvalidParams(format!(
        "{raw} is not available for this security type; it offers {}",
        names.join(", ")
    )))
}

fn handle_arg() -> Value {
    json!({"type": "string", "description": "Handle from stream_watch."})
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "stream_watch",
            "description": "Subscribe to live data and return a handle to read it with. A \
                snapshot is a round trip and has already moved by the time you read it; every \
                read here reports how old its value is. Indices have no quote stream: they \
                offer trade (which carries the index price) and market_value. Market value is \
                a derived midpoint, not a quote. The handle is collected after 15 minutes \
                without a read, so call stream_release when you are finished. If the feed has \
                dropped and given up reconnecting, this call restarts it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sec_type": {"type": "string", "enum": ["option", "stock", "index"]},
                    "kind": {"type": "string", "enum": ["trade", "quote", "market_value", "open_interest"]},
                    "root": {"type": "string", "description": "Ticker or option root, e.g. AAPL."},
                    "expiration": {"type": "integer", "description": "YYYYMMDD. Options only."},
                    "strike": {"type": "number", "description": "Strike in dollars. Options only."},
                    "right": {"type": "string", "enum": ["C", "P"], "description": "Options only."}
                },
                "required": ["sec_type", "kind", "root"]
            }
        }),
        json!({
            "name": "stream_latest",
            "description": "The newest message on the handle, with age_ms. An index reports \
                about once a second, so seconds of age are normal there and stale on an \
                option quote.",
            "inputSchema": {"type": "object", "properties": {"handle": handle_arg()}, "required": ["handle"]}
        }),
        json!({
            "name": "stream_window",
            "description": "Messages from the last N seconds, oldest first, with their \
                condition and exchange codes. Aggregation rules are yours to choose, so this \
                serves what arrived rather than bars built on assumptions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "handle": handle_arg(),
                    "seconds": {"type": "integer", "description": "Default 60."},
                    "limit": {"type": "integer", "description": "Newest N. Default 200."}
                },
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_prints",
            "description": "Recent trades, each with the quote that stood before it and the \
                next two after it. Watch trade and quote on the same contract to populate it.",
            "inputSchema": {
                "type": "object",
                "properties": {"handle": handle_arg(),
                               "count": {"type": "integer", "description": "Default 20."}},
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_status",
            "description": "What the handle covers: messages received, messages dropped \
                because the buffer filled, the age of the newest message, and the feed's \
                connection state. A non-zero drop count means a window is clipped, not \
                complete. A feed that is not Connected delivers nothing until stream_watch \
                restarts it.",
            "inputSchema": {"type": "object", "properties": {"handle": handle_arg()}, "required": ["handle"]}
        }),
        json!({
            "name": "stream_release",
            "description": "Close a watch. A subscription shared with another handle stays \
                open; only the last holder closes it.",
            "inputSchema": {"type": "object", "properties": {"handle": handle_arg()}, "required": ["handle"]}
        }),
    ]
}

fn tick_json(data: &StreamData, now: u64) -> Value {
    let age = seen_ms(data).map(|s| now.saturating_sub(s));
    let mut body = match data {
        StreamData::Quote {
            ms_of_day,
            date,
            bid,
            bid_size,
            bid_exchange,
            bid_condition,
            ask,
            ask_size,
            ask_exchange,
            ask_condition,
            ..
        } => json!({
            "type": "quote", "ms_of_day": ms_of_day, "date": date,
            "bid": bid, "bid_size": bid_size, "bid_exchange": bid_exchange, "bid_condition": bid_condition,
            "ask": ask, "ask_size": ask_size, "ask_exchange": ask_exchange, "ask_condition": ask_condition
        }),
        StreamData::Trade {
            ms_of_day,
            date,
            price,
            size,
            condition,
            exchange,
            sequence,
            ..
        } => json!({
            "type": "trade", "ms_of_day": ms_of_day, "date": date, "price": price,
            "size": size, "condition": condition, "exchange": exchange, "sequence": sequence
        }),
        StreamData::OpenInterest {
            ms_of_day,
            date,
            open_interest,
            ..
        } => json!({
            "type": "open_interest", "ms_of_day": ms_of_day, "date": date, "open_interest": open_interest
        }),
        StreamData::MarketValue {
            ms_of_day,
            date,
            market_bid,
            market_ask,
            market_price,
            ..
        } => json!({
            "type": "market_value", "ms_of_day": ms_of_day, "date": date,
            "market_bid": market_bid, "market_ask": market_ask, "market_price": market_price,
            "note": "derived midpoint, not a quote"
        }),
        other => json!({"type": "other", "debug": format!("{other:?}")}),
    };
    if let (Some(age), Some(obj)) = (age, body.as_object_mut()) {
        obj.insert(&"age_ms", Value::from(age));
    }
    body
}

fn stream_error(what: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::ServerError(format!("{what}: {}", sanitize_error(&e.to_string())))
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

fn build_contract(args: &Value, sec: SecType) -> Result<Contract, ToolError> {
    let root = args
        .get("root")
        .and_then(|v: &Value| v.as_str())
        .ok_or_else(|| ToolError::InvalidParams("root is required".into()))?;
    match sec {
        SecType::Stock => Ok(Contract::stock(root)),
        SecType::Index => Ok(Contract::index(root)),
        SecType::Option => {
            let exp = args.get("expiration").and_then(|v: &Value| v.as_u64());
            let strike = args.get("strike").and_then(|v: &Value| v.as_f64());
            let right = args.get("right").and_then(|v: &Value| v.as_str());
            let (Some(exp), Some(strike), Some(right)) = (exp, strike, right) else {
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
            .map_err(|e| ToolError::InvalidParams(format!("contract: {e}")))
        }
        _ => Err(ToolError::InvalidParams(
            "sec_type must be option, stock or index".into(),
        )),
    }
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
/// lets `watch` and `release` hand back subscriptions to open or close
/// outside the lock.
fn execute(client: &Client, name: &str, args: &Value) -> Result<Value, ToolError> {
    let reg = registry();
    let now = now_ms();
    let str_of = |k: &str| args.get(k).and_then(|v: &Value| v.as_str());
    let num_of = |k: &str, d: usize| {
        args.get(k)
            .and_then(|v: &Value| v.as_u64())
            .map_or(d, |v| v as usize)
    };

    if name == "stream_watch" {
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
        let kind = resolve_kind(
            sec,
            str_of("kind").ok_or_else(|| ToolError::InvalidParams("kind is required".into()))?,
        )?;
        let contract = build_contract(args, sec)?;

        ensure_streaming(client, reg)?;
        let (handle, first, expired) = reg.watch(contract.clone(), kind, now);

        // Close what the idle sweep released before opening anything new, so a
        // forgotten handle cannot hold an allowance the next caller needs.
        for (kind, contract) in expired {
            if let Err(e) = client.stream().unsubscribe(Subscription::Contract {
                contract: contract.clone(),
                kind,
            }) {
                tracing::warn!(%contract, ?kind, error = %e, "expired watch left its subscription open");
            }
        }

        if first {
            if let Err(e) = client.stream().subscribe(Subscription::Contract {
                contract: contract.clone(),
                kind,
            }) {
                // Roll the handle back. Leaving it would hand back a handle
                // that receives nothing, and the book it holds would make the
                // next watch on this contract skip subscribing too.
                let _ = reg.release(&handle, now);
                return Err(stream_error("subscribe failed", e));
            }
        }
        return Ok(json!({
            "handle": handle,
            "contract": contract.to_string(),
            "subscriptions_opened": usize::from(first),
            "expires_after_seconds": TTL.as_secs()
        }));
    }

    let handle =
        str_of("handle").ok_or_else(|| ToolError::InvalidParams("handle is required".into()))?;
    let bad = |_: UnknownHandle| {
        ToolError::InvalidParams(
            "unknown or expired stream handle; call stream_watch for a new one".into(),
        )
    };

    match name {
        "stream_latest" => {
            let (contract, last) = reg.latest(handle, now).map_err(bad)?;
            Ok(json!({
                "contract": contract.to_string(),
                "message": last.map(|d| tick_json(&d, now))
            }))
        }
        "stream_window" => {
            let seconds = num_of("seconds", 60) as u64;
            let (contract, rows) = reg
                .window(
                    handle,
                    now.saturating_sub(seconds * 1_000),
                    num_of("limit", 200),
                    now,
                )
                .map_err(bad)?;
            Ok(json!({
                "contract": contract.to_string(),
                "window_seconds": seconds,
                "count": rows.len(),
                "messages": rows.iter().map(|d| tick_json(d, now)).collect::<Vec<_>>()
            }))
        }
        "stream_prints" => {
            let (contract, rows) = reg.prints(handle, num_of("count", 20), now).map_err(bad)?;
            Ok(json!({
                "contract": contract.to_string(),
                "prints": rows.iter().map(|p| json!({
                    "trade": tick_json(&p.trade, now),
                    "quote_before": p.quote_before.as_ref().map(|q| tick_json(q, now)),
                    "quotes_after": p.quotes_after.iter().map(|q| tick_json(q, now)).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            }))
        }
        "stream_status" => {
            let h = reg.status(handle, now).map_err(bad)?;
            let feed = if reg.reconnects_exhausted.load(Ordering::Relaxed) {
                "ReconnectsExhausted".to_string()
            } else {
                format!("{:?}", client.stream().connection_status())
            };
            Ok(json!({
                "contract": h.contract.to_string(),
                "feed": feed,
                "open_for_ms": now.saturating_sub(h.opened_ms),
                "expires_after_ms_idle": TTL.as_millis() as u64,
                "received": h.received,
                "dropped": h.dropped,
                "held": h.held,
                "age_ms": h.age_ms
            }))
        }
        "stream_release" => {
            let freed = reg.release(handle, now).map_err(bad)?;
            let mut closed = 0usize;
            let mut failures = Vec::new();
            for (kind, contract) in freed {
                match client.stream().unsubscribe(Subscription::Contract {
                    contract: contract.clone(),
                    kind,
                }) {
                    Ok(()) => closed += 1,
                    // Report it. Claiming a close that did not happen tells the
                    // caller an allowance came back while the feed still holds it.
                    Err(e) => {
                        failures.push(format!("{contract}: {}", sanitize_error(&e.to_string())))
                    }
                }
            }
            Ok(json!({
                "released": true,
                "subscriptions_closed": closed,
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

    fn trade(c: &Contract, price: f64, received_at_ns: u64) -> StreamData {
        StreamData::Trade {
            contract: Arc::new(c.clone()),
            ms_of_day: 1,
            sequence: 1,
            condition: 0,
            size: 1,
            exchange: 0,
            price,
            date: 20260915,
            received_at_ns,
        }
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

    #[test]
    fn one_subscription_serves_every_handle_and_the_last_release_closes_it() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (a, fresh_a, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 1_000);
        let (b, fresh_b, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 1_001);

        assert!(fresh_a, "the first handle subscribes");
        assert!(!fresh_b, "the second handle must not re-subscribe");
        assert!(
            reg.release(&a, 1_002).expect("release a").is_empty(),
            "still held, so nothing closes"
        );
        assert_eq!(reg.release(&b, 1_003).expect("release b").len(), 1);
        assert_eq!(reg.release(&b, 1_004), Err(UnknownHandle));
    }

    #[test]
    fn the_ring_is_bounded_and_counts_what_it_drops() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (h, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        for i in 0..(RING + 10) {
            reg.ingest(trade(&c, i as f64, 0));
        }
        let h = reg.status(&h, 1).expect("status");
        assert_eq!(h.received as usize, RING + 10);
        assert_eq!(h.held, RING, "holds its capacity, not the tape");
        assert_eq!(
            h.dropped, 10,
            "a reader must be able to tell a clipped window from a whole one"
        );
    }

    #[test]
    fn a_window_is_the_newest_rows_inside_the_floor_oldest_first() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (h, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        for ms in [100u64, 200, 300, 400] {
            reg.ingest(trade(&c, ms as f64, ms * 1_000_000));
        }

        let (_, rows) = reg.window(&h, 250, 10, 500).expect("window");
        assert_eq!(
            rows.iter().map(price).collect::<Vec<_>>(),
            vec![300.0, 400.0],
            "the floor cuts, and what remains is served oldest first"
        );
        let (_, rows) = reg.window(&h, 0, 3, 500).expect("window");
        assert_eq!(
            rows.iter().map(price).collect::<Vec<_>>(),
            vec![200.0, 300.0, 400.0],
            "the limit keeps the newest, not the oldest"
        );
        let (_, last) = reg.latest(&h, 500).expect("latest");
        assert_eq!(last.as_ref().map(price), Some(400.0));
    }

    #[test]
    fn a_print_takes_the_quote_before_it_and_exactly_the_two_after() {
        let reg = Registry::default();
        let c = stock("AAPL");
        // Trade and quote are separate subscriptions, so a print needs both
        // handles open. The correlation is per contract, not per handle.
        let (h, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let (_q, _, _) = reg.watch(c.clone(), SubscriptionKind::Quote, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 0));
        reg.ingest(quote(&c, 1.02, 1.12));
        reg.ingest(quote(&c, 1.03, 1.13));
        reg.ingest(quote(&c, 1.04, 1.14));

        let (_, prints) = reg.prints(&h, 10, 1).expect("prints");
        assert_eq!(prints.len(), 1);
        assert!(prints[0].quote_before.is_some());
        assert_eq!(
            prints[0].quotes_after.len(),
            2,
            "the feed sends two after a print; a third belongs to the next one"
        );
    }

    #[test]
    fn releasing_one_kind_keeps_the_prints_the_other_still_serves() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (t, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let (q, _, _) = reg.watch(c.clone(), SubscriptionKind::Quote, 0);
        reg.ingest(quote(&c, 1.00, 1.10));
        reg.ingest(trade(&c, 1.08, 0));

        assert_eq!(reg.release(&q, 1).expect("release quote").len(), 1);
        let (_, prints) = reg.prints(&t, 10, 2).expect("prints");
        assert_eq!(
            prints.len(),
            1,
            "the trade handle is open; closing the quote one must not erase its history"
        );
        assert_eq!(reg.release(&t, 3).expect("release trade").len(), 1);
        assert!(
            reg.lock().contracts.is_empty(),
            "nothing held once the last book is gone"
        );
    }

    #[test]
    fn a_fresh_session_reopens_each_held_subscription_once() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (_a, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let (_b, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let (_q, _, _) = reg.watch(c.clone(), SubscriptionKind::Quote, 0);

        let mut subs = reg.subscriptions();
        subs.sort_by_key(|(k, _)| k.kind_str());
        assert_eq!(
            subs,
            vec![
                (SubscriptionKind::Quote, c.clone()),
                (SubscriptionKind::Trade, c)
            ],
            "two handles share one subscription; replaying it twice would double-subscribe"
        );
    }

    #[test]
    fn an_index_has_no_quote_stream() {
        // The vendor offers indices price (on the trade subscription) and
        // market value. Offering a quote would invent a product.
        assert!(resolve_kind(SecType::Index, "quote").is_err());
        assert!(resolve_kind(SecType::Index, "trade").is_ok());
        assert!(resolve_kind(SecType::Index, "market_value").is_ok());
        assert!(resolve_kind(SecType::Stock, "quote").is_ok());

        let why = format!("{:?}", resolve_kind(SecType::Index, "quote").unwrap_err());
        assert!(
            why.contains("trade"),
            "the refusal names what is on offer: {why}"
        );
    }

    #[test]
    fn an_idle_handle_is_collected_and_says_so() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (stale, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let past_ttl = TTL.as_millis() as u64 + 1;

        // A later watch runs the sweep; nothing is on a timer.
        let (_fresh, fresh_keys, _) = reg.watch(stock("MSFT"), SubscriptionKind::Trade, past_ttl);
        assert!(fresh_keys);

        assert_eq!(reg.status(&stale, past_ttl).err(), Some(UnknownHandle));
    }

    #[test]
    fn the_idle_sweep_hands_back_what_must_be_unsubscribed() {
        // The whole point of the reference count is that a forgotten handle
        // does not hold an allowance forever. If the sweep frees a book but
        // nobody is told, the subscription leaks and the next watch on that
        // contract opens a second one.
        let reg = Registry::default();
        let c = stock("AAPL");
        let (_stale, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        let past_ttl = TTL.as_millis() as u64 + 1;

        let (_next, fresh, expired) = reg.watch(stock("MSFT"), SubscriptionKind::Trade, past_ttl);
        assert!(fresh, "MSFT is new");
        assert_eq!(
            expired,
            vec![(SubscriptionKind::Trade, c)],
            "the swept handle's subscription must come back to be closed"
        );
    }

    #[test]
    fn a_tick_for_an_unwatched_book_creates_nothing() {
        let reg = Registry::default();
        let (_h, _, _) = reg.watch(stock("AAPL"), SubscriptionKind::Trade, 0);
        reg.ingest(trade(&stock("MSFT"), 5.0, 0));
        assert_eq!(reg.lock().contracts.len(), 1, "no book invented for MSFT");
    }
}
