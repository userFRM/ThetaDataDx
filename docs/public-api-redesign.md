# Public API Redesign

## Purpose

This document defines the migration plan for a more ergonomic public API across
Rust, Python, Go, C++, REST, and MCP without sacrificing the current
single-source-of-truth endpoint model.

The intent is not to replace the exact generated endpoint surface. The intent
is to keep that exact surface as the canonical parity layer while adding a
smaller, more intentional human-facing layer above it.

## Goals

- Preserve one canonical endpoint source of truth.
- Improve ease of use for common historical, live, and analytics workflows.
- Remove transport quirks from the default public API.
- Keep full parity and escape hatches for advanced users.
- Maintain cross-language conceptual consistency.
- Make documentation, REST, and MCP derive from the same semantic model where
  possible.

## Non-Goals

- Do not throw away the current generated endpoint surface.
- Do not force all public API layers to be generated.
- Do not split the core engineering work across multiple source repos at this
  stage.
- Do not hide which underlying endpoint is being used in ways that make
  debugging or parity validation harder.

## Repository Strategy

The monorepo remains the canonical source of truth.

Reasons:

- the endpoint surface, registry, REST, MCP, and bindings already share one
  coherent pipeline
- splitting now would introduce version skew, duplicated issue tracking,
  duplicated CI, and weaker parity discipline
- the strongest asset in the current architecture is one coordinated
  multi-language surface driven from one core specification

If branding or discoverability later benefits from language-specific front-door
repositories, those should be thin distribution repos pointing back to this
monorepo rather than becoming the primary source of logic.

## Target Public Shape

The public API should converge on two layers:

1. Exact layer
   - The current generated endpoint surface.
   - Full parity with the underlying service contract.
   - Best for advanced users, debugging, and transport-level validation.
2. Ergonomic layer
   - A small façade organized around user workflows.
   - Primary entrypoint for most users.
   - Thin and explicit, implemented on top of the exact layer.

Recommended top-level conceptual model:

- `historical`
- `live`
- `analytics`
- `direct` or `exact`

The exact naming can vary per language, but the concepts should remain aligned.

## Design Principles

### 1. Exact Surface Stays Canonical

The generated endpoint surface remains the parity contract and internal
foundation.

- It should continue to drive registry metadata.
- It should continue to drive REST/OpenAPI generation.
- It should continue to drive MCP tool generation.
- It should remain fully available to power users.

### 2. Ergonomic Surface Is Intentionally Designed

The ergonomic API should be mostly handwritten.

This layer should not be fully generated from endpoint metadata. Generation is
appropriate for repetitive contract projection. Human judgment is more
important for the user-facing façade.

### 3. Public APIs Should Use Semantic Types

Common raw strings should be wrapped in typed values where possible:

- `Date`
- `Year`
- `Interval`
- `TimeOfDay`
- `Right`
- `RequestType`
- `Expiration`
- `Strike`
- `Symbols`

Option bulk semantics should be expressed explicitly with selectors rather than
transport sentinels:

- `StrikeSelector::Exact`
- `StrikeSelector::Wildcard`
- `ExpirationSelector::Exact`
- `ExpirationSelector::Wildcard`

### 4. Transport Quirks Stay Internal

Users should not need to think in terms of `"0"` wildcard sentinels or legacy
query aliases in the default happy path. Those should remain translation
details inside the exact endpoint layer and transport adapters.

### 5. Historical and Live Should Feel Coherent

The current exact surface is wide and precise. The next layer should be narrow
and organized by workflow.

Example conceptual grouping:

- `client.historical().stock().eod(...)`
- `client.historical().option().chain(...)`
- `client.live().subscribe_quotes(...)`
- `client.analytics().all_greeks(...)`

## Migration Plan

### Phase 1: API Charter

Write down the public rules before coding:

- top-level naming
- layering model
- typed scalar policy
- request/result naming conventions
- compatibility policy
- deprecation policy

Deliverable:

- this design doc plus a short checklist in contribution docs for future API
  changes

### Phase 2: Typed Value Foundations

Introduce typed public wrappers in Rust first, then mirror the same concepts in
other languages where appropriate.

Initial targets:

- `Date`
- `Interval`
- `Right`
- `TimeOfDay`
- `Expiration`
- `Strike`
- wildcard selectors for option bulk queries

Requirements:

- parsing from current string forms
- formatting back to transport values
- validation localized in the type, not spread across callers

### Phase 3: Historical Façade

Add a workflow-oriented historical layer on top of the exact generated
endpoints.

Recommended organization:

- `historical.stock`
- `historical.option`
- `historical.index`
- `historical.calendar`
- `historical.rate`

This façade should:

- group related operations cleanly
- prefer typed request values
- call the exact generated methods underneath
- stay thin enough that endpoint provenance remains obvious

### Phase 4: Live Façade

Build a streaming façade that is easier to use than the current lower-level
subscription API while preserving full control for advanced users.

Focus areas:

- typed contract builders
- clear subscription helpers
- event iteration / polling ergonomics
- consistent event containers across languages

### Phase 5: Python Result Containers

The Python API should graduate from "lists of dicts as the primary story" to a
more intentional result model.

Target features:

- iterable result batches
- `.records`
- `.first()`
- `.to_dataframe()`
- `.to_polars()`
- metadata access
- replay helpers where applicable

Compatibility should remain straightforward:

- callers should still be able to obtain plain record lists easily

### Phase 6: Spec Enrichment

Extend the checked-in endpoint surface specification with semantic metadata that
improves downstream projection quality.

Candidate fields:

- `domain`
- `family`
- `operation_kind`
- `supports_bulk`
- `supports_range`
- `selector_semantics`
- `response_shape`
- `example_inputs`
- `subscription_tier`

This should continue to generate:

- registry metadata
- exact endpoint layer
- REST/OpenAPI
- MCP tool schemas
- repetitive reference documentation

### Phase 7: Documentation Migration

The documentation should present the ergonomic API first and the exact surface
second.

Rules:

- narrative guides should remain handwritten
- repetitive reference docs should be generated or validated mechanically
- examples should be executable or checked where practical
- exact endpoint reference should remain available for parity/debug work

### Phase 8: Compatibility Window

The transition should be additive first.

- add the façade
- keep the flat exact methods
- update docs to prefer the façade
- add migration examples from old to new usage
- only deprecate once the new path is stable and well documented

### Phase 9: Semver Cleanup

Recommended cadence:

- `v6.x`: additive façade and typed values
- `v7.0`: make façade primary in docs and public positioning; deprecate obvious
  redundant legacy convenience surfaces
- `v8.0`: remove compatibility layers that have completed a full migration cycle

## Generated vs Handwritten Boundaries

### Keep Generated

- exact endpoint declarations
- registry
- OpenAPI / REST metadata
- MCP tool definitions
- repetitive reference tables
- parity checks

### Keep Handwritten

- ergonomic façade design
- live UX
- result containers
- narrative documentation
- migration guides

This boundary is important. The exact layer benefits from generation. The
ergonomic layer benefits from deliberate product-level design.

## Guardrails

- No public happy-path API should require raw sentinel values.
- No duplicated endpoint truth across SDK, REST, MCP, and docs.
- No compatibility façade unless it buys a real migration or layering benefit.
- No drift between the checked-in endpoint spec and published docs.
- No public redesign without migration examples and tests.

## Validation Requirements

The redesign should ship with permanent checks.

Minimum validation:

- endpoint parity tests
- docs consistency checks
- OpenAPI path/operation ID checks
- MCP tool count and schema checks
- façade-level smoke tests in Rust and Python at minimum
- cross-language basic workflow tests for historical, live, and analytics

## Practical Recommendation

Execute this as a layered migration, not a rewrite.

The exact generated surface is already valuable. The right move is to preserve
that strength and put a better public experience on top of it.

That preserves:

- parity
- maintainability
- cross-language coherence
- auditability

while materially improving:

- ease of use
- discoverability
- boilerplate
- long-term API quality
