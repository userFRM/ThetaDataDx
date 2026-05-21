# Migrating from v9 to v10

ThetaDataDx v10 is a major version bump. SDK callers (Python /
TypeScript / C++) see NO public API breakage from the v10 transport
rewrite; direct consumers of the Rust core (`thetadatadx` crate)
need to update a handful of type names. The rest of this guide walks
through each change.

## TL;DR

| Surface | Change | Migration |
|---|---|---|
| Cargo pin | `thetadatadx = "9"` → `"10"` | Bump the `[dependencies]` line in `Cargo.toml` |
| Python pin | `thetadatadx>=9.1.0,<10` → `>=10.0.0,<11` | Bump `pyproject.toml` / `requirements.txt` |
| npm pin | `"thetadatadx": "^9.1.0"` → `"^10.0.0"` | Bump `package.json`; the prebuilt napi binding follows |
| C++ pin | `v9.1.0` tag → `v10.0.0` tag | Re-fetch the `libthetadatadx_ffi` artifact |
| `Error::Transport` (Rust core) | `Transport(tonic::transport::Error)` → `Transport { kind, message }` | See [In-house gRPC transport](#in-house-grpc-transport) below |
| Event payload field name | `event.contract: Contract` → `event.contract: ContractRef` | See [ContractRef rename](#contractref-rename) below |
| Python wheels | abi3 only | abi3 + free-threaded (`cp313t` / `cp314t`) — see [Free-threaded wheels](#free-threaded-wheels) below |
| Python streaming | sync callback / sync iter | sync callback + sync iter + **asyncio** (`streaming_async()`) — see [streaming_async](#streaming_async-asyncio-native) below |

## In-house gRPC transport

The MDDS server-streaming path no longer uses `tonic`. The v10 Rust
core drives `h2` directly through the new `thetadatadx::grpc::*`
module: prost encode → length-prefix frame → HTTP/2 DATA → response
stream → trailers parse, with no tower stack, no boxed bodies, no
`async-trait` dyn dispatch.

The SDK bindings (Python / TypeScript / C++) consume the Rust core
through the C ABI / PyO3 boundary and see no API change. Rust core
consumers see one source-level break:

```rust
// v9
match err {
    Error::Transport(tonic_err) => { /* ... */ }
}

// v10
match err {
    Error::Transport { kind, message } => {
        match kind {
            TransportErrorKind::Tcp => { /* ... */ }
            TransportErrorKind::Tls => { /* ... */ }
            TransportErrorKind::H2Stream => { /* ... */ }
            TransportErrorKind::ConnectionClosed => { /* ... */ }
            // ... full taxonomy in `thetadatadx::error::TransportErrorKind`
            _ => { /* non_exhaustive — match wildcard */ }
        }
    }
}
```

`TransportErrorKind` carries the typed fault category so retry
classifiers can dispatch on the concrete kind without parsing
`Display`. The Display shape stays
`transport error (<kind>): <message>` for legacy string-keyed
consumers — those will keep working.

The decoder pool also lands in v10:
`MddsConfig::decoder_threads` and `MddsConfig::decoder_ring_size`
control a dedicated pool that runs zstd decompress + protobuf
decode off the tokio reactor. `decoder_threads = 0` auto-sizes to
`min(channels, available_parallelism / 2)`; `decoder_ring_size`
must be a power of two `>= 64`.

## ContractRef rename

The FPSS event payload field that exposes the resolved contract was
previously typed `Contract` on every binding, colliding with the
fluent `Contract` builder used in `subscribe()` inputs. v10 renames
the event payload type to `ContractRef`:

```python
# v9
for event in iter:
    match event:
        case Trade(contract=c):
            # `c` was a `Contract` value with the same name as the
            # fluent builder. Type-checking was ambiguous and import
            # ordering occasionally surfaced the wrong class.

# v10
for event in iter:
    match event:
        case Trade(contract=c):
            # `c` is now a `ContractRef` — a read-only event payload
            # accessor with `.symbol`, `.sec_type`, `.expiration`,
            # `.right`, `.strike_dollars`, `.strike`. The fluent
            # `Contract` builder (used as `Contract.stock(...)`)
            # stays exactly where it was.
            print(c.symbol, c.strike_dollars)
```

The TypeScript binding ships the class as `ContractRef` (the napi-rs
emitter name) with a published `export const Contract: typeof
ContractRef` alias so existing `Contract.stock(...)` user code
continues to type-check. C++ exposes the event payload through the
existing `TdxContract` C ABI struct; the surface stays unchanged.

## Free-threaded wheels

v10 publishes free-threaded (PEP 703) Python wheels alongside the
abi3 wheel. `pip` picks the matching wheel automatically:

| Interpreter | Wheel | GIL state |
|---|---|---|
| `python3.9` – `python3.12` (stock) | `cp39-abi3-*` | GIL enabled |
| `python3.13t` (free-threaded) | `cp313-cp313t-*` | GIL disabled |
| `python3.14t` (free-threaded) | `cp314-cp314t-*` | GIL disabled |

The extension carries `gil_used = false` on `#[pymodule]` so the GIL
stays disabled after `import thetadatadx`. Every `block_on(...)`
call site on the unified, FPSS, and MDDS Python pyclasses releases
the GIL via `py.detach` before driving the tokio runtime; CPU-bound
Python threads run truly in parallel with the gRPC dispatcher under
contention.

A parallel-throughput CI gate asserts `< 1.8x` overhead under
contention on the free-threaded matrix entries (matching the
`test_no_gil.py::test_parallel_throughput_bench_runs` pytest
assertion). A regression that re-acquires the GIL on the hot path
trips both the gate and the test.

## `streaming_async()` (asyncio-native)

v10 adds an asyncio-native streaming surface alongside the sync
callback / sync iterator paths:

```python
import asyncio
from thetadatadx import Config, Contract, Credentials, ThetaDataDxClient

async def main():
    creds = Credentials.from_file("creds.txt")
    client = ThetaDataDxClient(creds, Config.production())

    async with client.streaming_async() as session:
        await session.subscribe(Contract.stock("QQQ").quote())
        async for batch in session:
            for event in batch:
                handle(event)

asyncio.run(main())
```

The session bridges the Disruptor consumer thread to the asyncio
event loop via a self-pipe wake FD: zero polling cost during quiet
periods, one OS wake per coalesced batch. The matching surface on
the standalone `FpssClient` (`fpss_client.streaming_async()`) opens
no MDDS / Nexus surface — useful for asyncio apps coexisting with a
parallel Java MDDS process.

## Standalone `FpssClient` / `MddsClient` Python pyclasses

v10 ships standalone Python pyclasses for the FPSS-only and
MDDS-only surfaces, mirroring the C ABI `tdx_fpss_*` / `tdx_client_*`
split and the C++ `tdx::FpssClient` / `tdx::Client` shape:

```python
from thetadatadx import FpssClient, MddsClient, Credentials, Config

# Real-time stream only — no MDDS gRPC channel, no Nexus auth.
fpss = FpssClient(Credentials.from_file("creds.txt"), Config.production())

# Historical / FLATFILES only — no FPSS TLS slot. Every FPSS-touching
# method raises `AttributeError`.
mdds = MddsClient(Credentials.from_file("creds.txt"), Config.production())
```

The bundled `ThetaDataDxClient` keeps its current behaviour — the
new classes are purely additive.

## CI invariant gates

v10 lands a 12-gate CI invariant suite (`scripts/check_*.py` +
matching workflow jobs). The gates cover cross-binding parity,
C ABI completeness against the compiled .so symbol table, wire
schema drift, version sync (Cargo / `package.json` / CMake / docs
pins), wheel + npm tarball content, stubtest `.pyi` ↔ runtime,
fresh-install venv smoke, doc-example harness, cargo-semver-checks
(anchored at `v10.0.0`), bench regression (25% threshold against
the GH-runner baseline), and the nogil throughput overhead gate.

If you fork or vendor the repository, the gates run on every PR by
default. Refresh the bench baseline by running the bench suite once
on a green main and committing the new `criterion.json` snapshot in
its own PR.

## Notes

- The `inhouse-grpc` feature flag is gone — the in-house transport
  is the only path on v10.
- `MddsClient::stub` was removed; internal call sites now reach the
  generated stubs through `proto::beta_theta_terminal::*` directly.
- `GrpcStatusKind::from_code()` renamed to
  `GrpcStatusKind::from_u32()` to match the wire type. The enum
  `repr` is now `u32` (was `i32`).
- `StatusParseError::MessageNotUtf8` was removed — malformed
  `grpc-message` no longer fails the trailers parse. Exhaustive
  matches need to drop the variant.

Direct questions: file an issue at
[github.com/userFRM/ThetaDataDx/issues](https://github.com/userFRM/ThetaDataDx/issues).
