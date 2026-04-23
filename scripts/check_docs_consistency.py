#!/usr/bin/env python3
"""Validate checked-in documentation against the endpoint surface spec.

This script keeps the human-facing docs aligned with the current SDK and REST
surface by checking a few high-signal invariants:

- endpoint/tool counts in top-level docs
- REST/OpenAPI path + operationId parity with `endpoint_surface.toml`
- docs-site API reference coverage for every endpoint
- current server docs staying on the v3 path scheme and current defaults
- `CHANGELOG.md` and `docs-site/docs/changelog.md` staying byte-identical
  (the docs-site build re-publishes whatever is on `main`, so any drift
  is a release-note bug the moment it lands)

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

# Paths exposed only by the thetadatadx-server binary (not by the upstream
# ThetaData terminal). Allowlist these so the drift check focuses on
# upstream-tracking endpoints. `/v3/system/shutdown` is a privileged
# graceful-shutdown route gated by a per-startup UUID token — see
# `tools/server/src/handler.rs::system_shutdown` and the hardening section
# in `docs-site/docs/tools/server.md`.
SERVER_SPECIFIC_PATHS = {"/v3/system/shutdown"}
BUILDER_PARAMS = {
    param["name"]
    for group in SURFACE["param_groups"].values()
    for param in group.get("params", [])
    if param.get("binding") == "builder"
}

# Global request-level options (not per-endpoint builder params, but still
# required fields on `TdxEndpointRequestOptions` / `EndpointRequestOptions`
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


def snake_to_go(name: str) -> str:
    acronyms = {
        "dte": "DTE",
        "nbbo": "NBBO",
    }
    return "".join(acronyms.get(part, part.capitalize()) for part in name.split("_"))


def expect_contains(path: Path, snippet: str) -> None:
    text = path.read_text()
    if snippet not in text:
        fail(f"{path.relative_to(ROOT)} missing expected text: {snippet!r}")


def expect_not_contains(path: Path, snippet: str) -> None:
    text = path.read_text()
    if snippet in text:
        fail(f"{path.relative_to(ROOT)} contains stale text: {snippet!r}")


def check_static_docs() -> None:
    expect_contains(ROOT / "README.md", "VitePress documentation site")
    expect_contains(
        ROOT / "README.md",
        f"MCP server - gives LLMs access to {EXPECTED_TOOL_COUNT} tools over JSON-RPC",
    )

    expect_contains(
        ROOT / "tools/mcp/README.md",
        f"## Available Tools ({EXPECTED_TOOL_COUNT} total)",
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        f"{len(REGISTRY_ENDPOINTS)} registry endpoints + 3 offline tools (ping, all_greeks, implied_volatility) = {EXPECTED_TOOL_COUNT} total.",
    )

    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        f"## Available Tools ({EXPECTED_TOOL_COUNT})",
    )
    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        f"{len(REGISTRY_ENDPOINTS)} data endpoints + ping + all_greeks + implied_volatility = {EXPECTED_TOOL_COUNT} tools.",
    )
    # Version strings in getting-started docs must match the workspace version.
    expect_contains(
        ROOT / "docs-site/docs/getting-started/installation.md",
        'thetadatadx = "7.3"',
    )
    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
    )

    # Website changelog must match repo root CHANGELOG.md
    repo_changelog = (ROOT / "CHANGELOG.md").read_text()
    site_changelog = (ROOT / "docs-site/docs/changelog.md").read_text()
    if repo_changelog != site_changelog:
        fail(
            "docs-site/docs/changelog.md is out of sync with CHANGELOG.md. "
            "Run: cp CHANGELOG.md docs-site/docs/changelog.md"
        )

    for path in [
        ROOT / "tools/server/README.md",
        ROOT / "docs-site/docs/tools/server.md",
    ]:
        expect_contains(path, "25503")
        expect_contains(path, "/v3/stock/history/ohlc_range")
        expect_contains(path, "/v3/option/snapshot/quote?symbol=")
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
    expect_contains(sdk_overview, "`TdxUnified` / `TdxFpssHandle`")
    expect_contains(sdk_overview, "| **Unified** | `tdx_unified_connect`, `tdx_unified_historical`, `tdx_unified_*`, `tdx_unified_free` |")
    expect_contains(sdk_overview, "26 functions: `tdx_fpss_connect`")
    expect_contains(sdk_overview, "Windows is validated with the GNU Rust target (`x86_64-pc-windows-gnu`)")

    go_readme = ROOT / "sdks/go/README.md"
    expect_contains(go_readme, "Windows: CI-validated via a GNU-targeted Rust FFI build (`x86_64-pc-windows-gnu`)")
    expect_contains(go_readme, "cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi")

    option_docs = list((ROOT / "docs-site/docs/historical/option").rglob("*.md"))
    strike_docs = option_docs + [
        ROOT / "docs-site/docs/api-reference.md",
        ROOT / "docs/api-reference.md",
        ROOT / "tools/cli/README.md",
        ROOT / "docs-site/docs/tools/cli.md",
        ROOT / "tools/server/README.md",
        ROOT / "docs-site/docs/tools/server.md",
        ROOT / "docs-site/public/thetadatadx.yaml",
    ]
    for path in strike_docs:
        expect_not_contains(path, "scaled integer")
        expect_not_contains(path, "500000")

    expect_contains(
        ROOT / "crates/thetadatadx/endpoint_surface.toml",
        'description = "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"',
    )
    expect_contains(
        ROOT / "tools/cli/README.md",
        "tdx stock at_time_trade AAPL 20240101 20240301 09:30:00.000",
    )
    expect_contains(
        ROOT / "docs/api-reference.md",
        '`time_of_day` uses `HH:MM:SS.SSS` ET wall-clock format (e.g. `"09:30:00.000"`). Legacy millisecond strings such as `"34200000"` are also accepted.',
    )
    expect_contains(
        ROOT / "docs-site/docs/api-reference.md",
        '| `time_of_day` | string | Yes | ET wall-clock time in `HH:MM:SS.SSS`',
    )
    expect_contains(
        ROOT / "docs-site/public/thetadatadx.yaml",
        'description: ET wall-clock time in HH:MM:SS.SSS (e.g. "09:30:00.000" for 9:30 AM ET; legacy "34200000" is also accepted)',
    )
    expect_not_contains(
        ROOT / "docs-site/docs/api-reference.md",
        'Ms from midnight ET',
    )
    expect_not_contains(
        ROOT / "tools/cli/README.md",
        '34200000   # 9:30 AM',
    )

    # Streaming section guards — catch stale FPSS counts and wrong return types
    expect_not_contains(ROOT / "docs/architecture.md", "7 FFI FPSS functions")
    expect_not_contains(ROOT / "docs/architecture.md", "18 FFI FPSS functions")
    expect_not_contains(ROOT / "docs/architecture.md", "symbol-level subscribe/unsubscribe only")
    expect_contains(ROOT / "docs-site/docs/api-reference.md", "FpssEventPtr")
    expect_not_contains(
        ROOT / "docs-site/docs/api-reference.md",
        "Python only (uses Rust SDK directly)",
    )
    expect_not_contains(
        ROOT / "docs-site/docs/api-reference.md",
        "Python only (FFI only supports symbol-level)",
    )
    expect_contains(
        ROOT / "sdks/python/README.md",
        "`subscribe_option_quotes(symbol, expiration, strike, right)`",
    )
    # Streaming method-reference guards (retargeted after the top-level
    # streaming.md orphan was deleted in favor of the streaming/*.md subdirectory).
    for streaming_page in [
        ROOT / "docs-site/docs/streaming/index.md",
        ROOT / "docs-site/docs/streaming/connection.md",
        ROOT / "docs-site/docs/streaming/events.md",
        ROOT / "docs-site/docs/streaming/reconnection.md",
        ROOT / "docs-site/docs/streaming/latency.md",
    ]:
        expect_not_contains(streaming_page, "Python does not expose reconnect_streaming() directly.")
        expect_not_contains(streaming_page, "Go does not expose reconnect_streaming() directly.")
        expect_not_contains(streaming_page, "C++ does not expose reconnect_streaming() directly.")
        expect_not_contains(streaming_page, "| `active_subscriptions` | `() -> std::string` |")
    expect_contains(ROOT / "docs-site/docs/streaming/events.md", "| `Reconnect` | `() error` |")
    expect_contains(
        ROOT / "docs-site/docs/streaming/events.md",
        "| `contract_map` | `() -> std::map<int32_t, std::string>` |",
    )
    expect_contains(
        ROOT / "docs-site/docs/streaming/events.md",
        "| `SubscribeOptionQuotes` | `(symbol, expiration, strike, right string) (int, error)` |",
    )
    expect_contains(
        ROOT / "docs-site/docs/streaming/reconnection.md",
        "Python, TypeScript/Node.js, Go, and C++ expose `reconnect()` on their public streaming clients.",
    )


def check_api_reference() -> None:
    api_reference = (ROOT / "docs-site/docs/api-reference.md").read_text()
    headings = set(re.findall(r"^### ([a-z0-9_]+)$", api_reference, re.MULTILINE))
    missing = sorted(ENDPOINT_NAMES - headings)
    if missing:
        fail(f"docs-site/docs/api-reference.md missing endpoint headings: {', '.join(missing)}")


def check_openapi() -> None:
    text = (ROOT / "docs-site/public/thetadatadx.yaml").read_text()
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
            "docs-site/public/thetadatadx.yaml path set drifted. "
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
            "docs-site/public/thetadatadx.yaml operationId set drifted. "
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
        r"pub struct TdxEndpointRequestOptions \{(.*?)\n\}",
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

    for path in [
        ROOT / "sdks/go/endpoint_request_options.h.inc",
        ROOT / "sdks/cpp/include/endpoint_request_options.h.inc",
    ]:
        c_fields = extract_struct_fields(
            path,
            r"typedef struct \{(.*?)\n\}\s*TdxEndpointRequestOptions;",
            r"^\s*(?:const char\*|int32_t|double|uint64_t)\s+([a-z_]+);",
        )
        # Exclude has_* sentinel flags (FFI implementation detail, not builder params)
        c_fields = {f for f in c_fields if not f.startswith("has_")}
        if c_fields != ALL_OPTION_FIELDS:
            missing = sorted(ALL_OPTION_FIELDS - c_fields)
            extra = sorted(c_fields - ALL_OPTION_FIELDS)
            fail(
                f"{path.relative_to(ROOT)} endpoint option fields drifted from endpoint_surface.toml. "
                f"missing={missing or '[]'} extra={extra or '[]'}"
            )

    go_fields = extract_struct_fields(
        ROOT / "sdks/go/endpoint_options.go",
        r"type EndpointRequestOptions struct \{(.*?)\n\}",
        r"^\s*([A-Z][A-Za-z0-9]+)\s+\*",
    )
    expected_go_fields = {snake_to_go(name) for name in ALL_OPTION_FIELDS}
    if go_fields != expected_go_fields:
        missing = sorted(expected_go_fields - go_fields)
        extra = sorted(go_fields - expected_go_fields)
        fail(
            "sdks/go/endpoint_options.go EndpointRequestOptions fields drifted from endpoint_surface.toml. "
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
        ROOT / "sdks/go/ffi_bridge.h",
        ROOT / "sdks/cpp/include/thetadx.h",
        ROOT / "sdks/go/client.go",
        ROOT / "sdks/cpp/include/thetadx.hpp",
        ROOT / "sdks/cpp/src/thetadx.cpp",
        ROOT / "sdks/go/README.md",
        ROOT / "sdks/cpp/README.md",
        ROOT / "docs-site/docs/historical/option/history/greeks-eod.md",
    ]:
        expect_not_contains(path, "OptionRequestOptions")
        expect_not_contains(path, "TdxOptionRequestOptions")


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
    check_api_reference()
    check_openapi()
    check_endpoint_option_surface()
    check_tier_badges()
    print("docs consistency: ok")


if __name__ == "__main__":
    main()
