#!/usr/bin/env python3
"""Validate `<TierBadge tier="..." />` badges in docs-site against upstream truth.

The authoritative source is ThetaData's OpenAPI YAML
(https://docs.thetadata.us/openapiv3.yaml), which encodes the minimum
subscription tier of every endpoint in a top-level `x-min-subscription:`
field. We keep a checked-in snapshot of that mapping in
`scripts/upstream_tiers.json` (refreshed by hand, traceable via
`_source` / `_captured_at`).

This script walks `docs-site/docs/historical/**/*.md`, extracts each page's
`<TierBadge tier="..." />`, maps the docs path to an upstream endpoint path,
and fails if any badge disagrees with the snapshot. Also fails if a docs
page maps to an endpoint absent from the snapshot (so new endpoints surface
loudly), apart from a small allow-list of endpoints upstream doesn't
document.

No network calls at CI time -- the snapshot is the contract.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SNAPSHOT_PATH = ROOT / "scripts/upstream_tiers.json"
HISTORICAL_ROOT = ROOT / "docs-site/docs/historical"

# Docs pages that intentionally have no upstream counterpart. These are
# endpoints ThetaDataDx exposes (or static informational pages) that the
# upstream ThetaData OpenAPI schema doesn't cover. Keys are paths relative
# to `docs-site/docs/`.
ALLOWLIST: dict[str, str | None] = {
    "historical/rate/eod.md": None,
}

TIER_RE = re.compile(r'<TierBadge\s+tier="([^"]+)"\s*/>')

# Segment-level special cases: docs path component -> upstream component.
SEGMENT_REWRITES: dict[str, str] = {
    "index-data": "index",
    "at-time": "at_time",
}

# Leaf-file stem rewrites for compound names that don't round-trip via
# simple hyphen-to-underscore.
GREEK_LEAF_REWRITES: dict[str, str] = {
    "greeks-iv": "greeks/implied_volatility",
    "greeks-all": "greeks/all",
    "greeks-eod": "greeks/eod",
    "greeks-first-order": "greeks/first_order",
    "greeks-second-order": "greeks/second_order",
    "greeks-third-order": "greeks/third_order",
    "trade-greeks-iv": "trade_greeks/implied_volatility",
    "trade-greeks-all": "trade_greeks/all",
    "trade-greeks-first-order": "trade_greeks/first_order",
    "trade-greeks-second-order": "trade_greeks/second_order",
    "trade-greeks-third-order": "trade_greeks/third_order",
}

# Full-path rewrites for endpoints whose docs path can't be mechanically
# derived. Keys are docs paths relative to `docs-site/docs/`.
PATH_REWRITES: dict[str, str] = {
    "historical/calendar/open-today.md": "/calendar/today",
    "historical/calendar/year.md": "/calendar/year_holidays",
    "historical/option/list/roots.md": "/option/list/symbols",
    "historical/option/list/dates.md": "/option/list/dates/{request_type}",
    "historical/option/list/contracts.md": "/option/list/contracts/{request_type}",
    "historical/stock/list/dates.md": "/stock/list/dates/{request_type}",
}


def fail(message: str) -> None:
    print(f"tier badge check error: {message}", file=sys.stderr)
    raise SystemExit(1)


def docs_path_to_endpoint(rel_path: str) -> str | None:
    """Map a docs path (relative to docs-site/docs/) to an upstream endpoint.

    Returns None for files that are neither endpoint docs nor covered by
    explicit rewrites (e.g. section index.md pages).
    """
    if rel_path in PATH_REWRITES:
        return PATH_REWRITES[rel_path]

    # Drop the `historical/` prefix.
    if not rel_path.startswith("historical/"):
        return None
    tail = rel_path[len("historical/") :]
    if tail.endswith("/index.md") or tail == "index.md":
        return None
    if not tail.endswith(".md"):
        return None

    stem_path = tail[: -len(".md")]
    parts = stem_path.split("/")
    if len(parts) < 2:
        return None

    leaf = parts[-1]
    parents = parts[:-1]

    # Handle greek leaf special cases (these always live under `snapshot/`
    # or `history/` parent categories).
    if leaf in GREEK_LEAF_REWRITES:
        leaf_translated = GREEK_LEAF_REWRITES[leaf]
    else:
        leaf_translated = leaf.replace("-", "_")

    parents_translated = [SEGMENT_REWRITES.get(p, p).replace("-", "_") for p in parents]

    endpoint = "/" + "/".join(parents_translated + [leaf_translated])
    return endpoint


def find_tier_badge(text: str) -> str | None:
    match = TIER_RE.search(text)
    if match is None:
        return None
    return match.group(1)


def load_snapshot() -> dict[str, str]:
    if not SNAPSHOT_PATH.exists():
        fail(f"{SNAPSHOT_PATH.relative_to(ROOT)} missing")
    data = json.loads(SNAPSHOT_PATH.read_text())
    endpoints = data.get("endpoints")
    if not isinstance(endpoints, dict):
        fail(f"{SNAPSHOT_PATH.relative_to(ROOT)} has no `endpoints` object")
    return endpoints


def main() -> int:
    snapshot = load_snapshot()

    mismatches: list[tuple[str, str, str, str]] = []  # (rel, endpoint, docs_tier, upstream_tier)
    unmapped: list[tuple[str, str]] = []  # (rel, guessed_endpoint)
    missing_badge: list[str] = []

    for md_path in sorted(HISTORICAL_ROOT.rglob("*.md")):
        rel = md_path.relative_to(ROOT / "docs-site/docs").as_posix()
        text = md_path.read_text()
        badge = find_tier_badge(text)

        endpoint = docs_path_to_endpoint(rel)
        if endpoint is None:
            # Section index pages -- no endpoint, shouldn't have a badge.
            if badge is not None:
                fail(f"{rel} has a TierBadge but isn't an endpoint page")
            continue

        if rel in ALLOWLIST:
            # Allow-listed: no upstream tier to compare against, but if a
            # badge is present we just trust it. Skip further checks.
            continue

        if badge is None:
            missing_badge.append(rel)
            continue

        upstream = snapshot.get(endpoint)
        if upstream is None:
            unmapped.append((rel, endpoint))
            continue

        if badge != upstream:
            mismatches.append((rel, endpoint, badge, upstream))

    ok = True

    if missing_badge:
        ok = False
        print("Missing <TierBadge> on endpoint pages:", file=sys.stderr)
        for rel in missing_badge:
            print(f"  {rel}", file=sys.stderr)

    if unmapped:
        ok = False
        print(
            "Docs pages whose mapped endpoint is not in scripts/upstream_tiers.json "
            "(either refresh the snapshot, add to ALLOWLIST, or fix the mapping):",
            file=sys.stderr,
        )
        for rel, endpoint in unmapped:
            print(f"  {rel} -> {endpoint}", file=sys.stderr)

    if mismatches:
        ok = False
        print("TierBadge mismatches vs upstream:", file=sys.stderr)
        width_rel = max(len(m[0]) for m in mismatches)
        width_ep = max(len(m[1]) for m in mismatches)
        header = f"  {'docs path'.ljust(width_rel)}  {'endpoint'.ljust(width_ep)}  docs -> upstream"
        print(header, file=sys.stderr)
        print("  " + "-" * (len(header) - 2), file=sys.stderr)
        for rel, endpoint, docs_tier, upstream_tier in mismatches:
            print(
                f"  {rel.ljust(width_rel)}  {endpoint.ljust(width_ep)}  {docs_tier} -> {upstream_tier}",
                file=sys.stderr,
            )

    if not ok:
        return 1
    print("tier badges: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
