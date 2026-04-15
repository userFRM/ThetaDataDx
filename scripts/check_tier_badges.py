#!/usr/bin/env python3
"""Validate ``<TierBadge tier="..." />`` badges against upstream ThetaData.

The authoritative source is ThetaData's OpenAPI spec
(``https://docs.thetadata.us/openapiv3.yaml``), which encodes the minimum
subscription tier of every endpoint via a top-level
``x-min-subscription:`` field.

Source of truth: ``scripts/upstream_openapi.yaml`` (a checked-in snapshot
of the upstream YAML). The Rust validator generator parses the same file,
so both machinery and humans reason against one pinned schema. Refresh
the snapshot by rerunning this script with ``--refresh-snapshot``; the
fetch path below is retained for that single purpose.

The script walks ``docs-site/docs/historical/**/*.md``, extracts each
page's ``<TierBadge tier="..." />``, maps the docs path to an upstream
endpoint path, and fails if any badge disagrees with upstream. Also fails
if a docs page maps to an endpoint absent from upstream (so new endpoints
surface loudly), apart from a small allow-list of endpoints upstream
doesn't document.
"""

from __future__ import annotations

import argparse
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
HISTORICAL_ROOT = ROOT / "docs-site/docs/historical"
SNAPSHOT_PATH = ROOT / "scripts/upstream_openapi.yaml"

UPSTREAM_URL = "https://docs.thetadata.us/openapiv3.yaml"
FETCH_TIMEOUT_SECS = 30
FETCH_MAX_ATTEMPTS = 4  # ~= 0s, 2s, 4s, 8s backoff cumulative <= 15s

# Docs pages that intentionally have no upstream counterpart. These are
# endpoints ThetaDataDx exposes (or static informational pages) that the
# upstream ThetaData OpenAPI schema doesn't cover. Keys are paths relative
# to ``docs-site/docs/``.
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
# derived. Keys are docs paths relative to ``docs-site/docs/``.
PATH_REWRITES: dict[str, str] = {
    "historical/calendar/open-today.md": "/calendar/today",
    "historical/calendar/year.md": "/calendar/year_holidays",
    "historical/option/list/roots.md": "/option/list/symbols",
    "historical/option/list/dates.md": "/option/list/dates/{request_type}",
    "historical/option/list/contracts.md": "/option/list/contracts/{request_type}",
    "historical/stock/list/dates.md": "/stock/list/dates/{request_type}",
}

# OpenAPI YAML is large but highly regular: every endpoint starts with
# two spaces + `/` + path + `:`, and every `x-min-subscription:` line is
# indented four spaces. Avoid adding a PyYAML dep just to pull two fields
# out of one file.
ENDPOINT_LINE_RE = re.compile(r"^  (/\S+?):\s*$")
TIER_LINE_RE = re.compile(r"^    x-min-subscription:\s+(\S+)")


def fail(message: str) -> None:
    print(f"tier badge check error: {message}", file=sys.stderr)
    raise SystemExit(1)


def fetch_upstream_yaml() -> str:
    """Fetch the upstream OpenAPI YAML with retries. Fail-closed on exhaustion."""
    last_err: Exception | None = None
    for attempt in range(1, FETCH_MAX_ATTEMPTS + 1):
        try:
            with urllib.request.urlopen(UPSTREAM_URL, timeout=FETCH_TIMEOUT_SECS) as resp:  # noqa: S310
                return resp.read().decode("utf-8")
        except (urllib.error.URLError, TimeoutError, ConnectionError) as exc:
            last_err = exc
            if attempt == FETCH_MAX_ATTEMPTS:
                break
            sleep_s = 2 ** (attempt - 1)
            print(
                f"  upstream fetch attempt {attempt}/{FETCH_MAX_ATTEMPTS} failed "
                f"({exc}); retrying in {sleep_s}s...",
                file=sys.stderr,
            )
            time.sleep(sleep_s)
    fail(
        f"could not fetch {UPSTREAM_URL} after {FETCH_MAX_ATTEMPTS} attempts: {last_err}"
    )
    return ""  # unreachable; fail() raises


def read_snapshot_yaml() -> str:
    if not SNAPSHOT_PATH.exists():
        fail(
            f"snapshot missing at {SNAPSHOT_PATH.relative_to(ROOT)}; "
            "run `python3 scripts/check_tier_badges.py --refresh-snapshot` to populate."
        )
    return SNAPSHOT_PATH.read_text()


def refresh_snapshot() -> None:
    """Fetch the live upstream YAML and overwrite the committed snapshot.

    Prepends a frontmatter block (``_captured_at``, ``_source``,
    ``_refresh_with``) so later readers know when and how the snapshot was
    taken. The parser here and the Rust generator both tolerate leading
    ``#`` comment lines because they only match on indentation-specific
    structural lines.
    """
    print(f"refreshing {SNAPSHOT_PATH.relative_to(ROOT)} from {UPSTREAM_URL} ...", flush=True)
    body = fetch_upstream_yaml()
    # UTC ISO-8601 timestamp, second precision -- no need for millis here.
    captured_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    frontmatter = (
        f"# _captured_at: {captured_at}\n"
        f"# _source: {UPSTREAM_URL}\n"
        "# _refresh_with: python3 scripts/check_tier_badges.py --refresh-snapshot\n"
        "#\n"
        "# This file is the pinned upstream ThetaData OpenAPI v3 spec. Both the\n"
        "# tier-badge check (scripts/check_tier_badges.py) and the Rust validator\n"
        "# generator (crates/thetadatadx/build_support/endpoints.rs) derive endpoint\n"
        "# min-subscription tiers and expiration-wildcard support from it, so both\n"
        "# human docs and generated machinery agree on one pinned schema. Refresh\n"
        "# with the command above when ThetaData publishes a new spec.\n"
    )
    content = frontmatter + body
    SNAPSHOT_PATH.parent.mkdir(parents=True, exist_ok=True)
    SNAPSHOT_PATH.write_text(content)
    line_count = content.count("\n") + (0 if content.endswith("\n") else 1)
    print(f"  wrote {line_count} lines to {SNAPSHOT_PATH.relative_to(ROOT)} (captured_at={captured_at})")


def parse_tier_mapping(yaml_text: str) -> dict[str, str]:
    """Extract ``path -> x-min-subscription`` from the OpenAPI YAML."""
    mapping: dict[str, str] = {}
    current_path: str | None = None
    for line in yaml_text.splitlines():
        m = ENDPOINT_LINE_RE.match(line)
        if m:
            current_path = m.group(1)
            continue
        m = TIER_LINE_RE.match(line)
        if m and current_path is not None:
            mapping[current_path] = m.group(1)
    if not mapping:
        fail("parsed 0 endpoints from upstream YAML -- schema changed?")
    return mapping


def docs_path_to_endpoint(rel_path: str) -> str | None:
    """Map a docs path (relative to docs-site/docs/) to an upstream endpoint.

    Returns None for files that are neither endpoint docs nor covered by
    explicit rewrites (e.g. section index.md pages).
    """
    if rel_path in PATH_REWRITES:
        return PATH_REWRITES[rel_path]

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

    if leaf in GREEK_LEAF_REWRITES:
        leaf_translated = GREEK_LEAF_REWRITES[leaf]
    else:
        leaf_translated = leaf.replace("-", "_")

    parents_translated = [SEGMENT_REWRITES.get(p, p).replace("-", "_") for p in parents]

    return "/" + "/".join(parents_translated + [leaf_translated])


def find_tier_badge(text: str) -> str | None:
    match = TIER_RE.search(text)
    if match is None:
        return None
    return match.group(1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument(
        "--refresh-snapshot",
        action="store_true",
        help=(
            "Fetch the live upstream YAML and overwrite "
            "scripts/upstream_openapi.yaml, then exit."
        ),
    )
    args = parser.parse_args()

    if args.refresh_snapshot:
        refresh_snapshot()
        return 0

    print(f"reading {SNAPSHOT_PATH.relative_to(ROOT)} ...", flush=True)
    yaml_text = read_snapshot_yaml()
    upstream = parse_tier_mapping(yaml_text)
    print(f"  parsed {len(upstream)} upstream endpoints", flush=True)

    mismatches: list[tuple[str, str, str, str]] = []
    unmapped: list[tuple[str, str]] = []
    missing_badge: list[str] = []

    for md_path in sorted(HISTORICAL_ROOT.rglob("*.md")):
        rel = md_path.relative_to(ROOT / "docs-site/docs").as_posix()
        text = md_path.read_text()
        badge = find_tier_badge(text)

        endpoint = docs_path_to_endpoint(rel)
        if endpoint is None:
            if badge is not None:
                fail(f"{rel} has a TierBadge but isn't an endpoint page")
            continue

        if rel in ALLOWLIST:
            continue

        if badge is None:
            missing_badge.append(rel)
            continue

        upstream_tier = upstream.get(endpoint)
        if upstream_tier is None:
            unmapped.append((rel, endpoint))
            continue

        if badge != upstream_tier:
            mismatches.append((rel, endpoint, badge, upstream_tier))

    ok = True

    if missing_badge:
        ok = False
        print("Missing <TierBadge> on endpoint pages:", file=sys.stderr)
        for rel in missing_badge:
            print(f"  {rel}", file=sys.stderr)

    if unmapped:
        ok = False
        print(
            "Docs pages whose mapped endpoint is not in upstream openapiv3.yaml "
            "(add to ALLOWLIST if upstream doesn't document it, or fix the mapping):",
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
