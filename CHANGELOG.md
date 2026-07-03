# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **TypeScript projected Arrow-IPC is now drivable from a live historical call.** Every columnar historical method gains a `<method>WithColumns` variant returning `{ rows, presentColumns, symbol?, symbols? }`: the same rows as the plain method plus the columns the response's wire actually carried, the broadcast root `symbol` when the response has one constant across every row, and a per-row `symbols` array when a multi-symbol snapshot varies the symbol row to row. Feed `presentColumns` plus `symbol` / `symbols` straight to `<tick>ToArrowIpcProjected` for a terminal-exact columnar frame that omits the columns the wire omitted and attributes each row to its symbol, without hand-supplying a header list. The existing `Array<Tick>` methods are unchanged. This brings TypeScript to parity with Python's presence-carrying tick list and the C and C++ `_with_options` presence out-param.

### Fixed

- **Slicing or reversing a multi-symbol snapshot tick list keeps each row's own `symbol`.** A Python `<Tick>List` from a multi-symbol snapshot (`stock_snapshot_quote(["AAPL", "MSFT"])`) carries one `symbol` value per row, but a slice such as `lst[::-1]` or `lst[0:1]` rebuilt the sub-list against the full response-order symbols, so `.to_arrow()` / `.to_pandas()` / `.to_polars()` on the result either attributed each row to a neighbour's symbol or raised on the row-count mismatch. The slice now gathers the per-row symbols with the same index walk it selects rows by, so every kept row keeps its own underlying. The contract-universe listing (`option_list_contracts`), whose rows already own a `symbol` field, no longer also materializes a redundant per-row symbol copy that nothing reads, and a snapshot symbol that carries an interior NUL crossing the C Arrow-IPC boundary now substitutes an empty string for that one value instead of silently dropping the whole per-row column.
- **A slow Python `*_stream_async` handler no longer degrades other concurrent historical requests.** The async streaming path invoked the per-chunk handler on the shared async runtime that also drives every other in-flight historical call on the process, so a slow or blocking handler held one of those shared workers under the GIL and starved the network tasks serving unrelated requests. The per-chunk handler now runs off the shared workers, keeping the pool at capacity, so one caller's slow handler can no longer slow everyone else's calls. Delivery is unchanged: the handler still fires once per chunk, in order, and a handler exception still aborts the stream and propagates the same way. The synchronous `*_stream` path already ran the handler on the caller's own thread and was never affected.
- **A streaming subscription stays tracked and replayed on reconnect when a subscribe races an in-flight reconnect.** If the command queue was saturated (its drain paused while an auto-reconnect ran) a subscribe that had already registered its subscription rolled that registration back by value on the queue-full error, which could drop a subscription the reconnect had meanwhile re-sent onto the fresh session, so it went missing from the active-subscription snapshot and was never replayed on the next reconnect even though the server was still streaming it. The rollback now undoes only the registration the failed send still owns, so a subscription that another path has put back on the wire stays tracked and replayed.
- **A concurrent duplicate subscribe under command-queue backpressure stays tracked and replayed on reconnect.** When two subscribes for the same contract or full-stream ran concurrently and the first one's send failed on a saturated command queue, its rollback could remove the shared tracked entry even though the second subscribe's frame had already reached the wire, so the live subscription went missing from the active-subscription snapshot and was never replayed on the next reconnect. Each tracked entry is now reference-counted by the subscribes that share it, so one subscribe's rollback drops only its own reference and a subscription another subscribe put on the wire stays tracked and replayed. The active-subscription snapshot still reports each live subscription exactly once.
- **A mid-stream reconnect on a historical `*_stream` no longer duplicates the leading rows.** When a server-streaming call delivered some chunks and then dropped with a transient status (or `Unauthenticated`), the shared retry loop restarted the call from chunk zero even though the buffered collectors behind the Python / TypeScript / C / C++ bindings had already appended those chunks, producing a successful result with duplicated leading rows. Because the upstream has no resume cursor, a restart is now allowed only while no chunk has reached the handler; once delivery has begun a later transient is surfaced as an error instead of silently replaying the delivered prefix. A transient before the first chunk still retries.
- **Config invariants now run on every historical connect path.** `Client::connect`, `Client::connect_with_api_key`, and `ClientBuilder::config(...)` accepted a hand-built `DirectConfig` without running `DirectConfig::validate`, so an out-of-range `historical.max_message_size` could drive an unbounded decode allocation from a server-supplied size hint. Validation (and the historical flow-control window clamps) now runs at the single connect funnel every path routes through, mirroring the re-check already on the streaming side.
- **A server retry hint is clamped to the configured retry ceiling.** A `google.rpc.RetryInfo` cooldown from the server is now honored only up to the retry policy's `max_delay`. A hostile or misconfigured hint can no longer stretch the backoff sleep past the client's ceiling, which under a per-call `with_deadline(Duration::ZERO)` request would otherwise hold a request-semaphore permit for an unbounded sleep.
- **`historical.request_timeout_secs = 0` no longer documented as a deadline opt-out.** A configured `0` is floored to the 300s default at the request path so a live-but-silent server cannot hang a deadline-less request; the binding docs (Rust, C, C++, Python, TypeScript) previously still promised a `0` opt-out and now describe the real behavior. The per-call `with_deadline(Duration::ZERO)` remains the way to run a single request without a deadline.
- **A panicking FPSS I/O thread now surfaces to callback and binding consumers, not just pull consumers.** The blocking `for_each` / `poll_batch` drain returns the new `PollOutcome::Failed` on an I/O-thread unwind, and the unified Rust callback dispatcher plus the Python and TypeScript dispatchers flip the session to failed so `is_streaming()` reports the dead loop immediately. Previously only the `next_event` pull path surfaced the fault, so a callback stream read a graceful end-of-stream on an I/O-thread panic and kept reporting itself as streaming. A clean shutdown still returns `PollOutcome::Shutdown` and never a false failure.
- **A reconnect login handshake is interruptible by a client teardown.** The reconnect-path login wait now checks the shutdown flag between frames, so a server that keeps the handshake alive with pre-`METADATA` heartbeats (which reset the stall timeout) can no longer wedge a shutting-down I/O thread, which previously left a client drop joining the I/O thread forever. Mirrors the dial loop's between-host shutdown check.
- **A dispatcher-machinery panic wakes the columnar batch reader with an error instead of hanging it.** If the columnar dispatcher's iteration machinery or its linger flush panics, or the I/O thread unwinds, the reader's blocking pull now returns a dispatcher-failed error rather than parking forever waiting for a terminal marker the dead dispatcher will never publish.
- **Projected historical frames keep the trading `date`.** A response whose wire sends one `Timestamp` header split into a time-of-day field and `date` (every EOD and trade/quote/greeks history endpoint) no longer drops `date` from the Arrow / Polars frame, so rows spanning multiple days are distinguishable instead of collapsing to a near-constant time-of-day.
- **A multi-symbol snapshot now carries the per-row `symbol` so rows are attributable.** A snapshot requested for several symbols (`stock_snapshot_ohlc` / `trade` / `quote` / `market_value` over a symbol list) returns rows whose wire carries a per-row-varying `symbol` column. The projected Arrow / Polars frame now emits a real per-row `symbol` leading column carrying each row's own underlying, instead of dropping the column and leaving the rows unattributable. A single-symbol snapshot and the option / index responses keep broadcasting their one constant `symbol`. The per-row values are also reachable without a frame: TypeScript's `<method>WithColumns` adds a `symbols` array, and the C / C++ presence carrier conveys them to the projected Arrow-IPC export.
- **Flat-files block decode fails loud on two drifted-header shapes.** A header declaring zero columns over a non-empty DATA block, and a row carrying more fields than the header's column count, now return a typed decode error instead of silently emitting zero rows or clipping the surplus field. This matches the FPSS delta path's width guard and the existing mid-row truncation guard.
- **A panic inside a callback stream's scope closure flips the session to failed instead of leaving it falsely healthy.** A panic that escaped the batch drain (the drain machinery, or the interpreter-lock scope the Python client wraps each batch in, which can panic during interpreter finalization) was only logged, so the dispatcher thread exited quietly and the session kept reporting `is_streaming()` true behind a dead thread. The callback dispatcher now records the fault and re-raises, matching the columnar path, so a callback stream never appears alive after its dispatcher has died.
- **A concurrent stream restart is no longer clobbered by a superseded stop.** On the standalone Python and TypeScript streaming clients, a `stop_streaming` whose dispatcher join observed a panic recorded the failed state unconditionally after releasing its locks, so a fresh `start_streaming` that had already installed a live session in that window was falsely marked failed and its dispatcher handle orphaned. The failed state is now recorded only while the slot is still idle, matching the C ABI.
- **A same-port Prometheus exporter re-install returns Ok instead of failing the connect.** A second `Client::connect` with the same `metrics_port` bound the exporter's listener before it could detect the already-registered recorder, so it failed at the port bind and turned a benign re-install into a hard connect error. The installer now recognizes its own prior install and treats a re-install as a warn-and-continue, matching the documented contract; the first exporter keeps serving the port.
- **Python streaming teardown and superseded starts no longer risk an interpreter deadlock.** On the superseded-start and dispatcher-spawn-failure paths the standalone Python client dropped the streaming client while holding the GIL, so `Drop`'s inline I/O-thread join could deadlock against a custom reconnect callback re-acquiring the GIL. The drop now runs off the GIL, matching the stop and `Drop` paths.
- **FPSS subscribes and pings are not starved during a run of unknown frames.** An unknown wire frame skipped the command-channel drain, so a subscribe or ping issued during a burst of unknown frames waited until the next known frame. The loop now drains queued commands after every frame, known or not, while still not counting an unknown frame as liveness against the read deadline.
- **`Config.setReconnectCallback` no longer keeps a Node process alive.** The reconnect-decision callback registered on a `Config` held a strong reference to the libuv event loop, so a process that called `setReconnectCallback` never exited on its own even if no client ever connected. The callback is now a weak reference; the existing "callback unavailable stops reconnecting" path already handles a callback the runtime releases on shutdown.
- **TypeScript `RecordBatchStream` is referenceable as a value.** The class is exported at runtime but its type declaration was interface-only, so `stream instanceof RecordBatchStream` did not typecheck. It now carries a paired value declaration like `StreamingSession`.
- **The standalone TypeScript `StreamingClient` supports context-managed streaming, and its session teardown no longer leaks.** The standalone client had no `streaming(callback)` factory, and a `StreamingSession` wrapping one resolved its teardown surface through the unified client's `.stream` sub-view only, which is absent on the standalone client, so `await using session = await streamingClient.streaming(cb)` disposed nothing while the stream kept running. The standalone client now exposes the `streaming(callback)` factory and `using` / `await using` disposers, and the session resolves the lifecycle surface for both client kinds, so scope exit stops and drains the stream. Matches the Python standalone client.
- **The TypeScript streaming disposer docs no longer over-promise a hard callback barrier.** `stopStreaming() + awaitDrain()`, and the `await using` session and client disposers, wait on the streaming consumer thread, but the per-event callback is delivered to the Node main thread through a bounded threadsafe-function queue downstream of that thread, so a bounded backlog of already-queued events can still invoke the callback on later event-loop turns after the disposer resolves. The docs previously stated the consumer thread was guaranteed to have finished firing the callback before the closure was released, which holds on the Python and C++ bindings (the callback runs on the consumer thread) but not on the napi delivery path. The disposer docs now describe the real guarantee so a caller does not free callback-referenced state at scope exit assuming zero further invocations. A regression test drives the real threadsafe-function path offline to pin the behavior.

## [13.0.0-rc.13] - 2026-07-02

### Added

- The buffered historical `_with_options` entry points in the C and C++ bindings now surface the response's projected column set through an optional out-param (`ThetaDataDxColumnPresence*` in C, `ColumnPresence*` in C++), so a caller can drive the projected Arrow export from a real endpoint response instead of hand-supplying header names. `StringList` endpoints, which carry no column set, keep their existing signature. Closes #1082.
- `RecordBatchStream::ring_dropped()` reports events dropped upstream at the FPSS event ring when a stalled reader lets the ring fill. Under the default `Backpressure::Block`, where the reader queue never drops, this is the only loss signal a reader has, so a lossless-mode reader can now observe ring-side loss that `dropped()` (queue-side only) never counted (#1083).

### Changed

- The default streaming read timeout is now 10s (was 3s), matching the terminal's streaming socket read timeout so a client rides through the brief silence right after a full-market subscribe instead of tripping the deadline and forcing an unnecessary reconnect. Still configurable via `streaming.timeout_ms`.
- **Python `AsyncClient.close()` and `async with` teardown no longer block the event loop.** The stop-and-drain runs on a worker thread and is awaited, so a callback that round-trips through the loop can finish instead of forcing the teardown into its drain timeout. `AsyncClient.close()` is now awaitable: use `await client.close()` (#1086).
- **Python Arrow DataFrame conversion releases the GIL while building each batch**, so sibling Python threads keep running during a large `.to_arrow()` / `.to_pandas()` / `.to_polars()` on a returned tick list. The pyarrow handoff now uses the stable Arrow PyCapsule interface, falling back to the older path on pyarrow below 14 (#1090).

### Added

- **TypeScript projected Arrow-IPC exit for decoded history rows.** Alongside the full-schema `<tick>ToArrowIpc`, every columnar tick type gains `<tick>PresentColumns(headers)` and `<tick>ToArrowIpcProjected(rows, presentColumns, symbol?)`, so a caller can serialize only the columns the response carried, matching the projected frame Python's `<TickName>List.to_arrow()` and the C++ terminal produce (#1089).

### Fixed

- **Config validation rejects unusable historical settings up front.** `DirectConfig::validate` now band-checks `historical.connect_timeout_secs` (1..=300, where a `0` timed out every connect), range-checks `historical.max_message_size` (1 byte..=64 MiB, the same ceiling the `[grpc] max_message_size_mb` spelling already enforced, so the two spellings agree), and rejects a `0` `historical.port`. A retry policy with more than one attempt now requires a non-zero `initial_delay` / `initial_backoff`, so an enabled retry ladder cannot fire as an unthrottled burst. A `[streaming] hosts` entry with an empty host or a `0` port is rejected at load, and a non-http(s) `THETADATA_NEXUS_URL` is skipped with a warning instead of silently re-pointing auth at an unreachable endpoint.
- **A cross-channel environment selector surfaces as a typed error, not a panic.** Resolving the config from the environment with `THETADATA_HISTORICAL_TYPE=DEV` (a value the historical channel does not support) now returns the `expected PROD or STAGE` error to the caller through the new `DirectConfig::try_production`, instead of unwinding inside the connect path.
- **Historical `*_stream` endpoints now honor the request-timeout backstop and the shared retry loop.** `stock_history_trade_stream`, `stock_history_quote_stream`, `option_history_trade_stream`, and `option_history_quote_stream` mapped an unset deadline to no bound at all, so a call against a live-but-silent server could hang forever holding a request-semaphore permit and starve every later historical request on that client. They also hand-rolled a retry loop that dropped a session refresh on the final attempt and ignored the wall-clock envelope. All four now resolve an unset deadline to the configured `request_timeout_secs` (an explicit `Duration::ZERO` still opts out) and route through the same `run_streaming_retry_loop` the buffered builders use (#1081).
- **Empty stream payloads cross the C ABI as null.** An empty Ping or UnknownFrame payload now surfaces a null pointer, matching the header's null-when-empty contract, instead of a non-null dangling pointer.
- **C++ `<tick>_present_columns` reports failures.** The wrapper now clears and checks the error slot and throws on failure, instead of silently returning an empty presence.
- **Undersized explicit tail padding on five C tick structs.** The explicit `_tail_padding` array on `ThetaDataDxTradeTick`, `ThetaDataDxTradeQuoteTick`, `ThetaDataDxPriceTick`, `ThetaDataDxOpenInterestTick`, and `ThetaDataDxMarketValueTick` now fills to the full struct size (the ABI was already correct via the compiler's implicit padding; this makes the explicit-padding contract accurate).
- **Streaming `Backpressure::Block` is no longer described as lossless end to end.** Block mode bounds loss at the reader queue, but the upstream event ring still drops on overflow by design (blocking the I/O thread would stall heartbeats and drop the vendor session). The docs now state the true bound and the new `ring_dropped()` surfaces the ring-side count (#1083).
- **A panicking FPSS I/O thread now surfaces as a failure instead of a clean end-of-stream.** If the I/O loop unwinds, the client is marked unauthenticated and the blocking drain returns a dispatcher-failed error rather than a graceful `Ok(None)` (#1083).
- **A rejected subscription can no longer linger and replay on reconnect.** A subscribe now registers its tracked entry and request correlation before the wire send, rolling both back if the send fails, so a fast server rejection is always reconciled instead of racing the registration (#1083).
- **Client teardown is prompt while the ping thread or a reconnect is mid-wait.** The ping heartbeat, the reconnect dial loop, and the unknown-frame skip now observe the shutdown flag between steps, so dropping a client no longer blocks for up to a full ping interval, dial, or frame skip (#1083).
- **Option contracts reject a zero or negative strike** at the builder instead of emitting a subscribe for an impossible strike (#1083).
- **A real metrics exporter bind failure is now surfaced.** A `metrics_port` bind error (for example the port already being in use) returns a configuration error instead of being masked as a benign "another recorder is active" (#1090).
- **Windows server archive attaches to the release.** The Windows `thetadatadx-server` archive was built, but the upload step received a path the artifact action could not resolve on Windows, so the GitHub Release carried no Windows binary. The archive is now referenced by its native workspace path and attaches correctly.
- **The `batches()` doc example now builds the reader before subscribing.** Building the reader starts the session, so subscribing beforehand errored at runtime; the example ordering is corrected (#1090).
- **Python `close()` can no longer deadlock against an in-flight streaming start.** `start_streaming` now releases its callback lock before the blocking connect, so a concurrent `close()` / `__exit__` / `__aexit__` on the client cannot wedge the interpreter.
- **Python standalone `StreamingClient.start_streaming` can no longer wedge the interpreter.** It now releases its callback lock before the blocking connect and registers the callback with an ownership check, so a concurrent `stop_streaming` / `reconnect` / second `start_streaming` cannot deadlock the interpreter or clobber a newer registration (#1080).
- **Python `session.subscribe(...)` after `close()` raises the uniform closed error.** The context-managed session's attribute proxy now surfaces the "client is closed" error on a closed unified client instead of masking it as an `AttributeError`.
- **Python `close()` called from inside a streaming callback returns promptly.** Closing from the dispatcher thread now skips the drain wait it could never observe, instead of burning the full drain timeout and emitting a spurious warning.
- **The credential exchange no longer follows redirects.** The Nexus auth client is set to not follow redirects, so a 3xx from the auth endpoint surfaces as a server error instead of the credential request body being replayed to the redirect target. Nexus never redirects auth.
- **`SessionSnapshot` no longer prints its session UUID.** The `Debug` output now redacts the UUID with the same `***` marker the auth response uses, so the bearer token cannot land in panic output or `Debug`-formatted logs.
- **A concurrent burst of failed session refreshes now issues a single Nexus round-trip.** When several requests observe an expired session at once and the re-authentication fails, callers queued behind the first coalesce onto that outcome instead of each re-authenticating in series.
- **Flat-files reject a truncated FIT block instead of emitting a garbage final row.** A per-contract block that ends before its terminating marker now raises a typed decode error, matching the streaming delta path, rather than pushing a partially decoded final row (#1084).
- **End-of-day responses with fully drifted headers now error instead of returning zero-filled rows.** A rows-present EOD response whose headers fail to resolve raises a missing-required-header error like every other parser; an empty response still returns no rows (#1084).
- **Flat-file downloads no longer destroy a prior good file or leave a partial under the final name.** Both the raw download and the decoded output write to a sibling temp file and rename onto the final path only after the write finishes, and the temp is removed on failure (#1085).
- **Local disk faults during a flat-file download surface immediately.** A full, read-only, or permission-denied output filesystem is now terminal rather than triggering a full re-download on the retry ladder.
- **FIT integer fields saturate on overflow.** A digit run longer than an i32 can hold now saturates to the signed bound instead of silently dropping the surplus digits and emitting a plausible wrong value.
- **Price comparison agrees with display for absent prices.** A price carrying type 0 now compares as zero, consistent with its `0.0` rendering and `to_f64`.
- **Python `close()` after `stop_streaming()` now drains the consumer.** The drain no longer hinges on a streaming-live check that flips false the instant the stream stops, so `close()` waits for the last callback to finish firing instead of returning while it is still running (#1086).
- **Python `close()` during a concurrent streaming start no longer stalls the interpreter.** The teardown reads dispatcher state with the GIL released, so it never holds a binding lock across a blocking connect (#1086).
- **Python `is_streaming()` no longer self-deadlocks a re-entrant log handler.** The failure reason is copied out and both binding locks are released before the debug log is emitted (#1086).
- **TypeScript standalone `StreamingClient` no longer leaks a live session under concurrent start/stop.** A `startStreaming` superseded by a concurrent `stopStreaming` plus a newer `startStreaming` no longer publishes its connection over the live one; the superseded start shuts its own connection down and returns an error, and every failure path clears the callback slot only when the start still owns it (#1080).
- **TypeScript `client.stream.startStreaming` no longer wipes a newer session's callback on a superseded handshake failure.** The callback slot is cleared on failure only when this start still owns it, so a concurrent stop plus restart keeps its registration and `reconnect()` still finds the callback (#1080).
- **Disposing or forwarding through a closed TypeScript client is a no-op again.** The context-managed streaming session's `asyncDispose` and attribute proxy no longer throw when the underlying client is already closed, matching the base client's disposer.

## [13.0.0-rc.12] - 2026-07-01

### Fixed

- **C++ standalone `StreamingClient` teardown.** Destroying (or move-assigning) a `StreamingClient` with no callback installed no longer spins a CPU core to the drain quiescence cap and then leaks the handle; the deferred reclaim is skipped when no callback is installed and the reclaim loop paces itself (#1064).
- **Wrong-width streaming rows no longer trap a contract.** A complete-but-narrow first row (e.g. a 7-field trade) is rejected before it seeds the per-contract field-width cache, so a later correct row decodes cleanly instead of the contract being dropped until the next session reset. Applies uniformly to every stream tick shape; dropped rows are logged and counted (#1047).
- **Windows server archive.** The `thetadatadx-server` Windows build now zips to the expected path, so the release attaches the Windows binary (previously the archive step wrote to a mismatched path and the upload found nothing).

### Changed

- Streaming login is bounded solely by the socket read timeout, consistent with the rest of the read path (the separate wall-clock handshake deadline is removed) (#1064).

## [13.0.0-rc.11] - 2026-07-01

### Removed

- The configurable ring wait-strategy and its tuning knobs (`wait_strategy`, `wait_spin_iters`, `wait_yield_iters`, `wait_park_us`) across every binding. The streaming consumer now runs a single fixed low-latency wait (#1057).
- The `data_watchdog_ms` reconnect timer, which duplicated the read timeout under the default configuration (#1055).
- The unused `flatfile_value_arrow_type` helper and several unused `DirectConfig` builder methods (#1058).

### Changed

- Streaming Trade and OHLCVC events carry only the fields the wire provides; the previously-synthesized extended-trade fields are removed (#1042).
- Internal streaming simplifications with no public API or wire-behavior change: the FPSS frame reader lost its mid-frame drain-yield and per-frame deadline handling, and the delta decoder lost several unused runtime guards (#1055, #1059).

### Fixed

- **C++ callback use-after-free:** a retired callback is now dropped only after the consumer thread confirms quiescence; if the quiescence cap elapses first, the node is intentionally leaked (a bounded one-time leak) rather than destroyed while an invocation may still be in flight (#1056).

## [13.0.0-rc.10] - 2026-06-30

### Added

- An internal-only `__internal` cargo feature re-exports the FPSS decode, wire, framing, and login surface as `fpss::internals`, for a downstream consumer that drives the decode into its own ingest loop. It is not part of the public API and carries no SemVer guarantee (#1038, #1039).

### Fixed

- **Post-reconnect tick desync:** the FPSS delta-decode state is now reset on reconnect, so the first ticks after a reconnect decode against a fresh baseline instead of a stale one (#1036).

### Documentation

- Every API reference page now renders an interactive request builder in place of the static language tabs. Pick a language, toggle the client and auth method, edit any parameter, and the example code and sample response regenerate live (#1037).
- Scoped the full-trade event-handling note to the full-trade example in the README (#1034).

## [13.0.0-rc.9] - 2026-06-29

### Added

- **MCP server on npm:** the `thetadatadx-mcp` server is available on npm and runs with `npx -y thetadatadx-mcp`, the one-line config every MCP client expects, with no Rust toolchain required. `cargo install` stays for Rust users.

### Documentation

- The full-trade streaming pages now document the complete event delivery: for each traded contract the stream delivers a quote (BBO for stocks, NBBO for options), an OHLC bar, and the trade itself, and the quote terminology is now correct per asset class (#1029).

## [13.0.0-rc.8] - 2026-06-29

### Added

- **MCP server on npm:** the `thetadatadx-mcp` server is available on npm and runs with `npx -y thetadatadx-mcp`, the one-line config every MCP client expects, with no Rust toolchain required. `cargo install` stays for Rust users.

### Documentation

- The full-trade streaming pages now document the complete event delivery: for each traded contract the stream delivers a quote (BBO for stocks, NBBO for options), an OHLC bar, and the trade itself, and the quote terminology is now correct per asset class (#1029).

## [13.0.0-rc.7] - 2026-06-29

### Added

- **MCP server on npm:** the MCP server is now distributed as an npm package and runs with `npx -y thetadatadx-mcp`, the one-line config every MCP client expects, with no Rust toolchain required (#1023). `cargo install` stays for Rust users.

### Documentation

- The streaming reference is corrected: per-security-type market-value pages are added for stocks and options, the streamed-trade field set is narrowed to what the feed actually delivers (with the extended fields marked as extended-format-only), a WebSocket-frame note clarifies the payload subset carried on the wire, and an open-interest availability banner records that open interest has no streaming frame while remaining a valid SDK event (#1018).
- The READMEs drop the removed client-side Greeks calculator; the option Greeks are served straight from the option endpoints (#1024).
- The npx MCP config is surfaced in the README install block, and the public roadmap is published (#1025, #1021).

## [13.0.0-rc.6] - 2026-06-29

### Breaking changes

- **Server:** the bundled HTTP server now speaks the v3 REST and WebSocket contract (#959). REST responses drop the older `{header, response}` envelope for the v3 `{response}` body, fold the separate date and ms-of-day columns into one ISO local-datetime per endpoint, spell the option right as `CALL` / `PUT`, group option rows under their `{expiration, strike, right}` contract, default an absent `format` to CSV with CRLF framing and the v3 column order, and return errors as a plain-text status. The WebSocket surface emits and parses the v3 stream shape. The server also now binds `0.0.0.0` by default.
- **CLI tool removed:** the bundled `thetadatadx` command-line binary is removed (#1011). The SDKs and the bundled HTTP server are unaffected.
- **Client-side Greeks calculator removed:** the SDK's local Black-Scholes Greeks calculator is removed across every binding (#1016): Rust `all_greeks` / `implied_volatility` and the per-Greek primitives, the TypeScript `allGreeks` / `impliedVolatility`, and the FFI, C++, and MCP equivalents. ThetaData's own served Greeks are unchanged and still available: the eod, trade, and snapshot greeks endpoints and their tick types are untouched. Only the client-computed calculator is gone. The option-right parser moves out of the greeks namespace to `thetadatadx::right`.

### Removed

- The client-side Black-Scholes Greeks calculator (#1016) and the bundled CLI tool (#1011), both listed under Breaking changes above.
- A number of unused public API items with no callers, across the core and the FFI surface (#964, #965, #971, #975, #994).

### Fixed

- **C++:** an async historical-view use-after-free is fixed (#997, #1001, #1002). Async historical methods are ref-qualified so a call on a dangling temporary view is a compile error, the underlying handle is shared into the async future rather than captured by reference, and the streaming callback state is co-owned by the view so a future can no longer outlive the state it reads.
- **Streaming teardown:** two teardown deadlocks are fixed. Shutting down, freeing, or reconnecting a standalone streaming client no longer hangs when the user callback, draining events, re-enters a status read on the same client; teardown now releases the lock that read needs before waiting for the drain (#979, #980).
- **TypeScript:** numeric inputs are validated at the boundary (#998, #999, #1000). The config knobs, the drain timeouts, and the worker, CPU, and iteration setters reject a non-finite, negative, fractional, or out-of-range value as `InvalidParameterError` instead of silently coercing it; the Arrow reconstruct path rejects a bigint that would not narrow losslessly into an i64 column; and the streaming dispatcher publishes a failed state when its event loop dies, so `isStreaming()` and `isAuthenticated()` report the dead loop even if teardown is never called.
- **Server:** the WebSocket subscribe and contract envelope carry the option strike in dollars, not thousandths, matching the unit the SDK, REST, and docs already use (#981).
- **Server:** a malformed flat-file path segment now returns the canonical JSON error envelope its sibling routes return, rather than a default plain-text rejection (#988).

### Documentation

- The expiration and right chain wildcards are documented from the endpoint capability set (#960, #963).
- The streaming guide uses the synchronous pull (`next_blocking`) for its blocking example (#995).
- The documentation site landing page is redesigned around the SDK surfaces and the developer experience (#1015).

### Security

- A routine dependency refresh across the Rust and npm trees (#996), with zero advisories outstanding. This is maintenance, not a fix for any reported vulnerability.

## [13.0.0-rc.5] - 2026-06-24

### Breaking changes

- **Configuration:** the two server channels are selected independently and named after the data they serve. The historical channel (production or staging) and the streaming channel (production or dev) each have their own selector: `HistoricalEnvironment` / `StreamingEnvironment` in Rust, with `with_historical_environment` / `with_streaming_environment` setters and `historical_environment` / `streaming_environment` getters across every binding. The single combined environment selector is removed.
- **Environment variables:** `THETADATA_MDDS_TYPE` is renamed to `THETADATA_HISTORICAL_TYPE` (`PROD` or `STAGE`) and `THETADATA_FPSS_TYPE` to `THETADATA_STREAMING_TYPE` (`PROD` or `DEV`). An unrecognized or cross-channel value (for example `THETADATA_HISTORICAL_TYPE=DEV`) is now a hard error naming the offending key and the valid set rather than a silent fallback; a malformed host or port override is still skipped with a warning.
- **Server CLI:** `--mdds-region` is renamed to `--historical-region` (`production` or `stage`) and `--fpss-region` to `--streaming-region` (`production` or `dev`). `--streaming-region` no longer accepts `stage`; there is no streaming staging cluster.
- **Bindings:** the inline-construction selectors are renamed to match — Python `historical_type` / `streaming_type` and TypeScript `historicalType` / `streamingType`.
- **Event and error taxonomy:** the C ABI streaming event-kind constants are renamed from the `THETADATADX_FPSS_*` prefix to `THETADATADX_STREAM_*` (the `_TRADE` / `_QUOTE` / `_OHLCVC` / ... suffixes are unchanged), and the umbrella error variant `Error::Fpss` becomes `Error::Stream` with the `Display` text `stream error (...)`. Python and TypeScript carry the event kind as a lowercase string union and are unaffected.
- **Server routes:** the system status routes are renamed to the channel they report — `GET /v3/system/mdds/status` becomes `GET /v3/system/historical/status` and `GET /v3/system/fpss/status` becomes `GET /v3/system/streaming/status` (operation ids `systemHistoricalStatus` / `systemStreamingStatus`).

### Added

- **Streaming:** a pull-based Arrow `RecordBatch` reader, `batches()`, as a columnar delivery mode alongside the per-event callback (#950). Instead of one callback per event, the reader coalesces decoded events into Arrow record batches you pull on your own schedule: Rust exposes both a `futures::Stream` and a `.blocking()` iterator; Python is a synchronous and an async iterable and a context manager, handing each batch to pyarrow zero-copy over the Arrow C-Data interface; TypeScript is an `AsyncIterable<RecordBatch>` decoded from Arrow IPC; C++ returns a native `arrow::RecordBatchReader`. Batching is tunable by `batch_size` and a `linger` flush interval, with a backpressure choice of `Block` (lossless, stalls the producer when the consumer falls behind) or `DropOldest` (bounded, evicts the oldest batch and reports the count through a `dropped()` counter). Every batch carries one fixed unified streaming schema across all bindings, and dropping or closing the reader tears the streaming session down. The per-event callback delivery mode is unchanged.

### Fixed

- **Authentication:** api-key and email/password resolution is unified across the server, the CLI, and the MCP tool under one precedence: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` with `THETADATA_PASSWORD`, then the credentials file. The CLI and MCP previously had no api-key path, and the MCP read non-canonical variable names. Authentication errors now carry only the HTTP status and never the upstream response body, on both the success-parse and non-success paths, so a gateway that reflects the submitted request can never surface a credential through the error chain.
- **Configuration:** host and environment selection is provenance-driven: an explicit host override (`THETADATA_HISTORICAL_HOST`, `THETADATA_STREAMING_HOST` / `_PORT`, a config-file host list, or a `.env` entry) is tracked as a typed override and survives `stage()`, `dev()`, and `with_environment()`, with the most recent override winning and a config-file host list winning outright. `dev` is now a first-class environment with its own cluster and a production-equivalent auth marker, so an override on a dev config keeps the dev cluster. The `.env` reader covers every selection key with no split between clusters, and blank or quoted-whitespace values are ignored. A streaming configuration that resolves to no usable host is rejected rather than dialing an empty target.
- **Streaming:** the reconnect budget resets only after a stable connected window, so a flapping connection cannot reconnect indefinitely. A per-frame wall-clock deadline is persisted across resumed partial reads, so a peer that trickles bytes cannot hold a half-read frame open forever. Reconnect marks the session live only after replay succeeds, and the consumer-thread identity and CPU pin track the true drainer.
- **Historical transport:** the connection carries a connect timeout, so a lazy reconnect dial fails fast and is classified retryable, a refused stream is retried, and list endpoints honor the request timeout and expose a deadline opt-out (a zero or disabled deadline) so a live-but-silent stream can no longer hang a list call indefinitely.
- **Bindings:** C++ client view accessors are lvalue-only, so a view bound to a temporary client is a compile error, and a fresh callback node is installed on replace and released only on confirmed quiescence so callback state stays valid across a client move. The gRPC, MCP, and CLI error paths are char-boundary safe and no longer panic when upstream text is non-ASCII.
- **Tools:** the published OpenAPI document matches the served `/v3` routes, drops a phantom document-wide auth scheme, and carries the correct server URL and version, and the flat-file surfaces expose only the served dataset matrix.
- **Streaming teardown (FFI / C ABI):** shutting down, freeing, or reconnecting a standalone streaming client could deadlock when the user callback, draining events, re-entered a status read on the same client; teardown now completes the dispatcher join without holding the lock the status read needs, so it can no longer hang.
- **Streaming teardown (TypeScript):** stopping the stream could hang when a slow callback had saturated the bounded delivery queue; teardown now wakes the blocked consumer before joining, so shutdown completes promptly. Reconnect, which reuses the same callback, is unaffected.
- **Configuration:** selecting a channel environment after setting a custom configuration no longer resets that configuration to defaults when a later validation check rejects an out-of-range value; the custom hosts and tuning survive the rejected call.
- **CLI:** `--format json` emits `null` for a non-finite number (NaN or infinity) instead of a fabricated `0`, matching the JSON the other frontends produce.
- **Configuration:** loading a config file no longer panics, and no longer picks up ambient environment overrides, when an unrecognized environment selector is present in the environment.
- **CLI:** an empty or whitespace-only `--api-key` is treated as unset and falls through to the next credential source instead of being used as a key; endpoint arguments are validated before the network connection is opened, so a malformed argument fails fast; and the TypeScript slow-callback-threshold setter rejects a value that does not fit losslessly rather than silently wrapping it.

### Security

- The TypeScript docs-site toolchain bumps esbuild to clear GHSA-gv7w-rqvm-qjhr. The Rust dependency tree upgrades memmap2 past RUSTSEC-2026-0186, and aligns h2 and webpki-roots and bumps rustls-platform-verifier across the tracked lockfiles. None of these reached a shipped SDK API; they are dependency and toolchain updates.

### Internal

- CI workflows are tiered into a fast lane and a heavy lane. The fast lane runs on every pull request, on every push to `main`, and in the merge queue: formatting, the workspace clippy gate, the core library unit tests, and the cheap script gates (C-ABI completeness, cross-binding parity, wire-schema drift, SAFETY-comment hygiene, public-surface leak, documented-config defaults, source-docs framing, docs consistency, version sync, and the dependency advisory gate). The cross-platform builds (the macOS, Windows, and MSRV lint matrix, the FFI builds, the Python wheels, the TypeScript native addon matrix, the full and feature-gated test runs, rustdoc, semver, and the benchmark and performance gates) run on push to `main`, on a nightly schedule, on release tags, in the merge queue, and on a pull request only when relevant paths change or a `full-ci` label is present. No check is removed; every check still runs at least nightly and at release.
- The binding-parity and docs gates fail closed. The parity gate resolves the TypeScript entry from the package manifest and requires the runtime export rather than the type declaration alone, following only genuine re-export forms, and the docs gate derives the served route set and the flat-file and MCP tool inventories from source so a contract or tool that drifts from the code can no longer pass.

## [13.0.0-rc.4] - 2026-06-19

### Added

- API-key authentication as an alternative to email and password. A key can be supplied inline, read from the `THETADATA_API_KEY` environment variable, or loaded from a `.env` file, and it authenticates both historical and streaming access. It is available across the Rust, Python, TypeScript, and C++ SDKs and the bundled server (`--api-key` flag or the `THETADATA_API_KEY` environment variable). New `Credentials` factories cover the sourcing options (per-binding casing): `api_key` / `from_api_key` for an inline key, `api_key_with_email` / `from_api_key_with_email` to pair a key with an email, `from_env_or_file` to take the key from the environment or fall back to a file, and `from_dotenv` to read it from a `.env` file. Email and password authentication is unchanged (creds file, inline, or a custom file path).

## [13.0.0-rc.3] - 2026-06-18

### Removed

- The discontinued Go SDK's lingering references are removed from the public surface. The OpenAPI reference document no longer lists or renders Go code samples, and the crate metadata and comments no longer mention Go. No other binding is affected and there are no API changes since rc.2.

### Internal

- The repository's automation scripts are reorganized into `ci`, `release`, and `dev` groups behind a single gate dispatcher, with the dormant Go codegen scaffolding removed. This is repository tooling only and does not change any published package.

## [13.0.0-rc.2] - 2026-06-18

### Added

- Rust — `Client::flat_files()` returns a `FlatFiles` view that reaches flat files the same way the Python, TypeScript, and C++ bindings do (`client.flat_files().option_trade_quote(date)`), closing the last cross-binding access-shape asymmetry where Rust reached flat files only through standalone free functions. The view exposes one method per served dataset (`option_trade_quote`, `option_open_interest`, `option_eod`, `stock_trade_quote`, `stock_eod`), a generic `request(sec_type, req_type, date)` dispatcher, and a write-to-disk `to_path(...)` entry. The standalone `thetadatadx::flatfile_request*` free functions remain available as the lower-level API.

### Changed

- Rust — the public streaming error type is renamed `FpssError` to `StreamError` (and its classification enum `FpssErrorKind` to `StreamErrorKind`), removing the last vendor-protocol acronym from the public Rust error surface and matching the `StreamError` name the Python, TypeScript, and C++ bindings already use. This is a breaking rename with no compatibility alias, which is acceptable within this pre-release; the type is reached at its canonical path `thetadatadx::streaming::StreamError` and `StreamErrorKind` stays at the crate root.
- `thetadatadx::streaming` is now the canonical module for the streaming surface. It re-exports everything a streaming client needs (`StreamingClient`, `StreamingClientBuilder`, the `StreamEvent` / `StreamData` / `StreamControl` events, `PollOutcome`, `StreamError`, and the `Contract` / `Subscription` / `OptionLeg` subscription-building types), so a Rust example imports them all from one place (`use thetadatadx::streaming::{StreamingClient, StreamEvent, Contract};`). The older `thetadatadx::fpss` path is now a hidden compatibility alias: existing `use thetadatadx::fpss::...` imports keep compiling unchanged, the path just no longer shows up in the rendered documentation. Prefer `thetadatadx::streaming` in new code.
- `thetadatadx::historical` is now the canonical module for the standalone historical client (`HistoricalClient`), mirroring `thetadatadx::streaming`. `HistoricalClient` and `SubscriptionTier` are now part of the default public Rust surface (previously feature-gated), so a Rust example builds a historical-only client from one place (`use thetadatadx::historical::HistoricalClient;`) and the type returned by `Client::historical()` is nameable. Both `thetadatadx::HistoricalClient` and `thetadatadx::historical::HistoricalClient` resolve. `thetadatadx::mdds` remains a hidden internal alias.

## [13.0.0-rc.1] - 2026-06-17

### Breaking changes

- C++ — the public header and library are renamed from `thetadx` to `thetadatadx` (`#include "thetadatadx.hpp"`, the C ABI header `thetadatadx.h`, and the `thetadatadx` CMake target) so the C++ surface matches the `thetadatadx` namespace, the C-ABI symbol prefix, and the package name across every other binding. Update the include and link names; there is no compatibility alias.
- The flat-file surface is restricted to the datasets the distribution actually serves — option `trade_quote` / `open_interest` / `eod` and stock `trade_quote` / `eod`. The `option_quote`, `option_trade`, `option_ohlc`, `stock_quote`, and `stock_trade` convenience methods are removed from every binding (those request types are served by the historical endpoints, not as flat files), and the generic flat-file request path now rejects an unsupported `(security, request)` pair with a typed invalid-parameter error before any network round-trip instead of surfacing a server `INVALID_PARAMS` rejection.
- Clients are named by their role. The unified client is `Client` (Rust `thetadatadx::Client`, Python `Client` with the async companion `AsyncClient`, TypeScript `Client`, C++ `thetadatadx::Client`); the historical-only client is `HistoricalClient` and the streaming-only client is `StreamingClient` on every binding. The prior long client name and the protocol-keyed `MddsClient` / `FpssClient` names are gone with no aliases. The streaming callback payload types are `StreamEvent`, `StreamData`, and `StreamControl` (the prior `FpssEvent` family is renamed in lockstep).
- The `tdx` token is gone from the entire client-facing surface. C-ABI symbols carry the `thetadatadx_` prefix (was `tdx_`), C structs and handles carry the `ThetaDataDx` prefix (was `Tdx`, e.g. `ThetaDataDxConfig`), the C++ namespace is `thetadatadx` (was `tdx`), and the CLI binary is `thetadatadx` (was `tdx`). The credentials-from-memory constructor is `thetadatadx_credentials_from_email` / C++ `Credentials::from_email` with the `(email, password)` order unchanged. Recompilation and call-site renames are required; no short-prefix aliases remain.
- C ABI — the public preprocessor constants are renamed from the `TDX_` prefix to `THETADATADX_` (`THETADATADX_ERR_*`, `THETADATADX_SUB_SCOPE_*` / `THETADATADX_SUB_KIND_*`, `THETADATADX_FPSS_*`, `THETADATADX_CALENDAR_STATUS_*`, `THETADATADX_RETRY_AFTER_NONE`, `THETADATADX_ALIGN64_*`) so the C header's constants match the `thetadatadx_` function prefix and the brand across every binding; the integer values are unchanged. Recompile and update any `#if`/`switch` on the old names; there is no alias.
- Config sections are role-named to match the clients: `config.historical` and `config.streaming` (Rust `HistoricalConfig` / `StreamingConfig`; the `DirectConfig` fields were `mdds` / `fpss`). The environment variables follow — `THETADATA_HISTORICAL_HOST` / `_PORT` and `THETADATA_STREAMING_HOST` / `_PORT` (were `THETADATA_MDDS_*` / `THETADATA_FPSS_*`) — and the C-ABI config getters and setters rekey accordingly (`thetadatadx_config_get_historical_host`, `thetadatadx_config_set_streaming_ring_size`, the full role-keyed family). The internal protocol modules keep their wire names; only the client-facing surface is role-named.
- Data surfaces — the unified client exposes historical data through a `historical` sub-namespace and real-time streaming through a `stream` sub-namespace in every binding: Python getters (`client.historical.stock_history_eod(...)`, `client.stream.subscribe(...)`), TypeScript getters (`client.historical.stockHistoryEOD(...)`, `client.stream.startStreaming(...)`), and C++ view accessors (`client.historical().stock_history_eod(...)`, `client.stream().set_callback(...)`). The Rust core routes through `client.historical()` and `client.stream()`, and `StreamSurface` is exported from the crate root. Flat-file requests stay on the client directly (`client.flat_files` / `flat_files()`). The standalone `HistoricalClient` and `StreamingClient` keep their flat surfaces.
- Streaming diagnostics live under the stream sub-namespace on the unified client: `panic_count()` and `active_full_subscriptions()` move off `Client` onto the stream view (`client.stream.panic_count()` / `client.stream.active_full_subscriptions()` in Python and TypeScript, `client.stream().panic_count()` in C++). The standalone `StreamingClient` keeps these on its flat surface.
- `Contract.option(...)` takes the option leg as one named form instead of three transposable positional strings: a typed `OptionLeg { expiration, strike, right }` in Rust (added to the prelude), a named-key object in TypeScript (`Contract.option("SPY", { expiration: "20260620", strike: "550", right: "C" })` exporting an `OptionLeg` interface), and C++ designated initializers (`Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"})`). The Python builder was already keyword-only (`Contract.option(symbol, *, expiration, strike, right)`) and is unchanged.
- Python — the `Contract.sec_type` builder getter returns a string (`"STOCK"` / `"OPTION"` / `"INDEX"` / `"RATE"`) instead of a `SecType` object, matching the streaming `ContractRef.sec_type`, both TypeScript surfaces, and the builder's other scalar getters. Full-stream subscriptions are still built from the `SecType` class (`SecType.OPTION.full_trades()`).
- TypeScript — the client connect factories take a `Credentials` value and an optional `Config`: `Client.connect(creds, config?)` and `Client.connectFromFile(path, config?)`, with a new `Credentials` class (`new Credentials(email, password)`, `Credentials.fromFile(path)`, redacted `toString`). The prior `(email, password)` connect signature and the separate `connectWithConfig` / `connectFromFileWithConfig` factories are removed in favour of the optional trailing `Config`.
- TypeScript — every network entry point is asynchronous so a round-trip never blocks the Node event loop. The connect factories `Client.connect` / `Client.connectFromFile` / `HistoricalClient.connect` / `HistoricalClient.connectFromFile` return `Promise<Client>` / `Promise<HistoricalClient>`; the streaming lifecycle `startStreaming` / `reconnect` (on both the unified stream view and the standalone `StreamingClient`) return `Promise<void>`; and the flat-file methods (`optionTradeQuote` / `optionOpenInterest` / `optionEod` / `stockTradeQuote` / `stockEod` / `request` and `flatFileToPath`) return `Promise`. Call sites add `await`.
- The error vocabulary is typed and unified across bindings. A `ConfigError` leaf (Python / TypeScript / C++ subclasses of `ThetaDataError`, pinned to the reserved C-ABI `THETADATADX_ERR_CONFIG` code) carries environmental faults that previously fell through to the root error; `InvalidParameterError` is the typed leaf for invalid configuration, sequence, and flat-file inputs (new C-ABI code `THETADATADX_ERR_INVALID_PARAMETER = 13`). Streaming faults (`FlatFilesUnavailable`, `PartialReconnect`) route to `StreamError` in Python and TypeScript to match C++ and the C ABI. Python adds `NotFoundError`, `DeadlineExceededError`, and `UnavailableError` leaves (with `NoDataFoundError` / `TimeoutError` retained as aliases). Rate-limit errors expose the decoded server back-off: `RateLimitError.retry_after` (Python, seconds), `retryAfter` (TypeScript), `retry_after()` (C++), and the C-ABI `thetadatadx_last_error_retry_after_ms()` accessor (sentinel `THETADATADX_RETRY_AFTER_NONE = -1`).
- Several validators reject bad input with a typed error instead of silently coercing it: the C-ABI `thetadatadx_config_set_reconnect_policy` returns `int32_t` and rejects a policy outside `{0, 1}` with `THETADATADX_ERR_INVALID_PARAMETER` (was `void`, coerced to `Auto`); the trade-sequence converters return a status with an out-pointer and reject out-of-wire-range inputs; TypeScript and C++ reject non-finite, negative, fractional, or out-of-range request timeouts (`timeoutMs` / `with_deadline`) as `InvalidParameterError` rather than coercing them; the Arrow-IPC tick reconstruction rejects out-of-vocabulary `status` / `right` values instead of coercing them.
- Tick rows — the option `right` field is the logical character on every typed surface: `char` in Rust (`'C'` / `'P'`; `'\0'` when contract identity is absent), one-character string in Python and TypeScript, `uint32_t` Unicode scalar value in the C header (same 4-byte slot the previous ASCII integer occupied — cast to `char` for display), with both wire encodings (`Number` ASCII code, `Text` `"CALL"`/`"C"`/`"PUT"`/`"P"`) decoded at the boundary. The raw ASCII integer no longer appears anywhere, including `OptionContract` rows and the CLI table renderer.
- Tick rows — `strike` means dollars under that one name on every surface. The streaming contract payload (`ContractRef` in Python, `Contract` in TypeScript) types `strike` as dollars (`float`) and exposes the exact wire integer under the unit-named `strike_thousandths` (replacing the old `strike_dollars` twin); the fluent builders accept the strike in dollars as a number or string (`550`, `550.0`, `"550"`) and read the same dollar value back; the C `ThetaDataDxContract.strike` field is `double` dollars (layout note below); the WS server emits dollars under the `"strike"` key and accepts dollars (JSON number) on subscribe payloads; decoded flat-file rows carry `strike` in dollars (`Float64` in the Arrow projection). The Rust core codec field is renamed `Contract.strike_thousandths` so the fixed-point wire integer is reachable only under its unit-bearing name (`strike_dollars()` unchanged).
- `EodTick` — the two time columns carry the vendor's v3 field semantics under unit-suffixed names: `ms_of_day` is now `created_ms_of_day` (EOD report creation time, NOT a trade time) and `ms_of_day2` is now `last_trade_ms_of_day` (time of the day's final trade; `0` on no-trade days, which also zero-fill `open`/`high`/`low`/`close`). Renamed across Rust, Python, TypeScript, C, C++, Arrow/pandas/polars columns, and the CLI; the HTTP server emits the vendor's own `created` / `last_trade` JSON keys.
- `CalendarDay` — `is_open` is a boolean and `status` carries the vendor day-type vocabulary instead of undocumented integers: a `CalendarStatus` enum in Rust (`Open` / `EarlyClose` / `FullClose` / `Weekend`, exported from the crate root), the strings `"open"` / `"early_close"` / `"full_close"` / `"weekend"` in Python, TypeScript, Arrow output, and the HTTP server, and an `int32_t` code plus `thetadatadx_calendar_status_name()` lookup in C. Unknown wire text still fails decode loudly. A rows-present calendar response missing the `type` column now surfaces a typed missing-header error instead of silently filling closed days.
- Python — Greeks rows render the keyword-colliding `lambda` column as `lambda_` (PEP 8 keyword convention) on `GreeksAllTick`, `GreeksEodTick`, `GreeksFirstOrderTick`, `TradeGreeksAllTick`, and `TradeGreeksFirstOrderTick`: attribute, constructor kwarg, and repr all use `lambda_`, so the attribute is reachable with normal syntax. Arrow / pandas columns keep the logical `lambda` name. The generators now route every field through per-language reserved-word tables, so a future keyword-colliding column cannot reintroduce the defect in any binding.
- Rows — absent contract identity is `None` / `undefined` in Python and TypeScript (and an Arrow null in columnar output) instead of the `0` / `0.0` / `""` sentinels, matching the streaming payload convention; the C-layout Rust and C rows keep their documented fills (`0`, `0.0`, `'\0'`) with `has_contract_id()` as the presence check. Zero-fill remains, documented per field, where absence is unambiguous (sizes, volumes, the no-trade EOD price columns).
- TypeScript — every endpoint method takes its required parameters positionally followed by ONE optional trailing options object (`stockHistoryEOD("AAPL", { startDate, endDate, timeoutMs })` style) instead of positional optional parameters with `undefined` holes; each endpoint exports a `<Method>Options` interface and `timeoutMs` rides in the same object.
- TypeScript — every historical endpoint method returns a `Promise` resolved off the runtime's execution thread instead of a synchronous value: all 61 data-fetch methods (`stockHistory*` / `optionHistory*` / `indexHistory*` / `*Snapshot*` / `*AtTime*` / `*List*` / `calendar*` / `interestRate*`) now resolve `Promise<Array<T>>` where the element type `T` is unchanged, so `const rows = await client.historical.stockHistoryOHLC("AAPL", "20250303", { interval: "1m" })` replaces the bare assignment. The network round-trip runs on a worker, so a fetch never holds the Node event loop — timers fire, queued promises advance, and concurrent requests make progress while a request is in flight. Call sites add `await`.
- Streaming events — the protocol parse-error event class is `ParseError` in every binding (kind tag `parse_error`); no binding ships a class named bare `Error` anymore (the Python exception tree roots at `ThetaDataError`, the JS global `Error` is unshadowed, C/C++ expose `ThetaDataDxStreamParseError` / `thetadatadx::StreamParseError`). The `ThetaDataDxStreamEventKind` C enum discriminants renumber accordingly.
- C ABI — `ThetaDataDxContract.strike` is `double` dollars (was `int32_t` thousandths) at offset 24, and the contract gains a trailing `int32_t strike_thousandths` (the exact wire integer) at offset 32, growing the struct from 32 to 40 bytes. Recompilation against the new header is required, but no existing `ThetaDataDxContract` field moves. `ThetaDataDxOptionContract.right` and the per-tick `right` fields are `uint32_t` Unicode scalar values (same offsets); `ThetaDataDxCalendarDay.is_open` is C99 `bool` (same offset, 3 padding bytes follow).
- Columnar output — DataFrame / Arrow column names follow the public field names everywhere: `OptionContract` tables emit `symbol` (was the wire spelling `root`) and `InterestRateTick` tables emit `date` (was the wire spelling `created`), matching the row-object attributes.
- HTTP server — REST responses carry the vendor's v3 field shapes for contract identity: `expiration` is an ISO `YYYY-MM-DD` string and EOD rows use the `created` / `last_trade` keys. Time-of-day values remain Eastern-Time millisecond integers paired with `date`, documented as the SDK's raw-time convention.
- Python stub — the `StreamParseError` stub alias and the phantom `Error` exception stub are gone; the stub names match the runtime exactly.
- The async worker-thread configuration knob is renamed to a neutral client name on every binding: Python `worker_threads`, TypeScript `setWorkerThreads` / `workerThreads` (and the exported `WorkerThreadsSetting`), C++ `set_worker_threads` / `get_worker_threads`, and the C-ABI `thetadatadx_config_set_worker_threads` / `thetadatadx_config_get_worker_threads` (the prior names carried a runtime-internal token). The presence-plus-value ABI shape is unchanged.

### Added

- `strike_thousandths` on the streamed option contract across every binding, alongside the existing dollars `strike`: the exact wire integer (a `$550.00` strike is `550000`), for callers that key on an exact integer rather than a float. Exposed as `ThetaDataDxContract.strike_thousandths` (C ABI / C++), `contract.strike_thousandths` (Python), and `contract.strikeThousandths` (TypeScript); Rust already carried it as the primary `strike_thousandths` field with `strike_dollars()` for the dollars view.
- An optional `consumer_cpu` knob that pins the streaming tick-consumer thread to a CPU core for deterministic, low-jitter delivery (`None` default leaves it under the OS scheduler), exposed on Python (`consumer_cpu`), TypeScript (`consumerCpu` / `setConsumerCpu`), C++ (`set_consumer_cpu` / `get_consumer_cpu`), and the C ABI (`thetadatadx_config_set_consumer_cpu` with a negative `THETADATADX_CONSUMER_CPU_UNPINNED` sentinel); Rust additionally exposes a zero-cost generic `StreamingClient::for_each_with_wait_strategy` override that accepts any user `WaitStrategy` impl for the drain loop.
- Epoch-instant accessors on every row that carries `date` plus a milliseconds-of-day column, computed on read (raw integer fields stay primary): Rust methods and Python properties named `timestamp_ms` for the bare `ms_of_day` column and `<prefix>_timestamp_ms` for prefixed columns (`created_timestamp_ms`, `last_trade_timestamp_ms`, `underlying_timestamp_ms`, `quote_timestamp_ms`), the C function `thetadatadx_timestamp_ms(date, ms_of_day)`, and the C++ `thetadatadx::timestamp_ms` wrapper. All return Unix epoch milliseconds (UTC, DST-aware) and signal absent dates (`None` / `-1`).
- `thetadatadx::time::date_ms_to_epoch_ms` — the DST-aware inverse of the epoch-to-Eastern split, shared by every accessor above.
- `thetadatadx::CalendarStatus` — the exported calendar day-type enum with `as_str()` / `from_wire_text()` / `from_code()` / `is_open()`.
- Live market value across every binding — a per-contract theoretical bid / ask the SDK computes from the real-time quote, delivered as `StreamData::MarketValue` with `market_bid`, `market_ask`, and the integer-midpoint `market_price`. The subscription is built with `Contract::market_value()` (Python `market_value()`, TypeScript `marketValue()`, plus the C++ and C-ABI generated forms); it is a per-contract subscription with no full-stream variant.
- TypeScript reaches cross-binding parity with the other SDKs on three surfaces: the offline Greeks calculator (`allGreeks(...)` returning a typed object with the 23 Greek fields, and `impliedVolatility(...)` returning the `(iv, iv_error)` pair); historical-result streaming via `<endpoint>Stream(...)` that delivers typed row chunks through a thread-safe callback so peak memory tracks one chunk rather than the full result; and the `deriveOhlcvc` config toggle. The C ABI and C++ also gain historical-result streaming through a tick-chunk callback (`thetadatadx_<endpoint>_stream` / the C++ `<endpoint>_stream(..., handler)` over a contiguous span), with `option_list_contracts` remaining buffered-only on the C ABI.
- An Arrow-IPC terminal on the TypeScript and C++ history results — per-collection `<tick>ToArrowIpc(rows)` (TypeScript) and `thetadatadx::<collection>_to_arrow_ipc(rows)` (C++) emit the same Arrow IPC stream bytes as the existing flat-file terminal; an empty result is a valid zero-row stream carrying the schema.
- A `from_file` client-construction convenience across the bindings: the Python unified `Client.from_file(path, config=None)`, the C-ABI `thetadatadx_*_connect_from_file(path, config)` trio, and the C++ `from_file(path, config = Config::production())` statics, all defaulting to the production configuration so a credentials file is the only required input.
- Python `AsyncClient` gains awaitable constructors `await AsyncClient.connect(creds, config)` and `await AsyncClient.connect_from_file(path, config=None)` so async callers can establish a connection from inside a coroutine without the authentication handshake stalling the running event loop. The synchronous `AsyncClient(creds, config)` and `AsyncClient.from_file(...)` constructors stay available for construction outside a running loop.
- Python flat-file fetches gain awaitable `*_async` twins so the full-day blob download resolves off the event loop when reached through `AsyncClient.flat_files`: `option_trade_quote_async` / `option_open_interest_async` / `option_eod_async` / `stock_trade_quote_async` / `stock_eod_async` / `request_async` on the namespace yield the same `FlatFileRowList`, and `Client.flatfile_to_path_async(...)` yields the on-disk path. The synchronous methods keep their blocking behaviour for plain `Client` use; `await flat_files.option_eod_async(date)` inside a coroutine no longer stalls the running loop for the duration of the download.
- TypeScript precomputed epoch-instant fields on every tick that carries a `date` plus a milliseconds-of-day column (`createdTimestampMs`, `lastTradeTimestampMs`, `timestampMs`, `underlyingTimestampMs`, `quoteTimestampMs`), one-for-one with the Python `*_timestamp_ms` properties and resolved through the same DST-aware core conversion.
- TypeScript `Subscription` exposes the `contract` and `secType` getters Python already had, and `toString()` rendering is available on the TypeScript `ContractRef`, `Subscription`, and `SecType` values; C++ gains `operator<<` and a `thetadatadx::str(...)` rendering for the same fluent value types.
- Trade flag-word accessors are generated into every binding (previously Rust-only on `TradeTick`): `is_cancelled`, `regular_trading_hours`, `is_seller`, `trade_condition_no_last`, `price_condition_set_last`, and `is_incremental_volume` (Python computed properties, TypeScript precomputed boolean fields, C++ free functions). The C ABI also gains `thetadatadx_contract_strike_dollars`, the dollar-valued counterpart of the existing C++ `thetadatadx::strike(...)` accessor.
- List endpoints (`*_list_*`) return their values sorted ascending in every binding, numeric-aware for strike / date lists.
- Python fluent `Contract` stubs the `expiration` / `strike` / `right` getters.
- `sdks/parity.toml` gains `[[value_field]]` rows — declared per-binding field TYPES on the load-bearing value classes, enforced by `scripts/check_binding_parity.py`, so a unit-bearing field type cannot silently drift between bindings again — and a `[[utility]]` section and `[[historical_streaming]]` family so the offline calculators and the per-endpoint streaming terminals carry the same machine-checked cross-binding contract.
- The historical migration page (`docs-site/docs/migration/v12-to-v13.md`) walks every breaking surface change per binding with before / after samples, the strike-units hazard called out first.
- The slow-callback watchdog reaches every binding on both streaming surfaces (the unified stream view and the standalone streaming client): a `slow_callback_count()` cumulative counter and a microsecond threshold setter (`set_slow_callback_threshold_us(...)`) — Python `slow_callback_count()` / `set_slow_callback_threshold_us(...)`, TypeScript `slowCallbackCount()` / `setSlowCallbackThresholdUs(...)`, C++ `slow_callback_count()` / `set_slow_callback_threshold_us(...)`, and the C-ABI `thetadatadx_client_slow_callback_count` / `thetadatadx_client_set_slow_callback_threshold_us` plus the `thetadatadx_streaming_*` pair. The watchdog is observability-only — it counts callback invocations that run over budget and logs them at a rate-limited cadence, and never cancels or kills a callback.
- A `request_timeout_secs` historical-channel configuration knob (default 300 seconds) seeds a per-request deadline when a caller sets none, so a server that holds the response stream open while sending no data cannot hang a collect or drain indefinitely. An explicit per-request deadline still overrides it, a zero-duration per-request deadline opts a single request out, and a configured default of `0` disables the fallback. The knob is forwarded across every binding.
- C++ closes the last async-parity gap with the other SDKs: every buffered historical, snapshot, at-time, list, calendar, and interest-rate query on `thetadatadx::HistoricalClient` and the unified client's `historical()` view gains an `<endpoint>_async(...)` companion that returns `std::future<std::vector<Row>>` over the same row type as the blocking call (for example `auto fut = client.stock_history_eod_async("AAPL", "20240101", "20240131"); auto rows = fut.get();`), so callers run a request off the calling thread without managing their own threads, matching the TypeScript `Promise` and Python `list_async()` query surfaces. The companion runs the existing blocking call on a fresh thread via `std::async` and re-raises any typed error on `future::get()`; the async companions are implemented inline in the header over the existing blocking C ABI, which stays callback/poll by design, and a single client handle is not safe for concurrent in-flight requests, so fan-out shares one handle per request or synchronises externally. `sdks/parity.toml` gains a `[[historical_async]]` family so the per-endpoint async query surface is machine-checked across Python, TypeScript, and C++.

### Changed

- The `tdbe` time-and-calendar crate is folded into `thetadatadx` as a private internal module, so the workspace builds and publishes a single `thetadatadx` artifact. The curated public surface is unchanged (`TradeTick`, `greeks::all_greeks`, `SecType`, `utils::conditions`, and the calendar types are reached at `thetadatadx::*` paths); the offline-analytics error stays addressable as `greeks::Error` at its public crate path, while the data-layer internals — the fixed-point price encoding (`Price` / `PriceType` / `PriceError` / `MAX_PRICE_TYPE`), the DST epoch math, the canonical-JSON helpers, and the FIT / FIE codecs — stay behind the existing `__internal` feature and off the stable public surface.
- The fixed-point price encoding is an internal wire-transport detail and no longer appears on the public crate surface: `Price`, `PriceType`, `PriceError`, and `MAX_PRICE_TYPE` are removed from the curated API and rendered documentation. Every tick row already carries its price as decoded `f64` dollars, so a client never constructs, sets, or reasons about the raw mantissa-and-exponent pair; the decode boundary is the only place the encoding is built, and an out-of-range exponent is now unrepresentable by construction rather than clamped on each read.
- The direct `StreamingClientBuilder` now defaults its streaming buffer to the same size as the production streaming configuration, so a builder-constructed client gets production-grade overflow headroom by default for large streams (10k-15k option contracts plus full trade streams) instead of a much smaller buffer that drops events under market bursts; set `.ring_size(..)` to choose a smaller footprint. The streaming backpressure documentation is also corrected to state that newly arriving events are dropped when the buffer is full, not the oldest buffered events.
- The bring-your-own wait-strategy trait and the ready-made strategies used by the Rust generic streaming-drain escape hatch are re-exported under `thetadatadx::streaming::wait`, so a caller naming a custom wait strategy refers only to crate-owned paths and never adds an internal dependency to their own manifest. The zero-cost generic override is unchanged; only the path it is named through moved onto the crate's own surface.
- The flat-file request-type and contract security-type fields render as stable wire tokens (`trade_quote`, `open_interest`, `eod`, `STOCK`, `OPTION`, `INDEX`, `RATE`) on the JSON, WebSocket, and fluent client surfaces instead of an internal variant identifier, so the token a client parses against is decoupled from the implementation and cannot shift under an internal rename. Output is identical to the prior rendering for every current value.

### Removed

- The `concurrent_requests` configuration knob is removed from the public client API on every binding; historical request concurrency is now derived automatically from the account's subscription tier (Free=1, Value=2, Standard=4, Pro=8) and sized into the connection pool at connect time, so there is no value to set, clamp, or migrate.

### Fixed

- The TypeScript surface no longer renders internal runtime types in its public type declarations: the streaming callback and the namespace-handle doc comments described their plumbing in terms of internal wrappers, which the generated `index.d.ts` shipped to clients. The docs now describe the behavior on its own terms, so the published declarations carry only the client-facing surface.
- TypeScript `streamingRingSize` is a `bigint`, matching the unsigned-pointer width the field carries on every other binding (it was a 32-bit number that silently capped a large value); the setter and getter round-trip the full range.
- The embedded async runtime in the C ABI, Python, and TypeScript bindings honors the `worker_threads` configuration. The runtime was a hard-coded global singleton that ignored the knob; it is now built lazily from the first client's configuration at connect time, so the value the first client in the process sets takes effect. Python no longer constructs the runtime at module import.
- The C ABI now installs the ring rustls `CryptoProvider` from each connect entrypoint (`thetadatadx_client_connect`, `thetadatadx_historical_connect`, `thetadatadx_streaming_connect`). A C ABI library has no module-init hook, so consumers linking the shared library — the C++ binding among them — previously hit an unconfigured-provider panic on the first TLS handshake and could not open a connection. The Python, TypeScript, CLI, and server entrypoints already installed it at load time.
- The streaming decode path saturates the market-value arithmetic at the integer extremes instead of overflowing, so a malformed or adversarial quote can no longer panic the decoder; output is byte-identical on real prices.
- OHLCVC `volume` and `count` decode as unsigned 32-bit wire fields, so a session above roughly 2.2 billion shares no longer decodes to a negative value. The public `volume` / `count` fields were already 64-bit signed; only the decoded value is corrected.
- Streaming reconnect prefers the host that was last serving data: once a session survives the stable window the last-known-good address is pinned and tried first on the next reconnect, then the full configured host-selection policy runs.
- The CLI's raw OHLC output (`--format json-raw` / `csv`) emits the `vwap` value in its own column. The raw value row was one field short of its header set, so `vwap` was dropped and `date`, `expiration`, `strike`, and `right` rendered under the wrong column names. The typed SDK surfaces and the CLI's presentation `json` were unaffected.
- Python — the standalone `StreamingClient` streaming connect, reconnect, and subscribe / unsubscribe paths no longer block other Python threads across their blocking I/O (the TLS connect and handshake, and the per-subscription wire write), so other Python threads keep running while a connect or subscribe is in flight; the typed exception raised on failure is unchanged.
- The standalone Python and TypeScript streaming clients now forward the full streaming and reconnect config, so every tuning knob — including consumer-core affinity, host selection, watchdog and keepalive cadences, and the reconnect backoff and replay pacing — is honored, matching the unified client and the C ABI.
- The bundled WebSocket server acknowledges stream requests with ThetaData's stream-verification values: a successful subscribe, unsubscribe, or stop now returns `SUBSCRIBED` instead of `OK`, and a subscribe that arrives before streaming has started returns `ERROR` with a descriptive message instead of a false-positive `OK` that claimed success while installing nothing.
- Oversized streaming credentials now fail with a typed invalid-parameter configuration error before the connection is opened instead of panicking the caller. The credentials payload is validated against the 255-byte protocol frame limit up front, and the error names both the limit and the actual size so a too-long email or password is reported as a normal recoverable failure.
- Active streaming subscriptions are de-duplicated by contract and by full-stream security type, so a repeated subscribe no longer accumulates duplicate tracked entries that get replayed multiple times after a reconnect; unsubscribe still removes the tracked entry. The control channel that carries subscribe / unsubscribe / heartbeat commands to the I/O worker is now bounded, so a control-plane burst cannot grow it without limit: the heartbeat takes natural backpressure and the public subscribe / unsubscribe methods return a typed queue-full error rather than dropping a command or accumulating unbounded memory.
- Parsing an OCC-21 option identifier no longer panics on non-ASCII input. The 20-byte repair path that re-pads a root sliced at a fixed byte offset, which could land inside a multi-byte character; a non-ASCII identifier now falls through to the bare-root validator and returns a typed error naming the offending input, instead of aborting.
- The offline Greeks and implied-volatility entry points reject a non-positive or non-finite spot or strike with a typed invalid-parameter error that names the offending value, instead of letting it flow into the model and surfacing as `NaN` or a JSON `null`. The infallible bundle path treats the same inputs as degenerate and returns an all-zero, all-finite result.
- Flat-file CSV and JSONL output emit option strikes in dollars, matching the Arrow and typed-row surfaces; the same request previously produced two different strike units depending on the chosen output format, and the raw scaled wire integer could reach a CSV or JSONL consumer. A single shared conversion now feeds all four formats so they agree on the exact value, sub-dollar strikes included.
- The flat-file download path enforces connect and read timeouts, so a host that accepts the socket but never finishes the handshake, or a server that stalls mid-stream, can no longer block a download indefinitely. The connect bound defaults to 10 seconds and the per-frame read bound to 60 seconds (far beyond any healthy inter-chunk gap), both classified as transient so the existing retry ladder reconnects on a fresh session; the bounds are configurable.
- The real-time streaming socket now sets a write timeout alongside its read timeout, so a write against a peer whose receive window has stalled — alive enough to ACK at the kernel but not draining the socket — surfaces as a fatal I/O error that the reconnect path takes over, rather than blocking the credentials write or a steady-state ping or subscribe indefinitely.
- The streaming `Drop` self-join guard, which prevents an inline join deadlock when a user callback drops the last client handle while running on the consumer thread, now fires in shipped builds. It previously read a consumer thread id that only the test harness recorded, leaving the protection inert outside tests; the id is now captured at the single drain entry point every path routes through.
- Error cause-chains are preserved through the public error type: the configuration and decode error families now carry their typed cause via `std::error::Error::source()`, so a chain walker reaches the original error without parsing the display text, matching the I/O, HTTP, and TLS variants that already did so. The credentials-file read carries its underlying I/O error the same way; display output is unchanged.
- The C-ABI set-callback entries reject a null callback function pointer at registration with a normal error and diagnostic message, instead of storing it and faulting the process when the first event arrives. A consumer that passes a null callback now gets a recoverable failure at the call that installed it; the symbol signature and header representation are unchanged.
- The streaming control-event discriminants (disconnect and reconnect reasons, and the subscription result) convert onto the C-ABI integer through their declared representation, so every discriminant widens losslessly and totally with no silent wrap, on the FFI, Python, and TypeScript surfaces alike. Should a representation ever stop fitting the wire integer the conversion fails to compile rather than truncating onto the wire.
- The configuration loader rejects misconfiguration at load time instead of silently running a default. A misspelled section or field, a removed section, or an unrecognized streaming preset name (`flush_mode`) now surfaces as a load error naming the problem, where it previously parsed without complaint and changed nothing; a missing section still falls back to the production default. The shipped sample uses the canonical `[historical]` / `[streaming]` section names (the prior sample names silently discarded every override a user copied), and an out-of-range gRPC message size, an out-of-range or sub-read-timeout data watchdog, and an oversized event ring are each bounded and rejected by name rather than overflowing, uncapping a budget, or committing an absurd allocation.
- Documentation examples are corrected to compile and run against the current surface, and the shipped `config.default.toml` template carries the real production default values, so copying the sample no longer overrides production behaviour the moment a client uses it. The streaming `ms_of_day` field is documented uniformly as milliseconds since midnight Eastern Time, and the CLI `--format json-raw` mode — which emits dates and times as raw integers rather than ISO-formatted values — is documented alongside the other format choices.
- TypeScript — the bounded integer query filters `maxDte` and `strikeRange` reject a non-finite, negative, fractional, or out-of-range value with `InvalidParameterError` rather than coercing it. The options field rides in as a JS number and is validated before the request is issued, so a hostile or oversized input (for example `3e9`, which would otherwise wrap to a negative count) no longer silently reaches the wire; a valid whole non-negative value is unchanged.
- TypeScript — the published `index.d.ts` examples compile against the package they ship in: the snippets import from `thetadatadx` (the real package name) and call `stockHistoryEOD` (the real method casing), the credentials and historical examples bring `Credentials` / `Client` / `HistoricalClient` into scope so each block is self-contained, and the standalone `StreamingClient.startStreaming` callback is typed as `(event: StreamEvent) => void` instead of an unresolved identifier, so a caller's `event` parameter carries the typed payload. The doc-example gate type-checks every shipped snippet against the declarations, so a broken example fails the build instead of reaching clients.
- Out-of-order trade corrections on the derived OHLCVC stream no longer double-count volume and trade count: a late correction that revises an already-counted trade now adjusts the running bar in place instead of adding to it, so the derived bar's volume and count match the corrected tape. A failed write to the streaming socket now escalates to a reconnect rather than being swallowed, so a half-open connection is recovered instead of silently stalling the stream. The login handshake is bounded by a wall-clock deadline, so a server that accepts the connection but never completes authentication fails with a timeout and the reconnect path takes over, rather than blocking the connect indefinitely.
- The offline implied-volatility solver returns the no-implied-volatility signal for a sub-cent target option price instead of a fabricated converged value. A target option price below the solver's price floor previously reported a converged implied volatility that was off by several volatility points; it now signals that no implied volatility is available for that price. This is distinct from the non-positive / non-finite input rejection above: it covers a valid but vanishingly small price the solver cannot resolve.
- The Python `Contract` builder hashes consistently with its equality, so it works correctly as a dictionary key or a set member. It defined equality without a matching hash, so two equal contracts could land in different buckets and a contract used as a key behaved unpredictably; the hash now agrees with equality.

### Security

- `pyo3` and `pyo3-async-runtimes` are upgraded to 0.29, clearing RUSTSEC-2026-0176 (an out-of-bounds read in the bound list and tuple iterators' `nth` / `nth_back`) and RUSTSEC-2026-0177 (a missing `Sync` bound in closure construction) in both the Python binding and the core crate's bench-only development dependency. With the upgrade in place the advisory deferrals are removed and the audit gate runs with an empty ignore list. The DataFrame export switches to the Arrow C Stream Interface so the binding's `pyo3` line is no longer pinned by `arrow-pyarrow`; the free-threaded CI target moves to CPython 3.14t, since `pyo3` 0.29 floors free-threaded support at 3.14.
- The streaming, historical, and flat-file client TLS configurations are built with an explicit `ring` crypto provider and explicit protocol versions rather than relying on a process-global default; the historical / authentication HTTP path is handed the same preconfigured provider, so `ring` is the sole crypto provider in the dependency graph and a connect no longer depends on an installed process default.
- The server's general per-IP rate limiter is now opt-in and off by default on every bind regardless of address: operators enable it by setting `THETADATADX_RATE_LIMIT_PER_SECOND` and/or `THETADATADX_RATE_LIMIT_BURST_SIZE` (previously it auto-enabled on non-loopback binds); the shutdown-route limiter stays active on every bind.
- The streaming login no longer leaves the account password in a freed memory buffer after connect and reconnect. The credentials are now wiped from memory the moment the login frame is sent, so the cleartext password is no longer retained in released heap memory or in a buffered protocol frame once authentication completes, on the first connect and on every reconnect alike.

## [12.0.0] - 2026-06-04

### Breaking changes

- Rust core — `StreamingClient::builder(&creds, &hosts)` is the sole public constructor for streaming. Fluent setters (`.ring_size()`, `.flush_mode()`, `.read_timeout_ms()`, `.connect_timeout_ms()`, `.reconnect_policy()`) replace the prior struct-literal connection-parameter bundle; the underlying `StreamingClient::connect(args)` entry and the streaming connection parameters are crate-internal.
- Rust core — drain primitives live on `StreamingClient` itself: `next_event` (blocking), `try_next_event` (non-blocking), `poll_batch(FnMut)`, `for_each(FnMut)`, and `Iterator for &StreamingClient`. The standalone streaming event-poller type is removed.
- Rust core — new typed `FpssError` enum (`#[non_exhaustive]`) returned by the builder, polling methods, and iterator. `From<FpssError> for Error` maps each variant losslessly into a distinct umbrella `Error` variant (`Auth`, `Config`, `Io`, tagged `Fpss`); the docstring on `FpssError` lists every row of the mapping table plus the two known sources of information loss (`io::ErrorKind` collapse on `Io` round-trip, `Config.field` regeneration).
- Rust core — engine internals (`mdds`, `endpoint`, `decode`, `wire`, `EndpointArgs`, `ENDPOINTS`, `HistoricalClient`, `SubscriptionTier`) are gated behind the `__internal` Cargo feature and marked `#[doc(hidden)]`. Downstream crates that imported these symbols directly should either move to the public surface (every endpoint is wired through methods on `Client`) or pin the `__internal` feature with the understanding that those symbols are not covered by the semver contract.
- Rust core — `Contract.symbol` type changed from `String` to a reference-counted shared string. The field continues to deref through `&str`, `PartialEq<str>`, slicing, and `Display` so most call sites compile unchanged; deep-clone sites should switch to `Arc::clone` to benefit from the interned-symbol cache. The Rust decode path interns the symbol bytes inside the per-session contract cache so a sustained subscription set allocates once per unique symbol for the lifetime of the session. Python and TypeScript bindings retain `String` types at the language boundary (runtime ownership) populated via `.to_string()` on each delivered event.
- Rust core — `ThetaDataDx::start_streaming_iter`, `start_streaming_iter_with_wake`, and `start_streaming_iter_with_wake_policy` removed. The push-callback `start_streaming(callback)` is preserved on the unified client; it internally spawns a dedicated `"client-fpss-dispatcher"` thread that drives the event-queue iterator behind a one-shot startup gate so the first delivered event only fires once the streaming slot is fully installed.
- Rust core — REST fallback escape hatch (`Config::with_rest_fallback`, `FallbackPolicy`, `option_history_*_with_fallback`) removed. The library now speaks ThetaData's historical gRPC endpoint, streaming feed, and the native flat-file distribution directly; the standalone `tools/server` binary retains its terminal-compatible HTTP / WebSocket front end for existing consumers.
- C ABI — `thetadatadx_config_set_flush_mode(ThetaDataDxConfig*, int32_t mode)` returns `int32_t` (was `void`). Pass `0` for Batched or `1` for Immediate. Any other integer (including null `ThetaDataDxConfig*`) returns `-1` and sets `thetadatadx_last_error` plus `thetadatadx_last_error_code = THETADATADX_ERR_CONFIG`. The C++ wrapper (`set_flush_mode(int)`) translates the failure into `std::runtime_error`, mirroring the existing `set_nexus_url` pattern.
- C ABI — pull-iterator surfaces (`ThetaDataDxStreamEventIterator`, `thetadatadx_client_start_streaming_iter`, `thetadatadx_streaming_event_iter_next`, `thetadatadx_streaming_event_iter_close`, `thetadatadx_streaming_event_iter_free`) removed. Use `thetadatadx_client_set_callback` or `thetadatadx_streaming_set_callback` for delivery. The `thetadatadx_streaming_free` and `thetadatadx_client_free` docstrings document the lifecycle restriction: do not call them from inside the user callback.
- Python — pull-iterator surfaces (`streaming_iter`, `streaming_async`, `streaming_async_batches`) and their session pyclasses (`EventIterator`, `StreamingIterSession`, `StreamingAsyncSession`, `StreamingAsyncBatchesSession`, `BackpressurePolicy`) removed. Use `client.start_streaming(callback)` (one-shot) or `with client.streaming(callback): ...` (context manager).
- TypeScript — `EventIterator` napi class and `client.startStreamingIter()` removed. Use `client.startStreaming(callback)`.
- C++ — `thetadatadx::FallbackPolicy` class and the `_with_fallback` method family removed.

### Added

- Streaming reconnect engine hardened for multi-minute upstream outages. The `Auto` policy drives an exponential delay ladder for generic transient drops (`ReconnectConfig::wait_ms` doubling to the new `wait_max_ms` cap) bounded by both an attempt budget and a new wall-clock envelope (`ReconnectAttemptLimits::max_elapsed`, `0` disables); `ServerRestarting` gets its own attempt class with a flat patient cadence (`wait_server_restart_ms`, `max_server_restart_attempts`); the rate-limited class keeps its multi-hour budget and is exempt from the envelope. Every reconnect delay is jittered per the new `ReconnectConfig::jitter` knob (`JitterMode` enum: `Full` default / `Equal` / `Decorrelated` / `None`); the rate-limited floor is honoured in full with the jitter window above it.
- `StreamControl::ReconnectsExhausted { reason, attempts }` — typed terminal event published whenever the streaming I/O loop stops attempting recovery for a non-user-initiated cause (budget or envelope exhaustion, permanent disconnect reason, `Manual` policy, `Custom` policy returning `None`, permanent login rejection during reconnect). Operators can now distinguish "recovery gave up" from a clean `shutdown()`. Wired through the generated Python / TypeScript / C / C++ event surfaces with `reason_name` resolution alongside `Disconnected` and `Reconnecting`.
- Fault-domain-aware host selection. `StreamingConfig::host_selection` (`HostSelectionPolicy::Shuffled` default / `FixedOrder` escape hatch) groups the host list by hostname, shuffles per client, and interleaves across groups — a fleet spreads its connects across physical machines and the first failover attempt lands on a different box instead of a second port on the machine that just failed. `host_shuffle_seed` makes the order deterministic for fleet sharding and tests; the reconnect path additionally tries the last successfully-connected address first, exposed via `last_connected_addr()` on every binding.
- TCP keepalive on the streaming socket: `StreamingConfig::keepalive_idle_secs` / `keepalive_interval_secs` / `keepalive_retries` (defaults 5 s / 2 s / 2 — roughly 9 s kernel-side detection of a peer that vanished without closing the connection, versus the 2+ hour platform default).
- Last-frame watchdog + public staleness clock. `StreamingConfig::data_watchdog_ms` (default 30 s, `0` disables) force-reconnects when no frame of any kind has arrived inside the window, backstopping widened read timeouts; `millis_since_last_event()` / `last_event_received_at_unix_nanos()` expose the clock on `Client` and `StreamingClient` across every binding so operators can build their own staleness alerts.
- Paced subscription replay. Auto-reconnect restores saved subscriptions in bursts (`ReconnectConfig::replay_burst_size`, default 50) with a jittered pause between bursts (`replay_pace_ms`, default 5 ms) instead of writing the entire set back-to-back at a recovering server. The public `restore_subscriptions(per_contract, full_type)` method on `Client` and `StreamingClient` is the same paced engine for caller-driven flows; both embedded-binding reconnect paths route through it instead of re-implementing the replay loop.
- Custom reconnect policies reachable from every binding: `thetadatadx_config_set_reconnect_callback` (C, with a documented cross-thread contract), `config.reconnect_callback = callable` (Python), `setReconnectCallback(fn)` (TypeScript, threadsafe-driven with a bounded decision wait), `set_reconnect_callback` (C++). The closure receives only retriable reasons — permanent disconnects (bad credentials, account conflicts) short-circuit before any policy is consulted, on the built-in and custom paths alike.
- Shared backoff module (`thetadatadx::backoff`): one capped exponential ladder, the AWS full-jitter sampler, the `JitterMode` enum, and the hash-stable transport-reconnect jitter now serve every retry surface (streaming reconnect, historical retry, transport reconnect, flatfile retry).
- `RetryPolicy::max_elapsed` (default 5 minutes, `0` disables) — wall-clock envelope on historical-channel retry sequences, so operators can state "retry for up to N minutes" directly instead of deriving it from attempt counts; `FlatFilesConfig::jitter` (default on) de-synchronises post-outage backfill retries across a fleet. Server-supplied `google.rpc.RetryInfo` hints are decoded from `grpc-status-details-bin` into `Error::Grpc { retry_after }` and the retry loop raises its sleep to at least the hint.
- Cross-binding exposure for the streaming transport scalars that were previously tunable from Rust only: `streaming_timeout_ms`, `streaming_connect_timeout_ms`, `streaming_ping_interval_ms`, `streaming_ring_size`, plus the new `streaming_io_read_slice_ms`, `streaming_data_watchdog_ms`, keepalive trio, and host-selection pair — each as a Python property, TypeScript setter + getter, C-ABI `thetadatadx_config_set_*`/`get_*` pair, and C++ forwarder, with `sdks/parity.toml` rows. The reconnect budgets gain readback getters (policy selector, per-class attempt budgets, stable window) on the C ABI and C++ so operator dashboards can verify deployed configuration.
- `StreamingFlushMode` (Batched / Immediate) is exposed across every binding: Rust enum on the builder, C ABI `thetadatadx_config_set_flush_mode(int32_t mode) -> int32_t`, C++ `set_flush_mode(int)`, Python `cfg.flush_mode = "batched" | "immediate"` (case-insensitive setter, canonical-lowercase getter), TypeScript `cfg.setFlushMode("batched" | "immediate")` plus `cfg.flushMode` getter. `sdks/parity.toml` carries the matching method-level rows; `sdks/python/tests/test_config_flush_mode.py` and `sdks/typescript/__tests__/flush_mode.test.mjs` exercise the round-trip including the invalid-string `ValueError` path.
- Per-callback panic isolation plus a unified `panic_count` counter — every user-callback invocation is panic-isolated on the Rust, C ABI, and Python event-delivery paths, so a panic in one callback can neither abort the process nor stall the feed. The Python binding routes `PyErr` through `write_unraisable` plus the binding-only `record_panic` shim so the same counter increments for Rust panics and Python exceptions. `panic_count()` is the public read-side accessor.
- Queue-occupancy observability for the streaming event queue: `ring_occupancy()` (point-in-time count of events published but not yet drained into the callback — the leading back-pressure signal that predicts drops before `dropped_event_count()` moves) and `ring_capacity()` (the configured `streaming_ring_size`) on `StreamingClient` and `Client`, mirrored across every binding — Python `ring_occupancy()` / `ring_capacity()` on both client classes, TypeScript `ringOccupancy()` / `ringCapacity()` (`bigint`), C ABI `thetadatadx_client_ring_occupancy` / `thetadatadx_client_ring_capacity` and `thetadatadx_streaming_ring_occupancy` / `thetadatadx_streaming_ring_capacity`, C++ `ring_occupancy()` / `ring_capacity()` on `UnifiedClient` and `StreamingClient`, with `sdks/parity.toml` rows. Occupancy is recorded as events are published and as they are drained, with no contention added to the hot path; sampling it never blocks the feed.
- One canonical streaming-session state machine replaces three duplicate per-binding state machines (formerly maintained separately on the Rust client, the C ABI, and the Python binding), so streaming lifecycle behavior is identical across bindings and a delivery-thread failure is observed deterministically rather than through a racy side channel.
- The streaming start-up handshake signals install success or rollback before the first event is delivered, closing a window where an event could fire against a half-installed session.
- `docs-site/docs/migration/v11-to-v12.md` (new) walks the streaming reshape, the curated public Rust surface, the REST fallback removal, and the cross-binding default flips with before / after Python, TypeScript, C ABI, and C++ snippets. The historical migration page carries a one-line banner pointing at the latest guide.

### Changed

- Tick decoding resolves the column layout once per response and then bulk-extracts each column, lowering per-row decode cost on large responses. Per-cell semantics are unchanged (null zero-fill, mixed `Price`/`Number` price columns, header aliases, strict accept-sets); a wire-shape mismatch now names the schema column and row in the diagnostic.
- Resilience defaults flipped so an unattended client survives a multi-minute upstream pool outage instead of exiting within seconds. Streaming reconnect: `wait_ms` 2 000 → 250 (now the initial rung of an exponential ladder; previously a flat per-attempt delay), `max_attempts` 3 → 30, plus the new `wait_max_ms = 30_000`, `max_elapsed = 300 s`, `wait_server_restart_ms = 5_000`, `max_server_restart_attempts = 60`, `jitter = full`, `replay_burst_size = 50`, `replay_pace_ms = 5`. Streaming transport: `timeout_ms` 10 000 → 3 000 (the upstream heartbeats every ~100 ms, so three seconds of total silence is a dead link), `ping_interval_ms` 100 → 250 (the upstream heartbeat is the primary inbound liveness signal; the client ping mainly proves write-side health), the I/O read slice is now the validated `io_read_slice_ms` knob (default 25 ms, previously hardcoded 50 ms), and `data_watchdog_ms = 30_000` / keepalive 5 s/2 s/2 are on by default. Historical channel: `RetryPolicy::max_attempts` 5 → 20 with `max_elapsed = 300 s`, so the ladder actually reaches and rides its 30 s cap instead of exhausting in roughly eight seconds. Flatfiles: `max_attempts` 3 → 10, `max_backoff` 4 s → 30 s, jitter on (`max_attempts` validation widened to `[1, 100]`).
- The streaming read deadline is enforced on a wall clock measured from the last received frame rather than by counting fixed-width timeout slices, so the configured `timeout_ms` holds exactly regardless of the read-slice size.
- Reconnect cooldowns sleep in 100 ms slices and wake promptly on shutdown — previously a rate-limited 130 s cooldown was uninterruptible and a shutdown raised mid-cooldown waited it out in full. Commands queued against a dead session are discarded before the replacement dial (graceful shutdown is honoured), so stale heartbeats and duplicate subscribe frames never land on a fresh peer.
- `ReconnectPolicy::Custom` closures no longer receive permanent disconnect reasons; the I/O loop short-circuits them first, so a closure can no longer turn a credential rejection into an infinite retry loop.
- The historical-channel pool's rotation cursor is seeded with a per-instance random offset so a fleet restarting together spreads its first requests across the pool instead of pinning them all to the first member.
- `Error::Grpc` carries a new `retry_after: Option<Duration>` field (the decoded server backoff hint); exhaustive matches on the variant need the extra field or `..`.
- TLS / crypto provider — `rustls`, its async and HTTP integration crates, and `rustls-platform-verifier` are pinned `default-features = false, features = ["ring", ...]` across every workspace in the tree (root, `tools/mcp`, `tools/server`, `sdks/python`, `sdks/typescript`). `aws-lc-rs` and its companion crates (`aws-lc-sys`, `cmake`, `dunce`, `fs_extra`) are absent from every Cargo.lock; `cargo tree --invert aws-lc-rs` reports no matches in any of the five workspaces. The binding-side crypto-provider install at module load is now the standard rustls 0.23 `install_default` call rather than a multi-provider tie-breaker.
- `reqwest` is configured with `features = ["json", "query", "rustls-no-provider"], default-features = false` so the historical / Nexus auth path links into the same single rustls provider as the streaming connect path.
- The streaming connect path is simpler internally, with the prior queue-handoff layer removed; behavior is unchanged.
- Connect validates configuration up front and returns the fully-assembled `StreamingClient` directly; the prior tuple return shape is gone.
- `ThetaDataDx::start_streaming` and `reconnect_streaming` serialise through a single-flight lifecycle lock; the FFI handle's lifecycle (`thetadatadx_streaming_set_callback` / `_reconnect` / `_shutdown` / `_free`) is fully serialised through the same per-handle mechanism.
- The panic-recording write side is hidden from the public Rust surface; external crates.io consumers see only the read-side `panic_count()` accessor. The first-party bindings that need to record panics keep doing so unchanged.
- `cargo-semver-checks` CI gate stays anchored on the `11.0.0` baseline; `[package.metadata.docs.rs]` continues to pin the rendered feature list (`arrow`, `polars`, `frames`, `config-file`) so docs.rs renders zero engine internals.
- Generator templates emit `thetadatadx::*` paths so generated SDK code resolves against the single public crate.
- `scripts/check_docs_consistency.py` now scans `docs-site/docs/.vitepress/theme/components/*.vue` for the same dead-API tokens it already blocks in `docs-site/docs/streaming/*.md`. The interactive query-builder live recipes (`live_quote_monitor`, `trade_tape`, `option_flow_scanner`, `live_option_chain`) are rewritten onto the push-callback shape so a paste-and-run of the generated Python lands on the supported API immediately on docs deploy.

- The historical-data transport now rides a mature third-party gRPC stack, replacing the in-house implementation. The closed-loop comparison measured the third-party stack at parity or ahead in every production-reachable cell: +17.6% throughput and -19% p50 at the 16-concurrent account ceiling on small frames, +3.2% throughput on 10 MiB frames, and 2.4x lower allocation per small request. Decoding each response inline measured faster than the prior dedicated decode workers (+5.6% throughput and -11.6% p50 at the ceiling), so the separate decode stage is removed without a replacement. Concurrency still uses one connection per concurrent request: a single shared connection measured at roughly half the throughput at the 16-concurrent ceiling (79 027 vs 42 926 small-frame req/s; 872 vs 385 large-frame MB/s) because every stream then shares one flow-control window. No type from the third-party stack appears in any public signature: transport faults are converted at the crate boundary into the crate's own typed errors (`Error::Transport { kind }` / `Error::Grpc { kind, retry_after }`, carrying the decoded `google.rpc.RetryInfo` back-off hint where present). Recovery after a server-initiated connection close keeps the same caller-visible contract (a transient `ConnectionClosed`, with the next request landing on a fresh connection). TLS reuses the same single-provider configuration as the streaming connect path.
- `HistoricalConfig::window_size_kb`, `connection_window_size_kb`, `keepalive_secs`, and `keepalive_timeout_secs` are load-bearing again: the transport threads them into the channel builder at connect time (HTTP/2 initial stream / connection windows, keepalive PING cadence and timeout). They had been accepted-but-ignored since the transport rewrite that landed in 10.0.0.
- Intermediary HTTP 502 / 503 / 504 replies that carry no gRPC status (a proxy or load balancer answering in place of the server) now classify as the canonical retryable `Unavailable` status, so the retry shell re-dispatches them with backoff; previously they surfaced as a terminal transport error.
- Response frames above `mdds.max_message_size` now surface as `Error::Grpc { kind: OutOfRange }` — the canonical gRPC over-limit status the decode layer emits — instead of `Error::Transport { kind: Codec }`. Both classifications are terminal for the retry shell; the configured ceiling remains load-bearing on every chunk.

### Removed

- Direct dependencies: `subtle` (replaced by a documented constant-time byte compare in the streaming pinning module), the cache-padding utility crate (replaced by a hand-rolled cache-padded newtype), `pastey` (only use site was a no-op `paste! { ... }` wrapper), `pin-project-lite` (every `ServerStreaming` field is `Unpin`; the projection macro was dead scaffolding), `uuid` (replaced by `rand::random::<[u8; 16]>()` plus a hex helper), `chrono` (Python SDK only; replaced by `is_valid_ymd` next to the existing Howard Hinnant date math), the async-stream adapter crate (replaced by a six-line `StreamNextExt` extension trait over `futures-core::Stream`). `regex` moved behind the `config-file` Cargo feature so default builds drop `regex` + `regex-syntax` + `aho-corasick`. The prior bounded event-queue crate is also no longer required.
- Redundant feature flags: the napi async-runtime feature (already implied by `async`), pyo3-async-runtimes `attributes` (no attribute macros used in source), `zeroize` `derive` (only `Zeroizing<T>` wrapper is used).
- The unused `AsyncIterator`, `Awaitable`, and `Iterator` imports in `sdks/python/python/thetadatadx/__init__.pyi`; the dead `StreamingIterSession` row in `scripts/check_binding_parity.py`'s `CPP_ALIASES` map.
- The prior queue-path soak suite; workspace `cargo test` coverage replaces it.

- The in-house gRPC transport implementation (roughly 6.2K lines) and its dedicated benches and internal tests, now that the historical transport rides the third-party stack. The transport regression benchmark stays in tree, now driving the production transport against the recorded baselines.
- The historical decode-thread, decode-queue-depth, and decoder-ring-size knobs (since removed), with their cross-binding setters and getters: Python `decode_threads` / `decode_queue_depth` / `decoder_ring_size` properties, TypeScript `setDecodeThreads` / `setDecodeQueueDepth` / `setDecoderRingSize` (+ getters), C ABI `thetadatadx_config_set_decode_threads[_explicit]` / `thetadatadx_config_set_decode_queue_depth[_explicit]` / `thetadatadx_config_set_decoder_ring_size` (+ getters), and the C++ forwarders. The separate decode stage these knobs tuned no longer exists; decoding now runs inline on each request, sized by the resolved request concurrency alone.
- `TransportErrorKind::{Codec, EmptyResponse, UnexpectedHttpStatus, DecoderPoisoned, DecoderReplyDropped}` — fault categories the transport can no longer produce (the underlying stack normalizes wire-shape violations into gRPC statuses; the decoder pool is gone). The enum keeps `Tcp`, `Tls`, `InvalidServerName`, `H2Handshake`, `H2Stream`, `ConnectionClosed`, and `InvalidPath`.
- Direct dependencies: the bounded-channel crate and `percent-encoding`; both were used only by the in-house transport and were removed with it.

### Security

- Streaming TLS pinning is unchanged: SHA-256 SubjectPublicKeyInfo pin captured 2026-04-20, hostname allow-list of `nj-a.thetadata.us` and `nj-b.thetadata.us`, explicit TLS 1.2 / 1.3 signature verification via `webpki`. The `pin_matches_captured_thetadata_leaf` regression test pins the SPKI byte for byte; `pinned_digest_matches_openssl_output` re-derives it from the captured cert as a typo guard.
- No credentials are logged in plaintext on any code path; the streaming credentials payload is sent on the encrypted channel; `Credentials.password` is wrapped in `Zeroizing<String>` with a `Debug` impl that redacts both fields.

## [11.0.1] - 2026-05-29

### Fixed

- The historical-channel decode path no longer keeps an idle worker busy-spinning a core. Its wait strategy now parks on a 30 µs microsleep floor once its spin and yield phases elapse, instead of ending in a bare CPU-pause hint that never yields to the scheduler, which left an idle worker re-polling continuously and pinning a core at roughly 100% CPU. Idle decode CPU now drops to about 0% with no measurable change to active-path decode latency. ([#619](https://github.com/userFRM/ThetaDataDx/issues/619))

## [11.0.0] - 2026-05-29

### Breaking changes

- `option_history_greeks_eod` return type changes from `Vec<GreeksAllTick>` to `Vec<GreeksEodTick>`. The v10 routing returned a 28-column interval-sampled Greeks shape and dropped the twelve EOD trade/quote columns the server emits on this endpoint (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`, `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`, `ask_condition`). Greek field names are unchanged on `GreeksEodTick`; consumers that need the EOD bar + closing NBBO snapshot must add reads for the new columns. Wire layout verified against the live server.
- `index_at_time_price` return type changes from `Vec<PriceTick>` to `Vec<IndexPriceAtTimeTick>`. The v10 routing returned a 3-column shape and dropped the seven trade-side execution columns (`sequence`, `ext_condition1..4`, `condition`, `size`, `exchange`). `ms_of_day`, `price`, and `date` field names are unchanged; consumers that need the per-row SIP source attribution must read the new `exchange` column.
- Five `option_history_trade_greeks_*` endpoint return types change shape: `Vec<GreeksAllTick>` to `Vec<TradeGreeksAllTick>`, `Vec<GreeksFirstOrderTick>` to `Vec<TradeGreeksFirstOrderTick>`, `Vec<GreeksSecondOrderTick>` to `Vec<TradeGreeksSecondOrderTick>`, `Vec<GreeksThirdOrderTick>` to `Vec<TradeGreeksThirdOrderTick>`, `Vec<IvTick>` to `Vec<TradeGreeksImpliedVolatilityTick>`. The new types carry the nine trade-side execution columns (`sequence`, `ext_condition1..4`, `condition`, `size`, `exchange`, `price`) the per-OPRA-trade endpoints actually emit; Greek field names are unchanged.
- `IvTick` recovers seven columns the v3 server emits on `option_history_greeks_implied_volatility` and the pre-v11 decoder dropped: `bid`, `bid_implied_volatility`, `midpoint`, `ask`, `ask_implied_volatility`, `underlying_ms_of_day`, `underlying_price`. Struct size grows from 64 to 128 bytes; the wire headers `bid_implied_vol` / `ask_implied_vol` resolve through new `HEADER_ALIASES` rows.
- `OhlcTick` recovers the SIP-rule `vwap` column emitted by every `*_history_ohlc` endpoint; snapshot variants default `vwap` to `0.0` via the optional-column path. Field offsets shift past `count`; struct size is unchanged at 128 bytes after alignment.
- `InterestRateTick.ms_of_day` field removed. The server never emits a `ms_of_day` column on `interest_rate_history_eod`; the field was speculative. The C ABI `ThetaDataDxInterestRateTick` re-pads from `{i32, pad, f64, i32, pad[40]}` to `{i32, pad, f64, pad[48]}`; the Python pyclass constructor drops the `ms_of_day` parameter; the TypeScript interface drops `msOfDay`; the C++ wrapper alias follows the C ABI struct.
- `grpc` module narrows from `pub mod` (with `#[doc(hidden)]`) to `pub(crate) mod` in the default-features build; public visibility re-opens only under the private `__test-helpers` feature. Nine names lose their `thetadatadx::grpc::*` reachability: `Channel`, `ChannelError`, `ChannelPool`, `ChannelLease`, `DecoderHandle`, `DecoderPool`, `default_decoder_thread_count`, `Status`, `ServerStreaming`. Transport-layer errors continue to reach consumers via `impl From<grpc::ChannelError> for Error` at the crate boundary; pattern-match on the public `crate::Error` type.
- `fpss::protocol::wire` narrows from `pub mod` to `pub(crate) mod`; the `pub use ... build_*` / `parse_*` re-exports on `fpss::protocol` are gone. Crate-internal callers keep working via `pub(crate) use` shadows; integration tests and benches that need fixture builders import from a new `fpss::protocol::test_wire` re-export module gated on the private `__test-helpers` feature.
- `observability` module narrows from `pub mod` to `pub(crate) mod`. The exporter setup is reachable via the public `DirectConfig::with_metrics_port` knob; no consumer-visible API change.
- `fpss::__test_internals` re-export module is feature-gated on `cfg(any(test, feature = "__test-helpers"))`; `cargo-semver-checks` stops tracking it as a SemVer commitment.
- `Error::Transport` payload restructured from `String` into a typed `Transport { kind: TransportErrorKind, message: String }` shape mirroring the `Grpc { kind, message }` / `Decode { kind, message }` layout. `TransportErrorKind` enumerates the concrete fault categories (`Tcp`, `Tls`, `InvalidServerName`, `H2Handshake`, `H2Stream`, `ConnectionClosed`, `UnexpectedHttpStatus`, `EmptyResponse`, `InvalidPath`, `Codec`, `DecoderPoisoned`, `DecoderReplyDropped`). `Display` stays at `transport error (<kind>): <message>` so string-keyed consumers keep working.
- `ReconnectPolicy::Auto` shape changed to carry the new per-class budget fields (`max_attempts`, `max_rate_limited_attempts`, `stable_window_secs`); callers constructing the variant manually must populate the new fields.
- `GrpcStatusKind` `repr` changed; pattern-match callers depending on the previous discriminant layout must rebuild.
- `SubscriptionInfo` marked `#[non_exhaustive]`; exhaustive matches must add a catch-all arm.
- `Error::config_other`, `Error::decode_other`, and `Error::decompress_other` constructors removed (previously `#[doc(hidden)]` + `#[deprecated(since = "10.0.1")]`). Use the typed kind constructors (`config_invalid`, `config_internal`, `decode_protobuf`, `decode_codec`, `decompress_zstd`, `decompress_unknown_algorithm`). The `Other(String)` variant on each of `ConfigErrorKind`, `DecodeErrorKind`, `DecompressErrorKind` is also removed; the enums are `#[non_exhaustive]` so consumers already had a catch-all arm.
- The historical `decoder_threads` config field removed (previously a deprecated alias for the decompress-thread count in the older split decode path). That thread count now auto-sizes from the available CPU count at connect time and is no longer user-tunable; tune the protobuf-decode and Tick-build worker pool through the decode-threads knob instead. Cross-binding setters (`thetadatadx_config_set_decoder_threads`, `Config.decoder_threads`, `setDecoderThreads`, `thetadatadx::Config::set_decoder_threads`) are deleted in the same sweep.
- TypeScript `Config.flatfileToPath` lowercase backwards-compat alias removed. Use `flatFileToPath`; the lowercase form was retained as a one-version alias for code written against pre-v10.
- `FallbackPolicy::RestOnH2Disconnect` and `FallbackPolicy::RestAlwaysForDateRange` variants removed. `FallbackPolicy::RestAlways` is retained as the user-facing escape hatch for callers who want every historical-quote call routed over a locally-running Terminal's REST surface.
- `_with_fallback` per-endpoint shims tied to the deleted variants are removed on Python, TypeScript, C++, and FFI. The four remaining `option_history_*_with_fallback` methods dispatch on `FallbackPolicy::RestAlways` vs `Disabled` only.
- `Channel::is_dead`, `Channel::mark_dead`, `Channel::dead_handle`, `ChannelPool::all_dead`, `ChannelPool::dead_count` removed and replaced by in-place reconnect. The `ChannelLease` picker no longer scans for live channels separately; every channel in the pool stays a valid pick because the channel itself swaps its inner HTTP/2 session on observed faults.
- `ChannelError::UpstreamCascade` folded into the existing `ChannelError::ConnectionClosed` variant. The mid-stream classifier `classify_h2_error_mid_stream` is gone; the open-phase classifier `classify_h2_error` covers both phases via a new `classify_h2_error_ref` thin wrapper.
- TypeScript `Config.setReconnectStableWindowSecs` accepts `bigint` (was `number`) for true `u64` parity with Python / C++ / FFI. Callers passing `Number` should wrap with `BigInt(60)`; negative or above-`u64::MAX` BigInt inputs are rejected at the boundary with a descriptive error rather than silently truncating.
- `RestError::CsvDecode` and `RestError::MissingColumn` lift into `Error::Transport { kind: Codec, .. }` instead of `Error::Config { internal, .. }`, matching the gRPC-side `ChannelError::Codec` mapping so retry classifiers dispatch uniformly across both transports.
- Minimum supported Python raised to 3.12. The abi3 floor and `requires-python` move from 3.9 to 3.12; CPython 3.9 (EOL Oct 2025), 3.10, and 3.11 wheels are no longer published. Free-threaded 3.13t / 3.14t wheels are unaffected.

### Added

- `GreeksEodTick` carries the full 39-column EOD wire row published on `option_history_greeks_eod`: twelve EOD trade/quote columns (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`, `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`, `ask_condition`) alongside every Greek and the underlying snapshot. Available on every binding: Rust direct, tdbe, FFI (`ThetaDataDxGreeksEodTickArray`), Python pyclass, TypeScript napi, and C++ (`thetadatadx::GreeksEodTick`).
- `IndexPriceAtTimeTick` carries the full 10-column wire row published on `index_at_time_price`: seven trade-side execution columns (`sequence`, `ext_condition1..4`, `condition`, `size`, `exchange`) alongside `ms_of_day`, `price`, and `date`. Available on every binding (Rust direct, tdbe, FFI, Python, TypeScript, C++).
- Five `TradeGreeks*Tick` types model the per-OPRA-trade Greek endpoints (`option_history_trade_greeks_all`, `_first_order`, `_second_order`, `_third_order`, `_implied_volatility`). Each carries the nine trade-side execution columns alongside the relevant Greek subset and the underlying snapshot. Available on every binding.
- EOD-Greek, index-price, and trade-Greek tick types re-exported at the `thetadatadx` crate root so consumers naming return types of `HistoricalClient::*` methods do not need a second `tdbe` dependency: `thetadatadx::{GreeksEodTick, IndexPriceAtTimeTick, TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeGreeksImpliedVolatilityTick}`. Mirrors the same single-dep policy applied to `GreeksAllTick`, `EodTick`, etc.
- `FlatFilesConfig.max_attempts`, `FlatFilesConfig.initial_backoff_secs`, and `FlatFilesConfig.max_backoff_secs` per-field setters and getters bound across every binding. FFI: `thetadatadx_config_set_flatfiles_max_attempts(u32)` / `thetadatadx_config_get_flatfiles_max_attempts(*mut u32) -> i32` plus the two seconds pairs. C++: `thetadatadx::Config::set_flatfiles_max_attempts(uint32_t)` and the matching getter, plus the seconds pairs. Python: `Config.flatfiles_max_attempts`, `flatfiles_initial_backoff_secs`, `flatfiles_max_backoff_secs` getters and setters with `.pyi` rows. TypeScript napi: `Config.setFlatFilesMaxAttempts(number)` and `flatFilesMaxAttempts` getter plus the `InitialBackoffSecs` and `MaxBackoffSecs` pairs (BigInt on the two seconds fields). `Duration` fields cross the binding boundary as `u64` seconds.
- `RetryPolicy` per-field setters and getters bound across every binding. FFI: `thetadatadx_config_set_retry_initial_delay_ms(u64)` / `thetadatadx_config_get_retry_initial_delay_ms(*mut u64) -> i32` plus `_max_delay_ms`, `_max_attempts(u32)`, `_jitter(bool)`. C++: `thetadatadx::Config::set_retry_initial_delay_ms(uint64_t)` and matching getter plus the other three fields. Python: `Config.retry_initial_delay_ms`, `retry_max_delay_ms`, `retry_max_attempts`, `retry_jitter` getters and setters with `.pyi` rows. TypeScript napi: `Config.setRetryInitialDelayMs(bigint)` and `Config.retryInitialDelayMs` getter plus the other three fields. Method-shape helpers (`disabled()`, `delay_for_attempt()`, `capped_backoff()`) stay Rust-only.
- `ReconnectConfig.wait_ms` and `ReconnectConfig.wait_rate_limited_ms` fully wired into the streaming auto-reconnect path. Values flow from `DirectConfig.reconnect` into the auto-reconnect path and are consumed by the `ReconnectPolicy::Auto` arm. The prior built-in reconnect-delay defaults stay in place but caller overrides now take effect. FFI: `thetadatadx_config_set_reconnect_wait_ms(*mut ThetaDataDxConfig, u64)` / `thetadatadx_config_get_reconnect_wait_ms(*const ThetaDataDxConfig, *mut u64) -> i32` plus the `_rate_limited_ms` pair. C++, Python, and TypeScript napi mirror the surface.
- `RuntimeConfig` gains a worker-thread-count knob and a `RuntimeConfig::build_runtime()` helper that builds a multi-threaded async runtime honouring the configured worker count, with `Some(0)` clamped to `1`. The crate itself stays runtime-agnostic; the helper is the single source of truth that embedded bindings (FFI, Python, napi) consume when they own the runtime. Cross-binding surface mirrors the explicit `(has_value, n)` shape used by the historical worker-count knobs so the `Some(0)` sentinel survives the C boundary.
- `HistoricalConfig.warn_on_buffered_threshold_bytes` (Rust `usize`, default 100 MiB) bound across every binding. A buffered historical response whose estimated size exceeds the threshold logs a single warning with `endpoint`, `row_count`, `bytes_est`, and `threshold_bytes` fields suggesting `.stream(handler)` for the workload. Fires once per request after the rows materialise; set to `0` to disable.
- A historical decode path that separates decompression from protobuf decode so the two scale independently under load. Worker count and queue depth were independently tunable through the historical decode-thread and decode-queue-depth knobs (the worker count defaulting to the available CPU count, the queue depth to a multiple of the pool size); `Some(0)` clamps to 1 on both. The path applies back-pressure rather than dropping when the queue is full, and a decode-worker panic is contained: subsequent decodes fail with `Error::Transport { kind: DecoderPoisoned, .. }` instead of wedging the path. (This split decode path was later removed once inline decode measured faster.)
- A historical pool-sizing surface — the request-concurrency, decoder-thread, and decoder-ring-size knobs (all since removed) — exposed on every binding. A request-concurrency of `0` auto-detects from the resolved subscription tier; an explicit value above the tier cap is clamped at connect time with a logged warning, and request concurrency stays coupled to the resolved connection pool. A decoder-thread count of `0` auto-sizes from the available CPU count independent of connection count. The decoder-ring-size setters reject invalid sizes (zero, non-power-of-two, below 64) at the call site rather than at connect-time validation.
- Universal `.stream(handler)` method on every parsed historical builder. The buffered `.await -> Vec<T>` path held three live copies (HTTP/2 frames plus concatenated proto payload plus decoded `Vec<T>`) plus a `Vec::push` doubling transient, yielding 6x memory amplification on tick-interval responses. The new `.stream(handler)` decodes one chunk at a time, hands the slice to `handler`, then drops the chunk before the next is fetched. Peak resident memory drops to roughly one chunk regardless of total row count. The streaming variant is macro-emitted alongside the existing `IntoFuture` impl on every `parsed_endpoint!` builder. Python builders gain `.stream(handler)` / `.stream_async(handler)` terminals that hand each chunk to the user's callback as a typed `list[Tick]`; buffered `.await` / `.list()` / `.list_async()` paths remain unchanged for back-compat.
- Zero-copy frame handling on the historical streaming path. The previous design re-copied the receive buffer on every poll (an O(polls x buffer-size) memory tax); each frame is now detached without copying its bytes, cutting allocation and CPU on large server-streaming responses. Decode correctness is unchanged.
- Asyncio-native streaming surface `ThetaDataDx.streaming_async()` / `StreamingClient.streaming_async()` (PEP 703). The session wakes the asyncio loop only when events arrive: zero polling cost during quiet periods, one OS wake per coalesced batch. Companion `streaming_async_batches()` yields `pyarrow.RecordBatch` whose column buffers alias the decoded data directly (no per-event Python object construction on the read path). `queue_depth()` and `dropped_event_count()` getters expose the in-flight queue depth and dropped-event counters on both sessions.
- Free-threaded (PEP 703) wheels for CPython 3.13t and 3.14t. The extension leaves free-threading enabled after `import thetadatadx` on a free-threaded interpreter. Every blocking call on the unified client and on the standalone streaming and historical pyclasses waits without blocking other Python threads, so compute threads run in parallel; the parallel-throughput gate asserts under 1.8x overhead under contention on the free-threaded matrix entries.
- Hardened CI invariant suite. Gates cover: format, banned vocabulary, cross-binding parity (`sdks/parity.toml` matrix), extended-surfaces parity (`AuthConfig`, `MetricsConfig`), wire-schema validation, rustdoc, stubtest (`.pyi` against runtime), C-ABI bidirectional completeness, `cargo deny`, RustSec advisories, SAFETY-comment boilerplate detection, bench regression (threshold 25%), lockfile drift across the tracked Cargo.lock files for security-critical and SDK-owned crates, doc-consistency.
- `cargo-semver-checks` CI gate hard-fails (`continue-on-error: false`) with the v11.0.0 baseline. `[package.metadata.docs.rs]` switched from `all-features = true` to an explicit feature list (`arrow`, `polars`, `frames`, `config-file`) so the rendered docs.rs page no longer surfaces bench-only or test-only symbols.
- In-place connection-pool reconnect with bounded exponential backoff, so a long-running historical client no longer cascades after a transport fault. Every transport-level fault (a server-initiated close, IO failure, peer shutdown, or open-phase drop) reconnects the affected connection in place with bounded backoff (50 ms initial, 30 s cap, 8 attempts) rather than marking it permanently dead; the retry path re-dispatches once onto the fresh connection.
- Standalone `StreamingClient` and `HistoricalClient` pyclasses on the Python binding. `StreamingClient(creds, config)` opens only the streaming TLS transport (no historical gRPC channel, no Nexus HTTP auth); `HistoricalClient(creds, config)` opens only the historical gRPC channel plus Nexus authentication and surfaces the historical / FLATFILES API while raising `AttributeError` on every streaming-touching method. The bundled `Client` keeps its current behaviour; the new classes align the Python surface with the standalone clients already exposed by the C ABI (`thetadatadx_streaming_*` / `thetadatadx_client_*`) and the C++ wrapper (`thetadatadx::StreamingClient` / `thetadatadx::Client`).
- gRPC `grpc-timeout` header emitted on every deadline-bearing RPC (`server_streaming_with_deadline`, `mdds_client.with_deadline(...)`). The header carries the smallest unit that fits the budget per the gRPC HTTP/2 spec (`n` / `u` / `m` / `S` / `M` / `H`) so the server can short-circuit on deadline elapse instead of completing work the client will discard.
- `TransportErrorKind` exported from `thetadatadx::error` for binding consumers and retry classifiers.
- `tdbe::types::price::Price::with_value_and_type` constructor rejects out-of-range `price_type` values with a typed `PriceError` instead of clamping. `Price::value()` and `Price::price_type()` accessors join the public API so future field visibility changes do not break call sites. `Price::is_unset()` and `Price::is_zero_value()` split the previous `is_zero()` conflation into the two distinct signals (sentinel vs real zero). `Price` POW10 indexing paths carry `debug_assert!` invariant guards.
- Renamed streaming event payload type from `Contract` to `ContractRef`. `event.contract` now returns `ContractRef` (the read-only event payload accessor) without colliding with the fluent `Contract` builder used in `subscribe()` inputs.
- `BackpressurePolicy` enum on the streaming pull-iter and async surfaces. Variants: `Block` (stalls the event-queue consumer until the queue drains), `DropOldest` (evicts the queue head on full, preserves recency), `DropNewest` (skips the new event on full, preserves history; legacy behaviour). Exposed in Python as `thetadatadx.BackpressurePolicy` and threaded through `streaming_iter(max_queue_depth=, backpressure=)`, `streaming_async(max_queue_depth=, backpressure=)`, and `streaming_async_batches(max_queue_depth=, backpressure=)`.
- `AuthConfig` (`nexus_url`, `client_type`) and `MetricsConfig` (`port`) per-field setters and getters bound across every binding. FFI: `thetadatadx_config_set_nexus_url(*const c_char) -> i32` / `thetadatadx_config_get_nexus_url() -> *mut c_char` (the owned string is released with `thetadatadx_string_free`) plus the identical `_client_type` pair, and `thetadatadx_config_set_metrics_port(bool has_value, u16) -> i32` / `thetadatadx_config_get_metrics_port(*mut bool, *mut u16) -> i32` carrying the `Option<u16>` as the widened `(has_value, port)` shape. C++: `thetadatadx::Config::set_nexus_url(const std::string&)` / `get_nexus_url()` plus the `client_type` pair and `set_metrics_port(std::optional<uint16_t>)` / `get_metrics_port()`. Python: `Config.nexus_url`, `client_type`, and `metrics_port` (`Optional[int]`) getters and setters with `.pyi` rows. TypeScript napi: `Config.setNexusUrl(string)` / `nexusUrl`, `setClientType(string)` / `clientType`, and `setMetricsPort(number | null)` / `metricsPort`. The two `AuthConfig` fields are `String`; `MetricsConfig.port` is `Option<u16>` where the `None` sentinel disables the Prometheus exporter.
- `StreamingClient::connect` rejects a non-power-of-two `ring_size` with `Error::Config` rather than silently rounding to the next power of two; default configs (`131_072`) are unchanged and still valid.
- Streaming control events surface as typed-per-variant classes across Python, TypeScript, and the C / C++ FFI surface, mirroring the Rust `StreamControl` enum one-for-one. Python dispatch via `match event: case LoginSuccess(permissions=p): ... case Disconnected(reason=r): ...`; TypeScript via the discriminated union's `kind` field; C consumers via `event->kind` into the matching `event-><variant>` payload; C++ uses re-exported `thetadatadx::Stream<Variant>` aliases. Schema bumped to version 5 (`fpss_event_schema.toml`).
- A historical tier-clamp override knob (Rust `bool`, default `false`, since removed) that bypassed the connect-time clamp of request concurrency to the resolved subscription-tier cap. Rust-only by design (no binding setters); intended for exercising the over-provisioning path against a stubbed authentication response, not production use.
- `StreamingClient::connect_consumer` returns the client paired with an event-poller whose `run` loop drives the streaming event queue on the caller's own thread. No consumer thread is spawned and no intermediate queue is allocated, so an embedded Rust consumer drains the single SDK event queue directly with a zero-copy borrow per event. The poller's `poll_batch` adds a non-blocking single-batch drain returning a `PollOutcome` for callers that integrate the drive into their own loop. The existing push-callback (`StreamingClient::connect`) and pull-iterator (`StreamingClient::connect_iter`) delivery modes are unchanged; the I/O reader, reconnect, and re-subscribe paths are shared across all three. Rust-only surface.

### Changed

- The historical connection pool reconnects in place on a transport close rather than marking connections permanently dead, so long-running clients no longer cascade after sustained load.
- Historical reconnect backoff gains decorrelated +/- 10% jitter keyed on `(host, port, attempt)` so a population of clients seeing the same server-initiated close no longer reconnects in lock-step.
- Account subscription scope and the auth-routing URL are no longer recorded at the default log level, so production deployments do not log account permissions or auth topology unless trace logging is explicitly enabled.
- Vendor-neutral terminology across user-facing surfaces. C++ public headers (`thetadx.h`, `thetadx.hpp`) section banners and prose comments substituted internal protocol jargon ("FPSS client" / "FPSS handle" become "streaming client" / "streaming handle"; "MDDS retry policy" becomes "historical-channel retry policy"; ABI-locked symbol names stay unchanged). Python pyo3 docstrings rewritten analogously. The docs-site streaming pages (`docs-site/docs/streaming/{index,connection,events,latency,reconnection}.md`, `docs-site/docs/getting-started/streaming.md`, `docs-site/docs/migration/v9-to-v10.md`) substitute bare `FPSS` for `streaming`, bare `MDDS` for `historical channel`, and replace internal queue-implementation jargon with the neutral "bounded event queue" wording. Mermaid diagrams updated.
- REST endpoint builders are now generated from `endpoint_surface.toml` rather than hand-written. Adding a REST endpoint is now a single TOML change (`transport = "both"`) instead of per-endpoint boilerplate. The generated builders are byte-identical to the previous hand-written ones and the public API is unchanged.
- Every historical builder ships a "When to use `.await` vs `.stream(handler)`" decision matrix in its rustdoc, attached automatically after the per-endpoint description.
- `crate::decode::row_date` accepts `Text` cells in addition to `Number` and `Timestamp`, routing them through `parse_iso_date`. Every existing call site is strictly more permissive; the new arm unblocks `InterestRateTick` decoding of the ISO `created` column.
- `interest_rate_history_eod` rustdoc lists the 12 valid `symbol` values from upstream `RateType` (`SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`, `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`, `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`). The wire signature stays `&str` so a future server-side maturity addition does not break SDK callers.
- The lenient 6/11/12-field NBBO decoder is kept as a defensive measure: `find_header` plus `opt_number(row, None) -> 0` is good API design regardless of cause; a subset NBBO layout could surface from any upstream storage tier in the future. The test `quote_tick_decodes_legacy_six_field_shape_with_zero_fill` remains as a regression pin.
- The REST transport (`crate::rest`) is kept as the user-facing alternative transport reachable via `FallbackPolicy::RestAlways`.
- `_with_fallback` shims accept `(start_date, end_date)` instead of a single `date` on all four affected endpoints (`option_history_quote`, `option_history_trade_quote`, `option_history_greeks_implied_volatility`, `option_history_greeks_first_order`). The `RestAlwaysForDateRange` policy (now removed; see Breaking changes) keyed on `start_date`.
- Per-base-url `RestClient` cache. The four `_with_fallback` shims share an `Arc<RestClient>` per distinct base URL via a lazily-initialised `OnceLock<RwLock<HashMap<String, Arc<RestClient>>>>` on `Client`, eliminating the per-call `reqwest::Client` construction (TLS + connection-pool init) the previous shape paid.
- `RestClient::with_max_response_bytes` cap with default 256 MiB (`DEFAULT_MAX_RESPONSE_BYTES`). `fetch_csv` rejects oversized responses via `Content-Length` pre-flight and a streamed chunk-count check; surfaces `RestError::ResponseTooLarge { size, limit }`.
- `rest::csv::cell_i32_or_zero` and `cell_f64_or_zero` distinguish three input cases: column absent (`0`, legacy fill), empty cell (`0`, Terminal null), malformed non-empty cell (structured `CsvDecode` error). `cell_f64_or_zero` additionally rejects `NaN` and `+/-Inf` so Rust's permissive `f64::from_str` does not silently poison downstream comparisons.
- `config::fallback::DEFAULT_REST_BASE_URL` re-exports `rest::client::DEFAULT_TERMINAL_BASE_URL` rather than inlining the literal; single source of truth for the Terminal default URL.
- `decode_*_csv` helpers validate required columns (`ms_of_day`, `date`) before allocating the `Vec::with_capacity(rows.len())` output buffer; surfaces a `MissingColumn` error before potentially-large allocations on malformed bodies.
- `decode_greeks_first_order_csv` hoists all 13 `column_index` calls above the row loop.
- Historical `extract_text_column`, `extract_number_column`, and `extract_price_column` resolve headers through the alias-aware `headers::find_header` helper. Upstream column renames (e.g. `symbol` to `root`, `timestamp` to `ms_of_day`) now resolve through `HEADER_ALIASES` instead of returning a silent empty `Vec`. A non-empty `DataTable` whose column cannot be resolved emits a `warn` log naming the requested header and the available set.
- `HistoricalClient::open_channel_pool` routes `ChannelError -> Error` through the canonical `From<ChannelError> for Error` impl, dropping a hand-mapped duplicate `match` arm at the connect site. The channel-index context (`"channel {idx}: ..."`) is preserved on the `Transport`-shaped output.
- Workspace clippy invocation widened to `cargo clippy --workspace --all-targets --locked -- -D warnings` so `[[bench]]` and `#[cfg(test)]` unsafe blocks travel through `clippy::undocumented_unsafe_blocks` on every PR.
- The decode pipeline's back-pressure path no longer hangs forever if every decode worker panics while the producer is already blocked on a full queue. The producer now wakes on a 50 ms cadence to re-check for a poisoned pool, so a worker panic mid-wait surfaces within one slice instead of wedging the pipeline.
- The streaming reader thread uses a non-blocking enqueue on every publish path (handshake control frames, login success, data frames, disconnect emission, reconnect control frames). A saturated queue no longer wedges the TLS reader; overflow increments the shared `dropped` counter and emits a `warn` log.
- The streaming auto-reconnect re-subscribe path allocates fresh `req_id` values from the shared `next_req_id` counter instead of emitting `-1`. Server-side `ReqResponse` events on the reconnected session now carry ids correlatable to the original subscribe.
- `Contract::option(symbol, expiration, strike, right)` exposes the explicit four-argument signature; the wire-format integer-triple constructor is exposed as `Contract::option_raw(symbol, expiration, is_call, strike_raw)`. The `IntoOptionSpec` sealed trait is removed.
- `StreamingConfig` tuning knobs `timeout_ms`, `connect_timeout_ms`, and `ping_interval_ms` are wired into the runtime. Values flow through to the connection (TCP `connect_timeout`), framing (mid-frame stall budget + I/O loop overall deadline), and ping-heartbeat layers, and validate their range at config-load time: `timeout_ms` `[100, 60_000]`, `connect_timeout_ms` `[1_000, 60_000]`, `ping_interval_ms` `[100, 300_000]`. `DirectConfig::validate` returns `Result<Self, Error>`; the production / dev / stage presets remain infallible by construction.
- The historical REST-fallback path no longer logs per-call `start_date` / `end_date` at the default log level, so routine operator logs no longer echo request date ranges.
- `interval` millisecond-shorthand normalization realigned to the upstream preset vocabulary. The `"0"` sentinel now resolves to `"tick"` (every event) instead of the prior silent `"100ms"`, and the `1..=10` ms band resolves to `"10ms"` (previously folded into `"100ms"`). Callers that relied on `"0"` meaning `"100ms"` must pass an explicit preset.
- Dynamic endpoint invocation (`EndpointArgs`, used by the CLI and MCP surfaces) strictly validates the `interval` argument against the upstream preset set (`tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`) or an all-digit millisecond shorthand. Non-preset strings such as `"1minute"` — previously accepted by the looser alphanumeric check — now surface a typed `EndpointError::InvalidParams` naming the accepted values before any request dispatches.
- The `stock_list_symbols/in_house` benchmark is excluded from the hard bench-regression gate (it is a network-bound gRPC round-trip whose variance exceeds the threshold by construction); it still runs for information. CPU microbenches remain gated. (#614)

### Fixed

- Strict decode rejection of out-of-range `price_type` on every wire boundary. Frames carrying `price_type` outside `0..=19` now surface as decode errors instead of producing silently-clamped magnitudes. Coverage spans historical row helpers, column extractors (`extract_price_column`), the EOD `Price` cell render, streaming frame decode, and the flatfiles row renderer.
- Strict decode rejection of calendar-invalid dates and clock-invalid times on both text and numeric arms. Numeric `YYYYMMDD` wire integers that fall outside the valid Gregorian calendar (including out-of-range months, days, and non-leap February 29) are rejected via the canonical validator rather than silently zero-filling. Malformed text dates and times propagate `DecodeError` instead of substituting `0`.
- Strict bounds-check on wire `int64 -> i32` narrowing for `ms_of_day`, `sequence`, `size`, `exchange`, bid/ask sizes, `open_interest`, and EOD integer fields. Wire values outside `i32` range now surface as a decode error instead of wrapping silently.
- Strict validation of contract identity fields on flatfile INDEX entries. Numeric `right` rejects values outside ASCII `'C'` (67) and `'P'` (80); `expiration` and trailing `date` reject calendar-invalid YYYYMMDD via the canonical Gregorian validator before the row is rebuilt.
- Unknown enum text on `right` and calendar-type fields is rejected rather than silently coerced to zero.
- Numeric date arms that would overflow `i32` are rejected before Gregorian validation.
- Generator-emitted `contract_id` expiration values are validated against the Gregorian calendar.
- Flatfile server: scratch path is now unique per request with an atomic rename on completion, closing a concurrent-write race that could surface partial bytes to a reader.
- Removed the speculative numeric status arm in `parse_calendar_days_v3`; upstream uses text-only status, and the numeric arm could mask real decode failures.
- `flatfiles_byte_match` test hard-fails on a missing fixture once `THETADATADX_FLATFILE_FIXTURES_PATH` is set, replacing the prior silent skip that defeated the opt-in flag.
- `InterestRateTick` schema corrected to two columns (`created` as ISO-date Text, `rate` as percent Number). The pre-v11 decoder errored `column 0: expected Number|Timestamp, got Text` on every `interest_rate_history_eod` call. Verified against the live server and `docs.thetadata.us/operations/interest_rate_history_eod.html`. Cross-binding regression coverage in `tests/test_interest_rate_schema.rs`, `sdks/python/tests/test_interest_rate.py`, `sdks/typescript/__tests__/interest_rate.test.mjs`, and `sdks/cpp/tests/interest_rate.cpp`.
- `next_req_id` widened from `AtomicI32` to `AtomicI64` with a `wire_req_id` clamp at every wire-boundary call site. The previous 32-bit counter could wrap into the wire protocol's `-1` "uncorrelated" sentinel after roughly 2^31 allocations (~5 days at 5k subs/sec). The clamp masks the sign bit (`x & 0x7FFF_FFFF`) so wire ids stay strictly non-negative even past `i32::MAX`.
- Per-frame `received_at_ns` cast uses saturating `u64::try_from` so the schema timestamp stops wrapping at the 2554 boundary.
- Out-of-range subscription-tier wire bytes (negative, `> 3`, `i32::MAX`) now fold to `Free` and log a warning instead of panicking.
- Per-tick metric lookups on the streaming decode hot path are cached rather than re-resolved per event, dropping the per-call observability cost from ~30 ns to ~5 ns.
- Rate-limited the "no contract for ID" warning at 1024 emissions to match the existing slow-callback and clock-skew cadence. A server-side replay-boundary anomaly that ticked unrecognised ids previously logged one line per tick and crowded out genuinely diagnostic warnings.
- Python `thetadatadx.__version__` resolves through `importlib.metadata.version("thetadatadx")` (PEP 396). The attribute was missing on the top-level package, breaking `pip show`, downstream version-pinning, and environment snapshot scripts.
- A dropped historical connection now aborts its background connection-driver task, so under repeated connect/disconnect cycles a parked connection can no longer outlive its owner for an unbounded interval.
- A timeout-backoff multiplier on the streaming path is now overflow-safe, so an extreme `max_consecutive_timeouts` setting cannot wrap the backoff diagnostic.
- The per-disconnect metric label no longer allocates a string per disconnect.
- The streaming `ring_size` doc comment is corrected: the documented per-event footprint figure was overstated, so the per-`StreamingClient` event-queue footprint at the default `ring_size = 4096` is now documented accurately as approx 384 KiB.
- Python `StreamingAsyncSession.__aexit__` and `StreamingAsyncBatchesSession.__aexit__` close the asyncio read-end FD unconditionally even when `event_loop.remove_reader` raises (e.g. event loop closed mid-shutdown, FD already unregistered). The previous code propagated the `remove_reader` error via `?` before the close path, so a shutdown-race permanently leaked the pipe read-end because `self.closed` short-circuited re-entry. The error is now captured, the FD is reclaimed, and the captured error is re-raised so callers still see the underlying fault.
- `tdbe::types::price::Price::Display` widens the mantissa to `i64` before negating, so formatting a `Price` whose `value` is `i32::MIN` no longer overflows. Debug builds previously raised `attempt to negate with overflow` and release builds wrapped silently; both now render the correct magnitude. Tracking: [#609](https://github.com/userFRM/ThetaDataDx/issues/609).

### Removed

- `Error::config_other`, `Error::decode_other`, `Error::decompress_other`, and the `Other(String)` variants on `ConfigErrorKind`, `DecodeErrorKind`, `DecompressErrorKind`. See Breaking changes.
- The historical `decoder_threads` field. See Breaking changes.
- TypeScript `Config.flatfileToPath` lowercase alias. See Breaking changes.
- `InterestRateTick.ms_of_day` field. See Breaking changes.
- `Channel::is_dead`, `Channel::mark_dead`, `Channel::dead_handle`, `ChannelPool::all_dead`, `ChannelPool::dead_count`. See Breaking changes.
- `ChannelError::UpstreamCascade`. See Breaking changes.
- `FallbackPolicy::RestOnH2Disconnect` and `FallbackPolicy::RestAlwaysForDateRange`. See Breaking changes.

### Known issues

- `tdbe::time::timestamp_to_ms_of_day` and `timestamp_to_date` perform a `u64 -> i64` cast on the wire timestamp without a bounds check; values above `i64::MAX` wrap silently. Vendor wire payloads do not emit values in that range, but a fixture or fuzzer can hit it. Deferred to v12 because the fix touches `tdbe` crate source which is outside the v11.0.0 branch scope.

## [10.0.0] - 2026-05-09

**In-house gRPC transport** replaces the third-party gRPC stack on the
historical server-streaming path. The SDK now drives HTTP/2 directly:
protobuf encode → length-prefix frame → HTTP/2 DATA → response stream →
trailers parse, with no intermediary middleware stack, no boxed
response bodies, and no dynamic-dispatch trait indirection. New public
module `thetadatadx::grpc::*` exposes `Channel`, `ChannelPool`,
`ChannelLease`, `ServerStreaming`, `Codec`, `Status`, `DecoderPool`,
`DecoderHandle` and the matching error types (`ChannelError`,
`CodecError`, `StatusParseError`, `DecoderPoolError`,
`DecoderSubmitError`).

Semver-honest version bump for the v9.1.0 surface. The v9.0.x →
v9.1.0 wave introduced 12 major API breaks per
`cargo-semver-checks` (subscribe_*-family removal, polymorphic
`subscribe(spec)`, `Contract::option` arity change, `Error` enum
reshape, `StreamData::*` `contract_id` removal in favour of typed
`Arc<Contract>`, `Client` → `Client` rename,
`mdds::decode::v3` → `mdds::decode::dual_type_columns` module
rename, `IntoOptionSpec` trait removal, streaming connection-parameter
additions, the streaming-config `queue_depth` removal, flat
`ThetaDataDxStreamControl` → typed per-variant structs, Go SDK removal).
Rust semver classifies that diff as a major bump.

### Changed

- **In-house gRPC transport** replaces the third-party gRPC stack; full module surface
  (`Channel`, `ChannelPool`, `ChannelLease`, `ServerStreaming`,
  `Codec`, `Status`, `DecoderPool`, `DecoderHandle`,
  `ChannelError`, `CodecError`, `StatusParseError`,
  `DecoderPoolError`, `DecoderSubmitError`).
- `Error::Transport` payload changed from the third-party transport
  error type to `String` (later restructured to typed `{ kind, message }` —
  see `[Unreleased]`).
- `ChannelPool::next()` returns a `ChannelLease<'a>` instead of
  `&'a Channel`. The lease pre-reserves an in-flight slot on the
  picked channel synchronously so concurrent burst dispatches
  observe each reservation immediately and route around loaded
  channels.
- `Status::from_trailers` tolerates malformed `grpc-message`
  trailers per the gRPC HTTP/2 spec. The parser percent-decodes
  (RFC 3986, `%HH` escapes only); if decoded bytes are valid UTF-8
  they become the message, otherwise the raw header bytes fall
  back to UTF-8 interpretation, with an empty message for opaque
  non-UTF-8 inputs.
- Project version: 9.1.0 → 10.0.0 across `thetadatadx`,
  `tdbe` dependents, `ffi`, `tools/{cli,mcp,server}`,
  `sdks/{python,typescript}`. tdbe stays at 0.13.1 (no API change
  in this bump). All standalone Cargo.lock files re-locked.

### Added (in-house gRPC)

- The historical decoder-thread and decoder-ring-size knobs (since
  removed) control the dedicated decoder pool that runs zstd decompress +
  protobuf decode off the async I/O path. A decoder-thread count of `0`
  auto-sizes from the channel count and the available CPU count;
  the decoder ring size must be a power of two `>= 64`.
- `DecoderHandle::submit` returns
  `Result<oneshot::Receiver<DecodeResult>, DecoderSubmitError>`.
  Submits made after a worker-thread panic poisoned the pool fail
  fast with `DecoderSubmitError::Poisoned` rather than parking the
  caller on a dead consumer queue.
- `ChannelError` variant routing: connection-level HTTP/2 failures —
  `GOAWAY` (either direction), IO failure on the HTTP/2 transport, peer
  shutdown, and open-phase connection drops (failures observed on
  `ready()` / `send_request()` / `send_data()`) — surface as
  `ChannelError::ConnectionClosed`. `ChannelError::H2Stream` is
  scoped strictly to per-stream `RST_STREAM` (any reason code) and
  HTTP/2 stream-scoped protocol errors.

### Removed (in-house gRPC)

- The third-party gRPC dependency is removed. The `inhouse-grpc`
  feature flag is also gone — the in-house transport is the only path.
  Direct uses of the third-party stack's channel, status, or
  streaming types through `thetadatadx` re-exports are no longer
  available.
- `HistoricalClient::stub` was removed. Internal call sites now reach the
  generated stubs through `proto::beta_theta_terminal::*` directly.
- `GrpcStatusKind::from_code()` was renamed to
  `GrpcStatusKind::from_u32()` to match the wire type. The enum
  `repr` is now `u32` (was `i32`).
- `StatusParseError::MessageNotUtf8` was removed. Malformed
  `grpc-message` no longer fails the trailers parse; exhaustive
  matches on `StatusParseError` need to drop the variant.

### Migration (in-house gRPC)

- Replace `Error::Transport(msg)` pattern with the typed shape:
  `Error::Transport { kind, message }`. The `Display` shape stays
  `transport error (<kind>): <message>` for legacy string-keyed
  consumers.
- Replace `GrpcStatusKind::from_code(n)` with
  `GrpcStatusKind::from_u32(n)`.
- When constructing the historical config field-by-field, set the
  decoder-thread and decoder-ring-size knobs (or call the historical
  config's production-defaults constructor).
- Update `match` arms on `StatusParseError` — drop the
  `MessageNotUtf8` branch.
- `pool.next()` callers that bind the result across an `await`
  must keep the lease alive for the dispatch window. Pattern:
  `let lease = pool.next(); stub_fn(&lease, req).await?;`.
- `DecoderHandle::submit` returns `Result<_, DecoderSubmitError>`.
  Update callers from `let rx = handle.submit(r); rx.await` to
  `let rx = handle.submit(r)?; rx.await`.

### Migration from v9.1.0

No source-level changes required. Update the version pin:

| Surface | v9.1.0 | v10.0.0 |
|---|---|---|
| `Cargo.toml` | `thetadatadx = "9"` | `thetadatadx = "10"` |
| `pyproject.toml` / `requirements.txt` | `thetadatadx>=9.1.0,<10` | `thetadatadx>=10.0.0,<11` |
| `package.json` | `"thetadatadx": "^9.1.0"` | `"thetadatadx": "^10.0.0"` |
| C++ pin | `cargo build --release -p thetadatadx-ffi` from `v9.1.0` tag | `v10.0.0` tag |


### Added

- Fluent contract-first streaming API. `Contract::stock("AAPL")`,
  `Contract::option("SPY", "20260620", "550", "C")`, and the
  `contract.quote()` / `.trade()` / `.open_interest()` methods return
  a typed `Subscription` value. Full-stream subscriptions come from
  `SecType::Option.full_trades()` /
  `SecType::Option.full_open_interest()`. The new polymorphic
  `client.subscribe(Subscription)`, `client.subscribe_many([...])`,
  `client.unsubscribe(Subscription)`, and
  `client.unsubscribe_many([...])` on `Client` (Rust),
  `Client` (Python pyclass), `Client` (TypeScript
  napi), and `thetadatadx::UnifiedClient` / `thetadatadx::StreamingClient` (C++) accept
  that value type directly.
- Polymorphic C ABI: new `thetadatadx_client_subscribe` /
  `thetadatadx_client_unsubscribe` / `thetadatadx_streaming_subscribe` /
  `thetadatadx_streaming_unsubscribe` take a `ThetaDataDxSubscriptionRequest` payload; one
  entry point handles every per-contract or full-stream variant.
- `AsyncClient` Python class — async-only sibling of
  `Client`. Attribute access is restricted to `*_async`
  historical methods plus the streaming lifecycle helpers; the
  synchronous historical surface raises `AttributeError` so callers
  that opt into the async path do not accidentally block on a sync
  method.
- `thetadatadx::prelude` Rust module — re-exports `Credentials`,
  `Client`, `Contract`, `Subscription`, `SecTypeExt`,
  `SecType`, etc. for a one-import fluent path.

### Changed

- Public client name: previous unified-client struct name is gone
  (no alias, no compat shim). Every binding ships only
  `Client` (Rust struct, Python pyclass, TypeScript napi
  class).
- Python streaming: `client.streaming(on_event)` context manager is
  the recommended path; the bound session forwards every public
  `Client` method through `__getattr__`, so the new
  polymorphic `subscribe` / `unsubscribe` are reachable on the
  session with zero hand-listed mirror.

### Removed

Hard break — the typed subscribe / unsubscribe surface is gone.
Every typed `subscribe_*` / `unsubscribe_*` and `subscribe_option_*`
entry on the public client (Rust, Python, TypeScript, C++) plus the
matching typed C ABI entry points (`thetadatadx_client_subscribe_*`,
`thetadatadx_streaming_subscribe_*`, `thetadatadx_client_unsubscribe_*`,
`thetadatadx_streaming_unsubscribe_*`, including the option-overload variants)
have been deleted. Replacement is the polymorphic
`subscribe(Subscription)` / `unsubscribe(Subscription)` /
`subscribe_many([...])` / `unsubscribe_many([...])` paths.

Migration map (documentation only — no compat layer ships):

| Removed | Wave K replacement |
|---|---|
| Rust: `client.subscribe_quotes(&c)` | `client.subscribe(c.quote())` |
| Rust: `client.subscribe_trades(&c)` | `client.subscribe(c.trade())` |
| Rust: `client.subscribe_open_interest(&c)` | `client.subscribe(c.open_interest())` |
| Rust: `client.subscribe_full_trades(SecType::Option)` | `client.subscribe(SecType::Option.full_trades())` |
| Rust: `client.subscribe_full_open_interest(SecType::Option)` | `client.subscribe(SecType::Option.full_open_interest())` |
| Rust: `client.subscribe_all(&c)` (quotes + trades batcher) | `client.subscribe_many(vec![c.quote(), c.trade()])` |
| Python: `client.subscribe_quotes("AAPL")` | `client.subscribe(Contract.stock("AAPL").quote())` |
| Python: `client.subscribe_option_trades("SPY", e, k, r)` | `client.subscribe(Contract.option("SPY", expiration=e, strike=k, right=r).trade())` |
| Python: `client.subscribe_full_trades("OPTION")` | `client.subscribe(SecType.OPTION.full_trades())` |
| TS: `client.subscribeQuotes("AAPL")` | `client.subscribe(ContractRef.stock("AAPL").quote())` |
| TS: `client.subscribeFullTrades("OPTION")` | `client.subscribe(SecType.option().fullTrades())` |
| C ABI: `thetadatadx_client_subscribe_quotes(h, sym)` | `thetadatadx_client_subscribe(h, &ThetaDataDxSubscriptionRequest{...})` |
| C ABI: every `thetadatadx_*_subscribe_*` / `thetadatadx_*_unsubscribe_*` typed entry point | `thetadatadx_*_subscribe` / `thetadatadx_*_unsubscribe` (polymorphic) |
| C++: `fpss.subscribe_quotes("AAPL")` | `fpss.subscribe(thetadatadx::Contract::stock("AAPL").quote())` |

## [9.1.0] - 2026-05-07

Single-queue SSOT for the streaming pipeline (closes #513). The
prior topology composed two queues — the lock-free event queue plus a
second bounded channel of 8192 slots — with a per-tick
`StreamEvent::clone` between them. The `start_streaming` path now
invokes the user callback directly from the event-delivery thread.

### Migration from v9.0.x

The flat `ThetaDataDxStreamControl { kind, id, detail }` C ABI envelope is
replaced by one typed struct per `StreamControl::*` Rust
variant. Old code dispatched on `event.control.kind` then read
`event.control.id` / `event.control.detail`; new code dispatches on
`event.kind` then reads the matching `event.<variant>` payload.
Field-by-field mapping for every control variant:

| v9.0.x (flat envelope) | v9.1.0 (typed struct) |
|---|---|
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 0; control.detail` | `kind == THETADATADX_FPSS_LOGIN_SUCCESS; login_success.permissions` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 1; control.id, control.detail` | `kind == THETADATADX_FPSS_CONTRACT_ASSIGNED; contract_assigned.id, contract_assigned.contract` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 2; control.id, control.detail` | `kind == THETADATADX_FPSS_REQ_RESPONSE; req_response.req_id, req_response.result` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 3` | `kind == THETADATADX_FPSS_MARKET_OPEN` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 4` | `kind == THETADATADX_FPSS_MARKET_CLOSE` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 5; control.detail` | `kind == THETADATADX_FPSS_SERVER_ERROR; server_error.message` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 6; control.detail (formatted)` | `kind == THETADATADX_FPSS_DISCONNECTED; disconnected.reason` (i32 RemoveReason) |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 8; control.id, control.detail` | `kind == THETADATADX_FPSS_RECONNECTING; reconnecting.reason, reconnecting.attempt, reconnecting.delay_ms` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 9` | `kind == THETADATADX_FPSS_RECONNECTED` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 10; control.detail` | `kind == THETADATADX_FPSS_ERROR; error.message` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 11; control.id, control.detail (hex)` | `kind == THETADATADX_FPSS_UNKNOWN_FRAME; unknown_frame.code, unknown_frame.payload, unknown_frame.payload_len` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 12` | `kind == THETADATADX_FPSS_UNKNOWN_CONTROL` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 13` | `kind == THETADATADX_FPSS_CONNECTED` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 14; control.detail (hex)` | `kind == THETADATADX_FPSS_PING; ping.payload, ping.payload_len` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 15` | `kind == THETADATADX_FPSS_RECONNECTED_SERVER` |
| `kind == THETADATADX_FPSS_CONTROL && control.kind == 16` | `kind == THETADATADX_FPSS_RESTART` |

Field-by-field mapping for the data-variant `contract_id` removal
and the hidden internal-only `RawData` / `Empty` variants:

| v9.0.x | v9.1.0 |
|---|---|
| `event.quote.contract_id` (i32, wire-internal) | `event.quote.contract.symbol` (and `expiration` / `strike` / `is_call` for options) — same for `trade`, `open_interest`, `ohlcvc` |
| Rust: `StreamData::Quote { contract_id, contract, .. }` | `StreamData::Quote { contract, .. }` (id removed) |
| Python: `event.contract_id` | `event.contract.symbol` |
| TypeScript: `event.quote.contract_id` | `event.quote.contract.symbol` |
| C: `event.quote.contract_id` | `event.quote.contract.symbol` (NUL-terminated, may be null pre-`ContractAssigned`) |
| C++: `event.quote.contract_id` | `event.quote.contract.symbol` |
| `StreamEvent::RawData { code, payload }` matched on user callback | Removed; truncated FIT frames bump `thetadatadx.fpss.decode_failures` and never reach the callback. Unrecognised wire codes still surface as `StreamControl::UnknownFrame { code, payload }` (typed control variant). |
| `StreamEvent::Empty` queue-slot placeholder visible to user code | Removed; queue slots use a crate-private internal placeholder, filtered before user delivery. |

Numeric values of `ThetaDataDxStreamEventKind` renumber alphabetically; reach
for the symbolic names (`THETADATADX_STREAM_LOGIN_SUCCESS` in C,
`FpssLoginSuccessEvent` in Go) — they are stable across the rename.
C++ consumers using `thetadatadx::Stream<Variant>` aliases get the same
borrowed-pointer ownership rules as before: pointers are valid only
for the duration of the user callback. Python and TypeScript consumer
code does not change.

### Changed

- Streaming control events surface as typed-per-variant classes across
  every language binding — Python, TypeScript, AND the C / C++ / Go
  FFI surface — mirroring the Rust `StreamControl` enum one-for-one.
  Replaces the previous flattened `Simple` event type and the flat
  `ThetaDataDxStreamControl { kind, id, detail }` C ABI. Python users dispatch
  via `match event: case LoginSuccess(permissions=p): ... case
  Disconnected(reason=r): ...`; TypeScript users dispatch via the
  discriminated union's `kind` field with one typed payload per
  variant (`event.loginSuccess`, `event.disconnected`,
  `event.reconnecting`, ...). C consumers dispatch via `event->kind`
  into the matching `event-><variant>` payload
  (`event->login_success.permissions`, `event->disconnected.reason`,
  `event->reconnecting.{reason, attempt, delay_ms}`, ...). C++
  consumers read the same fields through the re-exported
  `thetadatadx::Stream<Variant>` aliases; Go consumers read the matching
  `event.<Variant>` pointer (`event.LoginSuccess.Permissions`,
  `event.Disconnected.Reason`, ...). The `ThetaDataDxStreamEventKind` enum
  gains one discriminant per control variant — numeric values
  renumber alphabetically; symbolic names
  (`THETADATADX_STREAM_LOGIN_SUCCESS`, `FpssLoginSuccessEvent`, etc.) are
  stable. Schema bumped to version 5
  (`fpss_event_schema.toml`); generated outputs
  regenerated; codegen idempotency check enforced in CI. See the
  v9.0.x → v9.1.0 migration table for the old→new field mapping.
- `ThetaDataDx::start_streaming` now invokes the user callback directly
  from the event-delivery thread, with each invocation panic-isolated.
  There is exactly ONE queue between the TLS reader and the user
  callback (the bounded event queue); the per-tick `StreamEvent::clone`
  shim on the client is gone.
- `dropped_event_count()` keeps the same public signature but now
  reports non-blocking-enqueue failures (queue overflow when the
  consumer falls behind) instead of bounded-channel-full rejections.
- The TLS reader uses a non-blocking enqueue for every data event so
  a slow user callback can never block the reader. Handshake-time
  control frames (`Connected`, `Ping`, `LoginSuccess`, `Reconnecting`,
  `Disconnected`) keep the original blocking-publish semantics so
  wire-order ordering relative to `LoginSuccess` is preserved.
- `thetadatadx_client_free` and `thetadatadx_streaming_free` now apply the drain barrier
  internally before destroying the handle. `_free` calls the equivalent
  of `stop_streaming` (or `shutdown` for streaming) and then polls the
  drain flag with a 5-second timeout; on overrun it logs an
  error and proceeds. Callers no longer need to call
  `_await_drain` before `_free` to keep the callback `ctx` alive.
  The C++ wrapper's `StreamingClient` move-assign now invokes
  `thetadatadx_streaming_await_drain` between `thetadatadx_streaming_shutdown` and releasing the
  staged `std::function` storage, closing an analogous use-after-free
  window.
- The `StreamingConfig` tuning knobs `timeout_ms`, `connect_timeout_ms`,
  and `ping_interval_ms` are now wired into the runtime. Previous
  releases shipped these as no-op fields whose values were ignored —
  the streaming pipeline used hardcoded protocol-level constants
  (`READ_TIMEOUT_MS`, `CONNECT_TIMEOUT_MS`, `PING_INTERVAL_MS`) for
  every connection. Each knob now flows through to
  the connection (TCP `connect_timeout`), framing (mid-frame stall
  budget + I/O loop overall deadline), and ping-heartbeat layers, and
  validates its range at config-load time:
  `timeout_ms` `[100, 60_000]`, `connect_timeout_ms` `[1_000, 60_000]`,
  `ping_interval_ms` `[100, 300_000]`. `DirectConfig::validate` now
  returns `Result<Self, Error>`; the production / dev / stage presets
  remain infallible by construction. The redundant secondary-queue
  streaming `queue_depth` knob (and its event-channel-depth
  accessor) is removed: the post-SSOT pipeline has exactly one queue
  (the event queue sized by `ring_size`), so a separate
  event-channel-depth knob is dead. TOML configs that set
  `[streaming] queue_depth = ...` should switch to `ring_size`.
- The cross-language response-shape agreement validator
  (`scripts/validate_agreement.py`) now consumes a TypeScript
  shape manifest alongside the Python / CLI / C++ runtime artifacts.
  The TS SDK emits its public-surface field set from `index.d.ts`
  via `sdks/typescript/scripts/emit_validator_manifest.mjs`; the
  diff engine treats shape-only artifacts as field-presence-only
  (values do not contribute to value-vs-value diffs, status-PASS
  entries do not fold into runtime status disagreements). Any
  TypeScript public-surface drift relative to the runtime SDKs now
  surfaces as a pre-merge agreement failure rather than going
  unnoticed until a downstream consumer hit the missing / extra
  field.
- Repo hygiene pass. Root tree trimmed to a standard institutional
  layout: moved `ROADMAP.md` → `docs/`,
  moved `config.default.toml` into the `thetadatadx` crate, deleted unused
  `cliff.toml`. Architecture ADRs inlined into source-code comments at
  their relevant locations; `docs/architecture/` removed. Generated
  SDK files moved to `_generated/` subdirectories under each SDK
  (`sdks/python/src/_generated/`, `sdks/typescript/src/_generated/`)
  so hand-written code leads the public surface listing — the SSOT
  codegen for cross-language parity is preserved unchanged, only the
  output paths moved. The internal parity-tracking checklist removed
  (historical artifact; parity is shipped).
- Typed `Error` enum: `Error::Decode`, `Error::Decompress`,
  `Error::Config`, and `Error::Grpc` now carry structured `kind`
  fields (`DecodeErrorKind`, `DecompressErrorKind`,
  `ConfigErrorKind`, `GrpcStatusKind`) instead of bare `String`
  payloads. Callers can pattern-match on the kind for programmatic
  recovery without parsing error messages — e.g. distinguish a
  `DecodeErrorKind::TruncatedRow { row_idx, expected_columns,
  actual_columns }` from a `Protobuf(String)` codec failure, or
  branch on `GrpcStatusKind::DeadlineExceeded` vs. `Unauthenticated`
  without re-implementing the gRPC status-code Debug-string mapping.
  The transport-status conversion populates `Error::Grpc { kind: GrpcStatusKind::from_code(status.code()), .. }`
  so the retry classifier and Python exception mapper share the same
  typed dispatch path. Migration: replace
  `if let Error::Decode(msg) = err { ... }` with
  `if let Error::Decode { kind, message } = err { match kind { DecodeErrorKind::TruncatedRow { .. } => ..., _ => ... } }`;
  `Error::Config(format!(...))` constructions become
  `Error::config_invalid(field, message)` /
  `Error::config_out_of_range(field, value, min, max)` /
  `Error::config_missing(field)` etc. (helper constructors on
  `Error`). The Python `to_py_err` mapper preserves its existing
  `thetadatadx.SchemaMismatchError` / `RateLimitError` /
  `SubscriptionError` leaf classes — only the internal dispatch
  switched from string-comparing `status` to matching the typed
  `kind`.

### Fixed

- **Round-3 review caught two pull-iter regressions left over from the
  Wave-M iterator surface.** (a) The `EventIterator` terminal predicate
  keyed off the raw `client.shutdown` flag, which `stop_streaming()`
  flipped BEFORE the event-delivery thread had finished pushing
  the tail of in-flight events into the iterator's second bounded
  queue. Any caller polling `next_timeout` between those two moments
  saw an empty queue + asserted shutdown and returned `Closed`,
  dropping tail events on the floor. Replaced with a dedicated
  `iter_closed: Arc<AtomicBool>` flag flipped by a drop guard captured
  inside the event-delivery closure — the guard fires only when the
  producer is dropped at reader-loop exit, which only happens after
  the consumer thread has joined and every in-flight event has been
  pushed. Soak test
  `streaming_soak_tests.rs::iter_does_not_false_eof_during_drain`
  pins the contract: 100 pre-queued tail events with the global
  shutdown asserted MUST surface as `Ready` before the iterator
  signals `Closed`. (b) Six docs files taught a non-existent
  `events()` API on the client; corrected to the actual public entries
  (`start_streaming_iter()` Rust/Python/C++; `startStreamingIter()`
  TypeScript; `streaming_iter()` context manager on Python).
- **External multi-model audit: pull-iter `next_timeout` conflated
  timeout with terminal close.** `EventIterator::next_timeout()` now
  returns a typed three-state `NextEvent` enum (`Ready` / `Timeout` /
  `Closed`) instead of `Option<StreamEvent>`. Pre-fix, `None` overloaded
  "deadline expired on a quiet-but-live stream" with "upstream shut
  down + queue drained", which propagated into every binding:
  - C ABI `thetadatadx_streaming_event_iter_next` returned `-1` (terminal) on a
    quiet live stream, so C consumers saw false EOF.
  - C++ `EventIterator::ended_` latched on the false EOF; the STL
    `for (const auto& e : iter)` adapter terminated on the first
    timeout instead of re-polling.
  - Python `__next__` retried indefinitely after `stop_streaming()`
    because every 50 ms slice returned `None` (timeout-shaped) and
    the loop never observed a terminal signal.
  - TypeScript async `next()` had the same defect: the Promise spun
    forever once the upstream queue closed.
  Fixed by lifting `NextEvent` through the C ABI's three-state return
  (`0` ready / `1` timeout / `-1` closed), only latching the C++
  wrapper's `ended_` on `-1`, looping the C++ STL adapter on `1`,
  raising `StopIteration` from Python on `Closed`, and resolving the
  TS promise to `null` on `Closed`. Soak tests
  `streaming_soak_tests.rs::iter_returns_timeout_then_event_on_quiet_then_active_stream`
  and `iter_returns_closed_after_stop_streaming` pin both branches;
  Python `tests/test_iter_mode.py::test_iter_terminates_after_stop`
  asserts the Python `for event in iterator:` loop exits within 1 s
  of `stop_streaming()`. The blocking `Iterator::next` impl on
  `EventIterator` (no timeout) is unchanged — `None` there
  unambiguously means terminal because that path blocks until either
  an event arrives or the queue closes.
- **Docs site lead-pages still referenced the pre-Wave-K
  `Client` API class name and the removed `subscribe_quotes` /
  `subscribe_trades` / `subscribe_option_quotes` per-kind methods.**
  `docs-site/docs/getting-started/installation.md`,
  `docs-site/docs/getting-started/quickstart.md`, and
  `docs-site/docs/streaming/reconnection.md` now lead with
  `Client` plus the unified contract-first
  `client.subscribe(contract.quote())` /
  `client.subscribe(contract.trade())` API across Rust, Python,
  TypeScript, and C++. Event-payload examples switched from the
  removed `event.contract_id` (wire-internal) to
  `event.contract.symbol`. Migration tables in the CHANGELOG /
  release notes intentionally retain the old names — those document
  the rename, not the post-rename API.
- **External audit: single-slot drain barrier could falsely report
  quiescence under stacked lifecycle transitions.** The
  `prev_drained` slot tracked only the most recently retired session's
  flag, so a `start → stop → start → stop` sequence in which the
  earlier session was still draining when the later one retired
  silently lost the earlier flag. `await_drain()` then returned `true`
  based on the latest generation while the earlier callback could
  still be firing on the FFI `ctx` — a use-after-free vector under
  reconnect-storm scenarios. The slot is now a
  `Mutex<Vec<Arc<AtomicBool>>>`; every retired session's flag is
  pushed onto the Vec, and `await_drain()` / `thetadatadx_*_free` walk the
  full set, lazily GC'ing flags that have flipped. Mirrored on the
  FFI handle's `prev_drained` field. Regression coverage:
  `streaming_soak_tests.rs::multi_gen_drain_waits_for_all_retired_sessions`
  drives three real `StreamingClient` instances back-to-back with slow
  callbacks and asserts the barrier waits for every generation.
- **WS payload now carries `unresolved_contract_id` for
  pre-`ContractAssigned` ticks.** Pre-Wave-G the WS bridge surfaced
  the wire-internal numeric id; post-removal of the public `contract_id`
  field, ticks that arrived before the matching `ContractAssigned`
  frame serialised as an empty `Contract` envelope with no diagnostic
  channel for operators to correlate. The decoder now builds an
  unresolved-contract sentinel whose `symbol` is `__pending:<id>`
  (the canonical `sec_type == SecType::Unknown` check still gates
  consumer code paths); the WS formatter detects the prefix, emits
  `contract: {"status": "pending"}`, and surfaces the parsed wire id
  as a top-level `unresolved_contract_id` integer. The public SDK
  callback signature is unchanged — `__pending:` is a diagnostic
  payload, not a stable identifier.
- **WS `/subscribe` option path now runs the canonical Gregorian
  validator.** The Wave H `tdbe::time::is_valid_yyyymmdd` calendar
  check ran on the historical / REST surfaces but not on the WS
  option-subscribe path, which only applied the cheap
  `is_valid_yyyymmdd_range` bounds check. Impossible dates like
  `20260230` (Feb 30), `20260431` (Apr 31), or `20251301` (month 13)
  leaked through. Both gates now run; the bounds check is the
  precheck, the calendar validator is the real gate.
- **Python and TypeScript bindings: stop / shutdown clear the
  registered callback.** The unified C API preserves the callback
  across stop/reconnect, but the high-level bindings deliberately
  diverge: `stop_streaming()` and `shutdown()` clear the stored
  callback, so a subsequent `reconnect()` raises until the caller
  re-registers via `start_streaming(callback)`. Documented on every
  affected method (`stop_streaming`, `shutdown`, `reconnect` on both
  bindings) so the explicit-handoff model is no longer surprising.
- **`stop_streaming()` race that could resurrect streaming after
  stop returned.** An in-flight `start_streaming*()` that began
  before `stop_streaming()` could re-establish a live session after
  stop had already taken effect. Each `stop_streaming` now
  invalidates any concurrent start, so a start that raced a stop
  refuses to install rather than reviving the stopped session.
  Regression tests pin both branches of the race.
- **Historical `validate_date` accepted impossible Gregorian dates**
  (`00000000`, `20260230`, `19990431`, `21010101`). The shape-only
  check (length + ASCII digits) is now followed by a real calendar
  check via the new `tdbe::time::is_valid_gregorian_date` /
  `is_valid_yyyymmdd` validator (year ∈ 1900..=2100, valid month,
  day-of-month including the 4 / 100 / 400 leap rule). The
  `flatfiles::request::validate_date` helper routes through the
  same canonical validator.
- **Streaming `Contract::option` and OCC-21 parsing accepted impossible
  expirations** silently. Both paths now defer to the same
  canonical Gregorian validator as the historical path, so dates like Feb 30,
  Apr 31, or `00000000` fail at construction with an explicit
  error naming the offending input.
- **Silent system-clock failure in the streaming frame decoder.**
  A clock skew before the Unix epoch used to silently produce
  `received_at_ns = 0`; the path now logs a rate-limited warning
  (every 1024 failures) and falls back to `0` only after surfacing
  the condition to operators. The WS server's JSON-serialization
  failure path gets the same treatment: a new
  `json_serialize_failures` counter is exposed alongside the
  existing `broadcast_dropped` counter on
  `GET /v3/system/streaming/status`, and the failure path logs a
  rate-limited error.
- **Self-join deadlock when the user callback calls
  `stop_streaming()`.** With the consumer-thread dispatch in place, a
  callback that drops the last `Arc<StreamingClient>` (which is what
  `ThetaDataDx::stop_streaming()` does internally) used to block on
  `StreamingClient::Drop`'s `io_handle.join()`. The I/O thread's exit path
  drops the event-queue producer, and dropping the producer joins
  the consumer thread — the very thread running the callback. `Drop`
  now captures the consumer thread's `ThreadId` on first dispatch
  (`OnceLock`), detects the self-join case, and detaches the join
  onto a helper thread named `fpss-shutdown-detach`. Cleanup completes
  asynchronously: `is_streaming()` flips to `false` immediately on
  the `Live → Stopped` swap, BEFORE the helper has joined; the new
  `ThetaDataDx::await_drain` / `thetadatadx_*_await_drain` barrier (see
  Added) is the way to confirm full quiescence — i.e. that the
  previous user callback has stopped firing.
  `streaming_soak_tests.rs::callback_triggered_stop_does_not_self_join`
  drives the real `StreamingClient` through this path under a 5-second
  watchdog, and `callback_triggered_stop_then_await_drain_completes`
  asserts the new barrier returns `true` within budget and no further
  callback invocations happen after it returns.
- **Round-2 review caught two follow-up gaps.** (a) The non-blocking
  C ABI poll path (`thetadatadx_streaming_event_iter_next(.., 0)`) was still
  collapsing timeout + closed via `try_next()` returning
  `Option<StreamEvent>`. Fixed by promoting `EventIterator::try_next()`
  to also return the typed `NextEvent` enum (symmetric with
  `next_timeout`); the FFI now drives off the typed shape uniformly,
  so a C client polling after `stop_streaming()` sees rc `-1`
  (terminal) instead of rc `1` (timeout) forever. The C++ wrapper's
  `try_next()` calls the C ABI directly with `timeout_ms = 0` and
  latches `ended_` only on rc `-1`. The Python `try_next` and
  TypeScript `tryNext` keep their `Option<…>` public surfaces by
  collapsing both `Timeout` and `Closed` to `None` / `null` (single-
  state non-blocking polling stays the documented contract). New
  soak test
  `streaming_soak_tests.rs::iter_try_next_returns_closed_after_drain`
  pins the contract: `try_next()` returns `Closed` (not `Timeout`)
  once the queue is drained on a stopped session, and stays sticky
  on subsequent calls. (b) Front-door docs-site pages still taught
  the pre-Wave-K API. Updated
  `docs-site/docs/index.md`, `docs-site/docs/api-reference.md`,
  `docs-site/docs/getting-started/{authentication,first-query,streaming}.md`,
  and `docs-site/docs/streaming/{index,connection,events}.md` to use
  `Client` (Rust / Python / TypeScript) plus
  `thetadatadx::UnifiedClient` (C++), the polymorphic
  `client.subscribe(contract.quote())` /
  `client.subscribe(sec_type.full_trades())` API, the
  `client.start_streaming_iter()` / `client.streaming_iter()` pull-iter idiom, and
  `event.contract.symbol` on data events. Migration tables in this
  CHANGELOG and the v9.1.0 release notes intentionally retain the
  pre-Wave-K names — those document the rename, not the post-rename
  surface.

### Added

- Flat-files ecosystem coverage across the tools surface. The `thetadatadx` CLI
  gains a `flatfile` subcommand group (`quotes`, `trades`, `trade_quote`,
  `ohlc`, `open_interest`, `eod`, the four `stock_*` equivalents, and
  the generic `request` arm). Each subcommand takes a single `YYYYMMDD`
  date plus `--format csv|jsonl` and `-o/--output` flags; missing
  `-o` streams the bytes to stdout. (closes #433)
- REST server adds `GET /v3/flatfile/{sec_type}/{req_type}?date=...&format=...`
  and `POST /v3/flatfile/request` route handlers. The bytes ride a
  chunked streaming response body so even hundred-MB blobs do not pin
  server memory; `Content-Type` is
  `text/csv; charset=utf-8` for CSV or `application/x-ndjson; charset=utf-8`
  for JSONL. Flat files are batch downloads, not streaming
  subscriptions, so the WebSocket surface is unchanged. (closes #432)
- MCP server exposes eleven flat-file tools mirroring the Rust
  convenience methods: `thetadatadx_flatfile_request` (generic) plus
  `thetadatadx_flatfile_option_quote` / `_trade` / `_trade_quote` / `_ohlc` /
  `_open_interest` / `_eod` and the four `thetadatadx_flatfile_stock_*`
  shortcuts. Each tool writes the decoded blob to disk and returns
  the path so the LLM client can hand the file off to a downstream
  consumer that already speaks CSV / JSONL. (closes #431)
- Cross-language utility helpers (`condition_name`, `condition_description`,
  `is_cancel`, `updates_volume`, `quote_condition_name`,
  `quote_condition_description`, `is_firm`, `is_halted`, `exchange_name`,
  `exchange_symbol`, `sequence_signed_to_unsigned`,
  `sequence_unsigned_to_signed`) now exposed in every binding. Python
  surfaces them as `thetadatadx.util.*`; TypeScript as the `Util` class
  with camelCase methods (`Util.conditionName(0)`); C++ as inline
  wrappers in the `thetadatadx::util::*` namespace; the C ABI as
  `thetadatadx_condition_name`, `thetadatadx_exchange_name`, `thetadatadx_sequence_signed_to_unsigned`,
  etc. The Rust source-of-truth tables in `tdbe::{conditions, exchange,
  sequences}` drive every binding directly — no language-specific
  duplication of the lookup data. (closes #424)
- docs-site dedicated FLATFILES section under
  `docs-site/docs/flatfiles/` (overview, quickstart, API reference)
  with code samples in Python, TypeScript, C++, the `thetadatadx` CLI, the
  REST server, and the MCP server. Wired into the VitePress sidebar
  alongside Real-Time Streaming. (closes #441)
- The query-builder docs page now covers FLATFILES request
  construction alongside the per-contract historical builder, including the
  parameter table, every snippet shape, and the bandwidth caveats.
  (closes #442)
- ROADMAP gains a Binding Coverage Matrix tracking which features are
  exposed in each SDK / tool surface (Rust, Python, TypeScript, C, C++,
  MCP, CLI, REST/WS). Wave O flips the FLATFILES-tools and
  cross-language-utils rows to shipped. (closes #446)
- Pull-iter delivery mode restored (was deleted in v8.0.30; now
  back as a sibling to push-callback). Adds
  `ThetaDataDx::start_streaming_iter()` returning a
  `thetadatadx::EventIterator` in Rust; `start_streaming_iter()`
  / `with client.streaming_iter() as it:` returning the same iterator
  on Python (`for event in it:`); `startStreamingIter()` returning
  an async-iterable `EventIterator` napi class on TypeScript
  (`for await (const event of iter)`); and
  `thetadatadx_client_start_streaming_iter` / `thetadatadx_streaming_event_iter_next`
  / `thetadatadx_streaming_event_iter_close` / `thetadatadx_streaming_event_iter_free` in
  the C ABI plus a move-only `thetadatadx::EventIterator` with STL-iterator
  adapters in the C++ wrapper.

  The event-delivery thread pushes each event into a second bounded
  queue sized to match the event queue; the user thread drains that
  queue under one lock acquisition per batch. On
  the Python binding this batches what was per-event Python-side
  overhead into one step across the whole drain, which is the dominant
  throughput cost for tuple-build / deque-append integrators —
  `streaming_throughput.rs::pyo3_iter_next_drain` measures
  ~4.6 Melem/s vs. ~1.1 Melem/s for the equivalent push-callback
  shape (`pyo3_deque_append`), a 4.1× win on the same per-event
  Python work.

  Push-callback (`start_streaming(callback)`) remains the
  recommended low-latency default; pull-iter is for high-throughput
  batch processing where amortising the lock cost dominates.
  Backpressure semantics match the callback path: when the iterator
  falls behind and the queue saturates, the consumer drops the new
  event and increments the same `dropped_event_count()` counter
  callbacks already surface. Mode is chosen at start; push and pull
  are mutually exclusive on a given client. Switch by stopping
  streaming and starting again.
- `ThetaDataDx::panic_count()` and `StreamingClient::panic_count()` — new
  public methods that snapshot the count of user-callback panics
  caught by the event-delivery thread's panic-isolation boundary. Each
  caught panic is also logged as an error.
- `ThetaDataDx::await_drain(timeout)` — Rust quiescence barrier.
  Polls the previous streaming session's drain flag (set after the
  I/O thread and event-delivery thread have joined) and returns `true`
  when the previous user callback is guaranteed to have stopped
  firing. Pair with `stop_streaming` / `reconnect_streaming` from a
  thread other than the consumer thread when the application needs
  to free a captured context, replace the callback closure, or
  otherwise depend on full quiescence.
- `thetadatadx_client_await_drain(handle, timeout_ms)` and
  `thetadatadx_streaming_await_drain(handle, timeout_ms)` — C ABI mirror of
  `await_drain`. Returns `1` once the previous event-delivery
  thread has joined, `0` on timeout. Required between
  `thetadatadx_*_stop_streaming` / `_reconnect` / `_shutdown` and freeing
  `ctx`; the FFI `ctx` lifetime contract is now explicit that
  stop / reconnect are asynchronous on the consumer side.
- `StreamingClient::drained_flag()` — exposes the shared `Arc<AtomicBool>`
  the higher-level barrier polls; useful for binding-layer code that
  wants to wire its own quiescence semantics.
- Python `await_drain(timeout_ms) -> bool` and `with client.streaming(callback)
  as session:` context manager. TypeScript `awaitDrain(timeoutMs)` and
  `await using session = await client.streaming(callback)` (TC39 explicit
  resource management). Both auto-call `stop_streaming` + `await_drain`
  on scope exit, mirroring the C++ RAII destructor lifecycle. The
  bound `session` proxies every `subscribe_*` / `unsubscribe_*` call
  to the underlying client (Python `__getattr__`, TypeScript `Proxy`)
  so the streaming surface stays a single source of truth rooted in
  the Rust crate. Drain timeouts emit a `RuntimeWarning` (Python) or
  `console.warn` (TypeScript) without masking exceptions raised inside
  the body.
- `RingSizeError` (`TooSmall { provided, minimum }` /
  `NotPowerOfTwo { provided, suggested }`) — surfaced through
  `Error::Config` from `StreamingClient::connect` so a misconfigured
  buffer budget fails closed at construction with the offending
  value and the nearest valid size (ADR-002).
- `streaming_soak_tests.rs` — four soak
  tests (slow callback, panicking callback, callback-triggered stop,
  burst overload) plus the await-drain quiescence and free-blocks-
  until-drain tests, all exercising the consumer-thread wiring
  without a live streaming connection. Lives inside the crate (rather
  than `tests/`) so the harness constructor stays `#[cfg(test)]`-only.
- Python SDK: "Streaming buffering" section in `sdks/python/README.md`
  documenting the `collections.deque` (Pattern A, default) and
  `queue.Queue` (Pattern B, cross-thread blocking) consumer patterns.
- Vendor failure-mode resilience: capture+replay test harness against
  recorded streaming bytes (`tests/replay_capture.rs`), mid-frame TLS
  disconnect injection (`tests/midframe_disconnect.rs`),
  reconnect-storm test (`tests/reconnect_storm.rs`), vendor schema-drift
  coverage (`tests/vendor_schema_drift.rs`), property-based frame-decoder
  fuzz target (`tests/decode_fuzz_property.rs`), callback-watchdog API
  + slow-callback counter + rate-limited warning log
  (`tests/callback_watchdog.rs`).
- `ThetaDataDx::set_slow_callback_threshold(Duration)` and
  `ThetaDataDx::slow_callback_count() -> u64` (mirrored on
  `StreamingClient`) — opt-in observability for user callbacks that exceed
  a wall-clock threshold. The event-delivery thread measures every
  callback's elapsed time and increments a counter when over budget;
  a warning is logged rate-limited per 1024 over-budget events to
  avoid log amplification. Observability only — Rust cannot safely
  cancel arbitrary user code, so the watchdog never kills the consumer.
  `Duration::ZERO` disables the timer path entirely.
- Flat-file SDK parity across Python, TypeScript, and C++. New
  `client.flat_files.*` (Python) / `client.flatFiles.*` (TypeScript) /
  `client.UnifiedClient::flat_files()` (C++) namespace returning a
  row-list with `.to_arrow()` / `.to_pandas()` / `.to_polars()` /
  `.to_list()` (Python) / `.toArrowIpc()` / `.toJson()` (TypeScript) /
  `.to_arrow_ipc()` (C++) terminals plus a generic `request(sec_type,
  req_type, date)` dispatcher and `flatfile_to_path` raw-bytes helper.
  The dynamic schema (columns determined at runtime by `(SecType,
  ReqType)`) is implemented as thin hand-written wrappers over the
  flat-file `rows_to_arrow` builder rather than through the codegen
  pipeline, which targets static-schema surfaces only.

### Removed

- `contract_id: i32` removed from every `StreamData::*` variant across
  every binding (Rust, Python, TypeScript, C, C++). The wire-internal
  numeric id the streaming server assigns is no longer surfaced on data
  events; consumers read `event.contract.symbol` (or other `Contract`
  fields — `expiration`, `strike`, `is_call`) for identity. Code that
  needs an id-keyed map builds it from the
  `StreamControl::ContractAssigned` event stream. The
  `fpss_event_schema.toml` SSOT bumps to `version = 5`; every
  generated binding regenerates without a `contract_id` field.
- `StreamEvent::{RawData, Empty}` no longer in the public type. Truncated
  FIT payloads are filtered out and counted on the
  `thetadatadx.fpss.decode_failures` metric rather than surfaced as
  events, with no added per-event cost on the delivery path. The
  server's contract map-and-relookup is gone: the contract now rides on
  the event itself, eliminating a reconnect / market-close race on the
  WebSocket bridge.
- **Go SDK deleted end-to-end.** The previous half-state (Go files
  shipped, but no CI, no live validation, and not advertised) was a
  drift hazard; the C ABI remains the supported integration path for
  any third-party C / C++ consumer. Its generator support is removed
  with it.
- The internal streaming-dispatcher layer and its public exports.
  Panic isolation, drop counting, and consumer-thread invariants now
  live directly on the event-delivery path.
- The bounded-channel runtime dependency on `thetadatadx`.
- `expert-mode` and `test-harness` Cargo features on `thetadatadx`.
  The C ABI no longer exposes `thetadatadx_*_set_inline_callback`; the
  queued and inline paths shared the same event-delivery
  pipeline post-#513, so the parallel entry points were theatre.
  The test-harness constructor (`for_self_join_test`) is now
  `#[cfg(test)]`-only and lives alongside the soak tests inside
  the crate.
- `IntoOptionSpec` sealed trait. `Contract::option(symbol,
  expiration, strike, right)` returns to its explicit four-argument
  form; callers holding wire-format integer triples use
  `Contract::option_raw(symbol, expiration, is_call, strike_raw)`
  instead.
- The `Default` impl on the streaming connection parameters. The previous
  impl manufactured empty-string credentials inside a `OnceLock` so callers
  could spread `..Default::default()`; that produced a value that could
  not actually connect. Use the explicit `new(&creds, &hosts)` constructor
  and override the optional fields explicitly.

### Changed (continued)

- TypeScript CI now runs `npm test` on every advertised platform
  (Linux, macOS, Windows) instead of gating to Linux only. CI parity
  with the Python and Rust matrices: every platform we ship a
  prebuilt addon for has its tests run on that platform.
- `README.md`, the `thetadatadx` crate README, `docs/architecture.md`,
  `sdks/README.md`, `docs-site/docs/streaming/events.md` no longer
  advertise Go SDK support; the Rust install examples in
  `README.md`, the `frames` module docs, and
  `docs-site/docs/getting-started/{quickstart,installation}.md` now
  show `thetadatadx = "9"`.
- `StreamingClient::connect` now rejects a non-power-of-two `ring_size`
  with `Error::Config` rather than silently rounding to the next
  power of two (ADR-002). Default configs (`131_072`) are unchanged.
- `Contract::option(symbol, expiration, strike, right)` reverts to
  the explicit four-argument signature; the wire-format integer
  triple constructor moves to `Contract::option_raw(...)`.

### Performance

- Per-event cost on the `start_streaming` path drops by removing an
  event copy and an intermediate hand-off hop that previously sat
  between the event-delivery thread and the user callback.

  Microbenchmark methodology (the `streaming_channels` bench): each
  variant publishes exactly 100,000 events per sample and counts only
  events actually delivered to the callback, so the reported figures
  are per-delivered-event cost. Earlier revisions divided wall-clock
  by attempted publishes, which silently understated cost when the
  consumer fell behind; the count of delivered events is now asserted
  against the target so the figure cannot drift.

  Indicative numbers from `cargo bench --bench streaming_channels --
  --quick` on a recent x86-64 Linux laptop (Criterion median, native
  release build, throughput per delivered event):
  - live SSOT path (non-blocking enqueue + event-delivery thread +
    per-callback panic isolation):
    ≈ 1.46 ms / 100k ≈ 14.6 ns / delivered event
    (≈ 68 Melem/s).
  - same pipeline without the panic-isolation boundary:
    ≈ 1.47 ms / 100k ≈ 14.7 ns / delivered event
    (≈ 68 Melem/s) — the panic-isolation cost is below Criterion's
    noise floor on `Empty` events.
  - production-shape topology (producer on a worker thread, consumer
    on the event-delivery thread):
    ≈ 1.49 ms / 100k ≈ 14.9 ns / delivered event
    (≈ 67 Melem/s).
  - direct callback (prospective TLS-reader-direct path modelled
    via `Box<dyn Fn>` adapter, no queue, no consumer thread):
    ≈ 533 µs / 100k ≈ 5.3 ns / delivered event
    (≈ 188 Melem/s).

  Run `cargo bench --bench streaming_channels` for per-machine
  numbers; the absolute values are sensitive to CPU model and
  governor settings, so the SDK ships the methodology and the
  variants rather than locking in figures that age out with each
  hardware refresh. The earlier "1.13 ns / event" figure for the
  queued variants was an artefact of dividing wall-clock by
  attempt count, not delivered events; the corrected number above
  reflects per-callback-delivery cost.

## [9.0.2] - 2026-05-07

### Added

- Property-based tests on five hot paths via `proptest = "1.5"`
  (dev-dependency only; no public-API surface change).
  - `tdbe` FIE codec: encoder/decoder round-trip on the
    full FIE alphabet (excluding `'n'`, the documented terminator
    nibble), strict-vs-panicking-encoder agreement, and per-character
    nibble round-trip.
  - `tdbe` FIT codec: single-row FIT round-trip via a
    cfg(test) encoder against `FitReader::read_changes`,
    `decode_fit_buffer_bulk` agreement on the same byte stream, and
    `flush_digits` non-negative-monotonicity on partial digit runs.
  - `tdbe` greeks: Black-Scholes invariants over the
    market range `(spot, strike) in [0.01, 10000.0]`,
    `rate in [-0.05, 0.20]`, `div_yield in [0.0, 0.10]`,
    `tte in [1/365, 5.0]`, `iv in [0.001, 5.0]` -- put-call parity
    (tolerance scaled to `norm_cdf` approximation error), call/put
    delta bounds, vega non-negativity, gamma non-negativity.
  - `tdbe` time: `civil_to_epoch_days` monotonicity over
    1970..=2099, `eastern_offset_ms` returns exactly EST or EDT for
    every timestamp in 2000..=2099, and DST cutover sanity at the
    spring-forward / fall-back boundaries across 1990..=2099 (covers
    both pre- and post-2007 rule windows).
  - Streaming protocol contract:
    `Contract -> to_bytes -> from_bytes` round-trip for stocks and
    options (expiration 2000..=2099, strike 1..=99_999_999,
    right in {C, P}, root 1..=6 ASCII uppercase), OCC-21 string
    parser round-trip, and composite `OCC-21 -> Contract -> bytes`
    round-trip pinning the parser and wire codec against each other.

## [9.0.1] - 2026-05-07

### Changed

- Stripped 76 per-line internal provenance comments across the
  streaming, historical, and config trees, consolidating the protocol-parity
  rationale into a single module-header note in each affected file.
- Reorganised the streaming I/O internals so the login handshake and
  ping heartbeat live alongside the main loop instead of in one large
  file. Internal-only; no behavior change.
- Removed `docs/public-api-redesign.md`; v9.0.0 shipped the redesign
  it described, and a one-line note in `docs/architecture/README.md`
  records that the planning prose for shipped surfaces is intentionally
  not preserved.
- Declared `rust-version = "1.88"` on every workspace `[package]`. CI's
  `Lint` matrix grows a `1.88` axis on Linux so dependency bumps that
  raise the rustc requirement fail before release; `README.md` adds a
  `Requirements` section.
- Added a `Semver check` CI job that runs
  `obi1kenobi/cargo-semver-checks-action@v2` against the `v9.0.0` tag
  on every PR. `CONTRIBUTING.md` documents the public-API stability
  policy and the local invocation.

## [9.0.0] - 2026-05-07

### Breaking

- **`Contract::option` is now polymorphic; `Contract::option_raw` is gone.**
  A new sealed `IntoOptionSpec` trait accepts either `(&str, &str, &str)`
  (human-friendly: expiration / strike / right) or `(i32, bool, i32)`
  (wire-format integer triple). Callers pass one tuple instead of four
  loose arguments, and the wire-format constructor moves under the same
  method name.

  ```rust
  // Before:
  let c = Contract::option("SPY", "20261218", "60", "C")?;
  let c = Contract::option_raw("SPY", 20261218, true, 60_000);

  // After:
  let c = Contract::option("SPY", ("20261218", "60", "C"))?;
  let c = Contract::option("SPY", (20261218, true, 60_000))?;
  ```

- **`StreamingClient::connect` takes one streaming connection-parameters
  struct instead of seven loose arguments.** The struct exposes `creds`,
  `hosts`, `ring_size`, `flush_mode`, `policy`, `derive_ohlcvc`. `Default`
  and a `new(creds, hosts)` shortcut cover the common path.

  ```rust
  // Before:
  StreamingClient::connect(&creds, &hosts, 4096, StreamingFlushMode::default(),
                      ReconnectPolicy::default(), true, handler)?;

  // After: creds, hosts, and the optional knobs ride in one
  // connection-parameters value `args`, then:
  StreamingClient::connect(args, handler)?;
  ```

- **Wire-internal `contract_id: i32` removed from every public surface.**
  Dropped from Rust (`ThetaDataDx::contract_map`, `ThetaDataDx::contract_lookup`,
  `StreamingClient::contract_map`, `StreamingClient::contract_lookup`), C ABI
  (`thetadatadx_client_contract_map`, `thetadatadx_client_contract_lookup`,
  `thetadatadx_streaming_contract_map`, `thetadatadx_streaming_contract_lookup`,
  `thetadatadx_contract_map_array_free`, `ThetaDataDxContractMapArray`, `ThetaDataDxContractMapEntry`),
  Python (`contract_map()`, `contract_lookup()`), TypeScript
  (`contractMap()`, `contractLookup()`), and C++ (`StreamingClient::contract_map`,
  `StreamingClient::contract_lookup`). Users identify contracts by
  `(symbol, expiration, right, strike)`; the wire id stays inside the
  reader-thread cache and is delivered alongside every event via
  `StreamControl::ContractAssigned { id, contract }` for callers that
  still need to maintain their own id→contract map.

  ```rust
  // Before:
  let map = client.contract_map()?;
  if let Some(c) = client.contract_lookup(id)? { ... }

  // After: build the map yourself from the event stream.
  client.start_streaming(|event| {
      if let StreamEvent::Control(StreamControl::ContractAssigned { id, contract }) = event {
          my_map.insert(*id, Arc::clone(contract));
      }
  })?;
  ```

- **`pub mod proto` is now `pub(crate)`.** Generated protobuf types are
  wire-internal. Bindings that need `DataTable` / `DataValueList` /
  `ResponseData` / `Price` / `data_value::*` go through the new
  `thetadatadx::wire` re-export, which surfaces only the types
  offline-decode harnesses actually need.

  ```rust
  // Before:
  use thetadatadx::proto::{DataTable, ResponseData};

  // After:
  use thetadatadx::wire::{DataTable, ResponseData};
  ```

- **Streaming submodules `connection`, `framing`, `dispatcher`, `ring`
  reduced to `pub(crate)`.** Only `protocol` remains a public submodule
  of `fpss`. `Frame`, `read_frame`, `write_frame` are surfaced as items
  at `thetadatadx::fpss::` for benchmark consumers; everything else
  (TLS connect, event-queue wait strategies, dispatcher internals) is
  now crate-private.

  ```rust
  // Before:
  use thetadatadx::fpss::framing::{read_frame, write_frame, Frame};

  // After:
  use thetadatadx::fpss::{read_frame, write_frame, Frame};
  ```

### Added

- `IntoOptionSpec` sealed trait + impls for `(&str, &str, &str)` and
  `(i32, bool, i32)` — see `fpss::protocol::IntoOptionSpec`.
- Streaming connection-parameters struct + its `new(creds, hosts)`
  shortcut.
- `thetadatadx::wire` module — the supported re-export surface for the
  generated protobuf payload types (`DataTable`, `DataValueList`,
  `DataValue`, `ResponseData`, `Price`, `CompressionAlgo`,
  `CompressionDescription`, `data_value`).

### Changed

- **Comprehensive public-API discipline sweep.** `auth::{creds, nexus,
  session}` reduced to `pub(crate)`; user-facing types (`Credentials`,
  `AuthResponse`, `AuthUser`, `SessionToken`, `authenticate`,
  `authenticate_at`) re-exported at `thetadatadx::auth::*` and the
  crate root. Every `pub fn` / `pub struct` reachable from a public
  path was audited; internal helpers (TLS connect entry points,
  ring-size constants, framing reader idle predicates) are now
  crate-private.
- `tdbe` 0.12.10 → 0.13.0 (eastern-time + json_canon + conditions
  codegen surface expansion warrants the minor bump).

### Removed

- `Contract::option_raw` (folded into `Contract::option` via
  `IntoOptionSpec`).
- `ThetaDataDx::contract_map`, `ThetaDataDx::contract_lookup`,
  `StreamingClient::contract_map`, `StreamingClient::contract_lookup`.
- C ABI: `thetadatadx_client_contract_map`, `thetadatadx_client_contract_lookup`,
  `thetadatadx_streaming_contract_map`, `thetadatadx_streaming_contract_lookup`,
  `thetadatadx_contract_map_array_free`, `ThetaDataDxContractMapArray`,
  `ThetaDataDxContractMapEntry`.
- Python SDK: `ThetaDataDx.contract_map`, `ThetaDataDx.contract_lookup`.
- TypeScript SDK: `ThetaDataDx.contractMap`, `ThetaDataDx.contractLookup`.
- C++ SDK: `StreamingClient::contract_map`, `StreamingClient::contract_lookup`.
- `pub mod proto` (now `pub(crate)`; consumers use `thetadatadx::wire`).
- `pub mod fpss::{connection, framing, dispatcher, ring}` (now
  `pub(crate)`; surfaces preserved as items at `fpss::` root where
  needed).
- Dead helpers removed: `fpss::connection::connect_to`,
  `fpss::framing::FrameReadState::is_idle`,
  `fpss::ring::DEFAULT_RING_SIZE`.

## [8.0.37] - 2026-05-07

### Added

- **Typed `SubscriptionTier` enum** (`Free`, `Value`, `Standard`, `Pro`)
  replacing raw `Option<i32>` on `HistoricalClient`.
  `max_concurrent_requests(self)` codifies the `2^tier` semaphore
  semantics; `from_wire(i32)` decodes the wire byte (returning `None`
  for unknown values rather than silently coercing). Re-exported as
  `thetadatadx::SubscriptionTier`. The wire-side `auth::nexus::AuthUser`
  keeps its raw `Option<i32>` fields so deserialization stays
  infallible for unknown future tiers; the typed enum is the
  post-decode in-memory shape callers see.

### Changed

- **Streaming state machine consolidated into a single lock-free slot.**
  The client's prior three-field streaming state collapses into one
  atomically-swapped `Idle` / `Live` / `Stopped` value, so every read
  path (`is_streaming`, `connection_status`, `with_streaming`, every
  per-subscription forwarder) is a single lock-free load while lifecycle
  transitions keep serial semantics. Behavior is unchanged.
- **Internal layout moves.** The historical-specific modules were relocated
  under the `mdds` module tree and the generated-code files segregated
  into their own submodules. The public re-exports
  (`thetadatadx::endpoint::*`, `thetadatadx::EndpointMeta`,
  `thetadatadx::ENDPOINTS`) are preserved at the crate root for
  back-compat.

### Fixed

- (LOW 3.2) `extract_*_column` return type left as `Vec<Option<T>>` —
  iterator conversion deferred. The three helpers are public surface
  exercised by benches, integration tests, the macro-driven list
  endpoints, and the Polars / Arrow column projections; switching to
  `impl Iterator<Item = Option<T>>` would force every caller to deal
  with the iterator shape and lose the missing-header early-return the
  warn-log path relies on.
- (LOW 3.9) `Drop::drop` on `Client` documents its idempotency
  invariant. The `Idle` / `Live` / `Stopped` state machine guarantees
  the streaming / dispatcher shutdown sequence runs at most once across
  `stop_streaming` + `Drop`.
- (LOW 3.10) `#[allow(dead_code)]` removed from
  `flatfiles/framing.rs::msg`. Every one of the ten `u16` wire-code
  constants is genuinely used by `flatfiles::request` and
  `flatfiles::session`; the attribute was a false positive.

## [8.0.36] - 2026-05-07

### Changed

- **The `decode.rs` module (2177 LoC) split into 7 modules**
  under `mdds/decode/{error,headers,transport,extract,cell,v3}`. Pure
  structural refactor; public API unchanged via `mdds::decode::*` re-exports.
- **Eastern-time + DST primitives lifted to `tdbe::time`.**
  `eastern_offset_ms`, `march_second_sunday_utc`, `november_first_sunday_utc`,
  `april_first_sunday_utc`, `october_last_sunday_utc`, `civil_to_epoch_days`,
  `timestamp_to_ms_of_day`, `timestamp_to_date` — single canonical module
  reused by mdds, fpss, flatfiles. tdbe 0.12.9 → 0.12.10.
- **The streaming `protocol.rs` module (1613 LoC) split into 4 modules**
  under `fpss/protocol/`. `mod.rs` keeps constants and re-exports;
  `contract.rs` holds `Contract` + 6 constructors + `Display` + `FromStr` +
  OCC-21 parser; `wire.rs` holds payload builders / parsers; `subscription.rs`
  holds `SubscriptionKind`.
- **The `config.rs` module (1396 LoC, 30 flat fields) refactored
  into 7 nested typed sub-configs.** `DirectConfig` now contains `mdds`,
  `fpss`, `reconnect`, `retry`, `auth`, `metrics`, `runtime`. Field-read
  accessors preserved on `DirectConfig` for back-compat (`config.mdds_host()`
  etc still work). Field-write callers must migrate to nested form
  (`config.fpss.queue_depth = ...`). Adds `mdds.connect_timeout_secs`
  (default 10s, covers prior LOW finding).
- **The `tdbe` conditions module refactored to TOML-driven codegen.**
  The trade and quote condition tables (149 and 75 entries) are now
  generated from TOML source into compile-time const arrays at build
  time. Public surface unchanged; a new test pins 12 known entries
  against the generated arrays for round-trip protection.

## [8.0.35] - 2026-05-07

### Documentation

- **Sweep stale `root` / `exp_date` references across the doc tree.**
  Post-#484 (8.0.28) follow-up: `docs/api-reference.md`, `docs/macro-guide.md`,
  `docs/architecture.md`, `docs-site/docs/api-reference.md`,
  `docs-site/docs/streaming/{connection,events}.md`, `docs-site/docs/historical/option/list/{roots,contracts}.md`,
  `sdks/cpp/README.md` — Rust SDK references rewritten to use the post-#484
  `symbol` / `expiration` vocabulary. Closes #503.

## [8.0.33] - 2026-05-07

### Added

- **Seven Architecture Decision Records** under `docs/architecture/`:
  ADR-001 (JVM terminal parity sourcing), ADR-002 (streaming ring power-of-two
  capacity), ADR-003 (historical `2^tier` concurrent-request mapping), ADR-004
  (Eastern-time DST cutover), ADR-005 (OCC-21 century scope, expires
  2099-12-31), ADR-006 (streaming reconnect policy with rate-limit-aware backoff),
  ADR-007 (flatfiles historical SPKI pin rotation policy).
- **`tdbe::json_canon`** — JSON canonicalisation (non-finite f64 to `null`)
  is now a `tdbe` submodule. Existing `json_canon::*` callers in the CLI,
  MCP, and server crates migrate to `tdbe::json_canon::*`.

### Changed

- **`ParsedRight::from_wire_byte` is now wired at the four `is_call` /
  `is_put` magic-number sites in the `tdbe` tick types.** The
  `right == 67` / `right == 80` raw integer comparisons go through the
  typed parser; behaviour is identical, the magic numbers are gone.

### Removed

- **`json_canon` workspace member** folded into `tdbe::json_canon`.
  External consumers should switch to `tdbe = "0.12.9"` and import
  `tdbe::json_canon::{canonicalize, canonicalize_and_serialize, finite_or_null}`.
- **Internal version references in source comments** (`v8.0.10`, `v7.2.0`,
  `v8.0.3`, `v8.0.2`, `v6.0.1+`). Semantic content preserved; release
  metadata pruned out of code paths where it adds no maintenance value.
- **Banned-vocabulary sweep on full-stream subscription wording** in
  source doc comments, README.md, and ROADMAP.md. Replaced with
  `full-stream` / `full-type`. CHANGELOG history is intentionally
  untouched.

## [8.0.32] - 2026-05-06

### Fixed

- **StreamingDispatcher drain loop now catches user-callback panics**
  and continues serving subsequent events. New `panic_count`
  diagnostic counter exposed alongside `dropped_count`. Previously a
  panic in user code (Rust closure / PyO3 callable / napi
  ThreadsafeFunction / C extern fn) silently killed the dispatcher
  thread; only `shutdown()` surfaced it.
- **Streaming C ABI handle state machine.** `thetadatadx_streaming_set_callback` /
  `_inline_callback` / `_reconnect` / `_shutdown` enforce the public
  contract: at most one registration per handle, shutdown is
  terminal, post-shutdown ops return -1 with a clear `thetadatadx_last_error()`
  string. Previously the contract was documented but unenforced.
- **C ABI `ctx` lifetime contract documented.** The public header now
  states `ctx` must outlive registration until shutdown / free
  returns (or a successful unified re-registration). Queued vs inline
  thread-affinity also documented.
- **Python `dropped_event_count()` doc + test corrected** to
  reflect the actual reset-on-reconnect / zero-after-stop semantics.
  The counter lives on the StreamingDispatcher and resets when the
  dispatcher is rebuilt (matches the TypeScript binding).
- **Dispatcher `dropped` counter is now strictly queue-full.** The
  `Disconnected` variant of `TrySendError` (rare; happens only during
  shutdown races) feeds a new separate `disconnected_count` so the
  user-facing drop metric isn't inflated by lifecycle noise.
- **TypeScript `index.d.ts` regenerated** so the JS-visible doc
  comment matches the Rust source — the counter resets on reconnect.

### Changed

- **Unified C ABI `set_callback` after stop is REPLACEMENT.** Documented
  explicitly in `thetadx.h` and the rustdoc: contrary to the streaming
  one-shot rule, the unified high-level path supports stop +
  re-register as a normal user flow (this is what `reconnect_streaming`
  is built on). The contract divergence is intentional.

## [8.0.31] - 2026-05-06

### tdbe

- `tdbe::right::ParsedRight::from_wire_byte(byte: i32) -> Option<Self>`
  — `const fn` decoder for the streaming wire `right` byte (`67` for
  `'C'`, `80` for `'P'`). Inverse of the existing `as_wire_byte()`.
  Removes the rationale for downstream tick decoders to re-type the
  `67` / `80` magic numbers at every trust boundary; round-trip
  property test confirms `from_wire_byte(self.as_wire_byte().unwrap())
  == Some(self)` for every variant where the forward direction is
  defined. Patch bump tdbe 0.12.8 → 0.12.9.

## [8.0.30] - 2026-05-06

This release closes #482: the entire streaming stack — Rust core,
C ABI, Python, TypeScript, and C++ — moves to a callback-driven
delivery model backed by a single `StreamingDispatcher` SSOT. Bundles
PR #489 (dispatcher core), #490 (C ABI), #492 (Python), #493
(TypeScript), and #494 (C++ wrapper).

### Breaking

- **Python `client.next_event(timeout_ms)` REMOVED.** Replaced with
  `client.start_streaming(callback)`: events are now delivered to a
  callback rather than pulled one at a time, and the binding wires
  straight through the shared streaming delivery path. The drop
  counter is exposed via `client.dropped_event_count()`. There is
  deliberately no inline-on-the-reader-thread variant: a slow Python
  callback running on the streaming reader thread would back up the
  kernel TCP receive buffer and trigger a vendor-side disconnect, so
  delivery always runs on a dedicated thread.

  Migration:

  ```python
  # Before
  client.start_streaming()
  while True:
      event = client.next_event(100)
      if event:
          process(event)

  # After
  def handler(event):
      process(event)
  client.start_streaming(callback=handler)
  ```

- **TypeScript `client.nextEvent(timeoutMs)` REMOVED.** Replaced with
  `client.startStreaming(callback)`. The dispatcher thread routes
  events through napi-rs `ThreadsafeFunction` to the Node main
  thread; the user's JS callback runs there, decoupled from the
  streaming reader. Migration:

  ```typescript
  // Before
  client.startStreaming();
  while (true) {
    const event = await client.nextEvent(100);
    if (event) process(event);
  }

  // After
  client.startStreaming((event) => process(event));
  ```

  The `droppedEvents()` getter is renamed to `droppedEventCount()`
  and now forwards to the shared streaming delivery path so the value
  matches every other binding. The old intermediate queue is gone from
  the TypeScript binding. The TypeScript binding deliberately does NOT
  expose a `start_streaming_inline` opt-in: Node's libuv requires
  JS callbacks on the main thread, and `ThreadsafeFunction`'s
  internal `uv_async_t` queue is the only safe path.

- **C ABI streaming**: `thetadatadx_client_next_event`, `thetadatadx_streaming_next_event`,
  `thetadatadx_client_start_streaming`, and `thetadatadx_streaming_event_free` REMOVED.
  Replaced with a callback-only surface that wires through the SSOT
  `StreamingDispatcher`:

  - `thetadatadx_client_set_callback(handle, fn, ctx)` /
    `thetadatadx_streaming_set_callback(handle, fn, ctx)` — queued: events flow
    `streaming reader -> bounded 8192-slot channel -> event-delivery
    thread -> user fn`. The reader never blocks on user code; overflow
    events are dropped and counted via `thetadatadx_*_dropped_events`.
  - `thetadatadx_client_set_inline_callback(handle, fn, ctx)` /
    `thetadatadx_streaming_set_inline_callback(handle, fn, ctx)` — inline: user fn
    fires directly on the streaming reader thread (microsecond-budget
    contract, identical semantics to `start_streaming_inline`).

  `thetadatadx_streaming_connect` now defers the streaming TLS connection until the first
  `set_callback` / `set_inline_callback` call (callback registration
  and connect are atomic). The old poll-based receive path is removed
  from the FFI streaming surface.

- **C++ wrapper**: the poll-based `thetadatadx::StreamingClient::next_event` and the
  owning poll-based event-pointer / deleter types are gone. Event
  delivery is now exclusively callback-driven through the new
  `set_callback` / `set_inline_callback` methods (see Added).

### Changed

- **New `StreamingDispatcher` core in the streaming `dispatcher`
  module.** Lock-free bounded 8192-slot channel
  between the streaming reader thread and a dedicated event-delivery thread that
  drains the queue and invokes the user-registered Rust callback.
  Existing `start_streaming(callback)` API now wires through this
  dispatcher transparently — callers see no behavior change. Reader
  thread never blocks on user code; queue overflow drops events with
  a counter.

### Added

- `start_streaming_inline(callback)` — power-user opt-in Rust API.
  Callback fires directly from the streaming reader thread, bypassing the
  dispatcher. Trade: zero queueing overhead (~12 ns/event vs 58 ns
  for the dispatcher path) but slow callbacks block the reader and
  cause vendor disconnects. Documented contract: callback must
  return within microseconds.
- **C++ wrapper callback API.** `thetadatadx::StreamingClient::set_callback
  (std::function<void(const StreamEvent&)>)` for the default queued
  path; `set_inline_callback` for power-user opt-in directly on
  the streaming reader thread. Both wrap the C ABI
  `thetadatadx_streaming_set_callback` / `thetadatadx_streaming_set_inline_callback`
  shipped in this release. The `fpss_smoke` example is restored on
  the callback path.

## [8.0.29] - 2026-05-06

### Removed

- **Go SDK.** The cgo bridge between Rust's C ABI and Go's runtime
  carries per-call overhead that masks the upstream throughput this
  SDK is engineered for. Users who need Go bindings can build their
  own cgo wrapper against the unchanged C ABI in the `ffi` crate —
  header at `sdks/cpp/include/thetadx.h`, all FFI types and free fns
  exported as `thetadatadx_*` symbols.

  Closes #481.

## [8.0.28] - 2026-05-06

### Breaking

- **`Contract`, `OptionContract`, `FlatFileRow`, and `IndexEntry` rename
  `root` to `symbol` and `exp_date` to `expiration` to match the v3
  vendor surface documented in the [v2 → v3 migration guide][v3-mig].
  The wire codec is unchanged — `Contract::to_bytes` /
  `Contract::from_bytes` still serialize the field as `root` per
  the contract codec parity, and the FLATFILES decoder still resolves both
  v2 (`root`) and v3 (`symbol`) response columns through the existing
  `decode::HEADER_ALIASES`. Per-language renames:

  - **Rust** (`thetadatadx::fpss::protocol::Contract`,
    `tdbe::types::tick::OptionContract`,
    `thetadatadx::flatfiles::FlatFileRow`):
    - `Contract.root` → `Contract.symbol`
    - `Contract.exp_date` → `Contract.expiration`
    - `Contract::stock(root)` → `Contract::stock(symbol)`
    - `Contract::index(root)` → `Contract::index(symbol)`
    - `Contract::rate(root)` → `Contract::rate(symbol)`
    - `Contract::option(root, exp_date, …)` →
      `Contract::option(symbol, expiration, …)`
    - `Contract::option_raw(root, exp_date, …)` →
      `Contract::option_raw(symbol, expiration, …)`
    - `OptionContract.root` → `OptionContract.symbol`
    - `FlatFileRow.root` → `FlatFileRow.symbol`
  - **Python** (`thetadatadx.Contract`, `thetadatadx.OptionContract`):
    `contract.root` / `contract.exp_date` →
    `contract.symbol` / `contract.expiration`;
    `OptionContract(root=…)` constructor keyword → `symbol=…`.
  - **TypeScript** (`Contract`, `OptionContract`):
    `contract.root` / `contract.expDate` →
    `contract.symbol` / `contract.expiration`.
  - **Go** (`thetadatadx.Contract`, `thetadatadx.OptionContract`):
    `c.Root` / `c.ExpDate` → `c.Symbol` / `c.Expiration`.
  - **C++** (`OptionContract`, `ThetaDataDxContract`, `ThetaDataDxOptionContract`):
    `c.root` / `c.exp_date` / `c.has_exp_date` →
    `c.symbol` / `c.expiration` / `c.has_expiration`.
  - **C ABI**: `ThetaDataDxContract.root` → `ThetaDataDxContract.symbol`,
    `ThetaDataDxContract.exp_date` → `ThetaDataDxContract.expiration`,
    `ThetaDataDxContract.has_exp_date` → `ThetaDataDxContract.has_expiration`,
    `ThetaDataDxOptionContract.root` → `ThetaDataDxOptionContract.symbol`.
  - **FLATFILES CSV / JSONL**: contract-prefix headers and JSON keys
    change from `root,expiration,strike,right,…` to
    `symbol,expiration,strike,right,…`. Stock blobs go from `root,…` to
    `symbol,…`. The vendor's response columns are unchanged; only the
    SDK's emitted file headers change.
  - **REST / WebSocket / MCP outputs** in `tools/server` and
    `tools/mcp` emit `"symbol"` / `"expiration"` keys on every contract
    payload (option lists, streaming event contracts, FLATFILES rows).

[v3-mig]: https://docs.thetadata.us/Articles/Getting-Started/v2-migration-guide.html#_5-parameter-mapping

### Changed

- Workspace 8.0.27 → 8.0.28, tdbe 0.12.7 → 0.12.8. The tdbe bump rides
  the regenerated `OptionContract.symbol` field; every other change
  ships as patch deltas off the existing v8 line per repo policy.
- `tools/cli` raw column header for `OptionContract` is `symbol`
  instead of `root`, sourced from `tick_schema.toml::field` so future
  schema renames flow through the CLI without a helper edit.

## [8.0.27] - 2026-05-06

### Changed

- **polars 0.52 -> 0.53.** Adopts the new `DataFrame::new` signature in
  the generated frame builders. Closes #464.

### Fixed

- **The `tdbe` conditions table trade condition 61 renamed from
  a third-party product mark to `PRICEVOLUMEADJ`.** Same scrub pattern
  as the v8.0.26 exchange-code-0 rename. Single tracked-source
  occurrence; `rg 'NANEX'` now clean. Description unchanged. Closes
  #476's sibling, filed as #480.

## [8.0.26] - 2026-05-05

### Breaking

- **`GreeksTick` removed in every language and on the C ABI.** The full
  union now ships as `GreeksAllTick` and the per-order endpoints return
  typed subsets. Callers update imports and method return types -- no
  forwarding shim. Renames:
  - Rust / Python / TypeScript / Go / C++ `GreeksTick` -> `GreeksAllTick`
    (full union returned by `option_*_greeks_all` and
    `option_*_greeks_eod`).
  - C ABI free fn `thetadatadx_greeks_tick_array_free` -> `thetadatadx_greeks_all_tick_array_free`.
  - Endpoints now returning `GreeksFirstOrderTick`:
    `option_snapshot_greeks_first_order`,
    `option_history_greeks_first_order`,
    `option_history_trade_greeks_first_order`.
  - Endpoints now returning `GreeksSecondOrderTick`:
    `option_snapshot_greeks_second_order`,
    `option_history_greeks_second_order`,
    `option_history_trade_greeks_second_order`.
  - Endpoints now returning `GreeksThirdOrderTick`:
    `option_snapshot_greeks_third_order`,
    `option_history_greeks_third_order`,
    `option_history_trade_greeks_third_order`.
  - `GreeksAllTick` adds `bid`, `ask`, `underlying_ms_of_day`,
    `underlying_price` columns the upstream OpenAPI publishes but the
    legacy `GreeksTick` did not carry. Field offset of every existing
    Greek shifts by 16 bytes (bid + ask) on the FFI mirror; rebuild any
    binary that links the C struct.

### Added

- **Per-endpoint typed Greeks structs.** The vendor's
  `option_*_greeks_first_order`, `_second_order`, `_third_order`
  endpoints emit strict subsets of the full Greek column set. The SDK
  now exposes `GreeksFirstOrderTick` (delta / theta / vega / rho /
  epsilon / lambda + bid/ask + IV pair + underlying snapshot),
  `GreeksSecondOrderTick` (gamma / vanna / charm / vomma / veta + bid/
  ask + IV pair + underlying snapshot), and `GreeksThirdOrderTick`
  (speed / zomma / color / ultima + bid/ask + IV pair + underlying
  snapshot). Each per-order endpoint returns `Vec<<Type>>` directly --
  no zero-default columns leak from one subset into another.
- New C ABI free symbols: `thetadatadx_greeks_all_tick_array_free`,
  `thetadatadx_greeks_first_order_tick_array_free`,
  `thetadatadx_greeks_second_order_tick_array_free`,
  `thetadatadx_greeks_third_order_tick_array_free`. Matching FFI array types
  emitted on every binding (`ThetaDataDxGreeksAllTickArray`,
  `ThetaDataDxGreeksFirstOrderTickArray`, etc.).
- New header alias `underlying_ms_of_day` -> `underlying_timestamp` in
  the decode `HEADER_ALIASES` table so the wire
  Timestamp -> ms-of-day conversion flows through the standard
  `row_number` path on every Greeks endpoint.
- Per-field `offset_of!` layout assertions in
  the `tdbe` tick `layout_asserts`. Field-offset drift
  (e.g. swapping two same-size fields) sneaks past `size_of` /
  `align_of` checks alone -- the new asserts pin every observable Rust
  field offset that the C / Go FFI mirrors index into.

### Changed

- **Generated `tdbe` tick struct definitions.** The tick struct
  definitions are now generated from `tick_schema.toml`. The
  hand-written code keeps the contract-id implementations, the
  `TradeTick` flag helpers, and `OptionContract::is_call` / `is_put`;
  everything else flows from the schema. Adding a new tick type means
  adding one `[types.X]` row.
- **Schema-driven C++ layout asserts and Go FFI sizes.** The generated
  C++ layout asserts and Go FFI size checks now compute every struct's
  size and alignment from the schema rather than dispatching on the
  type name. Adding a tick type to `tick_schema.toml` produces the
  size/align pair, the C++ `static_assert`, and the Go size test entry
  with no per-language generator edit.
- `OpenInterestTick` and `TradeQuoteTick` gained the missing `align = 64`
  directive in `tick_schema.toml`. The schema now matches the 64-byte
  cache-line alignment declared on the corresponding `tdbe` types --
  the schema-derived FFI size used to under-count by the alignment
  rounding (32/144 vs 64/192) before reaching the C++ layout assert.
- Exchange code 0 in the `tdbe` exchange table renamed from a
  third-party product mark to neutral SIP terminology (`Composite`).
  Symbol stays `COMP`; wire byte 0 still resolves to the same array
  slot. Closes #476.

### Fixed

- **`option_*_greeks_*_order` no longer spams `expected column header
  not found` warnings.** Decoding logged a warning every time a parser
  asked for an optional column that was absent from the wire response.
  The Greeks family splits the column set across the wire —
  `_greeks_first_order` ships seven Greeks, `_greeks_second_order`
  ships five, and `_greeks_third_order` ships four — but the shared
  `GreeksTick` schema carries the full 23-Greek union. Calling
  `option_snapshot_greeks_third_order` therefore produced eight warning
  lines per response (zomma, color, ultima, d1, d2, dual_delta,
  dual_gamma, vera, …) before any user-visible decoding finished. That
  diagnostic is now emitted at trace level, so it is still reachable
  via `RUST_LOG=thetadatadx=trace` for genuine schema-drift
  investigations but stays out of stderr on routine subset calls.
  Required-column drift continues to surface as a typed
  `Error::MissingRequiredHeader`. Closes #472.

### Changed

- **Per-tick-type binding names are TOML-driven instead of hand-coded.**
  Every per-language name a tick type needs across the bindings (Rust
  return type, generated parser, Go struct and converter, FFI array
  struct and free function, C++ value type, the Python converters and
  pyclass name, and the TypeScript class and converter) moves into
  `[types.X.render]` blocks in `tick_schema.toml`. Adding a tick type
  now requires one TOML row and no per-language edits. The generated
  SDK surfaces are byte-identical against `main` because the TOML rows
  reproduce the names that were previously hardcoded.
- Per-endpoint vendor-schema column lists for the four Greeks families
  pinned and documented in `tick_schema.toml::GreeksTick` against the
  upstream OpenAPI capture in `scripts/upstream_openapi.yaml`. The
  `GreeksTick` struct itself is unchanged — every Greeks endpoint still
  returns `Vec<GreeksTick>` and the union layout is the same — but the
  schema doc-comment now spells out which Greeks each endpoint
  publishes, why the others zero-default, and where the per-endpoint
  vendor schema is captured. The codegen pickup is doc-only; no SDK
  surface drift.
- Three new unit tests drive Greeks decoding against the
  `_first_order`, `_second_order`, and `_third_order` wire shapes
  (column lists pinned to upstream OpenAPI). Each test asserts
  bit-exact decoded values for the wire-present columns and `0.0`
  defaults for the documented gaps, so a future regression back to
  warning-level log spam — or any column-list drift in either
  direction — surfaces as a behavioural test failure.
- `tdbe` 0.12.6 → 0.12.7.

## [8.0.25] - 2026-05-05

### Fixed

- **Windows `ERROR_IO_PENDING` (os error 997) no longer trips a fatal
  streaming read error.** On Windows the overlapped socket layer surfaces
  in-flight reads as `ERROR_IO_PENDING` instead of `WSAEWOULDBLOCK`,
  which the streaming read path treated as fatal: Python users on
  Windows saw `FPSS read error error=IO error: Overlapped I/O
  operation is in progress. (os error 997)` spam followed by a
  reconnect storm. The read path now treats os error 997 as a
  transient read alongside `WouldBlock` and `TimedOut`, so it drains
  queued commands and retries the way it does on Linux and macOS.
  Closes #469.

### Changed

- `tdbe` 0.12.5 → 0.12.7.

## [8.0.24] - 2026-05-04

### Added

- `tdbe::greeks::vera` — public free function exposing the DvegaDr
  formula `-K * exp(-r*T) * T * sqrt(T) * phi(d2)` so callers can pull
  the single Greek without computing the full bundle.
- `tdbe::greeks::compute_full_bundle_with_iv(s, x, v, r, q, t, is_call)`
  — full `GreeksResult` computation that skips the bisection IV solver
  and uses a caller-supplied volatility. Tier-0 intermediates are shared
  across every Greek in the bundle; ~2× faster than computing the
  Greeks one at a time. Typical use case is the IV-cache hot path. Takes
  `is_call: bool` rather than `&str right` because callers in this
  path have already parsed the side; `all_greeks` and
  `implied_volatility` keep the `&str right` surface.

### Changed

- `tdbe` 0.12.4 → 0.12.5.

## [8.0.23] - 2026-05-01

### Fixed

- **REST + MCP no longer return empty bodies on serialisation failure.**
  Both previously swallowed a JSON-serialization error, producing a
  `200 OK` with an empty body (REST) or a successful but empty
  `tools/call` result (MCP) when a tick payload contained a non-finite
  f64 cell. The REST handler now surfaces the failure as a structured
  `500` carrying the underlying error message in the existing JSON
  envelope, and the MCP handler returns a JSON-RPC `-32603` Internal
  Error. The cross-language non-finite f64 -> JSON `null`
  canonicalisation rule is now shared by the CLI, REST, and MCP
  frontends so all three produce byte-identical output for the same
  payload.
- **Streaming WebSocket broadcast queue is now bounded.** The server's
  broadcast queue was previously unbounded and could grow without
  limit if the broadcast task lagged behind the feed. It is now
  bounded at 65 536 slots; on overflow an event is dropped rather than
  buffered indefinitely, drops are counted and exposed via
  `GET /v3/system/streaming/status` as `broadcast_dropped`, and a warning is
  logged once per 1024 drops to surface back-pressure without flooding
  stderr.
- **`ThetaDataDx::reconnect_streaming` now fails explicitly on partial
  re-subscription.** On reconnect the unified client previously logged
  re-subscribe failures and returned `Ok(())`, hiding partial
  reconnects from programmatic callers. It now collects every failed
  subscription and, when any failed, returns the new
  `Error::PartialReconnect { failed }` variant. The per-failure
  warning lines stay for operational visibility.
- **Version metadata drift cleared.** `sdks/cpp/CMakeLists.txt`
  (`8.0.9` -> `8.0.23`) and the workspace comment in `Cargo.toml`
  (`7.x` -> `8.0.x`) now match the rest of the v8 line. Banned
  vocabulary purged from `SECURITY.md`, the `[8.0.8]` changelog entry,
  and the dropped-events Python test.

### Added

- `json_canon` — a small shared module exposing
  `finite_or_null`, `canonicalize`, and `canonicalize_and_serialize`
  for non-finite f64 -> JSON `null` collapse, surfacing the
  serialisation error rather than swallowing it. Shared by the CLI,
  server, and MCP frontends.
- New `Error::PartialReconnect { failed: Vec<(SubscriptionKind, Contract)> }`
  variant in `thetadatadx::error` and a new
  `Contract::full_type_marker(sec_type)` constructor used to encode a
  failed full-type subscription inside the structured failure list.
- `AppState::record_fpss_broadcast_drop` and
  `AppState::fpss_broadcast_dropped` on the REST server, and a new
  `broadcast_dropped` field on `GET /v3/system/streaming/status`.
- A pytest CI step in `.github/workflows/python.yml` that runs
  `pytest sdks/python/tests/` after the wheel install. The existing
  import smoke is kept as a separate step.

### Changed

- The CLI's f64-to-JSON rule now delegates to the shared
  canonicalisation helper.
- The server's CSV value rendering canonicalises before serialising and
  emits a `<csv-render-error: …>` sentinel rather than an empty cell on
  serialisation failure.
- `tdbe` bumped to `0.12.4` to keep all member crates on a fresh
  patch line for the 8.0.23 release.

## [8.0.22] - 2026-05-01

### Fixed

- **`fpss::accumulator::change_price_type` now matches the JVM terminal's
  price-type rescale byte-for-byte.** v8.0.21 widened the
  multiplication through `i64` and returned the unscaled input on
  overflow, which broke parity with the JVM terminal (its `i32 * i32`
  rescale silently wraps under two's-complement). The rescale now uses
  `i32::wrapping_mul`, reproducing the JVM terminal's exact wire bits in both
  debug and release. Tests pin the wrapping result to manually-computed
  reference values (e.g. `2_148 * 10^6 → -2_146_967_296`).
- **`decode::row_number_i64` clamps `price_type` to `0..=19`** so the
  same wire cell decodes identically through `row_number_i64` and
  `row_price_f64` (the latter routes through `tdbe::Price::new`, which
  has clamped to that range since it was introduced). Under the clamped
  contract, `i32::MAX * 10^9 ≈ 2.15e18` is well below `i64::MAX`, so
  scale-up cannot overflow and the previous `Price overflowing i64`
  error path is no longer reachable.

### Added

- `tests/flatfiles_synthetic_golden.rs` — a
  deterministic decoder-only golden test that builds a synthetic
  FLATFILES blob (header + INDEX + FIT-encoded DATA) in Rust and pins
  the CSV writer's output byte-for-byte. Runs in plain `cargo test`
  with no live wire and no env var, giving CI a hard regression gate
  on the FIT decoder, INDEX walker, and price formatter on every push.

### Changed

- Documentation references to "22 Greeks" updated to "23 Greeks" to
  reflect the `vera` field added in v8.0.21. Touches the `tdbe`
  README + module docs, the `thetadatadx` README tick-types table,
  and the Python and C++ SDK READMEs.

## [8.0.21] - 2026-04-30

### Fixed

- **Price rescaling on the streaming feed no longer overflows `i32`.**
  A rescale whose result does not fit `i32` now returns the original
  price unchanged and logs a warning with the price and the source and
  target price types, rather than silently saturating or panicking. A
  live BRK.A wire integer in cents (71_396_865) rescaled to
  `price_type=4` is the canonical trigger.
- **`decode::row_number_i64` no longer routes `Price` cells through
  `f64`.** Large integer fields delivered as `Price` cells now decode
  with i64-native scaling (`checked_pow` / `checked_mul`), preserving
  every ULP past `2^53`. Scale-ups that overflow `i64` surface as
  `DecodeError::TypeMismatch { expected: "i64-fitting Price",
  observed: "Price overflowing i64" }` rather than a saturated `f64 as
  i64`.

### Changed

- **`tdbe::greeks::all_greeks` and `tdbe::greeks::implied_volatility`
  now return `Result<_, tdbe::Error>`.** Both helpers previously
  panicked when `right` did not parse as a single side. They now
  return `tdbe::Error::Config` for unrecognised or wildcard rights.
  Every in-repo call site (`ffi`, `tools/cli`, `tools/mcp`,
  `sdks/python`, `tdbe` benches) was updated. Direct callers of
  these helpers must add `?` or `.expect()`.
- **`tdbe::greeks::GreeksResult` gained a `vera: f64` field** computed
  inside `all_greeks`. Vera (a.k.a. DvegaDr) is the textbook
  cross-sensitivity of vega to the risk-free rate: `-K * exp(-r*T) * T
  * sqrt(T) * phi(d2)`. The downstream `ThetaDataDxGreeksResult` C-ABI struct,
  the C++/Go/Python SDK Greeks structs, and the CLI/MCP output objects
  all carry the new field.

### Added

- `tests/flatfiles_byte_match.rs` gained a second
  test, `option_eod_csv_byte_matches_vendor`, that pulls OPTION/EOD
  for `20260428` and byte-matches against a vendor reference CSV
  pointed to by a new env var, `THETADATADX_REFERENCE_EOD_CSV`. The
  EOD path exercises the CSV price formatter end-to-end against
  vendor output — the existing OPEN_INTEREST byte-match did not, since
  OPEN_INTEREST has no price columns. The test skips when the
  reference CSV is missing; the doc comment documents the regeneration
  recipe.
- `live.yml` gained a `cargo test --features live-tests --test
  flatfiles_byte_match` step in the live `smoke` job. It skips
  gracefully when the reference CSV is not provisioned for the runner.

## [8.0.20] - 2026-04-30

### Fixed

- **FLATFILES price decoding was off by powers of ten across every output
  format.** The CSV / JSONL writers and the typed in-memory `FlatFileRow`
  return path were dividing the wire integer by `10^N` where N was read
  directly from the row's `PRICE_TYPE` column. The vendor convention is
  `real_price = value * 10^(price_type - 10)` (see
  [`tdbe::types::price::Price`]), so for `price_type = 8` (cents) the
  correct factor is `0.01` (i.e. `value / 10^2`), not `value / 10^8` —
  off by **10^6**. Effect: option `bid` / `ask` / `price` columns came
  out near-zero (e.g. `1.9e-6` instead of `1.90`), and the CSV
  `{:.4}`-formatted output rounded those to `0.0000`. Every consumer of
  the flat-file pipeline was affected.
- The CSV price formatter no longer hardcodes 4 fractional digits.
  Rust's default `f64` Display now preserves the full IEEE-754
  precision the wire decoder produced, so micro-priced contracts
  survive the on-disk round-trip.

### Changed

- `flatfiles::writer::price_divisor` (private API) replaced with
  `price_type_for_row` + `decode_price`. Both new helpers route price
  decoding through `tdbe::types::price::Price::to_f64()`, which is the
  authoritative implementation of the ThetaData price convention.
- The `OPTION/OPEN_INTEREST` byte-match integration test still passes —
  open-interest rows have no price columns, so the test never exercised
  the broken formatter. A new unit test
  (`decode_price_uses_vendor_semantics`) locks the corrected
  behaviour, and `fmt_price_preserves_full_precision` asserts that
  micro-priced rows do not round to zero.

## [8.0.19] - 2026-04-30

### Changed

- `tools/mcp` replaced `Arc<RwLock<Option<ThetaDataDx>>>` with
  `Arc<OnceCell<ThetaDataDx>>`. The JSON-RPC handler no longer holds
  a read guard across awaited tool execution; `OnceCell::get` is
  lock-free.
- `tools/cli` `get_arg()` now uses `unreachable!()` with an explicit
  invariant comment. All call sites declare the argument with clap's
  `required(true)`, so the `None` branch indicates a clap config bug,
  not user input.

### Added

- `tdbe::FitRows::get()` returns `Option<&[i32]>` for
  caller-supplied indices. The existing `FitRows::row()` keeps its
  panic-on-OOB contract with a clearer message.

## [8.0.18] - 2026-04-30

### Fixed

- Workspace version drift: `ffi`, `tools/cli`, `tools/server`, and
  `tools/mcp` were pinned at 8.0.15 while the SDK crates moved
  through 8.0.16 → 8.0.17. Every Rust crate, every npm
  `package.json`, and the TypeScript `package-lock.json` now report
  a single 8.0.18 surface.
- `Contract::to_bytes()` no longer panics on caller input. New
  `Contract::validate()` returns a typed `Result<(), Error::Config>`;
  new `Contract::try_to_bytes()` is the fallible encoder.
  `build_subscribe_payload()` now returns `Result<Vec<u8>, Error>`,
  and `StreamingClient::{subscribe, unsubscribe}` validate before
  encoding. The reconnect re-subscribe loop logs and skips invalid
  contracts instead of failing the whole reconnect. Net: malformed
  roots flow back as `Error::Config` to every binding instead of
  crashing the process.
- `SessionToken::refresh` no longer holds an async mutex across the
  Nexus `authenticate_at(...).await`. Replaced with
  an async read-write lock for state plus a separate async mutex
  for refresh dedup. Concurrent `snapshot()` / `current_uuid()`
  readers continue against the previous (still-valid) UUID
  throughout.

## [8.0.17] - 2026-04-30

### Added

- **FLATFILES** — third public surface alongside historical and streaming.
  Pulls one whole-universe INDEX + DATA blob per
  `(SecType, ReqType, date)` tuple from
  `nj-{a,b}.thetadata.us:12000` over a TLS PacketStream protocol
  distinct from historical gRPC and streaming. Server identity pinned
  to the production keypair via `MddsSpkiVerifier`. Login:
  CREDENTIALS + VERSION → SESSION_TOKEN + METADATA, with PING
  heartbeats during auth tolerated and terminal login errors
  short-circuiting host retry. The raw download path uses async
  file I/O with a 1 MB BufWriter; decode + write run on worker threads
  off the async I/O path so streaming / historical tasks on the same
  runtime do not stall.
- The `thetadatadx::flatfiles` module: `framing`, `mdds_spki`,
  `session`, `request`, `index`, `decode`, `decoded`, `decoded_row`,
  `format`, `types`, `writer`, `datatype` submodules.
- Three free-function entry points: `flatfile_request`,
  `flatfile_request_decoded`, `flatfile_request_raw`. Mirror methods
  on the unified `Client` client. Convenience methods for the
  option / stock × `{open_interest, trade_quote, trade, quote, eod}`
  matrix.
- Public types: `FlatFileFormat::{Csv, Jsonl}`, `SecType`, `ReqType`,
  `FlatFileRow`, `FlatFileValue`, `FlatFilesUnavailableReason`.
- `examples/flatfile_demo.rs` end-to-end CLI
  example.
- `tests/flatfiles_byte_match.rs` live integration
  test (`live-tests` feature gate) that byte-matches CSV output
  against the vendor reference output.

### Changed

- `tdbe` 0.12.0 → 0.12.1. Republishes the SSOT-generator enum surface
  (`Interval`, `RequestType`, `Version`) so `thetadatadx` publish
  resolves on crates.io.

### Notes

- Cross-language coverage of FLATFILES (CLI, MCP, REST/WS server,
  FFI, Python, TypeScript, Go, C++) is tracked in the issue tracker;
  the Rust core is shipped today. See `ROADMAP.md` for the binding
  coverage matrix.

## [8.0.16] - 2026-04-30

### Added

- `thetadatadx::utils` namespace exposes `conditions`, `exchange`,
  `sequences` for tick post-processing without a separate `tdbe`
  dependency.
- Re-exports at the `thetadatadx` crate root for every tick struct
  returned by an SDK method (`CalendarDay`, `EodTick`, `GreeksTick`,
  `InterestRateTick`, `IvTick`, `MarketValueTick`, `OhlcTick`,
  `OpenInterestTick`, `OptionContract`, `PriceTick`, `QuoteTick`,
  `TradeQuoteTick`, `TradeTick`), the enums named on those structs
  (`DataType`, `Interval`, `RateType`, `RemoveReason`, `RequestType`,
  `Right`, `SecType`, `StreamMsgType`, `StreamResponseType`, `Venue`,
  `Version`), `Price`, and the offline Greeks helpers (`all_greeks`,
  `implied_volatility`, `GreeksResult`).

### Changed

- `ROADMAP.md` aligned with the 2026-04-20 validator run (127 PASS /
  7 subscription-tier-blocked / 0 FAIL) and the 2026-04-29 / 04-30
  FLATFILES live run.

## [8.0.15] - 2026-04-24

### Fixed

- Linux wheel tag moved from `manylinux_2_38` to `manylinux_2_17` so
  the published `thetadatadx-*-manylinux_2_17_x86_64.whl` installs on
  every glibc 2.17+ runtime (CentOS 7 / RHEL 7+ / Ubuntu 18.04+ /
  Debian 10+ / Google Colab / Databricks). The v8.0.14 wheel was
  built on `ubuntu-latest` (now Ubuntu 24.04 / glibc 2.38), which
  silently gated every older environment — `pip install thetadatadx`
  would fall through to the sdist and fail the source build because
  Rust is not available on most hosted Python runtimes.

### Changed

- `.github/workflows/python.yml` Linux wheel step now uses
  `PyO3/maturin-action@v1` with `manylinux: '2014'` (glibc 2.17
  toolchain inside a Docker container). macOS and Windows continue
  to build natively on their matrix runners.

## [8.0.14] - 2026-04-23

### Fixed

- Re-publish the v8.0.13 chain to crates.io and GitHub Releases. The
  v8.0.13 tag CI failed on the `Extended Surfaces` docs-consistency
  gate because the squash merge of #412 captured an intermediate
  branch state (top-level `CHANGELOG.md` had the final wording while
  the mirrored `docs-site/docs/changelog.md` still had the pre-cp
  wording). PyPI and npm published v8.0.13 successfully; crates.io
  and the GitHub Release did not. v8.0.14 re-publishes everything
  from the synced main tip. No behavior change vs v8.0.13.

## [8.0.13] - 2026-04-23

### Fixed

- Mid-stream chunk header drift in the historical response accumulator was
  silently masked: `HistoricalClient::collect_stream` / `for_each_chunk` would
  keep the first chunk's `headers` and pile subsequent chunks' rows
  underneath, even if a later chunk carried a different non-empty
  header set. A server-side schema change mid-response would therefore
  surface as silent data corruption instead of an error. Both paths
  now compare the saved first-chunk schema against every non-empty
  chunk header set and raise a new `DecodeError::ChunkHeaderDrift`
  on mismatch (P13 from the external bench handoff).

### Added

- `decode::DecodeError::ChunkHeaderDrift { chunk_index, first, chunk }`
  variant.

### Known

- **`option_at_time_quote` 0.67× vs vendor** (bench handoff §8 #1).
  The v8.0.5 uniform `mdds_query_field_expr` rule that empties the
  top-level `expiration` field on any option query carrying a
  `ContractSpec` may have flipped this specific endpoint into a
  slower server-side path. Needs a bench-validated per-endpoint
  override in `endpoint_surface.toml`. Not fixed in this release
  because a speculative generator carve-out without bench
  re-validation would risk regressing the other option endpoints
  that benefit from the current rule.
- **`option_history_greeks_eod` 0.704× vs vendor** (bench handoff §8
  #2). Persistent across v8.0.0 / v8.0.4 / v8.0.10. Likely server-
  side per-contract aggregation path rather than a wire-shape
  issue; needs proto-level diff against the other
  `option_history_greeks_*` endpoints (which are DX wins at
  4-6× faster).

## [8.0.12] - 2026-04-23

### Removed

- `scripts/test_drift_injection.sh` + the `streaming drift injection` CI job
  (`.github/workflows/ci.yml`). The test was designed when the C++
  `static_assert(offsetof)` guards in `thetadx.hpp` were hand-maintained
  against a Rust-generated C struct layout. v8.0.11 moved both sides
  under the same SSOT generator, so swapping a field in
  `fpss_event_schema.toml` regenerates the C struct and the assert
  value in lockstep and the assertion can no longer fail. Removed
  rather than kept as a misleading safety net; `regen_byte_identical`
  covers generator consistency and the assertions still fire at C++
  compile time against hand-committed C header corruption.

## [8.0.11] - 2026-04-23

### Added

- `endpoint_surface.toml` now declares the endpoint-surface enums used by
  `right`, `venue`, `interval`, `rate_type`, `request_type`, and
  `version`. The generator emits the Rust `tdbe` enums, Python enum
  pyclasses, and the TypeScript napi string enums from the same TOML
  variant lists.
- Go now gets generator-owned FFI drift artifacts for every checked size
  and offset: `endpoint_ffi_sizes_generated.go`,
  `tick_ffi_sizes_generated.go`, `fpss_ffi_sizes_generated.go`,
  `ffi_layout_generated_test.go`, and
  `fpss_ffi_offset_checks_generated.go`.
- C++ now gets generator-owned layout assertion includes:
  `tick_layout_asserts.hpp.inc` from `tick_schema.toml` and
  `fpss_layout_asserts.hpp.inc` from `fpss_event_schema.toml`.
- `.github/release-notes/v8.0.11.md` records the SSOT refactor and local
  verification plan for this release.

### Changed

- The `tdbe` enums module now includes generator-emitted
  endpoint-surface enums instead of hand-maintaining `Right`, `Venue`,
  `Interval`, `RateType`, `RequestType`, and `Version`.
- The Python binding now includes generator-emitted enum pyclasses
  instead of a hand-maintained enum block.
- `sdks/go/tick_ffi_mirrors.go` no longer embeds hand-maintained expected
  sizes or streaming offset literals; it consumes generator-owned constants and
  offset tables.
- `sdks/go/ffi_layout_test.go` has been replaced by the generated
  `sdks/go/ffi_layout_generated_test.go`, so the Go tick-layout drift
  detector now reads its expected values from TOML-derived generation.
- `sdks/cpp/include/thetadx.hpp` now includes generated layout assertion
  fragments instead of hand-maintaining `static_assert(sizeof(...))` and
  `static_assert(offsetof(...))` blocks.
- Live docs and READMEs no longer hardcode endpoint, tick-type, or tool
  counts; they describe the generated surface instead.
- Release metadata bumps `8.0.10 -> 8.0.11` across `thetadatadx`,
  `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-server`,
  `thetadatadx-mcp`, `thetadatadx-py`, and `thetadatadx-napi`. TypeScript
  package metadata, loader version guards, and the checked-in OpenAPI
  version now match `8.0.11`.
- `tdbe` stays at `0.12.0`.

## [8.0.10] - 2026-04-23

### Added

- `endpoint_surface.toml` now carries upstream-verified defaults for
  every builder-bound optional param that the ThetaData OpenAPI spec
  documents as optional with a server-side fallback: `venue = "nqb"`,
  `rate_type = "sofr"`, `version = "latest"`, `exclusive = true`,
  `use_market_value = false`, `underlyer_use_nbbo = false`. These flow
  through the `parsed_endpoint!` macro as the initial builder value, so
  callers that omit the field hit the same wire payload the official
  Python library produces — no per-endpoint runtime fallback needed.
- Parameter descriptions in the SSOT now enumerate accepted values for
  `venue`, `rate_type`, `version`, `exclusive`, `use_market_value`, and
  `underlyer_use_nbbo`, which propagates into the per-language generator
  outputs (Rust docstrings, Go `endpoint_options.go`, C++
  `endpoint_options.hpp.inc`, Python builder docstrings).
- SSOT defaults now cover `right = "both"`, `strike = "*"`, and
  `interval = "1s"`. The option contract endpoints no longer require
  `right` and `strike` as positional Rust method arguments; callers set
  concrete values through the existing options builder fields when they
  need to override the server defaults.
- Python bindings expose module-level `Right`, `Venue`, `Interval`,
  `RateType`, `RequestType`, and `Version` string enum classes. Enum
  constrained parameters accept either plain strings or those enum
  objects.
- TypeScript declarations expose matching literal-union types and const
  companions for `Right`, `Venue`, `Interval`, `RateType`, `RequestType`,
  and `Version`.

### Changed

- The `venue=nqb` default moved from a runtime constant
  (`wire_semantics::DEFAULT_STOCK_VENUE`) into the SSOT, making
  `endpoint_surface.toml` the single source of truth for every
  parameter default across every emitter. The generator's query-
  assembly path now wraps default-bearing `Str` fields in `Some(...)`
  when marshalling into the proto request, keeping the wire shape
  identical to the previous release.
- `collapse_redundant_wires` in the build-time mode matrix now reads
  per-endpoint SSOT defaults instead of the hardcoded `venue=nqb`
  branch, so future additions to the default set automatically collapse
  their redundant `with_<name>` validator cells.
- Release metadata bumps 8.0.9 -> 8.0.10 across every Rust crate
  (`thetadatadx`, `thetadatadx-ffi`, `thetadatadx-cli`,
  `thetadatadx-server`, `thetadatadx-mcp`, `thetadatadx-py`,
  `thetadatadx-napi`), every TypeScript package (`sdks/typescript` root
  plus the three platform subpackages under `sdks/typescript/npm/`),
  the TypeScript native binding version guard in
  `sdks/typescript/index.js`, and the OpenAPI contract in
  `docs-site/public/thetadatadx.yaml`.
- `tdbe` stays at `0.12.0`; the encoding crate is untouched.
- Rust, Python, TypeScript, Go, and C++ endpoint surfaces now project
  proto `repeated string symbol` endpoints as bulk-capable symbol inputs.
  Singular-symbol wire endpoints remain singular.
- Python historical date parameters (`date`, `expiration`, `start_date`,
  `end_date`) accept `str`, `datetime.date`, or `datetime.datetime`.
  Python time parameters (`start_time`, `end_time`, `min_time`,
  `time_of_day`) accept `str` or `datetime.time`.
- TypeScript historical date and time parameters accept either `string`
  or JavaScript `Date` values at the native binding boundary.

## [8.0.9] - 2026-04-23

### Fixed

- The TypeScript package lock now matches `package.json` for version,
  license, Node engine, and platform optional dependency pins.
- The requested repo-root `scripts/regen_byte_identical.sh` gate now
  delegates to the checked-in generator determinism harness, and the docs
  consistency and tier badge scripts are executable.
- User-facing docs and release notes no longer point at deleted
  `thetadatadx` modules or removed streaming shortcut APIs.
- `CHANGELOG.md` and `docs-site/docs/changelog.md` use only the
  Keep-a-Changelog section buckets and avoid banned performance phrasing.

### Changed

- Release metadata now points at `8.0.9` across Rust crates, the
  TypeScript root package and platform packages, the TypeScript native
  binding version guard, the C++ package metadata, and the checked-in
  OpenAPI contract.
- Every Rust crate version bumps `8.0.8 -> 8.0.9`: `thetadatadx`,
  `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-server`,
  `thetadatadx-mcp`, `thetadatadx-py`, `thetadatadx-napi`.
- `sdks/typescript/package.json` and every platform subpackage under
  `sdks/typescript/npm/` bump to `8.0.9` so the npm dependency graph
  stays coherent.
- `tdbe` stays at `0.12.0`; this patch is metadata, docs, and tooling
  hygiene only.

## [8.0.8] - 2026-04-23

Follow-up patch to v8.0.7. Addresses the review findings surfaced against
the code-strip release: rustdoc breakage inside `tdbe`, TypeScript loader
and subpackage versions drifting from the root package, a `[8.0.7]`
changelog section that accidentally absorbed v8.0.6 content, stale
references to removed modules, and a handful of doc
inaccuracies around DataFrame terminals and SDK parameter names. No
behaviour changes; every item is documentation, packaging metadata, or
tooling hygiene.

### Fixed

- The `tdbe` FIT codec module — broken intra-doc link on
  `FitReader`'s module-level docstring now resolves via
  `[FitReader::read_changes]`.
- The `tdbe` right module — five redundant explicit link targets on
  `[Error::Config]` references dropped; rustdoc resolves the bare path
  against the in-scope `use crate::error::Error`.
- `sdks/typescript/index.js` — native-binding version guard now compares
  against `'8.0.8'` (was stale sentinel `'8.0.0'`). Mismatched binaries
  are caught when `NAPI_RS_ENFORCE_VERSION_CHECK` is set.
- `sdks/typescript/package.json` — `optionalDependencies` pin each
  platform subpackage to `8.0.8` (was `8.0.4`). The three published
  subpackages (`thetadatadx-linux-x64-gnu`, `thetadatadx-darwin-arm64`,
  `thetadatadx-win32-x64-msvc`) bump from `8.0.7` to `8.0.8` in lockstep.
- `CHANGELOG.md` / `docs-site/docs/changelog.md` — v8.0.6 content
  (snapshot fast-path, Rust `frames` module) split back out of the
  v8.0.7 section into a standalone `[8.0.6]` entry; the `### Changed`
  bucket on v8.0.6 was renamed `### Changed` to stay within the Keep a
  Changelog vocabulary.
- `docs/api-reference.md` — two references to the old `tdbe` error
  module repointed to `tdbe::error`.
- The internal parity-tracking checklist — stale normalization-module
  path updated to `mdds/endpoints.rs`, the current home of
  `normalize_interval` after the v8.0.7 fold.
- The `wire_semantics` module — stale normalization-module
  parenthetical removed from the module docstring.
- `docs-site/docs/api-reference.md` — DataFrame-terminals section
  narrowed: `.to_pandas()` / `.to_polars()` / `.to_arrow()` are
  available on the `<TickName>List` list-wrapper return types;
  snapshot-fast-path endpoints return a plain `list[TickClass]` and do
  not carry the chainable terminals.
- `sdks/python/README.md`, `sdks/go/README.md`, `sdks/cpp/README.md` —
  parameter-name tables now use the canonical SSOT names
  (`expiration`, `start_date`, `end_date`) instead of the `exp`,
  `start`, `end` shorthand.

### Changed

- `docs-site/docs/.vitepress/config.ts` — `vite.build.chunkSizeWarningLimit`
  raised to `1500` kB. The docs site bundles Mermaid and Vue chunks that
  exceed the default 500 kB threshold; the warning was non-actionable.
- `deny.toml` — unused license allowances pruned from `[licenses].allow`;
  remaining entries carry a short comment explaining why each is there.
  `cargo deny check` now produces zero warnings.

## [8.0.7] - 2026-04-23

Code-strip release. No new features. Every item removes dead or
near-dead code, narrows module visibility, or consolidates parallel
FFI surfaces. `tdbe` bumps to `0.12.0` (public module removed).

### Removed

- Historical normalization forwarding layer over `crate::wire_semantics`. The
  three wire canonicalizers
  (`normalize_expiration`, `wire_strike_opt`, `wire_right_opt`) stay
  at `crate::wire_semantics`; the historical-scoped `normalize_interval`,
  `normalize_time_of_day`, and `contract_spec!` macro move next to
  their generated consumers in the `mdds::endpoints` module.
- `fpss::session::reconnect` — 90 LOC public function, zero callers.
  `ThetaDataDx::reconnect_streaming` remains the reconnect entry point.
  `reconnect_delay` is kept (used by `fpss::decode`).
- The crate-local right-parser re-export shim was removed.
  `parse_right`, `parse_right_strict`, and `ParsedRight` stay at the
  crate root via a direct `pub use tdbe::right::*`.
- The unreachable retry helper trio and the crate-level
  `#![allow(dead_code)]` attribute that masked them were removed.
  `StatusClass` moved into `macros.rs` as a private enum.
- The `tdbe` `errors` module — folded into `tdbe::error`. The two
  used items (`HTTP_STATUS_CODE_KEY`, `error_from_http_code`) are now
  reachable at `tdbe::error::*`; the unused `error_name` helper and
  the `errors` module itself are gone.
- 24 `StreamingClient` / `Client` per-security shortcut methods (and
  their unsubscribe twins). Callers use the
  `Contract`-taking `subscribe_quotes` / `subscribe_trades` /
  `subscribe_open_interest` methods directly.
- 61 `HistoricalClient::<endpoint>_with_deadline` sibling methods on every
  list endpoint. Per-call deadlines route through
  `EndpointArgs::with_timeout_ms` (FFI / Python / TS / Go / C++) or
  the builder `.with_deadline(Duration)` setter on parsed endpoints.
  SDK generators now wrap the bare call in a local deadline timeout
  instead of calling the deleted `_with_deadline` variant.
- 61 `thetadatadx_<endpoint>` (no-options) FFI entry points. The C++ SDK
  already calls the `thetadatadx_<endpoint>_with_options` variants, so the
  plain-name declarations in `sdks/cpp/include/thetadx.h` and the
  hand-written historical FFI wrappers are gone.
- The protobuf-crate re-export at the `thetadatadx` crate root.
  Downstream consumers that need the protobuf `Message` trait
  (`sdks/python`) now pull the crate in as a direct dependency pinned
  to the same `=0.14.3` version.
- `HistoricalClient::raw_query`, `HistoricalClient::raw_query_info`,
  `HistoricalClient::channel` — zero callers anywhere in the tree.

### Changed

- `pub mod unified` and `pub mod registry` narrowed to `pub(crate)`.
  The documented types (`Client`, `SubscriptionInfo`,
  `ConnectionStatus`, `EndpointMeta`, `ParamMeta`, `ParamType`,
  `ReturnType`, `ENDPOINTS`, plus `by_category`, `find`,
  `param_type_to_json_type`, `CATEGORIES` for the CLI / MCP tools)
  stay public via `pub use`.
- `DirectConfig::production_defaults` narrowed to `pub(crate)`; the
  only caller outside `config.rs` is in-crate (`observability.rs`).
- `tdbe` bumps to `0.12.0` (breaking: `pub mod errors`
  removed). The public `ThetaDataError` struct, `error_from_http_code`
  fn, and `HTTP_STATUS_CODE_KEY` const are still reachable at the
  new `tdbe::error::*` path.
- FFI surface consolidated: every SDK — C++, Go, Python,
  TypeScript — now calls the `thetadatadx_<endpoint>_with_options` entry
  points. The plain-name FFI entry points are no longer exported.

## [8.0.6] - 2026-04-23

Snapshot-endpoint latency fast-path on the Python binding and new opt-in
Rust `frames` module. Reduces residual latency on the 5 flagged snapshot /
calendar endpoints (`stock_snapshot_ohlc`, `stock_snapshot_quote`,
`stock_snapshot_market_value`, `calendar_on_date`, `calendar_open_today`),
and brings chainable `.to_polars()` / `.to_arrow()` DataFrame ergonomics
to Rust consumers behind opt-in Cargo features so polars and arrow stay
out of the default dep graph.

### Added

- **Rust `frames` module — `TicksPolarsExt` / `TicksArrowExt` extension traits behind `polars` / `arrow` / `frames` Cargo features.** Chain `.to_polars()` / `.to_arrow()` off a decoder-owned `&[tick::T]` in Rust the same way Python users chain off `<TickName>List`. Per-tick-type impls are generated from `tick_schema.toml`, covering every entry — `CalendarDay`, `EodTick`, `GreeksTick`, `InterestRateTick`, `IvTick`, `MarketValueTick`, `OhlcTick`, `OpenInterestTick`, `OptionContract`, `PriceTick`, `QuoteTick`, `TradeQuoteTick`, `TradeTick`. Column-shape is shared with the Python path: both generators read `tick_schema.toml` and apply the same field-type → Arrow-dtype mapping, so `ticks.as_slice().to_polars()?` in Rust produces the same DataFrame schema (column order, dtypes, the `QuoteTick.midpoint` virtual column, the contract-id `expiration` / `strike` / `right` tail, the `OptionContract.right` i32 → string projection) as `client.stock_history_eod(...).to_polars()` in Python. Dep footprint stays opt-in: `polars = ["dep:polars"]`, `arrow = ["dep:arrow-array", "dep:arrow-schema"]`, `frames = ["polars", "arrow"]`; polars and the arrow crates are pinned with minimal features and aligned to a single major version across the repo. Opt-in form: `thetadatadx = { version = "8", features = ["polars"] }`.

### Changed

- **Snapshot-kind endpoints now return plain `list[TickClass]` instead of the `<TickName>List` wrapper.** Applies to every endpoint with `subcategory = "snapshot"` or `"snapshot_greeks"` in `endpoint_surface.toml`, plus every `category = "calendar"` + `kind = "parsed"` entry — 20 endpoints total: 4 `stock_snapshot_*`, 11 `option_snapshot_*` (OHLC, trade, quote, open_interest, market_value, + 5 greeks variants + 1 IV variant), 3 `index_snapshot_*`, 3 `calendar_*`. The `<T>List` allocation cost was pure overhead on the latency-sensitive path — callers never chain `.to_polars()` on a 1-row calendar result. Classification is entirely TOML-driven via `helpers::is_snapshot_endpoint`; no hand-curated allowlist, so adding a new snapshot-kind endpoint to the TOML automatically opts it into the fast path on the next generator run. Return-type annotation changes (`list[CalendarDay]` instead of `CalendarDayList`); positional args and kwargs on the public pymethod signature are unchanged.
- **Python snapshot calls drop the periodic signal-check tax for a bounded 5-second deadline.** The previous signal-check poll loop taxed every sub-100 ms call with 1-5 ms of first-tick jitter in the worst case. Snapshot calls now wait on the result under a 5-second deadline without blocking other Python threads, so compute threads keep running during the wait. The 5-second upper bound is a liveness safeguard: every observed production snapshot call completes in under 200 ms, so the bound adds zero steady-state cost. Ctrl+C is still honoured once the result resolves or the deadline fires. Parsed / list / streaming endpoints keep the existing path unchanged.
- **Ctrl+C latency on short Python historical calls tightened from ~100 ms to ~20 ms.** The signal-check cadence on parsed-kind calls is 5x finer; signal checks are about 1 microsecond each, so the steady-state cost is negligible. Long-running endpoints see no behavioural change beyond a slightly finer-grained Ctrl+C cancellation window.
- **`README.md` / `sdks/python/README.md` — positioning refreshed.** Dropped the old small snapshot / calendar latency caveat now that the fast-path reduces overhead on every measured endpoint. Added a feature-gated Rust DataFrame quickstart example showing `thetadatadx = { version = "8", features = ["polars"] }` plus the chained `ticks.as_slice().to_polars()?` call site.
- **Generated snapshot fast-path converters for the Python binding.** One converter per snapshot-return tick type (9 in total, covering calendar, OHLC, quote, trade, market-value, open-interest, IV, Greeks, and price ticks); a converter for a tick type not reached by any snapshot endpoint is suppressed at generation time to avoid dead code. The set is derived from `endpoint_surface.toml`, so adding a snapshot endpoint of a new tick type automatically opts its converter in on the next generator run. The converted Python list contents are byte-identical to the existing `to_list()` path.

## [8.0.5] - 2026-04-22

Endpoint performance fixes discovered during a pre-release performance review.
Four regressions on the historical wire surface, all converging on one generator-level
asymmetry: the Rust request builder was sending a different wire shape than the
request contract on option endpoints, and on a subset of calls that
difference tipped the server into an enumeration slow-path. No behaviour changes
on the returned tick data, no signature changes on the SDK surface.

### Fixed

- **`option_list_dates` — duplicate expiration field removed from the request wire shape.** The v3 request carries both a `ContractSpec` (whose `expiration` is the contract identity) and a top-level `expiration` field that predates it. Populating both with the same date forced the server onto a slow per-contract enumeration path. The top-level field is now left empty when the request also carries a `ContractSpec`, across every option request that carries both fields, with no per-endpoint edits.
- **`option_at_time_quote` — duplicate expiration field removed from the at-time quote path.** The same top-level `expiration` duplicate that bottlenecked `option_list_dates` also penalized the at-time-quote path on dense option chains. Same generator-level fix applies: `expiration` on `OptionAtTimeQuoteRequestQuery` now emits `String::new()`.
- **`option_history_greeks_eod` — wire-shape parity restored on the wide-schema path.** Same fix as the two items above; greeks-EOD sent the duplicate `expiration` field through the same code path.
- **`ContractSpec.strike` / `ContractSpec.right` — wildcard sentinels now marshal as literal `"*"` / `"both"` on the wire.** The previous `wire_strike_opt` / `wire_right_opt` mapping reinterpreted the SDK-surface wildcards (`""`, `"*"`, `"0"` for strike; `"both"` for right) as proto-unset optional fields. Upstream request examples populate these fields literally; the v3 server treats an **unset** optional as "enumerate every strike / right for this contract" (slow path) and an explicit `"*"` / `"both"` as "chain-wide lookup" (fast path). Both helpers now always return `Some(...)` with the canonical wildcard literal. No signature changes on the SDK surface; callers continue to pass `"*"` / `"both"` unchanged.

### Changed

- **`README.md` / `sdks/python/README.md` — positioning corrected to measured v8.0.4 bench numbers.** Dropped legacy headline claims from v8.0.0-era measurements and replaced them with endpoint-specific, reproducible notes. Small snapshot / calendar calls are no longer described as speedups because network round-trip time dominates those calls.
- **8.0.2 slice-direct Arrow narrative scoped to builder terminals.** The 8.0.2 changelog bullet ("`.arrow()` / `.pandas()` / `.polars()` feed decoder-owned tick slices straight into Arrow column builders, peaking RSS at about the tick payload") described the builder-terminal path. The `<Type>List.to_polars()` non-builder terminal also reaches the slice-direct converter, but the column-builder pass holds both the decoder-owned slice and the column vectors in memory simultaneously. The narrative in both `CHANGELOG.md` and `docs-site/docs/changelog.md` now scopes the memory note to the implementation path that provides it.

## [8.0.4] - 2026-04-22

Pre-release review hotfixes on the Python binding. Four silent bugs on the
hand-written pyo3 glue — Gregorian date validation, Python logging-hierarchy
normalization, async thread-pool contention on heavy convert paths, and
interpreter-finalization safety on Python 3.13+. No behaviour changes on
the generated endpoint surface; every fix is confined to the hand-written
utility files the endpoint generator layers depend on.

### Fixed

- **The Python binding's date chunking accepted Gregorian-impossible dates.** The hand-rolled parser range-checked month `1..=12` and day `1..=31` independently, so `20230229` (Feb 29 in a non-leap year), `20240231` (Feb 31), `20240431` (Apr 31) and every other calendar-invalid combination slipped through and was silently normalized to a neighbouring valid day, producing wrong chunk boundaries when the 365-day auto-chunk helper split a range starting or ending on an impossible date. The validator now enforces leap-year and month-length rules from the canonical Gregorian calendar, via a dependency that was already present transitively (no new crate in the build graph). Covered by 12 new tests across the leap-year and month-length edge cases plus end-to-end rejection through the range splitter.
- **Python `logging` parent-level `setLevel` now filters Rust-side log events.** The Rust core emits log targets as `::`-separated module paths (`thetadatadx::auth::...`), but Python's `logging` hierarchy is `.`-separated, so `logging.getLogger("thetadatadx").setLevel(logging.DEBUG)` did NOT propagate to those events — Python treated them as unrelated top-level loggers with no parent-level filtering. The v8.0.2 release notes' claim that parent-level `setLevel` filters Rust-side events was therefore false. The logging bridge now normalizes target names to the dotted form before handing them to `logging.getLogger(...)`, so the full `getLogger -> setLevel -> isEnabledFor` hierarchy works. Covered by a new test on the canonical target names and a Python-level hierarchy-propagation test.
- **Concurrent Python `*_async` calls no longer serialize.** Building the Python result for an `_async` call (e.g. a 955 237-row quote-tick list) used to block the async path while it ran, so two concurrent `*_async` calls serialized end-to-end even when runtime workers were free. The result-building work now runs on the blocking pool, leaving the async runtime free to service other calls, with the awaitable's error surface unchanged. Covered by a new wall-clock test that fires two concurrent `_async` calls with 100 ms result-building work and asserts the combined elapsed time is under 1.5x single-task (the pre-fix serial behaviour was roughly 2x).
- **Python could crash during interpreter shutdown on Python 3.13+.** A background thread emitting a log event while the interpreter was tearing down could abort the process before the logging layer could swallow the error. The logging path now detects when the interpreter is unavailable (finalizing, not yet initialized, or mid-GC) and silently drops the event instead. Losing a log event at shutdown is an acceptable tradeoff against a crash on interpreter exit. Covered by a regression test on the live-interpreter path and a note in the threading-model documentation.

## [8.0.3] - 2026-04-22

Python-UX polish: DataFrame conversion is now a chain on the returned list
(`client.stock_history_eod(...).to_polars()`). The free-function and client-method
`to_polars(ticks)` / `to_arrow(ticks)` / `to_pandas(ticks)` / `to_dataframe(ticks)`
entry points are removed hard — there is now exactly one surface for converting
tick data into a DataFrame.

### Changed

- **Chained DataFrame conversion on every list-returning endpoint.** Every endpoint wraps its result in a typed `<ReturnType>List` pyclass (`EodTickList`, `TradeTickList`, `QuoteTickList`, …, plus `StringList`, `OptionContractList`, `CalendarDayList` for non-tick list returns). The wrapper exposes `.to_polars()`, `.to_arrow()`, `.to_pandas()`, `.to_list()` and the list protocol. Usage is `client.stock_history_eod(...).to_polars()` — no intermediate variable, no free-function round-trip. Builder terminals collapse from four parallel `.list()` / `.arrow()` / `.pandas()` / `.polars()` methods to a single `.list()` whose return carries the same chained terminals.

### Removed

- **Free-function and client-method conversion helpers removed.** `thetadatadx.to_polars(ticks)`, `thetadatadx.to_arrow(ticks)`, `thetadatadx.to_pandas(ticks)`, `thetadatadx.to_dataframe(ticks)` and the identically-named methods on the client handle are deleted. Consumers migrate by chaining the terminal off the endpoint return value (`client.stock_history_eod(...).to_polars()` in place of `thetadatadx.to_polars(client.stock_history_eod(...))`). One path, one SSOT, one place to audit.

### Changed

- **Generated `_async` methods share one awaitable helper** instead of each inlining its own async-bridge scaffolding, shedding roughly 599 lines of duplicated plumbing. Internal-only; the async surface is unchanged.
- **Docs-site restructure.** Deleted the standalone benchmark page, the migration-from-thetadata guide, the five per-language `quickstart/*.md` files, and the separate async-python narrative. Replaced with a unified code-group quickstart exposing Rust / Python / TypeScript / Go / C++ via language tabs so one page stays in sync across SDKs.

## [8.0.2] - 2026-04-21

Bigger than a typical patch: ships a P0 decode-correctness fix alongside
a feature-additive wave across the Rust SDK and the Python bindings.
Every surface added here is backward-compatible — no method signatures
change, no types are removed, no client code needs to migrate. The
patch-level version reflects that existing callers continue to compile
unchanged; the additive surface opens new opt-in paths.

### Fixed

- **P11 — `stock_history_trade_quote` / `option_history_trade_quote` silently returned `Ok(vec![])` on non-empty responses.** The v3 historical server emits the combined-row pair as `trade_timestamp` / `quote_timestamp`; `tick_schema.toml` declared them as `ms_of_day` / `quote_ms_of_day` with no aliases. `find_header` failed both required-header guards and the parser short-circuited before decoding any row. Added aliases `ms_of_day` ↔ `trade_timestamp`, `quote_ms_of_day` ↔ `quote_timestamp`, `date` ↔ `trade_timestamp`. Verified against a fresh prod capture: AAPL `stock_history_trade_quote` now returns 955 237 rows, SPY option returns 98. Captured-response regression fixtures ship for seven endpoints (`stock_history_trade_quote`, `option_history_trade_quote`, `stock_history_eod`, `option_history_greeks_all`, `option_history_trade`, `option_snapshot_ohlc`, `calendar_open_today`) so the same class of schema drift fails at PR time next release.
- **Decoder audit — `parse_<tick>_ticks` guard no longer drops rows on schema drift.** Generator template and the hand-written `parse_option_contracts_v3` now raise `DecodeError::MissingRequiredHeader` when the `DataTable` carries rows but declares none of the expected columns. Empty responses continue to return `Ok(vec![])` (a holiday with no trades remains a legitimate outcome). Walked every `Vec::new()` / `unwrap_or_default()` call-site in `decode.rs` and `fpss/decode.rs` — the remaining ones are intentional soft-fail accessors (bench / macro) or per-event nibble buffers, flagged as such in the audit report.

### Added

- **Async Python surface — every historical endpoint gains an `_async` companion.** `client.stock_history_eod_async(...)` returns an awaitable bridged to the async runtime. Sync and async paths share the same process-wide async-runtime singleton — one runtime, one connection pool, one request semaphore.
- **Fluent builders — `client.<endpoint>_builder(...)` returns a per-endpoint `#[pyclass]` with chainable setters and `.list()` / `.arrow()` / `.pandas()` / `.polars()` terminals plus `_async` companions.** Builder holds `Arc<thetadatadx::Client>` so every terminal drives the original client without re-authenticating.
- **`decode_response_bytes(endpoint, chunks)`** — generator-emitted `#[pyfunction]` that feeds recorded `Vec<&[u8]>` `proto::ResponseData` frames through the Rust decoder and returns the typed pyclass list, so external parity benches can attribute wall-clock cost between network and decode without a historical round-trip. Auto-wired for every endpoint that has a typed decoder.
- **Layered exception hierarchy** — `thetadatadx.ThetaDataError` root plus nine leaves: `AuthenticationError`, `InvalidCredentialsError`, `SubscriptionError`, `RateLimitError`, `SchemaMismatchError`, `NetworkError`, `TimeoutError`, `NoDataFoundError`, `StreamError`. `to_py_err` maps every `thetadatadx::Error` variant (plus gRPC status strings) onto the correct leaf. `#[non_exhaustive]` catch-all.
- **Python logging bridge** — `tracing_subscriber::Layer` that forwards every `tracing` event to `logging.getLogger(target).log(...)`. Filter-first via `isEnabledFor(level)` so default WARN loggers pay a single bool check per event with no formatting. Installed at module init.
- **Slice-based Arrow fast path on builder terminals** — `.arrow()` / `.pandas()` / `.polars()` (and their `_async` companions) feed the decoder-owned `Vec<tick::T>` straight into the Arrow column builders, skipping the pyclass-list double-buffer. The `<Type>List.to_polars()` terminals on the typed-list wrapper also reach this slice-direct path; the column-builder pass holds the decoder-owned slice and the column vectors simultaneously. Schema is bit-identical to the pyclass-list path so downstream consumers alias either source interchangeably. (Language narrowed from the initial memory-footprint claim in v8.0.5 — see that entry.)
- **`RetryPolicy`** — initial_delay 250 ms, max_delay 30 s, max_attempts 5, full jitter by default. Retries only on `Unavailable` / `DeadlineExceeded` / `ResourceExhausted`. Unit-tested backoff math, jitter bounds, and the `disabled()` shortcut.
- **Session auto-refresh** — `auth::SessionToken` holds the session UUID behind an async mutex + monotonic version counter. On `Unauthenticated` the retry loop snapshots the token, re-auths via Nexus, swaps the UUID in place, and retries exactly once. A second 401 fails permanently. Concurrent 401s dedupe into a single Nexus round-trip via version-check short-circuit.
- **Environment-variable config matrix** — `DirectConfig::production()` layers env vars on the hardcoded defaults: `THETADATA_HISTORICAL_HOST`, `THETADATA_HISTORICAL_PORT` (upstream-compat), plus DX extensions `THETADATA_NEXUS_URL`, `THETADATA_STREAMING_HOST`, `THETADATA_STREAMING_PORT`, `THETADATA_CLIENT_TYPE`. Precedence: explicit builder setter > env var > hardcoded default.
- **Optional `metrics-prometheus` cargo feature** — pulls `metrics-exporter-prometheus` and wires an HTTP `/metrics` listener on `DirectConfig::metrics_port`. Exporter starts inside `ThetaDataDx::connect` so the first RPC counter is already covered. Feature-gated; default build stays dep-free.
- **Vendor docstring lift** — 60 endpoint docstrings threaded through `endpoint_surface.toml` → model → parser → generator so sync / async / builder variants share one SSOT.
- **`split_date_range(start, end)`** — pure Rust 365-day-window splitter exposed as `thetadatadx.split_date_range` for tooling and the auto-chunk pre-flight. Tested on single-day, exact boundary, multi-year contiguity, leap-day, and invalid input.
- **Capture fixtures** — seven `tests/fixtures/captures/<endpoint>.{pb.zst,meta.toml}` pairs anchor expected row counts, exact server header lists, and first-row field values. `tests/test_decode_captures.rs` feeds each fixture through the same `decode_data_table` → tick-parser path the `HistoricalClient` uses and asserts three invariants per fixture. Two regression guards ensure `MissingRequiredHeader` fires on non-empty schema drift and empty responses still return `Ok(vec![])`.

### Changed

- **Regenerated SDK surfaces** — `historical_methods.rs`, `tick_arrow.rs`, `decode_bench.rs` rebuilt off the merged generator. Byte-identical check passes.
- **Parser generator raises `MissingRequiredHeader` on schema drift** — the generated `parse_<tick>_ticks` template no longer silently returns `Ok(vec![])` when a required column is absent on a non-empty `DataTable`. Empty responses continue to pass through unchanged.

## [8.0.1] - 2026-04-21

### Fixed

- **`tdbe` bumped to 0.11.0 to publish the new `SecType::Unknown` variant to crates.io** — the 8.0.0 release added `SecType::Unknown` (empty-contract sentinel) but kept `tdbe` at `0.10.0`. `cargo publish --verify` for `thetadatadx 8.0.0` pulled `tdbe = 0.10.0` from the registry, which does not contain `Unknown`, and failed with `E0599`. The `thetadatadx`, `ffi`, `cli`, `mcp`, `server`, `py`, and `napi` crates bump to `8.0.1` so all three ecosystems (crates.io, PyPI, npm) end up on matching, publishable versions. npm and PyPI had already published 8.0.0 successfully; crates.io 8.0.0 was never materialized.
- **Streaming handshake surfaces every typed control frame** — `wait_for_login` collects `Connected` (code 4), `Ping` (code 10), `ReconnectedServer` (code 13), and `Restart` (code 31) frames that arrive before `METADATA` into an ordered buffer; the I/O loop drains the buffer onto the event bus before emitting `LoginSuccess` so user callbacks see the exact wire order. Previously all typed control frames except `Connected` were silently dropped by the handshake's trace-and-continue branch. Applies to the initial login AND the reconnect-path login.
- **Reconnect-path login short-circuits on permanent rejection** — `LoginResult::Disconnected(reason)` during the reconnect handshake now consults `reconnect_delay(reason)` as the single source of truth for "no retry will fix this" and exits the I/O loop with `shutdown = true` + a `StreamControl::Disconnected` event. Previously bad credentials burned `MAX_RECONNECT_ATTEMPTS` (5) cycles of `Reconnecting` / `Disconnected` noise before giving up.
- **Mid-frame reader yields to the command drain on a bounded budget** — `FrameReadState` threads partial-frame progress across `read_frame_into` calls. A new `MID_FRAME_DRAIN_WINDOW_MS = 200` (4× the 50 ms drain cadence) caps the total wall time spent retrying a partial frame before the reader yields control to the I/O loop, which drains outbound commands and re-enters the reader with the preserved state. Previously a trickling sender could block heartbeats / user writes for up to `READ_TIMEOUT_MS` (10 s) because the per-stall deadline reset on every successful byte.
- **`Contract::from_str` accepts 1..=16-char roots** — `validate_root` widens from `1..=6` to `1..=16` chars, matching the wire-codec upper bound in `Contract::to_bytes()` / `Contract::from_bytes()`. `from_str` / `to_bytes` / `from_bytes` now round-trip symmetrically; the wire is the ground truth. Round-trip coverage for every length 7..=16 added.
- **Auth email redacted across `Debug` and tracing** — `AuthResponse::Debug`, `AuthUser::Debug`, and the `authenticate()` tracing line that previously rendered `email = %creds.email` now emit `<redacted>` / a prefix-only `ali...@example.com` form. Full emails no longer land in panic output, structured logs, or crash dumps.
- **Credentials parsing pipeline wraps every transient in `Zeroizing`** — `from_file` reads the file into `Zeroizing<String>` so the on-disk password bytes are wiped on drop; `parse()` / `new()` wrap the intermediate owned password `String` in `Zeroizing` before assigning to the struct. A panic or early-return between allocation and struct construction still wipes the plaintext on unwind. Completes the coverage the 8.0 release notes claimed; the previous implementation zeroed only the final `Credentials.password` field.
- **Empty-contract sentinel documentation unified** — `StreamData::{Quote,Trade,OpenInterest,Ohlcvc}` docstrings now promote `contract.sec_type == SecType::Unknown` as the canonical check for the empty-contract placeholder (matching `fpss::decode`'s guidance). `root.is_empty()` is retained as a secondary mention but no longer the primary documented check -- it was brittle against future root-charset relaxations.

## [8.0.0] - 2026-04-21

Major release. Three headline groups land in one pass:

1. **Streaming events now carry a parsed `Arc<Contract>`** (#389). Every `StreamData::{Quote,Trade,OpenInterest,Ohlcvc}` replaces the bare `symbol` string field with `contract: Arc<Contract>`, and the `contract_map` lifts from `HashMap<i32, Contract>` to `HashMap<i32, Arc<Contract>>`. Decoded events carry the full typed contract (`root`, `sec_type`, `exp_date?`, `is_call?`, `strike?`) at refcount cost rather than a bare symbol string; every language SDK exposes a matching typed `Contract`. `SecType::Unknown` is added as the sentinel for not-yet-assigned contract IDs so exhaustive matches stay sound.
2. **`impl FromStr for Contract` plus historical streaming subscribe shortcuts** (#389). `"AAPL".parse::<Contract>()?` yields a stock contract; `"SPY   260417C00550000".parse::<Contract>()?` parses the OCC 21-char option identifier (2000–2099 scope, trim-tolerant 20-char pad, every parse failure returns `Error::Config` with the offending input). `StreamingClient` and `Client` gained per-security subscribe and unsubscribe shortcuts — one-liners over the underlying typed subscribe machinery.
3. **Streaming control codes 4 / 10 / 13 / 31 decode into typed variants** (#389). `StreamControl::{Connected, Ping { payload }, ReconnectedServer, Restart}` replace the `UnknownFrame` fallthrough these codes used to hit. The `Restart` arm clears delta decode state so subsequent ticks no longer decode against a stale baseline. FFI kind tags grow 13..=16; every SDK mirrors the new constants.

### Removed

- **`StreamData::{Quote,Trade,OpenInterest,Ohlcvc}::symbol` removed** (#389) — migrate to `event.contract.root` for the symbol string; option fields `exp_date`, `strike`, `is_call` are now direct attribute access on `contract`.
- **`StreamControl::ContractAssigned { contract: Contract }` → `{ contract: Arc<Contract> }`** (#389) — pattern matches that bind by value must bind by `Arc<Contract>` and clone via `Arc::clone` if owned value was previously expected.
- **`contract_lookup()` / `contract_map()` return `Arc<Contract>` / `HashMap<i32, Arc<Contract>>`** (#389) — was by-value `Contract` / `HashMap<i32, Contract>` before. Call-site fix: drop one layer of `.clone()`.
- **`Restart` (code 31) and `Connected` (code 4) frames no longer arrive as `UnknownFrame`** (#389) — handlers matching on `StreamControl::UnknownFrame { code: 4 | 10 | 13 | 31, .. }` need updated arms or a fallthrough on the new typed variants.
- **`SecType::Unknown` variant added to `tdbe::types::enums::SecType`** (#389) — exhaustive `match` statements without a wildcard arm must add a branch.
- **`StreamData::{Quote,Trade,OpenInterest,Ohlcvc}` no longer `derive(Clone)` on the Python SDK pyclasses** (#389) — `Py<Contract>` cannot be cloned without an active Python context; the derive was dead code (events flow one-way from Rust to Python).

### Changed

- **License switched to Apache-2.0** across every `Cargo.toml`, `package.json`, `pyproject.toml`, and the top-level `LICENSE`. `deny.toml` allowlist cleaned up accordingly.
- **Top-level `README.md` rewritten** as a professional SDK landing page: tagline, highlights, per-SDK quickstart (Rust / Python / TypeScript / Go / C++), architecture diagram, JVM terminal parity note. Neutral technical framing throughout.
- **A consolidated parity-tracking checklist added** as the single source of truth for terminal parity — feature-by-feature table (parity / deviation / partial) covering wire protocol, authentication, control events, reconnection, streaming, tick decoding, Greeks, validation, and intentional improvements over the JVM terminal. Three earlier stand-alone parity notes folded in.
- **Internal `docs/dev/` design notes removed** (no longer load-bearing).
- **`DirectClient` renamed to `HistoricalClient`** (#383) — the historical-data gRPC client now carries the name of the service it actually speaks to (MDDS = Market Data Delivery Service). `use thetadatadx::DirectClient` call sites break; update to `use thetadatadx::HistoricalClient`. The `DirectConfig` associated config type keeps its name. High-level consumers of `Client` (Python / TypeScript / Go / C++ / Rust facade) are unaffected.
- **The `direct.rs` module split into the `mdds/` module** (#383) — 732-line monolith broken into six concern-separated files (`client`, `endpoints`, `endpoint_arg_ext`, `normalize`, `validate`, `mod`). Pure move; wire behavior unchanged; all 304 workspace tests pass.
- **The historical-service proto file renamed to reflect MDDS** (#385) — the file described only MDDS messages, so the filename now matches. The generated protobuf include and every downstream Rust import resolve unchanged, because the package declaration (not the filename) drove the module name. The build, the proto tooling, the generated-header strings, and every doc reference were updated in the same sweep.
- **`fpss_event_schema.toml` schema version bumped 2 → 3** (#389) — carries the new nested `Contract` column type for every data-event variant. Every SDK Contract type (Python pyclass, TypeScript `#[napi(object)]`, Go struct with `*int32`/`*bool` pointer optionals, C/C++ typedef with `has_*` tagged-optional flags, Rust FFI C-layout `ThetaDataDxContract` with `CString`-backed root pointer) is generator-emitted from the updated schema.

### Added

- **Parsed `Arc<Contract>` on every streaming data event** (#389) — `StreamData::{Quote,Trade,OpenInterest,Ohlcvc}::contract: Arc<Contract>` replaces the former bare `symbol` string. Option events now expose `event.contract.exp_date`, `.strike`, `.is_call` without a second lookup; stock events read `event.contract.root`. Refcount-only per-event clone. Carries the resolved contract identity on each event without a second lookup or JSON round-trip. `contract_lookup` and `contract_map` return `Arc<Contract>` / `HashMap<i32, Arc<Contract>>` on every SDK.
- **`impl FromStr for Contract`** (#389) — `"AAPL".parse::<Contract>()?` yields a stock contract (1..=6 ASCII A-Z, `.` permitted); `"SPY   260417C00550000".parse::<Contract>()?` parses the OCC 21-char institutional option identifier (6-byte root right-padded with spaces, 6-byte YYMMDD century-adjusted to 2000–2099 YYYYMMDD, single-byte `C`/`P`, 8-byte strike in thousandths of a dollar). 20-byte inputs are tolerated with a trailing-space pad. Parse failures return `Error::Config` naming the offending input and the specific failure (length, root charset, expiration digits, right byte, strike digits).
- **Historical streaming subscribe shortcuts** (#389) — per-security subscribe and matching unsubscribe counterparts were added on `StreamingClient` and `Client`. Each wraps the `Contract` builder plus the typed `subscribe` / `unsubscribe` call into one line; no duplicate request-ID or frame-build machinery.
- **Typed decoding of streaming control codes 4 / 10 / 13 / 31** (#389) — `StreamControl::Connected` (4), `StreamControl::Ping { payload }` (10), `StreamControl::ReconnectedServer` (13 — server-side ack, distinct from the client-side auto-reconnect `Reconnected` variant), and `StreamControl::Restart` (31) replace the `UnknownFrame` fallthrough these codes used to hit. The `Restart` arm clears delta decode state so subsequent ticks no longer decode against a stale baseline. FFI `ThetaDataDxStreamControl` kind tags grow 13..=16; Go `FpssCtrl*` constants mirror them.
- **`Contract` type surfaced on every language SDK** (#389) — Python pyclass (`Py<Contract>` embedded in each event, cloned via `clone_ref(py)`), TypeScript `#[napi(object)]`, Go struct with `*int32` / `*bool` pointer optional fields, C/C++ typedef with `has_*` tagged-optional flags, Rust FFI C-layout `ThetaDataDxContract` with a `CString`-backed `root` pointer. `Contract.sec_type == SecType::Unknown` is the sentinel for not-yet-assigned contract IDs; every SDK exposes the new variant.
- **`thetadatadx.to_arrow(ticks) -> pyarrow.Table`** (#379) — new public Python entry point that returns the Arrow table directly, for users wiring DuckDB / Arrow-Flight / cuDF / polars-arrow pipelines without a pandas or polars roundtrip. Requires `pip install thetadatadx[arrow]` (pyarrow only).
- **`hint=` kwarg on `to_arrow` / `to_dataframe` / `to_polars`** (#380) — optional `hint: str` names the tick pyclass (e.g. `hint="EodTick"`) so the Arrow schema is materialised even when the input list is empty. Previous empty-list calls returned a zero-column table; downstream pipelines asserting a fixed schema now get the right columns on empty market-hours windows.
- **Generated `#[new]` constructors on every tick pyclass** (#379) — `EodTick(ms_of_day=1, volume=1_000_000, ...)`, `OhlcTick(...)`, `TradeTick(...)`, etc. All fields are keyword-only with zero / empty-string defaults, so test fixtures and user-side data construction are possible from Python (previously pyclass instances could only be produced by Rust endpoints).
- **`AllGreeks` pyclass** (#378) — `all_greeks(...)` now returns a frozen `AllGreeks` pyclass with 22 `#[pyo3(get)]` f64 fields (value / iv / delta / gamma / theta / vega / rho plus every second- and third-order Greek) and a `__repr__` showing the six most-referenced values. Replaces the untyped 22-key `PyDict` that was the sole remaining dict-typed public return in the Python SDK.
- **`__repr__` on every streaming event pyclass** (#380) — `Ohlcvc`, `Quote`, `Trade`, `OpenInterest`, `Simple`, `RawData` now render up to six live field values at the Jupyter / print boundary (matching the pattern already on tick pyclasses). Opaque `Vec<u8>` payloads and `received_at_ns` skipped as noise.
- **`dropped_events()` counter on every streaming SDK** (#377) — a dropped-event counter that survives reconnect, exposed as `client.dropped_events() -> int` (Python), `client.droppedEvents(): bigint` (TypeScript), `client.DroppedEvents() uint64` (Go), `client.dropped_events() -> uint64_t` (C++), and `thetadatadx_streaming_dropped_events(handle)` / `thetadatadx_client_dropped_events(handle)` (FFI). Call sites that previously dropped a buffered event silently now bump the counter and log a debug event.
- **`POST /v3/system/shutdown` endpoint on `thetadatadx-server`** (#377) — graceful shutdown over a privileged route gated by a per-startup random UUID `X-Shutdown-Token` header (constant-time compared via `subtle::ConstantTimeEq`). Prints the token to stderr at startup only; never into structured logs. Dedicated governor allows one attempt per hour, burst 3. Method is `POST` (not `GET`) so the action is neither cached nor prefetched.

### Changed

- **DataFrame adapter migrated to Apache Arrow columnar pipeline** (#379) — `to_dataframe(ticks)` / `to_polars(ticks)` / `to_arrow(ticks)` build a single `arrow::RecordBatch` in Rust and hand it to pyarrow via the Arrow C Data Interface (zero-copy at the pyo3 boundary). pandas 2.x aliases the numeric columns in place; polars consumes via `polars.from_arrow`. At 100k x 20 `EodTick` rows wall-clock drops from ~300-500 ms (legacy dict-of-lists) to ~8 ms — substantially. SSOT preserved: Arrow schema + converters are generated from `tick_schema.toml`; no hand-maintained Arrow code.
- **Per-endpoint DataFrame convenience wrappers removed** (#379) — the four per-endpoint `stock_history_{eod,ohlc,trade,quote}` Rust-tick-slice fast-path helpers on `Client` were deleted. The unified recipe is one extra line with identical performance:

  ```python
  ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
  df    = thetadatadx.to_dataframe(ticks)   # Arrow-backed, zero-copy on pandas 2.x
  pdf   = thetadatadx.to_polars(ticks)      # Arrow-backed, zero-copy
  table = thetadatadx.to_arrow(ticks)       # DuckDB / cuDF / Arrow-Flight
  ```

  Single code path, single generator, single test surface — 100% SSOT restored on the Python DataFrame surface.
- **Deleted** the old PyDict-based columnar emission (#379) — replaced end-to-end by the generated Arrow path. `pip install thetadatadx[pandas]` / `[polars]` now pull `pyarrow>=14.0` alongside the DataFrame library; `pip install thetadatadx[arrow]` is the pyarrow-only extras bundle.

### Changed

- **Historical endpoints now return `list[TickClass]` instead of a columnar `dict[str, list]`** (#364 / #365). The 53 tick-returning historical methods (list endpoints returning scalar `Vec<String>` — symbols, dates, expirations, strikes — are unchanged) in the Python SDK (`stock_history_eod`, `option_history_trade`, `calendar_*`, ...) now return a Python list of typed pyclass objects — `EodTick`, `TradeTick`, `QuoteTick`, `OhlcTick`, `TradeQuoteTick`, `OpenInterestTick`, `MarketValueTick`, `GreeksTick`, `IvTick`, `PriceTick`, `CalendarDay`, `InterestRateTick`, `OptionContract`. Brings the Python SDK into line with Rust core, TypeScript, Go, and C++ FFI. Migration:

  ```python
  # before
  ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
  close = ticks["close"][i]            # string key, silent typo failures

  # after
  ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
  close = ticks[i].close               # attribute access, typed
  ```

  `to_dataframe(ticks)`, `to_polars(ticks)`, and `to_arrow(ticks)` transparently pivot the new shape into a pandas / polars frame or a `pyarrow.Table`.

### Changed

- **C++ `ThetaDataDxStreamEvent` field order realigned with Rust + Go** (#376) — the hand-written `ThetaDataDxStreamEvent` in `sdks/cpp/include/thetadx.h` declared `{ kind, quote, trade, open_interest, ohlcvc, control, raw_data }` while the Rust generator (and the Go C header) emits `{ kind, ohlcvc, open_interest, quote, trade, control, raw_data }`. Every `event->quote.*` / `event->trade.*` / `event->ohlcvc.*` access in existing C++ consumers was reading from the wrong offset — data corruption with no compile-time signal. `thetadx.h` now `#include`s the generator-emitted `fpss_event_structs.h.inc` (byte-identical to the Go C header) and `thetadx.hpp` gains `static_assert(offsetof / sizeof)` covering every field of every `ThetaDataDxStream*` struct. Any future drift is compile-fatal.
- **Go `FpssControlData` renamed to `StreamControl`, `FpssOpenInterest*` → `FpssOpenInterest`** (#376) — Go-idiomatic naming on the mirror struct set. Callers referencing the old names will fail to compile; rename one-for-one. The nested field names on `StreamEvent` (`ev.RawData.Code`, `ev.RawData.Payload`) are unchanged.

### Changed

- **`thetadatadx::direct` module renamed to `thetadatadx::mdds`** and split into concern-separated submodules (connect, response helpers, validators, wire-format canonicalizers, generated endpoints). "MDDS" is the actual upstream service name; "direct" conveyed nothing.
- **`DirectClient` renamed to `HistoricalClient`** — the struct inside the (now) `mdds/` module takes its module's name. Re-exported at the crate root as `thetadatadx::HistoricalClient`. `Client` still `Deref<Target = HistoricalClient>`s, so every historical endpoint method is reached unchanged via the unified client.

### Changed

- **`thetadatadx-server`: governor layer is now outermost, rate-limited traffic short-circuits first** (#377) — axum `.layer(X).layer(Y)` makes Y the outer wrapper, so the previous `ConcurrencyLimit → BodyLimit → Governor` order had the per-IP limiter innermost. Every rate-limited request still consumed a concurrency permit and ran the body-length check before being rejected. Reordered so the governor runs first; body-limit and concurrency gates are only touched by allowed traffic.
- **`thetadatadx-server`: `PeerIpKeyExtractor` on the REST + WS routers** (#377 / #378) — the per-IP rate limiter now keys on the real TCP socket source instead of the forwarded-header-trusting extractor used before. The server defaults to `127.0.0.1` without a trusted reverse proxy in front, so trusting `X-Forwarded-For` / `X-Real-IP` / `Forwarded` let a local attacker cycle fake IPs and bypass the per-IP rate limit. Module doc comment spells out the deployment policy.
- **`thetadatadx-server`: `BoundedQuery<N>` extractor caps query-string params during parse** (#378) — the previous check ran after axum's `Query<HashMap<String, String>>` had already parsed the entire query string into a HashMap, so a `?a=1&b=2&...` flood still allocated MB+ before hitting the count check. `BoundedQuery<32>` counts `&`-delimited pairs on the raw URI before `serde_urlencoded::from_str`, rejects over-limit with 400, and caps HashMap capacity.
- **`thetadatadx-server`: WS subscribe + every REST validator now run `ensure_no_control_chars` + per-field length caps** (#377) — symbol / root ≤ 16, expiration == 8 (YYYYMMDD), strike ≤ 10, right == 1, date == 8, venue ≤ 8. Returns 400 with a descriptive error, never 500. Unknown query-param names surface the real name in the error instead of an opaque `"parameter"` fallback.
- **`thetadatadx-server`: REST global concurrency limit 256, per-IP governor 20 rps / burst 40, body limit 64 KiB, WS text-frame cap 4 KiB** (#377 / #378) — explicit layers on both routers. Legitimate subscribe commands are <200 B; 4 KiB is generous for pathological clients.
- **`thetadatadx-server`: shutdown rate limit fixed — one token per hour, burst 3** (#377 follow-up) — `per_second(3600)` treats the argument as "requests per second", so the "3 attempts per hour" config was actually allowing ~3600 rps. Switched to `.period(Duration::from_secs(3600))`; constant renamed to `SHUTDOWN_REPLENISH_PERIOD`.
- **`thetadatadx-server`: hot-path `String::clone` eliminated on the streaming TOCTOU contract map** (#378) — the broadcast path now holds `HashMap<i32, Arc<Contract>>` instead of `HashMap<i32, Contract>`; the broadcast channel carries `(StreamEvent, Option<Arc<Contract>>)`. The hot-path clone is a refcount bump instead of a `String` allocation. Micro-bench (100k lookups): 26 ns/op → 22 ns/op, zero hot-path heap allocations. A regression test asserts the clone is a refcount bump rather than a string allocation to prevent future regressions.
- **TypeScript `const enum` of streaming event kinds removed** (#376) — the generated enum broke downstream consumers with `"isolatedModules": true` in `tsconfig.json` (all modern Vite / esbuild / ts-jest / Next.js setups). `StreamEvent.kind` is now `pub kind: &'static str` with a `#[napi(ts_type = "'ohlcvc' | 'open_interest' | 'quote' | 'trade' | 'simple' | 'raw_data'")]` override. Zero-allocation preserved; discriminated-union narrowing unchanged.
- **`go.mod` toolchain bumped to 1.23** (#378) — Go 1.21 released mid-2023; CI matrix already runs 1.23. Node.js `engines.node` bumped from `">= 18"` to `">= 20"` (Node 18 EOL 2025-04-30).
- **`paste` crate replaced by `pastey`** (#377) — upstream `paste` was archived on 2024-10-07 (RUSTSEC-2024-0436). `pastey = "0.2.1"` is the actively-maintained successor; API compatible (`::paste::paste!` → `::pastey::paste!`). Single call-site in the `macros` module.

### Fixed

- **FFI boundary catches Rust panics** (#380) — no panic isolation existed across the FFI layer before this change. A Rust panic crossing the C ABI on Rust 1.81+ aborts the host process, so C / Go / Python / C++ callers died with no way to recover. Every `extern "C"` function now contains panics at the boundary: the panic message is recorded and retrievable via the C error API (`thetadatadx_last_error()`), and the function returns its declared default (`ptr::null_mut()` / `-1` / `0` / sentinel-empty-array). **Coverage: all 145 production `extern "C"` functions** (the hand-written and the generated entry points alike), with the wrapper generator-emitted so future regeneration preserves parity. Backed by regression tests.
- **Python `next_event(timeout_ms)` honours Ctrl+C within 100 ms** (#380) — previously the call waited for the full user-supplied timeout (up to 5 minutes), so Ctrl+C was swallowed for the duration of the wait. The wait now checks for an interrupt every 100 ms and returns on the deadline.
- **`ThetaDataDx::new` constructor is cancellable** (#380) — swapped the uninterruptible blocking-wait path for `run_blocking(py, async { connect(...).await })` so a TLS / auth handshake hang stays Ctrl+C-interruptible.
- **Streaming TLS: SPKI pinning replaces `NoVerifier`** (#377) — `PinnedVerifier` parses the leaf cert via `x509-parser`, computes SHA-256 over the SubjectPublicKeyInfo DER bytes, and constant-time compares (`subtle::ConstantTimeEq`) against the captured `FPSS_SPKI_SHA256` (verified identical across prod `nj-a:20000` / `nj-b:20000`, dev `:20200`, stage `:20100` — single keypair across every streaming environment). Rejects with `CertificateError::NotValidForName` on hostname mismatch (allowlist) or `RustlsError::General("FPSS SPKI pin mismatch: ...")` on pin mismatch. `verify_tls12_signature` / `verify_tls13_signature` delegate to rustls' proper signature verification. Previously any on-path attacker terminating TLS to `nj-a.thetadata.us:20000` could present any cert and harvest the plaintext `StreamMsgType::Credentials` frame.
- **Password zeroized on drop** (#377) — `Credentials.password` and every internal copy of it wipe their backing buffer when dropped, so a core dump or `/proc/<pid>/mem` read no longer recovers the password once `Credentials` is gone. Call sites are unchanged.
- **CSV formula injection defused on `thetadatadx-server` exports** (#377) — `escape_csv_field` now prefixes cells whose first byte is `=`, `+`, `-`, `@`, or `\t` with a single-quote `'` and encloses in CSV quotes. Defuses `=cmd|'/C calc'!A1`, `@SUM(A1:A10)`, `+1+cmd|...` etc from executing in Excel downloads. Regression test covers all five payload shapes.
- **Streaming mid-frame read retry with per-read deadline reset** (#370) — previously a mid-frame read timeout desynced the decoder. The client now retries transparently with the per-read deadline reset, matching the JVM terminal's behaviour.
- **WS subscribe strike / expiration use `i32::try_from`** (#377) — client-supplied expiration / strike no longer silently narrow via `as i32`. Returns `REQ_RESPONSE { response: "ERROR", ... }` with a descriptive message on overflow. Validates `exp` against `[19000101, 21000101]` YYYYMMDD bounds and `strike > 0` before building the streaming frame (#378).
- **`validate_generic_named` sanitises parameter names in error messages** (#377 / follow-up) — ANSI escape sequences / control chars in a user-supplied param name can no longer escape into terminal-rendered log output. Names are passed through `sanitize_param_name` (ASCII alphanumeric + `_` + `-`).
- **Shutdown token constant-time compare** (#377) — the server's shutdown-token check now uses a constant-time comparison instead of `==`, closing a timing oracle on the token prefix.
- **Reconnect-path write errors are surfaced, not masked** (#377) — the streaming reader loop silently dropped write failures while draining commands on reconnect. They are now logged as warnings with the error and the frame code.
- **FFI reconnect paths surface resubscribe errors** (#378) — the unified and streaming reconnect paths previously dropped resubscribe errors silently; they are now logged as warnings with the error, kind, and contract context.
- **Python `Credentials.__repr__` redacts the email** (#377 / #378) — was `Credentials(email="user@example.com")`; email leaked into Jupyter, pytest output, and crash logs. Now `Credentials(email=<redacted>)`. Matches the redacted `Debug` impl on the Rust `Credentials` type.
- **CSV headers union across rows** (#376) — the server's CSV renderer seeded column keys from the first row only; mixed-type queries (index rows without `expiration` / `strike` / `right` ahead of option rows with them) silently dropped those columns. Headers now union across every row, sorted.
- **Streaming `Simple` control events carry `event_type` + nullable `detail` / `id`** (#378) — OpenAPI `Control` variant was documenting the internal numeric `kind: int32`, which no SDK surfaces. Aligned to the client-facing shape (`kind: "simple"` + `event_type` enum + nullable `detail` / `id` + `received_at_ns`).
- **Python `greeks.py` example + README quick-start use attribute access on `AllGreeks`** (#380) — `g['iv']` / `g['delta']` dict subscripts would have crashed at runtime because `AllGreeks` is a frozen pyclass without `__getitem__`. Rewritten to `g.iv`, `g.delta`, etc.
- **Typed `list[TickClass]` examples across every endpoint page** (#378) — ~50 files under `docs-site/docs/historical/` had stale dict-key Python examples (subscript access on the old columnar shape). Switched to attribute access on the typed pyclass surface. `scripts/fpss_smoke.py` / `scripts/fpss_soak.py` likewise switched from dict subscript on streaming events to attribute access (both scripts are wired into live CI).

### Security

- **Streaming TLS authenticity anchored on captured SPKI pin, no longer trust-on-first-use** (#377) — see `Fixed` above. Cert rotation tolerated as long as the keypair stays; expiry sidestepped entirely (current ThetaData leaf expired 2024-01-12). Six new tests cover captured-leaf positive, hostname mismatch rejection, malformed-cert rejection, and openssl fingerprint reproducibility.
- **Cargo-deny advisory / licence / drift gates in CI** (#377) — new `.github/workflows/security-audit.yml` runs RustSec `audit-check` on PR + push + weekly Monday 03:00 UTC cron + manual dispatch. New `cargo-deny` job reads policy from `deny.toml` (advisories deny, licences allowlist, bans duplicates warn, sources crates.io only). New `drift-injection` job runs `scripts/test_drift_injection.sh` which flips `bid` ↔ `ask` in the streaming schema, regenerates, and verifies the C++ `static_assert(offsetof)` guards fail the cmake build.

### Changed

- **Generator audit cleanup** (#380) — `PYTHON_TICK_ARROW_DIRECT_TYPES` constant + `render_python_tick_arrow_batch_fn` (~70-line emitter) were orphaned by the `*_df` removal in #379 and survived only because of the module-level `#![allow(dead_code)]` umbrella. Deleted. The trait-driven `pyclass_list_to_arrow_table` path is the sole public DataFrame entry point, backed by `<T as ArrowFromPyclassList>::read_batch`. `render_python_tick_arrow` doc rewritten to describe the two still-emitted surfaces (`arrow_schema_for_qualname` + `pyclass_list_to_arrow_table`). `clippy::type_complexity` on a 4-tuple in `sdk_surface.rs` cleared via a `MethodShape<'a>` alias.
- **Go layout regression: `TestTickFieldOffsets` covers every tick mirror field** (#376) — the previous `ffi_layout_test.go` only asserted total struct `sizeof`; same-size field reorders (e.g. swapping two i32 slots) passed the test while silently corrupting data. Streaming mirror types were not tested at all. cgo-typed streaming offset asserts moved into `tick_ffi_mirrors.go::init()` (Go forbids cgo in `_test.go`).
- **Full stale-data sweep + i64 widening across every doc surface** (#375 / #378) — `OhlcTick` / `EodTick` volume + count widened from `i32` to `i64` (#372 on the Rust side). Docs updated across `docs/api-reference.md`, `docs-site/docs/api-reference.md`, `docs-site/public/thetadatadx.yaml`, and every per-endpoint page. Stale `14 tick types` references corrected to 13. `[Unreleased]` compare link fixed from `v7.2.0...HEAD` to `v7.3.1...HEAD`; missing `v7.2.1` / `v7.3.0` / `v7.3.1` tag compares added.
- **Toml crate metadata warning silenced** (#377) — `toml = "1.1.2+spec-1.1.0"` → `toml = "1.1.2"` in both `[dependencies]` and `[build-dependencies]`. Every `cargo build` invocation no longer warns about ignored semver metadata.
- **Workspace manifest consolidated via `[workspace.package]` + `[workspace.lints]`** (#384) — duplicate `edition`/`license`/`authors`/`repository`/`homepage`/`rust-version` removed from every member `Cargo.toml` and hoisted to the workspace root; each member inherits via `x.workspace = true`. A new `[workspace.lints.rust]` table denies the rustc `warnings` group (matching CI's `-D warnings`) and promotes `unsafe_op_in_unsafe_fn` to deny alongside; `[workspace.lints.clippy]` denies `clippy::all`. Every member crate opts in via `[lints] workspace = true`. Versions intentionally stay per-crate because `tdbe` ships on a `0.x` track independent of the `7.x` SDK line.
- **Server WebSocket module split into concern-separated submodules** (#384) — an internal-only reorganisation, with visibility tightened where external visibility wasn't needed. Pure move; every server unit and integration test passes.
- **FFI layer reorganised into topic modules** (#384) -- an internal-only refactor. **ABI is byte-for-byte identical**: the same 211 `thetadatadx_*` symbols are exported on both the shared and static libraries before and after the change, so downstream C / C++ / Go / Node consumers see zero difference.
- **Three largest code generators split by render target** (#384) — each broken into concern-separated sub-modules. A regen harness hashes every generated artifact before and after a clean rebuild and fails on any drift. **Verified: 450 files, zero diff.**
- **Multi-line generator templates externalised into `.tmpl` files** (#386) — the Rust generators no longer carry embedded Python / TypeScript / Go / C++ source as raw string literals; templates are loaded from files and rendered through the existing machinery, with no new runtime dependency. Line-count reductions of roughly a third on the largest generator files. Every template is pinned to LF line endings so Windows checkouts cannot leak CRLF into the generated output, and the regen harness confirms zero drift.

## [7.3.1] - 2026-04-16

### Added

- **npm pre-built native binaries for Linux x64, macOS arm64, Windows x64** (#335) -- `npm install thetadatadx` now works without a Rust toolchain. Platform-specific packages (`thetadatadx-linux-x64-gnu`, `thetadatadx-darwin-arm64`, `thetadatadx-win32-x64-msvc`) are selected automatically via `optionalDependencies`. Unsupported platforms get a clear error message at import time. CI publishes all platform packages via GitHub Actions with OIDC provenance.

## [7.3.0] - 2026-04-16

### Added

- **TypeScript/Node.js SDK via napi-rs** (#332) -- native addon exposing all 61 historical endpoints, the streaming method surface, and 13 tick types to Node.js 18+. Every method, type, and streaming dispatch is SSOT-generated from the same TOML surface that drives Python, Go, and C++. TypeScript type definitions included. CI builds and smoke-tests on every PR. npm publish workflow coming in a follow-up.

### Fixed

- **Streaming auto-reconnect now re-subscribes all active contracts** (#333) -- the reconnect path authenticated successfully but never re-sent subscription frames, so data stopped flowing after an involuntary disconnect. After reconnect login, every active subscription is now re-sent before the command channel is drained.
- **Unrecognized streaming frame codes now emitted as `UnknownFrame` with raw bytes** -- previously logged at trace level and silently dropped, so users had no visibility into unexpected server frames. Now surfaced as `StreamControl::UnknownFrame { code, payload }` with hex-encoded wire bytes in the Python and TypeScript SDKs.
- **Python and TypeScript SDKs explicitly map `Reconnecting`, `Reconnected`, and `MarketClose` control events** -- these previously fell through to the catch-all `"unknown_control"` label, which was confusing in soak-test logs.
- **FFI + Go SDK now expose `UnknownFrame` with raw payload bytes** -- the C FFI bridge maps `UnknownFrame` to kind 11 with the hex-encoded payload in the detail field (was kind 99 with no detail). Go SDK adds the `FpssCtrlUnknownFrame` constant and a complete control-kind enum for all 11 event types. All four SDKs (Python, TypeScript, Go, C++) now surface unrecognized server frames consistently.

### Changed

- **Subscription state shared with the reconnect path** (#333) -- the active per-contract and full-stream subscription tables are now reachable by the reconnect path directly, without a command-channel round-trip, and a snapshot is taken before writing frames so no lock is held during I/O.

## [7.2.1] - 2026-04-16

### Fixed

- **Greek and IV decoders regressed by v7.2.0 strict decode** -- every Greek endpoint (`option_snapshot_greeks_*`, `option_history_greeks_*`) returned `Decode failed: column N: expected Number, got Price` on live payloads. The v7.2.0 tightening routed every `f64` tick column through `row_float`, which accepts only `Number` cells, but the v3 server legitimately sends Greeks and implied-volatility values as `Price`-encoded cells. `f64` columns now decode through `row_price_f64` and accept both `Price` and `Number` cells. Regression surfaced on live run 24520486541.
- **Bulk option-chain validator cells timed out at 60 s** -- `all_strikes_one_exp` and `bulk_chain` cells on `option_history_ohlc`, `option_history_quote`, `option_history_trade_quote`, `option_history_greeks_first_order`, `option_history_greeks_implied_volatility`, and `option_at_time_quote` legitimately stream a full-chain payload that does not fit in the 60-second per-cell budget. The CLI / Python / Go / C++ validators now apply a 180-second deadline to bulk-chain / all-strike modes and keep the 60-second baseline for every other cell.

## [7.2.0] - 2026-04-16

### Added

- **Per-request deadlines and async cancellation** (#298) -- every historical endpoint now accepts `with_timeout_ms(u64)` or `with_deadline(Instant)` on its builder and a matching `WithTimeoutMs` / `WithDeadline` option in the Go SDK, C FFI, Python SDK, and C++ SDK. Underlying implementation applies a deadline timeout to the gRPC future, so cancellation is cooperative and frees server-side work promptly. Python surfaces a new `TimeoutError` class distinct from `ThetaDataError` so callers can catch slow endpoints without swallowing other failures.
- **New `tdbe::error::DecodeError` enum** (#325) -- per-cell decoding errors now carry structured `{ column, expected, observed }` context instead of a generic string. Folds cleanly into `thetadatadx::Error::Decode` at the `DirectClient` boundary.
- **`tdbe::codec::fit::FitRows`** -- a typed container replacing the previous `Vec<Vec<i32>>` return from the bulk FIT decoder. Exposes `row(i)` and `iter()` for column-major access without per-row heap allocations, materially reducing decode allocation pressure in sustained streaming.
- **Live parameter-mode matrix validator** (#287, #288, #290, #291) -- every SDK release validator (`scripts/validate_cli.py`, `scripts/validate_python.py`, `sdks/go/validate.go`, `sdks/cpp/examples/validate.cpp`) now runs one test per `(endpoint, mode)` pair instead of one per endpoint. Modes are emitted by the endpoint generator from the wire shape:
  - **List** endpoints: one `basic` mode.
  - **Stock / index / calendar / rate** endpoints: one `concrete` mode.
  - **Option `ContractSpec` endpoints** (29 endpoints): six modes each -- `concrete`, `concrete_iso`, `all_strikes_one_exp`, `all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`.
  - **Per-optional-parameter coverage**: every optional builder parameter gets its own `with_<param>` cell, plus a compound `all_optionals` cell. Compound pairs like `start_time`+`end_time` collapse into a single `with_intraday_window` cell.
  - Streaming endpoints remain exercised by `scripts/fpss_smoke.py` / `fpss_soak.py`.
- **Upstream-derived tier and wildcard maps** (#290, #291) -- dropped hand-maintained `endpoint_min_tier` and `endpoint_supports_expiration_wildcard` match statements in favor of generator-time lookups against a pinned upstream OpenAPI snapshot. The parser fails closed on three drift classes: missing `x-min-subscription`, zero-endpoint snapshots, and unknown `expiration` variants. Surfaced and corrected one stale label (`option_snapshot_market_value` was `value`, upstream says `standard`).
- **Cross-language agreement check** (#290, #291) -- `scripts/validate_agreement.py` loads per-language validator artifacts at `artifacts/validator_<lang>.json` and asserts every `(endpoint, mode)` cell present in at least two SDKs agrees on `status` and `row_count`. `scripts/validate_release.sh` runs CLI -> Python -> Go -> C++ -> agreement in order.
- **Structured field-level diff in validator output** (#293) -- the release validator now emits per-field diffs instead of opaque equality failures, so drift between SDKs is traceable without re-running.
- **Per-cell 60-second timeout on every validator** -- every cell is bounded by a hard 60-second timeout with language-specific hygiene (daemon thread + queue on Python, `packaged_task` + `_Exit` on C++, goroutine + timeout-channel + deferred-close gate on Go, `subprocess.run(timeout=60)` on CLI).
- **Public API redesign charter** (#282) -- `docs/public-api-redesign.md` lays out the layered ergonomic facade plan (canonical parity layer, handwritten `historical` / `realtime` / `analytics` facades, typed value foundations, compatibility window). The streaming category is named `realtime` to avoid overloading the meanings of `live` in CI and run-mode contexts.

### Changed

- **SDK surface is now fully declarative TOML** (#300) -- every generated method signature, optional-parameter shape, streaming dispatch, FFI wrapper, Python binding, Go function, and C++ method is projected from `sdk_surface.toml`, `endpoint_surface.toml`, and `tick_schema.toml`. Adding or changing a method is a TOML edit plus a regeneration step, with no hand-editing of per-language glue.
- **`parse_*_ticks`, `parse_option_contracts_v3`, `parse_calendar_days_v3` now return `Result<Vec<T>, DecodeError>`** (#325) -- the generated and hand-written row-decoders previously returned `Vec<T>` and silently coalesced per-cell type mismatches to zero. Mismatches now propagate as `DecodeError::TypeMismatch { column, expected, observed }` which folds into `Error::Decode` at the `DirectClient` boundary. This is a Rust-caller-visible breaking change for anyone reaching past `DirectClient::*` into the free functions; the SDK `Result<Vec<T>, Error>` shape users actually call is unchanged, so no ABI / FFI / Python / Go / C++ contract moves.
- **`Contract::option` now returns `Result`** (#324) -- constructing an option contract from user-supplied strings can now surface invalid `expiration` / `strike` / `right` input through `?` instead of panicking on malformed callers.
- **FIT decoder exposes `FitRows`** (tdbe 0.10.0) -- bulk decode returns a dedicated type instead of `Vec<Vec<i32>>`. Callers who passed the old nested-vec shape into downstream helpers need to switch to `FitRows::row()` / `iter()`.
- **`Error::Decode` display text now reads "Decode failed: ..."** (was "Protobuf decode failed: ...") -- the variant now carries both protobuf deserialization errors and post-decode per-cell type-mismatch failures, so the old label was misleading.
- **Endpoint code generator split into a focused module tree** (#294) -- what was one 2700-line file is now organized by concern. Public behavior is unchanged; finding where a code-gen step lives is now a two-click navigation instead of a search.
- **Generator templates moved into files** (#296, #301) -- the generators no longer carry generated source as embedded Rust string literals; each generated language has its own template directory. Editing a generated code shape no longer means editing a Rust string literal with embedded foreign syntax.
- **Test-mode fixtures now live in TOML** (#295) -- per-mode test-fixture values were previously a Rust match statement; they are now in `sdk_surface.toml` under `[test_modes.<mode>]`. The generator reads them and emits identical code.
- **`scripts/check_tier_badges.py` live-fetches upstream `openapiv3.yaml`** (#280) -- removed `scripts/upstream_tiers.json` and pulls the authoritative `x-min-subscription` map at check time, with 4 retries + exponential backoff and fail-closed on exhaustion. Eliminates the manual snapshot-refresh drift vector.
- **Validator tier gating is server-driven** -- the four live matrix validators no longer depend on a client-side `VALIDATOR_ACCOUNT_TIER` env var. Every cell is attempted; `PermissionDenied` / `subscription` errors from the server classify as `SKIP: tier-permission` with the declared min_tier echoed, and real bugs continue to surface as `FAIL`. Wildcard-expiration modes (`all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`) are suppressed on the 7 endpoints upstream binds to `expiration_no_star`, because the v3 server rejects `*` for those.
- **Full-vocabulary wildcard support for option contract parameters** (#284) -- `validate_expiration` accepts `*`, `YYYYMMDD`, and `YYYY-MM-DD`; new `validate_strike` accepts `*` / `0` / empty (wildcard) or a positive decimal. `direct::wire_strike_opt` and `direct::wire_right_opt` map wildcard sentinels to `None` so `ContractSpec` leaves the field unset on the proto, matching what the server documents. Live-verified against production across 64 parameter-mode combinations. A full option chain's open interest for QQQ now returns all 10,158 rows in ~1s (a single bulk call), down from a 34-expiration serial loop (~22s).
- **`tdbe` bumped to 0.10.0** -- carries the `FitRows` shape change and the `DecodeError` enum (both public-surface breaking under 0.x rules).

### Fixed

- **Streaming client is now `Sync`-safe** (#324) -- the internal read/write halves and session state are now properly guarded so sharing an `StreamingClient` across threads is sound. Previously a latent data race existed on reconnection bookkeeping. Marked `unsafe impl Sync` with the exact invariants documented inline.
- **Python streaming deadlock on shutdown** (#324) -- `next_event()` no longer blocks other Python threads while it waits on the event queue, and shutdown coordinates with the blocking reader so Ctrl+C interrupts streaming loops cleanly instead of hanging.
- **Python Ctrl+C interruptibility** (#324) -- long-running gRPC calls now run without blocking other Python threads and cooperate with Python's signal handling, so Ctrl+C returns control to the interpreter without waiting for the server.
- **FFI `CString` interior-NUL swallowing** (#303, #324) -- string outputs across the C ABI now surface `CString::new` failures via `thetadatadx_last_error` instead of silently truncating at the embedded NUL byte. Callers that previously saw empty strings on malformed input now see a diagnosable error.
- **gRPC `Status` parsing propagates ThetaData error codes** (#303) -- the server's numeric error codes are now extracted from the `Status` trailers and surfaced by name, so failures like `INVALID_SYMBOL` read as `INVALID_SYMBOL` instead of the raw integer.
- **Protobuf `DataValue` type coercion** (#303) -- mixed `Price` / `Number` encoding on OHLC cells is normalized consistently across all endpoints; previously a minority of Greeks rows decoded as zero when the server encoded them differently from the cell type hint.
- **Go TLS error-channel races on reconnect** (#324) -- closing a streaming TLS connection concurrently with an in-flight read no longer produces a spurious send-on-closed-channel panic on Go. The error channel is now drained with a select-default rather than assuming the receiver is still alive. CGo callbacks are also pinned to the calling OS thread to keep the TLS session's thread-local state consistent.
- **Subscription drop on lock poison** (#324) -- active streaming subscriptions used to silently vanish if a panic poisoned the internal state mutex; the subscription tables now recover via `.into_inner()` so reconnection still finds the intended subscriptions.
- **Float → i32 overflow and panic on invalid strike input** (#324) -- strike parsing now bounds-checks the implied i32 representation before conversion, returning a structured error instead of panicking on a pathological user input (e.g. `"999999999.99"`).
- **Greeks recomputation avoided on unchanged inputs** (#324) -- the Black-Scholes call path memoizes on the common `(spot, strike, vol, rate, t)` tuple so the analytics endpoints no longer recompute identical Greeks on back-to-back rows.
- **FIT decoder allocator thrash** (#324) -- the bulk FIT decoder now reuses a single backing buffer through `FitRows` instead of allocating per-row, cutting sustained streaming allocation rate by roughly an order of magnitude on busy symbols.
- **Double string allocation on `Contract` clone** (#324) -- `Contract` now shares its symbol behind a reference count so cloning into per-subscription bookkeeping does not copy the byte buffer twice.
- **JSON serialization moved off the streaming I/O thread** (#324) -- `next_event` now returns typed structs and the serialization step is only paid at the FFI boundary when the caller asks for JSON, keeping the streaming hot path allocation-free.
- **`parse_right` no longer panics on unrecognized input** (#324) -- the canonical right parser returns a structured error for unknown vocabulary instead of panicking, so a single malformed row can no longer take down the decoder.
- **Unset `DataValue` oneof fails loud in every strict decoder** (#326) -- `parse_option_contracts_v3` (expiration, right), `parse_calendar_days_v3` (date, type, open, close), and the generator-emitted EOD helpers plus contract-id injected `expiration` / `right` used to treat a `DataValue` whose `data_type` oneof was unset as a legitimate null and coalesce to `0`. They now return `DecodeError::TypeMismatch { observed: "Unset" }`, matching `row_number` / `row_date` / `row_float` / `row_text` / `row_number_i64` / `row_price_f64` and the JVM terminal's default arm. `NullValue` is still coalesced (legitimate null); only the wire-anomaly path changes.
- **Option contract wildcard rejection** (#284) -- before this release the SDK had no working path to the server's bulk-chain mode: `*` was rejected client-side by `validate_expiration`, and `0` was rejected server-side. The SDK vocabulary now covers the full cross-product the server accepts.
- **Validator tier detection drift** (#289) -- dropped the static tier gate that classified legitimate server responses as SKIP. The runtime permission fallback still catches drift between docs and the wire (for example, `interest_rate_history_eod` being labelled `free` on docs but gated higher by the server).
- **CI unbroken on `main`** (#299) -- fixed a `timeout_ms` TOML field mismatch and made the Go pin-test CRLF-robust.
- **Streaming internal visibility tightening** -- subscription state is now scoped to the streaming module tree rather than left more broadly visible, and the reconnect-delay tests assert against the named delay constants instead of hard-coded millisecond literals so they cannot drift from the real values. Internal-only; no public surface change.

### Security

- **Session token no longer leaks via `Debug`** (#324) -- the auth response's `session_token` field is now redacted in its `Debug` output, so a debug log of the response no longer writes the bearer token into logs. Credentials were already redacted; this closes the parallel leak on the response side.

### Changed

- **Generator bloat cleanup** (#302) -- stripped roughly 1,500 lines of ceremony, over-abstraction, and redundant tests across the code generators and the SDK layers. Behavior identical, surface identical, just less to read.
- **Streaming module split into focused submodules** (#327) -- what was a 2,143-line single file is now organized by responsibility (accumulator, decode, events, I/O loop, session). Internal-only; public behavior is unchanged.
- **Per-cell rationale + redundancy audit in tests** (#297) -- generated test cells now carry a one-line rationale in the comment, so deleted or merged cells leave an obvious trail for reviewers.
- **Consolidated CI workflow cleanup** (#323) -- shared the Rust-dep setup across jobs via a reusable composite action (`.github/actions/setup-rust-deps`), removed duplicated workflow steps, and narrowed `live` to manual dispatch so routine CI stays deterministic.
- **Python abi3 smoke CI no longer rebuilds the wheel** (#304) -- the smoke job now reuses the wheel built earlier in the pipeline, cutting the job's runtime materially.

## [7.1.0] - 2026-04-14

### Removed

- **Greeks utilities now take `right: &str` instead of `is_call: bool`** (#278) -- `tdbe::greeks::all_greeks` and `tdbe::greeks::implied_volatility` accept the same permissive vocabulary as the rest of the SDK (`"C"`/`"P"`, `"call"`/`"put"`, case-insensitive) via the canonical `parse_right_strict`. Panics with a descriptive message on unrecognised input or the `both`/`*` wildcards. The signature change cascades to the Python SDK (`right: str`), Go SDK (`right string`), C++ SDK (`const std::string& right`), C FFI ABI (`thetadatadx_all_greeks` / `thetadatadx_implied_volatility` take `const char* right`), the `thetadatadx greeks` / `thetadatadx iv` CLI subcommands, and the MCP `all_greeks` / `implied_volatility` tool input schemas. The low-level per-Greek primitives (`value`, `delta`, `theta`, ...) continue to take raw `bool` — they are pure-math helpers not in scope. Motivation: consistency with `Contract::option`, `normalize_right`, and `validate_right` so callers stop flipping between `"C"` strings and `true` bools in the same session.
- **`tdbe` bumped to 0.9.0** -- breaking public signature change in `greeks`.
- **`thetadatadx`, `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-mcp`, `thetadatadx-server`, `thetadatadx-py`, and the C++ SDK (CMake project) bumped to 7.1.0** -- downstream version bumps to carry the breaking FFI ABI change.

### Changed

- **`thetadatadx::right` is now a thin re-export of `tdbe::right`** (#278) -- the canonical `right` parser moved into the pure-data `tdbe` crate so `tdbe::greeks` could reuse it without `tdbe` reverse-depending on `thetadatadx`. Public API (`parse_right` / `parse_right_strict` / `ParsedRight` with all four projections) is unchanged at the `thetadatadx::right` path. The error type now returns `tdbe::error::Error::Config` instead of `thetadatadx::error::Error::Config`; a `From<tdbe::error::Error> for thetadatadx::Error` conversion is provided so `?` in `thetadatadx`-returning functions keeps working.
- **Top-level re-exports for offline Greeks** (#278) -- `thetadatadx::{all_greeks, implied_volatility, GreeksResult}` now re-export from `tdbe::greeks` so SDK consumers can avoid reaching into the `tdbe` crate directly. Docs prefer `use thetadatadx::all_greeks;`.
- **Centralized `right` parsing** (#270) -- new `thetadatadx::right` module exposes `parse_right` / `parse_right_strict` returning a `ParsedRight` enum that carries every downstream representation (historical lowercase string, streaming `is_call` bool, short-form `"C"`/`"P"`, streaming wire byte). `normalize_right` in `direct.rs`, `validate_right` in `validate.rs`, and `Contract::option` in `fpss/protocol.rs` all route through it.
- **OpenAPI YAML aligned with upstream ThetaData** (#270) -- `right-param` enum in `docs-site/public/thetadatadx.yaml` extended to `[call, put, both, C, P, c, p, CALL, PUT, Call, Put, "*"]` to match what the server actually accepts (strict superset of upstream's `[call, put, both]`). Response `right` stays `type: string` with a note documenting the current `"C"`/`"P"` output shape.

### Fixed

- **Silent put-default on invalid `right` in `Contract::option`** (#270) -- previously `Contract::option(..., "xyz")` silently constructed a put contract because the parser only checked for call forms. Now panics with a descriptive message, consistent with the existing strike/expiration panic style.

### Changed

- Every Greeks example in the docs-site, READMEs, Python example, and notebooks updated to pass `right: "C"` / `right="C"` / `right: "C"` instead of `is_call: true`.
- Note added to `docs-site/docs/api-reference.md` and `docs/api-reference.md` clarifying that the low-level per-Greek primitives still take `is_call: bool`, while the user-facing aggregates take `right: &str`.
- **Corrected 31 subscription-tier badges across `docs-site/docs/historical/**/*.md`** (#276) -- audit against ThetaData's canonical `openapiv3.yaml` (`x-min-subscription` field) found 31 of 57 endpoint docs advertised the wrong subscription tier. Fixed against upstream truth.
- **Renamed misnamed doc file** (#276) -- `historical/option/at-time/ohlc.md` actually documented the `option_at_time_quote` endpoint; renamed to `quote.md`, fixed the nav link in `docs-site/docs/.vitepress/config.ts`, and updated the sole inbound reference in `historical/option/index.md`.
- **New `scripts/check_tier_badges.py`** (#276) -- validates every `<TierBadge>` in the historical docs against `scripts/upstream_tiers.json`, a checked-in snapshot of ThetaData's authoritative `x-min-subscription` map (with `_source` and `_captured_at` keys for traceability). Wired into `scripts/check_docs_consistency.py` so the existing `Extended Surfaces` CI job gates tier drift automatically. No network calls at CI time.
- **Deleted orphan docs-site pages** (#272) -- removed top-level single-page versions (`getting-started.md`, `historical.md`, `historical/{stock,option,index-data,calendar}.md`, `streaming.md`, `tools/index.md`) superseded by the subdirectory navigation. Added a `## Client Model` section to `docs-site/docs/streaming/index.md` that makes the per-SDK split (Rust/Python unified `Client`, Go/C++ standalone `StreamingClient`) unmistakable. Removed `ignoreDeadLinks: true` from `docs-site/docs/.vitepress/config.ts` so future link rot fails the VitePress build.
- **Sidebar landings for Historical Data and Tools sections** (#274) -- added `link:` fields on both top-level sidebar entries so clicking the section headers lands on the category overview. Created a new `tools/index.md` overview describing the CLI / MCP / REST Server trio.

## [7.0.0] - 2026-04-14

### Removed

- **`SnapshotTradeTick` deleted from all layers** -- removed from Rust core, FFI, Python, Go, and C++ SDKs. Dead type that was never returned by any endpoint.
- **FFI options use explicit `has_*` flags** -- replaced NaN/`-1` sentinel-based optional fields with `has_exclusive`, `has_max_dte`, `has_strike_range`, `has_annual_dividend`, etc. C, Go, and C++ consumers must check the companion `has_*` i32 flag (0 = unset, 1 = set) before reading the value.
- **The checked-in code generator restored as the surface authority** -- the codegen step is required again and is the canonical way to regenerate and verify the generated SDK/FFI/tool surfaces from TOML.
- **Streaming endpoints generated from TOML** -- hand-written streaming endpoint blocks in `direct.rs` replaced by TOML-driven codegen. Method signatures unchanged but internal dispatch is generated.
- **Endpoint, utility, streaming wrapper, and tick projection surfaces are spec-driven** -- Rust, FFI, Python, Go, C++, CLI, and MCP now project their generated public surfaces from `endpoint_surface.toml`, `sdk_surface.toml`, and `tick_schema.toml`.
- Removed the misleading per-contract `subscribe_option_full_*` / `unsubscribe_option_full_*` streaming methods from the C FFI, Go SDK, and C++ SDK. Per-contract streams use `subscribe_option_*`; full-stream subscriptions remain `subscribe_full_*` by security type.
- Python streaming option subscription helpers now take `(symbol, expiration, strike, right)` to match Rust, Go, and C++ argument order.
- **Go/C++ `contract_map` API replaced** -- `ContractMapJSON()` / `contract_map_json()` removed; replaced with typed `ContractMap()` / `contract_map()` returning `map[int32]string` / `std::map<int32_t, std::string>`. Callers of the old JSON variant will fail to compile.

### Removed

- `public-api-redesign.md` and README reference.
- `migration-from-rest-ws.md` and navigation/index references.
- 1,134 lines of commented-out legacy Python methods.
- obsolete claim that the checked-in code generator had been removed.

### Changed

- Workspace version bumped from 6.0.0 to 7.0.0.
- `tdbe` bumped from 0.7.0 to 0.8.0. `tdbe@0.7.0` was yanked from crates.io because it shipped with a broken `MarketValueTick` schema (five stale fundamental fields); the 0.8.0 release carries the corrected `market_bid` / `market_ask` / `market_price` layout.
- Docs consistency checker now points at correct generated files.
- `StreamControl::LoginSuccess { permissions }` documented as opaque diagnostic metadata.
- Public endpoint and utility surfaces now project optional request parameters consistently across Rust, Python, Go, C++, CLI, MCP, and REST from the checked-in specs.
- Python now exposes `reconnect()` on the unified streaming client, matching the existing Go/C++ streaming reconnect capability.
- `time_of_day` accepts both legacy millisecond strings and formatted wall-clock inputs such as `9:30`, `09:30:00`, and `09:30:00.000`, then normalizes to canonical `HH:MM:SS.SSS`.
- Release validation and live smoke harnesses were added and the GitHub live workflow was narrowed to manual dispatch so routine CI stays deterministic.

### Fixed

- `market_value` endpoints now decode `Price` cells correctly instead of returning zeroed prices.
- Release validation, generated Python/Go validators, and cross-platform CLI validation now use valid fixtures and treat legitimate empty responses correctly.
- C++ tick ABI layout now matches the aligned Rust FFI structs, fixing multi-element array stepping bugs.
- Windows Go FFI builds now use the correct GNU-targeted Rust artifacts when building with CGo on GitHub runners.
- Docs and OpenAPI now reflect the real at-time contract and strike wildcard semantics.
- Docs consistency checker no longer references deleted `migration-from-rest-ws.md`.
- `cargo fmt` applied to the endpoint code generator.

## [6.0.1] - 2026-04-06

### Removed

- **All tick price fields changed from `i32` to `f64`** -- prices are decoded during parsing. Users access `tick.bid`, `tick.price`, `tick.open` directly as `f64`. No more `price_type` or `_f64()` helpers.
- **`price_type` removed from all public APIs** -- historical ticks, streaming events, FFI, Python, Go, C++.
- **`strike_price_type` removed** -- `strike` is now `f64` on all tick structs.
- **All `_f64()` and `_price()` helper methods removed** -- `bid_f64()`, `get_price()`, `open_price()`, `trade_price()`, `midpoint_price()`, `midpoint_value()`, `strike_price()` no longer exist.
- **Streaming events: prices are `f64`** -- `StreamData::Quote`, `Trade`, `Ohlcvc` expose `f64` fields directly. No `price_type`. No `_f64` dual fields.
- **`Contract::option()` takes 4 strings** -- `Contract::option("SPY", "20260417", "550", "C")` instead of `(root, i32, bool, i32)`. Matches the historical API experience.
- **Python SDK**: `subscribe_option_*` takes `(symbol, exp_date, right, strike)` as strings. Removed `price_raw`, `bid_raw`, `price_type` from dicts.
- **Go SDK**: removed `RightRaw`, `StrikePriceType`, `PriceRaw`, `BidRaw`/`AskRaw`/`OpenRaw`/etc., `PriceToF64()`.
- **C++ SDK**: all price fields are `double`. Removed `thetadatadx::price_to_f64()`, `thetadatadx::bid_f64()`, `thetadatadx::open_f64()`, etc.
- **CLI**: `price_type` column removed from all table output.

### Added

- **`QuoteTick.midpoint`** -- pre-computed `(bid + ask) / 2.0` at parse time.
- **`Contract::option_raw()`** -- raw wire-format constructor for the drop-in REST/WS server.
- **Go FFI layout tests** -- compile-time `unsafe.Sizeof` assertions for all 12 C-mirror structs.
- **WebSocket zero-copy fan-out** -- per-client broadcast channel, JSON serialized once.
- **Server `--no-ohlcvc` flag** -- disable OHLCVC bar derivation from trades.
- **CLI price formatting** -- preserves up to 6 meaningful decimals, trims trailing zeros.

### Fixed

- **`tools/server` and `tools/mcp` compilation** -- updated for f64 migration (were excluded from workspace, broke silently).
- **Go FFI struct padding** -- 8 structs had incorrect tail padding causing memory corruption on multi-element arrays.
- **`OptionContract` missing `Debug + Clone` derives** -- accidentally removed during refactor.
- **Server dead match arm** -- removed v2 parameter fallback code.

### Changed

- All 61 endpoint pages updated: f64 fields, no `price_type`, no `_f64()` helpers.
- All SDK READMEs updated (Rust, Python, Go, C++).
- Streaming docs rewritten for f64 events.
- OpenAPI spec purged of `price_type`.
- JVM deviations doc: new sections for streaming f64 events and `Contract::option` clean API.
- Internal docs (architecture, api-reference, endpoint-schema) updated.
- README now explicitly warns that streaming is not yet production-ready due to the upstream framing issue tracked in `#192`.

## [5.4.0] - 2026-04-05

### Removed

- **`start_streaming_no_ohlcvc()` removed** -- use `DirectConfig::derive_ohlcvc(false)` instead. (#129)
- **Go SDK**: `SnapshotTradeTick` type removed (was dead code after FFI cleanup).

### Added

- **`DirectConfig::derive_ohlcvc(bool)`** -- config-driven OHLCVC opt-out, replaces duplicate method. (#129)
- **REST server drop-in replacement** -- `--email`/`--password`, `--config`, `--streaming-region` CLI args. `/v3/system/status` endpoint. Startup banner. (#128)
- **Error suppression 5s after STOP** -- matches the JVM terminal's behavior. (#124)
- **Auth retry on transient errors** -- 3 attempts, 2s delay, network errors only. (#125)
- **Config validation** -- clamps queue_depth (16-1M), window_size (64-1024) with warnings. (#126)
- **Password character warning** -- on INVALID_CREDENTIALS disconnect. (#127)
- **Clippy pedantic zero warnings** -- `#[must_use]`, inlined format args, numeric separators, `try_from` casts, error docs. No blanket suppression. (#131)

### Fixed

- Zero `#[allow(dead_code)]` in entire project.
- Go SDK dangling extern for removed `ThetaDataDxSnapshotTradeTickArray`.
- Doc comment typo `100_0000` -> `1_000_000`.
- Test warning on unused `#[must_use]` return.
- All `#[allow]` annotations have reason comments.

## [5.3.1] - 2026-04-04

### Added

- **Streaming auto-reconnect** with configurable policy: `Auto` (default, matches the JVM terminal), `Manual`, `Custom(fn)`. New control events: `Reconnecting`, `Reconnected`. (#119)
- **Trade/quote condition descriptions** with special-case annotations (e.g., `*update last if only trade`).

### Fixed

- **Greeks returned all zeros** on intraday endpoints (`greeks_first_order`, `greeks_iv`, etc.). The v3 server sends Greeks as Price-encoded cells; `row_float()` now decodes them. (#118)
- **`expiration=0` on wildcard EOD** -- contract ID extraction now handles ISO date text ("2024-01-31" -> 20240131). (#117)
- **`implied_volatility` -> `implied_vol`** header alias added for v3 server column name.
- **Raw strike encoding in docs** -- replaced "500000" with "500" (dollar amounts) across 37 files.
- **`"EOD"` removed from docs** -- v3 uses `"TRADE"` / `"QUOTE"` only.
- **Options examples** rewritten to use wildcard bulk queries instead of per-strike loops.

## [5.3.0] - 2026-04-04

### Removed

- **Go SDK**: `EodTick`, `OhlcTick`, `TradeTick`, `QuoteTick`, `TradeQuoteTick`, `PriceTick`, `SnapshotTradeTick` gain additional fields (raw prices, ext_conditions, price_type). `Right` is now `string` ("C"/"P") with `RightRaw int32` for raw access.
- **Python SDK**: trade dicts gain `ext_condition1..4`. Quote/OHLC/EOD/TradeQuote dicts gain raw price and detail fields.
- **Rust**: `normalize_right()` maps `"C"` -> `"call"`, `"P"` -> `"put"`, `"*"` -> `"both"` for v3 server.

### Added

- **`tdbe::exchange`** -- 78 exchange codes with O(1) lookup: `exchange_name()`, `exchange_symbol()`. (#112)
- **`tdbe::conditions`** -- 149 trade conditions + 75 quote conditions with semantic flags (cancel, volume, high, low, last). (#112)
- **`tdbe::sequences`** -- streaming sequence tracking with wrapping-aware gap detection. (#112)
- **`tdbe::error`** -- 14 ThetaData HTTP error codes mapped to human-readable names. gRPC errors now include the ThetaData error name. (#113)
- **OHLC price normalization** -- `row_price_value_normalized()` and `change_price_type()` handle mixed price_types across OHLC fields. (#106)
- **Greeks from Price cells** -- `row_float()` decodes Price-typed cells. `implied_vol` header alias. (#106)
- **Calendar v3 parser** -- handles text dates, text times, and type codes from v3 server. (#109)
- **`normalize_right()`** -- maps C/P/* to call/put/both for v3 server. Go `RightStr()` helper. (#111)
- **Full SDK parity** -- Python and Go SDKs now expose every field from every Rust tick type.
- **Latency physics documentation** -- speed-of-light calculations, colocation guidance, Mermaid diagrams.

### Fixed

- **37% of OHLC intraday bars had wrong prices** -- mixed price_type per cell caused 10x errors. (#106)
- **All Greeks returned 0.0** -- server sends Greeks as Price cells, not Number cells. (#106)
- **`option_list_contracts` returned 0** -- v3 server uses "symbol" not "root", ISO dates, text right. (#97)
- **Calendar endpoints returned zeros** -- v3 text format mismatch. (#109)
- **Dev server streaming crashes** -- binary Error frames and unknown codes handled gracefully. (#85)
- **`PriceToF64` Go formula wrong** -- was `value / 10^pt`, corrected to `value * 10^(pt-10)`.
- **Python `greeks_tick_to_dict` missing 15 fields** -- now has all 24.

### Changed

- 14 documentation fixes across 13 files
- Mermaid diagrams replacing ASCII art in VitePress docs
- Latency physics section with speed-of-light calculations per geography
- 3 new JVM deviations documented
- v3 migration guide compliance verified

## [5.2.1] - 2026-04-04

### Fixed

- `option_list_contracts` returned 0 contracts. The v3 historical server sends `symbol` (not `root`), ISO date strings (not YYYYMMDD integers), and `PUT`/`CALL` text (not integer codes). Added `root` -> `symbol` header alias and a v3-aware parser. (#97)
- Dev server streaming replay boundary corruption handled gracefully. Binary Error frames are silently skipped. Unknown message codes are skipped with bounded retry (5 consecutive = framing corruption -> clean disconnect). (#85)

## [5.2.0] - 2026-04-04

### Removed

- **Go SDK**: price fields on public structs are now `float64` (decoded). Raw `int32` values available as `*Raw` fields. `PriceType` removed from public structs.
- **Go streaming events**: `FpssQuote.Bid`/`Ask`, `FpssTrade.Price`, `FpssOhlcvc.Open`/`High`/`Low`/`Close` are now `float64`. Raw values as `*Raw` fields.
- **Rust streaming events**: `StreamData::Quote`, `Trade`, `Ohlcvc` gain pre-decoded `*_f64` fields (`bid_f64`, `price_f64`, etc.).

### Added

- **Rust `_f64()` convenience methods** on all tick types: `price_f64()`, `bid_f64()`, `ask_f64()`, `open_f64()`, `high_f64()`, `low_f64()`, `close_f64()`, `midpoint_f64()`. (#95)
- **Go pre-decoded f64 prices** on all public structs and streaming events. Users get `tick.Price` as `float64` ready to use. (#95)
- **C++ `thetadatadx::` price helpers** -- 17 inline functions for f64 price decoding on all tick types.
- **FFI streaming events** gain `*_f64` fields (`bid_f64`, `ask_f64`, `price_f64`, `open_f64`, `high_f64`, `low_f64`, `close_f64`) pre-computed during event construction.

### Fixed

- **Go `PriceToF64` formula** was `value / 10^pt` instead of `value * 10^(pt-10)`. All streaming prices would have been wrong. (#95)

## [5.1.1] - 2026-04-03

### Fixed

- `tdbe` dependency bumped to 0.2.0 for crates.io publish (0.1.x was yanked). No code changes.

## [5.1.0] - 2026-04-03

### Removed

- **Streaming FFI events now use C-layout typed structs** instead of JSON serialization. `thetadatadx_streaming_next_event` and `thetadatadx_client_next_event` return `*mut ThetaDataDxStreamEvent` (a flat tagged struct with quote, trade, open interest, OHLCVC, control, and raw_data variants). Free with `thetadatadx_streaming_event_free`. (#82)
- C++ SDK: `StreamingClient::next_event()` returns an owning event pointer (RAII unique_ptr to `ThetaDataDxStreamEvent`).
- Go SDK: `StreamingClient.NextEvent()` returns `*StreamEvent` with typed Go structs.
- Streaming event prices are now raw integers with `price_type` (matching the wire format). Callers decode with `Price::new(value, price_type).to_f64()` or `thetadatadx::price_to_f64(value, price_type)`.
- The JSON dependency is removed from the FFI crate -- zero JSON crosses the FFI boundary.

### Added

- **Contract identification on all 10 option tick types** -- `expiration`, `strike`, `right`, `strike_price_type` fields populated by the server on wildcard queries. Helper methods `strike_price()`, `is_call()`, `is_put()`, `has_contract_id()` on all 10 tick types via `impl_contract_id!` macro. (#84)
- **8-field trade tick support** -- the streaming dev server sends abbreviated 8-field trade ticks; production sends 16-field. `decode_tick()` now auto-detects the field count from the first absolute tick per contract and dispatches to the correct index mapping. (#86)
- **C-layout streaming event structs** in all SDKs -- `ThetaDataDxStreamQuote`, `ThetaDataDxStreamTrade`, `ThetaDataDxStreamOpenInterest`, `ThetaDataDxStreamOhlcvc`, `ThetaDataDxStreamControl`, `ThetaDataDxStreamRawData` with tagged `ThetaDataDxStreamEvent` wrapper. (#82)
- `FfiBufferedEvent` with owned backing storage for safe cross-thread `Send` of pointer-containing structs.
- Go SDK: `FpssQuote`, `FpssTrade`, `FpssOpenInterestData`, `FpssOhlcvc`, `FpssControlData` Go structs mirroring the Rust C-layout.
- C++ SDK: `StreamingClient` class with an owning RAII event pointer for streaming.
- Python SDK: `greeks_tick_to_dict` now emits all 24 fields (was 8). (#92)
- `tdbe`: contract ID fields and `impl_contract_id!` macro on all 10 tick types.

### Fixed

- **9 stale JSON references** in FFI doc comments, FFI README, Go README, docs-site API reference, and macro guide -- all now correctly describe typed structs. (#92)
- Python SDK `greeks_tick_to_dict` missing 16 fields (vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda, vera, date). (#92)
- Go SDK README documented `ActiveSubscriptions()` return type as `json.RawMessage` -- actually returns `[]Subscription`. (#92)
- docs-site Go streaming example said "returns json.RawMessage or nil" -- now says "*StreamEvent or nil".

## [5.0.2] - 2026-04-03

### Fixed

- OHLCVC accumulator `volume` and `count` fields widened from `i32` to `i64` to prevent integer overflow on high-volume symbols during dev server replay. (#80)

## [5.0.1] - 2026-04-03

### Fixed

- `StreamingClient::connect()` now uses `DirectConfig::streaming_hosts` instead of hardcoded production servers. `dev()` and `stage()` configs now correctly connect to their respective streaming servers. (#77)
- Removed dead `SERVERS` constant from `protocol.rs`

## [5.0.0] - 2026-04-02

### Removed

- **Builder pattern on all 61 endpoints** -- methods return builders with `IntoFuture`. `start_time`/`end_time` are now builder methods, not positional params. All optional proto params exposed as chainable setters.
- `received_at_ns: u64` added to every `StreamData` variant (Quote, Trade, OpenInterest, Ohlcvc)
- `DirectConfig::dev()` now uses actual ThetaData dev streaming servers (port 20200, infinite replay) instead of production with reduced buffers

### Added

- **Builder pattern** -- all endpoints return chainable builders. Zero noise for simple calls, all optional proto params discoverable via autocomplete.
- **`received_at_ns`** -- nanosecond receive timestamp on every streaming event for latency measurement
- **`tdbe::latency::latency_ns()`** -- DST-aware wire-to-application latency computation
- **`StreamingFlushMode`** -- `Batched` (default, matches the JVM terminal) or `Immediate` (lowest latency)
- **Metrics** -- `metrics` crate integration. Counters/histograms on all gRPC, streaming, and auth operations. Zero overhead when no backend installed.
- **Config file** -- `DirectConfig::from_file()` behind `config-file` feature flag. TOML format matching v3 terminal.
- **`DirectConfig::stage()`** -- staging streaming servers (port 20100)
- **3 streaming methods** in all SDKs -- `subscribe_full_open_interest`, `unsubscribe_full_trades`, `unsubscribe_full_open_interest`
- **Cross-platform CI** -- Format, Lint, Test, FFI Build on Ubuntu + macOS + Windows
- **Macro guide** -- `docs/macro-guide.md` for contributors
- **DST pre-2007 safety net** -- handles old US DST rules (April-October) for pre-2007 dates
- **`unsubscribe_option_open_interest`** in Python SDK (was missing)
- **Go `StreamingClient`** -- complete standalone streaming client wrapper (`sdks/go/fpss.go`)

### Fixed

- 30 documentation findings from production audit (version pins, method tables, CHANGELOG, SECURITY)
- 14 public methods missing doc comments on `Client`
- Python SDK `lock().unwrap()` changed to poison recovery
- Legacy `config.default.properties` removed (v2 artifact)

## [4.5.0] - 2026-04-02

### Removed

- **FFI: C-layout typed struct arrays replace JSON** -- all 60 data endpoints now return native struct arrays across the FFI boundary. C++ and Go SDKs read fields directly, zero JSON serialization. Streaming events remain JSON (variable schemas).
- C++ `OptionContract` now uses `std::string root` (was `const char*`)
- Go SDK gains 9 previously missing Greeks endpoints

### Added

- **DST-aware timezone conversion** -- `eastern_offset_ms()` correctly handles EST/EDT transitions using US Energy Policy Act 2005 rules. Historical data from November-March now has correct ms_of_day values. (#32)
- **gRPC flow control config** -- `DirectConfig` gained `mdds_window_size_kb` and `mdds_connection_window_size_kb`, wired into the gRPC channel builder. (#36)
- Go SDK: `OptionSnapshotGreeksFirstOrder`, `OptionSnapshotGreeksSecondOrder`, `OptionSnapshotGreeksThirdOrder`, `OptionHistoryGreeksFirstOrder/SecondOrder/ThirdOrder`, `OptionHistoryTradeGreeksFirstOrder/SecondOrder/ThirdOrder` (#39)
- Go SDK: `SnapshotTradeTick` type and converter
- Go SDK: `Vera` field on `GreeksTick`
- FFI: 13 typed tick array types (`ThetaDataDxEodTickArray`, `ThetaDataDxOhlcTickArray`, etc.) with `from_vec`/`free`
- FFI: `ThetaDataDxStringArray` for list endpoints, `ThetaDataDxOptionContractArray` for contracts
- C++ header: `thetadx.h` with all C-layout struct definitions and function signatures

### Fixed

- **Timezone hardcoded UTC-4** -- was producing ms_of_day shifted +1 hour for all Nov-Mar historical data. Now DST-aware with 5 unit tests. (#32)
- **EOD parser divergent alias system** -- unified to shared `find_header()`. (#34)
- **reconnect_wait_ms** -- changed from 1000 to 2000 to match the JVM terminal. (#35)
- **C++ OptionContract use-after-free** -- root string was dangling after array free. Now deep-copies to `std::string`. (#39)
- **Active subscriptions not cleared on explicit shutdown** -- `shutdown()` clears, involuntary disconnect preserves for reconnect. (#38)
- Mermaid diagram syntax in architecture.md (#30)

### Changed

- Price type per-row variation documented as a known limitation (#37)
- Streaming event-queue capacity monitoring as known limitation

## [4.4.0] - 2026-04-02

v3 historical DataTable parsing (Timestamp cells), DST-aware timezone, gRPC flow control, header aliases for EOD. See v4.5.0 for cumulative details.

## [4.3.0] - 2026-04-02

### Added

- **`start_time` and `end_time` parameters** exposed on all 25 endpoints that support time filtering. Pass `Some("04:00:00")` for pre-market, `Some("20:00:00")` for extended hours, or `None` for RTH defaults (09:30:00-16:00:00). Affects stock history/snapshot/at-time, option history, and index history endpoints.

### Fixed

- Version pins in README and getting-started docs updated to `"4.2"`
- Default venue `"nqb"` (NASDAQ Best) documented

## [4.2.0] - 2026-04-01

### Fixed

- **Interval conversion**: the historical server accepts preset shorthand (`1m`, `5m`, `1h`), not raw milliseconds. `normalize_interval()` now converts `"60000"` -> `"1m"`, `"300000"` -> `"5m"`, etc. Sub-second presets supported: `"100"` -> `"100ms"`, `"500"` -> `"500ms"`. Users can pass either milliseconds or shorthand directly.
- **Default start_time/end_time**: the JVM terminal defaults these to `"09:30:00"` and `"16:00:00"`. Our SDK left them as None, causing `"Invalid time format: Expected hh:mm:ss.SSS"` on trade/quote/greeks endpoints. Now defaults to RTH.
- **extract_text_column**: now handles Number and Price DataTable values. `option_list_strikes` was returning 0 results because strikes come as Number values, not Text.
- **Streaming TLS certificate**: ThetaData's streaming servers have certificates expired since Jan 2024. Skip certificate verification for streaming connections (matching the JVM terminal's behavior).

### Added

`100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`

## [4.1.2] - 2026-04-01

Interval format conversion (later superseded by shorthand normalization in v4.2.0).

## [4.1.1] - 2026-04-01

### Fixed

- PyPI publish workflow: add `skip-existing: true` to prevent duplicate upload failures on tag re-push

## [4.1.0] - 2026-04-01

### Added

- `subscribe_full_open_interest(sec_type)` -- full-stream open interest subscription (was missing, the JVM terminal has it)
- `unsubscribe_full_trades(sec_type)` -- full-stream trade unsubscribe (was missing)
- `unsubscribe_full_open_interest(sec_type)` -- full-stream OI unsubscribe (was missing)
- `reconnect_streaming(handler)` on `Client` -- saves active subscriptions, stops streaming, restarts with new handler, re-subscribes all per-contract and full-type subscriptions automatically
- `active_full_subscriptions()` accessor for full-type subscription tracking
- Internal parity coverage notes tracking which terminal behaviors have a Rust equivalent and which are intentionally out of scope

### Fixed

- DNS hostname resolution in the streaming connection -- `SocketAddr::parse()` replaced with `ToSocketAddrs` to resolve hostnames like `nj-a.thetadata.us` (was silently failing)

### Changed

- Greeks operator precedence (veta, speed, zomma, color, dual_gamma) -- Rust follows the textbook Black-Scholes formulas
- Streaming event-queue capacity monitoring -- documented as known limitation (the event-queue implementation exposes no fill-level API)

## [4.0.0] - 2026-04-01

### Removed

- **`tdbe` crate extracted** -- all data types, codecs, greeks, price, enums, and flags moved to standalone `tdbe` crate with zero networking dependencies. Users must add `tdbe` as a dependency and change imports: `use tdbe::{Price, TradeTick, EodTick}`.
- `thetadatadx` no longer exports `types/`, `codec/`, `greeks.rs`. These modules live in `tdbe`.

### Added

- **`tdbe` crate** -- pure data-format crate. Single dependency (`thiserror`). Contains:
  - 14 hand-written tick structs (no build-time codegen)
  - FIT/FIE nibble codecs
  - Price fixed-point encoding
  - 22 Black-Scholes Greeks + IV solver
  - All enums (SecType, DataType, StreamMsgType, etc.)
  - Error types (Decode, Encode, Conversion, Io)
  - Flags module (trade conditions, price flags, volume types)
  - 6 criterion benchmarks
- **Interactive Query Builder** on docs site -- 13 real-world recipes (GEX, vol surface, option chains, live trade tape, etc.) with symbol autocomplete, dynamic dates, and copy-paste code generation for Rust and Python
- **Inline credential construction** -- all SDK examples now show both `from_file("creds.txt")` and `Credentials::new("email", "password")` patterns
- **JSON serialization benchmark** -- covers streaming events, REST responses, tick-table serialization, and JSON parsing

### Fixed

- Query builder syntax highlighter regex cross-contamination (visible `class="hl-string"` in rendered code)

### Changed

- Tick types in `tdbe` are hand-written (no `include!()`, no `tick_schema.toml` codegen). IDE-navigable, visible in source.
- Magic numbers in `TradeTick` impl replaced with `tdbe::flags::` named constants
- Documentation updated across the affected files for new import paths

## [3.2.2] - 2026-03-30

### Fixed

- Cleaned git history and consolidated documentation commits.
- Added contributor workflow documentation (conventional commits, pre-commit checks).

## [3.2.0] - 2026-03-30

### Added

- **Fully typed returns for all 61 endpoints** - 9 new tick types (`TradeQuoteTick`, `OpenInterestTick`, `MarketValueTick`, `GreeksTick`, `IvTick`, `PriceTick`, `CalendarDay`, `InterestRateTick`, `OptionContract`). All 31 endpoints that previously returned a raw protobuf table now return typed `Vec<T>`. Zero raw protobuf in the public API.
- **TOML-driven codegen** - `tick_schema.toml` is the single source of truth for all tick type definitions and tick-table column schemas. Rust structs and parsers are generated at compile time. Adding a new column = one line in the TOML.
- **Proto maintenance guide** - step-by-step instructions for adding columns, RPCs, or replacing proto files.
- 10 new parse functions on the decode path (including `parse_eod_ticks`)
- All downstream consumers updated: FFI (9 new JSON converters), CLI (9 new renderers), Server (9 new serializers), MCP (9 new serializers), Python SDK (9 new dict converters)
- The `thetadatadx` crate README and FFI README (`ffi/README.md`)
- Python SDK: polars support documented (`pip install thetadatadx[polars]`)

### Fixed

- **Comprehensive documentation sweep** - every doc page, README, notebook, and example file audited against the actual source code. Fixed fabricated homepage examples, wrong C++ include paths (`thetadatadx.hpp` -> `thetadx.hpp`), stale `client.` variable names, missing typed return annotations, wrong Python `all_greeks()` parameter name, version pins (`3.0` -> `3.1`), `for_each_chunk` signature in API reference, and incorrect license in footer.
- **Parameter/response display redesign** - replaced flat markdown tables with vertical card layout across 60 endpoint documentation pages.
- Root README streamlined with navigation table (removed 90-line endpoint listing)
- Notebook 105: fixed event kinds and removed raw payload access pattern
- OpenAPI yaml: fixed license, GitHub URLs, removed DataTable response types

## [3.1.0] - 2026-03-27

### Fixed

- **Go SDK: price encoding was fundamentally wrong** - `priceToFloat()` used a switch-case instead of `value * 10^(price_type - 10)`. Every price returned by the Go SDK was incorrect. Now matches Rust exactly.
- **Python docs: streaming examples used wrong event key** - streaming-event dict access changed from the legacy `type` key to the canonical `kind` key across README and all docs-site pages.
- **`Price::new()` no longer panics in release** - an out-of-range price type is now clamped to the valid range and logged as a warning instead of panicking, so a corrupt frame no longer crashes production.
- **C++ `StreamingClient`: added missing `unsubscribe_quotes()`** - was present in FFI but missing from C++ RAII wrapper.
- **FFI streaming: mutex poison safety** - all 12 `.lock().unwrap()` calls replaced with `.unwrap_or_else(|e| e.into_inner())`. Prevents undefined behavior (panic across `extern "C"`) on mutex poisoning.
- **`Credentials.password` visibility** - changed from `pub` to `pub(crate)` with `password()` accessor. Prevents accidental credential logging by downstream code.
- **WebSocket server: added OPEN_INTEREST + FULL_TRADES dispatch** - previously silently dropped.
- **C++ SDK type parity** - `MarketValueTick` expanded from 3 to 7 fields, `CalendarDay` added `status`, `InterestRateTick` added `ms_of_day`.
- **Python README: removed ghost methods** - `is_authenticated()` and `server_addr()` were listed but did not exist.
- **Root README: stock method count** - "Stock (13)" corrected to "Stock (14)".

## [3.0.0] - 2026-03-27

### Removed

- **Unified `Client` client** — single entry point replacing `DirectClient` + `StreamingClient`.
  Connect once, auth once. Historical available immediately, streaming connects lazily.
- **`DirectClient` removed from crate root re-exports** — still accessible as `thetadatadx::direct::DirectClient` but all methods available via `Client` (Deref)
- **`StreamingClient` removed from crate root re-exports** — use `client.start_streaming(handler)` instead
- **Python SDK**: `DirectClient` and `StreamingClient` classes removed. Use `Client` only.

### Added

- `ThetaDataDx::connect(creds, config)` — one auth, gRPC channel ready, no streaming yet
- `client.start_streaming(handler)` — lazy streaming connection on demand (reads `derive_ohlcvc` from config)
- `client.stop_streaming()` — clean shutdown of streaming, historical stays alive
- `client.is_streaming()` — check if streaming is active
- All 61 historical methods via `Deref<Target = DirectClient>`
- All streaming methods (subscribe/unsubscribe) directly on `Client`
- FFI: `thetadatadx_client_connect()`, `thetadatadx_client_start_streaming()`, `thetadatadx_client_stop_streaming()`
- Server: graceful `stop_streaming()` on shutdown

### Fixed

- Server shutdown now calls `stop_streaming()` before notifying waiters
- Python SDK: removed duplicate method definitions (DirectClient + ThetaDataDx had same methods)

## [2.0.0] - 2026-03-27

### Added

- **`thetadatadx` CLI** (`tools/cli/`) — command-line tool with all 61 endpoints + Greeks + IV.
  Dynamically generated from endpoint registry. `cargo install thetadatadx-cli`
- **MCP Server** (`tools/mcp/`) — Model Context Protocol server giving LLMs instant
  access to 64 tools (61 endpoints + ping + greeks + IV) over JSON-RPC stdio.
  Works with Cursor and every other MCP-compatible client.
- **REST+WS Server** (`tools/server/`) — a local REST + WebSocket server exposing the same surface as the JVM terminal.
  v3 API on port 25503, WebSocket on 25520 with a real streaming bridge, SIMD-accelerated JSON.
- **VitePress documentation site** (`docs-site/`) — 33 pages covering API reference,
  guides, SDK docs, wire protocol internals. Deployed to GitHub Pages.

### Removed

- **StreamEvent split** — `StreamEvent::Quote { .. }` is now `StreamEvent::Data(StreamData::Quote { .. })`.
  Control events are `StreamEvent::Control(StreamControl::*)`. Migration: wrap your match arms.
- **OHLCVC derivation opt-in/out** — `connect()` still derives OHLCVC (default).
  Set `DirectConfig::derive_ohlcvc` to `false` to disable for lower overhead on full trade streams.
- **StreamingClient is fully sync** — no async runtime in the streaming path. Lock-free
  bounded event queue. Callback API: `FnMut(&StreamEvent)`.

### Added

- **Endpoint registry** — auto-generated from proto at build time. Single source of
  truth consumed by CLI, MCP, server. 61 endpoints.
- **Repo reorganization** — the CLI, MCP, and server tools moved under a top-level `tools/` directory
- **SIMD-accelerated JSON** — the CLI, MCP, and server now use a SIMD-accelerated JSON serializer
- **Zero-alloc streaming hot path** — reusable frame buffer, tuple return (no Vec per frame),
  pre-allocated decode buffer, wrapping_add for delta parity
- **Full SDK parity** — all streaming methods (subscribe_full_trades, contract_lookup,
  active_subscriptions, etc.) exposed in Python, Go, C++, FFI
- **Full trade stream docs** — explains the server's quote+trade+OHLC bundle behavior
- **v3 REST API** — server routes match ThetaData's OpenAPI v3 spec (was v2)
- **43 benchmarks** — 10 per-module bench files covering every hot path

### Fixed

- **SIMD FIT removed** — was 2.2x slower than scalar (regression). Pure scalar now.
- **Server trade_greeks routes** — 5 option history trade_greeks endpoints were silently
  dropped due to subcategory mismatch in path generation
- **Audit findings (hot-path)** — hot-path allocations, wrapping_add, BufWriter, find_header
  fallback, DATE marker handling, MCP sanitization, Price dedup
- **Audit findings (server/CLI)** — server security (CORS, shutdown auth), CLI expect(), MCP
  JSON-RPC validation, stale docs
- **Auth response parsing** — subscription fields are integers not strings

### Changed

- Streaming frame read: zero-alloc (reusable buffer)
- Streaming decode: zero-alloc (tuple return, pre-allocated tick buffer)
- Delta: wrapping_add (matches the JVM terminal, no branch)
- Required column validation (skip rows on missing headers, no garbage parse)
- 43 criterion benchmarks across all modules

## [1.2.2] - 2026-03-26

### Added

- **Polars support** in Python SDK: `pip install thetadatadx[polars]`
- `to_polars(ticks)` function converts tick dicts directly to polars DataFrame via `polars.from_dicts()`
- Optional dependency groups: `[pandas]`, `[polars]`, `[all]` for both

### Fixed

- **Multi-platform Python wheels** — now builds for Linux, macOS, and Windows (was Linux-only)
- Source distribution (sdist) included for pip build-from-source fallback
- Auth response parsing: subscription fields are integers (0-3), not strings — fixes connection failures

## [1.2.1] - 2026-03-26

### Fixed

- **Auth: subscription fields are integers** — Nexus API returns `"stockSubscription": 0` (int), not strings. Fixes `"failed to parse Nexus API response"` error on connect.
- **Multi-platform Python wheels** — CI now builds for Linux + macOS + Windows (was Linux x86_64 only). Fixes `"no matching distribution found"` for macOS/Windows users.
- **Source distribution** — sdist included so `pip install` can build from source when no pre-built wheel matches.
- Removed hallucinated "row deduplication" from docs (was never implemented, would have dropped real trades).

## [1.2.0] - 2026-03-26

### Added

- **OHLCVC-from-trade derivation** — `OhlcvcAccumulator` derives OHLCVC bars from trade
  ticks in real time. Only emits `StreamEvent::Data(StreamData::Ohlcvc { .. })` after a
  server-seeded initial bar, matching the JVM terminal's behavior. Subsequent trades
  update open/high/low/close/volume/count incrementally.
- **StreamEvent split: `StreamData` + `StreamControl`** — the monolithic `StreamEvent` enum is now
  a 3-variant wrapper: `Data(StreamData)` for market data (Quote, Trade, OpenInterest, Ohlcvc),
  `Control(StreamControl)` for lifecycle events (LoginSuccess, Disconnected, MarketOpen, etc.),
  and `RawData` for unparsed frames. This enables `match` arms that handle all data without
  touching control flow, and vice versa — an intentional improvement beyond the JVM terminal.
- **Streaming `_stream` endpoint variants** — `stock_history_trade_stream`,
  `stock_history_quote_stream`, `option_history_trade_stream`, `option_history_quote_stream`
  process gRPC response chunks via callback without materializing the full response in memory.
  Ideal for endpoints returning millions of rows.
- **Slab-recycled zstd decompressor** — thread-local `(Decompressor, Vec<u8>)` pair reuses
  the working buffer across calls. The internal slab retains its capacity, avoiding allocator
  pressure for repeated decompressions of similar-sized payloads.
- **148 tests** — new tests for OHLCVC accumulator, StreamEvent split, and
  streaming endpoints.

### Fixed

18 correctness and protocol-conformance fixes from a full audit against the JVM terminal:

**Streaming Protocol**

1. **Streaming contract ID is FIT-decoded** — CONTRACT message contract IDs are now FIT-decoded
   (matching the JVM terminal), not read as raw big-endian i32. Previously produced wrong
   contract-to-symbol mappings.
2. **Delta off-by-one fixed** — `apply_deltas` field indexing corrected; previous
   implementation could shift all fields by one position, corrupting tick data.
3. **Delta state cleared on START/STOP** — per-contract delta accumulators are now reset
   when the server sends START (market open) or STOP (market close), matching the JVM terminal's behavior.
   Previously, stale deltas from the previous session leaked into the next session's ticks.
4. **ROW_SEP unconditional reset** — ROW_SEP (0xC) now unconditionally resets the field
   index to SPACING (5), matching the JVM terminal's FIT reader. Previously this was conditional,
   which could produce misaligned fields.
5. **Credential sign-extension** — credential length fields are now read as an unsigned
   16-bit integer, matching the JVM terminal. Previously, passwords longer than 127 bytes
   could produce a negative length.
6. **Flush only on PING** — the streaming write buffer is now flushed only when sending PING
   messages, matching the JVM terminal's batching behavior. Previously, every write triggered a flush,
   increasing syscall overhead and wire chattiness.
7. **Ping 2000ms initial delay** — the first PING is now delayed by 2000ms after
   authentication, matching the JVM terminal's 2000 ms pause before entering the
   ping loop. Previously, pings started immediately.

**Historical / gRPC Protocol**

8. **`null_value` added to DataValue proto** — the `DataValue` oneof now includes a
   `null_value` variant (bool), matching the server's proto definition. Previously,
   null cells were silently dropped during deserialization.
9. **`"client": "terminal"` in query_parameters** — all gRPC requests now include
   `"client": "terminal"` in the `query_parameters` map, matching the JVM terminal.
   Previously this field was omitted.
10. **Dynamic concurrency from subscription tier** — the historical request-concurrency
    knob (since removed) is now derived from the `AuthUser` response's subscription tier
    (`2^tier`), matching the JVM terminal's concurrency model. The config field still allows manual override.
11. **Unknown compression returns error** — `decompress_response` now returns
    `Error::Decompress` for unrecognized compression algorithms instead of silently
    treating the data as uncompressed.
12. **Empty stream returns empty DataTable** — `collect_stream` now returns an empty
    `DataTable` (with headers, zero rows) when the gRPC stream contains no data chunks,
    instead of returning `Error::NoData`. Callers can check `.data_table.is_empty()`.
13. **gRPC flow control window** — the gRPC channel now configures
    `initial_connection_window_size` and `initial_stream_window_size` to match the JVM
    terminal's flow-control settings, preventing throughput bottlenecks on large responses.

**Auth / User Model**

14. **Per-asset subscription fields in AuthUser** — `AuthUser` now includes `stock_tier`,
    `option_tier`, `index_tier`, and `futures_tier` fields from the Nexus auth response,
    enabling per-asset-class concurrency and permission checks.
15. **Auth 401/404 handling** — Nexus HTTP responses with status 401 (Unauthorized) or
    404 (Not Found) are now treated as invalid credentials, matching the JVM terminal's
    behavior. Previously these could surface as generic HTTP errors.

**Observability**

16. **Column lookup warns instead of silent fallback** — `extract_*_column` functions now
    emit a `warn!` log when a requested column header is not found in the DataTable,
    instead of silently returning a vec of `None`s. This makes schema mismatches
    immediately visible in logs.

**Greeks**

17. **6 Greeks formula fixes** — operator precedence corrections across 6 Greek functions
    to match the JVM terminal's evaluation order. All formulas now produce bit-identical results to
    the JVM terminal for the same inputs.
18. **`Vera` DataType code (166)** — second-order Greek `Vera` added to the `DataType` enum,
    completing the full set of second-order Greeks (vanna, charm, vomma, veta, vera, sopdk).

### Security

- **Contract wire format fix** — contract binary serialization now matches the JVM terminal
  exactly. Previous versions could produce incorrect wire bytes for option contracts, causing
  subscription failures or wrong contract assignments. This was a **protocol-level bug**;
  upgrading to 1.2.x is strongly recommended.

### Changed

- **Slab-recycled zstd** — thread-local decompressor reuses its working buffer, eliminating
  per-chunk allocation overhead.
- **Streaming `_stream` endpoints** — process gRPC responses chunk-by-chunk without
  materializing the full DataTable in memory.

See `TODO.md` (as of the 1.2.0 release) for the production readiness checklist and performance roadmap.

## [1.1.1] - 2026-03-26

### Added

- **Historical request-concurrency semaphore** on DirectClient (the knob since removed) — configurable limit on in-flight
  gRPC requests (default 2), exposed via the historical request-concurrency config field
- **Streaming `for_each_chunk` method** on DirectClient — process gRPC response chunks via
  callback without materializing the full response in memory
- **Pre-allocation hint in `collect_stream`** — uses `original_size` from `ResponseData` to
  pre-allocate the decompression buffer, reducing reallocations
- **Horner-form `norm_cdf`** — replaced Abramowitz & Stegun polynomial approximation with
  Zelen & Severo Horner-form evaluation (~1e-7 accuracy, fewer multiplications)
- **Python SDK: streaming** — `StreamingClient` class with `subscribe()`, `next_event()`,
  and `shutdown()` methods for real-time market data in Python
- **Python SDK: pandas DataFrame conversion** — `to_dataframe()` function plus per-endpoint
  DataFrame convenience methods on DirectClient (later superseded in #379 by the unified
  `to_dataframe(ticks)` Arrow-backed path); install with `pip install thetadatadx[pandas]`
- **FFI crate: streaming support** — 7 new `extern "C"` functions for the streaming lifecycle
  (`fpss_connect`, `fpss_subscribe_quotes`, `fpss_subscribe_trades`,
  `fpss_subscribe_open_interest`, `fpss_next_event`, `fpss_shutdown`, `fpss_free_event`)
- **Go SDK: streaming** — `StreamingClient` Go struct wrapping the FFI streaming functions
- **C++ SDK: streaming** — `StreamingClient` C++ RAII class wrapping the FFI streaming functions

### Fixed

- Version bump for crates.io/PyPI publish (v1.1.0 tag was re-pushed during history restore)

### Changed

- All TODO performance items now complete: streaming iterator (`for_each_chunk`),
  optimized `norm_cdf` (Horner-form), concurrent request semaphore (the historical request-concurrency knob, since removed)

## [1.1.0] - 2026-03-26

### Added

- **All 61 endpoints** via declarative macro (was 19 hand-written) — covers every
  v3 gRPC RPC: stock, option, index, interest rate, calendar
- **All 61 endpoints in every SDK** — Python, Go, C++, C FFI all match Rust core
- **Zero-allocation streaming path** — fully sync I/O thread + lock-free bounded event queue,
  no async runtime in the streaming hot path
- **Cache-line aligned tick types** — 64-byte cache-line alignment on TradeTick, QuoteTick, OhlcTick, EodTick
- **Cached QueryInfo template** — no per-request String allocation
- **Precomputed DataTable column indices** — O(1) per row, not O(headers)
- **pow10 lookup tables** for Price comparison and conversion
- **`#[inline]`** on all hot-path functions (FIT decode, Price ops, tick accessors)
- **Reusable thread-local zstd decompressor** — no fresh allocation per chunk
- **Criterion benchmarks** — fit_decode, price_to_f64, price_compare, all_greeks, fie_encode
- **AdaptiveWaitStrategy** — 3-phase spin/yield/hint tuned for ~100us streaming tick intervals

### Changed

- Authenticated against real Nexus API (session established)
- Retrieved 25,341 stock symbols from the historical service
- Retrieved 42 AAPL EOD ticks (Jan-Mar 2024) with correct OHLCV data
- Retrieved 2,010 SPY option expirations
- Retrieved 13,160 index symbols
- Calendar endpoint returned valid data
- `client_type = "rust-thetadatadx"` accepted by server

## [1.0.1] - 2026-03-26

### Changed

- Renamed crate from `thetadx` to `thetadatadx` (crates.io + PyPI)
- Renamed repository from `thetadx` to `Client`
- Changed license metadata
- Updated top-level README
- README updated with GitHub callouts (NOTE, TIP, IMPORTANT, WARNING, CAUTION)
- Fixed PyPI package description (was empty — added readme field to pyproject.toml)

## [1.0.0] - 2026-03-26

### Added

- **DirectClient** for historical gRPC — all 60 gRPC RPCs exposed as 61 typed endpoint methods
  (stock/option/index/rate/calendar: list, history, snapshot, at-time, greeks) via
  declarative `define_endpoint!` macro
- **StreamingClient** for streaming — real-time quotes, trades, open interest, OHLC
  via TLS/TCP with heartbeat and manual reconnection
- **Auth module** — Nexus API authentication (email/password → session UUID)
- **FIT/FIE codec** — nibble-based tick compression/decompression at JVM terminal parity
- **Greeks calculator** — full Black-Scholes: 22 Greeks + IV bisection solver with
  precomputed shared intermediates and edge-case guards (t=0, v=0)
- **All tick types** — TradeTick, QuoteTick, OhlcTick, EodTick, OpenInterestTick,
  SnapshotTradeTick, TradeQuoteTick with fixed-point Price encoding
- **91 DataType enum codes** — quotes, trades, OHLC, all Greek orders, dividends,
  splits, fundamentals
- **Proto definitions** — the full v3 protocol surface
- **Runtime configuration** — `DirectConfig` with all JVM-equivalent tuning knobs
- `contract_lookup(id)` on `StreamingClient` for single-entry hot-path lookup
- `StreamEvent::Error` variant for surfacing protocol parse failures
- Date parameter validation on all `DirectClient` methods
- `async-zstd` feature flag for optional streaming decompression
- **Python SDK** (PyO3/maturin) — wraps the Rust crate, not a reimplementation
- **Go SDK** — CGo FFI bindings over the C ABI layer
- **C++ SDK** — RAII C++ wrapper over the C header
- **C FFI crate** (`thetadatadx-ffi`) — stable `extern "C"` ABI for all SDKs
- **Documentation** — architecture (Mermaid), API reference, JVM terminal parity checklist
- **CI/CD** — GitHub Actions (fmt, clippy, test, FFI build, crates.io publish, PyPI publish, GitHub Release)
- **Project infrastructure** — CHANGELOG, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT,
  clippy.toml, cliff.toml, rust-toolchain.toml, LICENSE

### Security

- Credential `Debug` redaction — passwords never appear in debug output
- `AuthRequest` does not derive `Debug` (prevents password in error traces)
- Session UUID redaction — bearer tokens logged at `debug!` level only, first 8 chars
- `assert!` on streaming frame size limits — enforced in release builds
- Unified TLS via rustls for all connections (historical gRPC + streaming TCP + Nexus HTTP)
- Timeouts on all network operations (auth 10s/5s, gRPC keepalive, streaming connect, streaming read 10s)
- 7 credential/account errors treated as permanent disconnect (no futile reconnect loops)
- Contract root length validated before wire serialization
- FIT decoder uses i64 accumulator with i32 saturation (no silent overflow)
- Price type range enforced with `assert!` in release builds

[Unreleased]: https://github.com/userFRM/ThetaDataDx/compare/v13.0.0-rc.5...HEAD
[13.0.0-rc.5]: https://github.com/userFRM/ThetaDataDx/compare/v13.0.0-rc.4...v13.0.0-rc.5
[13.0.0-rc.4]: https://github.com/userFRM/ThetaDataDx/compare/v13.0.0-rc.3...v13.0.0-rc.4
[13.0.0-rc.3]: https://github.com/userFRM/ThetaDataDx/compare/v13.0.0-rc.2...v13.0.0-rc.3
[13.0.0-rc.2]: https://github.com/userFRM/ThetaDataDx/compare/v13.0.0-rc.1...v13.0.0-rc.2
[13.0.0-rc.1]: https://github.com/userFRM/ThetaDataDx/compare/v12.0.0...v13.0.0-rc.1
[12.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v11.0.1...v12.0.0
[11.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v11.0.0...v11.0.1
[11.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v10.0.0...v11.0.0
[10.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v9.1.0...v10.0.0
[9.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v9.0.1...v9.1.0
[9.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v9.0.0...v9.0.1
[9.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.37...v9.0.0
[8.0.37]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.36...v8.0.37
[8.0.36]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.35...v8.0.36
[8.0.35]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.33...v8.0.35
[8.0.33]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.32...v8.0.33
[8.0.32]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.31...v8.0.32
[8.0.31]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.30...v8.0.31
[8.0.30]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.29...v8.0.30
[8.0.29]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.28...v8.0.29
[8.0.28]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.27...v8.0.28
[8.0.27]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.26...v8.0.27
[8.0.26]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.25...v8.0.26
[8.0.25]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.24...v8.0.25
[8.0.24]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.23...v8.0.24
[8.0.23]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.22...v8.0.23
[8.0.22]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.21...v8.0.22
[8.0.21]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.20...v8.0.21
[8.0.20]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.19...v8.0.20
[8.0.19]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.18...v8.0.19
[8.0.18]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.17...v8.0.18
[8.0.17]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.16...v8.0.17
[8.0.16]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.15...v8.0.16
[8.0.15]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.14...v8.0.15
[8.0.14]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.13...v8.0.14
[8.0.13]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.12...v8.0.13
[8.0.12]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.11...v8.0.12
[8.0.11]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.10...v8.0.11
[8.0.10]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.9...v8.0.10
[8.0.9]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.8...v8.0.9
[8.0.8]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.7...v8.0.8
[8.0.7]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.6...v8.0.7
[8.0.6]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.5...v8.0.6
[8.0.5]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.4...v8.0.5
[8.0.4]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.3...v8.0.4
[8.0.3]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.2...v8.0.3
[8.0.2]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.1...v8.0.2
[8.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.0...v8.0.1
[8.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.3.1...v8.0.0
[7.3.1]: https://github.com/userFRM/ThetaDataDx/compare/v7.3.0...v7.3.1
[7.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.2.1...v7.3.0
[7.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v7.2.0...v7.2.1
[7.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.1.0...v7.2.0
[7.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.0.0...v7.1.0
[7.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v6.0.1...v7.0.0
[6.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.4.0...v6.0.1
[5.4.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.3.1...v5.4.0
[5.3.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.3.0...v5.3.1
[5.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.2.1...v5.3.0
[5.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.2.0...v5.2.1
[5.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.1.1...v5.2.0
[5.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.1.0...v5.1.1
[5.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.2...v5.1.0
[5.0.2]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.1...v5.0.2
[5.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.0...v5.0.1
[5.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.5.0...v5.0.0
[4.5.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.4.0...v4.5.0
[4.4.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.3.0...v4.4.0
[4.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.2...v4.3.0
[4.1.2]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.1...v4.1.2
[4.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.0...v4.1.1
[4.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v3.2.2...v4.1.0
[3.2.2]: https://github.com/userFRM/ThetaDataDx/compare/v3.2.0...v3.2.2
[3.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v3.1.0...v3.2.0
[3.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v2.0.0...v3.1.0
[2.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.2...v2.0.0
[1.2.2]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/userFRM/ThetaDataDx/releases/tag/v1.0.0
