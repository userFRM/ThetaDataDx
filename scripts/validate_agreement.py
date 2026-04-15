#!/usr/bin/env python3
"""Cross-language agreement check for the live parameter-mode matrix.

Loads per-language validator artifacts from `artifacts/validator_<lang>.json`
(written by `scripts/validate_cli.py`, `scripts/validate_python.py`,
`sdks/go/cmd/validate`, and `sdks/cpp` validator) and compares every
(endpoint, mode) cell across SDKs on:

* status (PASS / SKIP / FAIL)
* row_count (for cells that all passed)
* first_row contents (field-by-field, when the artifact carries a
  canonicalized `first_row` dict)

Disagreements are reported as per-cell tables that pinpoint the exact
field values that differ across SDKs. Exits non-zero on any
disagreement.

Cells missing from any SDK's artifact are surfaced as "partial" but
don't fail the run by default, since the CLI validator deliberately
skips some per-optional-param modes the other SDKs run (PR #291). Pass
`--require-all-sdks` to make missing cells a hard failure.

Artifact format:

    {"lang": "...",
     "records": [
       {"endpoint": "stock_snapshot_ohlc", "mode": "concrete",
        "status": "PASS", "row_count": 1, "duration_ms": 120,
        "detail": "", "first_row": {"bid": 685.86, ...}   # optional
       }, ...
     ]}

`first_row` is optional. When at least two SDKs populate it for the
same cell, the cell is compared field-by-field; otherwise the diff
falls back to status + row_count.

Canonical `first_row` contract
------------------------------

Every producer MUST emit raw numeric values with no presentation
formatting. The contract is enforced consumer-side by `_canonicalize_row`
but producers are encouraged to emit the raw form directly to keep
artifacts human-readable:

* **dates**                : YYYYMMDD ints (e.g. `20260417`), NOT strings like `"2026-04-17"`.
                             The sentinel `0` (no date) is passed through verbatim
                             by every SDK; the consumer collapses it to `None`.
* **ms_of_day**            : raw i32 ms since midnight (e.g. `34200000`), NOT `"HH:MM:SS.mmm"`.
                             Negative values (sentinel for "missing") are passed through
                             verbatim; the consumer collapses them to `None`.
* **prices**               : f64 rounded to 6 decimals (e.g. `685.86`)
* **sizes / counts / ids** : raw ints (NOT sentinel-normalized -- `volume == 0` is a
                             legitimate trading value and must stay distinct from missing)
* **enum codes**           : raw ints (not stringified names)
* **keys**                 : lowercase; mixed-case producers are normalized on load
* **missing fields**       : omitted from the dict (distinguishable from `null`)
* **non-finite floats**    : `NaN` / `Inf` normalized to `null`

Presentation strings like `"2026-04-17"` would cause false diffs the
moment a second SDK starts populating `first_row` with raw ints. The
CLI emitter calls the `tdx` binary with `--format json-raw` so its
output matches the contract; adding that flag was part of PR #293.

Consumer-side canonicalization (`_canonicalize_row`) handles:

1. recursive lowercase dict keys (so `{"Bid": ...}` compares equal to
   `{"bid": ...}` even when one producer regressed)
2. recursive 6-decimal float rounding (collapses 1-ULP JSON round-trip
   noise; also catches `685.86` vs `685.860001`)
3. NaN / +Inf / -Inf normalized to Python `None` — cross-language
   serialization of non-finite floats is ambiguous (JSON rejects them
   outright; CLI's f64 reparse at tools/cli/src/main.rs:270 drops them
   silently), so we collapse all three to a single unambiguous sentinel
4. date-shaped fields (`date`, `expiration`, or ending in `_date`):
   value `0` -> `None`. Every SDK emits the sentinel verbatim (Python
   `sdks/python/src/tick_columnar.rs:7,38`; Go `sdks/go/tick_structs.go:10,35`;
   server `tools/server/src/format.rs:346`). Without this normalization,
   a producer that happens to see `date == 0` (no-data cell, pre-market
   snapshot) would false-diff against one that serializes the same
   cell as `null`.
5. ms-shaped fields (`ms_of_day`, `ms_of_day2`, `quote_ms_of_day`,
   `open_time`, `close_time`, `time`, `quote_time`, or ending in
   `_time`): negative value -> `None`. Same reasoning -- every SDK
   emits negative-ms sentinels as raw ints.

Defense in depth: producers SHOULD canonicalize too, but the consumer
is the authoritative enforcer. A producer bug (mixed-case key, stray
NaN, float precision regression, sentinel mapped to `null` vs raw int)
won't silently turn into a false disagreement.

Refs: #287, #290, #291, #292, #293.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS_DIR = ROOT / "artifacts"

LANGS = ("python", "cli", "go", "cpp")

# Rounding precision for float comparison in canonicalized first rows.
# 6 decimals matches the canonicalization contract documented in the
# per-SDK emitters. SDKs with raw float precision above this must round
# before emitting.
FLOAT_PRECISION = 6


def load_artifact(lang: str, artifacts_dir: Path) -> list[dict] | None:
    """Load one SDK's artifact. Returns None if missing (treated as a
    soft skip unless --require-all-sdks is on)."""
    path = artifacts_dir / f"validator_{lang}.json"
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


# Exact-match field names that carry YYYYMMDD date semantics. Listed
# against the lowercased canonical form produced by `_canonicalize_row`.
# Any i32 `0` in these fields is a sentinel for "no date" (trading data
# never has year `0000`) and canonicalizes to None.
_DATE_FIELD_NAMES: frozenset[str] = frozenset({"date", "expiration"})

# Exact-match field names that carry ms-of-day semantics. Any value `< 0`
# is a sentinel for "missing" (ms of day is non-negative by construction,
# bounded by 86_400_000) and canonicalizes to None. CalendarDay's
# `open_time` / `close_time` and CLI column aliases `time` / `quote_time`
# are included because they hold the same i32 ms-of-day underlying value.
_MS_FIELD_NAMES: frozenset[str] = frozenset(
    {
        "ms_of_day",
        "ms_of_day2",
        "quote_ms_of_day",
        "open_time",
        "close_time",
        "time",
        "quote_time",
    }
)

# Strike-shaped field names. Sentinel `0.0` -> None. Trading data has no
# zero-strike options.
_STRIKE_FIELD_NAMES: frozenset[str] = frozenset({"strike"})

# Right-shaped field names. Tick types emit `right` as `"C" / "P" / ""`
# (Python `tick_columnar.rs:41`, Go `RightStr`); `OptionContract` uses
# raw int 67/80/0. Empty string OR int 0 are both sentinels -> None.
_RIGHT_FIELD_NAMES: frozenset[str] = frozenset({"right"})

# Union of all sentinel-shaped field names for omit-vs-null normalization.
# A producer that omits the field (Go `omitempty`, server skip-when-zero)
# is equivalent to one that emits `null` or the raw sentinel value (Python
# tick_columnar emits 0/`""` verbatim). The consumer strips post-canonical
# `None` values for these fields so all three shapes converge to "absent".
_SENTINEL_SHAPED_FIELDS: frozenset[str] = (
    _DATE_FIELD_NAMES | _MS_FIELD_NAMES | _STRIKE_FIELD_NAMES | _RIGHT_FIELD_NAMES
)


def _is_date_field(name: str) -> bool:
    """True if `name` holds YYYYMMDD date semantics under the canonical
    contract. Matches exact-listed names plus any `_date` suffix so
    future tick-type additions pick up the rule automatically."""
    return name in _DATE_FIELD_NAMES or name.endswith("_date")


def _is_ms_field(name: str) -> bool:
    """True if `name` holds ms-of-day semantics under the canonical
    contract. Matches exact-listed names plus any `_time` suffix."""
    return name in _MS_FIELD_NAMES or name.endswith("_time")


def _is_strike_field(name: str) -> bool:
    """True if `name` holds option-strike semantics. `0.0` is sentinel."""
    return name in _STRIKE_FIELD_NAMES or name.endswith("_strike")


def _is_right_field(name: str) -> bool:
    """True if `name` holds option-right semantics. `""` and int `0` are
    sentinels."""
    return name in _RIGHT_FIELD_NAMES or name.endswith("_right")


def _is_sentinel_shaped_field(name: str) -> bool:
    """True if `name` is a contract-id field where producers diverge on
    omit / null / sentinel-value (`0`, `""`). Consumer strips post-
    canonical None values for these fields so all three shapes converge
    to "absent" in the comparison."""
    return (
        _is_date_field(name)
        or _is_ms_field(name)
        or _is_strike_field(name)
        or _is_right_field(name)
    )


def _canonicalize_scalar(key: str, value: Any) -> Any:
    """Per-field scalar normalization. Runs inside the dict walk so the
    key is available for field-name-based sentinel rules:

      * float: NaN / +Inf / -Inf -> None; finite rounded to FLOAT_PRECISION
      * date-shaped + int 0 -> None
      * ms-shaped + int < 0 -> None
      * strike-shaped + 0.0 (or int 0) -> None
      * right-shaped + empty string OR int 0 -> None
      * everything else passes through

    Bool is handled before int because Python `isinstance(True, int)` is
    True; we don't want to sentinel-normalize booleans.
    """
    if isinstance(value, float):
        if not math.isfinite(value):
            return None
        # Strike sentinel: 0.0 in a strike-shaped field is "no strike".
        if _is_strike_field(key) and value == 0.0:
            return None
        return round(value, FLOAT_PRECISION)
    if isinstance(value, bool):
        return value
    if isinstance(value, int):
        if _is_date_field(key) and value == 0:
            return None
        if _is_ms_field(key) and value < 0:
            return None
        # Right field as raw int (OptionContract emits int; tick types
        # emit string -- handled below). 0 is the upstream "not set" code.
        if _is_right_field(key) and value == 0:
            return None
        # Strike emitted as int (a producer that rounds to int) -- treat
        # 0 the same as 0.0.
        if _is_strike_field(key) and value == 0:
            return None
        return value
    if isinstance(value, str):
        # Right tick field (`"C" / "P" / ""`). Empty string is sentinel.
        if _is_right_field(key) and value == "":
            return None
    return value


def _canonicalize_row(value: Any, key: str = "") -> Any:
    """Recursively normalize a first_row value per the canonical contract.

    Applied to every loaded first_row regardless of what producers did.
    The `key` parameter carries the (already-lowercased) dict key from
    the caller so sentinel normalization can fire on contract-id-shaped
    fields; for the root call and list elements it's empty.

    Handles:
      * recursive lowercase dict keys
      * recursive 6-decimal float rounding
      * NaN / +Inf / -Inf -> None
      * date-shaped fields: value `0` -> None
      * ms-shaped fields: negative value -> None
      * strike-shaped fields: value `0.0` -> None
      * right-shaped fields: value `""` or int `0` -> None
      * post-canonical None -> stripped from the dict for any field in
        `_SENTINEL_SHAPED_FIELDS`. This makes producer divergence on
        omit-vs-null-vs-sentinel-value invisible to the diff engine
        (Go `omitempty` strips zero contract-ids; server skips them in
        `tools/server/src/format.rs:89`; Python emits raw `0`/`""`;
        CLI emits raw `0`/`""` after round-3). All four shapes converge
        to "field absent from the dict" post-canonicalization.

    This is the authoritative enforcer of the first_row contract -- a
    producer bug (mixed-case key, non-finite float, precision regression,
    sentinel mapped to null vs raw int, omitted vs null) surfaces here
    instead of as a spurious cross-language disagreement.
    """
    if isinstance(value, dict):
        canonicalized: dict[str, Any] = {}
        for k, v in value.items():
            lk = str(k).lower()
            cv = _canonicalize_row(v, lk)
            # Strip canonical-None values for sentinel-shaped contract-id
            # fields so omit / null / `0` / `""` all converge to absent.
            # Non-sentinel fields keep their None (e.g. an explicit null
            # for a regular nullable column is a real distinction from
            # the field being missing).
            if cv is None and _is_sentinel_shaped_field(lk):
                continue
            canonicalized[lk] = cv
        return canonicalized
    if isinstance(value, list):
        # List elements inherit the parent key for sentinel lookup (so
        # `dates: [0, 20260417]` would canonicalize the 0). Rare in
        # practice but free to support.
        return [_canonicalize_row(v, key) for v in value]
    return _canonicalize_scalar(key, value)


def index_by_cell(records: list[dict]) -> dict[tuple[str, str], dict]:
    """Index records by (endpoint, mode). Raise on duplicates -- each cell
    should appear at most once per SDK. Canonicalizes `first_row` on load
    so producer-side divergence (key case, float precision, non-finite
    floats) doesn't surface as a spurious disagreement."""
    idx: dict[tuple[str, str], dict] = {}
    for rec in records:
        key = (rec.get("endpoint", ""), rec.get("mode", ""))
        if key in idx:
            raise ValueError(f"duplicate cell in artifact: {key}")
        first_row = rec.get("first_row")
        if isinstance(first_row, dict):
            rec = {**rec, "first_row": _canonicalize_row(first_row)}
        idx[key] = rec
    return idx


# Sentinel for "field not present in this SDK's canonical first row".
# Needs a distinct identity so we don't confuse it with a real `None`
# that an SDK explicitly emitted for a nullable field.
class _Missing:
    __slots__ = ()

    def __repr__(self) -> str:
        return "<missing>"


MISSING = _Missing()


def _values_equal(a: Any, b: Any) -> bool:
    """Equality that treats floats within FLOAT_PRECISION as equal.

    Canonicalization rounds floats to 6 decimals before emission, but
    emitters in different languages can still disagree by 1 ULP after
    the round-trip through JSON. Compare at the rounded precision to
    keep the diff engine honest about which fields truly differ.
    NaN never equals NaN -- both sides have to agree on being numeric.
    """
    if isinstance(a, _Missing) or isinstance(b, _Missing):
        return type(a) is type(b)
    if a is None or b is None:
        return a is b
    if isinstance(a, float) or isinstance(b, float):
        try:
            af = float(a)
            bf = float(b)
        except (TypeError, ValueError):
            return False
        if math.isnan(af) or math.isnan(bf):
            return False
        return round(af, FLOAT_PRECISION) == round(bf, FLOAT_PRECISION)
    return a == b


def _flatten(obj: Any, prefix: str = "") -> dict[str, Any]:
    """Flatten a JSON-like object to a dotted-path map of leaf values.

    {"a": {"b": 1}, "c": [2, 3]}  ->  {"a.b": 1, "c[0]": 2, "c[1]": 3}

    Keeps the walk shallow-stable so two SDKs producing the same
    structure produce the same key set. Lists are indexed by position;
    dicts by key name. Leaves are anything that isn't a dict or list
    (strings, numbers, bools, None).

    Empty dicts and empty lists contribute zero entries -- there's
    nothing to diff inside them, and treating "no fields" as a sentinel
    `<root>` entry would flag a noisy mismatch any time post-canonical
    sentinel stripping leaves one SDK with an empty contract-id block.
    """
    out: dict[str, Any] = {}
    if isinstance(obj, dict):
        for k in sorted(obj):
            v = obj[k]
            path = f"{prefix}.{k}" if prefix else str(k)
            out.update(_flatten(v, path))
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            path = f"{prefix}[{i}]"
            out.update(_flatten(v, path))
    else:
        out[prefix or "<root>"] = obj
    return out


def _diff_first_row(
    first_rows: dict[str, dict],
) -> list[tuple[str, dict[str, Any]]]:
    """Walk first_row dicts field-by-field and return differing fields.

    Input: {lang: first_row_dict}. Output: list of (field_path,
    {lang: value_or_MISSING}) tuples, one per field that disagrees.
    """
    flat_per_lang = {lang: _flatten(fr) for lang, fr in first_rows.items()}
    all_fields: set[str] = set()
    for flat in flat_per_lang.values():
        all_fields.update(flat)

    diffs: list[tuple[str, dict[str, Any]]] = []
    for field in sorted(all_fields):
        values = {lang: flat.get(field, MISSING) for lang, flat in flat_per_lang.items()}
        langs = list(values)
        agree = True
        for i in range(len(langs)):
            for j in range(i + 1, len(langs)):
                if not _values_equal(values[langs[i]], values[langs[j]]):
                    agree = False
                    break
            if not agree:
                break
        if not agree:
            diffs.append((field, values))
    return diffs


def _fmt_value(v: Any) -> str:
    """Single-line representation for the diff table. Floats render at
    FLOAT_PRECISION so they line up across SDKs."""
    if isinstance(v, _Missing):
        return "<missing>"
    if v is None:
        return "null"
    if isinstance(v, float):
        return f"{round(v, FLOAT_PRECISION):.{FLOAT_PRECISION}f}"
    if isinstance(v, str):
        return repr(v) if len(v) <= 60 else repr(v[:57] + "...")
    return repr(v)


def _format_cell_diff(
    cell: tuple[str, str],
    present: dict[str, dict],
    disagreement_kind: str,
    stream,
) -> None:
    """Print one cell's diff to `stream`. disagreement_kind is one of
    status | row_count | first_row | mixed.
    """
    endpoint, mode = cell
    label = f"{endpoint}::{mode}"
    langs = sorted(present)
    stream.write(f"\n  {label}  [{disagreement_kind} disagreement]\n")

    # Carry forward the per-cell rationale W6 (#297) attached to each
    # validator artifact -- it tells the reader what the cell was supposed
    # to prove. All SDKs see the same generator output for a given cell,
    # so rationale is identical across present[lang]; pick whichever is
    # available. Fall back to "(missing)" for older artifacts written
    # before the field landed.
    rationale = next(
        (rec.get("rationale") for rec in present.values() if rec.get("rationale")),
        None,
    )
    if rationale:
        stream.write(f"    rationale: {rationale}\n")

    header = "    {:8s} | {:<8s} | {:<8s} | {:<40s}".format("sdk", "status", "rows", "detail")
    stream.write(header + "\n")
    stream.write("    " + "-" * (len(header) - 4) + "\n")
    for lang in langs:
        rec = present[lang]
        detail = str(rec.get("detail", ""))
        if len(detail) > 40:
            detail = detail[:37] + "..."
        stream.write(
            "    {:8s} | {:<8s} | {:<8s} | {:<40s}\n".format(
                lang,
                str(rec.get("status", "")),
                str(rec.get("row_count", "")),
                detail,
            )
        )

    if disagreement_kind in ("first_row", "mixed"):
        first_rows = {
            lang: present[lang]["first_row"]
            for lang in langs
            if isinstance(present[lang].get("first_row"), dict)
        }
        if len(first_rows) >= 2:
            field_diffs = _diff_first_row(first_rows)
            if field_diffs:
                stream.write("\n    field-level diff:\n")
                col_header = "    {:<32s}".format("field")
                for lang in langs:
                    col_header += " | {:<20s}".format(lang)
                stream.write(col_header + "\n")
                stream.write("    " + "-" * (len(col_header) - 4) + "\n")
                for field, values in field_diffs:
                    row = "    {:<32s}".format(field[:32])
                    for lang in langs:
                        row += " | {:<20s}".format(_fmt_value(values.get(lang, MISSING))[:20])
                    stream.write(row + "\n")


def _classify_cell(present: dict[str, dict]) -> str | None:
    """Return disagreement kind for a cell, or None if all SDKs agree.

    status    - some SDKs PASS, others FAIL/SKIP
    row_count - all PASS but row_count differs
    first_row - status + row_count match but first_row fields differ
    mixed     - status agrees but both row_count and first_row disagree
    """
    statuses = {rec.get("status") for rec in present.values()}
    if len(statuses) > 1:
        return "status"

    pass_recs = {lang: rec for lang, rec in present.items() if rec.get("status") == "PASS"}
    if len(pass_recs) < 2:
        return None

    row_counts = {rec.get("row_count", 0) for rec in pass_recs.values()}
    row_count_differs = len(row_counts) > 1

    first_rows = {
        lang: rec["first_row"]
        for lang, rec in pass_recs.items()
        if isinstance(rec.get("first_row"), dict)
    }
    first_row_differs = False
    if len(first_rows) >= 2:
        field_diffs = _diff_first_row(first_rows)
        first_row_differs = bool(field_diffs)

    if row_count_differs and first_row_differs:
        return "mixed"
    if row_count_differs:
        return "row_count"
    if first_row_differs:
        return "first_row"
    return None


def compare(
    per_lang: dict[str, dict[tuple[str, str], dict]],
    max_cell_diff_rows: int,
    stream,
) -> tuple[int, int, int]:
    """Compare cells across SDKs, print diffs, return (total_cells,
    disagreement_count, partial_count)."""
    all_cells: set[tuple[str, str]] = set()
    for idx in per_lang.values():
        all_cells.update(idx.keys())

    disagreements: list[tuple[tuple[str, str], dict[str, dict], str]] = []
    missing_per_cell: dict[tuple[str, str], list[str]] = defaultdict(list)

    for cell in sorted(all_cells):
        present = {lang: idx[cell] for lang, idx in per_lang.items() if cell in idx}
        absent = [lang for lang, idx in per_lang.items() if cell not in idx]
        if absent:
            missing_per_cell[cell] = absent
        if len(present) < 2:
            continue
        kind = _classify_cell(present)
        if kind is not None:
            disagreements.append((cell, present, kind))

    partial_count = sum(1 for v in missing_per_cell.values() if v)

    if disagreements:
        stream.write("\nDISAGREEMENTS:\n")
        for cell, present, kind in disagreements[:max_cell_diff_rows]:
            _format_cell_diff(cell, present, kind, stream)
        if len(disagreements) > max_cell_diff_rows:
            stream.write(
                f"\n  ... and {len(disagreements) - max_cell_diff_rows} more cells\n",
            )

    return len(all_cells), len(disagreements), partial_count


def main(argv: list[str] | None = None) -> int:
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
        help="Cap on disagreement cells printed (others summarized).",
    )
    parser.add_argument(
        "--artifacts-dir",
        type=Path,
        default=None,
        help="Override artifacts directory (for testing). Default: artifacts/.",
    )
    args = parser.parse_args(argv)

    artifacts_dir = args.artifacts_dir if args.artifacts_dir is not None else ARTIFACTS_DIR

    per_lang: dict[str, dict[tuple[str, str], dict]] = {}
    missing: list[str] = []
    for lang in LANGS:
        records = load_artifact(lang, artifacts_dir)
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
            f"(expected at {artifacts_dir}/validator_<lang>.json)",
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

    total, disagreements, partial = compare(per_lang, args.max_cell_diff_rows, sys.stderr)

    if disagreements == 0:
        sdk_set = ", ".join(sorted(per_lang))
        print(f"\n\u2713 {total} cells agree across {{{sdk_set}}}")
    else:
        print(
            f"\nagreement: {total} cells across {len(per_lang)} SDKs, "
            f"{disagreements} disagreements, {partial} cells partial",
        )

    if partial:
        print(
            f"\nnote: {partial} cells missing from at least one SDK "
            "(CLI skips per-optional-param modes by design -- see PR #291)",
            file=sys.stderr,
        )

    return 1 if disagreements else 0


if __name__ == "__main__":
    sys.exit(main())
