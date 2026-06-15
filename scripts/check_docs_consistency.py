#!/usr/bin/env python3
"""Validate checked-in documentation against the endpoint surface spec.

This script keeps the human-facing docs aligned with the current SDK and REST
surface by checking a few high-signal invariants:

- endpoint/tool counts in top-level docs
- REST/OpenAPI path + operationId parity with `endpoint_surface.toml`
- one generated docs-site reference page per registry endpoint (and no
  stale extras), each carrying the fixed page anatomy markers
- `llms.txt` covering every page on the site (and naming no deleted page)
- current server docs staying on the v3 path scheme and current defaults
- `CHANGELOG.md` and `docs-site/docs/changelog.md` staying byte-identical
  (the docs-site build re-publishes whatever is on `main`, so any drift
  is a release-note bug the moment it lands)

The generated reference tree itself is byte-verified by
`cargo run -p thetadatadx --features config-file,__internal --bin generate_docs_site -- --check`
in CI; the structural checks here are the cargo-free fast path the docs
deploy workflow runs.

Exits non-zero on any mismatch. Run from repo root; the script resolves
the workspace root from its own path so `cd` is not required.
"""

from __future__ import annotations

from pathlib import Path
import re
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
SURFACE = tomllib.loads((ROOT / "crates/thetadatadx/endpoint_surface.toml").read_text())
ENDPOINTS = SURFACE["endpoints"]
TEMPLATES = SURFACE["templates"]
ENDPOINT_NAMES = {ep["name"] for ep in ENDPOINTS}
REST_PATHS = {ep["rest_path"].removeprefix("/v3") for ep in ENDPOINTS}

DOCS_SITE = ROOT / "docs-site/docs"
OPENAPI_YAML = DOCS_SITE / "public/thetadatadx.yaml"

# Paths exposed only by the thetadatadx-server binary (not by the upstream
# ThetaData terminal). Allowlist these so the drift check focuses on
# upstream-tracking endpoints. `/v3/system/shutdown` is a privileged
# graceful-shutdown route gated by a per-startup UUID token — see
# `tools/server/src/handler.rs::system_shutdown` and the hardening section
# in `docs-site/docs/server/index.md`.
SERVER_SPECIFIC_PATHS = {"/v3/system/shutdown"}
BUILDER_PARAMS = {
    param["name"]
    for group in SURFACE["param_groups"].values()
    for param in group.get("params", [])
    if param.get("binding") == "builder"
}

# Global request-level options (not per-endpoint builder params, but still
# required fields on `ThetaDataDxEndpointRequestOptions` / `EndpointRequestOptions`
# so the FFI struct layout must include them).
GLOBAL_REQUEST_OPTIONS = {opt["name"] for opt in SURFACE.get("request_options_global", [])}

# All fields the request-options structs must expose (per-endpoint builders +
# cross-cutting globals). Drift in either direction is a bug.
ALL_OPTION_FIELDS = BUILDER_PARAMS | GLOBAL_REQUEST_OPTIONS


def endpoint_kind(endpoint: dict) -> str:
    kind = endpoint.get("kind")
    template_name = endpoint.get("template")
    seen: set[str] = set()
    while kind is None and template_name is not None:
        if template_name in seen:
            fail(f"template cycle while resolving kind for endpoint {endpoint['name']}")
        seen.add(template_name)
        template = TEMPLATES.get(template_name)
        if template is None:
            fail(f"endpoint {endpoint['name']} references unknown template {template_name!r}")
        kind = template.get("kind")
        template_name = template.get("extends")
    return kind or "parsed"


REGISTRY_ENDPOINTS = [ep for ep in ENDPOINTS if endpoint_kind(ep) != "stream"]
EXPECTED_TOOL_COUNT = len(REGISTRY_ENDPOINTS) + 3


def lower_camel(snake: str) -> str:
    head, *tail = snake.split("_")
    return head + "".join(part.capitalize() for part in tail)


def fail(message: str) -> None:
    print(f"docs consistency error: {message}", file=sys.stderr)
    raise SystemExit(1)


def expect_contains(path: Path, snippet: str) -> None:
    text = path.read_text()
    if snippet not in text:
        fail(f"{path.relative_to(ROOT)} missing expected text: {snippet!r}")


def expect_not_contains(path: Path, snippet: str) -> None:
    text = path.read_text()
    if snippet in text:
        fail(f"{path.relative_to(ROOT)} contains stale text: {snippet!r}")


def check_static_docs() -> None:
    expect_contains(ROOT / "README.md", "Documentation site (GitHub Pages)")
    expect_contains(
        ROOT / "README.md",
        "MCP server exposing every historical endpoint to AI clients",
    )

    expect_contains(
        ROOT / "tools/mcp/README.md",
        "## Available Tools",
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        "Every generated historical endpoint plus 3 offline tools (`ping`, `all_greeks`, `implied_volatility`).",
    )

    expect_contains(
        DOCS_SITE / "mcp.md",
        "Every generated historical endpoint plus `ping`, `all_greeks`, and `implied_volatility`.",
    )
    # Version strings in getting-started docs must match the workspace
    # major; the version-sync gate (`scripts/check_version_sync.py`)
    # enforces this against `crates/thetadatadx/Cargo.toml` canonically.
    expect_contains(
        DOCS_SITE / "articles/getting-started.md",
        'thetadatadx = "12"',
    )
    expect_contains(
        DOCS_SITE / "mcp.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
    )

    # Website changelog must match repo root CHANGELOG.md
    repo_changelog = (ROOT / "CHANGELOG.md").read_text()
    site_changelog = (DOCS_SITE / "changelog.md").read_text()
    if repo_changelog != site_changelog:
        fail(
            "docs-site/docs/changelog.md is out of sync with CHANGELOG.md. "
            "Run: cp CHANGELOG.md docs-site/docs/changelog.md"
        )

    server_pages = [
        DOCS_SITE / "server/index.md",
        DOCS_SITE / "server/http.md",
        DOCS_SITE / "server/websocket.md",
    ]
    expect_contains(ROOT / "tools/server/README.md", "25503")
    expect_contains(ROOT / "tools/server/README.md", "/v3/stock/history/ohlc_range")
    expect_contains(ROOT / "tools/server/README.md", "/v3/option/snapshot/quote?symbol=")
    expect_contains(DOCS_SITE / "server/index.md", "25503")
    expect_contains(DOCS_SITE / "server/http.md", "/v3/stock/history/ohlc_range")
    expect_contains(DOCS_SITE / "server/http.md", "/v3/option/snapshot/quote?symbol=")
    for path in [ROOT / "tools/server/README.md", *server_pages]:
        expect_not_contains(path, "/v2/")
        expect_not_contains(path, "/v3/hist/")
        expect_not_contains(path, "/v3/snapshot/")
        expect_not_contains(path, "/v3/list/roots/")
        expect_not_contains(path, "/v3/list/dates/")
        expect_not_contains(path, "/v3/at_time/")
        expect_not_contains(path, "?root=")
        expect_not_contains(path, "&exp=")
        expect_not_contains(path, "&ivl=")

    sdk_overview = ROOT / "sdks/README.md"
    expect_contains(sdk_overview, "`ThetaDataDxClient` / `ThetaDataDxStreamHandle`")
    expect_contains(sdk_overview, "| **Unified** | `thetadatadx_client_connect`, `thetadatadx_client_historical`, `thetadatadx_client_*`, `thetadatadx_client_free` |")
    # The streaming surface exposes a polymorphic
    # thetadatadx_streaming_subscribe / _unsubscribe pair over
    # ThetaDataDxSubscriptionRequest. The contract here is "the standalone
    # streaming row leads with thetadatadx_streaming_connect" — exact wording
    # sufficient.
    expect_contains(sdk_overview, "**Standalone streaming** | `thetadatadx_streaming_connect`")

    # Strikes are dollars on every public surface; the scaled-integer
    # vocabulary must never reappear (the WebSocket envelope's
    # thousandths note on server/websocket.md spells the exception
    # without the banned phrasing).
    strike_docs = list((DOCS_SITE / "reference/option").rglob("*.md")) + [
        ROOT / "tools/cli/README.md",
        DOCS_SITE / "cli.md",
        ROOT / "tools/server/README.md",
        *server_pages,
        OPENAPI_YAML,
        DOCS_SITE / "examples/option-chain.md",
        DOCS_SITE / ".vitepress/theme/components/QueryBuilder.vue",
    ]
    for path in strike_docs:
        expect_not_contains(path, "scaled integer")
        # Word-bounded: capture-backed sample tables legitimately carry
        # timestamps like `34500000` that embed the digit string.
        if re.search(r"\b500000\b", path.read_text()):
            fail(f"{path.relative_to(ROOT)} contains stale text: '500000'")

    expect_contains(
        ROOT / "crates/thetadatadx/endpoint_surface.toml",
        'description = "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"',
    )
    expect_contains(
        ROOT / "tools/cli/README.md",
        "thetadatadx stock at_time_trade AAPL 20240101 20240301 09:30:00.000",
    )
    # The generated at-time pages inherit the same wording from the
    # registry; pin one so a registry rewrite that loses the format
    # note fails here too.
    expect_contains(
        DOCS_SITE / "reference/stock/at-time/trade.md",
        "ET wall-clock time in HH:MM:SS.SSS",
    )
    expect_contains(
        OPENAPI_YAML,
        'description: ET wall-clock time in HH:MM:SS.SSS (e.g. "09:30:00.000" for 9:30 AM ET; legacy "34200000" is also accepted)',
    )
    expect_not_contains(
        ROOT / "tools/cli/README.md",
        '34200000   # 9:30 AM',
    )

    # Wave K replaced the per-tick subscribe_* family with the polymorphic
    # subscribe(Subscription) entry point that takes a typed value built
    # via Contract.option(...).quote() / .trade() / .open_interest(). The
    # README now documents the new shape; the assertion below pins it.
    expect_contains(
        ROOT / "sdks/python/README.md",
        "`Contract.option(symbol, *, expiration, strike, right)`",
    )
    # Streaming pages (hand-written guides + generated stream-type pages)
    # must never reference removed or internal delivery APIs. Every
    # binding exposes `start_streaming(callback)` as the sole delivery
    # path; pin that contract.
    streaming_pages = sorted((DOCS_SITE / "streaming").rglob("*.md"))
    if len(streaming_pages) < 10:
        fail(
            f"expected the streaming section to hold the 3 guide pages plus the "
            f"generated stream-type pages; found {len(streaming_pages)}"
        )
    for streaming_page in streaming_pages:
        expect_not_contains(streaming_page, "start_streaming_iter")
        expect_not_contains(streaming_page, "streaming_iter")
        expect_not_contains(streaming_page, "streaming_async")
        expect_not_contains(streaming_page, "startStreamingIter")
        expect_not_contains(streaming_page, "StreamEventPoller")
        expect_not_contains(streaming_page, "EventIterator")
        expect_not_contains(streaming_page, "thetadatadx_streaming_event_iter")
        expect_not_contains(streaming_page, "```go [Go]")
        expect_not_contains(streaming_page, "contract_map")
        expect_not_contains(streaming_page, "contract_lookup")
        expect_not_contains(streaming_page, "SubscribeOptionQuotes")
        expect_not_contains(streaming_page, 'event.kind == "simple"')
        expect_not_contains(streaming_page, "event.event_type")
        expect_not_contains(streaming_page, "StreamEvent::RawData")
        expect_not_contains(streaming_page, "RawData (undecoded fallback)")
        expect_not_contains(streaming_page, "ring-reader thread")
        expect_not_contains(streaming_page, "subscribe_option_")
        expect_not_contains(streaming_page, "subscribe_quotes")
        expect_not_contains(streaming_page, "subscribe_trades")
        expect_not_contains(streaming_page, "subscribe_full_trades")
    expect_contains(
        DOCS_SITE / "streaming/reliability.md",
        "Caller-driven recovery is always available: `reconnect()`",
    )
    # Same streaming-API guards apply to interactive Vue components under the
    # VitePress theme. Code samples embedded in recipe builders deploy to the
    # public docs site on every push to main; a dead-API reference there
    # ships broken paste-and-run examples to readers.
    vue_components_dir = DOCS_SITE / ".vitepress/theme/components"
    for vue_file in sorted(vue_components_dir.rglob("*.vue")):
        expect_not_contains(vue_file, "start_streaming_iter")
        expect_not_contains(vue_file, "streaming_iter")
        expect_not_contains(vue_file, "streaming_async")
        expect_not_contains(vue_file, "startStreamingIter")
        expect_not_contains(vue_file, "EventIterator")
        expect_not_contains(vue_file, "StreamEventPoller")
        expect_not_contains(vue_file, "thetadatadx_streaming_event_iter")


def endpoint_page_path(endpoint: dict) -> Path:
    """Mirror of the generator's path rule: REST path, hyphenated."""
    rest = endpoint["rest_path"].removeprefix("/v3/")
    return DOCS_SITE / "reference" / (rest.replace("_", "-") + ".md")


def check_reference_pages() -> None:
    """One generated page per registry endpoint, no stale extras, fixed anatomy."""
    expected: dict[Path, str] = {
        endpoint_page_path(ep): ep["name"] for ep in REGISTRY_ENDPOINTS
    }
    for path, name in sorted(expected.items()):
        if not path.is_file():
            fail(
                f"missing generated reference page for endpoint {name}: "
                f"{path.relative_to(ROOT)} — run the docs generator"
            )
        text = path.read_text()
        for marker in ("@generated", "<SdkTabs>", "## Parameters", "## Response"):
            if marker not in text:
                fail(f"{path.relative_to(ROOT)} missing page-anatomy marker {marker!r}")
        if not re.search(r'<TierBadge tier="(free|value|standard|professional)" />', text):
            fail(f"{path.relative_to(ROOT)} missing or malformed <TierBadge>")

    actual = {p for p in (DOCS_SITE / "reference").rglob("*.md") if p.name != "index.md"}
    extra = sorted(p.relative_to(ROOT) for p in actual - set(expected))
    if extra:
        fail(
            "stale reference pages with no matching registry endpoint "
            f"(delete or regenerate): {', '.join(str(p) for p in extra)}"
        )


def site_page_url(md_path: Path) -> str:
    rel = md_path.relative_to(DOCS_SITE).as_posix().removesuffix(".md")
    if rel == "index":
        return "/"
    if rel.endswith("/index"):
        return "/" + rel.removesuffix("index")
    return "/" + rel


def check_llms_txt() -> None:
    """`llms.txt` lists every page on the site and names no deleted page."""
    llms_path = DOCS_SITE / "public/llms.txt"
    if not llms_path.is_file():
        fail("docs-site/docs/public/llms.txt missing — run the docs generator")
    listed = {
        line.split(" — ", 1)[0].strip()
        for line in llms_path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    }
    on_disk = {
        site_page_url(p)
        for p in DOCS_SITE.rglob("*.md")
        if ".vitepress" not in p.parts and "node_modules" not in p.parts
    }
    missing = sorted(on_disk - listed)
    stale = sorted(listed - on_disk)
    if missing or stale:
        fail(
            "docs-site/docs/public/llms.txt drifted from the page tree. "
            f"missing={missing or '[]'} stale={stale or '[]'} — run the docs generator"
        )


def check_openapi() -> None:
    text = OPENAPI_YAML.read_text()
    actual_paths = {
        match.group(1)
        for match in re.finditer(r"^  (/[A-Za-z0-9_/-]+):\s*$", text, re.MULTILINE)
    }
    # Server-specific paths are expected in our OpenAPI (they document
    # functionality the thetadatadx-server binary exposes) but are NOT in
    # the upstream endpoint registry. Filter them out before comparing to
    # `REST_PATHS` so the drift check only fires on real upstream drift.
    effective_actual_paths = actual_paths - SERVER_SPECIFIC_PATHS
    if effective_actual_paths != REST_PATHS:
        missing = sorted(REST_PATHS - effective_actual_paths)
        extra = sorted(effective_actual_paths - REST_PATHS)
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} path set drifted. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    actual_ops = {
        match.group(1) for match in re.finditer(r"^\s*operationId:\s*(\S+)", text, re.MULTILINE)
    }
    expected_ops = {lower_camel(ep["name"]) for ep in REGISTRY_ENDPOINTS}
    if actual_ops != expected_ops:
        missing = sorted(expected_ops - actual_ops)
        extra = sorted(actual_ops - expected_ops)
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} operationId set drifted. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )


def extract_struct_fields(path: Path, struct_pattern: str, field_pattern: str) -> set[str]:
    text = path.read_text()
    match = re.search(struct_pattern, text, re.DOTALL)
    if not match:
        fail(f"{path.relative_to(ROOT)} missing expected struct pattern: {struct_pattern!r}")
    return set(re.findall(field_pattern, match.group(1), re.MULTILINE))


def check_endpoint_option_surface() -> None:
    rust_fields = extract_struct_fields(
        ROOT / "ffi/src/endpoint_request_options.rs",
        r"pub struct ThetaDataDxEndpointRequestOptions \{(.*?)\n\}",
        r"^\s*pub\s+([a-z_]+)\s*:",
    )
    # Exclude has_* sentinel flags (FFI implementation detail, not builder params)
    rust_fields = {f for f in rust_fields if not f.startswith("has_")}
    if rust_fields != ALL_OPTION_FIELDS:
        missing = sorted(ALL_OPTION_FIELDS - rust_fields)
        extra = sorted(rust_fields - ALL_OPTION_FIELDS)
        fail(
            "ffi/src/endpoint_request_options.rs endpoint option fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    c_path = ROOT / "sdks/cpp/include/endpoint_request_options.h.inc"
    c_fields = extract_struct_fields(
        c_path,
        r"typedef struct \{(.*?)\n\}\s*ThetaDataDxEndpointRequestOptions;",
        r"^\s*(?:const char\*|int32_t|double|uint64_t)\s+([a-z_]+);",
    )
    # Exclude has_* sentinel flags (FFI implementation detail, not builder params)
    c_fields = {f for f in c_fields if not f.startswith("has_")}
    if c_fields != ALL_OPTION_FIELDS:
        missing = sorted(ALL_OPTION_FIELDS - c_fields)
        extra = sorted(c_fields - ALL_OPTION_FIELDS)
        fail(
            f"{c_path.relative_to(ROOT)} endpoint option fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    cpp_fields = extract_struct_fields(
        ROOT / "sdks/cpp/include/endpoint_options.hpp.inc",
        r"struct EndpointRequestOptions \{(.*?)\n\};",
        r"^\s*std::optional<[^>]+>\s+([a-z_]+);",
    )
    if cpp_fields != ALL_OPTION_FIELDS:
        missing = sorted(ALL_OPTION_FIELDS - cpp_fields)
        extra = sorted(cpp_fields - ALL_OPTION_FIELDS)
        fail(
            "sdks/cpp/include/endpoint_options.hpp.inc EndpointRequestOptions fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    for path in [
        ROOT / "ffi/src/lib.rs",
        ROOT / "sdks/cpp/include/thetadatadx.h",
        ROOT / "sdks/cpp/include/thetadatadx.hpp",
        ROOT / "sdks/cpp/src/thetadatadx.cpp",
        ROOT / "sdks/cpp/README.md",
        DOCS_SITE / "reference/option/history/greeks/eod.md",
    ]:
        expect_not_contains(path, "OptionRequestOptions")
        expect_not_contains(path, "ThetaDataDxOptionRequestOptions")


def check_tier_badges() -> None:
    """Delegate to scripts/check_tier_badges.py so CI catches tier drift."""
    checker = ROOT / "scripts/check_tier_badges.py"
    if not checker.exists():
        fail(f"{checker.relative_to(ROOT)} missing")
    result = subprocess.run(
        [sys.executable, str(checker)],
        cwd=ROOT,
        check=False,
    )
    if result.returncode != 0:
        fail("tier badge check failed (see scripts/check_tier_badges.py output above)")


def main() -> None:
    check_static_docs()
    check_reference_pages()
    check_llms_txt()
    check_openapi()
    check_endpoint_option_surface()
    check_tier_badges()
    print("docs consistency: ok")


if __name__ == "__main__":
    main()
