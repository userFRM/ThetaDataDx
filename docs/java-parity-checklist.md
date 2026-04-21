# Java Terminal Parity Checklist

Feature-by-feature comparison of the Rust SDK against the Java terminal's
behavior. `[✓]` = parity, `[✗]` = intentional deviation (documented),
`[~]` = partial / in progress.

> **Last audited**: 2026-04-03 against the Java terminal v202603181.
> Coverage: all 21 `StreamMsgType` codes, all 19 `RemoveReason` codes, all 4
> `StreamResponseType` codes, all 4 `SecType` codes, all 30 `ReqType` codes,
> `TradeTick` (16-field), `TradeRef` (8-field), `QuoteTick` (11-field),
> `OhlcTick` (9-field), `OpenInterestTick` (3-field), OHLCVC derivation,
> `Contract` serialization, FIT codec, `PriceCalcUtils`, `FPSSClient`
> lifecycle, `WSEvents` trade output.

## Wire protocol

| Feature | Parity | Notes |
|---------|:------:|-------|
| gRPC proto definitions (field numbers, types, service methods) | [✓] | Canonical `mdds.proto` from ThetaData engineering. |
| FPSS frame layout (`1-byte LEN` + `1-byte CODE` + `payload`) | [✓] | Byte-for-byte match of `PacketStream.readFrame()`. |
| FPSS auth handshake (`CREDENTIALS` -> `METADATA`/`DISCONNECTED`) | [✓] | |
| FIT nibble encoding (digit values, separators, `DATE` marker, `SPACING=5`) | [✓] | |
| FIT delta compression (first tick absolute, subsequent deltas) | [✓] | |
| `Contract` binary serialization (stock vs option wire format) | [✓] | Fixed in v1.2.0. |
| FPSS ping interval (100 ms), payload `[0x00]`, 2 000 ms initial delay | [✓] | |
| FPSS credential length read as unsigned (`readUnsignedShort`) | [✓] | |
| FPSS write buffer flushed only on `PING` (batched writes) | [✓] | |
| FPSS `ROW_SEP` resets field index to `SPACING` unconditionally | [✓] | |
| FPSS contract ID extracted via FIT decode | [✓] | |
| FPSS delta state cleared on `START`/`STOP` signals | [✓] | |
| `"client": "terminal"` in gRPC `query_parameters` | [✓] | |
| Price encoding formula (`value * 10^(type - 10)`) | [✓] | |
| All enum codes (`StreamMsgType`, `RemoveReason`, `SecType`, `DataType`, ...) | [✓] | |

## Authentication

| Feature | Parity | Notes |
|---------|:------:|-------|
| `StreamMsgType::Credentials` plaintext email+password over TLS | [✓] | |
| Nexus auth URL, terminal key, request/response format | [✓] | |
| Nexus 401/404 handling treated as invalid credentials | [✓] | |
| `SubjectPublicKeyInfo` (SPKI) pinning on FPSS TLS | [✗] | Intentional improvement — Java trusts the system CA store, which accepts an expired FPSS certificate and would let any system-trusted cert impersonate the server. The Rust verifier pins on the SPKI digest and enforces a hostname allowlist. |

## Connection lifecycle

| Feature | Parity | Notes |
|---------|:------:|-------|
| Embedded library vs standalone daemon | [✗] | Rust is an in-process library; Java launches as a JVM daemon exposing REST/WS. Same connection longevity, no IPC overhead. |
| Unified `ThetaDataDx::connect` (auth + MDDS + FPSS in one client) | [✗] | Intentional improvement — one auth call, persistent gRPC channel, lazy FPSS connection that stays alive until `stop_streaming()` or `Drop`. |
| MDDS gRPC endpoint (`mdds-01.thetadata.us:443`) | [✓] | |
| FPSS server list (`nj-a:20000/20001`, `nj-b:20000/20001`) | [✓] | |
| Connect timeout | [✓] | Java uses `socket.connect(addr, 2000)` covering TCP+TLS. Rust splits into separate TCP and TLS timeouts, both wrapped in `tokio::time::timeout`. Same effective behavior. |
| Read timeout (10 s) | [✓] | `socket.setSoTimeout(10000)` vs `tokio::time::timeout` around `read_frame()`. |
| Auth HTTP timeout | [~] | Java uses 5 s connect + 5 s read; Rust uses 5 s connect + 10 s total request. Slightly more generous. |
| DNS hostname resolution | [✓] | Both accept hostnames and IPs; Rust uses `ToSocketAddrs` (matches Java's `InetSocketAddress`). |
| TLS stack | [✗] | Java uses JSSE + cacerts; Rust uses `rustls` (ring backend) + `webpki-roots`. Same TLS 1.2/1.3, different implementation. |

## Control events

| Variant | Java code | Rust code | Parity |
|---------|-----------|-----------|:------:|
| `LoginSuccess` | 0 | 0 | [✓] |
| `ContractAssigned` | 1 | 1 | [✓] |
| `ReqResponse` | 2 | 2 | [✓] |
| `Start` | 3 | 3 | [✓] |
| `Stop` | 4 | 4 | [✓] |
| `Quote` | 5 | 5 | [✓] |
| `Trade` | 6 | 6 | [✓] |
| `OpenInterest` | 7 | 7 | [✓] |
| `Ohlc` | 8 | 8 | [✓] |
| `Ohlcvc` | 9 | 9 | [✓] |
| `Disconnected` | 10 | 10 | [✓] |
| `Error` | 11 | 11 | [✓] |
| `Ping` | 12 | 12 | [✓] |
| (full enumeration) | all 21 | all 21 | [✓] |

All 21 `StreamMsgType` codes have byte-identical values. See
[`crates/tdbe/src/types/enums.rs`](../crates/tdbe/src/types/enums.rs).

## Reconnection

| Behavior | Parity | Notes |
|----------|:------:|-------|
| `ReconnectPolicy::Auto` default | [✗] | Java always auto-reconnects (except on `AccountAlreadyConnected`); Rust exposes `reconnect()` as a caller-driven method for explicit retry/backoff control. `reconnect_delay()` helper matches Java's delay calculation. |
| `TooManyRequests` -> 130 s backoff | [✓] | |
| Permanent auth errors -> no retry | [✗] | Rust treats 7 reasons as permanent (`InvalidCredentials`, `InvalidLoginValues`, `InvalidLoginSize`, `AccountAlreadyConnected`, `FreeAccount`, `ServerUserDoesNotExist`, `InvalidCredentialsNullUser`); Java only stops on `AccountAlreadyConnected`. Avoids burning rate limits on bad credentials. |
| Resubscribe active contracts after reconnect | [✓] | |

## Endpoint generation

| Aspect | Java | Rust |
|--------|------|------|
| Handler structure | Each of the 60 gRPC handlers hand-coded with per-endpoint request/response logic | All 61 methods generated from `endpoint_surface.toml` + `mdds.proto`; `MddsClient` macros remain an internal expansion target |
| Source | `net.thetadata.providers.*` handler classes | `crates/thetadatadx/build_support/endpoints/`, `endpoint_surface.toml`, `mdds/endpoints.rs` macro layer |
| Wire contract | Identical | Identical |

Rationale: the Java terminal duplicates boilerplate (auth injection,
QueryInfo setup, response streaming, zstd decompression) across 60 handlers.
`thetadatadx` centralizes the endpoint contract in a checked-in surface spec,
validates it against the wire contract, and generates the registry/runtime/
client projections from that data. Requests remain wire-identical.

## FPSS streaming

| Feature | Parity | Notes |
|---------|:------:|-------|
| Dispatch model (LMAX Disruptor pattern) | [✓] | Java: LMAX Disruptor ring buffer. Rust: `disruptor-rs` v4 — lock-free, bounded-latency, cache-line-padded sequence counters. FPSS I/O thread is fully synchronous. |
| Ring-buffer capacity monitoring | [~] | Java's `RingBuffer.remainingCapacity()` enables back-pressure warnings; `disruptor-rs` v4 does not expose a fill-level API. Known upstream limitation. |
| `FpssEvent` split (`FpssData` + `FpssControl`) | [✗] | Intentional API improvement — enables exhaustive `match` on data-only events without touching lifecycle events. Wire format unchanged. |
| FPSS streaming prices exposed as `f64` | [✗] | Intentional improvement — Rust decodes prices at frame-parse time using the per-cell `price_type`. Java exposes raw integers + `price_type` and requires callers to invoke `PriceCalcUtils` manually. |
| `Contract::option(root, exp, strike, right)` API | [✗] | Intentional improvement — Rust accepts string inputs matching MDDS historical (`"SPY"`, `"20260417"`, `"550"`, `"C"`). Java's `Contract(root, expDate, isCall, strike)` leaks wire-format details. `Contract::option_raw()` is available for the drop-in server. |
| FPSS subscription tracking | [✗] | Rust: per-instance `Mutex`. Java: static `ConcurrentHashMap` shared across all `FPSSClient` instances in the JVM. Rust isolates subscription state per client. |
| Full-type subscribe payload `[req_id: i32 BE] [sec_type: u8]` | [✓] | |
| `contract_map` cleared on `START`/`STOP` | [✓] | Matches Java's `idToContract.clear()`. |
| Binary error-frame skipping | [✗] | Pragmatic improvement for dev-server usability. Rust detects non-printable bytes in `ERROR` frames and skips the frame; Java logs garbled strings. Text error messages are still surfaced as `FpssControl::ServerError`. |

## Tick decoding

| Feature | Parity | Notes |
|---------|:------:|-------|
| `TradeTick` 16-field layout (`data[0]=ms_of_day` ... `data[15]=date`) | [✓] | |
| `QuoteTick` 11-field layout | [✓] | |
| `OhlcTick` 9-field layout | [✓] | |
| `OpenInterestTick` 3-field layout | [✓] | |
| `OHLCVC` server-seed 10-field layout | [✓] | `alloc[0]=id`, `[1]=time`, `[2..9]=OHLCVC` fields. |
| `OHLCVC::processTrade` field extraction (price, priceType, size) | [✓] | Different index paths (Rust indexes into the FIT-decoded array; Java indexes into a pre-parsed trade tick array), same values. |
| `OHLCVC` volume/count use `i64` | [✗] | Java uses `int` (32-bit) and wraps silently on high-volume symbols. Rust uses `i64` — correct values on symbols like SPY that exceed `i32::MAX` cumulative volume. |
| `OHLCVC`-from-trade derivation | [✓] | Default on, opt-out via `DirectConfig::derive_ohlcvc = false`. Java always derives with no toggle. |
| `TradeRef` 8-field vs 16-field auto-detection | [✗] | `thetadatadx` detects the field count from the first absolute tick per `(msg_type, contract_id)` and dispatches to the correct index mapping. Java's `TradeRef.java` hard-codes 8-field indices and applies them to 16-field arrays. |
| FIT overflow handling | [✗] | Java wraps `int` silently; Rust saturates `i64` accumulator to `i32::MAX/MIN`. Real market data never exceeds `i32` range. |
| `PriceCalcUtils.changePriceType()` (`exp <= 0` -> multiply, `exp > 0` -> divide) | [✓] | |
| `PriceCalcUtils.getPriceDouble()` formula (`DOUBLES[pType] * price`) | [✓] | |
| Price decoding: `f64` at parse time | [✗] | Intentional improvement — Rust decodes every `Price` cell to `f64` individually using the cell's own `price_type`. No `price_type` in the public API. Java exposes raw integers + `price_type` and leaves decoding to callers. |

## Greeks

| Feature | Parity | Notes |
|---------|:------:|-------|
| Operator precedence on all formulas | [✓] | Fixed in v1.2.0 to match Java bytecode. Higher-order Greeks (veta, speed, zomma, color, dual_gamma) follow canonical textbook formulas (decompilers lose parenthesization). |
| Vera (`DataType` code 166) | [✓] | Server-returned, not locally computed. |
| `norm_cdf` | [✗] | Java uses Apache Commons Math 3.x (continued-fraction expansion). Rust uses Horner-form Zelen & Severo (~1e-7 accuracy). Numerically equivalent, branch-free polynomial core. |
| `.exp()` vs `Math.pow(Math.E, x)` | [✗] | Rust uses `.exp()` (hardware); Java uses `Math.pow(Math.E, x)` which inserts a `ln(e)` multiply. ~1 ULP precision improvement. |
| Degenerate-input guard (`t=0`, `v=0`) | [✗] | Rust returns `0.0` (or intrinsic value for `value()`); Java returns `NaN`/`Inf`. Prevents silent corruption of downstream portfolio analytics. |
| Precomputed intermediates in `all_greeks()` | [✗] | Numerically identical to independent calls. Java recomputes `d1`/`d2` per-Greek; Rust precomputes once in `all_greeks()`. ~20x fewer transcendental function calls. |

## Validation

| Feature | Parity | Notes |
|---------|:------:|-------|
| Contract root length check | [✗] | Rust: `assert!(root.len() <= 244)`. Java: silent `as byte` truncation. |
| Price-type range check | [✓] | Both enforce `0 <= type < 20`. |
| Frame payload size | [✗] | Rust: `assert!(payload.len() <= 255)` in release. Java: implicit `u8` truncation. |
| Date format validation (8 ASCII digits) | [✗] | Rust validates client-side in `mdds/validate.rs::validate_date()`. Java relies on server-side rejection. |

## Error handling

| Feature | Parity | Notes |
|---------|:------:|-------|
| `CONTRACT` parse failure surfaced to caller | [✗] | Rust emits `FpssEvent::Error`. Java logs and silently drops. |
| `REQ_RESPONSE` parse failure surfaced to caller | [✗] | Same as above. |
| Truncated frame header treated as fatal | [✓] | `EOFException` vs `Error::FpssProtocol`, both error out. |

## Concurrency

| Feature | Parity | Notes |
|---------|:------:|-------|
| Concurrent request limit (`2^tier`) | [✓] | Derived from the Nexus auth response tier, with manual override via `DirectConfig::mdds_concurrent_requests`. |

## QueryInfo fields

| Field | Java | Rust | Parity |
|-------|------|------|:------:|
| `terminal_git_commit` | Build git hash | Empty string | [✗] |
| `client_type` | Empty | `"rust-thetadatadx"` | [✗] |
| `terminal_version` | Empty | Crate version | [✗] |

Rust sets `client_type`/`terminal_version` to help ThetaData's server-side
telemetry distinguish Rust SDK requests. Server accepts both populated and
empty forms.

## Endpoint defaults

| Feature | Parity | Notes |
|---------|:------:|-------|
| `start_time="09:30:00"` / `end_time="16:00:00"` on interval endpoints | [✓] | Matches Java (added v4.2.0). |
| `venue="nqb"` on stock snapshot + intraday history endpoints | [✓] | NASDAQ Basic / UTP SIP — matches Java (added v4.2.0). |
| Interval shorthand normalization (`"60000"` -> `"1m"`) | [✗] | Server accepts both; wire value differs (`normalize_interval()` in `mdds/normalize.rs`). |

## Response streaming

| Feature | Parity | Notes |
|---------|:------:|-------|
| `collect_stream` — materialize to typed `Vec<Tick>` | [~] | Java interposes `ArrayBlockingQueue(2)` between gRPC thread and HTTP writer; Rust has no HTTP writer. `collect_stream` uses an `original_size` pre-allocation hint. |
| `for_each_chunk` — streaming callback | [✗] | Intentional improvement — avoids full materialization for very large responses. |
| `_stream` endpoint variants | [✗] | SDK-only convenience — extend the `for_each_chunk` model to per-endpoint helpers. Ideal for millions-of-rows responses. |

## Right field representation

| Language | Type | Parity |
|----------|------|:------:|
| Rust core / FFI | `i32` (67=Call, 80=Put, 0=absent). `is_call()`/`is_put()` helpers. | [✗] |
| Go | `string` (`"C"`, `"P"`, `""`) | [✗] |
| Python | `string` | [✗] |
| Java internal | integer; WS JSON emits string | reference |

Higher-level SDKs convert at the language boundary to match user
expectations; the Rust core preserves the raw integer for zero-overhead C
interop.

## v2 -> v3 automatic normalizations

These conversions happen automatically in the Rust SDK so callers can pass
either v2-style or v3-style parameter values.

### Right

| v2 | v3 | Where |
|----|----|-------|
| `"C"` / `"c"` | `"call"` | `normalize_right()` in `wire_semantics.rs` |
| `"P"` / `"p"` | `"put"` | `normalize_right()` in `wire_semantics.rs` |
| `"*"` | `"both"` | `normalize_right()` in `wire_semantics.rs` |

### Interval

| v2 (ms) | v3 | Where |
|---------|----|-------|
| `"60000"` | `"1m"` | `normalize_interval()` in `mdds/normalize.rs` |
| `"1000"` | `"1s"` | `normalize_interval()` in `mdds/normalize.rs` |
| `"300000"` | `"5m"` | `normalize_interval()` in `mdds/normalize.rs` |
| already shorthand | pass-through | `normalize_interval()` in `mdds/normalize.rs` |

### Symbol field

The v3 protobuf uses `symbol` in `ContractSpec` (not `root` as in v2). The
Rust SDK has always used `symbol` in its public API and proto definitions —
no conversion needed.

### start_time / end_time

The v2 `rth` boolean is replaced by explicit `start_time`/`end_time`. The
Rust SDK defaults to `"09:30:00"`/`"16:00:00"` on all interval endpoints.

## Intentional deviations (value-adds over Java)

- **SPKI pinning** — authenticates the FPSS server on its public key alone,
  not on the expired certificate chain.
- **Typed event surface across 5 SDKs** — Java's API is untyped and
  callback-based; the Rust core exposes typed `FpssEvent` variants across
  Python / TypeScript / Go / C++ / Rust.
- **Arrow columnar DataFrame adapter** — Java has no DataFrame integration;
  Python's `to_arrow()` / `to_pandas()` / `to_polars()` pipe through
  zero-copy Arrow buffers.
- **Sub-millisecond decode path** — no JVM warmup, no GC pauses; nibble-
  packed FIT decoder and lock-free ring buffer on the streaming path.
- **Zero-copy FFI across Python / TypeScript / Go / C++** — one `extern "C"`
  ABI shared by all non-Rust SDKs.
- **Unified `ThetaDataDx` client** — auth, MDDS, and FPSS behind a single
  long-lived handle with `Deref<Target=MddsClient>` for historical
  methods.
- **Manual reconnect policy** — explicit control over retry policy, backoff
  strategy, and circuit breaking. `reconnect_delay()` helper matches Java's
  timing if desired.
- **Stricter permanent-disconnect handling** — 7 reason codes treated as
  fatal vs Java's 1; avoids futile reconnect loops on bad credentials.
- **Per-instance subscription state** — prevents cross-contamination between
  multiple clients in the same process.
- **`i64` OHLCVC counters** — correct cumulative volume on high-volume
  symbols (SPY, QQQ) where `int` would wrap.
- **FIT overflow saturation** — preserves sign and makes overflow
  detectable rather than silently corrupting tick data.
- **Binary error-frame skipping** — dev-server replay-loop boundary leaks
  raw FIT tick data into `ERROR` frames; Rust skips binary payloads
  instead of logging them as garbled strings.
- **`f64` prices at parse time** — no `price_type` in the public API; every
  `Price` cell is decoded using its own `price_type`.
- **Typed `Contract::option(root, exp, strike, right)` API** — strings
  matching the MDDS historical API; no wire-format leakage.

## Class-level mapping

For a complete enumeration of Java terminal classes and their Rust
equivalents (or why they're not needed), see the table below. It covers all
588 classes in the reference v202603181 build.

### Core protocol (implemented)

| Java class | Rust equivalent | Notes |
|------------|-----------------|-------|
| `fpssclient/FPSSClient.java` | `fpss/mod.rs` | Full streaming client with `disruptor-rs` ring buffer |
| `fpssclient/Contract.java` | `fpss/protocol.rs::Contract` | Wire serialization matches byte-for-byte |
| `fpssclient/OHLCVC.java` | `fpss/mod.rs::OhlcvcAccumulator` | Derives OHLCVC from trade stream |
| `fpssclient/PacketStream.java` | `fpss/framing.rs` | Frame read/write `[len:u8][code:u8][payload]` |
| `fpssclient/StreamPacket.java` | `fpss/framing.rs::Frame` | Frame struct |
| `fie/FITReader.java` | `tdbe::codec::fit` | FIT nibble decoder (738 LOC) |
| `FIE.java` | `tdbe::codec::fie` | FIE nibble encoder |
| `fie/TickIterator.java` | Inline in `fpss/mod.rs::decode_frame()` | Tick iteration over FIT-decoded rows |
| `grpc/GrpcHttpStreamBridge.java` | `mdds/client.rs` | gRPC response streaming (direct to typed structs, no HTTP bridge) |
| `grpc/AbstractGrpcBridge.java` | `mdds/client.rs::collect_stream()` | Base response collection |
| `grpc/GrpcMcpBridge.java` | `tools/mcp/` (separate crate) | MCP integration |
| `auth/UserAuthenticator.java` | `auth/nexus.rs` | Nexus API auth flow |
| `config/CredentialFileParser.java` | `auth/creds.rs` | `creds.txt` parsing |
| `config/ConfigurationManager.java` | `config.rs::DirectConfig` | Server addresses, timeouts |
| `config/BuildInfo.java` | `CARGO_PKG_VERSION` constant | Version identification |
| `math/Greeks.java` | `tdbe::greeks` | 22 Black-Scholes Greeks + IV solver |
| `RestResource.java` | `mdds/endpoints.rs` | REST-to-gRPC bridge, contains all endpoint defaults (venue, start_time, interval) |
| `BetaThetaTerminalGrpc.java` | `proto::beta_theta_terminal_client` | v3 gRPC service stub |

### Enums (implemented)

| Java class | Rust equivalent |
|------------|-----------------|
| `enums/StreamMsgType.java` | `tdbe::types::enums::StreamMsgType` (21 values, exact match) |
| `enums/DataType.java` | `tdbe::types::enums::DataType` (91 values, exact match) |
| `enums/RemoveReason.java` | `tdbe::types::enums::RemoveReason` (18 values, exact match) |
| `enums/SecType.java` | `tdbe::types::enums::SecType` (4 values; Java's `IGNORE(-1)` not needed) |
| `enums/StreamResponseType.java` | `tdbe::types::enums::StreamResponseType` (4 values, exact match) |
| `enums/ReqType.java` | `tdbe::types::enums::ReqType` (39 values, exact match) |
| `enums/RateType.java` | `tdbe::types::enums::RateType` (12 values, exact match) |
| `enums/AccountType.java` | Parsed as `i32` tier in `AuthUser` (functional match) |
| `enums/CalendarType.java` | Not needed (Java REST-layer enum, Rust sends values directly in gRPC) |
| `enums/ReqArg.java` | Not needed (Java REST HTTP parameter mapping, Rust uses typed macros) |

### Tick types (implemented)

| Java class | Rust equivalent |
|------------|-----------------|
| `types/tick/TradeTick.java` | `tdbe::TradeTick` |
| `types/tick/QuoteTick.java` | `tdbe::QuoteTick` |
| `types/tick/OhlcTick.java` | `tdbe::OhlcTick` |
| `types/tick/EodTick.java` | `tdbe::EodTick` |
| `types/tick/TradeQuoteTick.java` | `tdbe::TradeQuoteTick` |
| `types/tick/OpenInterestTick.java` | `tdbe::OpenInterestTick` |
| `types/tick/MarketValueTick.java` | `tdbe::MarketValueTick` |
| `types/tick/IndexSnapshotMarketValueTick.java` | Merged into `MarketValueTick` (same fields) |
| `types/tick/Tick.java` | Base trait methods on each struct impl |
| `types/tick/PriceableTick.java` | `get_price()` / `bid_price()` / `ask_price()` methods on tick structs |
| `types/Price.java` | `tdbe::Price` |
| `types/Right.java` | `tdbe::types::enums::Right` |
| `types/Venue.java` | `tdbe::types::enums::Venue` (`Nqb`, `UtpCta`) |
| `types/ResultsFormat.java` | Not needed (JSON/CSV/HTML enum for REST layer) |
| `types/MarketHoliday.java` | `tdbe::CalendarDay` |

### Utility classes

| Java class | Status | Reason |
|------------|--------|--------|
| `utils/PriceCalcUtils.java` | IMPLEMENTED | `Price::to_f64()` + `Price::new()` in `tdbe` |
| `utils/TimeUtils.java` / `TimeUtils.java` | NOT NEEDED | Rust uses `std::time`, no custom time utils required |
| `utils/Utils.java` | NOT NEEDED | General Java utilities (null checks, string helpers) |
| `utils/JsonResponseUtils.java` | NOT NEEDED | REST response formatting (we use `sonic_rs` directly) |
| `utils/PojoMessageUtils.java` | NOT NEEDED | Protobuf-to-POJO conversion for HTTP (we decode to typed structs) |
| `utils/StreamUtils.java` | NOT NEEDED | Java stream helpers |
| `utils/MarketCalendarUtils.java` | NOT NEEDED | Calendar formatting for REST responses |
| `ByteBuffCollection.java` | IMPLEMENTED | `decode.rs` (response buffering + zstd decompression) |
| `Timer.java` | NOT NEEDED | Rust uses `std::thread::sleep` |
| `Intervalized.java` | NOT NEEDED | Interface for interval aggregation (server-side) |

### Error / exception classes

| Java class | Rust equivalent |
|------------|-----------------|
| `auth/AuthException.java` | `Error::Auth(String)` |
| `exceptions/BadConfigurationException.java` | `Error::Config(String)` |
| `exceptions/ClientException.java` | `Error::Fpss(String)` / `Error::FpssProtocol(String)` |
| `exceptions/NoDataException.java` | `Error::NoData` |
| `exceptions/ProcessingError.java` | Various `Error` variants |
| `exceptions/BadRequestException.java` | Not needed (client-side validation in Rust) |
| `exceptions/BadSessionException.java` | `Error::Auth(String)` covers this |
| `exceptions/EntitlementsException.java` | `Error::Auth(String)` covers this |
| `exceptions/TerminalUpgradeException.java` | Not needed (no auto-update mechanism) |

### Not needed — JVM daemon infrastructure

These classes exist because the Java terminal runs as a standalone daemon
process with an embedded HTTP server. The Rust SDK is an embedded library —
users call it directly. No HTTP server, no WebSocket server, no CLI daemon.

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `Main.java` | JVM entry point | Rust is a library |
| `JettyRateLimiter.java` | HTTP request rate limiting | No HTTP server; `tokio::Semaphore` for gRPC |
| `Terminal3MgmtResource.java` | REST management (`/v3/terminal/fpss/status`, `/shutdown`) | No management API in a library |
| `CustomStatusCodes.java` | HTTP status codes for REST error responses | No HTTP layer |

### Not needed — WebSocket server

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `websocket/WSServer.java` | WebSocket server setup | Events delivered via callback, not WS |
| `websocket/WSEvents.java` | WS event formatting + heartbeat | Direct struct delivery, no serialization |
| `websocket/EventServlet.java` | WS servlet factory | No servlet container |
| `websocket/MessageType.java` | WS message type codes (46 values) | Internal WS protocol |
| `websocket/QuoteRef.java` | WS quote tick formatter | Ticks are Rust structs, not JSON |
| `websocket/TradeRef.java` | WS trade tick formatter | Same |

`tools/server/` replicates REST+WS as a drop-in Java-terminal replacement,
but that's a standalone tool, not part of the core SDK.

### Not needed — REST HTTP bridge

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `grpc/GrpcHttpStreamBridge.java` | gRPC -> HTTP response bridge | Direct typed struct return |
| `grpc/AbstractGrpcBridge.java` | Base bridge with format dispatch | No format negotiation |
| `types/ResultsFormat.java` | JSON/CSV/HTML/NDJSON enum | SDK returns typed data |

### Not needed — CDI / dependency injection

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `providers/AuthTokenProvider.java` | CDI bean: session token singleton | `SessionToken` held in `MddsClient` |
| `providers/ChannelProvider.java` | CDI bean: gRPC channel singleton | Channel held in `MddsClient` |
| `providers/NonV3RequestFilter.java` | HTTP request filter | No HTTP server |
| `providers/StringListParamConverterProvider.java` | JAX-RS parameter converter | No JAX-RS |
| `providers/ZonedDateTimeConverterProvider.java` | JAX-RS date converter | No JAX-RS |
| `provider/ConfigFile.java` | Config file CDI producer | `DirectConfig` is a plain struct |
| `provider/ObjectMapperResolver.java` | Jackson `ObjectMapper` CDI producer | No Jackson |

### Not needed — CLI daemon commands

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `cmds/CommandExecutor.java` | Stdin command loop (shutdown, status, ...) | Library, not daemon |
| `cmds/DomainCmd.java` | Command enum | No CLI daemon |

`tools/cli/` (`tdx`) covers command-line usage as a separate binary.

### Not needed — server-side / admin

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `UserValidator.java` | Older v2 auth class | Using v3 `UserAuthenticator` |
| `UserDB.java` | Server-side user database | Client-side only |
| `User2.java` | Server-side user model | Auth response parsed into `AuthUser` |
| `session/SessionInfo.java` | Session POJO | Internal to auth flow |
| `session/SessionInfoV3.java` | v3 session POJO | Internal to auth flow |
| `session/SessionRequest.java` | Session request POJO | Internal to auth flow |
| `session/SessionResponse.java` | Session response POJO | Internal to auth flow |
| `session/DisconnectRequest.java` | Disconnect request POJO | Internal to auth flow |
| `profiling/ProfilingTimer.java` | Performance profiling utility | `criterion` benchmarks instead |

### Not needed — config infrastructure

| Java class | Purpose | Why not needed |
|------------|---------|----------------|
| `config/AbstractConfigurationManager.java` | Base config class | `DirectConfig` is simpler |
| `config/AbstractCredentialsConfigurationManager.java` | Credential config base | `Credentials` struct handles this |

### Generated protobuf classes (497 classes)

The `generated/` and `generated/v3grpc/` directories in the Java terminal
contain 497 protobuf-generated classes (Request/Response/OrBuilder types for
every RPC). The Rust equivalent is `tonic::include_proto!()` output from
`mdds.proto`.

| Package | Class count | Rust equivalent |
|---------|-------------|-----------------|
| `generated/` (v2 proto) | ~250 | Historical; superseded by `mdds.proto` |
| `generated/v3grpc/` (v3 proto) | ~247 | `proto` module via `tonic::include_proto!("beta_endpoints")` |

All 60 v3 gRPC RPCs are covered.
