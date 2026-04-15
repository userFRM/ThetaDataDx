#!/usr/bin/env python3
"""Synthetic tests for scripts/validate_agreement.py.

Constructs four mock SDK artifacts with intentional disagreements and
asserts the diff engine's output names the right fields. Uses only
stdlib so it runs in CI without extra deps. Invoke via:

    python3 scripts/validate_agreement_test.py

Exits non-zero on the first failing assertion.
"""

from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import validate_agreement  # noqa: E402


def _write_artifact(base: Path, lang: str, records: list[dict]) -> None:
    path = base / f"validator_{lang}.json"
    path.write_text(json.dumps({"lang": lang, "records": records}, indent=2, sort_keys=True))


def _base_record(
    endpoint: str,
    mode: str,
    status: str = "PASS",
    row_count: int = 1,
    detail: str = "",
    first_row: dict | None = None,
) -> dict:
    rec = {
        "endpoint": endpoint,
        "mode": mode,
        "status": status,
        "row_count": row_count,
        "duration_ms": 100,
        "detail": detail,
    }
    if first_row is not None:
        rec["first_row"] = first_row
    return rec


class AgreementTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmpdir = tempfile.TemporaryDirectory()
        self.artifacts = Path(self.tmpdir.name)

    def tearDown(self) -> None:
        self.tmpdir.cleanup()

    def _run(self, extra_args: list[str] | None = None) -> tuple[int, str, str]:
        """Run main() with captured stdout/stderr against self.artifacts."""
        argv = ["--artifacts-dir", str(self.artifacts)]
        if extra_args:
            argv.extend(extra_args)
        out_buf = io.StringIO()
        err_buf = io.StringIO()
        orig_out, orig_err = sys.stdout, sys.stderr
        sys.stdout, sys.stderr = out_buf, err_buf
        try:
            code = validate_agreement.main(argv)
        finally:
            sys.stdout, sys.stderr = orig_out, orig_err
        return code, out_buf.getvalue(), err_buf.getvalue()

    def test_all_agree_exits_zero(self) -> None:
        for lang in validate_agreement.LANGS:
            _write_artifact(
                self.artifacts,
                lang,
                [
                    _base_record("stock_snapshot_ohlc", "concrete", row_count=1),
                    _base_record("stock_list_dates", "basic", row_count=42),
                ],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"expected exit 0, got {code}; out={out!r}")
        self.assertIn("2 cells agree across", out)

    def test_field_level_diff_pinpoints_bid(self) -> None:
        # All four SDKs passed, row_count matches (1 row each), but
        # go got a different bid price. The diff must name `bid` as
        # the differing field.
        base_row = {
            "ask": 685.88,
            "bid": 685.86,
            "bid_size": 100,
            "ask_size": 200,
            "date": "20250303",
        }
        for lang in ("python", "cli", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_snapshot_quote", "concrete", first_row=dict(base_row))],
            )
        go_row = dict(base_row)
        go_row["bid"] = 685.87
        _write_artifact(
            self.artifacts,
            "go",
            [_base_record("stock_snapshot_quote", "concrete", first_row=go_row)],
        )

        code, _, err = self._run()
        self.assertEqual(code, 1, f"expected exit 1 on disagreement; stderr={err!r}")
        self.assertIn("stock_snapshot_quote::concrete", err)
        self.assertIn("first_row disagreement", err)
        self.assertIn("field-level diff", err)
        self.assertIn("bid", err)
        self.assertIn("685.860000", err)
        self.assertIn("685.870000", err)
        # Fields that agree must NOT appear in the diff rows.
        self.assertNotIn("ask_size", _diff_section(err))

    def test_row_count_disagreement_without_first_row(self) -> None:
        # Legacy artifacts: no first_row field. Diff engine must fall
        # back to row_count comparison and still report the mismatch.
        for lang, rc in (("python", 10), ("cli", 10), ("go", 9), ("cpp", 10)):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", row_count=rc)],
            )
        code, _, err = self._run()
        self.assertEqual(code, 1)
        self.assertIn("stock_history_ohlc::concrete", err)
        self.assertIn("row_count disagreement", err)
        # Per-SDK status table still names each SDK and its row count.
        self.assertIn("go", err)
        self.assertIn("9", err)
        self.assertIn("10", err)

    def test_status_disagreement(self) -> None:
        # Python passed, CLI/go/cpp failed. Diff must flag as status
        # disagreement and show each SDK's status + detail.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("option_snapshot_trade", "concrete")],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [
                    _base_record(
                        "option_snapshot_trade",
                        "concrete",
                        status="FAIL",
                        row_count=0,
                        detail="wire-format error: unexpected tick kind",
                    )
                ],
            )
        code, _, err = self._run()
        self.assertEqual(code, 1)
        self.assertIn("option_snapshot_trade::concrete", err)
        self.assertIn("status disagreement", err)
        self.assertIn("wire-format error", err)

    def test_nested_first_row_diff(self) -> None:
        # first_row with nested dict + list. Diff engine must walk
        # into the nested structure and name `greeks.delta[0]` as the
        # differing field, not just `greeks`.
        base_row = {
            "symbol": "SPY",
            "greeks": {"delta": [0.5, 0.6], "gamma": 0.01},
        }
        for lang in ("python", "cli", "go"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("option_snapshot_greeks_all", "concrete", first_row=json.loads(json.dumps(base_row)))],
            )
        cpp_row = json.loads(json.dumps(base_row))
        cpp_row["greeks"]["delta"][0] = 0.55
        _write_artifact(
            self.artifacts,
            "cpp",
            [_base_record("option_snapshot_greeks_all", "concrete", first_row=cpp_row)],
        )

        code, _, err = self._run()
        self.assertEqual(code, 1)
        self.assertIn("greeks.delta[0]", err)
        # `greeks.gamma` and `greeks.delta[1]` are equal across SDKs
        # and must not appear in the field-level diff rows.
        diff_only = _diff_section(err)
        self.assertNotIn("greeks.gamma", diff_only)
        self.assertNotIn("greeks.delta[1]", diff_only)

    def test_missing_field_in_one_sdk(self) -> None:
        # cpp omits the `volume` field entirely (tick type doesn't
        # surface it). Diff must mark volume as <missing> for cpp and
        # show the real value for the others.
        full_row = {"price": 100.0, "volume": 5000}
        for lang in ("python", "cli", "go"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_trade", "concrete", first_row=dict(full_row))],
            )
        partial_row = {"price": 100.0}
        _write_artifact(
            self.artifacts,
            "cpp",
            [_base_record("stock_history_trade", "concrete", first_row=partial_row)],
        )

        code, _, err = self._run()
        self.assertEqual(code, 1)
        self.assertIn("volume", err)
        self.assertIn("<missing>", err)

    def test_soft_skip_missing_sdk_without_require(self) -> None:
        # Only 3 SDKs reported; without --require-all-sdks this is a
        # warning, not a failure.
        for lang in ("python", "cli", "go"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("calendar_open_today", "basic", row_count=1)],
            )
        code, out, err = self._run()
        self.assertEqual(code, 0)
        self.assertIn("warning: no artifact for cpp", err)
        self.assertIn("1 cells agree across", out)

    def test_require_all_sdks_fails_on_missing(self) -> None:
        for lang in ("python", "cli", "go"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("calendar_open_today", "basic", row_count=1)],
            )
        code, _, err = self._run(["--require-all-sdks"])
        self.assertEqual(code, 1)
        self.assertIn("--require-all-sdks set", err)

    def test_float_precision_tolerance(self) -> None:
        # 685.860000 == 685.8600004 after 6-decimal rounding. These
        # must compare equal; the diff engine must not flag a false
        # positive on 1-ULP float noise.
        for lang, bid in (("python", 685.86), ("cli", 685.8600004), ("go", 685.86), ("cpp", 685.8599996)):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_snapshot_quote", "concrete", first_row={"bid": bid})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0)
        self.assertIn("1 cells agree across", out)

    def test_partial_cells_note(self) -> None:
        # CLI skips per-optional-param mode; the other three SDKs run
        # it. Not a disagreement, but the summary must mention partial.
        for lang in ("python", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [
                    _base_record("stock_snapshot_ohlc", "concrete", row_count=1),
                    _base_record("stock_snapshot_ohlc", "with_venue", row_count=1),
                ],
            )
        _write_artifact(
            self.artifacts,
            "cli",
            [_base_record("stock_snapshot_ohlc", "concrete", row_count=1)],
        )
        code, out, err = self._run()
        self.assertEqual(code, 0)
        self.assertIn("cells missing from at least one SDK", err)

    def test_mixed_case_keys_canonicalize(self) -> None:
        # A producer regression that leaves keys in mixed case
        # (`{"Bid": 685.86}`) must compare equal to the canonical
        # lowercase form. Consumer-side canonicalization is the
        # authoritative enforcer -- producers can't be trusted to
        # emit lowercase 100% of the time.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_snapshot_quote", "concrete", first_row={"Bid": 685.86, "Ask": 685.88})],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_snapshot_quote", "concrete", first_row={"bid": 685.86, "ask": 685.88})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"mixed-case keys must canonicalize to equal; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_mixed_case_key_nested_canonicalize(self) -> None:
        # Canonicalization must recurse into nested dicts. A producer
        # that regressed ONLY on a nested key ({"Greeks": {"Delta": ...}})
        # still has to compare equal to the canonical form.
        _write_artifact(
            self.artifacts,
            "python",
            [
                _base_record(
                    "option_snapshot_greeks_all",
                    "concrete",
                    first_row={"Greeks": {"Delta": 0.5, "Gamma": 0.01}},
                )
            ],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [
                    _base_record(
                        "option_snapshot_greeks_all",
                        "concrete",
                        first_row={"greeks": {"delta": 0.5, "gamma": 0.01}},
                    )
                ],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0)
        self.assertIn("1 cells agree across", out)

    def test_nan_normalizes_to_null(self) -> None:
        # NaN is not equal to NaN under IEEE semantics, and cross-language
        # serialization of non-finite floats is ambiguous (JSON rejects
        # them; CLI's f64 reparse drops them silently). Consumer-side
        # canonicalization collapses NaN / +Inf / -Inf to Python None so
        # two producers emitting "missing / sentinel" in different shapes
        # converge on the same canonical value.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("option_snapshot_greeks_all", "concrete", first_row={"delta": float("nan")})],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("option_snapshot_greeks_all", "concrete", first_row={"delta": None})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"NaN must canonicalize to None; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_infinity_normalizes_to_null(self) -> None:
        # +Inf / -Inf same treatment as NaN.
        for lang, val in (
            ("python", float("inf")),
            ("cli", float("-inf")),
            ("go", None),
            ("cpp", None),
        ):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("option_snapshot_greeks_all", "concrete", first_row={"delta": val})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"±Inf must canonicalize to None; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_real_disagreement_still_detected_after_canonicalization(self) -> None:
        # Sanity check: canonicalization must NOT hide real diffs. Even
        # with mixed-case keys across producers, a genuinely different
        # value at the same canonical field still produces a diff row
        # naming the lowercased field.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_snapshot_quote", "concrete", first_row={"Bid": 685.86})],
        )
        _write_artifact(
            self.artifacts,
            "go",
            [_base_record("stock_snapshot_quote", "concrete", first_row={"bid": 685.87})],
        )
        for lang in ("cli", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_snapshot_quote", "concrete", first_row={"bid": 685.86})],
            )
        code, _, err = self._run()
        self.assertEqual(code, 1)
        # Diff table names the LOWERCASED field even though python sent
        # the mixed-case version; that's what makes field diffs readable
        # when producers disagree on case AND value.
        self.assertIn("bid", _diff_section(err))
        self.assertIn("685.870000", err)

    def test_date_zero_sentinel_normalizes_to_null(self) -> None:
        # CLI emits `date: 0` verbatim under the round-3 json-raw contract
        # (tools/cli/src/main.rs `raw_date`), while another producer might
        # serialize the same "no date" cell as JSON null. Both shapes must
        # canonicalize to None so they compare equal. Rationale: trading
        # data never has year 0000; `0` is always a sentinel.
        for lang, val in (
            ("python", 0),
            ("cli", 0),
            ("go", None),
            ("cpp", None),
        ):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", first_row={"date": val})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"date=0 must canonicalize to None; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_ms_of_day_negative_sentinel_normalizes_to_null(self) -> None:
        # Similar to date=0: ms-of-day is non-negative by construction
        # (bounded 0..86_400_000). Negative values are sentinels that some
        # producers emit as raw ints and others as JSON null. Both must
        # canonicalize to None.
        for lang, val in (
            ("python", -1),
            ("cli", -1),
            ("go", None),
            ("cpp", None),
        ):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_trade", "concrete", first_row={"ms_of_day": val})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"ms_of_day<0 must canonicalize to None; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_sentinel_vs_real_value_still_disagrees(self) -> None:
        # Sanity: sentinel normalization must NOT hide genuine diffs.
        # One producer emits `date: 20260417` (real date), another emits
        # `date: 0` (sentinel -> None). These are genuinely different
        # cells and the diff engine must report it.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"date": 20260417})],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", first_row={"date": 0})],
            )
        code, _, err = self._run()
        self.assertEqual(code, 1, "real date vs sentinel 0 must disagree")
        self.assertIn("date", _diff_section(err))
        # Python's 20260417 should appear in the diff table; the other
        # three SDKs' sentinel `0` is canonicalized to None and stripped
        # from the dict (Option B: omit-equivalence), so they show up as
        # `<missing>` in the field-level table.
        self.assertIn("20260417", err)
        self.assertIn("<missing>", _diff_section(err))

    def test_expiration_zero_sentinel_and_time_alias(self) -> None:
        # Covers two extra field-name patterns the canonicalization rule
        # must catch: `expiration` (date-shaped, server omits-when-zero)
        # and `time` (ms-shaped alias used by CLI OHLC / trade columns).
        for lang, row in (
            ("python", {"expiration": 0, "time": -1}),
            ("cli", {"expiration": 0, "time": -1}),
            ("go", {"expiration": None, "time": None}),
            ("cpp", {"expiration": None, "time": None}),
        ):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("option_history_ohlc", "concrete", first_row=row)],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"expiration=0 + time=-1 must canonicalize; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_zero_is_not_sentinel_for_non_date_fields(self) -> None:
        # Regression test: the field-name rule must be narrow. `volume=0`
        # is a legitimate trading value (no trades in the bar) and MUST
        # stay distinct from `volume=null`. Same for `bid_size=0`,
        # `count=0`, etc. Previously a by-value rule ("any int 0 in a
        # tick-shaped field") would have been far too aggressive.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"volume": 0})],
        )
        _write_artifact(
            self.artifacts,
            "cli",
            [_base_record("stock_history_ohlc", "concrete", first_row={"volume": None})],
        )
        code, _, err = self._run()
        self.assertEqual(code, 1, "volume=0 and volume=null are distinct; must disagree")
        self.assertIn("volume", _diff_section(err))

    # ------------------------------------------------------------------
    # Round 4 -- omit-vs-null-vs-sentinel-value equivalence (Codex r4).
    # Producers diverge on contract-id field shape:
    #   - Python emits `expiration: 0` always
    #   - Go's `omitempty` strips zero-valued fields from JSON
    #   - Server's `insert_contract_id_fields` skips when expiration==0
    #   - CLI raw helpers emit `expiration: 0` verbatim (round-3 fix)
    # All four shapes must canonicalize to the same thing.
    # ------------------------------------------------------------------

    def test_omit_vs_null_for_expiration_agrees(self) -> None:
        # Two producers omit `expiration` entirely (Go omitempty, server
        # skip-when-zero). One producer emits `expiration: null`. After
        # canonicalization, all three shapes must agree.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"expiration": None})],
        )
        for lang in ("cli", "go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", first_row={})],
            )
        code, out, _ = self._run()
        self.assertEqual(
            code, 0,
            "expiration:null and omitted expiration must agree post-canonicalization",
        )
        self.assertIn("1 cells agree across", out)

    def test_omit_vs_zero_for_expiration_agrees(self) -> None:
        # Three producers emit `expiration: 0` (Python tick_columnar,
        # CLI raw helpers). One producer omits it (Go omitempty / server
        # skip-when-zero). Both shapes must canonicalize to "absent".
        for lang in ("python", "cli", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", first_row={"expiration": 0})],
            )
        _write_artifact(
            self.artifacts,
            "go",
            [_base_record("stock_history_ohlc", "concrete", first_row={})],
        )
        code, out, _ = self._run()
        self.assertEqual(
            code, 0,
            "expiration:0 and omitted expiration must agree post-canonicalization",
        )
        self.assertIn("1 cells agree across", out)

    def test_omit_for_non_sentinel_field_still_differs(self) -> None:
        # The omit-equivalence rule only fires for sentinel-shaped fields
        # (date / expiration / strike / right / ms-of-day). A regular
        # nullable column like `volume` must keep distinguishing
        # "absent" from "explicitly null" -- the field set difference IS
        # the disagreement.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"volume": None})],
        )
        _write_artifact(
            self.artifacts,
            "cli",
            [_base_record("stock_history_ohlc", "concrete", first_row={})],
        )
        code, _, err = self._run()
        self.assertEqual(
            code, 1,
            "volume:null vs omitted volume must DISAGREE (omit-equiv only for sentinel fields)",
        )
        self.assertIn("volume", _diff_section(err))

    def test_strike_zero_sentinel_canonicalizes(self) -> None:
        # `strike: 0.0` (Python) vs omitted strike (Go omitempty) must
        # agree. Strike is a contract-id sentinel.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"strike": 0.0})],
        )
        _write_artifact(
            self.artifacts,
            "cli",
            [_base_record("stock_history_ohlc", "concrete", first_row={"strike": 0.0})],
        )
        for lang in ("go", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("stock_history_ohlc", "concrete", first_row={})],
            )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"strike=0.0 must canonicalize; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_right_empty_string_and_zero_canonicalize(self) -> None:
        # Right field has two valid sentinel shapes: empty string `""`
        # (Python tick_columnar `right: "C"/"P"/""`, Go RightStr) and raw
        # int `0` (server's right_label fall-through, OptionContract).
        # Both must canonicalize to "absent" so `right: ""` (Python),
        # `right: 0` (server / OptionContract emitter), and omitted
        # `right` (Go omitempty) all agree.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("stock_history_ohlc", "concrete", first_row={"right": ""})],
        )
        _write_artifact(
            self.artifacts,
            "cli",
            [_base_record("stock_history_ohlc", "concrete", first_row={"right": ""})],
        )
        _write_artifact(
            self.artifacts,
            "go",
            [_base_record("stock_history_ohlc", "concrete", first_row={})],
        )
        _write_artifact(
            self.artifacts,
            "cpp",
            [_base_record("stock_history_ohlc", "concrete", first_row={"right": 0})],
        )
        code, out, _ = self._run()
        self.assertEqual(code, 0, f"right empty/zero/omit must agree; got exit {code}")
        self.assertIn("1 cells agree across", out)

    def test_real_right_value_still_disagrees(self) -> None:
        # Sanity: omit-equivalence must not hide real right disagreement.
        # Python emits `right: "C"` (a real call), one SDK emits `"P"` (a
        # real put). Different actual options -- diff must report it.
        _write_artifact(
            self.artifacts,
            "python",
            [_base_record("option_history_ohlc", "concrete", first_row={"right": "C"})],
        )
        _write_artifact(
            self.artifacts,
            "go",
            [_base_record("option_history_ohlc", "concrete", first_row={"right": "P"})],
        )
        for lang in ("cli", "cpp"):
            _write_artifact(
                self.artifacts,
                lang,
                [_base_record("option_history_ohlc", "concrete", first_row={"right": "C"})],
            )
        code, _, err = self._run()
        self.assertEqual(code, 1, "right C vs P is a real disagreement")
        self.assertIn("right", _diff_section(err))


def _diff_section(text: str) -> str:
    """Return just the field-level diff rows, stripping headers / status
    tables. Used to make "field X doesn't appear" assertions precise."""
    lines = text.splitlines()
    out_lines: list[str] = []
    in_diff = False
    for line in lines:
        if "field-level diff" in line:
            in_diff = True
            continue
        if in_diff:
            if not line.strip() or line.lstrip().startswith("sdk "):
                in_diff = False
                continue
            if set(line.strip()) <= {"-", " "}:
                continue
            out_lines.append(line)
    return "\n".join(out_lines)


if __name__ == "__main__":
    unittest.main(verbosity=2)
