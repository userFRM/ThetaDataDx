# Contributing to ThetaDataDx

Thank you for your interest in contributing. This guide covers everything
you need to get started.

## Prerequisites

- **Rust stable** (see `rust-toolchain.toml` - includes rustfmt and clippy)
- **protoc** (Protocol Buffers compiler) - only needed if modifying `.proto` files
- **Python 3.9+** - for the Python SDK
- **maturin** - for building the PyO3 Python bindings (`pip install "maturin>=1.9.4,<2.0"`)
- **Node.js 18+** - for the TypeScript/Node.js SDK
- **Go 1.21+** - for the Go SDK

Note: `protoc` is required even if you're not modifying `.proto` files, because `build.rs` compiles protos during `cargo build`.

## Development Setup

```bash
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx

# Run the core workspace test suite
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
cargo clippy --workspace -- -D warnings

# 3. Test
cargo test --workspace

# 4. FFI build (if modified)
cargo build --release -p thetadatadx-ffi

# 5. Extended surfaces (if modified)
cargo clippy --manifest-path tools/mcp/Cargo.toml -- -D warnings
cargo test --manifest-path tools/mcp/Cargo.toml
cargo clippy --manifest-path tools/server/Cargo.toml -- -D warnings
cargo test --manifest-path tools/server/Cargo.toml
cargo clippy --manifest-path tools/cli/Cargo.toml -- -D warnings
cargo test --manifest-path tools/cli/Cargo.toml

# 6. Language SDK smoke checks (if modified)
cargo check --manifest-path sdks/python/Cargo.toml
(cd sdks/typescript && npm run build)
(cd sdks/go && LD_LIBRARY_PATH=../../target/release go test ./...)
c++ -std=c++17 -fsyntax-only -I sdks/cpp/include sdks/cpp/src/thetadx.cpp
cmake -S sdks/cpp -B build/cpp
cmake --build build/cpp --target thetadatadx_cpp
```

One-liner for the common Rust-only case:

```bash
cargo fmt --all -- --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

For full Linux parity with the current CI `surfaces` job, run the extended-surface and language-SDK checks above as well.

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
| `tdbe` | `crates/tdbe/` |
| `core` | `crates/thetadatadx/` |
| `ffi` | `ffi/` |
| `python` | `sdks/python/` |
| `typescript` | `sdks/typescript/` |
| `go` | `sdks/go/` |
| `cpp` | `sdks/cpp/` |
| `cli` | `tools/cli/` |
| `mcp` | `tools/mcp/` |
| `server` | `tools/server/` |
| `docs` | `docs/`, `docs-site/` |

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
feat(core)!: replace DirectClient with ThetaDataDx unified client
```

## How to Add a New Endpoint

> **Deep dive:** see [`docs/macro-guide.md`](docs/macro-guide.md) for the
> internal macro system and generated builder model.

The endpoint-facing source of truth is now split across:
- `crates/thetadatadx/proto/external.proto` for the wire contract
- `crates/thetadatadx/endpoint_surface.toml` for the normalized SDK surface
- `crates/thetadatadx/tick_schema.toml` for DataTable parser layouts

The build expands that metadata into the registry, shared endpoint runtime, and
`DirectClient` declarations automatically.

1. **Update the proto** (if the endpoint uses a new message type)
   - Update `crates/thetadatadx/proto/external.proto`
   - `cargo build` regenerates Rust types automatically

2. **Add or update the endpoint surface**
   - Add an entry to `crates/thetadatadx/endpoint_surface.toml`
   - Reuse existing `param_groups` / `templates` where possible
   - `cargo build` validates the declared surface against `external.proto` and generates the registry/runtime/direct surfaces

3. **Add the column schema** (if the response has a new layout)
   - Add a `[types.YourTick]` block to `crates/thetadatadx/tick_schema.toml`
   - `cargo build` generates the parser
   - See `docs/endpoint-schema.md` for the TOML format
   - Note: tick type structs, `Price`, enums, codecs, and Greeks live in `crates/tdbe/`.
     If you add a new tick type or modify existing types, edit `tdbe` first.

4. **Review the generated direct/runtime surfaces**
   - Most endpoint additions should not require hand-editing `direct.rs`
   - Only change `build_support/endpoints.rs` or the macro layer if the new endpoint shape cannot be expressed by the existing surface spec

5. **Regenerate downstream SDK/tool surfaces**
   - Endpoint wrappers project from `crates/thetadatadx/endpoint_surface.toml`
   - Non-endpoint SDK/tool surfaces project from `crates/thetadatadx/sdk_surface.toml`
   - Tick projection helpers project from `crates/thetadatadx/tick_schema.toml`
   - Run `cargo run -p thetadatadx --features config-file --bin generate_sdk_surfaces`
   - Only hand-edit SDK runtime plumbing when the change is intentionally outside the generated surface

6. **Update CHANGELOG.md** under `[Unreleased]`

See `crates/thetadatadx/proto/MAINTENANCE.md` for the full step-by-step guide.

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

## How to Update After a ThetaData Protocol Update

When ThetaData ships a new official proto revision:

1. Replace `crates/thetadatadx/proto/external.proto`
2. Update `endpoint_surface.toml` when the normalized SDK surface changes
3. Update `tick_schema.toml` if DataTable column layouts changed
4. Run the relevant checks from the sections above
5. See `crates/thetadatadx/proto/MAINTENANCE.md` for the detailed guide

`docs/reverse-engineering.md` is kept as historical context for how the project
was originally bootstrapped, not as the primary maintenance workflow.

## Community

Join the ThetaData Discord for questions and discussion: **[discord.thetadata.us](https://discord.thetadata.us/)**

## Code of Conduct

This project follows the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md).
Be respectful, constructive, and professional in all interactions.
