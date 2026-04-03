# Java Terminal Class Mapping

Complete enumeration of all 588 Java classes in ThetaTerminal v202603181 and their Rust equivalents (or why they're not needed).

## Core Protocol (IMPLEMENTED)

| Java Class | Rust Equivalent | Notes |
|-----------|----------------|-------|
| `fpssclient/FPSSClient.java` | `fpss/mod.rs` | Full streaming client with Disruptor ring buffer |
| `fpssclient/Contract.java` | `fpss/protocol.rs::Contract` | Wire serialization matches byte-for-byte |
| `fpssclient/OHLCVC.java` | `fpss/mod.rs::OhlcvcAccumulator` | Derives OHLCVC from trade stream |
| `fpssclient/PacketStream.java` | `fpss/framing.rs` | Frame read/write `[len:u8][code:u8][payload]` |
| `fpssclient/StreamPacket.java` | `fpss/framing.rs::Frame` | Frame struct |
| `fie/FITReader.java` | `tdbe::codec::fit` | FIT nibble decoder (738 LOC) |
| `FIE.java` | `tdbe::codec::fie` | FIE nibble encoder |
| `fie/TickIterator.java` | Inline in `fpss/mod.rs::decode_frame()` | Tick iteration over FIT-decoded rows |
| `grpc/GrpcHttpStreamBridge.java` | `direct.rs` | gRPC response streaming (direct to typed structs, no HTTP bridge) |
| `grpc/AbstractGrpcBridge.java` | `direct.rs::collect_stream()` | Base response collection |
| `grpc/GrpcMcpBridge.java` | `tools/mcp/` (separate crate) | MCP integration |
| `auth/UserAuthenticator.java` | `auth/nexus.rs` | Nexus API auth flow |
| `config/CredentialFileParser.java` | `auth/creds.rs` | `creds.txt` parsing |
| `config/ConfigurationManager.java` | `config.rs::DirectConfig` | Server addresses, timeouts |
| `config/BuildInfo.java` | `CARGO_PKG_VERSION` constant | Version identification |
| `math/Greeks.java` | `tdbe::greeks` | 22 Black-Scholes Greeks + IV solver |
| `RestResource.java` | `direct.rs` | REST-to-gRPC bridge, contains all endpoint defaults (venue, start_time, interval). Our SDK replicates these defaults in direct.rs. |
| `BetaThetaTerminalGrpc.java` | `proto_v3::beta_theta_terminal_client` | v3 gRPC service stub. Rust equivalent: `proto_v3::beta_theta_terminal_client` |

## Enums (IMPLEMENTED)

| Java Class | Rust Equivalent |
|-----------|----------------|
| `enums/StreamMsgType.java` | `tdbe::types::enums::StreamMsgType` (21 values, exact match) |
| `enums/DataType.java` | `tdbe::types::enums::DataType` (91 values, exact match) |
| `enums/RemoveReason.java` | `tdbe::types::enums::RemoveReason` (18 values, exact match) |
| `enums/SecType.java` | `tdbe::types::enums::SecType` (4 values; Java has IGNORE(-1), not needed) |
| `enums/StreamResponseType.java` | `tdbe::types::enums::StreamResponseType` (4 values, exact match) |
| `enums/ReqType.java` | `tdbe::types::enums::ReqType` (39 values, exact match) |
| `enums/RateType.java` | `tdbe::types::enums::RateType` (12 values, exact match) |
| `enums/AccountType.java` | Parsed as `i32` tier in `AuthUser` (functional match) |
| `enums/CalendarType.java` | Not needed (Java REST-layer enum, Rust sends values directly in gRPC) |
| `enums/ReqArg.java` | Not needed (Java REST HTTP parameter mapping, Rust uses typed macros) |

## Tick Types (IMPLEMENTED)

| Java Class | Rust Equivalent |
|-----------|----------------|
| `types/tick/TradeTick.java` | `tdbe::TradeTick` |
| `types/tick/QuoteTick.java` | `tdbe::QuoteTick` |
| `types/tick/OhlcTick.java` | `tdbe::OhlcTick` |
| `types/tick/EodTick.java` | `tdbe::EodTick` |
| `types/tick/SnapshotTradeTick.java` | `tdbe::SnapshotTradeTick` |
| `types/tick/TradeQuoteTick.java` | `tdbe::TradeQuoteTick` |
| `types/tick/OpenInterestTick.java` | `tdbe::OpenInterestTick` |
| `types/tick/MarketValueTick.java` | `tdbe::MarketValueTick` |
| `types/tick/IndexSnapshotMarketValueTick.java` | Merged into `MarketValueTick` (same fields) |
| `types/tick/Tick.java` | Base trait methods on each struct impl |
| `types/tick/PriceableTick.java` | `get_price()` / `bid_price()` / `ask_price()` methods on tick structs |
| `types/Price.java` | `tdbe::Price` |
| `types/Right.java` | `tdbe::types::enums::Right` |
| `types/Venue.java` | `tdbe::types::enums::Venue` (Nqb, UtpCta) |
| `types/ResultsFormat.java` | Not needed (JSON/CSV/HTML enum for REST layer) |
| `types/MarketHoliday.java` | `tdbe::CalendarDay` |

## Utility Classes (IMPLEMENTED or NOT NEEDED)

| Java Class | Status | Reason |
|-----------|--------|--------|
| `utils/PriceCalcUtils.java` | IMPLEMENTED | `Price::to_f64()` + `Price::new()` in tdbe |
| `utils/TimeUtils.java` / `TimeUtils.java` | NOT NEEDED | Rust uses `std::time`, no custom time utils required |
| `utils/Utils.java` | NOT NEEDED | General Java utilities (null checks, string helpers) |
| `utils/JsonResponseUtils.java` | NOT NEEDED | REST response formatting (we use sonic_rs directly) |
| `utils/PojoMessageUtils.java` | NOT NEEDED | Protobuf-to-POJO conversion for HTTP (we decode to typed structs) |
| `utils/StreamUtils.java` | NOT NEEDED | Java stream helpers |
| `utils/MarketCalendarUtils.java` | NOT NEEDED | Calendar formatting for REST responses |
| `ByteBuffCollection.java` | IMPLEMENTED | `decode.rs` (response buffering + zstd decompression) |
| `Timer.java` | NOT NEEDED | Custom timer; Rust uses `std::thread::sleep` |
| `Intervalized.java` | NOT NEEDED | Interface for interval aggregation (server-side) |

## Error/Exception Classes (IMPLEMENTED)

| Java Class | Rust Equivalent |
|-----------|----------------|
| `auth/AuthException.java` | `Error::Auth(String)` |
| `exceptions/BadConfigurationException.java` | `Error::Config(String)` |
| `exceptions/ClientException.java` | `Error::Fpss(String)` / `Error::FpssProtocol(String)` |
| `exceptions/NoDataException.java` | `Error::NoData` |
| `exceptions/ProcessingError.java` | Various `Error` variants |
| `exceptions/BadRequestException.java` | NOT NEEDED (Java REST-layer, client-side validation in Rust) |
| `exceptions/BadSessionException.java` | `Error::Auth(String)` covers this |
| `exceptions/EntitlementsException.java` | `Error::Auth(String)` covers this |
| `exceptions/TerminalUpgradeException.java` | NOT NEEDED (no auto-update mechanism) |

## NOT NEEDED -- JVM Daemon Infrastructure

These classes exist because the Java terminal runs as a standalone daemon process with an embedded HTTP server. Our Rust SDK is an embedded library -- users call it directly from their code. No HTTP server, no WebSocket server, no CLI daemon.

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `Main.java` | JVM entry point: starts Jetty HTTP + WS + FPSS + MCP | Rust is a library, not a daemon |
| `JettyRateLimiter.java` | HTTP request rate limiting via semaphore + queue | No HTTP server; Rust uses tokio::Semaphore for gRPC |
| `Terminal3MgmtResource.java` | REST management: `/v3/terminal/fpss/status`, `/shutdown` | No management API needed in a library |
| `CustomStatusCodes.java` | HTTP status codes (471-572) for REST error responses | No HTTP layer |

## NOT NEEDED -- WebSocket Server

The Java terminal exposes a WebSocket endpoint so local clients can receive streaming data over WS. Our Rust SDK delivers events directly via callback (Disruptor ring buffer).

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `websocket/WSServer.java` | WebSocket server setup | Events delivered via callback, not WS |
| `websocket/WSEvents.java` | WS event formatting + heartbeat | Direct struct delivery, no serialization overhead |
| `websocket/EventServlet.java` | WS servlet factory | No servlet container |
| `websocket/MessageType.java` | WS message type codes (46 values) | Internal WS protocol |
| `websocket/QuoteRef.java` | WS quote tick formatter | Ticks are Rust structs, not JSON |
| `websocket/TradeRef.java` | WS trade tick formatter | Same |

Note: We DO have a separate `tools/server/` crate that replicates the REST+WS server as a drop-in Java terminal replacement. But that's a standalone tool, not part of the core SDK.

## NOT NEEDED -- REST HTTP Bridge

The Java terminal bridges gRPC responses to HTTP/REST responses with format negotiation (JSON, CSV, HTML, NDJSON). Our SDK returns typed Rust structs directly.

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `grpc/GrpcHttpStreamBridge.java` | gRPC -> HTTP response bridge | Direct typed struct return |
| `grpc/AbstractGrpcBridge.java` | Base bridge with format dispatch | No format negotiation |
| `types/ResultsFormat.java` | JSON/CSV/HTML/NDJSON enum | SDK returns typed data |

Note: `tools/server/` replicates this for users who need the REST API.

## NOT NEEDED -- CDI / Dependency Injection

The Java terminal uses Jakarta CDI for dependency injection. Rust doesn't need DI -- dependencies are passed explicitly.

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `providers/AuthTokenProvider.java` | CDI bean: session token singleton | `SessionToken` held in `DirectClient` |
| `providers/ChannelProvider.java` | CDI bean: gRPC channel singleton | Channel held in `DirectClient` |
| `providers/NonV3RequestFilter.java` | HTTP request filter | No HTTP server |
| `providers/StringListParamConverterProvider.java` | JAX-RS parameter converter | No JAX-RS |
| `providers/ZonedDateTimeConverterProvider.java` | JAX-RS date converter | No JAX-RS |
| `provider/ConfigFile.java` | Config file CDI producer | `DirectConfig` is a plain struct |
| `provider/ObjectMapperResolver.java` | Jackson ObjectMapper CDI producer | No Jackson |

## NOT NEEDED -- CLI Daemon Commands

The Java terminal accepts stdin commands when running as a daemon. Our SDK is a library.

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `cmds/CommandExecutor.java` | Stdin command loop (shutdown, status, etc.) | Library, not daemon |
| `cmds/DomainCmd.java` | Command enum | No CLI daemon |

Note: We have a separate `tools/cli/` crate (`tdx`) for command-line usage.

## NOT NEEDED -- Server-Side / Admin

These classes handle server-side user management or admin functions not relevant to a client SDK.

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `UserValidator.java` | Older auth class (v2 endpoint) | Using v3 `UserAuthenticator` |
| `UserDB.java` | Server-side user database | Client-side only |
| `User2.java` | Server-side user model | Auth response parsed into `AuthUser` |
| `session/SessionInfo.java` | Session POJO | Internal to auth flow |
| `session/SessionInfoV3.java` | v3 session POJO | Internal to auth flow (v3 session metadata) |
| `session/SessionRequest.java` | Session request POJO | Internal to auth flow |
| `session/SessionResponse.java` | Session response POJO | Internal to auth flow |
| `session/DisconnectRequest.java` | Disconnect request POJO | Internal to auth flow |
| `profiling/ProfilingTimer.java` | Performance profiling utility | Use criterion benchmarks instead |

## NOT NEEDED -- Config Infrastructure

| Java Class | Purpose | Why Not Needed |
|-----------|---------|----------------|
| `config/AbstractConfigurationManager.java` | Base config class | `DirectConfig` is simpler |
| `config/AbstractCredentialsConfigurationManager.java` | Credential config base | `Credentials` struct handles this |

## Generated Protobuf Classes (497 classes)

The `generated/` and `generated/v3grpc/` directories contain 497 protobuf-generated Java classes (Request/Response/OrBuilder types for every RPC). These are the Java equivalent of our `tonic::include_proto!()` output.

| Package | Class Count | Rust Equivalent |
|---------|------------|----------------|
| `generated/` (v2 proto) | ~250 | `proto` module via `tonic::include_proto!("endpoints")` |
| `generated/v3grpc/` (v3 proto) | ~247 | `proto_v3` module via `tonic::include_proto!("beta_endpoints")` |

All 60 v3 gRPC RPCs are covered. The v2 proto types exist for backward compatibility but are not used by the v3 terminal.
