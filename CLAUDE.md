# CLAUDE.md

> **Before any code change in this repo, read
> [`docs/internal/audit-protocol.md`](docs/internal/audit-protocol.md).**
> The protocol is the standing rulebook for every contributor — human or
> LLM-assisted. It encodes the patterns the audit cycles have already paid
> for and the gates that catch regressions.

## Quick orientation

ThetaDataDx is a Rust SDK for ThetaData market data with four language
surfaces (Rust, Python, TypeScript, C++) over a single Rust core. The
canonical entry points:

- `crates/thetadatadx/` — core SDK (auth, MDDS gRPC, FPSS streaming).
- `crates/tdbe/` — tick types, FIT / FIE codec, Greeks, `Price`.
- `ffi/` — stable `extern "C"` layer; the C ABI is the supported
  third-party C / C++ integration path.
- `sdks/python/` — PyO3 / maturin wheel.
- `sdks/typescript/` — napi-rs prebuilt binary.
- `sdks/cpp/` — RAII header-only wrapper.
- `tools/cli/` `tools/mcp/` `tools/server/` — supporting tools.

## Pre-commit

Run before every commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --no-deps --workspace
```

See `CONTRIBUTING.md` for the extended-surface checks and
`docs/internal/audit-protocol.md` for the full self-review checklist.

## Rules of engagement

- **Conventional Commits required.** See `CONTRIBUTING.md` for types
  and scopes.
- **One PR, one scope.** Drive-by changes get their own issue.
- **No `--no-verify`, no `--force-push` to `main`.** Hooks and branch
  protection exist for a reason.
- **Every `// SAFETY:` names the invariant.** Gate 14 enforces this.
- **No `#[allow(dead_code)]` / `#[allow(unused)]`.** Delete unused code
  or wire the caller in the same PR.
- **No mega-PRs.** Past ~800 LOC / 8 commits, split.
- **Every change clears the Nicholas-lens + trading-system-SWE bar.**
  See `docs/internal/audit-protocol.md` section 1.

## Where to find what

| Need | File |
|------|------|
| Standing audit + review protocol | `docs/internal/audit-protocol.md` |
| Development setup, PR process | `CONTRIBUTING.md` |
| Architecture overview | `docs/architecture.md` |
| API reference | `docs/api-reference.md` |
| Per-binding coverage status | `docs/ROADMAP.md` |
| Macro / codegen guide | `docs/macro-guide.md` |
| Endpoint TOML schema | `docs/endpoint-schema.md` |
| Wire schema (proto + tick layouts) | `crates/thetadatadx/proto/`, `crates/thetadatadx/tick_schema.toml` |
| Security policy | `SECURITY.md` |
| CI gates | `.github/workflows/ci.yml` + `scripts/check_*.py` |
| Cross-binding parity | `sdks/parity.toml` |
