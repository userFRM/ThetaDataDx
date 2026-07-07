//! Streaming reference pages: one page per stream type, language tabs
//! for subscribe / callback / unsubscribe code, plus the event field
//! table from `fpss_event_schema.toml`.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Deserialize;

// ───────────────────────── Event schema (field tables) ──────────────────────

#[derive(Deserialize)]
struct EventSchema {
    events: HashMap<String, EventDef>,
}

#[derive(Deserialize)]
struct EventDef {
    columns: Vec<EventColumn>,
}

#[derive(Deserialize)]
struct EventColumn {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

/// One-sentence docs for streaming event fields. The tick-field names
/// reuse the same sentences as `tick_schema.toml`; the streaming-only
/// fields (`contract`, `received_at_ns`, …) are declared here. Kept in
/// the generator rather than `fpss_event_schema.toml` because that
/// registry also drives generated SDK structs — annotating it would
/// reshape every binding surface for a docs-only need.
fn event_field_doc(name: &str, book: &str) -> String {
    // The quote bid/ask sentences name the order book for the page's
    // security type: stocks carry the BBO on the Nasdaq Basic feed, options
    // carry the OPRA NBBO. Every other field is book-independent and falls
    // through to `event_field_doc_static`.
    match name {
        "bid" => format!("Last {book} bid price."),
        "ask" => format!("Last {book} ask price."),
        "bid_size" => format!("Last {book} bid size."),
        "ask_size" => format!("Last {book} ask size."),
        "bid_exchange" => format!("Exchange code of the {book} bid."),
        "ask_exchange" => format!("Exchange code of the {book} ask."),
        _ => event_field_doc_static(name).to_string(),
    }
}

/// Book-independent field sentences. Split from [`event_field_doc`] so the
/// bid/ask fields can interpolate the order-book term while everything else
/// stays a `&'static str`.
fn event_field_doc_static(name: &str) -> &'static str {
    match name {
        "contract" => "Resolved contract identity (symbol, security type, and option fields).",
        "received_at_ns" => "Local receive timestamp, nanoseconds since the Unix epoch.",
        "ms_of_day" => "Milliseconds since midnight Eastern Time.",
        "date" => "Trading date as a YYYYMMDD integer.",
        "bid_condition" => "Quote condition code on the bid side.",
        "ask_condition" => "Quote condition code on the ask side.",
        "price" => "Trade price.",
        "size" => "Number of contracts or shares traded.",
        "exchange" => "Exchange code where the trade executed.",
        "condition" => "Trade condition code.",
        "sequence" => "Exchange-assigned trade sequence number.",
        "open" => "Opening trade price of the bar.",
        "high" => "Highest traded price of the bar.",
        "low" => "Lowest traded price of the bar.",
        "close" => "Closing traded price of the bar.",
        "volume" => "Number of contracts or shares traded in the bar.",
        "count" => "Number of trades in the bar.",
        "open_interest" => "Total outstanding contracts.",
        "market_bid" => "Calculated market-value bid (stocks and options only).",
        "market_ask" => "Calculated market-value ask (stocks and options only).",
        "market_price" => "Calculated market value; the only populated value for an index.",
        other => panic!("no doc sentence for streaming event field {other}"),
    }
}

fn event_field_type(ty: &str) -> &'static str {
    match ty {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "f64" => "f64",
        "Contract" => "contract",
        other => panic!("unmapped streaming event field type {other}"),
    }
}

fn load_event_schema() -> Result<EventSchema, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string("fpss_event_schema.toml")?;
    Ok(toml::from_str(&raw)?)
}

/// Renders the "Event fields" table for the full event schema — the
/// native SDK callbacks receive every column. Returns `(markdown,
/// rendered field names)` so the caller can compare the table against
/// the WebSocket-frame subset.
fn render_event_table(schema: &EventSchema, event: &str, book: &str) -> (String, Vec<String>) {
    let def = schema
        .events
        .get(event)
        .unwrap_or_else(|| panic!("event {event} not found in fpss_event_schema.toml"));
    let cols: Vec<&EventColumn> = def.columns.iter().collect();
    let mut out = String::new();
    let _ = writeln!(out, "## `{event}` event fields\n");
    let _ = writeln!(
        out,
        "Each update arrives as a `{event}` event with these fields:\n"
    );
    out.push_str("| Field | Type | Description |\n|---|---|---|\n");
    for col in &cols {
        let ty = event_field_type(&col.ty);
        let doc = event_field_doc(&col.name, book);
        let _ = writeln!(out, "| `{}` | {ty} | {doc} |", col.name);
    }
    out.push_str(
        "\nThe `contract` field carries `symbol`, the security type, and — for options — \
         `expiration`, `right`, and the strike. See [Handling Events](/streaming/events) \
         for the full event catalogue and per-language field shapes.\n\n",
    );
    (out, cols.iter().map(|c| c.name.clone()).collect())
}

// ───────────────────────── Stream-type matrix ───────────────────────────────

struct StreamSpec {
    path: &'static str,
    title: &'static str,
    description: &'static str,
    /// Behavior prose under the title (1–3 sentences).
    prose: &'static str,
    /// Event type rendered in the field table.
    event: &'static str,
    /// Subscription-builder expression per language.
    rust_sub: &'static str,
    python_sub: &'static str,
    ts_sub: &'static str,
    cpp_sub: &'static str,
    /// WebSocket envelope fields for the server tab.
    ws_req_type: &'static str,
    ws_sec_type: &'static str,
    ws_contract: Option<&'static str>,
    /// Sidebar group: "Stocks" / "Options" / "Indices".
    group: &'static str,
    /// Sidebar item label.
    label: &'static str,
    /// Optional danger banner rendered at the top of the page body when
    /// the subscription is accepted by the SDK but the upstream feed does
    /// not deliver ticks for it yet.
    warning: Option<&'static str>,
}

const STREAMS: &[StreamSpec] = &[
    StreamSpec {
        path: "streaming/stocks/quote",
        title: "Stock Quotes",
        description: "Real-time BBO quote stream for a stock.",
        prose: "Streams every BBO quote for one stock from the Nasdaq Basic feed. Each change to the best bid or offer delivers a `Quote` event to the registered callback.",
        event: "Quote",
        rust_sub: "Contract::stock(\"AAPL\").quote()",
        python_sub: "Contract.stock(\"AAPL\").quote()",
        ts_sub: "Contract.stock('AAPL').quote()",
        cpp_sub: "thetadatadx::Contract::stock(\"AAPL\").quote()",
        ws_req_type: "QUOTE",
        ws_sec_type: "STOCK",
        ws_contract: Some(r#"{"symbol": "AAPL"}"#),
        group: "Stocks",
        label: "Quote",
        warning: None,
    },
    StreamSpec {
        path: "streaming/stocks/trade",
        title: "Stock Trades",
        description: "Real-time trade stream for a stock.",
        prose: "Streams every trade print for one stock. Each execution delivers a `Trade` event to the registered callback.",
        event: "Trade",
        rust_sub: "Contract::stock(\"AAPL\").trade()",
        python_sub: "Contract.stock(\"AAPL\").trade()",
        ts_sub: "Contract.stock('AAPL').trade()",
        cpp_sub: "thetadatadx::Contract::stock(\"AAPL\").trade()",
        ws_req_type: "TRADE",
        ws_sec_type: "STOCK",
        ws_contract: Some(r#"{"symbol": "AAPL"}"#),
        group: "Stocks",
        label: "Trade",
        warning: None,
    },
    StreamSpec {
        path: "streaming/stocks/full-trade",
        title: "Stock Full Trades",
        description: "Every trade across all stocks in one subscription.",
        prose: "Streams every trade print across the entire stock universe — one subscription, no per-symbol management. For each traded symbol the stream delivers three events, not just the trade: a `Quote` (the last BBO), an `Ohlcvc` bar, and then the `Trade` print itself. Read the symbol off each event's `contract`.",
        event: "Trade",
        rust_sub: "SecType::Stock.full_trades()",
        python_sub: "SecType.STOCK.full_trades()",
        ts_sub: "SecType.stock().fullTrades()",
        cpp_sub: "thetadatadx::SecType::stock().full_trades()",
        ws_req_type: "FULL_TRADES",
        ws_sec_type: "STOCK",
        ws_contract: None,
        group: "Stocks",
        label: "Full Trades",
        warning: None,
    },
    StreamSpec {
        path: "streaming/stocks/market-value",
        title: "Stock Market Value",
        description: "Real-time calculated market value for a stock.",
        prose: "Streams the calculated market value for one stock, delivered as a `MarketValue` event. Each update carries the calculated `market_bid`, `market_ask`, and `market_price`.",
        event: "MarketValue",
        rust_sub: "Contract::stock(\"AAPL\").market_value()",
        python_sub: "Contract.stock(\"AAPL\").market_value()",
        ts_sub: "Contract.stock('AAPL').marketValue()",
        cpp_sub: "thetadatadx::Contract::stock(\"AAPL\").market_value()",
        ws_req_type: "MARKET_VALUE",
        ws_sec_type: "STOCK",
        ws_contract: Some(r#"{"symbol": "AAPL"}"#),
        group: "Stocks",
        label: "Market Value",
        warning: None,
    },
    StreamSpec {
        path: "streaming/options/quote",
        title: "Option Quotes",
        description: "Real-time NBBO quote stream for an option contract.",
        prose: "Streams every NBBO update for one option contract. Each change to the national best bid or offer delivers a `Quote` event to the registered callback.",
        event: "Quote",
        rust_sub: "Contract::option(\"SPY\", OptionLeg { expiration: \"20260618\", strike: \"570\", right: \"C\" })?.quote()",
        python_sub: "Contract.option(\"SPY\", expiration=\"20260618\", strike=\"570\", right=\"C\").quote()",
        ts_sub: "Contract.option('SPY', { expiration: '20260618', strike: '570', right: 'C' }).quote()",
        cpp_sub: "thetadatadx::Contract::option(\"SPY\", {.expiration = \"20260618\", .strike = \"570\", .right = \"C\"}).quote()",
        ws_req_type: "QUOTE",
        ws_sec_type: "OPTION",
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570000, "right": "C"}"#),
        group: "Options",
        label: "Quote",
        warning: None,
    },
    StreamSpec {
        path: "streaming/options/trade",
        title: "Option Trades",
        description: "Real-time trade stream for an option contract.",
        prose: "Streams every trade print for one option contract. Each execution delivers a `Trade` event to the registered callback.",
        event: "Trade",
        rust_sub: "Contract::option(\"SPY\", OptionLeg { expiration: \"20260618\", strike: \"570\", right: \"C\" })?.trade()",
        python_sub: "Contract.option(\"SPY\", expiration=\"20260618\", strike=\"570\", right=\"C\").trade()",
        ts_sub: "Contract.option('SPY', { expiration: '20260618', strike: '570', right: 'C' }).trade()",
        cpp_sub: "thetadatadx::Contract::option(\"SPY\", {.expiration = \"20260618\", .strike = \"570\", .right = \"C\"}).trade()",
        ws_req_type: "TRADE",
        ws_sec_type: "OPTION",
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570000, "right": "C"}"#),
        group: "Options",
        label: "Trade",
        warning: None,
    },
    StreamSpec {
        path: "streaming/options/open-interest",
        title: "Option Open Interest",
        description: "Open-interest updates for an option contract.",
        prose: "Streams open-interest updates for one option contract. OPRA reports open interest each morning around 06:30 ET, reflecting the prior session; each report delivers an `OpenInterest` event.",
        event: "OpenInterest",
        rust_sub: "Contract::option(\"SPY\", OptionLeg { expiration: \"20260618\", strike: \"570\", right: \"C\" })?.open_interest()",
        python_sub: "Contract.option(\"SPY\", expiration=\"20260618\", strike=\"570\", right=\"C\").open_interest()",
        ts_sub: "Contract.option('SPY', { expiration: '20260618', strike: '570', right: 'C' }).openInterest()",
        cpp_sub: "thetadatadx::Contract::option(\"SPY\", {.expiration = \"20260618\", .strike = \"570\", .right = \"C\"}).open_interest()",
        ws_req_type: "OPEN_INTEREST",
        ws_sec_type: "OPTION",
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570000, "right": "C"}"#),
        group: "Options",
        label: "Open Interest",
        warning: Some("Streaming open interest is not live on the upstream feed yet, so this subscription does not deliver ticks. For open interest today, use the [flat files](/articles/flat-files) (last 7 days) or the [historical open-interest endpoint](/reference/option/history/open-interest)."),
    },
    StreamSpec {
        path: "streaming/options/full-trade",
        title: "Option Full Trades",
        description: "Every option trade across all underlyings in one subscription.",
        prose: "Streams every option trade print across the entire OPRA universe — one subscription, no per-contract management. For each traded contract the stream delivers more than the trade: a `Quote` (the last NBBO) and an `Ohlcvc` bar arrive before the `Trade` print, and the next two NBBO `Quote` updates for that contract arrive after it. Read the contract identity off each event's `contract`.",
        event: "Trade",
        rust_sub: "SecType::Option.full_trades()",
        python_sub: "SecType.OPTION.full_trades()",
        ts_sub: "SecType.option().fullTrades()",
        cpp_sub: "thetadatadx::SecType::option().full_trades()",
        ws_req_type: "FULL_TRADES",
        ws_sec_type: "OPTION",
        ws_contract: None,
        group: "Options",
        label: "Full Trades",
        warning: None,
    },
    StreamSpec {
        path: "streaming/options/full-open-interest",
        title: "Option Full Open Interest",
        description: "Open-interest updates for every option contract in one subscription.",
        prose: "Streams the morning open-interest reports for every option contract — one subscription covering the entire OPRA universe. Each report delivers an `OpenInterest` event.",
        event: "OpenInterest",
        rust_sub: "SecType::Option.full_open_interest()",
        python_sub: "SecType.OPTION.full_open_interest()",
        ts_sub: "SecType.option().fullOpenInterest()",
        cpp_sub: "thetadatadx::SecType::option().full_open_interest()",
        ws_req_type: "FULL_OPEN_INTEREST",
        ws_sec_type: "OPTION",
        ws_contract: None,
        group: "Options",
        label: "Full Open Interest",
        warning: Some("Streaming open interest is not live on the upstream feed yet, so this subscription does not deliver ticks. For open interest today, use the [flat files](/articles/flat-files) (last 7 days) or the [historical open-interest endpoint](/reference/option/history/open-interest)."),
    },
    StreamSpec {
        path: "streaming/options/market-value",
        title: "Option Market Value",
        description: "Real-time calculated market value for an option contract.",
        prose: "Streams the calculated market value for one option contract, delivered as a `MarketValue` event. Each update carries the calculated `market_bid`, `market_ask`, and `market_price`.",
        event: "MarketValue",
        rust_sub: "Contract::option(\"SPY\", OptionLeg { expiration: \"20260618\", strike: \"570\", right: \"C\" })?.market_value()",
        python_sub: "Contract.option(\"SPY\", expiration=\"20260618\", strike=\"570\", right=\"C\").market_value()",
        ts_sub: "Contract.option('SPY', { expiration: '20260618', strike: '570', right: 'C' }).marketValue()",
        cpp_sub: "thetadatadx::Contract::option(\"SPY\", {.expiration = \"20260618\", .strike = \"570\", .right = \"C\"}).market_value()",
        ws_req_type: "MARKET_VALUE",
        ws_sec_type: "OPTION",
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570000, "right": "C"}"#),
        group: "Options",
        label: "Market Value",
        warning: None,
    },
    StreamSpec {
        path: "streaming/indices/price",
        title: "Index Price",
        description: "Real-time price stream for an index.",
        prose: "Streams every index value update. Indices publish price prints through the trade feed, so each update delivers a `Trade` event whose `price` field carries the index value. Indices have no full-stream broadcast; subscribe per index.",
        event: "Trade",
        rust_sub: "Contract::index(\"SPX\").trade()",
        python_sub: "Contract.index(\"SPX\").trade()",
        ts_sub: "Contract.index('SPX').trade()",
        cpp_sub: "thetadatadx::Contract::index(\"SPX\").trade()",
        ws_req_type: "TRADE",
        ws_sec_type: "INDEX",
        ws_contract: Some(r#"{"symbol": "SPX"}"#),
        group: "Indices",
        label: "Price",
        warning: None,
    },
    StreamSpec {
        path: "streaming/indices/market-value",
        title: "Index Market Value",
        description: "Real-time calculated market value for an index.",
        prose: "Streams the calculated market value for an index, delivered as a `MarketValue` event. For an index only `market_price` is populated; the bid/ask market values that accompany stock and option market-value events do not apply to indices. Market value is a per-index subscription with no full-stream broadcast.",
        event: "MarketValue",
        rust_sub: "Contract::index(\"SPX\").market_value()",
        python_sub: "Contract.index(\"SPX\").market_value()",
        ts_sub: "Contract.index('SPX').marketValue()",
        cpp_sub: "thetadatadx::Contract::index(\"SPX\").market_value()",
        ws_req_type: "MARKET_VALUE",
        ws_sec_type: "INDEX",
        ws_contract: Some(r#"{"symbol": "SPX"}"#),
        group: "Indices",
        label: "Market Value",
        warning: None,
    },
];

// ───────────────────────── Per-language example blocks ──────────────────────

fn rust_tab(spec: &StreamSpec) -> String {
    let needs_sectype = spec.rust_sub.starts_with("SecType");
    let imports = if needs_sectype {
        "use thetadatadx::streaming::SecTypeExt;\nuse thetadatadx::streaming::{StreamData, StreamEvent};\nuse thetadatadx::SecType;"
    } else if spec.rust_sub.contains("OptionLeg") {
        "use thetadatadx::streaming::{Contract, OptionLeg};\nuse thetadatadx::streaming::{StreamData, StreamEvent};"
    } else {
        "use thetadatadx::streaming::Contract;\nuse thetadatadx::streaming::{StreamData, StreamEvent};"
    };
    // Full-trade pages handle all three delivered events; every other page
    // narrows on its single event kind.
    let callback = if spec.ws_req_type == "FULL_TRADES" {
        "client.stream().start_streaming(|event: &StreamEvent| match event {\n    StreamEvent::Data(StreamData::Quote { contract, bid, ask, .. }) => {\n        println!(\"{} quote bid={bid} ask={ask}\", contract.symbol);\n    }\n    StreamEvent::Data(StreamData::Ohlcvc { contract, open, high, low, close, .. }) => {\n        println!(\"{} bar o={open} h={high} l={low} c={close}\", contract.symbol);\n    }\n    StreamEvent::Data(StreamData::Trade { contract, price, size, .. }) => {\n        println!(\"{} trade price={price} size={size}\", contract.symbol);\n    }\n    _ => {}\n})?;".to_string()
    } else {
        let (pattern, print) = match spec.event {
            "Quote" => (
                "StreamEvent::Data(StreamData::Quote { contract, bid, ask, .. })",
                "println!(\"{} bid={bid} ask={ask}\", contract.symbol);",
            ),
            "Trade" => (
                "StreamEvent::Data(StreamData::Trade { contract, price, size, .. })",
                "println!(\"{} price={price} size={size}\", contract.symbol);",
            ),
            "OpenInterest" => (
                "StreamEvent::Data(StreamData::OpenInterest { contract, open_interest, .. })",
                "println!(\"{} oi={open_interest}\", contract.symbol);",
            ),
            "MarketValue" => (
                "StreamEvent::Data(StreamData::MarketValue { contract, market_price, .. })",
                "println!(\"{} market_price={market_price}\", contract.symbol);",
            ),
            other => panic!("no Rust callback template for event {other}"),
        };
        format!("client.stream().start_streaming(|event: &StreamEvent| {{\n    if let {pattern} = event {{\n        {print}\n    }}\n}})?;")
    };
    format!(
        "```rust\n{imports}\n\n{callback}\n\nlet sub = {};\nclient.stream().subscribe(sub.clone())?;\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream().unsubscribe(sub)?;\n```\n",
        spec.rust_sub
    )
}

fn python_tab(spec: &StreamSpec) -> String {
    let import = if spec.python_sub.starts_with("SecType") {
        "from thetadatadx import SecType"
    } else {
        "from thetadatadx import Contract"
    };
    // Full-trade pages handle all three delivered events; every other page
    // narrows on its single event kind.
    let body = if spec.ws_req_type == "FULL_TRADES" {
        "def on_event(event):\n    if event.kind == \"quote\":\n        print(event.contract.symbol, \"quote\", event.bid, event.ask)\n    elif event.kind == \"ohlcvc\":\n        print(event.contract.symbol, \"bar\", event.open, event.high, event.low, event.close)\n    elif event.kind == \"trade\":\n        print(event.contract.symbol, \"trade\", event.price, event.size)".to_string()
    } else {
        let (kind, print) = match spec.event {
            "Quote" => (
                "quote",
                "print(event.contract.symbol, event.bid, event.ask)",
            ),
            "Trade" => (
                "trade",
                "print(event.contract.symbol, event.price, event.size)",
            ),
            "OpenInterest" => (
                "open_interest",
                "print(event.contract.symbol, event.open_interest)",
            ),
            "MarketValue" => (
                "market_value",
                "print(event.contract.symbol, event.market_price)",
            ),
            other => panic!("no Python callback template for event {other}"),
        };
        format!("def on_event(event):\n    if event.kind == \"{kind}\":\n        {print}")
    };
    format!(
        "```python\n{import}\n\n{body}\n\nclient.stream.start_streaming(on_event)\n\nsub = {}\nclient.stream.subscribe(sub)\n\n# Remove this stream; the session stays open for other subscriptions.\nclient.stream.unsubscribe(sub)\n```\n",
        spec.python_sub
    )
}

fn typescript_tab(spec: &StreamSpec) -> String {
    let import = if spec.ts_sub.starts_with("SecType") {
        "import { SecType } from 'thetadatadx-ts';"
    } else {
        "import { Contract } from 'thetadatadx-ts';"
    };
    // Full-trade pages handle all three delivered events; every other page
    // narrows on its single event kind.
    let body = if spec.ws_req_type == "FULL_TRADES" {
        "await client.stream.startStreaming((event) => {\n  switch (event.kind) {\n    case 'quote': {\n      const q = event.quote!;\n      console.log(q.contract.symbol, 'quote', q.bid, q.ask);\n      break;\n    }\n    case 'ohlcvc': {\n      const b = event.ohlcvc!;\n      console.log(b.contract.symbol, 'bar', b.open, b.high, b.low, b.close);\n      break;\n    }\n    case 'trade': {\n      const t = event.trade!;\n      console.log(t.contract.symbol, 'trade', t.price, t.size);\n      break;\n    }\n  }\n});".to_string()
    } else {
        let (kind, payload, print) = match spec.event {
            "Quote" => (
                "quote",
                "quote",
                "console.log(e.contract.symbol, e.bid, e.ask);",
            ),
            "Trade" => (
                "trade",
                "trade",
                "console.log(e.contract.symbol, e.price, e.size);",
            ),
            "OpenInterest" => (
                "open_interest",
                "openInterest",
                "console.log(e.contract.symbol, e.openInterest);",
            ),
            "MarketValue" => (
                "market_value",
                "marketValue",
                "console.log(e.contract.symbol, e.marketPrice);",
            ),
            other => panic!("no TypeScript callback template for event {other}"),
        };
        format!("await client.stream.startStreaming((event) => {{\n  if (event.kind === '{kind}') {{\n    const e = event.{payload}!;\n    {print}\n  }}\n}});")
    };
    format!(
        "```typescript\n{import}\n\n{body}\n\nconst sub = {};\nclient.stream.subscribe(sub);\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream.unsubscribe(sub);\n```\n",
        spec.ts_sub
    )
}

fn cpp_tab(spec: &StreamSpec) -> String {
    // Full-trade pages handle all three delivered events; every other page
    // narrows on its single event kind.
    let body = if spec.ws_req_type == "FULL_TRADES" {
        "client.stream().set_callback([](const thetadatadx::StreamEvent& event) {\n    switch (event.kind) {\n        case THETADATADX_STREAM_QUOTE:\n            std::cout << event.quote.contract.symbol << \" quote bid=\" << event.quote.bid << \" ask=\" << event.quote.ask << \"\\n\";\n            break;\n        case THETADATADX_STREAM_OHLCVC:\n            std::cout << event.ohlcvc.contract.symbol << \" bar o=\" << event.ohlcvc.open << \" c=\" << event.ohlcvc.close << \"\\n\";\n            break;\n        case THETADATADX_STREAM_TRADE:\n            std::cout << event.trade.contract.symbol << \" trade price=\" << event.trade.price << \" size=\" << event.trade.size << \"\\n\";\n            break;\n        default:\n            break;\n    }\n});".to_string()
    } else {
        let (kind, payload, print) = match spec.event {
            "Quote" => (
                "THETADATADX_STREAM_QUOTE",
                "quote",
                "std::cout << e.contract.symbol << \" bid=\" << e.bid << \" ask=\" << e.ask << \"\\n\";",
            ),
            "Trade" => (
                "THETADATADX_STREAM_TRADE",
                "trade",
                "std::cout << e.contract.symbol << \" price=\" << e.price << \" size=\" << e.size << \"\\n\";",
            ),
            "OpenInterest" => (
                "THETADATADX_STREAM_OPEN_INTEREST",
                "open_interest",
                "std::cout << e.contract.symbol << \" oi=\" << e.open_interest << \"\\n\";",
            ),
            "MarketValue" => (
                "THETADATADX_STREAM_MARKET_VALUE",
                "market_value",
                "std::cout << e.contract.symbol << \" market_price=\" << e.market_price << \"\\n\";",
            ),
            other => panic!("no C++ callback template for event {other}"),
        };
        format!("client.stream().set_callback([](const thetadatadx::StreamEvent& event) {{\n    if (event.kind == {kind}) {{\n        auto& e = event.{payload};\n        {print}\n    }}\n}});")
    };
    format!(
        "```cpp\n{body}\n\nauto sub = {};\nclient.stream().subscribe(sub);\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream().unsubscribe(sub);\n```\n",
        spec.cpp_sub
    )
}

fn http_tab(spec: &StreamSpec) -> String {
    let mut envelope = format!(
        "{{\"msg_type\": \"STREAM\", \"sec_type\": \"{}\", \"req_type\": \"{}\", \"id\": 1, \"add\": true",
        spec.ws_sec_type, spec.ws_req_type
    );
    if let Some(contract) = spec.ws_contract {
        let _ = write!(envelope, ", \"contract\": {contract}");
    }
    envelope.push('}');

    let mut out = String::from(
        "```http\nGET ws://127.0.0.1:25520/v1/events\n```\n\nWebSocket streaming from the bundled [server binary](/server/). Send one JSON envelope per command; set `\"add\": false` to unsubscribe.\n\n**Example**\n\n",
    );
    let _ = write!(
        out,
        "```bash\nwebsocat ws://127.0.0.1:25520/v1/events\n{envelope}\n```\n"
    );
    if spec.ws_contract.is_some_and(|c| c.contains("strike")) {
        out.push_str(
            "\nThe WebSocket envelope takes the strike as the terminal's 1/10-cent integer (`570000` = $570.00) by default, matching the terminal wire; pass the server's `--strike-format dollars` flag to use a dollar value instead. The native SDK builders take dollars.\n",
        );
    }
    out
}

// ───────────────────────── Full-trade delivery section ──────────────────────

/// Renders the multi-event delivery section for a full-trade page: the
/// per-contract event sequence (quote + OHLC bar before the trade,
/// options also send the next two NBBO quotes after), an annotated
/// example of the sequence, the OHLC-bars note, and the caveats that
/// apply to the full-trade subscription.
fn full_trade_delivery(spec: &StreamSpec) -> String {
    let is_option = spec.ws_sec_type == "OPTION";
    // Options quote the NBBO; stocks quote the BBO on the Nasdaq Basic feed.
    let book = if is_option { "NBBO" } else { "BBO" };
    let after = if is_option {
        " The next two NBBO updates for that contract then arrive as `Quote` events after the trade."
    } else {
        ""
    };

    let mut out = String::from("## What the stream delivers\n\n");
    let _ = write!(
        out,
        "This is not a trade-only feed. For every traded contract the stream delivers three event \
         types: a `Quote`, an `Ohlcvc` bar, and the `Trade` print. The `Quote` (the last {book}) and \
         the `Ohlcvc` bar are sent automatically before the trade occurs, then the `Trade` follows.{after} \
         Narrow on `event.kind` (`quote` / `ohlcvc` / `trade`) to handle each, and read the contract \
         identity off every event's `contract`.\n\n",
    );

    // Example: show the sequence as it reaches the callback.
    let (contract_line, strike_note) = if is_option {
        (
            "QQQ 20231110 P 360.00",
            // TD encodes the strike in 1/10-cent on the wire; the SDK
            // surfaces it in dollars on the resolved contract.
            "\n\nThe `Ohlcvc` bar and the trailing `Quote` updates carry the same `contract`. ThetaData encodes the option strike in tenths of a cent (a $360.00 strike as `360000`); the server's WebSocket emits that integer verbatim by default, while the native SDK resolves it to the dollar strike on `event.contract`.\n",
        )
    } else {
        ("QQQ", "\n")
    };
    let _ = write!(
        out,
        "**Per-contract sequence**\n\n```text\nquote  {contract}  bid/ask (last {book})\nohlcvc {contract}  open/high/low/close, volume, count\ntrade  {contract}  price, size, exchange, condition{after_line}\n```\n{strike_note}\n",
        contract = contract_line,
        after_line = if is_option {
            "\nquote  QQQ 20231110 P 360.00  (next NBBO)\nquote  QQQ 20231110 P 360.00  (next NBBO)"
        } else {
            ""
        },
    );

    // OHLC behavior: the server sends one bar per traded contract,
    // automatically, ahead of the trade — no subscription, no toggle.
    out.push_str(
        "## OHLC bars\n\nThe `Ohlcvc` bars on this stream come from upstream automatically — one is sent for each traded contract before its trade, you do not subscribe to them separately.\n\n",
    );

    // Caveats carried from ThetaData's reference that apply to the request.
    let pro = if is_option {
        "an Options Pro subscription"
    } else {
        "a Stocks Pro subscription"
    };
    let _ = write!(
        out,
        "## Before you subscribe\n\n- This stream requires {pro}.\n- Each new stream request must use a higher `id` than the last; reusing an `id` stops the terminal from automatically resubscribing your earlier streams after a reconnect. The SDK manages the `id` for you; the WebSocket envelope sets it explicitly.\n",
    );
    if is_option {
        out.push_str(
            "- The server's WebSocket envelope takes the option strike as the terminal's 1/10-cent integer by default (`--strike-format dollars` switches to a dollar value); the native SDK builders take dollars.\n",
        );
    }
    out.push('\n');
    out
}

// ───────────────────────── Page assembly ────────────────────────────────────

/// WebSocket-frame field subset per event, mirroring the terminal
/// serializer in `tools/server/src/ws/format.rs` (the authority — keep
/// in sync). `None` = the event has no WebSocket frame.
fn ws_frame_fields(event: &str) -> Option<&'static [&'static str]> {
    Some(match event {
        "Quote" => &[
            "ms_of_day",
            "bid_size",
            "bid_exchange",
            "bid",
            "bid_condition",
            "ask_size",
            "ask_exchange",
            "ask",
            "ask_condition",
            "date",
        ],
        "Trade" => &[
            "ms_of_day",
            "sequence",
            "size",
            "condition",
            "price",
            "exchange",
            "date",
        ],
        "Ohlcvc" => &[
            "ms_of_day",
            "open",
            "high",
            "low",
            "close",
            "volume",
            "count",
            "date",
        ],
        "MarketValue" => &[
            "ms_of_day",
            "market_bid",
            "market_ask",
            "market_price",
            "date",
        ],
        _ => return None,
    })
}

/// Renders one reference page per stream type, returning each as a
/// `(path, contents)` pair: title, prose, per-language tabs, and the
/// event field table.
pub(super) fn render_stream_pages() -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let schema = load_event_schema()?;
    let mut pages = Vec::new();
    for spec in STREAMS {
        let mut out = String::new();
        let _ = writeln!(out, "---");
        let _ = writeln!(out, "title: {}", spec.title);
        let _ = writeln!(out, "description: \"{}\"", spec.description);
        let _ = writeln!(out, "---");
        out.push_str(
            "\n<!-- @generated by `generate_docs_site` from fpss_event_schema.toml. Do not edit by hand. -->\n\n",
        );
        let _ = writeln!(out, "# {}\n", spec.title);
        if let Some(body) = spec.warning {
            let _ = writeln!(
                out,
                "::: danger NOT YET WIRED BY THETADATA SOFTWARE ENGINEERS\n\n{body}\n\n:::\n"
            );
        }
        let _ = writeln!(out, "{}\n", spec.prose);
        out.push_str(
            "The snippets below assume a connected client with streaming started — see [Getting Started](/streaming/) for the connect-and-stream ladder.\n",
        );

        out.push_str("\n<SdkTabs>\n\n");
        let _ = write!(
            out,
            "<template #rust>\n\n{}\n</template>\n\n",
            rust_tab(spec)
        );
        let _ = write!(
            out,
            "<template #python>\n\n{}\n</template>\n\n",
            python_tab(spec)
        );
        let _ = write!(
            out,
            "<template #typescript>\n\n{}\n</template>\n\n",
            typescript_tab(spec)
        );
        let _ = write!(out, "<template #cpp>\n\n{}\n</template>\n\n", cpp_tab(spec));
        let _ = write!(
            out,
            "<template #http>\n\n{}\n</template>\n\n",
            http_tab(spec)
        );
        out.push_str("</SdkTabs>\n\n");

        // The full-trade stream is multi-event: upstream sends a quote and
        // an OHLC bar for every traded contract before the trade print
        // (options add the next two NBBO quotes after), so these pages lead
        // with the delivery sequence and document all three event types.
        let is_full_trade = spec.ws_req_type == "FULL_TRADES";
        // Order-book term for this page's quote fields: stocks carry the BBO
        // on the Nasdaq Basic feed, options the OPRA NBBO.
        let book = if spec.ws_sec_type == "STOCK" {
            "BBO"
        } else {
            "NBBO"
        };
        if is_full_trade {
            out.push_str(&full_trade_delivery(spec));
        }

        // The table documents the full event schema — the native SDK
        // callbacks receive every column. A narrower WebSocket frame is
        // covered by the note below, not by trimming the table. Full-trade
        // pages also render the Quote and Ohlcvc tables, since the stream
        // delivers those events too.
        if is_full_trade {
            let (quote_table, _) = render_event_table(&schema, "Quote", book);
            out.push_str(&quote_table);
            let (ohlcvc_table, _) = render_event_table(&schema, "Ohlcvc", book);
            out.push_str(&ohlcvc_table);
        }
        let (table, table_fields) = render_event_table(&schema, spec.event, book);
        out.push_str(&table);
        // The WebSocket-frame note only earns its place when the table
        // lists more fields than the raw frame carries.
        let ws_subset = ws_frame_fields(spec.event);
        if let Some(fields) = ws_subset.filter(|ws| table_fields.len() > ws.len()) {
            let inline = fields
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ");
            // Payload-object key = the frame's lowercased event type, the
            // same `event_type.to_ascii_lowercase()` the server uses in
            // `tools/server/src/ws/format.rs`. Keyed on the event, not the
            // subscription's `ws_req_type` (a FULL_TRADES subscription still
            // delivers `Trade` frames keyed `trade`).
            let payload = match spec.event {
                "Quote" => "quote",
                "Trade" => "trade",
                // Server emits the OHLCVC bar as an OHLC frame; key is the
                // lowercased `header.type` (`StreamData::Ohlcvc => "OHLC"`).
                "Ohlcvc" => "ohlc",
                "MarketValue" => "market_value",
                other => panic!("no WebSocket payload key for event {other}"),
            };
            let _ = write!(
                out,
                "## WebSocket frame\n\nThe native SDK callbacks (Rust/Python/TypeScript/C++) receive every field above. \
                 Each raw WebSocket frame (the **Server** tab) is `{{ \"header\": {{…}}, \"contract\": {{…}}, \"{payload}\": {{…}} }}`: \
                 `header` and `contract` are always present, while the `{payload}` payload object carries only the terminal-compatible subset: {inline}. \
                 The remaining event fields are delivered to the SDK callbacks, not the `{payload}` payload object.\n\n",
            );
        }
        pages.push((spec.path.to_string(), out));
    }
    Ok(pages)
}

/// Sidebar items for the generated stream-type pages, grouped by
/// security type. Imported by `config.ts` into the Streaming section.
pub(super) fn render_streaming_sidebar() -> String {
    let mut json = String::from("[\n");
    let groups = ["Stocks", "Options", "Indices"];
    for (gi, group) in groups.iter().enumerate() {
        let specs: Vec<&StreamSpec> = STREAMS.iter().filter(|s| s.group == *group).collect();
        let _ = writeln!(
            json,
            "  {{ \"text\": \"{group}\", \"collapsed\": true, \"items\": ["
        );
        for (i, spec) in specs.iter().enumerate() {
            let comma = if i + 1 < specs.len() { "," } else { "" };
            let _ = writeln!(
                json,
                "    {{ \"text\": \"{}\", \"link\": \"/{}\" }}{comma}",
                spec.label, spec.path
            );
        }
        let comma = if gi + 1 < groups.len() { "," } else { "" };
        let _ = writeln!(json, "  ]}}{comma}");
    }
    json.push_str("]\n");
    json
}
