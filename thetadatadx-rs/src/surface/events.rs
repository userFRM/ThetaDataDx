//! The streaming event table.
//!
//! Every event a binding delivers is written once, in `__events!`. The Python
//! and TypeScript bindings expand it into their typed event classes and the
//! conversion from the core [`crate::fpss::StreamEvent`]; the C ABI into its
//! `#[repr(C)]` event structs, the tagged `ThetaDataDxStreamEvent` and its
//! converter. `generate_sdk_surfaces` expands it too, to render the C header
//! mirror, the C++ layout asserts and the streaming reference pages.
//!
//! A consumer passes the name of one of its own macros and receives every
//! entry, in table order, after the tokens it passed:
//!
//! ```ignore
//! thetadatadx::__events! { my_consumer { leading tokens passed through } }
//! // expands to
//! my_consumer! { leading tokens passed through  <entry> <entry> ... }
//! ```
//!
//! # Entries
//!
//! ```ignore
//! data OpenInterest {
//!     doc: "Streaming OpenInterest tick.",
//!     columns: [
//!         contract: Contract "Contract this event refers to.",
//!         open_interest: i32 "Number of outstanding open contracts.",
//!     ],
//! }
//! #[disconnect]
//! control Disconnected {
//!     doc: "Streaming connection ended. ..",
//!     from: Disconnected { reason, cause },
//!     columns: [
//!         reason: i32 = (i32::from(*reason as i16)) "Disconnect code: ..",
//!         cause: i32 = (cause.code()) "Where the disconnect came from: ..",
//!     ],
//! }
//! ```
//!
//! - The name is the class name on every binding; the C ABI prefixes it with
//!   `ThetaDataDxStream`. Its snake case is the `kind` tag and the field name
//!   on the tagged C and TypeScript events.
//! - A `data` entry is a market-data event: each column is the field of the
//!   same name on the core `StreamData` variant of the same name.
//! - A `control` entry is a lifecycle event. `from` is the core
//!   `StreamControl` variant and the fields it binds, and each column says
//!   how its value is computed from them. An entry without `from` has no core
//!   variant: it is what a binding delivers for an event it does not know.
//! - Column types are `i32`, `i64`, `u64`, `u8`, `f64`, `String`, `Bytes` (a
//!   byte payload) and `Contract` (the core contract, read by reference).
//! - A column's string is its documentation on the C ABI.
//! - `#[disconnect]` marks an event that carries a disconnect `reason` and
//!   `cause`; Python and TypeScript add their resolved `reason_name` and
//!   `cause_name`.
//!
//! The table is in name order, which is the order of the C ABI kind tags:
//! a tag's value is its entry's position, so an entry is never reordered.

/// Every streaming event, in kind-tag order.
#[doc(hidden)]
#[macro_export]
macro_rules! __events {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            control Connected {
                doc: "Streaming server connection ack (wire code 4). Carries no payload.",
                from: Connected,
                columns: [],
            }
            control ContractAssigned {
                doc: "Streaming server assigned a contract id. The `contract` payload carries the full resolved contract (root, sec_type, expiration / strike / right for options).",
                from: ContractAssigned { id, contract },
                columns: [
                    id: i32 = (*id) "Wire-internal contract id the FPSS server assigns to this contract.",
                    contract: Contract = (contract) "Contract this event refers to.",
                ],
            }
            #[disconnect]
            control Disconnected {
                doc: "Streaming connection ended. `reason` is the integer disconnect code: the server's own when the server sent a disconnect message (wire code 12), and otherwise the code the client has always reported for the way the connection ended (4 TimedOut for its own read deadline, -1 Unspecified for a socket that closed or failed). Read the resolved reason-name field for its symbolic name, and the cause field for where the disconnect came from.",
                from: Disconnected { reason, cause },
                columns: [
                    reason: i32 = (i32::from(*reason as i16)) "Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended.",
                    cause: i32 = (cause.code()) "Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline.",
                    cause_io_error_kind: String = (cause.io_error_kind_name()) "Name of the I/O error kind behind a read or write failure (`\"ConnectionReset\"`, `\"BrokenPipe\"`, ...). Empty for every other cause.",
                    cause_timeout_ms: u64 = (cause.read_timeout_ms()) "Client read deadline that expired, in milliseconds. Zero for every other cause.",
                ],
            }
            data IndexMarketValue {
                doc: "Streaming index MarketValue tick (wire code 25, index contracts): `ms_of_day`, `date` and `market_price` as the feed sent them; an index has no bid/ask. Per-contract only (no full-stream variant).",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    market_price: f64 "Index market value (dollars): the vendor's indicative figure, the index level moved by a random 1 to 5 cents. The exact level is the index price.",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            control LoginSuccess {
                doc: "Streaming login succeeded. `permissions` is the server's opaque bundle string; diagnostic metadata only; for feature gating use the Nexus REST subscription tiers.",
                from: LoginSuccess { permissions },
                columns: [
                    permissions: String = (permissions.clone()) "Server \"Bundle\" string copied verbatim from the METADATA frame; opaque diagnostic metadata.",
                ],
            }
            control MarketClose {
                doc: "Streaming market-close signal (wire code 32). Carries no payload.",
                from: MarketClose,
                columns: [],
            }
            control MarketOpen {
                doc: "Streaming market-open signal (wire code 30). Carries no payload.",
                from: MarketOpen,
                columns: [],
            }
            data MarketValue {
                doc: "Streaming MarketValue tick (wire code 25). A calculated theoretical market value derived from the real-time bid/ask; `market_bid` / `market_ask` are the quote bid/ask after a size-imbalance + spread-aware nudge, `market_price` is their integer midpoint. Per-contract only (no full-stream variant).",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    market_bid: f64 "Calculated market bid (dollars), nudged from the quote bid.",
                    market_ask: f64 "Calculated market ask (dollars), nudged from the quote ask.",
                    market_price: f64 "Integer midpoint of `market_bid` / `market_ask` (dollars).",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            data Ohlcvc {
                doc: "Streaming OHLCVC bar.",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    open: f64 "Opening price of the bar.",
                    high: f64 "Highest traded price within the bar.",
                    low: f64 "Lowest traded price within the bar.",
                    close: f64 "Closing price of the bar.",
                    volume: i64 "Total traded volume within the bar, in contracts/shares.",
                    count: i64 "Number of trades aggregated into the bar.",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            data OpenInterest {
                doc: "Streaming OpenInterest tick.",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    open_interest: i32 "Number of outstanding open contracts.",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            control ParseError {
                doc: "Streaming protocol-level parse error. Named `ParseError` on every binding so it never collides with the language's own error types (Python's exception classes, the JS global `Error`).",
                from: Error { message },
                columns: [
                    message: String = (message.clone()) "Human-readable error text from the server.",
                ],
            }
            control Ping {
                doc: "Streaming server heartbeat (wire code 10). The server emits PING frames (observed 1-byte payload `[0]`) the client heartbeat logic does not have to answer; payload preserved for diagnostics.",
                from: Ping { payload },
                columns: [
                    payload: Bytes = (payload.clone()) "Raw frame payload bytes, preserved for diagnostics.",
                ],
            }
            data Quote {
                doc: "Streaming Quote tick.",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    bid_size: i32 "Number of contracts/shares resting at the bid.",
                    bid_exchange: i32 "Exchange code posting the bid.",
                    bid: f64 "Bid price.",
                    bid_condition: i32 "Quote condition code for the bid.",
                    ask_size: i32 "Number of contracts/shares resting at the ask.",
                    ask_exchange: i32 "Exchange code posting the ask.",
                    ask: f64 "Ask price.",
                    ask_condition: i32 "Quote condition code for the ask.",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            control Reconnected {
                doc: "Streaming auto-reconnect succeeded; connection is live again. Carries no payload.",
                from: Reconnected,
                columns: [],
            }
            control ReconnectedServer {
                doc: "Streaming server-side reconnect ack (wire code 13). Distinct from `Reconnected`, which the client emits from its auto-reconnect state machine once the new TLS session is authenticated.",
                from: ReconnectedServer,
                columns: [],
            }
            #[disconnect]
            control Reconnecting {
                doc: "Streaming auto-reconnect is about to attempt reconnection. Emitted before sleeping for `delay_ms` milliseconds. `attempt` is 1-based and saturates at the maximum 32-bit signed value if the reconnect loop exceeds 2^31 attempts. `reason` and the cause fields describe the disconnect that triggered the attempt.",
                from: Reconnecting { reason, cause, attempt, delay_ms },
                columns: [
                    reason: i32 = (i32::from(*reason as i16)) "Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended.",
                    cause: i32 = (cause.code()) "Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline.",
                    cause_io_error_kind: String = (cause.io_error_kind_name()) "Name of the I/O error kind behind a read or write failure (`\"ConnectionReset\"`, `\"BrokenPipe\"`, ...). Empty for every other cause.",
                    cause_timeout_ms: u64 = (cause.read_timeout_ms()) "Client read deadline that expired, in milliseconds. Zero for every other cause.",
                    attempt: i32 = (i32::try_from(*attempt).unwrap_or(i32::MAX)) "1-based index of this reconnect attempt.",
                    delay_ms: u64 = (*delay_ms) "Delay, in milliseconds, before the attempt fires.",
                ],
            }
            #[disconnect]
            control ReconnectsExhausted {
                doc: "Streaming auto-reconnect stopped without a user-initiated shutdown; terminal for the session. Emitted when the reconnect budget (attempt count or wall-clock envelope) is exhausted, a permanent disconnect reason short-circuits recovery, a manual policy declines to reconnect, or a custom policy returns no delay. `reason` is the integer disconnect code of the final drop; read the resolved reason-name field for the symbolic name and the cause field for where that drop came from. `attempts` is the number of consecutive reconnect attempts consumed before giving up (0 when no reconnect was attempted).",
                from: ReconnectsExhausted { reason, cause, attempts },
                columns: [
                    reason: i32 = (i32::from(*reason as i16)) "Disconnect code: the server's own when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended.",
                    cause: i32 = (cause.code()) "Where the disconnect came from: 0 the server sent a disconnect message, 1 the server ended the stream without one, 2 a read failed, 3 a write failed, 4 nothing arrived inside the client read deadline.",
                    cause_io_error_kind: String = (cause.io_error_kind_name()) "Name of the I/O error kind behind a read or write failure (`\"ConnectionReset\"`, `\"BrokenPipe\"`, ...). Empty for every other cause.",
                    cause_timeout_ms: u64 = (cause.read_timeout_ms()) "Client read deadline that expired, in milliseconds. Zero for every other cause.",
                    attempts: i32 = (i32::try_from(*attempts).unwrap_or(i32::MAX)) "Number of consecutive reconnect attempts consumed before giving up.",
                ],
            }
            control ReqResponse {
                doc: "Streaming subscription response (wire code 40). `result` is an integer status code (0=Subscribed, 1=Error, 2=MaxStreamsReached, 3=InvalidPerms).",
                from: ReqResponse { req_id, result },
                columns: [
                    req_id: i32 = (*req_id) "Identifier of the subscription request this response answers.",
                    result: i32 = (i32::from(*result as u8)) "Outcome of the subscription request.",
                ],
            }
            control Restart {
                doc: "Streaming server stream restart (wire code 31). The server restarts the stream without dropping the TCP connection; delta decode state should be cleared on receipt.",
                from: Restart,
                columns: [],
            }
            control ServerError {
                doc: "Streaming server-error message (wire code 11).",
                from: ServerError { message },
                columns: [
                    message: String = (message.clone()) "Human-readable error text from the server.",
                ],
            }
            data Trade {
                doc: "Streaming Trade tick.",
                columns: [
                    contract: Contract "Contract this event refers to.",
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time when the event was recorded.",
                    sequence: i32 "Exchange sequence number for ordering trades within the day.",
                    condition: i32 "Primary trade condition code.",
                    size: i32 "Trade size in contracts/shares.",
                    exchange: i32 "Exchange code where the trade printed.",
                    price: f64 "Trade price.",
                    date: i32 "Trading date as `YYYYMMDD`.",
                    received_at_ns: u64 "Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.",
                ],
            }
            control UnknownControl {
                doc: "Streaming control variant the SDK does not yet recognise. Surfaced when a newer protocol revision adds a control event this build predates; keep dispatch logic forward-compatible by handling this variant. Carries no payload.",
                columns: [],
            }
            control UnknownFrame {
                doc: "Streaming server sent a frame with an unrecognised wire code. Raw bytes preserved for diagnostics / upstream bug reports.",
                from: UnknownFrame { code, payload },
                columns: [
                    code: u8 = (*code) "Unrecognized frame code reported by the server.",
                    payload: Bytes = (payload.clone()) "Raw frame payload bytes, preserved for diagnostics.",
                ],
            }
        }
    };
}
