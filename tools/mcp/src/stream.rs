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
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sonic_rs::{json, JsonValueMutTrait, JsonValueTrait, Value};
use thetadatadx::streaming::{
    Contract, OptionLeg, StreamData, StreamEvent, Subscription, SubscriptionKind,
};
use thetadatadx::{Client, SecType, StreamMsgType};

use crate::ToolError;

/// Ticks retained per book. A busy contract prints far more than this in a
/// session; the tail is what a model can act on and the rest is weight.
const RING: usize = 4_096;
/// Prints retained per contract. Fewer than ticks: each one holds three
/// messages.
const PRINTS: usize = 256;
/// How long a handle survives without a read. Stated in the creating tool's
/// description, since that is where a model reads it.
const TTL: Duration = Duration::from_secs(900);

/// A book is one contract and one message shape. `StreamMsgType` is the
/// SDK's own discriminant for that shape, so nothing here re-derives it.
type BookKey = (Contract, StreamMsgType);
/// Subscriptions to open or close on the feed. A handle owns exactly one;
/// a set appears only when the idle sweep frees several at once.
type Subs = Vec<(Contract, SubscriptionKind)>;

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

fn msg_type_of(data: &StreamData) -> Option<StreamMsgType> {
    Some(match data {
        StreamData::Quote { .. } => StreamMsgType::Quote,
        StreamData::Trade { .. } => StreamMsgType::Trade,
        StreamData::OpenInterest { .. } => StreamMsgType::OpenInterest,
        StreamData::MarketValue { .. } => StreamMsgType::MarketValue,
        _ => return None,
    })
}

/// Display name for a contract, used only in tool output.
fn label(c: &Contract) -> String {
    match (c.expiration, c.is_call, c.strike_thousandths) {
        (Some(exp), Some(is_call), Some(strike)) => {
            let right = if is_call { 'C' } else { 'P' };
            format!("{} {exp} {right} {strike}", c.symbol)
        }
        _ => c.symbol.to_string(),
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

#[derive(Debug, Default)]
struct Book {
    /// Live handles wanting this subscription. One subscription serves all of
    /// them; only the last release closes it.
    refs: usize,
    ring: VecDeque<StreamData>,
    received: u64,
    dropped: u64,
}

#[derive(Debug, Default)]
struct Correlation {
    last_quote: Option<StreamData>,
    prints: VecDeque<Print>,
}

#[derive(Debug)]
struct Watch {
    contract: Contract,
    kind: SubscriptionKind,
    opened_ms: u64,
    read_ms: u64,
}

#[derive(Debug, Default)]
struct Inner {
    watches: HashMap<String, Watch>,
    books: HashMap<BookKey, Book>,
    correlation: HashMap<Contract, Correlation>,
    minted: u64,
}

#[derive(Debug, PartialEq)]
pub enum WatchError {
    /// Unknown, or expired and collected. The caller's recovery is the same
    /// either way: open a new one.
    UnknownHandle,
    NotWatched,
}

impl WatchError {
    fn message(&self) -> &'static str {
        match self {
            Self::UnknownHandle => {
                "unknown or expired stream handle; call stream_watch for a new one"
            }
            Self::NotWatched => "this handle does not cover that contract",
        }
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    inner: Mutex<Inner>,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Recover rather than propagate: one panicking read must not disable
        // every stream tool for the life of the process.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a watch. Returns its handle and the subscriptions that are new to
    /// this process — the ones the caller must open on the feed.
    fn watch(&self, contract: Contract, kind: SubscriptionKind, now: u64) -> (String, bool, Subs) {
        let mut inner = self.lock();
        // Two lists, because they need opposite actions: what the idle sweep
        // freed must be closed on the feed, what this watch is first to want
        // must be opened.
        let expired = Self::collect_expired(&mut inner, now);
        inner.minted += 1;
        let handle = format!("sw_{now:012x}{:04x}", inner.minted & 0xffff);

        let book = inner
            .books
            .entry((contract.clone(), kind.subscribe_code()))
            .or_default();
        let first = book.refs == 0;
        book.refs += 1;

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
    fn release(&self, handle: &str, now: u64) -> Result<Subs, WatchError> {
        let mut inner = self.lock();
        let watch = inner
            .watches
            .remove(handle)
            .ok_or(WatchError::UnknownHandle)?;
        let mut freed = Self::deref(&mut inner, &[(watch.contract, watch.kind)]);
        // A handle that timed out while this one was open still holds a live
        // subscription until someone closes it.
        freed.extend(Self::collect_expired(&mut inner, now));
        Ok(freed)
    }

    fn deref(inner: &mut Inner, subs: &[(Contract, SubscriptionKind)]) -> Subs {
        let mut freed = Vec::new();
        for (contract, kind) in subs {
            let key = (contract.clone(), kind.subscribe_code());
            let Some(book) = inner.books.get_mut(&key) else {
                continue;
            };
            book.refs -= 1;
            if book.refs == 0 {
                inner.books.remove(&key);
                inner.correlation.remove(contract);
                freed.push((contract.clone(), *kind));
            }
        }
        freed
    }

    /// Drop handles nobody has read inside the TTL.
    ///
    /// Swept on mutation rather than from a timer: an untouched handle costs
    /// nothing until someone else opens one, and a sweep tied to the map
    /// cannot drift from it.
    fn collect_expired(inner: &mut Inner, now: u64) -> Subs {
        let ttl = TTL.as_millis() as u64;
        let stale: Vec<String> = inner
            .watches
            .iter()
            .filter(|(_, w)| now.saturating_sub(w.read_ms) > ttl)
            .map(|(h, _)| h.clone())
            .collect();
        let mut freed = Vec::new();
        for handle in stale {
            if let Some(watch) = inner.watches.remove(&handle) {
                freed.extend(Self::deref(inner, &[(watch.contract, watch.kind)]));
            }
        }
        freed
    }

    /// Store a tick. Unknown books are ignored rather than created: the feed
    /// can deliver a contract after its unsubscribe, and inventing a book for
    /// one would leak.
    pub fn ingest(&self, data: StreamData) {
        let (Some(contract), Some(msg)) = (contract_of(&data), msg_type_of(&data)) else {
            return;
        };
        let key = (contract.clone(), msg);
        let mut inner = self.lock();
        let Some(book) = inner.books.get_mut(&key) else {
            return;
        };
        book.received += 1;
        if book.ring.len() == RING {
            book.ring.pop_front();
            book.dropped += 1;
        }
        book.ring.push_back(data.clone());

        let state = inner.correlation.entry(key.0).or_default();
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

    fn books_of(
        inner: &mut Inner,
        handle: &str,
        want: Option<&str>,
        now: u64,
    ) -> Result<Vec<BookKey>, WatchError> {
        let watch = inner
            .watches
            .get_mut(handle)
            .ok_or(WatchError::UnknownHandle)?;
        watch.read_ms = now;
        if want.is_some_and(|w| label(&watch.contract) != w) {
            return Err(WatchError::NotWatched);
        }
        Ok(vec![(watch.contract.clone(), watch.kind.subscribe_code())])
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();
static HANDLER: OnceLock<()> = OnceLock::new();

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

fn contract_arg() -> Value {
    json!({"type": "string", "description": "Restrict to one contract, as named in a previous response. Omit for all."})
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
                without a read, so call stream_release when you are finished.",
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
            "description": "The newest message on each book, with age_ms. An index reports \
                about once a second, so seconds of age are normal there and stale on an \
                option quote.",
            "inputSchema": {"type": "object", "properties": {"handle": handle_arg(), "contract": contract_arg()}, "required": ["handle"]}
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
                    "contract": contract_arg(),
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
                "properties": {"handle": handle_arg(), "contract": contract_arg(),
                               "count": {"type": "integer", "description": "Default 20."}},
                "required": ["handle"]
            }
        }),
        json!({
            "name": "stream_status",
            "description": "What the handle covers: messages received, messages dropped \
                because the buffer filled, and the age of each book's newest message. A \
                non-zero drop count means a window is clipped, not complete.",
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

fn install_handler(client: &Client) -> Result<(), ToolError> {
    if HANDLER.get().is_some() {
        return Ok(());
    }
    client
        .stream()
        .start_streaming(|event: &StreamEvent| {
            if let StreamEvent::Data(data) = event {
                registry().ingest(data.clone());
            }
        })
        .map_err(|e| ToolError::ServerError(format!("could not start streaming: {e}")))?;
    let _ = HANDLER.set(());
    Ok(())
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

        install_handler(client)?;
        let (handle, first, expired) = reg.watch(contract.clone(), kind, now);

        // Close what the idle sweep released before opening anything new, so a
        // forgotten handle cannot hold an allowance the next caller needs.
        for (contract, kind) in &expired {
            let _ = client.stream().unsubscribe(Subscription::Contract {
                contract: contract.clone(),
                kind: *kind,
            });
        }

        if first {
            if let Err(e) = client.stream().subscribe(Subscription::Contract {
                contract: contract.clone(),
                kind,
            }) {
                // Roll the handle back. Leaving it would hand back a handle
                // that receives nothing, and the reference it holds would make
                // the next watch on this contract skip subscribing too.
                let _ = reg.release(&handle, now);
                return Err(ToolError::ServerError(format!("subscribe failed: {e}")));
            }
        }
        return Ok(json!({
            "handle": handle,
            "contract": label(&contract),
            "subscriptions_opened": usize::from(first),
            "expires_after_seconds": TTL.as_secs()
        }));
    }

    let handle =
        str_of("handle").ok_or_else(|| ToolError::InvalidParams("handle is required".into()))?;
    let want = str_of("contract");
    let mut inner = reg.lock();
    let keys = Registry::books_of(&mut inner, handle, want, now)
        .map_err(|e| ToolError::InvalidParams(e.message().into()))?;

    match name {
        "stream_latest" => Ok(json!({
            "books": keys.iter().filter_map(|key| {
                let last = inner.books.get(key)?.ring.back()?;
                Some(json!({"contract": label(&key.0), "message": tick_json(last, now)}))
            }).collect::<Vec<_>>()
        })),
        "stream_window" => {
            let seconds = num_of("seconds", 60) as u64;
            let limit = num_of("limit", 200);
            let floor = now.saturating_sub(seconds * 1_000);
            Ok(json!({
                "window_seconds": seconds,
                "books": keys.iter().filter_map(|key| {
                    let book = inner.books.get(key)?;
                    let mut rows: Vec<&StreamData> = book.ring.iter()
                        .filter(|d| seen_ms(d).is_none_or(|s| s >= floor)).collect();
                    if rows.len() > limit { rows.drain(..rows.len() - limit); }
                    Some(json!({
                        "contract": label(&key.0),
                        "count": rows.len(),
                        "messages": rows.iter().map(|d| tick_json(d, now)).collect::<Vec<_>>()
                    }))
                }).collect::<Vec<_>>()
            }))
        }
        "stream_prints" => {
            let count = num_of("count", 20);
            Ok(json!({
                "books": keys.iter().filter_map(|key| {
                    let state = inner.correlation.get(&key.0)?;
                    let mut rows: Vec<&Print> = state.prints.iter().collect();
                    if rows.len() > count { rows.drain(..rows.len() - count); }
                    Some(json!({
                        "contract": label(&key.0),
                        "prints": rows.iter().map(|p| json!({
                            "trade": tick_json(&p.trade, now),
                            "quote_before": p.quote_before.as_ref().map(|q| tick_json(q, now)),
                            "quotes_after": p.quotes_after.iter().map(|q| tick_json(q, now)).collect::<Vec<_>>()
                        })).collect::<Vec<_>>()
                    }))
                }).collect::<Vec<_>>()
            }))
        }
        "stream_status" => {
            let opened = inner.watches.get(handle).map_or(0, |w| w.opened_ms);
            Ok(json!({
                "open_for_ms": now.saturating_sub(opened),
                "expires_after_ms_idle": TTL.as_millis() as u64,
                "books": keys.iter().filter_map(|key| {
                    let book = inner.books.get(key)?;
                    Some(json!({
                        "contract": label(&key.0),
                        "received": book.received,
                        "dropped": book.dropped,
                        "held": book.ring.len(),
                        "age_ms": book.ring.back().and_then(seen_ms).map(|s| now.saturating_sub(s))
                    }))
                }).collect::<Vec<_>>()
            }))
        }
        "stream_release" => {
            drop(inner);
            let freed = reg
                .release(handle, now)
                .map_err(|e| ToolError::InvalidParams(e.message().into()))?;
            let mut closed = 0usize;
            let mut failures = Vec::new();
            for (contract, kind) in &freed {
                match client.stream().unsubscribe(Subscription::Contract {
                    contract: contract.clone(),
                    kind: *kind,
                }) {
                    Ok(()) => closed += 1,
                    // Report it. Claiming a close that did not happen leaves
                    // the caller believing an allowance was returned when the
                    // feed still holds it.
                    Err(e) => failures.push(format!("{}: {e}", label(contract))),
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

    fn trade(c: &Contract, price: f64) -> StreamData {
        StreamData::Trade {
            contract: Arc::new(c.clone()),
            ms_of_day: 1,
            sequence: 1,
            condition: 0,
            size: 1,
            exchange: 0,
            price,
            date: 20260915,
            received_at_ns: 0,
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
        assert_eq!(reg.release(&b, 1_004), Err(WatchError::UnknownHandle));
    }

    #[test]
    fn the_ring_is_bounded_and_counts_what_it_drops() {
        let reg = Registry::default();
        let c = stock("AAPL");
        let (h, _, _) = reg.watch(c.clone(), SubscriptionKind::Trade, 0);
        for i in 0..(RING + 10) {
            reg.ingest(trade(&c, i as f64));
        }
        let mut inner = reg.lock();
        let keys = Registry::books_of(&mut inner, &h, None, 1).expect("books");
        let book = inner.books.get(&keys[0]).expect("book");
        assert_eq!(book.received as usize, RING + 10);
        assert_eq!(book.ring.len(), RING, "holds its capacity, not the tape");
        assert_eq!(
            book.dropped, 10,
            "a reader must be able to tell a clipped window from a whole one"
        );
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
        reg.ingest(trade(&c, 1.08));
        reg.ingest(quote(&c, 1.02, 1.12));
        reg.ingest(quote(&c, 1.03, 1.13));
        reg.ingest(quote(&c, 1.04, 1.14));

        let inner = reg.lock();
        let state = inner.correlation.get(&c).expect("correlation");
        assert_eq!(state.prints.len(), 1);
        let p = &state.prints[0];
        assert!(p.quote_before.is_some());
        assert_eq!(
            p.quotes_after.len(),
            2,
            "the feed sends two after a print; a third belongs to the next one"
        );
        drop(inner);
        let _ = h;
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

        let mut inner = reg.lock();
        assert_eq!(
            Registry::books_of(&mut inner, &stale, None, past_ttl),
            Err(WatchError::UnknownHandle)
        );
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
            vec![(c, SubscriptionKind::Trade)],
            "the swept handle's subscription must come back to be closed"
        );
    }

    #[test]
    fn a_contract_the_handle_does_not_cover_is_refused() {
        let reg = Registry::default();
        let (h, _, _) = reg.watch(stock("AAPL"), SubscriptionKind::Trade, 0);
        let mut inner = reg.lock();
        assert_eq!(
            Registry::books_of(&mut inner, &h, Some("MSFT"), 1),
            Err(WatchError::NotWatched)
        );
    }

    #[test]
    fn a_tick_for_an_unwatched_book_creates_nothing() {
        let reg = Registry::default();
        let (_h, _, _) = reg.watch(stock("AAPL"), SubscriptionKind::Trade, 0);
        reg.ingest(trade(&stock("MSFT"), 5.0));
        assert_eq!(reg.lock().books.len(), 1, "no book invented for MSFT");
    }
}
