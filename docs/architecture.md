# Architecture

## System Overview

```mermaid
graph LR
    App["Your Rust / Python<br/>Application"]

    subgraph ThetaData Infrastructure
        Nexus["Nexus API<br/>nexus-api.thetadata.us<br/>POST /identity/terminal/auth_user"]
        MDDS["MDDS<br/>mdds-01.thetadata.us:443<br/>gRPC server-streaming<br/>60 RPC methods"]
        FPSS["FPSS<br/>nj-a.thetadata.us:20000/20001<br/>nj-b.thetadata.us:20000/20001<br/>Custom TLS/TCP protocol"]
    end

    App -->|"HTTPS<br/>email + password"| Nexus
    App -->|"gRPC over TLS<br/>session UUID in QueryInfo"| MDDS
    App -->|"TLS/TCP<br/>email + password<br/>FIT-encoded ticks"| FPSS

    Nexus -. "session UUID" .-> App
```

## Authentication Flow

```mermaid
sequenceDiagram
    participant App as thetadatadx Client
    participant Nexus as Nexus API
    participant MDDS as MDDS (gRPC)
    participant FPSS as FPSS (TCP)

    App->>Nexus: POST /identity/terminal/auth_user<br/>Headers: TD-TERMINAL-KEY<br/>Body: {email, password}
    Nexus-->>App: {sessionId: UUID, user: {...}}

    rect rgb(230, 240, 255)
        note right of App: Historical Data Path
        App->>MDDS: gRPC call with QueryInfo<br/>.auth_token.session_uuid = UUID
        MDDS-->>App: stream ResponseData (zstd compressed)
    end

    rect rgb(230, 255, 230)
        note right of App: Streaming Data Path
        App->>FPSS: CREDENTIALS (code 0x00)<br/>[0x00][user_len][email][password]
        FPSS-->>App: METADATA (code 0x03)<br/>[permissions string]
        loop Every 100ms
            App->>FPSS: PING (code 0x0A) [0x00]
        end
    end
```

The terminal API key is a static UUID that identifies the terminal application, not the user. It ships in every copy of the Java terminal.

## MDDS Protocol (Historical Data)

MDDS is a standard gRPC service over TLS, operating on port 443.

### Service Definition

- **Package**: `BetaEndpoints`
- **Service**: `BetaThetaTerminal`
- **Methods**: 60 RPCs, all server-streaming (returning `stream ResponseData`). thetadatadx wraps all 60 gRPC RPCs plus 1 convenience range-query variant = **61 methods** on `ThetaDataDx`, generated from the checked-in endpoint surface spec (`endpoint_surface.toml`) validated against `external.proto`. The internal `DirectClient` still uses macro-generated builders, but endpoint declarations are no longer hand-maintained.
- **Categories**: Stock, Option, Index, Interest Rate, Calendar - each with List, History, Snapshot, AtTime, and Greeks sub-categories

### Request Structure

```mermaid
graph TD
    Req["Request Message"]
    QI["query_info: QueryInfo"]
    AT["auth_token: AuthToken"]
    SU["session_uuid: string<br/><i>from Nexus auth</i>"]
    QP["query_parameters: map"]
    CT["client_type: 'rust-thetadatadx'"]
    GC["terminal_git_commit"]
    TV["terminal_version"]
    P["params: EndpointSpecificQuery"]
    Sym["symbol: string"]
    D["date / start_date / end_date"]
    I["interval: string (ms)"]

    Req --> QI
    Req --> P
    QI --> AT
    QI --> QP
    QI --> CT
    QI --> GC
    QI --> TV
    AT --> SU
    P --> Sym
    P --> D
    P --> I
```

Authentication is **in-band** - the session UUID is inside the protobuf message, not in gRPC metadata headers.

### gRPC Flow Control

The gRPC channel configures `initial_connection_window_size` and `initial_stream_window_size` to match the Java terminal's Netty settings, preventing throughput bottlenecks on large responses (e.g., full trade history for a liquid symbol).

### Concurrent Request Limiting

ThetaDataDx enforces a semaphore (`mdds_concurrent_requests`) that limits the number of in-flight gRPC requests. The default is dynamically derived from the user's subscription tier via `2^tier` (matching the Java terminal's concurrency model), but can be overridden in `DirectConfig`. Each endpoint method acquires a permit before sending and releases it when the response is fully consumed.

### Response Structure

```mermaid
graph TD
    RD["ResponseData"]
    CD["compressed_data: bytes<br/><i>zstd-compressed protobuf</i>"]
    CS["compression_description"]
    AL["algo: ZSTD | NONE"]
    OS["original_size: int32"]

    DT["DataTable"]
    H["headers: [string]<br/><i>column names</i>"]
    R["data_table: [DataValueList]<br/><i>rows</i>"]
    DV["DataValue (oneof)"]
    T["text: string"]
    N["number: int64"]
    PR["price: Price {value, type}"]
    TS["timestamp: ZonedDateTime"]
    NV["null_value: bool"]

    RD --> CD
    RD --> CS
    CS --> AL
    RD --> OS
    CD -. "decompress" .-> DT
    DT --> H
    DT --> R
    R --> DV
    DV --> T
    DV --> N
    DV --> PR
    DV --> TS
    DV --> NV
```

### Response Processing Pipeline

```mermaid
flowchart TD
    A["gRPC stream<br/>(multiple ResponseData chunks)"] --> B["Decompress<br/>zstd::bulk::decompress<br/>(pre-allocated via original_size hint)"]
    B --> C["Decode protobuf<br/>prost::Message::decode → DataTable"]
    C --> D["Merge chunks<br/>concatenate rows, keep first headers"]
    D --> E["Parse to typed ticks<br/>DataTable → Vec&lt;TradeTick / QuoteTick / ...&gt;"]
```

If the `compression_description.algo` field contains an unrecognized algorithm, `decompress_response` returns `Error::Decompress` rather than silently treating the data as uncompressed.

Three response processing modes are available:
- **`collect_stream`** (default): materializes all chunks into a single merged `DataTable`. Uses `original_size` from the compression description as a pre-allocation hint for the decompression buffer. The decompressor uses a slab-recycled thread-local `(Decompressor, Vec<u8>)` pair — the internal buffer retains its capacity across calls, avoiding allocator pressure for repeated decompressions.
- **`for_each_chunk`**: streaming callback that processes each chunk individually without accumulating the full response in memory.
- **`_stream` endpoint variants**: `stock_history_trade_stream`, `stock_history_quote_stream`, `option_history_trade_stream`, `option_history_quote_stream` — these combine the gRPC call with `for_each_chunk` processing in a single method call, ideal for endpoints returning millions of rows.

### Build-time Generation Pipeline

ThetaDataDx has two generation pipelines at build time:
- tick parser generation from `tick_schema.toml`
- endpoint surface generation from `endpoint_surface.toml` validated against `external.proto`

```mermaid
flowchart LR
    TOML["tick_schema.toml<br/><i>14 tick type definitions<br/>with column schemas</i>"]
    SURFACE["endpoint_surface.toml<br/><i>endpoint spec<br/>groups + templates</i>"]
    PROTO["external.proto<br/><i>official wire contract</i>"]
    BUILD["build.rs<br/><i>delegates to build_support/</i>"]
    SUPPORT["build_support/<br/><i>endpoints.rs + ticks.rs</i>"]
    PARSERS["$OUT_DIR/decode_generated.rs<br/><i>parse_* functions</i>"]
    RUNTIME["$OUT_DIR/endpoint_generated.rs<br/><i>shared endpoint runtime</i>"]
    REGISTRY_GEN["$OUT_DIR/registry_generated.rs<br/><i>EndpointMeta static</i>"]
    DIRECT_GEN["$OUT_DIR/direct_*_generated.rs<br/><i>DirectClient declarations</i>"]
    TICK["crates/tdbe/src/types/tick.rs<br/><i>typed tick structs</i>"]
    DECODE["decode.rs<br/><i>include!() + hand-written helpers</i>"]
    ENDPOINT["endpoint.rs<br/><i>include!() + runtime glue</i>"]
    REGISTRY["registry.rs<br/><i>include!() + lookup helpers</i>"]
    DIRECT["direct.rs<br/><i>macro layer + generated declarations</i>"]

    TOML --> BUILD
    SURFACE --> BUILD
    PROTO --> BUILD
    BUILD --> SUPPORT
    SUPPORT --> PARSERS
    SUPPORT --> RUNTIME
    SUPPORT --> REGISTRY_GEN
    SUPPORT --> DIRECT_GEN
    PARSERS --> DECODE
    TICK --> DECODE
    RUNTIME --> ENDPOINT
    REGISTRY_GEN --> REGISTRY
    DIRECT_GEN --> DIRECT
```

The 13 tick layouts are: `TradeTick`, `QuoteTick`, `OhlcTick`, `EodTick`, `OpenInterestTick`, `TradeQuoteTick`, `MarketValueTick`, `GreeksTick`, `IvTick`, `PriceTick`, `CalendarDay`, `InterestRateTick`, `OptionContract`. 9 of these (all except `CalendarDay`, `InterestRateTick`, `PriceTick`, `OptionContract`) carry contract identification fields (`expiration`, `strike`, `right`) populated by the server on wildcard queries.

Adding a new endpoint now means updating the explicit endpoint surface spec rather than hand-wiring matches across multiple transports. See `crates/thetadatadx/proto/MAINTENANCE.md` for the current maintenance flow.

## FPSS Protocol (Real-Time Streaming)

FPSS is a custom binary protocol over TLS/TCP.

### Connection Establishment

```mermaid
sequenceDiagram
    participant C as Client
    participant S1 as nj-a:20000
    participant S2 as nj-a:20001
    participant S3 as nj-b:20000

    C->>S1: TCP connect (2s timeout)
    Note over C,S1: Try servers in order until one connects
    S1-->>C: TCP established
    C->>S1: TLS handshake (rustls + webpki-roots)
    S1-->>C: TLS established
    Note over C: Set TCP_NODELAY = true<br/>Split into read/write halves
```

### Wire Framing

```mermaid
packet-beta
    0-7: "LEN (u8)"
    8-15: "CODE (u8)"
    16-31: "PAYLOAD (LEN bytes) ..."
```

- **LEN**: Payload length (0-255). Does NOT include the 2-byte header.
- **CODE**: Message type (`StreamMsgType` enum).
- **PAYLOAD**: LEN bytes of message-specific data.

Total bytes per message on the wire = `LEN + 2`.

### Message Codes

| Code | Name | Direction | Description |
|------|------|-----------|-------------|
| 0x00 | CREDENTIALS | Client->Server | Auth: `[0x00] [user_len: u16 BE] [user bytes] [pass bytes]` |
| 0x01 | SESSION_TOKEN | Client->Server | Alternative session-based auth |
| 0x02 | INFO | Server->Client | Server info |
| 0x03 | METADATA | Server->Client | Login success, payload = permissions UTF-8 string |
| 0x04 | CONNECTED | Server->Client | Connection acknowledged |
| 0x0A | PING | Client->Server | Heartbeat: `[0x00]` every 100ms |
| 0x0B | ERROR | Server->Client | Error message (UTF-8 text) |
| 0x0C | DISCONNECTED | Server->Client | Disconnect reason: `[reason: i16 BE]` |
| 0x0D | RECONNECTED | Server->Client | Reconnection acknowledged |
| 0x14 | CONTRACT | Server->Client | Contract ID assignment: `[id: FIT-encoded i32] [contract bytes]` |
| 0x15 | QUOTE | Both | Subscribe(C->S) / data(S->C). FIT-encoded quote tick |
| 0x16 | TRADE | Both | Subscribe(C->S) / data(S->C). FIT-encoded trade tick |
| 0x17 | OPEN_INTEREST | Both | Subscribe(C->S) / data(S->C) |
| 0x18 | OHLCVC | Server->Client | FIT-encoded OHLC + volume + count snapshot |
| 0x1E | START | Server->Client | Market open signal |
| 0x1F | RESTART | Server->Client | Server restart signal |
| 0x20 | STOP | Both | Market close(S->C) / shutdown(C->S) |
| 0x28 | REQ_RESPONSE | Server->Client | Subscription result: `[req_id: i32 BE] [code: i32 BE]` |
| 0x33 | REMOVE_QUOTE | Client->Server | Unsubscribe quotes |
| 0x34 | REMOVE_TRADE | Client->Server | Unsubscribe trades |
| 0x35 | REMOVE_OI | Client->Server | Unsubscribe open interest |

### Auth Handshake

```mermaid
sequenceDiagram
    participant C as Client
    participant S as FPSS Server

    C->>S: CREDENTIALS (code 0x00)<br/>[0x00] [user_len: u16 BE]<br/>[email bytes] [password bytes]

    alt Success
        S-->>C: METADATA (code 0x03)<br/>[permissions UTF-8 string]
        Note over C: Start ping loop (100ms)
    else Failure
        S-->>C: DISCONNECTED (code 0x0C)<br/>[reason: i16 BE]
        Note over C: Check RemoveReason code
    end
```

### Subscription Flow

```mermaid
sequenceDiagram
    participant C as Client
    participant S as FPSS Server

    C->>S: QUOTE (code 0x15)<br/>[req_id: i32 BE] [contract bytes]
    S-->>C: REQ_RESPONSE (code 0x28)<br/>[req_id: i32 BE] [result: i32 BE]<br/>(0=OK, 1=ERR, 2=MAX, 3=PERMS)

    S-->>C: CONTRACT (code 0x14)<br/>[contract_id: FIT-encoded i32] [contract bytes]<br/>(assigns numeric ID)

    loop Continuous streaming
        S-->>C: QUOTE (code 0x15)<br/>[FIT-encoded tick payload]
    end

    Note over C,S: For full-type subscriptions:<br/>payload = [req_id: i32 BE] [sec_type: u8]<br/>(5 bytes = full type, longer = per-contract)
```

### Contract Binary Format

Contracts are serialized differently for equities vs options:

```mermaid
packet-beta
    title "Stock / Index / Rate Contract"
    0-7: "total_size (u8)"
    8-15: "root_len (u8)"
    16-47: "root (ASCII, root_len bytes)"
    48-55: "sec_type (u8)"
```

```mermaid
packet-beta
    title "Option Contract"
    0-7: "total_size (u8)"
    8-15: "root_len (u8)"
    16-47: "root (ASCII)"
    48-55: "sec_type (u8 = 1)"
    56-87: "exp_date (i32 BE, YYYYMMDD)"
    88-95: "is_call (u8, 1=C 0=P)"
    96-127: "strike (i32 BE, scaled)"
```

Security type codes: Stock=0, Option=1, Index=2, Rate=3.

### Heartbeat

After successful authentication, the client waits 2000ms before sending the first PING (matching the Java terminal's initial delay). After that, it sends a PING (code 0x0A) with payload `[0x00]` every 100ms. Failure to send pings causes the server to disconnect. The write buffer is flushed only on PING sends, batching any intervening subscription messages.

### Disruptor Ring Buffer

FPSS event dispatch uses a lock-free disruptor ring buffer (`disruptor-rs` v4), matching Java's LMAX Disruptor pattern. This eliminates channel overhead on the hot path and provides bounded-latency event delivery. The FPSS I/O thread is fully synchronous - no tokio in the streaming hot path.

Events delivered through the ring buffer use a split enum:
- **`FpssEvent::Data(FpssData)`** — market data: `Quote`, `Trade`, `OpenInterest`, `Ohlcvc`
- **`FpssEvent::Control(FpssControl)`** — lifecycle: `LoginSuccess`, `ContractAssigned`, `ReqResponse`, `MarketOpen`, `MarketClose`, `ServerError`, `Disconnected`, `Error`
- **`FpssEvent::RawData`** — unparsed frames

This split enables callers to `match` on data events without touching lifecycle logic, and vice versa.

### OHLCVC-from-Trade Derivation

The `OhlcvcAccumulator` derives OHLCVC bars from trade ticks in real time. Behavior:
1. The accumulator is **not active** until the server sends an initial OHLCVC bar (server-seeded).
2. After initialization, each incoming trade updates open/high/low/close/volume/count.
3. Derived OHLCVC events are emitted as `FpssEvent::Data(FpssData::Ohlcvc { .. })` alongside the trade event.
4. One accumulator per contract, stored in a `HashMap<i32, OhlcvcAccumulator>`.

This matches the Java terminal's behavior: OHLCVC bars are never emitted purely from trades without a server-provided seed.

FPSS streaming uses separate `FpssClient` wrappers in Go/C++ (not the historical `Client`):
- **Rust**: `tdx.start_streaming(callback)` launches a disruptor-backed event receiver
- **Python**: `tdx.start_streaming()`, `tdx.subscribe_quotes()`, `tdx.subscribe_option_*()`, `tdx.contract_map()`, `tdx.reconnect()`, `tdx.next_event()`, `tdx.stop_streaming()`
- **Go**: `FpssClient` wrapping 26 FFI FPSS functions, including option-level subscribe/unsubscribe, `ContractMap()`, and `Reconnect()`
- **C++**: `FpssClient` RAII class wrapping 26 FFI FPSS functions, including option-level subscribe/unsubscribe, `contract_map()`, and `reconnect()`
- **C FFI**: 26 `extern "C"` functions (`tdx_fpss_connect`, `tdx_fpss_subscribe_*`, `tdx_fpss_unsubscribe_*`, `tdx_fpss_contract_map`, `tdx_fpss_reconnect`, `tdx_fpss_next_event`, `tdx_fpss_shutdown`, `tdx_fpss_free`, etc.)

### Reconnection

| Disconnect Reason | Action |
|-------------------|--------|
| Credential/account errors (0, 1, 2, 6, 9, 17, 18) | **Permanent** - do NOT reconnect |
| `TooManyRequests` (12) | Wait 130 seconds, then reconnect |
| All others | Wait 2 seconds, then reconnect |

Permanent reasons: `InvalidCredentials` (0), `InvalidLoginValues` (1), `InvalidLoginSize` (2), `AccountAlreadyConnected` (6), `FreeAccount` (9), `ServerUserDoesNotExist` (17), `InvalidCredentialsNullUser` (18).

```mermaid
flowchart TD
    D["Disconnected"] --> Check{Reason?}
    Check -->|"Credential/account error<br/>(0,1,2,6,9,17,18)"| Stop["Stop permanently"]
    Check -->|"TooManyRequests (12)"| W130["Wait 130s"] --> Reconnect
    Check -->|"All others"| W2["Wait 2s"] --> Reconnect
    Reconnect["Reconnect"] --> TLS["New TLS connection"]
    TLS --> Auth["Re-authenticate"]
    Auth --> Resub["Re-subscribe all active<br/>subscriptions with req_id = -1"]
```

### Disconnect Reason Codes

| Code | Name |
|------|------|
| -1 | Unspecified |
| 0 | InvalidCredentials |
| 1 | InvalidLoginValues |
| 2 | InvalidLoginSize |
| 3 | GeneralValidationError |
| 4 | TimedOut |
| 5 | ClientForcedDisconnect |
| 6 | AccountAlreadyConnected |
| 7 | SessionTokenExpired |
| 8 | InvalidSessionToken |
| 9 | FreeAccount |
| 12 | TooManyRequests |
| 13 | NoStartDate |
| 14 | LoginTimedOut |
| 15 | ServerRestarting |
| 16 | SessionTokenNotFound |
| 17 | ServerUserDoesNotExist |
| 18 | InvalidCredentialsNullUser |

## FIT Tick Encoding

FPSS tick data uses **FIT** (Feed Interchange Transport) - a nibble-based variable-length integer encoding with delta compression.

### Nibble Values

Each byte contains two 4-bit nibbles: `byte = (high << 4) | low`.

| Nibble | Meaning |
|--------|---------|
| 0-9 | Decimal digit, accumulated left-to-right into current integer |
| 0xB | FIELD_SEPARATOR - flush integer to output, advance to next field |
| 0xC | ROW_SEPARATOR - flush, zero-fill fields up to index 4, jump to index 5 |
| 0xD | END - flush current integer, terminate row, return field count |
| 0xE | NEGATIVE - next flushed integer is negated |

### Encoding Example

The value sequence `[34200000, 1, 0, 0, 0, 100, 4, 15025]` encodes as:

```
34200000 COMMA 1 SLASH 100 COMMA 4 COMMA 15025 END
```

Where SLASH (ROW_SEP) zero-fills fields 2-4 (ext_condition slots), jumping directly to field index 5.

### Delta Compression

```mermaid
flowchart LR
    subgraph "First Tick (absolute)"
        T1["[34200000, 1, 0, 0, 0, 100, 4, 15025]"]
    end

    subgraph "Delta Row"
        D2["[500, 1, 0, 0, 0, 50, 0, -3]"]
    end

    subgraph "Resolved Tick 2"
        T2["[34200500, 2, 0, 0, 0, 150, 4, 15022]"]
    end

    T1 -->|"+"| D2
    D2 -->|"="| T2
```

- **First tick** per contract: absolute values (no delta applied)
- **Subsequent ticks**: each field is a delta added to the previous tick's value
- Fields not present in the delta row carry forward from the previous tick
- **Delta state is cleared** when the server sends START (market open) or STOP (market close) — the next tick after these signals is treated as absolute, not delta-compressed

### Special: DATE Marker

If the first byte of a row is `0xCE` (DATE marker), the entire row is consumed until an END nibble is found, and `read_changes` returns 0. This signals a date boundary in the stream.

## Price Encoding

Prices on the wire use a fixed-point `(value, type)` encoding. This is decoded to `f64` during parsing -- users never see raw integers. Internally, the formula is:

```
real_price = value * 10^(type - 10)
```

| price_type | Decimal places | Multiplier | Example |
|------------|----------------|------------|---------|
| 0 | Zero price | 0 | `(0, 0)` = 0.0 |
| 6 | 4 decimals | 0.0001 | `(1502500, 6)` = 150.2500 |
| 7 | 3 decimals | 0.001 | `(5, 7)` = 0.005 |
| 8 | 2 decimals | 0.01 | `(15025, 8)` = 150.25 |
| 10 | 0 decimals | 1.0 | `(100, 10)` = 100.0 |
| 12 | -2 decimals | 100.0 | `(5, 12)` = 500.0 |

The `Price` struct exists internally in `tdbe::types::price` for wire-level decoding. All public tick fields (`open`, `bid`, `price`, `strike`, etc.) are `f64` -- decoded at parse time.

## FIE String Encoding

FIE (Feed Interchange Encoding) is the complementary encoder used for building FPSS request payloads. It maps a 16-character alphabet to 4-bit nibbles:

| Character | Nibble |
|-----------|--------|
| `0`-`9` | 0-9 |
| `.` | 0xA |
| `,` | 0xB |
| `/` | 0xC |
| `n` | 0xD (newline/end marker) |
| `-` | 0xE |
| `e` | 0xF |

Characters are packed pairwise: `byte = (nibble(c1) << 4) | nibble(c2)`. Odd-length strings pad the last byte with `0xD`. Even-length strings append a `0xDD` terminator.

## Module Architecture

```mermaid
graph TD
    subgraph "tdbe crate"
        direction TB
        TDBE_LIB["lib.rs<br/><i>crate root, re-exports</i>"]

        subgraph tdbe_codec["codec/"]
            C_MOD["mod.rs"]
            C_FIT["fit.rs<br/><i>FIT nibble decoder</i>"]
            C_FIE["fie.rs<br/><i>FIE string encoder</i>"]
        end

        subgraph tdbe_types["types/"]
            T_ENUM["enums.rs<br/><i>91 DataType codes</i>"]
            T_PRICE["price.rs<br/><i>fixed-point Price</i>"]
            T_TICK["tick.rs<br/><i>13 tick types (generated)<br/>TradeTick, QuoteTick, OhlcTick,<br/>EodTick, OpenInterestTick,<br/>TradeQuoteTick,<br/>MarketValueTick, GreeksTick,<br/>IvTick, PriceTick, CalendarDay,<br/>InterestRateTick,<br/>OptionContract</i>"]
        end

        TDBE_GREEKS["greeks.rs<br/><i>22 Greeks + IV</i>"]
        TDBE_FLAGS["flags.rs<br/><i>condition codes</i>"]
        TDBE_LATENCY["latency.rs<br/><i>wire-to-app latency</i>"]
        TDBE_ERROR["error.rs<br/><i>encoding errors</i>"]
    end

    subgraph "thetadatadx crate"
        direction TB
        LIB["lib.rs<br/><i>crate root</i>"]

        subgraph auth["auth/"]
            A_MOD["mod.rs"]
            A_CREDS["creds.rs<br/><i>creds.txt parser</i>"]
            A_NEXUS["nexus.rs<br/><i>Nexus HTTP auth</i>"]
        end

        subgraph fpss["fpss/"]
            F_MOD["mod.rs<br/><i>FpssClient (internal)</i>"]
            F_CONN["connection.rs<br/><i>TLS/TCP failover</i>"]
            F_FRAME["framing.rs<br/><i>wire frames</i>"]
            F_PROTO["protocol.rs<br/><i>contracts, messages</i>"]
            F_RING["ring.rs<br/><i>Disruptor ring buffer</i>"]
        end

        UNIFIED["unified.rs<br/><i>ThetaDataDx — unified entry point<br/>Deref to DirectClient</i>"]
        DIRECT["direct.rs<br/><i>DirectClient (internal) — generated endpoint declarations<br/>on top of builder macros</i>"]
        ENDPOINT_RT["endpoint.rs<br/><i>shared endpoint runtime</i>"]
        CONFIG["config.rs<br/><i>DirectConfig</i>"]
        DECODE["decode.rs<br/><i>zstd + DataTable parsing<br/>(includes generated parsers)</i>"]
        REGISTRY["registry.rs<br/><i>EndpointMeta, ENDPOINTS static</i>"]

        subgraph codegen["Build-time Generation"]
            SCHEMA["tick_schema.toml<br/><i>14 tick type definitions</i>"]
            SURFACE["endpoint_surface.toml<br/><i>endpoint surface spec<br/>groups + templates</i>"]
            BUILD["build.rs<br/><i>delegates to build_support/</i>"]
            SUPPORT["build_support/<br/><i>endpoints.rs + ticks.rs</i>"]
        end

        subgraph proto["proto/"]
            P_EXT["external.proto<br/><i>official MDDS wire contract<br/>60 server-streaming RPCs</i>"]
        end
    end

    LIB --> TDBE_LIB
    UNIFIED --> DIRECT
    UNIFIED --> ENDPOINT_RT
    DIRECT --> auth
    DIRECT --> DECODE
    DIRECT --> proto
    SCHEMA --> BUILD
    SURFACE --> BUILD
    P_EXT --> BUILD
    BUILD --> SUPPORT
    SUPPORT --> DECODE
    SUPPORT --> DIRECT
    SUPPORT --> REGISTRY
    SUPPORT --> ENDPOINT_RT
    F_MOD --> tdbe_codec
    F_MOD --> F_CONN
    F_MOD --> F_FRAME
    F_MOD --> F_PROTO
```
