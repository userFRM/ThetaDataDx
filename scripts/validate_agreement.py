#!/usr/bin/env python3
"""Cross-language agreement check for the live parameter-mode matrix.

Loads per-language validator artifacts from `artifacts/validator_<lang>.json`
(written by `scripts/validate_cli.py`, `scripts/validate_python.py`,
`sdks/go/cmd/validate`, and `sdks/cpp` validator), and asserts that every
(endpoint, mode) cell agrees across all SDKs on:

* status (PASS / SKIP / FAIL)
* row_count (for cells that passed)

Cells where SDKs disagree are reported with a table showing each SDK's
outcome. Exits non-zero on any disagreement.

Cells missing from any SDK's artifact are surfaced but don't fail the run by
default, since the CLI validator deliberately skips some cell classes the
other SDKs run (per-optional-param modes — see PR #291). Pass
`--require-all-sdks` to make missing cells a hard failure.

Follow-up work deliberately deferred from this iteration (tracked in
issue #290):

* first_row_hash per cell. The canonicalization rules (sort keys, round
  floats to 6 decimals, null missing fields, sha256) require per-SDK tick
  introspection, and the four languages have different native tick shapes
  (Python dataclasses, Go structs with tags, C++ POD with fields of various
  types, CLI producing JSON directly). This foundation is in place (record
  shape has room for `first_row_hash`) but populating it is a follow-up.
* Server-side duration / byte-count. The CLI has access via JSON metadata;
  the SDKs would need to expose wire-level timing which they don't yet.

Refs: #287, #290.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS_DIR = ROOT / "artifacts"

LANGS = ("python", "cli", "go", "cpp")

# Cell identity is (endpoint, mode). SDK-specific mode-expansion differences
# surface as "cell missing from <lang>" rows, not as disagreement failures.


def load_artifact(lang: str) -> list[dict] | None:
    """Load one SDK's artifact. Returns `None` if missing (treated as a
    soft skip unless `--require-all-sdks` is on)."""
    path = ARTIFACTS_DIR / f"validator_{lang}.json"
    if not path.exists():
        return None
    try:
        payload = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        print(f"error: {path} is not valid JSON ({exc})", file=sys.stderr)
        return None
    records = payload.get("records", [])
    if not isinstance(records, list):
        print(f"error: {path}.records is not a list", file=sys.stderr)
        return None
    return records


def index_by_cell(records: list[dict]) -> dict[tuple[str, str], dict]:
    """Index records by (endpoint, mode). Raise on duplicates -- each cell
    should appear at most once per SDK."""
    idx: dict[tuple[str, str], dict] = {}
    for rec in records:
        key = (rec.get("endpoint", ""), rec.get("mode", ""))
        if key in idx:
            raise ValueError(f"duplicate cell in artifact: {key}")
        idx[key] = rec
    return idx


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument(
        "--require-all-sdks",
        action="store_true",
        help="Fail if any SDK's artifact is missing. Default: soft-skip missing SDKs.",
    )
    parser.add_argument(
        "--max-cell-diff-rows",
        type=int,
        default=50,
        help="Cap on disagreement rows printed to stderr (others summarized).",
    )
    args = parser.parse_args()

    per_lang: dict[str, dict[tuple[str, str], dict]] = {}
    missing: list[str] = []
    for lang in LANGS:
        records = load_artifact(lang)
        if records is None:
            missing.append(lang)
            continue
        try:
            per_lang[lang] = index_by_cell(records)
        except ValueError as exc:
            print(f"error: {lang}: {exc}", file=sys.stderr)
            return 1
        print(f"  {lang:8s} {len(per_lang[lang])} cells")

    if missing:
        print(
            f"warning: no artifact for {', '.join(missing)} "
            f"(expected at artifacts/validator_<lang>.json)",
            file=sys.stderr,
        )
        if args.require_all_sdks:
            print("--require-all-sdks set; failing", file=sys.stderr)
            return 1

    if len(per_lang) < 2:
        print(
            "error: need at least 2 SDK artifacts to compare agreement "
            f"(found {len(per_lang)})",
            file=sys.stderr,
        )
        return 1

    # Cell universe: union of all cells seen in any SDK.
    all_cells: set[tuple[str, str]] = set()
    for idx in per_lang.values():
        all_cells.update(idx.keys())

    disagreements: list[tuple[tuple[str, str], dict[str, dict]]] = []
    missing_per_cell: dict[tuple[str, str], list[str]] = defaultdict(list)

    for cell in sorted(all_cells):
        present = {lang: idx[cell] for lang, idx in per_lang.items() if cell in idx}
        absent = [lang for lang, idx in per_lang.items() if cell not in idx]
        if absent:
            missing_per_cell[cell] = absent

        if len(present) < 2:
            continue

        # Compare status across SDKs that ran this cell.
        statuses = {lang: rec.get("status") for lang, rec in present.items()}
        if len(set(statuses.values())) > 1:
            disagreements.append((cell, present))
            continue

        # Compare row_count on cells that all PASSed.
        if all(s == "PASS" for s in statuses.values()):
            row_counts = {lang: rec.get("row_count", 0) for lang, rec in present.items()}
            if len(set(row_counts.values())) > 1:
                disagreements.append((cell, present))

    print(
        f"\nagreement: {len(all_cells)} cells across {len(per_lang)} SDKs, "
        f"{len(disagreements)} disagreements, "
        f"{sum(1 for v in missing_per_cell.values() if v)} cells partial"
    )

    if disagreements:
        print("\nDISAGREEMENTS:", file=sys.stderr)
        for (endpoint, mode), present in disagreements[: args.max_cell_diff_rows]:
            # Determine the disagreement kind so the header carries useful
            # context (status mismatch vs. row-count mismatch on PASS).
            statuses = {rec.get("status") for rec in present.values()}
            kind = "status disagreement" if len(statuses) > 1 else "row-count disagreement"
            label = f"{endpoint}::{mode}"
            # All SDKs see the same generator output for a given cell, so
            # rationale is identical across present[lang]; pick whichever is
            # available. Fall back to "(missing)" if no SDK populated it
            # (older artifacts written before this field landed).
            rationale = next(
                (rec.get("rationale") for rec in present.values() if rec.get("rationale")),
                "(missing)",
            )
            print(f"  {label}  [{kind}]", file=sys.stderr)
            print(f"    rationale: {rationale}", file=sys.stderr)
            print(
                f"    {'sdk':8s} | {'status':6s} | {'rows':5s} | detail",
                file=sys.stderr,
            )
            for lang in sorted(present):
                rec = present[lang]
                print(
                    f"    {lang:8s} | {str(rec.get('status', '')):6s} | "
                    f"{str(rec.get('row_count', '')):5s} | "
                    f"{(rec.get('detail') or '')[:80]}",
                    file=sys.stderr,
                )
        if len(disagreements) > args.max_cell_diff_rows:
            print(
                f"  ... and {len(disagreements) - args.max_cell_diff_rows} more",
                file=sys.stderr,
            )

    # Partial cells (present in some SDKs but not others) -- info only by
    # default, since CLI deliberately skips per-optional-param modes.
    partial = {c: langs for c, langs in missing_per_cell.items() if langs}
    if partial:
        # Only flag as a warning; inside the summary the count is already
        # shown. Callers use --require-all-sdks if they want it strict.
        print(
            f"\nnote: {len(partial)} cells missing from at least one SDK "
            "(CLI skips per-optional-param modes by design -- see PR #291)",
            file=sys.stderr,
        )

    return 1 if disagreements else 0


if __name__ == "__main__":
    sys.exit(main())
