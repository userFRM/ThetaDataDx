# v7.0.0 Scorched Earth — Breaking Refactor Design

**Status:** Approved, pre-implementation
**Date:** 2026-04-10
**Target:** `v7.0.0` release
**Branch:** `release/v7.0`

## Problem

The ThetaDataDx repo has accumulated significant cruft across six months of migration cycles, dual generation models, hand-written duplicates of generated code, sentinel-based FFI patterns, and two failing CI checks shipped to main. The user has explicitly requested a "fully breaking refactor" that "destroys, obliterates any legacy and deprecated" code in service of a "new and bullet-proof" v7.0.0 surface.

This spec captures the kill list, the target architecture, the phasing strategy, and the verification gates for that refactor.

## Goals

1. **No dead code.** Every line in the repo is load-bearing in v7.0.0.
2. **Single source of truth.** `endpoint_surface.toml` drives every SDK surface, generated into source at build time.
3. **No sentinel patterns in the public API.** Optional values are explicit.
4. **Breaking is a feature.** v7.0.0 wire format is not backwards-compatible with v6 client code. Deprecations die.
5. **CI gates hold.** Every commit passes fmt, clippy, test, docs consistency, and FFI build before the next phase starts.

## Non-goals

- **FPSS wire protocol fixes** — Issue #192 is waiting on ThetaData server fix. Out of scope.
- **Proto definition changes** — `external.proto` is ThetaData's contract, not ours.
- **`tdbe` encoding logic** — Tested, correct, not legacy.
- **Semantic types / ergonomic façade** — `docs/public-api-redesign.md` is explicitly being killed as unimplemented aspiration. Any future façade is a separate v7.x or v8 project.

---

## The Kill List

### Phase 0 — Cleanup (non-breaking, quick wins)

**L1. Delete 1,134 commented-out Python method lines**
- File: `sdks/python/src/lib.rs:1135-2269`
- The entire block is a single `/* ... */` comment replaced by `include!("generated_historical_methods.rs")` at line 2334.
- Action: `git rm` the comment block.

**L2. Delete `docs/public-api-redesign.md`**
- 339 lines of aspirational ergonomic façade plan.
- Explicitly confirmed unimplemented.
- Action: delete file + any references in navigation/indexes.

**L3. Fix `cargo fmt` failure in `build_support/endpoints.rs`**
- Codex shipped unformatted code across ~16 hunks.
- Action: `cargo fmt --all` and commit.

**L4. Fix self-broken docs consistency checker**
- `scripts/check_docs_consistency.py` looks for `type EndpointRequestOptions struct` in `sdks/go/client.go`, but the struct moved to `sdks/go/generated_endpoint_options.go` in the same commit.
- Action: update the regex to search the correct file (or both).

**L14. Delete `migration-from-rest-ws.md`**
- 304 lines of v5/v6→SDK migration story.
- v7 docs start fresh. No v5/v6 users being supported.
- Action: `git rm` file + nav references.

### Phase 1 — Rust/SDK prune (breaking at the SDK level)

**L5. Delete hand-written Python methods duplicating generated**
- File: `sdks/python/src/lib.rs`
- Audit every `#[pymethods]` function. Delete any that has an equivalent in `generated_historical_methods.rs`.
- Keep: FPSS streaming methods, `all_greeks`/`implied_volatility` (offline), session/config methods, type converters (`*_to_dict`), `ThetaDataDx::new`/`from_file`/etc.
- Target: `lib.rs` shrinks from 2399 lines toward ~800 lines of glue.

**L7. Delete hand-written Go methods duplicating generated**
- File: `sdks/go/client.go` (currently 1773 lines)
- Delete every method on `*Client` that exists in `generated_historical.go`.
- Keep: FPSS streaming, session/auth, error types, FFI boundary helpers, init-time size assertions.
- Target: `client.go` shrinks toward ~600 lines.

**L8. Delete hand-written C++ methods duplicating generated**
- File: `sdks/cpp/src/thetadx.cpp` (currently 543 lines)
- Same treatment. Keep only FPSS + session + FFI glue.
- Target: `thetadx.cpp` shrinks toward ~200 lines.

**L13. Delete `SnapshotTradeTick` for good**
- The endpoint `stock_snapshot_trade` returns `TdxTradeTickArray`, NOT `TdxSnapshotTradeTickArray`.
- `tdbe::SnapshotTradeTick` is unused at the public surface.
- Action: delete from `crates/tdbe/src/types/tick.rs`, `crates/thetadatadx/tick_schema.toml`, `crates/thetadatadx/src/decode.rs`, all SDK headers, all language bindings. Close issue #227 as resolved.

### Phase 2 — FFI rewrite (breaking the FFI contract)

**L9. Replace sentinel-based FFI options with explicit flags**
- `TdxEndpointRequestOptions` currently uses NaN/-1/null as "unset" markers.
- New pattern: each optional field gets a companion `has_*: bool`.
- Example:
  ```c
  typedef struct {
    int32_t max_dte;        bool has_max_dte;
    int32_t strike_range;   bool has_strike_range;
    const char* venue;      /* NULL still valid for strings */
    const char* min_time;
    /* ... */
  } TdxEndpointRequestOptions;
  ```
- Rust side: generator emits `apply_endpoint_request_options()` that checks the `has_*` flag instead of comparing against NaN/-1.
- All 46 `tdx_*_with_options()` functions get regenerated.
- Go/C++/Python FFI wrappers regenerated to set the flags explicitly.

### Phase 3 — Generation unification (kill the dual model)

**L11. Kill the dual-source generation model**
- **What "kill checked-in" means here:** The generated Go/C++ files will still live in git (they have to, because `cargo build` of the Rust crate doesn't own those source trees for Go/C++ tooling). What dies is the **manual** `cargo run --bin generate_sdk_surfaces` step. After v7, `build.rs` refreshes those files on every `cargo build`, a `@generated DO NOT EDIT` header is enforced, and CI fails if `git diff` shows uncommitted changes after build. Drift becomes structurally impossible.
- Action: `build.rs` regenerates SDK surfaces directly into their source trees every build:
  - `crates/thetadatadx/src/endpoint.rs` pulls generated dispatch from `OUT_DIR` (unchanged)
  - `sdks/python/src/generated_historical_methods.rs` → written to `OUT_DIR` and included via `include!()` — `include!` path changes from source-relative to `OUT_DIR`-relative
  - `sdks/go/generated_historical.go` → written directly to source tree on each build (with a header comment `// @generated DO NOT EDIT`)
  - `sdks/cpp/include/generated_historical.{hpp,cpp}.inc` → same pattern
  - `ffi/src/generated_endpoint_with_options.rs` → same
- Delete `crates/thetadatadx/src/bin/generate_sdk_surfaces.rs` (binary) — `build.rs` handles it.
- Delete checked-in versions of the generated files. They become build artifacts in `.gitignore` or `// @generated` headers in `build.rs`-written files.

**Decision point:** For non-Rust SDK files (Go/C++), we cannot use `OUT_DIR` because `cargo build` of the Rust crate doesn't own the Go/C++ source tree. Options:
  - **Option A:** `build.rs` writes Go/C++ generated files directly to `sdks/go/` and `sdks/cpp/include/` with `@generated` headers. CI check: `git diff --exit-code` after `cargo build` — if the generator changes, the commit must include the updated generated file.
  - **Option B:** Require developers to run `cargo run --bin generate_sdk_surfaces` before committing. Reinstates the dual model we're trying to kill.

  **Choice: Option A.** `build.rs` writes directly. `.gitattributes` marks generated files with `linguist-generated=true`. CI runs `cargo build && git diff --exit-code -- 'sdks/**/generated_*'` to catch drift.

### Phase 4 — Streaming generation (kill hand-written streaming)

**L6. Regenerate streaming endpoints from TOML**
- `crates/thetadatadx/src/direct.rs` currently has 4 hand-written `streaming_endpoint!` blocks (stock trade/quote stream, option trade/quote stream).
- Action: extend `endpoint_surface.toml` schema to support `kind = "stream"` endpoints with a `callback_type` field.
- Extend `build_support/endpoints.rs` to generate streaming builder methods.
- Delete the hand-written blocks from `direct.rs`. `direct.rs` retains only the `DirectClient` struct, session management, gRPC channel setup, and the `query_info()` helper.
- Target: `direct.rs` shrinks from 645 lines to ~250.

### Phase 5 — Dispatch collapse

**L12. Collapse three dispatch shims into thin transport serializers**
- Each tool (CLI, REST, MCP) currently has its own match statement or helper dispatch.
- New shape: each tool is a ~150-line transport:
  ```
  tool → parse input into EndpointArgs → invoke_endpoint(name, args) → serialize output
  ```
- `tools/cli/src/main.rs`: clap argv parser → `EndpointArgs` builder → `invoke_endpoint` → tabular/JSON/CSV serializer.
- `tools/server/src/handler.rs`: axum query extractor → `EndpointArgs` → `invoke_endpoint` → JSON via `format.rs`.
- `tools/mcp/src/main.rs`: MCP JSON-RPC args → `EndpointArgs` → `invoke_endpoint` → MCP JSON response.
- No per-tool match statements on endpoint names. No per-tool option chaining. Everything flows through `invoke_endpoint`.
- Per-tool serializers (`format.rs`, MCP `serialize_*_ticks`) remain because output formatting is transport-specific, but they become pure `EndpointOutput → Value` functions.

### Phase 6 — Version bump + dependency lockdown

**L10. Kill v5/v6 compat, bump to v7.0.0**
- `Cargo.toml` workspace version → `7.0.0`.
- Audit for and delete: every `#[deprecated]` attribute's symbol, every `*_v5`, `*_v6_compat`, `legacy_*`, `raw_*` symbol. Preliminary grep shows minimal matches — this may be largely a no-op cleanup, but the audit still runs.
- Import paths break: `thetadatadx::direct::DirectClient` stays; `thetadatadx::legacy::*` dies if it exists.
- README: "v7 is not wire-compatible with v6 client code. See v7 migration guide (to be written post-release)."
- CHANGELOG: v7.0.0 entry lists every breaking change.
- Bump the embedded client version in `QueryInfo.terminal_version` (set via `env!("CARGO_PKG_VERSION")`) so ThetaData's server logs can distinguish v7 clients.

**L15. MSRV + dependency bump**
- Rust edition: `2024` (if stable) or `2021` with a 2024-prep pass.
- MSRV: `1.82` or latest stable minus 2.
- `cargo update` + review: tokio, tonic, prost, axum, sonic-rs, pyo3, criterion, clap, everything on latest.
- Any dep with no update in 6+ months gets audited; replaced or forked if abandoned.

### Phase 7 — Final verification

- Full workspace rebuild from scratch (`cargo clean && cargo build --release`)
- Full test sweep: `cargo test --workspace --release`
- Live endpoint test against production MDDS (61/61)
- Go SDK test, C++ SDK cmake build, Python SDK wheel build
- Codex independent review of the full diff
- Release notes draft
- Tag `v7.0.0`, push, let CI publish

---

## Target architecture

```
crates/thetadatadx/
  endpoint_surface.toml     ← single source of truth
                             (endpoints + option params + streaming flag)
  build.rs
  build_support/
    ├── parse external.proto
    ├── parse endpoint_surface.toml
    ├── cross-validate
    └── generate:
        ├── $OUT_DIR/registry_generated.rs
        ├── $OUT_DIR/endpoint_generated.rs       (invoke_endpoint dispatch)
        ├── $OUT_DIR/streaming_generated.rs      (FPSS-callback dispatch)  ← NEW
        ├── sdks/python/src/generated_historical_methods.rs  (direct write + @generated)
        ├── sdks/go/generated_historical.go                  (direct write + @generated)
        ├── sdks/go/generated_endpoint_options.go            (direct write + @generated)
        ├── sdks/cpp/include/generated_historical.hpp.inc    (direct write + @generated)
        ├── sdks/cpp/src/generated_historical.cpp.inc        (direct write + @generated)
        └── ffi/src/generated_endpoint_with_options.rs       (direct write + @generated)

crates/thetadatadx/src/
  endpoint.rs      ← invoke_endpoint, EndpointArgs, EndpointError
  direct.rs        ← ONLY: DirectClient session/gRPC. No endpoint definitions.
  fpss/            ← streaming protocol runtime
  validate.rs      ← shared validators (unchanged)
  unified.rs       ← ThetaDataDx (user-facing type)
  registry.rs      ← public metadata API (generated)

tools/
  cli/             ← ~150 lines: argv → EndpointArgs → stdout serializer
  server/          ← ~200 lines: axum → EndpointArgs → JSON
  mcp/             ← ~200 lines: MCP JSON-RPC → EndpointArgs → MCP JSON

sdks/
  python/src/
    lib.rs                              ← FPSS, session, offline tools, type converters
    generated_historical_methods.rs     ← @generated, included via include!()
  go/
    client.go                           ← FPSS, session, FFI boundary
    generated_historical.go             ← @generated
    generated_endpoint_options.go       ← @generated
  cpp/
    src/thetadx.cpp                     ← FPSS, session, FFI boundary
    include/generated_historical.{hpp,cpp}.inc  ← @generated
```

---

## Phasing & branch strategy

**Single branch:** `release/v7.0` off `main`.

Every phase is one or more commits on `release/v7.0`. No squashing until the final merge.

**Phase gate (runs after every commit):**

```bash
cargo fmt --all -- --check                              # formatting
cargo clippy --workspace --all-targets -- -D warnings   # lint (no warnings)
cargo test --workspace                                  # all tests
cargo build --release -p thetadatadx-ffi --locked       # FFI release
python3 scripts/check_docs_consistency.py               # docs parity
cd sdks/go && go build ./... && go test ./...           # Go SDK
cargo check --manifest-path sdks/python/Cargo.toml      # Python SDK compile
```

Any phase that fails a gate blocks progress. Fix first, then continue.

**Final merge:** At the end of Phase 7, `release/v7.0` gets rebased onto latest `main`, all commits preserved (not squashed — the history is the migration story), and tagged `v7.0.0`. Release notes published.

---

## Verification gates per phase

| Phase | Gate additions |
|-------|---------------|
| 0 | Standard gate |
| 1 | Standard gate + `cargo test --manifest-path sdks/python/Cargo.toml` smoke test |
| 2 | Standard gate + manual FFI calling convention smoke test (Go + C++ call 1 endpoint with options via new flag struct) |
| 3 | Standard gate + `git diff --exit-code -- 'sdks/**/generated_*' 'ffi/src/generated_*'` after `cargo build` |
| 4 | Standard gate + FPSS streaming smoke test (connect, receive 1 event) |
| 5 | Standard gate + end-to-end: CLI, REST, MCP each call `stock_history_eod` and get identical data |
| 6 | Standard gate + `cargo update` followed by full test suite rerun (dependencies are actually current AND still compile) |
| 7 | Full gate + live 61-endpoint sweep + Codex review |

---

## Rollback strategy

Each phase is a commit. If a phase breaks something we can't fix in <1 day, we `git revert` that commit and reopen the kill-list item for the next session.

`release/v7.0` stays alive until `v7.0.0` tag. If the whole effort stalls, `main` is untouched.

---

## Out of scope (explicitly not touching)

- FPSS wire protocol fixes (issue #192)
- Proto definitions (`external.proto`)
- `tdbe` encoding/decoding logic
- Live endpoint test data files
- GitHub Actions infrastructure beyond the docs gate
- Benchmark suite content

---

## Open questions (to resolve during execution, not now)

1. **Docs** — v7 gets a full docs rebuild or just a redirect page? Probably redirect: every v6 doc gets a v7 equivalent in the same path, old content archived under `docs-site/docs/archive/v6/`.
2. **MCP tool IDs** — breaking the MCP tool names means existing LLM prompts break. Either keep names stable across v6→v7 (recommended), or document the rename.
3. **Python wheel name** — if we break the API, do we bump package name to `thetadatadx-v7`? Probably no — PyPI version bump is enough, but users must pin `>=7.0.0,<8.0.0` explicitly.
4. **Go module path** — same question. Go modules treat major versions as separate paths (`.../v7`). This might force `sdks/go/v7/` directory. Resolve in Phase 6.
