# Contributing to ThetaDataDx

Thank you for your interest in contributing. This guide covers everything
you need to get started.

The SDK speaks two independent ThetaData services: MDDS (gRPC market
data, the whole-universe flat files included) and FPSS (streaming). When
opening an issue or PR, name the area in the title prefix:
`feat(mdds): ...`, `feat(fpss): ...`, `feat(flatfiles): ...`.
Cross-language binding parity is tracked under separate per-binding
issues.

## Prerequisites

- **Rust stable** (see `rust-toolchain.toml` - includes rustfmt and clippy)
- **protoc** (Protocol Buffers compiler) - only needed if modifying `.proto` files
- **Python 3.12+** - for the Python SDK
- **maturin** - for building the PyO3 Python bindings (`pip install "maturin>=1.9.4,<2.0"`)
- **Node.js 18+** - for the TypeScript/Node.js SDK

Note: a normal `cargo build` does not need `protoc`. The crate ships the committed gRPC snapshot (`proto/beta_endpoints.snapshot.rs`); `protoc` is only needed to regenerate that snapshot after editing a `.proto` file.

## Development Setup

```bash
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx

# Run the workspace test suite (every crate, bindings and tools included)
cargo test --workspace

# For integration tests against ThetaData servers, create creds.txt:
# Line 1: email
# Line 2: password
```

## Pre-commit Checks

Run these **before every commit**. CI will reject anything that fails.

```bash
# 1. Format
cargo fmt --all -- --check

# 2. Lint
cargo clippy --workspace --all-targets -- -D warnings

# 3. Test
cargo test --workspace

# 4. FFI build (if modified)
cargo build --release -p thetadatadx-ffi

# 5. Language SDK tests (if modified)
maturin develop -m thetadatadx-py/Cargo.toml   # needs an active virtualenv
python -m pytest thetadatadx-py/tests -q
(cd thetadatadx-ts && npm ci && npm test)
c++ -std=c++17 -fsyntax-only -I thetadatadx-cpp/include thetadatadx-cpp/src/thetadatadx.cpp
cmake -S thetadatadx-cpp -B build/cpp
cmake --build build/cpp --target thetadatadx_cpp
```

One-liner for the common Rust-only case:

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

For full Linux parity with the current CI `surfaces` job, run the language-SDK checks above as well.

## Commit Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).

### Format

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Types

| Type | When to use |
|------|-------------|
| `feat` | New feature or endpoint |
| `fix` | Bug fix |
| `docs` | Documentation only |
| `chore` | Build, CI, tooling, dependency updates |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `perf` | Performance improvement |
| `test` | Adding or updating tests |
| `style` | Formatting, whitespace (no code change) |

### Scopes (optional)

| Scope | What it covers |
|-------|---------------|
| `core` | `thetadatadx-rs/` (includes the `tdbe` data-format layer) |
| `ffi` | `thetadatadx-ffi/` |
| `python` | `thetadatadx-py/` |
| `typescript` | `thetadatadx-ts/` |
| `cpp` | `thetadatadx-cpp/` |
| `mcp` | `tools/mcp/` |
| `server` | `tools/server/` |
| `docs` | `docs-site/`, top-level Markdown |

### Examples

```
feat(core): add stock_history_vwap endpoint
fix(python): correct event key from "type" to "kind"
docs: update streaming examples for all languages
chore: bump version to 3.2.2
perf(core): use precomputed pow10 table in price decoding
```

### Breaking changes

Add `!` after the type or a `BREAKING CHANGE:` footer:

```
feat(core)!: replace MarketDataClient with the unified Client
```

## How to Add a New Endpoint

The endpoint-facing source of truth is split across:
- `thetadatadx-rs/proto/mdds.proto`, the wire contract. ThetaData provides it; do not hand-edit it, and do not rename its `BetaEndpoints` package, which production routes on.
- `thetadatadx-rs/src/surface/endpoints.rs`, the endpoint table, for every binding surface and the docs
- `thetadatadx-rs/src/surface/ticks.rs`, the tick table, for the tick types the endpoints return

The endpoint table expands at compile time into the registry, the shared endpoint dispatch, the `MarketDataClient` methods and builders, and the Python, TypeScript and C ABI endpoint surfaces. `generate_sdk_surfaces` renders the Python type stub, the C and C++ headers, the live validators and the docs site's endpoint pages from it.

1. **Update the proto** (if the endpoint uses a new message type)
   - Update `thetadatadx-rs/proto/mdds.proto`
   - Regenerate the committed message types and client stubs at `thetadatadx-rs/proto/beta_endpoints.snapshot.rs` with `cargo run -p xtask --features grpc-codegen --bin refresh_grpc_snapshot` (the only step that needs `protoc`) and commit the resulting diff alongside the proto change. CI runs the same command and fails when it changes the snapshot.

2. **Add or update the endpoint surface**
   - Add an entry to the endpoint table in `thetadatadx-rs/src/surface/endpoints.rs`, reusing the parameter definitions and groups in `__param!` where possible; its module documentation describes the entry grammar
   - `cargo build` checks the entry against the request messages of `mdds.proto`. `generate_sdk_surfaces` fails while an RPC of `mdds.proto` has no entry, and rejects an unknown shape, a list entry that does not return `StringList` (or a non-list entry that does), a default its type does not accept, and an enum parameter whose description does not list exactly the values of its enum

3. **Add the tick type** (if the response has a new layout; reuse an existing tick otherwise)
   - Add an entry to the tick table in `thetadatadx-rs/src/surface/ticks.rs`; its module documentation describes the entry grammar and the column types
   - The entry expands at compile time into the tick struct, its decoder, its Arrow and Polars builders, the Python and TypeScript row classes and the C ABI's array types and entry points, and `generate_sdk_surfaces` renders the C tick structs and their declarations, the C++ aliases, layout asserts, flag accessors and Arrow IPC helpers from it
   - A few projections are still written by hand per tick: the `EndpointOutput` and `ReturnType` variants and the tick imports in the core, and the JSON rows in `tools/server/src/format.rs` and `tools/mcp/src/main.rs`. Exhaustive matches fail the build until each one is there
   - `Price`, enums, codecs, and Greeks live in the internal data-format module `thetadatadx-rs/src/tdbe/`

   To add a column to an existing tick, add one line to its `columns` list and raise the field count in its first `doc` line, which the compiler checks.

4. **Review the generated mdds/runtime surfaces**
   - Most endpoint additions should not require hand-editing files under `mdds/`
   - Only change the macro layer (`mdds/endpoints.rs`, `mdds/macros.rs`) if the new endpoint shape cannot be expressed by the table's grammar

5. **Regenerate downstream SDK/tool surfaces**
   - Run `cargo run -p xtask --bin generate_sdk_surfaces` and commit the result; CI runs it with `--check`
   - Rebuild the TypeScript declarations with `npm run build` in `thetadatadx-ts` (or `python3 scripts/dev/check_napi_drift.py`) and commit `index.d.ts` and `index.js`
   - Only hand-edit SDK runtime plumbing when the change is intentionally outside the generated surface

6. **Update CHANGELOG.md** under `[Unreleased]`

## Pull Request Process

1. **Branch** from `main` using the convention: `feat/description`, `fix/description`, `docs/description`
2. **Pre-commit checks** must all pass (see above)
3. **Open PR** against `main` with a clear title following commit convention
4. **CI must pass** - format, lint, test, FFI build
5. **Squash merge** for clean history

Every PR must include:
- Passing CI
- Updated `CHANGELOG.md` if user-facing
- Updated documentation if any public API changed

## Public API Stability

The Rust crate published from this repo (`thetadatadx-rs`) follows [semver](https://semver.org/) on every release, with cargo's 0.x rules: while the version is below 1.0, the minor position is the breaking one. CI gates this with [`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks), comparing against the version published on crates.io. The check is advisory on pull requests and main, where an unreleased line normally accumulates changes, and fatal on a release tag: a break tagged as a patch release stops the publish. A release that breaks the public API bumps the minor version, says so in the PR body, and lists every removed or renamed symbol under `### Removed` or `### Changed` in `CHANGELOG.md`.

To run the same check locally before opening a PR:

```bash
cargo install cargo-semver-checks --locked  # one-time
cargo semver-checks check-release -p thetadatadx-rs
```

If a false positive surfaces (typically on macro-generated re-exports),
prefer hiding the offending item with `#[doc(hidden)]` over adjusting
the baseline; the gate stays useful only as long as it reports real
breakage.

## How to Update After a ThetaData Protocol Update

When ThetaData ships a new proto revision:

1. Replace `thetadatadx-rs/proto/mdds.proto`
2. Regenerate and commit the gRPC snapshot as in step 1 above
3. Entries whose request messages were renamed or removed no longer compile; fix them in the endpoint table. New RPCs need entries; `generate_sdk_surfaces` fails until every RPC has one
4. Update the tick table (`src/surface/ticks.rs`) if DataTable column layouts changed
5. Run the relevant checks from the sections above

## Community

Join the ThetaData Discord for questions and discussion: **[discord.thetadata.us](https://discord.thetadata.us/)**

## Code of Conduct

This project follows the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md).
Be respectful, constructive, and professional in all interactions.
