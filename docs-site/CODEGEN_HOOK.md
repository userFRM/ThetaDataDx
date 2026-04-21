# Per-endpoint docs codegen hook (SSOT)

**Status: not wired yet — narrative-only pages shipped in v8.0.2.**

The `docs-site/` content split into two classes of pages in v8.0.2:

| Class | Source of truth | Author method |
|-------|-----------------|---------------|
| Narrative pages (Getting Started, Streaming, Greeks, Migration, Performance, Quickstart by Language) | This worktree | Hand-authored prose |
| Per-endpoint pages (everything under `docs/historical/`) and the one-shot `api-reference.md` | `crates/thetadatadx/endpoint_surface.toml` | Hand-authored today; **must migrate to codegen** |

Anything that enumerates endpoint methods, parameters, return shapes, or per-endpoint kwargs must be generated from `endpoint_surface.toml` at build time — not hand-written. The narrative pages do not enumerate; they reference the API reference and the per-endpoint pages for specifics.

## Wiring sketch

VitePress does not ship a native pre-build codegen step, but two approaches fit:

### Option A — Node-side build script

Add a `prebuild` script in `docs-site/package.json`:

```json
{
  "scripts": {
    "prebuild": "node scripts/generate-endpoint-partials.mjs",
    "build": "vitepress build docs",
    "dev": "npm run prebuild && vitepress dev docs"
  }
}
```

The script reads `../../crates/thetadatadx/endpoint_surface.toml`, emits one markdown partial per endpoint to `docs-site/docs/historical/<kind>/<name>/_params.md`, and pages include it via VitePress's `<!--@include: ./_params.md-->`.

### Option B — Python-side build script (matches the existing `scripts/check_docs_consistency.py` pattern)

Add `scripts/generate_docs_partials.py` that the docs-site build target invokes before `vitepress build`. Same output shape, but stays in the Python tooling the repo already uses.

## What to generate

Per endpoint in `endpoint_surface.toml`:

1. Method signature per language (Rust, Python, TypeScript, Go, C++).
2. Parameter table (name, type, required, description, `"0"`/wildcard semantics).
3. Return type and tick field list.
4. Tier badge (from `endpoint_surface.toml` + `scripts/check_tier_badges.py`).
5. REST/OpenAPI operationId and path — already parity-checked in `check_docs_consistency.py`.

## Why not ship now

The v8.0.2 docs scope was narrative pages aimed at the ThetaData Python SDK migration story and the benchmark headline. Every per-endpoint page under `docs/historical/` is hand-maintained today and has been through a tier-badge consistency gate, so the pages are not stale — they are just not generated. Retrofit is a follow-up; tracked as "docs codegen" in the v8.0.3 planning.
