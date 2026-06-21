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


ROOT = Path(__file__).resolve().parents[2]
SURFACE = tomllib.loads((ROOT / "crates/thetadatadx/endpoint_surface.toml").read_text())
ENDPOINTS = SURFACE["endpoints"]
TEMPLATES = SURFACE["templates"]
ENDPOINT_NAMES = {ep["name"] for ep in ENDPOINTS}
# The OpenAPI `servers` block points at the HTTP server root
# (`http://localhost:25503`), so each documented path must carry the full
# served route (the registry `rest_path` verbatim, `/v3/...`) for a
# generated client to resolve it. Compare the registry paths as-is.
REST_PATHS = {ep["rest_path"] for ep in ENDPOINTS}

# The single server a generated client targets: the local HTTP server root.
# `tools/server` listens on this port by default (`tools/server/src/main.rs`),
# and the documented paths are the `/v3/...` routes served there. The block
# must name an `http(s)` URL on this host; a backend scheme/host (a `grpc://`
# URL, or an `mdds`/private-backend host) is a request a generated client
# cannot issue and must trip the gate.
OPENAPI_SERVER_URL = "http://localhost:25503"

DOCS_SITE = ROOT / "docs-site/docs"
OPENAPI_YAML = DOCS_SITE / "public/thetadatadx.yaml"

# The Rust source that is the single source of truth for the flat-file served
# matrix: the `(SecType, ReqType)` pairs the distribution serves
# (`SERVED_DATASETS`), plus the client-facing tokens those variants render to
# (`SecType::as_wire` lower-cased and `ReqType::as_str`). The OpenAPI flat-file
# enums must match this matrix exactly, so the gate derives the expected matrix
# from this file rather than carrying a hand-maintained copy that could drift.
FLATFILE_TYPES_RS = ROOT / "crates/thetadatadx/src/flatfiles/types.rs"

# Server source files that register the `/v3` HTTP routes. The server is the
# source of truth for the served route set, so the gate parses the `.route(...)`
# literals from these files rather than carrying a hand-maintained duplicate
# list that could silently drift from what the binary actually serves.
SERVER_ROUTER_FILES = (
    ROOT / "tools/server/src/router.rs",
    ROOT / "tools/server/src/flatfile_routes.rs",
)

# `.route("/v3/...")` literal. The path string may sit on the same line as
# `.route(` or wrap onto the next, so `\s*` (which spans newlines) bridges the
# two. Restricting the capture to `/v3/...` naturally excludes any non-served
# test-only route (e.g. a `/probe` probe router under `#[cfg(test)]`).
ROUTE_LITERAL_RE = re.compile(r"\.route\(\s*\"(/v3/[^\"]*)\"")


def served_v3_routes() -> set[str]:
    """The full `/v3` route set the server binary actually serves.

    Parsed from the server router source (`SERVER_ROUTER_FILES`), so a route
    added to or removed from the binary moves this set with it and the OpenAPI
    path-set assertion below tracks the real surface with no edit to this gate.
    Axum path params use `{name}` braces, matching the OpenAPI path templating.
    """
    routes: set[str] = set()
    for path in SERVER_ROUTER_FILES:
        routes |= set(ROUTE_LITERAL_RE.findall(path.read_text()))
    return routes


# Routes the server serves beyond the upstream-tracking registry endpoints:
# the system status / lifecycle routes and the flat-file download routes. These
# are documented in the OpenAPI contract (they are real served routes) but are
# not in `REST_PATHS`. Derived, not hand-listed, so it cannot drift.
SERVER_ONLY_PATHS = served_v3_routes() - REST_PATHS

# operationIds for the server-only routes documented in the OpenAPI contract.
# The upstream endpoints derive their operationId from the registry name; the
# server-only routes carry hand-authored operationIds, pinned here so the
# operationId-set equality below neither rejects them nor lets an unrelated id
# slip in. `/v3/system/shutdown` carries no operationId (it never has), so it
# contributes none.
SERVER_ONLY_OPERATION_IDS = {
    "systemStatus",
    "systemMddsStatus",
    "systemFpssStatus",
    "flatfileGet",
    "flatfileRequest",
}
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
    # major; the version-sync gate (`scripts/ci/check_version_sync.py`)
    # enforces this against `crates/thetadatadx/Cargo.toml` canonically.
    expect_contains(
        DOCS_SITE / "articles/getting-started.md",
        'thetadatadx = "13.0.0-rc.1"',
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

    # Strikes are dollars on every client-facing surface, with no
    # exception. The WebSocket subscribe envelope takes the strike in
    # dollars exactly like the SDKs; the scaled-integer wire form never
    # surfaces in the docs. The thousandths vocabulary (and the literal
    # 570000 example it travelled with) must never reappear: a client
    # who copies a thousandths example subscribes to a $570,000 strike.
    streaming_option_pages = sorted(
        (DOCS_SITE / "streaming/options").glob("*.md")
    )
    strike_docs = list((DOCS_SITE / "reference/option").rglob("*.md")) + [
        ROOT / "tools/cli/README.md",
        DOCS_SITE / "cli.md",
        ROOT / "tools/server/README.md",
        *server_pages,
        *streaming_option_pages,
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
    # The thousandths strike claim is the exact defect a contributor
    # caught; ban its vocabulary and literal example from every page
    # that carries a WS subscribe envelope.
    for path in [*server_pages, *streaming_option_pages, DOCS_SITE / "articles/symbology.md"]:
        expect_not_contains(path, "thousandths")
        if re.search(r"\b570000\b", path.read_text()):
            fail(f"{path.relative_to(ROOT)} contains stale strike text: '570000'")

    # The interactive query builder generates copy-paste Python and Rust
    # snippets. The emitted client identifier must be the symbol the SDK
    # actually exports — Python `from thetadatadx import Client` /
    # `Client(...)`, Rust `use thetadatadx::{Client, ...}` /
    # `Client::connect(...)`. `ThetaDataDxClient` is only the C-ABI
    # opaque-handle name; it is not a Python or Rust symbol, so a generated
    # Python/Rust snippet that names it does not compile. Pin the real
    # names and forbid the handle name in this component (which carries no
    # C-ABI branch).
    query_builder = DOCS_SITE / ".vitepress/theme/components/QueryBuilder.vue"
    expect_contains(query_builder, "from thetadatadx import Client")
    expect_contains(query_builder, "client = Client(creds, Config.production())")
    expect_contains(query_builder, "use thetadatadx::{Client, Credentials, DirectConfig};")
    expect_contains(query_builder, "Client::connect(&creds, DirectConfig::production())")
    expect_not_contains(query_builder, "ThetaDataDxClient")

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


def _openapi_server_urls(text: str) -> list[str]:
    """Return the `url:` values under the top-level `servers:` block.

    Reads from the `servers:` key to the next top-level key (a line starting in
    column zero), collecting each `- url: <value>` entry in order. Keeps the
    parse local to the block so a `url:` under `info`/`contact`/`license` never
    leaks in.
    """
    lines = text.splitlines()
    urls: list[str] = []
    in_block = False
    for line in lines:
        if not in_block:
            if re.match(r"^servers:\s*$", line):
                in_block = True
            continue
        # A new top-level key (no indentation) ends the servers block.
        if line and not line[0].isspace():
            break
        m = re.match(r"\s*-\s*url:\s*(\S+)", line)
        if m:
            urls.append(m.group(1).strip())
    return urls


def check_openapi() -> None:
    text = OPENAPI_YAML.read_text()

    # The `servers` block is the base URL a generated client prepends to every
    # documented path. It must name the local HTTP server root; a missing /
    # empty block, or a backend scheme/host (a `grpc://` URL or an
    # `mdds`/private-backend host), yields a contract a client cannot call.
    server_urls = _openapi_server_urls(text)
    if not server_urls:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} has no `servers:` entry. "
            f"Declare the local HTTP server: {OPENAPI_SERVER_URL}"
        )
    for url in server_urls:
        scheme = url.split("://", 1)[0].lower() if "://" in url else ""
        if scheme not in {"http", "https"}:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} servers url {url!r} is not an "
                f"http(s) endpoint a generated client can call. Use the local "
                f"HTTP server: {OPENAPI_SERVER_URL}"
            )
        host = url.split("://", 1)[1].split("/", 1)[0].lower()
        if "mdds" in host:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} servers url {url!r} points at a "
                f"backend host. Use the local HTTP server: {OPENAPI_SERVER_URL}"
            )
    if OPENAPI_SERVER_URL not in server_urls:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} servers block is missing the local "
            f"HTTP server url {OPENAPI_SERVER_URL!r}; found {server_urls}"
        )

    # OpenAPI path keys, including templated segments (`{sec_type}`): the brace
    # characters must be in the class or the flat-file path is invisible to the
    # gate, the gap that let a server-only route go undocumented.
    actual_paths = {
        match.group(1)
        for match in re.finditer(r"^  (/[A-Za-z0-9_/{}-]+):\s*$", text, re.MULTILINE)
    }
    # The contract must document the FULL served route set: the upstream-tracking
    # registry endpoints plus every server-only `/v3` route the binary serves
    # (system status / lifecycle + flat-file downloads), derived from the server
    # router source. A route the binary serves but the spec omits leaves a
    # generated client unable to call it; a spec path the binary does not serve
    # is a dangling contract. Either direction trips.
    expected_paths = REST_PATHS | SERVER_ONLY_PATHS
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} path set drifted from the served "
            f"route set. missing={missing or '[]'} extra={extra or '[]'}"
        )

    actual_ops = {
        match.group(1) for match in re.finditer(r"^\s*operationId:\s*(\S+)", text, re.MULTILINE)
    }
    # Registry endpoints derive their operationId from the registry name; the
    # server-only routes carry their pinned hand-authored ids. The full set must
    # match exactly so neither a dropped endpoint id nor a stray id slips by.
    expected_ops = {
        lower_camel(ep["name"]) for ep in REGISTRY_ENDPOINTS
    } | SERVER_ONLY_OPERATION_IDS
    if actual_ops != expected_ops:
        missing = sorted(expected_ops - actual_ops)
        extra = sorted(actual_ops - expected_ops)
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} operationId set drifted. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    # No global per-request security scheme. The server authenticates its
    # upstream connection once at startup; request paths carry no per-request
    # credential. The only allowed requirement is the route-scoped shutdown
    # token on POST /v3/system/shutdown. A top-level `security:` block (a
    # document-wide default applied to every operation) is a contract for a
    # credential the server never reads, so it must not reappear.
    if re.search(r"(?m)^security:\s*$", text):
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} declares a global `security:` block. "
            f"The server reads no per-request credential; keep only the "
            f"route-scoped shutdown token on POST /v3/system/shutdown."
        )


def extract_struct_fields(path: Path, struct_pattern: str, field_pattern: str) -> set[str]:
    text = path.read_text()
    match = re.search(struct_pattern, text, re.DOTALL)
    if not match:
        fail(f"{path.relative_to(ROOT)} missing expected struct pattern: {struct_pattern!r}")
    return set(re.findall(field_pattern, match.group(1), re.MULTILINE))


def _rust_match_arm_map(body: str, lhs_variant: str) -> dict[str, str]:
    """Parse `Self::Variant => "token",` arms into a `{Variant: token}` map.

    `body` is the source slice holding the match arms (e.g. the body of a
    `fn as_str` / `fn as_wire`). `lhs_variant` captures the variant identifier
    after `Self::`. The result keys the Rust variant to the string literal it
    renders to, so the gate maps `SERVED_DATASETS` entries to the client-facing
    tokens the OpenAPI enums carry without hand-coding either side.
    """
    return {
        m.group(1): m.group(2)
        for m in re.finditer(
            rf'Self::({lhs_variant})\s*=>\s*"([^"]+)"', body
        )
    }


def flatfile_served_matrix() -> dict[str, set[str]]:
    """Derive `{sec_type_token: {req_type_token, ...}}` from the Rust source.

    Parses `SERVED_DATASETS` (the `(SecType::X, ReqType::Y)` pairs the flat-file
    distribution serves) and the variant-to-token maps from `SecType::as_wire`
    and `ReqType::as_str`, all in `FLATFILE_TYPES_RS`. The sec_type token is the
    lower-cased `as_wire` value (`OPTION` -> `option`), matching the OpenAPI
    flat-file path/enum spelling; the req_type token is the `as_str` value
    verbatim. A single source means a served-matrix change in the Rust enum
    moves the expected matrix here with no edit to this gate.
    """
    text = FLATFILE_TYPES_RS.read_text()

    # Variant -> token maps from the two `as_*` methods. Restrict each search to
    # the method body so unrelated `Self::X => ...` arms (e.g. Display) are not
    # swept in.
    sec_body = re.search(
        r"fn as_wire\(self\) -> &'static str \{(.*?)\n    \}", text, re.DOTALL
    )
    req_body = re.search(
        r"fn as_str\(self\) -> &'static str \{(.*?)\n    \}", text, re.DOTALL
    )
    if not sec_body or not req_body:
        fail(
            f"{FLATFILE_TYPES_RS.relative_to(ROOT)} missing SecType::as_wire / "
            f"ReqType::as_str bodies the flat-file matrix gate parses"
        )
    sec_token = {
        variant: wire.lower()
        for variant, wire in _rust_match_arm_map(
            sec_body.group(1), r"Option|Stock|Index"
        ).items()
    }
    req_token = _rust_match_arm_map(
        req_body.group(1), r"Eod|Quote|OpenInterest|Ohlc|Trade|TradeQuote"
    )

    # The served pairs themselves. `SERVED_DATASETS` is a `&[(SecType::_,
    # ReqType::_)]` literal; capture each `(SecType::A, ReqType::B)` tuple.
    served_block = re.search(
        r"pub const SERVED_DATASETS:[^=]*=\s*&\[(.*?)\];", text, re.DOTALL
    )
    if not served_block:
        fail(
            f"{FLATFILE_TYPES_RS.relative_to(ROOT)} missing the SERVED_DATASETS "
            f"slice the flat-file matrix gate parses"
        )
    pairs = re.findall(
        r"\(\s*SecType::(\w+)\s*,\s*ReqType::(\w+)\s*\)", served_block.group(1)
    )
    if not pairs:
        fail(
            f"{FLATFILE_TYPES_RS.relative_to(ROOT)} SERVED_DATASETS parsed to no "
            f"(SecType, ReqType) pairs"
        )

    matrix: dict[str, set[str]] = {}
    for sec_variant, req_variant in pairs:
        if sec_variant not in sec_token:
            fail(f"SERVED_DATASETS names SecType::{sec_variant} with no as_wire token")
        if req_variant not in req_token:
            fail(f"SERVED_DATASETS names ReqType::{req_variant} with no as_str token")
        matrix.setdefault(sec_token[sec_variant], set()).add(req_token[req_variant])
    return matrix


def _yaml_block(text: str, header_re: str, *, after: int = 0) -> tuple[str, int]:
    """Return the indented body under the first `header_re` line at/after `after`.

    The body runs from the header line to the next line indented at or below the
    header's own indentation (or end of file). Used to scope an enum/oneOf parse
    to a single OpenAPI node so a later same-named key cannot bleed in. Returns
    the body text and the absolute offset where it ends.
    """
    m = re.search(header_re, text[after:], re.MULTILINE)
    if not m:
        return "", len(text)
    start = after + m.start()
    header_indent = len(m.group(0)) - len(m.group(0).lstrip())
    lines = text[start:].splitlines(keepends=True)
    out: list[str] = [lines[0]]
    pos = start + len(lines[0])
    for line in lines[1:]:
        if line.strip() and (len(line) - len(line.lstrip())) <= header_indent:
            break
        out.append(line)
        pos += len(line)
    return "".join(out), pos


def _enum_values(block: str) -> list[str]:
    """The inline-list `enum: [a, b, c]` values in `block`, stripped of quotes."""
    m = re.search(r"enum:\s*\[([^\]]*)\]", block)
    if not m:
        return []
    return [v.strip().strip("'\"") for v in m.group(1).split(",") if v.strip()]


def check_flatfile_matrix() -> None:
    """OpenAPI flat-file enums must equal the `SERVED_DATASETS` served matrix.

    The path form (`/v3/flatfile/{sec_type}/{req_type}`) takes the two segments
    independently, so its `sec_type` enum must be the served security types and
    its `req_type` enum the union of every served request type. The body form
    (`POST /v3/flatfile/request`) constrains the served pairs per security type
    through a `oneOf`, so each branch must pin one `sec_type` to exactly that
    type's served request types. A served-matrix drift in the checked-in spec
    in either direction fails the gate.
    """
    matrix = flatfile_served_matrix()
    expected_secs = set(matrix)
    expected_req_union = set().union(*matrix.values())

    text = OPENAPI_YAML.read_text()

    # --- Path form: /v3/flatfile/{sec_type}/{req_type} ----------------------
    path_block, _ = _yaml_block(
        text, r"^  /v3/flatfile/\{sec_type\}/\{req_type\}:\s*$"
    )
    if not path_block:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} missing the flat-file path "
            f"/v3/flatfile/{{sec_type}}/{{req_type}}"
        )
    sec_param, _ = _yaml_block(path_block, r"^        - name: sec_type\s*$")
    req_param, _ = _yaml_block(path_block, r"^        - name: req_type\s*$")
    path_secs = set(_enum_values(sec_param))
    path_reqs = set(_enum_values(req_param))
    if path_secs != expected_secs:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} flat-file path sec_type enum "
            f"{sorted(path_secs)} != served security types {sorted(expected_secs)}"
        )
    if path_reqs != expected_req_union:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} flat-file path req_type enum "
            f"{sorted(path_reqs)} != union of served request types "
            f"{sorted(expected_req_union)}"
        )

    # --- Body form: POST /v3/flatfile/request oneOf branches ----------------
    req_path_block, _ = _yaml_block(text, r"^  /v3/flatfile/request:\s*$")
    if not req_path_block:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} missing the flat-file body path "
            f"/v3/flatfile/request"
        )
    oneof_block, _ = _yaml_block(req_path_block, r"^              oneOf:\s*$")
    if not oneof_block:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} POST /v3/flatfile/request has no "
            f"oneOf constraining the served (sec_type, req_type) pairs"
        )
    # Each branch begins at a `- title:` list item; split on those markers.
    branch_starts = [m.start() for m in re.finditer(r"^                - ", oneof_block, re.MULTILINE)]
    if not branch_starts:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} flat-file request oneOf has no "
            f"branches"
        )
    branch_bounds = branch_starts + [len(oneof_block)]
    body_matrix: dict[str, set[str]] = {}
    for i in range(len(branch_starts)):
        branch = oneof_block[branch_bounds[i] : branch_bounds[i + 1]]
        sec_prop, _ = _yaml_block(branch, r"^                    sec_type:\s*$")
        req_prop, _ = _yaml_block(branch, r"^                    req_type:\s*$")
        secs = _enum_values(sec_prop)
        reqs = set(_enum_values(req_prop))
        if len(secs) != 1:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} flat-file request oneOf branch "
                f"must pin exactly one sec_type; got {secs}"
            )
        sec = secs[0]
        if sec in body_matrix:
            fail(
                f"{OPENAPI_YAML.relative_to(ROOT)} flat-file request oneOf has "
                f"duplicate branch for sec_type {sec!r}"
            )
        body_matrix[sec] = reqs
    if body_matrix != matrix:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} flat-file request oneOf matrix "
            f"{ {k: sorted(v) for k, v in body_matrix.items()} } != served matrix "
            f"{ {k: sorted(v) for k, v in matrix.items()} }"
        )


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
    """Delegate to scripts/ci/check_tier_badges.py so CI catches tier drift."""
    checker = ROOT / "scripts/ci/check_tier_badges.py"
    if not checker.exists():
        fail(f"{checker.relative_to(ROOT)} missing")
    result = subprocess.run(
        [sys.executable, str(checker)],
        cwd=ROOT,
        check=False,
    )
    if result.returncode != 0:
        fail("tier badge check failed (see scripts/ci/check_tier_badges.py output above)")


def main() -> None:
    check_static_docs()
    check_reference_pages()
    check_llms_txt()
    check_openapi()
    check_flatfile_matrix()
    check_endpoint_option_surface()
    check_tier_badges()
    print("docs consistency: ok")


if __name__ == "__main__":
    main()
