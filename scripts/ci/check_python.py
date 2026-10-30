#!/usr/bin/env python3
"""Live parameter-mode matrix validator for the Python SDK.

Runs every cell in `data/live_cells.json`, which
`cargo run -p xtask --bin generate_sdk_surfaces` renders from the endpoint
table before a live run (the file is not committed): one (endpoint, mode)
pair, its positional arguments and its keyword options. Modes cover
concrete fixtures plus wildcard / ISO-date variants for option ContractSpec
endpoints and one cell per optional parameter. Every cell is attempted
against production; the server is the ground truth for what the account can
access. Cells whose documented `min_tier` exceeds the live account tier come
back as a `SubscriptionError` and are classified `SKIP: tier-permission`.
Real configuration bugs (invalid arguments, wire-format errors) surface as
`FAIL`.

Each cell carries a per-call deadline (`timeout_ms`): 60 seconds, or 180 for
a chain-wide cell, whose payload legitimately takes longer than a minute. On
expiry the SDK cancels the in-flight gRPC stream and raises an error carrying
"Request deadline exceeded"; the client stays usable for the next cell.

Usage: check_python.py CREDS_FILE
"""
import json
import pathlib
import sys
import time

from thetadatadx import Credentials, Config, Client, SubscriptionError

HERE = pathlib.Path(__file__).resolve().parent
CELLS = json.loads((HERE / "data" / "live_cells.json").read_text())["cells"]
ARTIFACT_PATH = HERE.parents[1] / "artifacts" / "validator_python.json"

client = Client(Credentials.from_file(sys.argv[1]), Config.production())
market_data = client.market_data

pass_count = skip_count = fail_count = 0
records: list[dict] = []  # one record per cell for the agreement check


def _row_count(result):
    """Row count of a result: a tick list, a string list, or None."""
    if result is None:
        return 0
    try:
        return len(result)
    except TypeError:
        return 1


for cell in CELLS:
    endpoint, mode = cell["endpoint"], cell["mode"]
    label = f"{endpoint}::{mode}"
    record = {"endpoint": endpoint, "mode": mode, "rationale": cell["rationale"]}
    t0 = time.monotonic()
    try:
        value = getattr(market_data, endpoint)(*cell["args"], **cell["options"])
    except BaseException as exc:  # noqa: BLE001
        duration_ms = int((time.monotonic() - t0) * 1000)
        msg = str(exc).lower()
        if "request deadline exceeded" in msg:
            status, detail = "FAIL", f"timeout after {duration_ms // 1000}s"
            print(f"  {label:60s} FAIL  {detail}", flush=True)
        # Classify the entitlement skip on the typed error the SDK raises,
        # not on prose: a wiring regression whose message happens to mention
        # a permission or subscription must still count as a failure, the
        # sole input to the exit code.
        elif isinstance(exc, SubscriptionError):
            status, detail = "SKIP", "tier-permission"
            print(f"  {label:60s} SKIP: tier-permission (declared min_tier={cell['min_tier']})", flush=True)
        elif "no data found" in msg:
            status, detail = "PASS", "no data"
            print(f"  {label:60s} PASS  (no data)", flush=True)
        else:
            # Escape multi-line messages so the agreement-table row stays on
            # one line (see scripts/ci/check_agreement.py).
            status, detail = "FAIL", str(exc).replace("\n", "\\n").replace("\r", "\\r")[:200]
            print(f"  {label:60s} FAIL  {exc}", flush=True)
        row_count = 0
    else:
        duration_ms = int((time.monotonic() - t0) * 1000)
        status, detail, row_count = "PASS", "", _row_count(value)
        print(f"  {label:60s} PASS", flush=True)
    pass_count += status == "PASS"
    skip_count += status == "SKIP"
    fail_count += status == "FAIL"
    records.append({**record, "status": status, "row_count": row_count, "duration_ms": duration_ms, "detail": detail})

print(f"\nPython: {pass_count} PASS, {skip_count} SKIP, {fail_count} FAIL", flush=True)
print(f"COUNTS:{pass_count}:{skip_count}:{fail_count}", flush=True)
ARTIFACT_PATH.parent.mkdir(parents=True, exist_ok=True)
ARTIFACT_PATH.write_text(json.dumps({"lang": "python", "records": records}, indent=2, sort_keys=True))
print(f"artifact: {ARTIFACT_PATH}", flush=True)
# An empty result is a failure to measure, not a pass. Every cell can land in
# the SKIP branch (a lapsed entitlement, a vendor-side outage refusing every
# request) and the exit code, derived from `fail_count` alone, would still be
# 0. The live workflow reads only that code, and the release validator sums
# these counts into its failure total, where zero passes is indistinguishable
# from zero failures.
if pass_count == 0:
    print(
        f"\nPython: nothing passed ({skip_count} SKIP, {fail_count} FAIL of "
        f"{len(CELLS)} cells). An all-skip run measures nothing and is not a pass.",
        file=sys.stderr,
        flush=True,
    )
    sys.exit(1)

sys.exit(1 if fail_count > 0 else 0)
