#!/usr/bin/env python3
"""Validate checked-in documentation against the endpoint surface spec.

This script keeps the human-facing docs aligned with the current SDK and REST
surface by checking a few high-signal invariants:

- endpoint/tool counts in top-level docs, each derived from the list it heads
- REST/OpenAPI path + operationId parity with `endpoint_surface.toml`
- per-route OpenAPI parameter names, required flags and defaults matching
  the same registry (a generated client must not demand a parameter the
  server treats as optional, nor hide the value an omitted one takes)
- each option route's documented `expiration` pattern accepting the `*`
  wildcard exactly where the pinned vendor spec accepts it
- every route's `x-code-examples` being the call the registry's signature
  actually takes, rendered from the registry rather than kept by hand
  (`--write-examples` rewrites them)
- one generated docs-site reference page per registry endpoint (and no
  stale extras), each carrying the fixed page anatomy markers
- `llms.txt` covering every page on the site (and naming no deleted page)
- current server docs staying on the v3 path scheme and current defaults
- `CHANGELOG.md` and `docs-site/docs/changelog.md` staying byte-identical
  (the docs-site build re-publishes whatever is on `main`, so any drift
  is a release-note bug the moment it lands)

The generated reference tree itself is byte-verified by
`cargo run -p thetadatadx-rs --features config-file,__internal --bin generate_docs_site -- --check`
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
SURFACE = tomllib.loads((ROOT / "thetadatadx-rs/endpoint_surface.toml").read_text())
ENDPOINTS = SURFACE["endpoints"]
TEMPLATES = SURFACE["templates"]
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

# The pinned snapshot of the vendor's own v3 spec, the source of truth for
# which option endpoints accept `expiration=*`. Upstream models the two cases
# as separate components (`expiration` / `expiration_no_star`) and the Rust
# generator already derives `supports_expiration_wildcard` per endpoint from
# this file for the live validator's wildcard cells
# (`thetadatadx-rs/build_support_bin/upstream_openapi.rs`). Our published
# contract makes the same distinction by hand, with one anchor per case, so
# the gate derives the expected answer from the same snapshot rather than
# trusting 30-odd hand-written choices to stay right.
UPSTREAM_OPENAPI_YAML = ROOT / "scripts/ci/data/upstream_openapi.yaml"

# The Rust source that is the single source of truth for the flat-file served
# matrix: the `(SecType, ReqType)` pairs the distribution serves
# (`SERVED_DATASETS`), plus the client-facing tokens those variants render to
# (`SecType::as_wire` lower-cased and `ReqType::as_str`). The OpenAPI flat-file
# enums must match this matrix exactly, so the gate derives the expected matrix
# from this file rather than carrying a hand-maintained copy that could drift.
FLATFILE_TYPES_RS = ROOT / "thetadatadx-rs/src/flatfiles/types.rs"

# The MCP server source is the single source of truth for the tool surface a
# connected `tools/list` returns: the registry market-data endpoints (one tool
# per endpoint), the offline utilities (`OFFLINE_TOOL_NAMES` in `main.rs`), and
# the flat-file tools advertised by `push_flatfile_tool_definitions` in
# `flatfile_tools.rs`. The gate parses the non-registry tool names from these
# files so a tool added in code but left out of the docs trips the gate rather
# than the docs carrying a hand-maintained count that silently drifts.
MCP_MAIN_RS = ROOT / "tools/mcp/src/main.rs"
MCP_FLATFILE_TOOLS_RS = ROOT / "tools/mcp/src/flatfile_tools.rs"
MCP_UTILITIES_RS = ROOT / "tools/mcp/src/utilities.rs"

# Docs that must enumerate the connection-only MCP tools by name, each paired
# with the `##` heading of the section that carries the tool listing. The
# enumeration is checked inside that section only: a tool name mentioned in
# unrelated prose elsewhere in the file (an intro blockquote, an example) must
# not mask a missing entry in the actual tool list, so removing a tool from the
# listing trips the gate even when the name survives elsewhere.
MCP_DOC_TOOL_SECTIONS = (
    (ROOT / "tools/mcp/README.md", "## Available Tools"),
    (DOCS_SITE / "mcp.md", "## Tools"),
)

# Server source files that register the `/v3` HTTP routes. The server is the
# source of truth for the served route set, so the gate parses the `.route(...)`
# literals from these files rather than carrying a hand-maintained duplicate
# list that could silently drift from what the binary actually serves.
SERVER_ROUTER_FILES = (
    ROOT / "tools/server/src/router.rs",
    ROOT / "tools/server/src/flatfile_routes.rs",
)

# The server CLI is the source of truth for the documented flag defaults.
# The `#[arg(...)]` attributes on the `struct Args` fields carry each
# flag's default; the docs render those in a `| --flag | default | ... |`
# Markdown table. The gate parses the `#[arg(...)]` defaults from this file
# and asserts the doc-table rows match value-by-value, so a changed server
# default that the docs don't reflect trips the gate (the same
# derive-from-source pattern as `flatfile_served_matrix`). A substring
# check ("25503" appears somewhere) could not catch a default that changed
# to a value still mentioned elsewhere on the page.
SERVER_MAIN_RS = ROOT / "tools/server/src/main.rs"

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


# The server mirrors the JVM terminal's system surface 1:1, so every served
# `/v3/terminal/*` route is documented verbatim in the OpenAPI contract — the
# terminal publishes these exact paths (docs.thetadata.us) and the server is a
# drop-in for it. Nothing is served-but-undocumented here, so this set is empty.
OPENAPI_UNDOCUMENTED_PARITY_ALIASES: set[str] = set()

# Routes the server serves beyond the upstream-tracking registry endpoints:
# the system status / lifecycle routes and the flat-file download routes. These
# are documented in the OpenAPI contract (they are real served routes) but are
# not in `REST_PATHS`. Derived, not hand-listed, so it cannot drift. The
# vendor-parity aliases above are subtracted: they are served but intentionally
# undocumented (their documented equivalents carry the identical behaviour).
SERVER_ONLY_PATHS = (
    served_v3_routes() - REST_PATHS - OPENAPI_UNDOCUMENTED_PARITY_ALIASES
)

# operationIds for the server-only routes documented in the OpenAPI contract.
# The upstream endpoints derive their operationId from the registry name; the
# server-only routes carry hand-authored operationIds, pinned here so the
# operationId-set equality below neither rejects them nor lets an unrelated id
# slip in. `/v3/system/shutdown` carries no operationId (it never has), so it
# contributes none.
SERVER_ONLY_OPERATION_IDS = {
    # Terminal system routes, mirrored 1:1 from the JVM terminal: an
    # unauthenticated shutdown plus the two plain-text channel-health probes.
    # The route paths carry the vendor's codenames verbatim; the operationIds
    # describe the channel (streaming / market-data) to keep generated client
    # method names free of the transport codename.
    "terminalShutdown",
    "terminalStreamingStatus",
    "terminalMarketDataStatus",
    # Path-segment / renamed `/v3` aliases mounted at the server level for
    # endpoints the registry already exposes under a query-form or shorter
    # path. Each dispatches to the same registry endpoint as its sibling but
    # is a distinct OpenAPI operation, so it carries its own id rather than
    # reusing the sibling's (operationIds must be unique).
    "stockListDatesByRequestType",
    "optionListDatesByRequestType",
    "optionListContractsByRequestType",
    "calendarToday",
    "calendarYearHolidays",
    "interestRateHistoryEodPath",
    "flatfileGetBySecType",
}
# Query params the OpenAPI contract documents that the endpoint registry does
# not model. `format` is a server-side rendering knob (`tools/server`'s
# `parse_response_format`), not an upstream request parameter, so it has no
# registry row and must not read as parameter drift.
OPENAPI_REST_ONLY_PARAMS = {"format"}

# Server-only `/v3` route -> the registry endpoint it dispatches to, from
# `register_v3_path_routes` in `tools/server/src/router.rs`. Each of these is
# a second spelling of an endpoint the registry already serves (the terminal's
# `{request_type}` path-segment forms and its three renamed routes), handled by
# the same code with the same parameters, so the parameter check holds them to
# the same registry row as their sibling. The `{request_type}` forms move that
# one parameter from the query string into the path, which the check reads as
# the required, default-less parameter the registry declares.
OPENAPI_ALIAS_ENDPOINTS = {
    "/v3/stock/list/dates/{request_type}": "stock_list_dates",
    "/v3/option/list/dates/{request_type}": "option_list_dates",
    "/v3/option/list/contracts/{request_type}": "option_list_contracts",
    "/v3/calendar/today": "calendar_open_today",
    "/v3/calendar/year_holidays": "calendar_year",
    "/v3/interest_rate/history/eod": "interest_rate_history_eod",
}

# Builder-bound params (the optional fluent setters that materialize as fields
# on the `*EndpointRequestOptions` structs) come from two places in the surface:
# the reusable `[param_groups.*]` definitions AND inline `[[endpoints.params]]`
# entries that declare a param directly (a `name = ...` row) rather than
# referencing a group (`use = ...`). An endpoint that needs a one-off optional
# param (e.g. an optional `symbol` filter on a single listing endpoint) declares
# it inline, so collecting only the group params would let that field escape the
# FFI/SDK option-struct parity check. Both sources feed `BUILDER_PARAMS`; the
# `use = ...` references are already covered via their group, so only inline
# `name`-carrying params are read here.
def _builder_param_names() -> set[str]:
    names = {
        param["name"]
        for group in SURFACE["param_groups"].values()
        for param in group.get("params", [])
        if param.get("binding") == "builder"
    }
    for endpoint in SURFACE.get("endpoints", []):
        for param in endpoint.get("params", []):
            if "name" in param and param.get("binding") == "builder":
                names.add(param["name"])
    return names


BUILDER_PARAMS = _builder_param_names()

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


def lower_camel(snake: str) -> str:
    head, *tail = snake.split("_")
    return head + "".join(part.capitalize() for part in tail)


def fail(message: str) -> None:
    print(f"docs consistency error: {message}", file=sys.stderr)
    raise SystemExit(1)


def rel(path: Path) -> str:
    """`path` for a message, repo-relative where it can be.

    The self-test points a checked path at a synthetic fixture in a temp
    directory, which `Path.relative_to(ROOT)` refuses; a failure message is
    no place to raise.
    """
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


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
        "MCP server exposing every market-data endpoint to AI clients",
    )

    expect_contains(
        ROOT / "tools/mcp/README.md",
        "## Available Tools",
    )
    # Derive the three counts from the inventory rather than pinning the
    # sentence as a literal. Pinned, the gate required whatever number was
    # current when it was written: adding a tool left it green on a sentence
    # that had become wrong, and correcting the sentence failed it. The
    # heading-based count check below does not cover this line either, since
    # it sits in the intro paragraph before the first subheading.
    inventory = mcp_tool_inventory()
    offline = inventory["offline"]
    flatfile = inventory["flatfile"]
    utility = inventory["utility"]
    expect_contains(
        ROOT / "tools/mcp/README.md",
        (
            f"Every generated market-data endpoint plus {len(offline)} offline "
            f"tool ({plural_names(offline)}) and, when connected, "
            f"{len(flatfile)} flat-file tools and {plural_names(utility)}."
        ),
    )

    expect_contains(
        DOCS_SITE / "mcp.md",
        "Every generated market-data endpoint plus `ping`.",
    )
    # Version strings in getting-started docs must match the canonical
    # workspace version; derive it from `thetadatadx-rs/Cargo.toml`
    # so the pin tracks bumps automatically instead of pinning a literal
    # that silently ages. The version-sync gate
    # (`scripts/ci/check_version_sync.py`) enforces the major separately.
    cargo_version = tomllib.loads(
        (ROOT / "thetadatadx-rs/Cargo.toml").read_text()
    )["package"]["version"]
    expect_contains(
        DOCS_SITE / "articles/getting-started.md",
        f'thetadatadx-rs = "{cargo_version}"',
    )
    expect_contains(
        DOCS_SITE / "mcp.md",
        'Use `"strike":"*"` when you want a bulk chain-style response',
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        'Use `"strike":"*"` when you want a bulk chain-style response',
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
    expect_contains(ROOT / "tools/server/README.md", "/v3/option/snapshot/quote?symbol=")
    expect_contains(DOCS_SITE / "server/index.md", "25503")
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

    # Strike is dollars on the REST surface everywhere: the REST reference
    # pages, the server README, the REST server pages (index / http), and the
    # OpenAPI spec must never show the scaled-integer strike form. A client
    # who copies a `500000` / `570000` thousandths example on a REST surface
    # would subscribe to a $500,000 / $570,000 strike.
    #
    # The server's WebSocket, by contrast, defaults to the terminal's
    # 1/10-cent integer (a `$570` strike is `570000`) and is configurable to
    # dollars via `--strike-format`. So server/websocket.md and the
    # streaming/** docs legitimately show the terminal form and are EXEMPT
    # from the dollars-only strike rule below.
    rest_strike_docs = list((DOCS_SITE / "reference/option").rglob("*.md")) + [
        ROOT / "tools/server/README.md",
        DOCS_SITE / "server/index.md",
        DOCS_SITE / "server/http.md",
        OPENAPI_YAML,
    ]
    for path in rest_strike_docs:
        expect_not_contains(path, "scaled integer")
        # Word-bounded: capture-backed sample tables legitimately carry
        # timestamps like `34500000` that embed the digit string.
        if re.search(r"\b500000\b", path.read_text()):
            fail(f"{path.relative_to(ROOT)} contains stale text: '500000'")
    # Ban the thousandths vocabulary and the literal `570000` example from
    # the dollars-only REST-facing strike pages (the REST server pages and
    # the symbology article). The WebSocket page and the streaming docs are
    # exempt — they show the terminal's 1/10-cent form by default.
    for path in [
        DOCS_SITE / "server/index.md",
        DOCS_SITE / "server/http.md",
        DOCS_SITE / "articles/symbology.md",
    ]:
        expect_not_contains(path, "thousandths")
        if re.search(r"\b570000\b", path.read_text()):
            fail(f"{path.relative_to(ROOT)} contains stale strike text: '570000'")

    expect_contains(
        ROOT / "thetadatadx-rs/endpoint_surface.toml",
        'description = "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"',
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

    # Wave K replaced the per-tick subscribe_* family with the polymorphic
    # subscribe(Subscription) entry point that takes a typed value built
    # via Contract.option(...).quote() / .trade() / .open_interest(). The
    # README now documents the new shape; the assertion below pins it.
    expect_contains(
        ROOT / "thetadatadx-py/README.md",
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
        # The interactive request builder (`<RequestBuilder :cfg="cfg" />`) is the
        # current per-page anatomy; it replaced the static `<SdkTabs>` code block.
        for marker in ("@generated", "<RequestBuilder", "## Parameters", "## Response"):
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
    #
    # The only served routes excluded from `expected_paths` are the two
    # vendor-parity status aliases in `OPENAPI_UNDOCUMENTED_PARITY_ALIASES`
    # (already subtracted out of `SERVER_ONLY_PATHS`): they are served for
    # drop-in parity with the JVM terminal and dispatch to the same handlers as
    # the documented `/v3/terminal/{streaming,historical}/status` routes, while
    # their path segments are banned client-facing vocabulary that must not
    # appear in this public spec. See that constant for the full rationale.
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


def _struct_body(text: str, struct_name: str) -> str | None:
    """Return the `{ ... }` body of `struct <struct_name> { ... }`,
    brace-balanced. Used to scope the `#[arg(...)]` scan to the CLI args
    struct so an `#[arg]` on an unrelated type cannot leak in.
    """
    header = re.search(rf"struct\s+{re.escape(struct_name)}\s*\{{", text)
    if not header:
        return None
    depth = 1
    i = header.end()
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return text[header.end() : i - 1]


def server_arg_defaults() -> dict[str, str | None]:
    """Derive `{--flag: default}` for every server CLI flag from the
    `#[arg(...)]` attributes on `struct Args` in `tools/server/src/main.rs`.

    The default is:

    * the `default_value = "X"` string literal, verbatim;
    * the `default_value_t = N` literal (an integer such as `25503`);
    * for `default_value_t = <path>::Variant` (a clap `ValueEnum`), the
      kebab/lower-cased last path segment — clap renders `LogFormat::Text`
      as `text` with no `#[value(name=...)]` override;
    * `None` when the field carries no `default_value*` (an `Option<T>` or
      `bool` flag), meaning the doc-table default cell must be empty.

    The flag long name is clap's snake→kebab derivation of the field name
    (`http_port` -> `--http-port`). `value_parser = [...]` does not set a
    default and is ignored.
    """
    text = SERVER_MAIN_RS.read_text()
    body = _struct_body(text, "Args")
    if body is None:
        fail(f"{SERVER_MAIN_RS.relative_to(ROOT)} has no `struct Args` body")

    defaults: dict[str, str | None] = {}
    # Each field is an `#[arg(...)]` attribute followed by `<field>: <type>`.
    # The attribute arg list can itself contain `[...]` (a
    # `value_parser = ["production", "dev"]` allow-list) and nested `(...)`,
    # so the inner span is read with a bracket/paren balance counter — a
    # naive `#[arg([^\]]*)]` stops at the first `]` inside the value_parser
    # array and drops the flag (the same inner-delimiter trap the napi
    # parity collectors hit). After the attribute's closing `]`, the next
    # identifier before a `:` is the field name.
    after_field_re = re.compile(r"\s*(?:#\[[^\]]*\]\s*)*([a-z_][a-z0-9_]*)\s*:")
    for m in re.finditer(r"#\[\s*arg\b", body):
        i = m.end()
        while i < len(body) and body[i].isspace():
            i += 1
        attr = ""
        if i < len(body) and body[i] == "(":
            attr, after = _balanced_paren_span(body, i)
            if after is None:
                continue
        else:
            after = i
        # Advance past the attribute's closing `]`.
        while after < len(body) and body[after] != "]":
            after += 1
        after += 1
        fm = after_field_re.match(body, after)
        if not fm:
            continue
        field = fm.group(1)
        flag = "--" + field.replace("_", "-")
        defaults[flag] = _arg_default_from_attr(attr)
    if not defaults:
        fail(
            f"{SERVER_MAIN_RS.relative_to(ROOT)} `struct Args` parsed to no "
            "`#[arg(...)]` flags"
        )
    return defaults


def _balanced_paren_span(text: str, open_idx: int) -> tuple[str, int | None]:
    """Given `open_idx` at a `(`, return `(inner, after)` where `inner` is
    the balanced content up to the matching `)` and `after` is the index
    just past that `)`. Tracks `"..."` string literals (with `\\` escapes)
    and nested `(` / `[` so a `)` or `]` inside a string or a
    `value_parser = [...]` array does not close the span early. Returns
    `("", None)` if unbalanced.
    """
    depth = 0
    in_str = False
    esc = False
    start = open_idx + 1
    i = open_idx
    n = len(text)
    while i < n:
        c = text[i]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[start:i], i + 1
        i += 1
    return "", None


def _arg_default_from_attr(attr: str) -> str | None:
    """Extract the rendered default from one `#[arg(...)]` inner text, or
    `None` when the attribute sets no default.
    """
    # `default_value = "X"` — string literal verbatim.
    m = re.search(r'default_value\s*=\s*"([^"]*)"', attr)
    if m:
        return m.group(1)
    # `default_value_t = <expr>`.
    m = re.search(r"default_value_t\s*=\s*([^,]+)", attr)
    if m:
        expr = m.group(1).strip()
        # A clap `ValueEnum` path (`logging::LogFormat::Text`) renders as
        # the kebab/lower-cased last segment.
        if "::" in expr:
            variant = expr.rsplit("::", 1)[1]
            return _value_enum_render(variant)
        # Otherwise a literal (integer like `25503`, or `true`/`false`).
        return expr
    return None


def _value_enum_render(variant: str) -> str:
    """Render a clap `ValueEnum` PascalCase variant the way clap does by
    default: lower-case, words separated by `-` (`OpenInterest` ->
    `open-interest`, `Text` -> `text`). Matches clap's default
    `to_possible_value` kebab-casing when no `#[value(name=...)]` override
    is present.
    """
    out: list[str] = []
    for i, ch in enumerate(variant):
        if ch.isupper() and i > 0:
            out.append("-")
        out.append(ch.lower())
    return "".join(out)


# Markdown flag-table row: `| --flag[ <metavar>] | default | description |`.
# The first cell may carry a metavar (`--creds <path>`) or pair two flags
# (`--email / --password`); the default cell may be empty or an em-dash
# placeholder. The parser below normalises both.
_FLAG_ROW_RE = re.compile(
    r"^\|\s*(?P<flags>`[^|]+?`(?:\s*/\s*`[^|]+?`)*)\s*\|"
    r"\s*(?P<default>[^|]*?)\s*\|",
    re.MULTILINE,
)
_FLAG_CELL_RE = re.compile(r"`(--[a-z][a-z0-9-]*)(?:\s+[^`]*)?`")


def _parse_doc_flag_table(text: str) -> dict[str, str | None]:
    """Parse a Markdown flags table into `{--flag: default-or-None}`.

    Each flag in the first cell (a row may list two, `--email / --password`)
    maps to the row's default cell. An empty cell or an em-dash / hyphen
    placeholder (`—` / `-`) means "no default" and maps to `None`. The
    default literal is unwrapped from surrounding backticks so it compares
    against the raw `#[arg]` default.
    """
    out: dict[str, str | None] = {}
    for m in _FLAG_ROW_RE.finditer(text):
        flags = _FLAG_CELL_RE.findall(m.group("flags"))
        if not flags:
            continue
        raw_default = m.group("default").strip()
        # Strip backticks and treat placeholders as "no default".
        unwrapped = raw_default.strip("`").strip()
        default: str | None
        if unwrapped in ("", "—", "-", "–"):
            default = None
        else:
            default = unwrapped
        for flag in flags:
            out[flag] = default
    return out


def check_server_flag_defaults() -> None:
    """Assert the server flag-default tables in the docs match the
    `#[arg(...)]` defaults derived from `tools/server/src/main.rs`,
    value-by-value. A substring check could not catch a default that
    silently changed to a value still printed elsewhere on the page.

    Both the server README and the docs-site server page carry the table;
    each is checked. A doc may legitimately omit a flag from its table
    (e.g. a terse page), so the gate asserts agreement only for the flags a
    given table DOES document — but a documented flag whose default
    disagrees with the source, or a documented default for a flag the
    source says has none (and vice versa), trips.
    """
    source_defaults = server_arg_defaults()
    doc_tables = (
        ROOT / "tools/server/README.md",
        DOCS_SITE / "server/index.md",
    )
    for doc in doc_tables:
        if not doc.is_file():
            fail(f"{doc.relative_to(ROOT)}: server flag-table doc not found")
        documented = _parse_doc_flag_table(doc.read_text())
        # The table must document a meaningful subset of the real flags;
        # if it parsed to nothing, the table shape drifted and the gate is
        # silently blind — fail loudly.
        overlap = set(documented) & set(source_defaults)
        if not overlap:
            fail(
                f"{doc.relative_to(ROOT)}: no server flags parsed from the "
                "flag-default table (the table shape changed — the gate would "
                "be blind to default drift)"
            )
        for flag in sorted(overlap):
            want = source_defaults[flag]
            got = documented[flag]
            if want != got:
                fail(
                    f"{doc.relative_to(ROOT)} documents {flag} default as "
                    f"{got!r}, but `tools/server/src/main.rs` `#[arg]` sets "
                    f"{want!r}"
                )


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


def _expand_param_group(params: list[dict], chain: tuple[str, ...] = ()) -> list[dict]:
    """Flatten `[[...params]]` rows, resolving every `use = "<group>"` row.

    A row either declares a param inline (`name = ...`) or references a
    reusable `[param_groups.*]` entry, which may itself reference further
    groups. `chain` is the resolution path, so a group that references itself
    fails by name instead of recursing until the interpreter gives up.
    """
    out: list[dict] = []
    for param in params:
        group_name = param.get("use")
        if group_name is None:
            out.append(param)
            continue
        if group_name in chain:
            fail(
                "endpoint_surface.toml param_group cycle: "
                + " -> ".join((*chain, group_name))
            )
        group = SURFACE["param_groups"].get(group_name)
        if group is None:
            fail(f"endpoint_surface.toml references unknown param group {group_name!r}")
        out.extend(_expand_param_group(group.get("params", []), (*chain, group_name)))
    return out


def _template_params(name: str, chain: tuple[str, ...] = ()) -> list[dict]:
    """The params a `[templates.<name>]` contributes, base template first."""
    if name in chain:
        fail("endpoint_surface.toml template cycle: " + " -> ".join((*chain, name)))
    template = TEMPLATES.get(name)
    if template is None:
        fail(f"endpoint_surface.toml references unknown template {name!r}")
    out: list[dict] = []
    base = template.get("extends")
    if base is not None:
        out.extend(_template_params(base, (*chain, name)))
    out.extend(_expand_param_group(template.get("params", [])))
    return out


def registry_route_params() -> dict[str, dict[str, tuple[bool, str | None]]]:
    """`{rest_path: {param: (required, default)}}` for every registry route.

    The registry is the surface the SDK and `tools/server` actually implement,
    so it is what the published contract has to agree with. Params come from
    the endpoint's template chain plus its own inline rows, both resolved
    through `[param_groups.*]`; a default is the registry literal verbatim
    (`"*"`, `"1s"`, `"true"`), which is how the OpenAPI scalar reads too.

    The server-only path aliases in `OPENAPI_ALIAS_ENDPOINTS` resolve to the
    row of the endpoint they dispatch to.
    """
    resolved: dict[str, dict[str, tuple[bool, str | None]]] = {}
    by_name: dict[str, dict[str, tuple[bool, str | None]]] = {}
    for endpoint in REGISTRY_ENDPOINTS:
        params: list[dict] = []
        template_name = endpoint.get("template")
        if template_name is not None:
            params.extend(_template_params(template_name))
        params.extend(_expand_param_group(endpoint.get("params", [])))
        row = {
            param["name"]: (bool(param.get("required", False)), param.get("default"))
            for param in params
        }
        resolved[endpoint["rest_path"]] = row
        by_name[endpoint["name"]] = row
    for alias, endpoint_name in OPENAPI_ALIAS_ENDPOINTS.items():
        row = by_name.get(endpoint_name)
        if row is None:
            fail(
                f"OPENAPI_ALIAS_ENDPOINTS maps {alias} to {endpoint_name!r}, which is "
                f"not a registry endpoint"
            )
        resolved[alias] = row
    return resolved


# An `x-anchors` entry: `  <key>: &<anchor>` at the block's own indentation.
_OPENAPI_ANCHOR_RE = re.compile(r"^  ([A-Za-z0-9_-]+): &([A-Za-z0-9_-]+)\s*$", re.MULTILINE)
# A `parameters:` list item: `        - ` at the operation's item indentation.
_OPENAPI_PARAM_ITEM_RE = re.compile(r"^        - ", re.MULTILINE)
_OPENAPI_PARAM_ALIAS_RE = re.compile(r"\*([A-Za-z0-9_-]+)\s*$")


def _openapi_param_fields(
    block: str,
) -> tuple[str, bool, str | None, str | None] | None:
    """`(name, required, default, pattern)` for one OpenAPI parameter node.

    `None` when the node carries no `name:`, which is how the non-parameter
    anchors in `x-anchors` (the response envelope, the row schema) are passed
    over. Each key is read at its first occurrence: a parameter node orders
    them `name` / `in` / `required` / `schema.default` ahead of its
    `description`, so prose quoting one of those words cannot be read as the
    value.
    """
    name = re.search(r"^\s*(?:-\s+)?name:\s*(\S+)\s*$", block, re.MULTILINE)
    if name is None:
        return None
    required = re.search(r"^\s*required:\s*(true|false)\s*$", block, re.MULTILINE)
    default = re.search(r"^\s*default:\s*(\S.*?)\s*$", block, re.MULTILINE)
    pattern = re.search(r"^\s*pattern:\s*(\S.*?)\s*$", block, re.MULTILINE)
    return (
        name.group(1),
        required is not None and required.group(1) == "true",
        default.group(1).strip("'\"") if default else None,
        pattern.group(1).strip("'\"") if pattern else None,
    )


def openapi_route_params(
    text: str, paths: set[str] | None = None
) -> dict[str, dict[str, tuple[bool, str | None, str | None]]]:
    """`{path: {param: (required, default, pattern)}}` as the spec declares it.

    The spec writes most parameters once under `x-anchors` and aliases them per
    route (`- *strike-param`), so the anchors are read first and an alias
    resolves to its anchor's fields; a route that spells a parameter out inline
    is read in place. Parsed rather than loaded because this gate is the
    cargo-free, dependency-free fast path the docs deploy runs, and a YAML
    library is not available to it.

    `paths` restricts the read to the routes the caller is going to compare, so
    a route outside that set is passed over rather than held to a parameter
    shape this parser happens to understand.
    """
    anchors: dict[str, tuple[str, bool, str | None, str | None]] = {}
    for match in _OPENAPI_ANCHOR_RE.finditer(text):
        block, _ = _yaml_block(
            text, rf"^  {re.escape(match.group(1))}: &{re.escape(match.group(2))}\s*$"
        )
        fields = _openapi_param_fields(block)
        if fields is not None:
            anchors[match.group(2)] = fields

    routes: dict[str, dict[str, tuple[bool, str | None, str | None]]] = {}
    for match in re.finditer(r"^  (/[A-Za-z0-9_/{}-]+):\s*$", text, re.MULTILINE):
        path = match.group(1)
        if paths is not None and path not in paths:
            continue
        path_block, _ = _yaml_block(text, rf"^  {re.escape(path)}:\s*$")
        params: dict[str, tuple[bool, str | None, str | None]] = {}
        # A path may carry several operations; every one of their
        # `parameters:` blocks is read, so a second method is never skipped.
        position = 0
        while True:
            block, position = _yaml_block(
                path_block, r"^      parameters:\s*$", after=position
            )
            if not block:
                break
            for item in _OPENAPI_PARAM_ITEM_RE.split(block)[1:]:
                alias = _OPENAPI_PARAM_ALIAS_RE.fullmatch(item.strip())
                if alias:
                    fields = anchors.get(alias.group(1))
                    if fields is None:
                        fail(
                            f"{rel(OPENAPI_YAML)} {path} aliases unknown "
                            f"parameter anchor *{alias.group(1)}"
                        )
                else:
                    fields = _openapi_param_fields(item)
                    if fields is None:
                        fail(
                            f"{rel(OPENAPI_YAML)} {path} has a parameter "
                            f"entry with no `name:`"
                        )
                params[fields[0]] = (fields[1], fields[2], fields[3])
        routes[path] = params
    return routes


def check_openapi_parameters(
    expected: dict[str, dict[str, tuple[bool, str | None]]] | None = None,
) -> None:
    """Every documented route's parameters must match the registry row.

    Names, required flags and defaults are all compared. A parameter the spec
    marks required that the registry makes optional forces every generated
    client to send a value the server does not need; a missing default leaves
    a reader with no way to know what omitting the parameter does.

    Routes the registry does not own are not read at all: the system, shutdown
    and flat-file routes have no registry row, and a route added to the spec
    ahead of the registry is the path-set assertion's business, not this one.
    `checked` guards the other direction, where a spec reformat leaves the
    parser matching nothing and the gate silently blind.

    `expected` overrides the derived registry rows; the self-test passes
    synthetic ones so the comparison is exercised on both sides.
    """
    if expected is None:
        expected = registry_route_params()
    documented = openapi_route_params(OPENAPI_YAML.read_text(), set(expected))
    checked = 0
    for path, want in sorted(expected.items()):
        got = documented.get(path)
        if got is None:
            continue
        checked += 1
        extra = sorted(set(got) - set(want) - OPENAPI_REST_ONLY_PARAMS)
        missing = sorted(set(want) - set(got))
        if extra or missing:
            fail(
                f"{rel(OPENAPI_YAML)} {path} parameter set drifted from "
                f"endpoint_surface.toml. missing={missing or '[]'} "
                f"extra={extra or '[]'}"
            )
        for name in sorted(want):
            want_required, want_default = want[name]
            got_required, got_default, _ = got[name]
            if want_required != got_required:
                fail(
                    f"{rel(OPENAPI_YAML)} {path} documents {name} as "
                    f"required={got_required}, but endpoint_surface.toml declares "
                    f"required={want_required}"
                )
            if want_default != got_default:
                fail(
                    f"{rel(OPENAPI_YAML)} {path} documents {name} with "
                    f"default {got_default!r}, but endpoint_surface.toml declares "
                    f"{want_default!r}"
                )
    if checked == 0:
        fail(
            f"{rel(OPENAPI_YAML)} parsed to no registry route parameters "
            f"(the spec's layout changed: the gate would be blind to parameter drift)"
        )


def registry_route_endpoint_names() -> dict[str, str]:
    """`{documented path: registry endpoint name}` for every route we own.

    The endpoint name is the join key to the vendor snapshot, whose
    `operationId` is the registry name verbatim. The server-only path aliases
    resolve to the endpoint they dispatch to, so both spellings of one endpoint
    answer to the same upstream row.
    """
    names = {ep["rest_path"]: ep["name"] for ep in REGISTRY_ENDPOINTS}
    names.update(OPENAPI_ALIAS_ENDPOINTS)
    return names


def upstream_expiration_wildcard() -> dict[str, bool]:
    """`{endpoint name: accepts `expiration=*`}` from the pinned vendor spec.

    Upstream references one of two parameter components per operation:
    `expiration` accepts the wildcard, `expiration_no_star` rejects it. Only
    operations that reference one of them get an entry, so an endpoint that
    takes no expiration at all is simply absent.

    An empty result means the snapshot was refreshed into a shape this parser
    does not recognise (upstream renamed the components, or restructured the
    parameter blocks), which must fail loudly rather than silently excuse every
    route. The Rust derivation fails closed on the same condition.
    """
    text = UPSTREAM_OPENAPI_YAML.read_text()
    wildcard: dict[str, bool] = {}
    operation: str | None = None
    for line in text.splitlines():
        match = re.match(r"^\s+operationId:\s*(\S+)\s*$", line)
        if match:
            operation = match.group(1)
            continue
        if operation is None:
            continue
        if "parameters/expiration_no_star" in line:
            wildcard[operation] = False
        elif re.search(r'parameters/expiration"?\'?\s*$', line):
            wildcard[operation] = True
    if not wildcard:
        fail(
            f"{rel(UPSTREAM_OPENAPI_YAML)} yielded no expiration component "
            f"references, so the wildcard rule could not be derived for any "
            f"endpoint. Upstream changed the spec's shape; update this parser."
        )
    return wildcard


def check_openapi_expiration_wildcard() -> None:
    """A documented `expiration` accepts `*` exactly where upstream does.

    The contract carries two expiration anchors, one whose pattern matches the
    `*` wildcard and one whose pattern rejects it, and the choice is made per
    route. Made by hand, the two spellings of one endpoint drifted apart and
    published contradictory rules for the same request. The rule is asserted
    against the pinned vendor snapshot instead, which is where the answer
    actually lives.

    The pattern is tested by matching it against `*` rather than by reading the
    anchor's name or looking for an escaped asterisk, so it holds whatever way
    a future pattern spells the wildcard.
    """
    endpoint_names = registry_route_endpoint_names()
    wildcard = upstream_expiration_wildcard()
    documented = openapi_route_params(OPENAPI_YAML.read_text(), set(endpoint_names))
    checked = 0
    for path, params in sorted(documented.items()):
        expiration = params.get("expiration")
        if expiration is None:
            continue
        name = endpoint_names[path]
        if name not in wildcard:
            fail(
                f"{rel(OPENAPI_YAML)} {path} documents an `expiration`, but "
                f"{rel(UPSTREAM_OPENAPI_YAML)} records no expiration component "
                f"for {name}, so the wildcard rule cannot be verified"
            )
        pattern = expiration[2]
        if pattern is None:
            fail(
                f"{rel(OPENAPI_YAML)} {path} documents `expiration` with no "
                f"`pattern`, so it states no rule about the `*` wildcard"
            )
        try:
            allows_wildcard = re.match(pattern, "*") is not None
        except re.error as exc:
            fail(
                f"{rel(OPENAPI_YAML)} {path} `expiration` pattern "
                f"{pattern!r} is not a valid regular expression: {exc}"
            )
        checked += 1
        if allows_wildcard != wildcard[name]:
            upstream = "accepts" if wildcard[name] else "rejects"
            ours = "accepts" if allows_wildcard else "rejects"
            fail(
                f"{rel(OPENAPI_YAML)} {path} documents an `expiration` that "
                f"{ours} the `*` wildcard, but {rel(UPSTREAM_OPENAPI_YAML)} says "
                f"{name} {upstream} it. Alias the matching expiration anchor."
            )
    if checked == 0:
        fail(
            f"{rel(OPENAPI_YAML)} parsed to no documented `expiration` "
            f"parameters (the spec's layout changed: the gate would be blind to "
            f"a wrong wildcard rule)"
        )


# The optional parameters the published examples pin. Pinning `strike` and
# `right` turns an option route's wildcard default into the single-contract
# request a reader is usually after, and `interval` is the one tuning knob most
# intraday requests set. Mirrors `showcased_builder_params` in the docs
# generator (`build_support_bin/endpoints/docs_render/lang.rs`), which seeds the
# reference pages' request builder from the same three.
OPENAPI_EXAMPLE_OPTIONALS = ("strike", "right", "interval")

# One example per language, in the order the document carries them, with the
# label the renderer prints beside the snippet.
OPENAPI_EXAMPLE_LANGS = (("rust", "Rust"), ("python", "Python"), ("cpp", "C++"))


def _endpoint_attr(endpoint: dict, key: str) -> str | None:
    """An endpoint attribute, resolved through its template chain."""
    value = endpoint.get(key)
    template_name = endpoint.get("template")
    seen: set[str] = set()
    while value is None and template_name is not None and template_name not in seen:
        seen.add(template_name)
        template = TEMPLATES.get(template_name)
        if template is None:
            fail(f"endpoint {endpoint['name']} references unknown template {template_name!r}")
        value = template.get(key)
        template_name = template.get("extends")
    return value


def _example_value(endpoint: dict, param: dict) -> str:
    """The literal an example passes for `param`, from `[test_fixtures]`.

    The same table the generated live validators draw from, so a published
    example passes the values those validators send. A param the table cannot
    answer for fails the gate rather than being handed an invented literal.

    The values are not on their own a promise that the vendor answers the
    request: that also needs every parameter the vendor requires to be set,
    which is `_example_optionals`' business.
    """
    fixtures = SURFACE["test_fixtures"]
    name, param_type = param["name"], param["param_type"]
    override = fixtures.get("concrete_overrides", {}).get(name)
    if override is not None:
        return override
    if param_type in ("Symbol", "Symbols"):
        category = _endpoint_attr(endpoint, "category")
        symbol = fixtures.get("category_symbol", {}).get(category)
        if symbol is None:
            fail(
                f"[test_fixtures.category_symbol] has no symbol for category "
                f"{category!r} ({endpoint['name']}), so no example value exists"
            )
        return symbol
    value = fixtures.get("concrete_by_type", {}).get(param_type)
    if value is None:
        fail(
            f"[test_fixtures.concrete_by_type] has no value for param type "
            f"{param_type!r} ({endpoint['name']}.{name}), so no example value exists"
        )
    return value


def _example_variable(endpoint: dict) -> str:
    """The variable an example binds its result to, derived from the endpoint.

    A list route is named for the rows it lists (`symbols`, `dates`); any other
    is named for its return type with the `Ticks` suffix dropped
    (`QuoteTicks` -> `quote`, `TradeGreeksAllTicks` -> `trade_greeks_all`).
    """
    list_column = endpoint.get("list_column")
    if list_column is not None:
        return f"{list_column}s"
    returns = _endpoint_attr(endpoint, "returns")
    if returns is None:
        fail(f"endpoint {endpoint['name']} declares no return type to name a variable after")
    out: list[str] = []
    for i, ch in enumerate(returns.removesuffix("Ticks")):
        if ch.isupper() and i > 0:
            out.append("_")
        out.append(ch.lower())
    return "".join(out)


def _example_literal(param: dict, value: str, lang: str) -> str:
    """`value` as a literal of `param`'s type in `lang`."""
    param_type = param["param_type"]
    if param_type == "Bool":
        return value.capitalize() if lang == "python" else value
    if param_type in ("Int", "Float"):
        return value
    if param_type == "Symbols":
        # The C++ method is overloaded on `const std::string&` and
        # `const std::vector<std::string>&`, and a braced list converts to
        # either through a user-defined conversion, so a bare `{"AAPL"}` is an
        # ambiguous call that does not compile. Name the vector, as the
        # generated C++ validator does.
        return {
            "rust": f'&["{value}"]',
            "python": f'["{value}"]',
            "cpp": f'std::vector<std::string>{{"{value}"}}',
        }[lang]
    return f'"{value}"'


def _example_optionals(params: list[dict]) -> list[dict]:
    """The optional parameters an example sets, in the registry's own order.

    Two sources. `OPENAPI_EXAMPLE_OPTIONALS` are the knobs a reader wants to
    see set. The rest is a date: the vendor marks `date`, `start_date` and
    `end_date` optional on the history routes and then refuses a request that
    carries none of them, so an example that sets no date is a call that can
    only fail. The validator generator anchors every cell the same way and for
    the same reason (`anchor_dates` in
    `build_support_bin/endpoints/modes.rs`): a single `date` where the route
    takes one, both ends of the range where it takes a range instead.
    """
    builder = [p for p in params if p.get("binding") == "builder"]
    names = {p["name"] for p in builder}
    shown = names & set(OPENAPI_EXAMPLE_OPTIONALS)
    if "date" in names:
        shown.add("date")
    elif {"start_date", "end_date"} <= names:
        shown |= {"start_date", "end_date"}
    return [p for p in builder if p["name"] in shown]


def render_openapi_examples() -> dict[str, str]:
    """`{documented path: x-code-examples block}` rendered from the registry.

    The registry already records which parameters a call takes positionally
    (`binding = "method"`) and which are optional (`binding = "builder"`), so
    the snippets are derived from it rather than written by hand, where they
    came to pass optional parameters positionally into signatures that have no
    such positions. Each language gets the shape its binding exposes: a Rust
    builder chain, Python keyword arguments, and a C++ `EndpointRequestOptions`
    with fluent setters.
    """
    blocks: dict[str, str] = {}
    by_name: dict[str, str] = {}
    for endpoint in REGISTRY_ENDPOINTS:
        params: list[dict] = []
        template_name = endpoint.get("template")
        if template_name is not None:
            params.extend(_template_params(template_name))
        params.extend(_expand_param_group(endpoint.get("params", [])))
        required = [p for p in params if p.get("binding") == "method"]
        optional = _example_optionals(params)
        name = endpoint["name"]
        variable = _example_variable(endpoint)
        lines = ["      x-code-examples:"]
        for lang, label in OPENAPI_EXAMPLE_LANGS:
            args = ", ".join(
                _example_literal(p, _example_value(endpoint, p), lang) for p in required
            )
            if lang == "rust":
                chain = "".join(
                    f'.{p["name"]}({_example_literal(p, _example_value(endpoint, p), lang)})'
                    for p in optional
                )
                call = f"let {variable} = client.market_data().{name}({args}){chain}.await?;"
            elif lang == "python":
                kwargs = "".join(
                    f', {p["name"]}={_example_literal(p, _example_value(endpoint, p), lang)}'
                    for p in optional
                )
                call = f"{variable} = client.market_data.{name}({args}{kwargs})"
            else:
                setters = "".join(
                    f'.with_{p["name"]}({_example_literal(p, _example_value(endpoint, p), lang)})'
                    for p in optional
                )
                options = (
                    f", thetadatadx::EndpointRequestOptions{{}}{setters}" if setters else ""
                )
                call = f"auto {variable} = client.market_data().{name}({args}{options});"
            lines += [
                f"        - lang: {lang}",
                f"          label: {label}",
                "          source: |",
                f"            {call}",
            ]
        block = "\n".join(lines) + "\n"
        blocks[endpoint["rest_path"]] = block
        by_name[name] = block
    # A server-only path alias is a second spelling of one endpoint, served by
    # the same code through the same SDK method, so it carries that endpoint's
    # example.
    for alias, endpoint_name in OPENAPI_ALIAS_ENDPOINTS.items():
        blocks[alias] = by_name[endpoint_name]
    return blocks


def _openapi_example_blocks(text: str) -> dict[str, tuple[str, int, int]]:
    """`{path: (block, start, end)}` for each route's `x-code-examples`.

    `start` and `end` are offsets into `text`, so `--write-examples` can
    replace a block in place without reflowing the rest of the document.

    A blank line separates one route from the next and belongs to neither, but
    an indentation scan cannot see where an indented block ends and the blank
    run begins, so trailing blank lines are left outside the span. Replacing a
    block that swallowed them would delete the separator.
    """
    found: dict[str, tuple[str, int, int]] = {}
    for match in re.finditer(r"^  (/[A-Za-z0-9_/{}-]+):\s*$", text, re.MULTILINE):
        path = match.group(1)
        path_block, path_end = _yaml_block(text, rf"^  {re.escape(path)}:\s*$")
        offset = path_end - len(path_block)
        inner, inner_end = _yaml_block(path_block, r"^      x-code-examples:\s*$")
        if not inner:
            continue
        end = offset + inner_end
        trailing = len(inner) - len(inner.rstrip("\n"))
        # Keep the one newline that ends the block's last line.
        if trailing > 1:
            inner = inner[: -(trailing - 1)]
            end -= trailing - 1
        found[path] = (inner, end - len(inner), end)
    return found


def check_openapi_examples(write: bool = False) -> None:
    """Each route's `x-code-examples` must be the block the registry renders.

    Kept by hand, the snippets drifted from the signatures they claim to show:
    most passed the optional `strike` and `right` positionally, which no
    binding accepts, so a reader who copied one got a call that does not
    compile. Rendering them from the registry and comparing byte for byte means
    a signature change moves the examples with it. `write` rewrites the
    document instead of comparing, which is how the snippets are regenerated.
    """
    text = OPENAPI_YAML.read_text()
    expected = render_openapi_examples()
    documented = _openapi_example_blocks(text)
    if write:
        # Rewrite from the back so each replacement leaves the earlier offsets
        # untouched.
        for path, (block, start, end) in sorted(
            documented.items(), key=lambda item: item[1][1], reverse=True
        ):
            want = expected.get(path)
            if want is not None and want != block:
                text = text[:start] + want + text[end:]
        OPENAPI_YAML.write_text(text)
        print(f"openapi examples: rewrote {rel(OPENAPI_YAML)}")
        return
    checked = 0
    for path, want in sorted(expected.items()):
        entry = documented.get(path)
        if entry is None:
            fail(
                f"{rel(OPENAPI_YAML)} {path} carries no `x-code-examples`, so the "
                f"route publishes no runnable call. Run with --write-examples."
            )
        checked += 1
        if entry[0] != want:
            fail(
                f"{rel(OPENAPI_YAML)} {path} `x-code-examples` is not what the "
                f"registry renders. Run with --write-examples.\n"
                f"--- documented ---\n{entry[0]}--- registry ---\n{want}"
            )
    if checked == 0:
        fail(
            f"{rel(OPENAPI_YAML)} parsed to no `x-code-examples` blocks (the "
            f"spec's layout changed: the gate would be blind to example drift)"
        )


def check_flatfile_matrix() -> None:
    """OpenAPI flat-file enums must equal the `SERVED_DATASETS` served matrix.

    The path form (`/v3/{sec_type}/flat_file/{req_type}`) takes the two segments
    independently, so its `sec_type` enum must be the served security types and
    its `req_type` enum the union of every served request type. A served-matrix
    drift in the checked-in spec in either direction fails the gate.
    """
    matrix = flatfile_served_matrix()
    expected_secs = set(matrix)
    expected_req_union = set().union(*matrix.values())

    text = OPENAPI_YAML.read_text()

    # --- Path form: /v3/{sec_type}/flat_file/{req_type} ---------------------
    path_block, _ = _yaml_block(
        text, r"^  /v3/\{sec_type\}/flat_file/\{req_type\}:\s*$"
    )
    if not path_block:
        fail(
            f"{OPENAPI_YAML.relative_to(ROOT)} missing the flat-file path "
            f"/v3/{{sec_type}}/flat_file/{{req_type}}"
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


def _rust_str_array_items(text: str, const_name: str, path: Path) -> list[str]:
    """The string literals in a `const NAME: [...] = [ "a", "b", ... ];` array.

    Used to read `OFFLINE_TOOL_NAMES` from the MCP `main.rs` and `TOOL_NAMES`
    from `stream.rs`, so each tool set tracks its source const rather than a
    copy in this gate.
    """
    m = re.search(rf"const {const_name}:[^=]*=\s*\[(.*?)\];", text, re.DOTALL)
    if not m:
        fail(f"{path.relative_to(ROOT)} missing the {const_name} array")
    return re.findall(r'"([^"]+)"', m.group(1))


def _rust_fn_body(text: str, fn_signature_re: str, path: Path) -> str:
    """The brace-balanced body of the first `fn` matching `fn_signature_re`.

    Scopes a literal scan to one function so an identically-spelled literal in a
    sibling function (e.g. a temp-path format string, a test fixture) is never
    swept in.
    """
    m = re.search(fn_signature_re, text)
    if not m:
        fail(f"{path.relative_to(ROOT)} missing fn matching {fn_signature_re!r}")
    depth = 0
    start = None
    for i in range(m.start(), len(text)):
        ch = text[i]
        if ch == "{":
            if depth == 0:
                start = i + 1
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return text[start:i]
    fail(f"{path.relative_to(ROOT)} fn matching {fn_signature_re!r} has no closed body")
    return ""  # unreachable; fail() raises


def plural_names(names: list[str]) -> str:
    """Render tool names the way the README prose does: `a`, `b` and `c`."""
    quoted = [f"`{n}`" for n in sorted(names)]
    if len(quoted) <= 1:
        return quoted[0] if quoted else ""
    return ", ".join(quoted[:-1]) + " and " + quoted[-1]


def mcp_tool_inventory() -> dict[str, list[str]]:
    """The connected `tools/list` surface, derived from the MCP server source.

    Returns the tool names grouped by origin:

    - ``registry``: one tool per non-stream registry endpoint (the historical
      surface), named after the endpoint.
    - ``offline``: the always-available utilities, read from
      ``OFFLINE_TOOL_NAMES`` in ``main.rs``.
    - ``flatfile``: the flat-file tools advertised by
      ``push_flatfile_tool_definitions`` in ``flatfile_tools.rs``.
    - ``utility``: the generated utility tools in ``utilities.rs``, which
      include the offline ones; the offline set is subtracted so a tool is
      reported under one origin only.

    A connected `tools/list` returns the union; the docs gate asserts the docs
    enumerate it, so a tool added in code but absent from the docs fails here.
    Every source of tool names the connected list draws from is read here: a
    family parsed from none of these files would be undocumented with this
    gate green.
    """
    registry = [ep["name"] for ep in REGISTRY_ENDPOINTS]

    offline = _rust_str_array_items(
        MCP_MAIN_RS.read_text(), "OFFLINE_TOOL_NAMES", MCP_MAIN_RS
    )
    if not offline:
        fail(f"{MCP_MAIN_RS.relative_to(ROOT)} OFFLINE_TOOL_NAMES parsed to no tools")

    utility_body = _rust_fn_body(
        MCP_UTILITIES_RS.read_text(),
        r"fn push_generated_utility_tool_definitions\b",
        MCP_UTILITIES_RS,
    )
    utility = [
        name
        for name in dict.fromkeys(re.findall(r'"name":\s*"([a-z_]+)"', utility_body))
        if name not in offline
    ]

    flatfile_body = _rust_fn_body(
        MCP_FLATFILE_TOOLS_RS.read_text(),
        r"fn push_flatfile_tool_definitions\b",
        MCP_FLATFILE_TOOLS_RS,
    )
    flatfile_text = MCP_FLATFILE_TOOLS_RS.read_text()
    flatfile = set(re.findall(r'"(thetadatadx_flatfile_[a-z_]+)"', flatfile_body))
    # A name the function pushes through a const is still a tool it advertises.
    # Reading only the literals in the body missed the generic dispatcher, which
    # is named by `FLATFILE_DISPATCHER_TOOL`, so it was never compared with the
    # docs at all.
    for const_name, value in re.findall(
        r'const ([A-Z_]+): &str = "(thetadatadx_flatfile_[a-z_]+)"', flatfile_text
    ):
        if re.search(rf"\b{const_name}\b", flatfile_body):
            flatfile.add(value)
    flatfile = sorted(flatfile)
    if not flatfile:
        fail(
            f"{MCP_FLATFILE_TOOLS_RS.relative_to(ROOT)} push_flatfile_tool_definitions "
            f"advertised no flat-file tools"
        )

    return {
        "registry": registry,
        "offline": offline,
        "flatfile": flatfile,
        "utility": utility,
    }


def _markdown_section(text: str, heading: str, path: Path) -> str:
    """The body under a `##` `heading` up to the next `##`-or-shallower heading.

    Scopes a tool-enumeration scan to the doc's tool-listing section so a tool
    name appearing in unrelated prose elsewhere in the file cannot satisfy the
    check for a tool dropped from the listing itself.
    """
    lines = text.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.strip() == heading)
    except StopIteration:
        fail(f"{path.relative_to(ROOT)} missing the {heading!r} section the tool gate scans")
    level = len(heading) - len(heading.lstrip("#"))
    out: list[str] = []
    for line in lines[start + 1 :]:
        stripped = line.lstrip("#")
        depth = len(line) - len(stripped)
        if line.startswith("#") and stripped.startswith(" ") and depth <= level:
            break
        out.append(line)
    return "\n".join(out)


SECTION_COUNT_RE = re.compile(r"^### .*?\((\d+)")


def check_mcp_tool_counts(inventory: set[str]) -> None:
    """Each `### ... (N tools)` heading must count the tools listed under it.

    The counts are published in the package README, so each one is derived
    from the list beneath it rather than maintained by hand: a heading saying
    fourteen over thirteen tools is a number with nothing behind it. Only
    names in the server's own tool inventory count, so a backticked parameter
    or type in a bullet's prose is not mistaken for a tool.
    """
    readme, heading = MCP_DOC_TOOL_SECTIONS[0]
    section = _markdown_section(readme.read_text(), heading, readme)
    current: tuple[str, int] | None = None
    listed: set[str] = set()
    checked = 0

    def settle() -> int:
        if current is None:
            return 0
        title, claimed = current
        if claimed != len(listed):
            fail(
                f"{readme.relative_to(ROOT)} {title!r} claims {claimed} tools and lists "
                f"{len(listed)}. Correct the heading or the list."
            )
        return 1

    for line in section.splitlines():
        if line.startswith("### "):
            checked += settle()
            m = SECTION_COUNT_RE.match(line)
            current = (line.strip(), int(m.group(1))) if m else None
            listed = set()
        elif current is not None:
            listed |= {
                name for name in re.findall(r"`([a-z_]+)`", line) if name in inventory
            }
    checked += settle()
    if checked == 0:
        fail(
            f"{readme.relative_to(ROOT)} {heading!r} section has no counted "
            f"'### ... (N tools)' heading, so no count was checked."
        )


def check_mcp_tool_inventory() -> None:
    """The MCP docs must enumerate the connected `tools/list` surface.

    Every connection-only tool (the offline utilities, the flat-file tools, the
    live-feed tools and the generated utilities) must appear by name in each MCP
    doc's tool-listing section, so a tool advertised by the server but missing
    from the listing trips this check.
    The README also lists every registry market-data endpoint by name in its tool
    tables, so the full inventory is asserted against it; `mcp.md` defers the
    per-endpoint listing to the generated reference pages, so only the
    connection-only tools are pinned there.
    """
    inv = mcp_tool_inventory()
    connection_only = inv["offline"] + inv["flatfile"] + inv["utility"]

    for doc, heading in MCP_DOC_TOOL_SECTIONS:
        section = _markdown_section(doc.read_text(), heading, doc)
        missing = [name for name in connection_only if f"`{name}`" not in section]
        if missing:
            fail(
                f"{doc.relative_to(ROOT)} {heading!r} section does not enumerate MCP "
                f"tools that the connected tools/list advertises: {missing}. Document "
                f"them so the docs match the server's tool surface."
            )

    readme, readme_heading = MCP_DOC_TOOL_SECTIONS[0]
    readme_section = _markdown_section(readme.read_text(), readme_heading, readme)
    missing_registry = [name for name in inv["registry"] if f"`{name}`" not in readme_section]
    if missing_registry:
        fail(
            f"{readme.relative_to(ROOT)} {readme_heading!r} section does not list every "
            f"registry MCP tool by name: {missing_registry}. The README tool tables must "
            f"cover the full market-data surface the connected tools/list advertises."
        )

    check_mcp_tool_counts(set(inv["registry"]) | set(connection_only))


def check_endpoint_option_surface() -> None:
    rust_fields = extract_struct_fields(
        ROOT / "thetadatadx-ffi/src/endpoint_request_options.rs",
        r"pub struct ThetaDataDxEndpointRequestOptions \{(.*?)\n\}",
        r"^\s*pub\s+([a-z_]+)\s*:",
    )
    # Exclude has_* sentinel flags (FFI implementation detail, not builder params)
    rust_fields = {f for f in rust_fields if not f.startswith("has_")}
    if rust_fields != ALL_OPTION_FIELDS:
        missing = sorted(ALL_OPTION_FIELDS - rust_fields)
        extra = sorted(rust_fields - ALL_OPTION_FIELDS)
        fail(
            "thetadatadx-ffi/src/endpoint_request_options.rs endpoint option fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    c_path = ROOT / "thetadatadx-cpp/include/endpoint_request_options.h.inc"
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
        ROOT / "thetadatadx-cpp/include/endpoint_options.hpp.inc",
        r"struct EndpointRequestOptions \{(.*?)\n\};",
        r"^\s*std::optional<[^>]+>\s+([a-z_]+);",
    )
    if cpp_fields != ALL_OPTION_FIELDS:
        missing = sorted(ALL_OPTION_FIELDS - cpp_fields)
        extra = sorted(cpp_fields - ALL_OPTION_FIELDS)
        fail(
            "thetadatadx-cpp/include/endpoint_options.hpp.inc EndpointRequestOptions fields drifted from endpoint_surface.toml. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    for path in [
        ROOT / "thetadatadx-ffi/src/lib.rs",
        ROOT / "thetadatadx-cpp/include/thetadatadx.h",
        ROOT / "thetadatadx-cpp/include/thetadatadx.hpp",
        ROOT / "thetadatadx-cpp/src/thetadatadx.cpp",
        ROOT / "thetadatadx-cpp/README.md",
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


def _selftest() -> int:
    """Hermetic checks for the server flag-default derivation (G5) and the
    OpenAPI parameter comparison.

    Drives `server_arg_defaults`, `_parse_doc_flag_table`, and
    `_value_enum_render` on synthetic inputs:

    * The `#[arg]` parser reads string (`default_value = "x"`), integer
      (`default_value_t = 25503`), and `ValueEnum` (`default_value_t =
      LogFormat::Text` -> `text`) defaults, treats a flag with no default
      as `None`, and — critically — is NOT truncated by a
      `value_parser = ["a", "b"]` allow-list whose `]` would otherwise
      close a naive attribute scan (the inner-delimiter bypass).
    * The Markdown table parser maps `--flag <metavar>` and paired
      `--a / --b` rows to their default cells, reads an em-dash / empty cell
      as `None`, and unwraps backticked defaults.
    * A documented default that disagrees with the source value is caught
      value-by-value, where a substring scan would miss a default that
      changed to a value still printed elsewhere.

    The OpenAPI parameter cases drive `check_openapi_parameters` on a
    synthetic spec against synthetic registry rows, so both sides of the
    comparison are fixed here rather than tracking the live registry. A spec
    that agrees is accepted; a required flag or a default moved on one side
    only is refused; and a spec whose routes the parser cannot find is
    refused rather than passing on an empty comparison.
    """
    import tempfile

    global SERVER_MAIN_RS
    failures: list[str] = []

    # `_value_enum_render`.
    if _value_enum_render("Text") != "text":
        failures.append("value-enum: `Text` did not render to `text`")
    if _value_enum_render("OpenInterest") != "open-interest":
        failures.append("value-enum: `OpenInterest` did not render to `open-interest`")

    synthetic_main = (
        "struct Args {\n"
        '    #[arg(long, default_value = "creds.txt")]\n'
        "    creds: String,\n"
        "    #[arg(long)]\n"
        "    api_key: Option<String>,\n"
        '    #[arg(long, default_value = "production", '
        'value_parser = ["production", "dev"])]\n'
        "    streaming_region: String,\n"
        "    #[arg(long, default_value_t = 25503)]\n"
        "    http_port: u16,\n"
        "    #[arg(long, value_enum, default_value_t = logging::LogFormat::Text)]\n"
        "    log_format: logging::LogFormat,\n"
        "    #[arg(long)]\n"
        "    no_streaming: bool,\n"
        "}\n"
    )
    saved_main = SERVER_MAIN_RS
    with tempfile.TemporaryDirectory() as td:
        fake_main = Path(td) / "main.rs"
        fake_main.write_text(synthetic_main, encoding="utf-8")
        SERVER_MAIN_RS = fake_main
        try:
            derived = server_arg_defaults()
        finally:
            SERVER_MAIN_RS = saved_main

    expected = {
        "--creds": "creds.txt",
        "--api-key": None,
        "--streaming-region": "production",  # not truncated by value_parser `]`
        "--http-port": "25503",
        "--log-format": "text",
        "--no-streaming": None,
    }
    for flag, want in expected.items():
        got = derived.get(flag, "<<missing>>")
        if got != want:
            failures.append(
                f"arg-parse: {flag} derived as {got!r}, expected {want!r}"
            )

    # Markdown table parser: metavar, paired flags, em-dash, backticks.
    table = (
        "| Flag | Default | Description |\n"
        "|------|---------|-------------|\n"
        "| `--creds <path>` | `creds.txt` | creds file |\n"
        "| `--email` / `--password` | — | inline creds |\n"
        "| `--http-port <port>` | `25503` | port |\n"
        "| `--log-format <fmt>` | `text` | format |\n"
    )
    parsed = _parse_doc_flag_table(table)
    table_expected = {
        "--creds": "creds.txt",
        "--email": None,
        "--password": None,
        "--http-port": "25503",
        "--log-format": "text",
    }
    for flag, want in table_expected.items():
        got = parsed.get(flag, "<<missing>>")
        if got != want:
            failures.append(
                f"table-parse: {flag} parsed as {got!r}, expected {want!r}"
            )

    # Value-by-value mismatch detection. The comparison has to be exercised
    # through the gate itself: comparing two dicts written here would assert
    # that "25503" differs from "25504" and would still pass if
    # `check_server_flag_defaults` stopped comparing values altogether.
    # So move the port in the source and require the gate to refuse it
    # against the real docs, which still document the old one.
    moved_main = synthetic_main.replace("25503", "25504")
    if moved_main == synthetic_main:
        failures.append(
            "value-by-value: the fixture no longer carries the port default "
            "this case moves, so the case proves nothing"
        )
    with tempfile.TemporaryDirectory() as td:
        fake_main = Path(td) / "main.rs"
        fake_main.write_text(moved_main, encoding="utf-8")
        SERVER_MAIN_RS = fake_main
        try:
            check_server_flag_defaults()
        except SystemExit:
            pass
        else:
            failures.append(
                "value-by-value: a source default that moved away from the "
                "documented one was not refused by the gate"
            )
        finally:
            SERVER_MAIN_RS = saved_main

    # --- OpenAPI parameter comparison ---------------------------------------
    # One route, one aliased parameter and one inline one, so both resolution
    # paths are exercised. `format` is the REST-only parameter the registry
    # never carries and the comparison must forgive.
    synthetic_spec = (
        "x-anchors:\n"
        "  strike-param: &strike-param\n"
        "    name: strike\n"
        "    in: query\n"
        "    required: false\n"
        "    schema:\n"
        "      type: string\n"
        "      default: '*'\n"
        "    description: >-\n"
        "      Prose that says required: true and default: 9 to prove neither\n"
        "      is read out of a description.\n"
        "  format-param: &format-param\n"
        "    name: format\n"
        "    in: query\n"
        "    required: false\n"
        "    schema:\n"
        "      type: string\n"
        "      default: csv\n"
        "paths:\n"
        "  /v3/option/snapshot/trade:\n"
        "    get:\n"
        "      operationId: optionSnapshotTrade\n"
        "      parameters:\n"
        "        - name: symbol\n"
        "          in: query\n"
        "          required: true\n"
        "          schema:\n"
        "            type: string\n"
        "        - *strike-param\n"
        "        - name: interval\n"
        "          in: query\n"
        "          required: false\n"
        "          schema:\n"
        "            type: string\n"
        "            default: 1s\n"
        "        - *format-param\n"
        "      responses:\n"
        "        '200':\n"
        "          description: ok\n"
    )
    synthetic_rows = {
        "/v3/option/snapshot/trade": {
            "symbol": (True, None),
            "strike": (False, "*"),
            "interval": (False, "1s"),
        }
    }

    global OPENAPI_YAML
    saved_openapi = OPENAPI_YAML

    def openapi_case(label: str, spec: str, rows: dict, *, refuse: bool) -> None:
        """Run the parameter gate on `spec`; `refuse` says it must reject it."""
        global OPENAPI_YAML
        with tempfile.TemporaryDirectory() as td:
            fake_spec = Path(td) / "thetadatadx.yaml"
            fake_spec.write_text(spec, encoding="utf-8")
            OPENAPI_YAML = fake_spec
            try:
                check_openapi_parameters(rows)
            except SystemExit:
                if not refuse:
                    failures.append(
                        f"openapi-params: {label} was refused but agrees with the rows"
                    )
            else:
                if refuse:
                    failures.append(f"openapi-params: {label} was accepted")
            finally:
                OPENAPI_YAML = saved_openapi

    openapi_case("an agreeing spec", synthetic_spec, synthetic_rows, refuse=False)

    # A required flag moved on the spec side only. The aliased parameter is
    # the one moved, so an alias resolved to a stale or shared node would
    # show up here too.
    required_drift = synthetic_spec.replace(
        "    name: strike\n    in: query\n    required: false\n",
        "    name: strike\n    in: query\n    required: true\n",
    )
    if required_drift == synthetic_spec:
        failures.append(
            "openapi-params: the fixture no longer carries the optional "
            "`strike` this case marks required, so the case proves nothing"
        )
    openapi_case("a required-flag drift", required_drift, synthetic_rows, refuse=True)

    # A default moved on the registry side only.
    moved_default = {
        path: {**params, "interval": (False, "5s")}
        for path, params in synthetic_rows.items()
    }
    openapi_case("a default drift", synthetic_spec, moved_default, refuse=True)

    # A spec the parser finds no route in must fail rather than pass on an
    # empty comparison.
    openapi_case(
        "a spec with no comparable route",
        synthetic_spec,
        {"/v3/stock/snapshot/trade": {"symbol": (True, None)}},
        refuse=True,
    )

    if failures:
        print("check_docs_consistency --selftest: FAILED")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("check_docs_consistency --selftest: ok")
    return 0


def main() -> None:
    check_static_docs()
    check_server_flag_defaults()
    check_reference_pages()
    check_llms_txt()
    check_openapi()
    check_openapi_parameters()
    check_openapi_expiration_wildcard()
    check_openapi_examples()
    check_flatfile_matrix()
    check_mcp_tool_inventory()
    check_endpoint_option_surface()
    check_tier_badges()
    print("docs consistency: ok")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="Run the embedded self-test and exit.",
    )
    parser.add_argument(
        "--write-examples",
        action="store_true",
        help=(
            "Rewrite the OpenAPI document's x-code-examples from the endpoint "
            "registry instead of checking them, then exit."
        ),
    )
    args = parser.parse_args()
    if args.selftest:
        sys.exit(_selftest())
    if args.write_examples:
        check_openapi_examples(write=True)
        raise SystemExit(0)
    main()
