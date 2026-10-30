#!/usr/bin/env python3
"""Refresh the pinned upstream OpenAPI snapshot.

``scripts/ci/data/upstream_openapi.yaml`` is a capture of ThetaData's
published OpenAPI spec. The SDK surface generator reads each endpoint's
minimum subscription tier from it, so the reference pages' tier badges and
the OpenAPI document follow the capture. Run with ``--refresh-snapshot``
when ThetaData publishes a new spec, then rerun the generator.
"""

from __future__ import annotations

import argparse
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

SNAPSHOT_PATH = Path(__file__).resolve().parents[2] / "scripts/ci/data/upstream_openapi.yaml"
UPSTREAM_URL = "https://docs.thetadata.us/openapiv3.yaml"
FETCH_ATTEMPTS = 4

FRONTMATTER = """\
# _captured_at: {captured_at}
# _source: {source}
# _refresh_with: python3 scripts/ci/check_tier_badges.py --refresh-snapshot
#
# This file is the pinned upstream ThetaData OpenAPI v3 spec. The SDK surface
# generator (xtask/src/bin/generate_sdk_surfaces/) derives endpoint
# min-subscription tiers and expiration-wildcard support from it, so the docs
# and the generated surfaces agree on one pinned schema. Refresh with the
# command above when ThetaData publishes a new spec.
"""


def fetch() -> str:
    with urllib.request.urlopen(UPSTREAM_URL, timeout=30) as resp:  # noqa: S310
        return resp.read().decode("utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--refresh-snapshot",
        action="store_true",
        required=True,
        help=f"fetch {UPSTREAM_URL} and overwrite {SNAPSHOT_PATH.name}",
    )
    parser.parse_args()

    for attempt in range(1, FETCH_ATTEMPTS):
        try:
            body = fetch()
            break
        except (urllib.error.URLError, TimeoutError, ConnectionError) as exc:
            print(f"fetch attempt {attempt} failed ({exc}); retrying", file=sys.stderr)
            time.sleep(2 ** (attempt - 1))
    else:
        body = fetch()

    captured_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    SNAPSHOT_PATH.write_text(FRONTMATTER.format(captured_at=captured_at, source=UPSTREAM_URL) + body)
    print(f"wrote {SNAPSHOT_PATH} (captured_at={captured_at})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
