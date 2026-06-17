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
fn event_field_doc(name: &str) -> &'static str {
    match name {
        "contract" => "Resolved contract identity (symbol, security type, and option fields).",
        "received_at_ns" => "Local receive timestamp, nanoseconds since the Unix epoch.",
        "ms_of_day" => "Milliseconds since midnight Eastern Time.",
        "date" => "Trading date as a YYYYMMDD integer.",
        "bid" => "Last NBBO bid price.",
        "ask" => "Last NBBO ask price.",
        "bid_size" => "Last NBBO bid size.",
        "ask_size" => "Last NBBO ask size.",
        "bid_exchange" => "Exchange code of the NBBO bid.",
        "ask_exchange" => "Exchange code of the NBBO ask.",
        "bid_condition" => "Quote condition code on the bid side.",
        "ask_condition" => "Quote condition code on the ask side.",
        "price" => "Trade price.",
        "size" => "Number of contracts or shares traded.",
        "exchange" => "Exchange code where the trade executed.",
        "condition" => "Trade condition code.",
        "ext_condition1" | "ext_condition2" | "ext_condition3" | "ext_condition4" => {
            "Additional trade condition code."
        }
        "sequence" => "Exchange-assigned trade sequence number.",
        "condition_flags" => "Trade condition flags bitmap.",
        "price_flags" => "Trade price flags bitmap.",
        "volume_type" => "Volume reporting mode: 0 = incremental, 1 = cumulative.",
        "records_back" => "Offset of this record behind the most recent record.",
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

fn render_event_table(schema: &EventSchema, event: &str) -> String {
    let def = schema
        .events
        .get(event)
        .unwrap_or_else(|| panic!("event {event} not found in fpss_event_schema.toml"));
    let mut out = String::new();
    let _ = writeln!(out, "## Event fields\n");
    let _ = writeln!(
        out,
        "Each update arrives as a `{event}` event with these fields:\n"
    );
    out.push_str("| Field | Type | Description |\n|---|---|---|\n");
    for col in &def.columns {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            col.name,
            event_field_type(&col.ty),
            event_field_doc(&col.name)
        );
    }
    out.push_str(
        "\nThe `contract` field carries `symbol`, the security type, and — for options — \
         `expiration`, `right`, and the strike. See [Handling Events](/streaming/events) \
         for the full event catalogue and per-language field shapes.\n\n",
    );
    out
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
}

const STREAMS: &[StreamSpec] = &[
    StreamSpec {
        path: "streaming/stocks/quote",
        title: "Stock Quotes",
        description: "Real-time NBBO quote stream for a stock.",
        prose: "Streams every NBBO update for one stock. Each change to the national best bid or offer delivers a `Quote` event to the registered callback.",
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
    },
    StreamSpec {
        path: "streaming/stocks/full-trade",
        title: "Stock Full Trades",
        description: "Every trade across all stocks in one subscription.",
        prose: "Streams every trade print across the entire stock universe — one subscription, no per-symbol management. Each execution delivers a `Trade` event; read the symbol off the event's `contract`.",
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
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570, "right": "C"}"#),
        group: "Options",
        label: "Quote",
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
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570, "right": "C"}"#),
        group: "Options",
        label: "Trade",
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
        ws_contract: Some(r#"{"symbol": "SPY", "expiration": 20260618, "strike": 570, "right": "C"}"#),
        group: "Options",
        label: "Open Interest",
    },
    StreamSpec {
        path: "streaming/options/full-trade",
        title: "Option Full Trades",
        description: "Every option trade across all underlyings in one subscription.",
        prose: "Streams every option trade print across the entire OPRA universe — one subscription, no per-contract management. Each execution delivers a `Trade` event; read the contract identity off the event.",
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
    },
];

// ───────────────────────── Per-language example blocks ──────────────────────

fn rust_tab(spec: &StreamSpec) -> String {
    let needs_sectype = spec.rust_sub.starts_with("SecType");
    let imports = if needs_sectype {
        "use thetadatadx::fpss::protocol::SecTypeExt;\nuse thetadatadx::fpss::{StreamData, StreamEvent};\nuse thetadatadx::SecType;"
    } else {
        "use thetadatadx::fpss::protocol::Contract;\nuse thetadatadx::fpss::{StreamData, StreamEvent};"
    };
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
    format!(
        "```rust\n{imports}\n\nclient.stream().start_streaming(|event: &StreamEvent| {{\n    if let {pattern} = event {{\n        {print}\n    }}\n}})?;\n\nlet sub = {};\nclient.stream().subscribe(sub.clone())?;\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream().unsubscribe(sub)?;\n```\n",
        spec.rust_sub
    )
}

fn python_tab(spec: &StreamSpec) -> String {
    let import = if spec.python_sub.starts_with("SecType") {
        "from thetadatadx import SecType"
    } else {
        "from thetadatadx import Contract"
    };
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
    format!(
        "```python\n{import}\n\ndef on_event(event):\n    if event.kind == \"{kind}\":\n        {print}\n\nclient.stream.start_streaming(on_event)\n\nsub = {}\nclient.stream.subscribe(sub)\n\n# Remove this stream; the session stays open for other subscriptions.\nclient.stream.unsubscribe(sub)\n```\n",
        spec.python_sub
    )
}

fn typescript_tab(spec: &StreamSpec) -> String {
    let import = if spec.ts_sub.starts_with("SecType") {
        "import { SecType } from 'thetadatadx';"
    } else {
        "import { Contract } from 'thetadatadx';"
    };
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
    format!(
        "```typescript\n{import}\n\nclient.stream.startStreaming((event) => {{\n  if (event.kind === '{kind}') {{\n    const e = event.{payload}!;\n    {print}\n  }}\n}});\n\nconst sub = {};\nclient.stream.subscribe(sub);\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream.unsubscribe(sub);\n```\n",
        spec.ts_sub
    )
}

fn cpp_tab(spec: &StreamSpec) -> String {
    let (kind, payload, print) = match spec.event {
        "Quote" => (
            "THETADATADX_FPSS_QUOTE",
            "quote",
            "std::cout << e.contract.symbol << \" bid=\" << e.bid << \" ask=\" << e.ask << \"\\n\";",
        ),
        "Trade" => (
            "THETADATADX_FPSS_TRADE",
            "trade",
            "std::cout << e.contract.symbol << \" price=\" << e.price << \" size=\" << e.size << \"\\n\";",
        ),
        "OpenInterest" => (
            "THETADATADX_FPSS_OPEN_INTEREST",
            "open_interest",
            "std::cout << e.contract.symbol << \" oi=\" << e.open_interest << \"\\n\";",
        ),
        "MarketValue" => (
            "THETADATADX_FPSS_MARKET_VALUE",
            "market_value",
            "std::cout << e.contract.symbol << \" market_price=\" << e.market_price << \"\\n\";",
        ),
        other => panic!("no C++ callback template for event {other}"),
    };
    format!(
        "```cpp\nclient.stream().set_callback([](const thetadatadx::StreamEvent& event) {{\n    if (event.kind == {kind}) {{\n        auto& e = event.{payload};\n        {print}\n    }}\n}});\n\nauto sub = {};\nclient.stream().subscribe(sub);\n\n// Remove this stream; the session stays open for other subscriptions.\nclient.stream().unsubscribe(sub);\n```\n",
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
            "\nThe WebSocket envelope takes the strike in dollars (`570` = $570.00), the same as the SDKs.\n",
        );
    }
    out
}

// ───────────────────────── Page assembly ────────────────────────────────────

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

        out.push_str(&render_event_table(&schema, spec.event));
        pages.push((spec.path.to_string(), out));
    }
    Ok(pages)
}

/// Sidebar items for the generated stream-type pages, grouped by
/// security type. Imported by `config.ts` into the Streaming section.
pub(super) fn render_streaming_sidebar(pages: &[(String, String)]) -> String {
    let _ = pages;
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
