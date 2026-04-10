#!/usr/bin/env python3
"""Validate checked-in documentation against the endpoint surface spec.

This script keeps the human-facing docs aligned with the current SDK and REST
surface by checking a few high-signal invariants:

- endpoint/tool counts in top-level docs
- REST/OpenAPI path + operationId parity with `endpoint_surface.toml`
- docs-site API reference coverage for every endpoint
- current server docs staying on the v3 path scheme and current defaults
"""

from __future__ import annotations

from pathlib import Path
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
SURFACE = tomllib.loads((ROOT / "crates/thetadatadx/endpoint_surface.toml").read_text())
ENDPOINTS = SURFACE["endpoints"]
ENDPOINT_NAMES = {ep["name"] for ep in ENDPOINTS}
REST_PATHS = {ep["rest_path"].removeprefix("/v3") for ep in ENDPOINTS}
EXPECTED_TOOL_COUNT = len(ENDPOINTS) + 3


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
        f"{len(ENDPOINTS)} registry endpoints + 3 offline tools (ping, all_greeks, implied_volatility) = {EXPECTED_TOOL_COUNT} total.",
    )

    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        f"## Available Tools ({EXPECTED_TOOL_COUNT})",
    )
    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        f"{len(ENDPOINTS)} data endpoints + ping + all_greeks + implied_volatility = {EXPECTED_TOOL_COUNT} tools.",
    )
    expect_contains(
        ROOT / "docs-site/docs/.vitepress/config.ts",
        "{ text: 'Migration from REST & WS', link: '/getting-started/migration-from-rest-ws' }",
    )
    expect_contains(
        ROOT / "docs-site/docs/getting-started/index.md",
        "[Migration from REST & WebSocket](./migration-from-rest-ws)",
    )
    expect_contains(
        ROOT / "docs-site/docs/tools/mcp.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
    )
    expect_contains(
        ROOT / "tools/mcp/README.md",
        'Use `"strike":"0"` when you want a bulk chain-style response',
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
    if actual_paths != REST_PATHS:
        missing = sorted(REST_PATHS - actual_paths)
        extra = sorted(actual_paths - REST_PATHS)
        fail(
            "docs-site/public/thetadatadx.yaml path set drifted. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )

    actual_ops = {
        match.group(1) for match in re.finditer(r"^\s*operationId:\s*(\S+)", text, re.MULTILINE)
    }
    expected_ops = {lower_camel(ep["name"]) for ep in ENDPOINTS}
    if actual_ops != expected_ops:
        missing = sorted(expected_ops - actual_ops)
        extra = sorted(actual_ops - expected_ops)
        fail(
            "docs-site/public/thetadatadx.yaml operationId set drifted. "
            f"missing={missing or '[]'} extra={extra or '[]'}"
        )


def main() -> None:
    check_static_docs()
    check_api_reference()
    check_openapi()
    print("docs consistency: ok")


if __name__ == "__main__":
    main()
