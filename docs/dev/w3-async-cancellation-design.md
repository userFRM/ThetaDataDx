# W3 — First-class async cancellation (design)

## Problem

The four SDK validators (`scripts/validate_python.py`, `sdks/go/validate.go`,
`sdks/cpp/examples/validate.cpp`, `scripts/validate_cli.py`) enforce a 60-second
per-cell budget against an in-flight gRPC call. They do this from the *outside*
because the SDK has no way to cancel a call that is mid-flight on a shared
client handle.

The current workarounds are unsafe in three of the four languages:

- **Python**: daemon `threading.Thread` + `queue.Queue.get(timeout=60)`. The
  daemon thread keeps the PyO3 / Rust gRPC call alive past the timeout.
  The validator sets `aborted=True` and SKIPs every subsequent cell because
  another call on the same FFI handle would race the leaked worker.
- **C++**: `std::packaged_task` + detached thread + `wait_for(60s)` + `std::_Exit`
  on any timeout. The process bails immediately — every remaining cell is lost.
- **Go**: goroutine + `time.After(60s)` + `errCellTimeout` sentinel +
  `hadTimeout=true` + abandons remaining cells.
- **CLI**: subprocess timeout. Acceptable because the OS reaps the process
  cleanly; no in-process state to corrupt.

Net result: any one stuck cell loses all subsequent matrix coverage in three
of the four SDKs.

## Goal

Move cancellation into the SDK's async path so the validator's 60-second
deadline actually cancels the in-flight gRPC call, leaving the client handle
intact for subsequent calls. Same fix benefits every end-user, not just the
validator.

## Cancellation primitive

`tokio::time::timeout(d, fut)` cancels by *dropping* `fut` when the deadline
fires. In our endpoint macros, the request future captures:

- `let _permit = self.request_semaphore.acquire().await?;`
- `let stream = self.stub().<grpc>(request).await?.into_inner();`
- `self.collect_stream(stream).await`

Dropping the future drops `_permit` (release the inflight slot), drops `stream`
(tonic propagates RST_STREAM into the underlying `Channel`, freeing the
connection-level `connection_semaphore`), and drops every local. The
`DirectClient` is untouched. Subsequent calls on the same handle work fine.

This is the standard tokio cancellation contract — no special tonic plumbing
required, but verified by the new unit test `direct::deadline_cancels_call`.

## API surface

Every endpoint becomes a builder so `with_deadline(Duration)` is a uniform
extension point. List endpoints already had no optional setters; they pick
up `IntoFuture` so the existing `client.x().await` form continues to work.

Rust:
```rust
let symbols = client.stock_list_symbols()                           // unchanged
    .await?;
let symbols = client.stock_list_symbols()
    .with_deadline(Duration::from_secs(60))                          // NEW
    .await?;
let ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
    .with_deadline(Duration::from_secs(60))                          // NEW
    .await?;
```

A new `Error::Timeout { duration_ms }` variant carries the budget. Existing
`Error` variants and method signatures are unchanged.

FFI: `TdxEndpointRequestOptions` gains `timeout_ms: u64` + `has_timeout_ms: i32`.
Every endpoint that does not yet have a `_with_options` variant gets one
generated, so callers can always pass a deadline.

Python: every generated method gains a `timeout_ms: Optional[int] = None`
kwarg. `Error::Timeout` maps to Python's stdlib `builtins.TimeoutError`
(via PyO3's `PyTimeoutError`); inherits from `OSError` in 3.3+ so
callers can write `except TimeoutError` specifically or fall back to
`except Exception`. `timeout_ms=0` is normalized to "no deadline" at
the Rust setter — same convention as the Go and C++ surfaces.

Go: a new `WithTimeoutMs(uint64)` `EndpointOption` sets the cross-cutting
timeout. Existing builder-style methods accept it via the existing
`EndpointOption` slice. Endpoints without builder params get a new
`<name>WithOptions(...)` variant.

C++: `EndpointRequestOptions::timeout_ms` field, set via the existing
fluent setter pattern.

CLI: `--timeout` flag on every endpoint subcommand (forwards to
`EndpointArgs::with_timeout_ms`).

## Plumbing

```
SDK call surface
    -> EndpointArgs::with_timeout_ms(ms)                (registry path)
    -> invoke_endpoint(name, args)
       -> match arm builder.with_deadline(d).await
          -> macros wrap final gRPC + collect_stream in tokio::time::timeout
             on Elapsed: return Err(Error::Timeout { duration_ms: d.as_millis() })
```

Direct (non-registry) callers chain `.with_deadline(d)` on the builder.

## Out of scope

- **Streaming endpoints (FPSS)**. They are long-lived subscriptions, not
  unary requests; the matrix doesn't cover them; the daemon-thread pattern
  there would belong to a different design (cancellation-on-unsubscribe).
- **Per-call retry on Timeout**. A timed-out call returns the error; the
  caller decides whether to retry. The Java terminal does the same.
- **Connection-level deadlines** (e.g. `connect_timeout`). Already covered
  by `DirectConfig`; this PR is per-request deadlines.

## Validator rewrites

All four validator generators (`endpoints/render/{python,go,cpp,cli}_validate.rs`)
drop the daemon-thread / packaged_task / goroutine timeout shim and the
`aborted` / `hadTimeout` / `_Exit` cleanup. Each cell becomes:

```python
client.<endpoint>(*args, timeout_ms=60_000)
```

On `Error::Timeout`: classify FAIL `timeout after 60s` (same string as today,
no behaviour change for `scripts/validate_agreement.py`). Continue to next
cell on the same client handle.

The CLI validator already used `subprocess.run(timeout=60)`; it switches
to `--timeout 60000` so the SDK does the cancelling and the subprocess
exits cleanly with a `Timeout` error rather than being SIGKILLed.

## Acceptance

- `cargo fmt --all -- --check && cargo clippy --workspace -- -D warnings && cargo test --workspace` green
- New unit test `crates/thetadatadx/src/direct.rs::deadline_cancels_call` asserts
  - `with_deadline(1ms)` on a stalled grpc returns `Error::Timeout`
  - subsequent call on the same `DirectClient` succeeds
  - the `request_semaphore` permit is released (concurrency capacity
    measurable on a `Semaphore::available_permits()` snapshot)
- Live STANDARD-tier matrix run completes — no leaked threads / goroutines /
  detached `std::thread`, no `_Exit`, no `aborted`/`hadTimeout` SKIP cascade
- `scripts/validate_agreement.py` parity preserved across all 4 SDKs
