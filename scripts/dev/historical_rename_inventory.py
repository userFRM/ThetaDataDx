#!/usr/bin/env python3
"""Inventory + classify every `historical` occurrence for the historical->market_data rename.

Scans the tree (skipping build/vcs/dep dirs), finds every case-insensitive
`historical` hit, and buckets each line so the rename can be driven and then
verified. The `*_history_*` endpoints use "history", not "historical", so they
never match here.

Buckets:
  IDENT   - code identifiers to rename mechanically (types, symbols, env vars,
            config keys, namespace accessors/fields).
  CONCEPT - prose referring to the client/channel/namespace concept -> rewrite
            to "market data".
  DATA    - prose referring to historical DATA (the past-data category) -> keep.
  REVIEW  - anything else; eyeball it.

Usage:
  python3 scripts/dev/historical_rename_inventory.py            # summary + REVIEW list
  python3 scripts/dev/historical_rename_inventory.py --all      # every hit, by bucket
  python3 scripts/dev/historical_rename_inventory.py --bucket IDENT
"""
from __future__ import annotations
import os, re, sys, argparse

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SKIP_DIRS = {".git", "target", "node_modules", "dist", ".vitepress/dist", "__pycache__", ".claude"}
SKIP_EXT = {".lock", ".png", ".jpg", ".svg", ".ico", ".woff", ".woff2", ".node", ".zst", ".pb"}

HIST = re.compile(r"historical", re.IGNORECASE)

# Identifier tokens that must be renamed mechanically. Order-independent; a line
# matching any of these is IDENT.
IDENT_PAT = re.compile(
    r"MarketDataClient|MarketDataConfig|MarketDataEnvironment|MarketDataView"
    r"|ThetaDataDxHistorical|thetadatadx_market_data|thetadatadx_config_(get|set)_historical"
    r"|THETADATA_MARKET_DATA|market_data_environment|with_historical|market_data_type|marketDataType"
    r"|historical[-_]region|historical[-_]host|historical[-_]port"
    r"|get_historical|set_historical"
    r"|\[historical\]|\"historical\"|'historical'|\.market_data\b|historical:\s"
    r"|mod market_data|::market_data\b|historical_view|render_python_historical"
)
# Prose about the CLIENT/CHANNEL concept -> becomes "market data".
CONCEPT_PAT = re.compile(
    r"historical[- ](client|channel|view|namespace|connection|grpc|gRPC|sub-namespace|query|request|endpoint|method|surface|config|environment|region|host|port|pool|only)",
    re.IGNORECASE,
)
# Prose about the DATA category (past data) -> keep as-is.
DATA_PAT = re.compile(r"historical[- ](data|price|prices|bar|bars|record|quote|trade|response|frame|row)", re.IGNORECASE)

def classify(line: str) -> str:
    if IDENT_PAT.search(line):
        return "IDENT"
    if CONCEPT_PAT.search(line):
        return "CONCEPT"
    if DATA_PAT.search(line):
        return "DATA"
    return "REVIEW"

def walk():
    for dp, dns, fns in os.walk(ROOT):
        dns[:] = [d for d in dns if d not in SKIP_DIRS and not (os.path.relpath(os.path.join(dp, d), ROOT) in SKIP_DIRS)]
        for fn in fns:
            if os.path.splitext(fn)[1] in SKIP_EXT:
                continue
            p = os.path.join(dp, fn)
            rel = os.path.relpath(p, ROOT)
            if rel.startswith("scripts/dev/historical_rename_inventory.py"):
                continue
            try:
                with open(p, encoding="utf-8") as f:
                    for i, line in enumerate(f, 1):
                        if HIST.search(line):
                            yield rel, i, line.rstrip(), classify(line)
            except (UnicodeDecodeError, IsADirectoryError, PermissionError):
                continue

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--bucket", choices=["IDENT", "CONCEPT", "DATA", "REVIEW"])
    a = ap.parse_args()
    hits = list(walk())
    from collections import Counter, defaultdict
    bybucket = Counter(h[3] for h in hits)
    byfile = defaultdict(lambda: Counter())
    for rel, i, line, b in hits:
        byfile[rel][b] += 1
    print(f"total `historical` hits: {len(hits)}  across {len(byfile)} files")
    for b in ("IDENT", "CONCEPT", "DATA", "REVIEW"):
        print(f"  {b:8} {bybucket[b]}")
    print()
    if a.bucket:
        for rel, i, line, b in hits:
            if b == a.bucket:
                print(f"{rel}:{i}: {line.strip()[:160]}")
    elif a.all:
        for rel, i, line, b in hits:
            print(f"[{b}] {rel}:{i}: {line.strip()[:140]}")
    else:
        print("REVIEW (need eyes):")
        for rel, i, line, b in hits:
            if b == "REVIEW":
                print(f"  {rel}:{i}: {line.strip()[:150]}")
        print("\nfiles by bucket mix:")
        for rel in sorted(byfile):
            c = byfile[rel]
            print(f"  {rel}: " + " ".join(f"{k}={v}" for k, v in sorted(c.items())))

if __name__ == "__main__":
    main()
