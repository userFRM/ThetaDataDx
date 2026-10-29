# Contributing to ThetaDataDx

Thank you for your interest in contributing. This guide covers everything
you need to get started.

The SDK speaks three independent ThetaData surfaces: MDDS (gRPC), FPSS
(streaming), and FLATFILES (whole-universe daily blobs). When opening
an issue or PR that touches any of them, name the surface in the title
prefix: `feat(mdds): ...`, `feat(fpss): ...`, `feat(flatfiles): ...`.
Cross-language binding parity is tracked under separate per-binding
issues.

## Prerequisites

- **Rust stable** (see `rust-toolchain.toml` - includes rustfmt and clippy)
- **protoc** (Protocol Buffers compiler) - only needed if modifying `.proto` files
- **Python 3.12+** - for the Python SDK
- **maturin** - for building the PyO3 Python bindings (`pip install "maturin>=1.9.4,<2.0"`)
- **Node.js 18+** - for the TypeScript/Node.js SDK

Note: a normal `cargo build` does not need `protoc`. The crate ships the committed gRPC snapshot (`proto/beta_endpoints.snapshot.rs`); `protoc` is only needed with `--features grpc-codegen` to regenerate that snapshot after editing a `.proto` file.

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

# 6. Language SDK tests (if modified; outside the Cargo workspace)
maturin develop -m thetadatadx-py/Cargo.toml   # needs an active virtualenv
python -m pytest thetadatadx-py/tests -q
(cd thetadatadx-ts && npm ci && npm test)
c++ -std=c++17 -fsyntax-only -I thetadatadx-cpp/include thetadatadx-cpp/src/thetadatadx.cpp
cmake -S thetadatadx-cpp -B build/cpp
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
- `thetadatadx-rs/proto/mdds.proto` for the wire contract
- `thetadatadx-rs/endpoint_surface.toml` for the normalized SDK surface
- `thetadatadx-rs/tick_schema.toml` for DataTable parser layouts

The build expands that metadata into the registry, shared endpoint runtime, and
`MarketDataClient` declarations automatically.

1. **Update the proto** (if the endpoint uses a new message type)
   - Update `thetadatadx-rs/proto/mdds.proto`
   - `cargo build` regenerates Rust message types automatically
   - The committed gRPC codegen snapshot at
     `thetadatadx-rs/proto/beta_endpoints.snapshot.rs` is verified
     by the build script but never written by it. Refresh it
     explicitly with
     `cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot --features grpc-codegen`
     and commit the resulting diff alongside the proto change.

2. **Add or update the endpoint surface**
   - Add an entry to `thetadatadx-rs/endpoint_surface.toml`
   - Reuse existing `param_groups` / `templates` where possible
   - `cargo build` validates the declared surface against `mdds.proto` and generates the registry/runtime/mdds surfaces

3. **Add the column schema** (if the response has a new layout)
   - Add a `[types.YourTick]` block to `thetadatadx-rs/tick_schema.toml`
   - `cargo build` generates the parser
   - The header comments in `tick_schema.toml` document the TOML format
   - Note: tick type structs, `Price`, enums, codecs, and Greeks live in the
     internal data-format module `thetadatadx-rs/src/tdbe/`. If you add a new
     tick type or modify existing types, edit that module first.

4. **Review the generated mdds/runtime surfaces**
   - Most endpoint additions should not require hand-editing files under `mdds/`
   - Only change files under `build_support/endpoints/` or the macro layer if the new endpoint shape cannot be expressed by the existing surface spec

5. **Regenerate downstream SDK/tool surfaces**
   - Endpoint wrappers project from `thetadatadx-rs/endpoint_surface.toml`
   - Non-endpoint SDK/tool surfaces project from `thetadatadx-rs/sdk_surface.toml`
   - Tick projection helpers project from `thetadatadx-rs/tick_schema.toml`
   - Run `cargo run -p thetadatadx-rs --features config-file --bin generate_sdk_surfaces`
   - Only hand-edit SDK runtime plumbing when the change is intentionally outside the generated surface

6. **Update CHANGELOG.md** under `[Unreleased]`

See `thetadatadx-rs/proto/MAINTENANCE.md` for the full step-by-step guide.

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
2. Update `endpoint_surface.toml` when the normalized SDK surface changes
3. Update `tick_schema.toml` if DataTable column layouts changed
4. Run the relevant checks from the sections above
5. See `thetadatadx-rs/proto/MAINTENANCE.md` for the detailed guide

## Community

Join the ThetaData Discord for questions and discussion: **[discord.thetadata.us](https://discord.thetadata.us/)**

## Code of Conduct

This project follows the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md).
Be respectful, constructive, and professional in all interactions.
