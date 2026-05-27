#!/usr/bin/env python3
"""Test suite for `scripts/check_binding_parity.py` (Gate 2 / #595).

Feeds synthetic Rust source + binding sources via tempdir directories
and asserts positive (all-bound) and negative (missing-on-TS,
missing-on-C++, missing-on-FFI, rust_only-without-issue,
orphan-rust-field, `_explicit`-widened-ABI) cases.

The tests run as plain Python (no pytest required) so the
audit-protocol convention for CI gates can wire it into the same
`ci.yml` invocation as the production gate. The exit code is the
total failure count, so the script integrates with any CI runner
that interprets non-zero as failure.

Run::

    python3 scripts/test_check_binding_parity.py

The companion `--selftest` switch on the production script runs the
same matrix in-process for a fast smoke-check before the full test
file is loaded.
"""

from __future__ import annotations

import pathlib
import sys

# Import the production module so the test suite drives the same code
# path the gate uses in CI. The module's `_check_dotted_rows`,
# `_check_orphan_rust_fields`, `_collect_*_setters`, and
# `_collect_rust_pub_fields` helpers are the SSOT for the parity
# logic; importing keeps the tests in lockstep with any future
# refactor of the gate.
HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import check_binding_parity as cbp  # noqa: E402


# ─── Helpers ───────────────────────────────────────────────────────


_fails: list[str] = []


def _check(label: str, fn) -> None:
    """Run a test closure under a label; record failures."""
    try:
        fn()
    except AssertionError as e:
        _fails.append(f"FAIL: {label}: {e}")
    except Exception as e:  # noqa: BLE001
        _fails.append(f"ERROR: {label}: {type(e).__name__}: {e}")


# ─── Positive: all four bindings expose the field ──────────────────


def test_all_bound_passes() -> None:
    rows = [
        {
            "name": "FlatFilesConfig.max_attempts",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    py_setters = {"flatfiles_max_attempts"}
    ts_setters = {"flatfiles_max_attempts"}
    cpp_setters = {"flatfiles_max_attempts"}
    ffi_setters = {"flatfiles_max_attempts"}
    errors = cbp._check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )
    assert errors == [], f"all-bound row must pass; got {errors!r}"


# ─── Negative: missing-on-TS ───────────────────────────────────────


def test_missing_on_ts_trips() -> None:
    rows = [
        {
            "name": "FlatFilesConfig.max_attempts",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    py_setters = {"flatfiles_max_attempts"}
    ts_setters: set[str] = set()
    cpp_setters = {"flatfiles_max_attempts"}
    ffi_setters = {"flatfiles_max_attempts"}
    errors = cbp._check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )
    assert any("typescript" in e and "missing" in e for e in errors), (
        f"missing TS setter must trip; got {errors!r}"
    )


# ─── Negative: missing-on-C++ ──────────────────────────────────────


def test_missing_on_cpp_trips() -> None:
    rows = [
        {
            "name": "FlatFilesConfig.max_attempts",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    py_setters = {"flatfiles_max_attempts"}
    ts_setters = {"flatfiles_max_attempts"}
    cpp_setters: set[str] = set()
    ffi_setters = {"flatfiles_max_attempts"}
    errors = cbp._check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )
    assert any("cpp" in e and "missing" in e for e in errors), (
        f"missing C++ setter must trip; got {errors!r}"
    )


# ─── Negative: missing-on-FFI under cpp=true ──────────────────────


def test_missing_on_ffi_under_cpp_true_trips() -> None:
    rows = [
        {
            "name": "FlatFilesConfig.max_attempts",
            "python": False,
            "typescript": False,
            "cpp": True,
        }
    ]
    cpp_setters = {"flatfiles_max_attempts"}
    errors = cbp._check_dotted_rows(rows, set(), set(), cpp_setters, set())
    assert any("ffi" in e for e in errors), (
        f"missing FFI under cpp=true must trip; got {errors!r}"
    )


# ─── Positive: python-only setter does not require FFI symbol ─────


def test_python_only_no_ffi_required() -> None:
    rows = [
        {
            "name": "MddsConfig.host",
            "python": True,
            "typescript": False,
            "cpp": False,
        }
    ]
    py_setters = {"host"}
    errors = cbp._check_dotted_rows(rows, py_setters, set(), set(), set())
    assert errors == [], f"python-only setter must not require FFI; got {errors!r}"


# ─── Negative: rust_only without issue ─────────────────────────────


def test_rust_only_without_issue_trips() -> None:
    rows = [
        {
            "name": "FpssConfig.timeout_ms",
            "python": False,
            "typescript": False,
            "cpp": False,
            "rust_only": True,
        }
    ]
    errors = cbp._check_dotted_rows(rows, set(), set(), set(), set())
    assert any("issue" in e for e in errors), (
        f"rust_only without issue must trip; got {errors!r}"
    )


# ─── Negative: issue without rust_only ─────────────────────────────


def test_issue_without_rust_only_trips() -> None:
    rows = [
        {
            "name": "FpssConfig.timeout_ms",
            "python": False,
            "typescript": False,
            "cpp": False,
            "issue": "#595",
        }
    ]
    errors = cbp._check_dotted_rows(rows, set(), set(), set(), set())
    assert any("not `rust_only`" in e for e in errors), (
        f"issue without rust_only must trip; got {errors!r}"
    )


# ─── Negative: rust_only with a true binding column ────────────────


def test_rust_only_with_binding_true_trips() -> None:
    rows = [
        {
            "name": "FpssConfig.timeout_ms",
            "python": True,
            "typescript": False,
            "cpp": False,
            "rust_only": True,
            "issue": "#595",
        }
    ]
    errors = cbp._check_dotted_rows(rows, set(), set(), set(), set())
    assert any("rust_only = true" in e for e in errors), (
        f"rust_only with binding=true must trip; got {errors!r}"
    )


# ─── Orphan: undocumented Rust pub field ───────────────────────────


def test_orphan_rust_field_trips(tmpdir: pathlib.Path) -> None:
    cfg_dir = tmpdir / "config"
    cfg_dir.mkdir()
    (cfg_dir / "fake.rs").write_text(
        "pub struct FlatFilesConfig {\n"
        "    pub max_attempts: u32,\n"
        "    pub novel_field: u64,\n"
        "}\n",
        encoding="utf-8",
    )
    rust_fields = cbp._collect_rust_pub_fields(cfg_dir)
    assert rust_fields["FlatFilesConfig"] == {"max_attempts", "novel_field"}, (
        f"both fields must parse; got {rust_fields!r}"
    )
    rows = [
        {
            "name": "FlatFilesConfig.max_attempts",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    errors = cbp._check_orphan_rust_fields(rust_fields, rows)
    assert any("novel_field" in e for e in errors), (
        f"undocumented Rust field must trip; got {errors!r}"
    )
    # The documented row must NOT also trip.
    assert not any("max_attempts" in e for e in errors), (
        f"documented field must NOT trip; got {errors!r}"
    )


# ─── Positive: `_explicit` widened-ABI suffix accepted ──────────────


def test_explicit_widened_abi_accepted() -> None:
    """FFI emits `tdx_config_set_decode_threads_explicit` for the
    widened `(has_value, n)` ABI shape; the parity row uses the bare
    `decode_threads` name. The gate must accept the `_explicit`
    suffix as equivalent.
    """
    rows = [
        {
            "name": "MddsConfig.decode_threads",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    ffi_setters = {"decode_threads_explicit", "decode_threads"}
    py_setters = {"decode_threads"}
    ts_setters = {"decode_threads"}
    cpp_setters = {"decode_threads"}
    errors = cbp._check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )
    assert errors == [], f"_explicit suffix must satisfy; got {errors!r}"


# ─── Positive: per-row `setter` override resolves alternate name ───


def test_setter_override_resolves_alternate_name(tmpdir: pathlib.Path) -> None:
    """`MddsConfig.host` binds on Python as `set_mdds_host` (with the
    `mdds_` prefix). The row's `setter = "mdds_host"` field overrides
    the default `host` derivation and the gate accepts the binding.
    """
    rows = [
        {
            "name": "MddsConfig.host",
            "python": True,
            "typescript": False,
            "cpp": False,
            "setter": "mdds_host",
        }
    ]
    py_setters = {"mdds_host"}
    errors = cbp._check_dotted_rows(rows, py_setters, set(), set(), set())
    assert errors == [], f"setter override must resolve; got {errors!r}"


# ─── Positive: dotted-name on unknown struct is skipped ────────────


def test_unknown_struct_dotted_row_is_skipped() -> None:
    """Dotted rows whose struct prefix is not in `STRUCT_TO_PREFIX`
    (e.g. `Error.cross_binding_name_divergence`,
    `GreeksEodTick.cross_binding_anchor`) are documentation anchors.
    The gate must not gate on them — these declare class-level
    intent, not field-level binding.
    """
    rows = [
        {
            "name": "Error.cross_binding_name_divergence",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    errors = cbp._check_dotted_rows(rows, set(), set(), set(), set())
    assert errors == [], f"unknown struct dotted row must skip; got {errors!r}"


# ─── Driver ────────────────────────────────────────────────────────


def main() -> int:
    import tempfile

    _check("all-bound row passes", test_all_bound_passes)
    _check("missing on TS trips", test_missing_on_ts_trips)
    _check("missing on C++ trips", test_missing_on_cpp_trips)
    _check("missing on FFI under cpp=true trips", test_missing_on_ffi_under_cpp_true_trips)
    _check("python-only no FFI required", test_python_only_no_ffi_required)
    _check("rust_only without issue trips", test_rust_only_without_issue_trips)
    _check("issue without rust_only trips", test_issue_without_rust_only_trips)
    _check("rust_only with binding=true trips", test_rust_only_with_binding_true_trips)
    with tempfile.TemporaryDirectory() as tmp:
        _check("orphan Rust field trips", lambda: test_orphan_rust_field_trips(pathlib.Path(tmp)))
    _check("`_explicit` widened-ABI suffix accepted", test_explicit_widened_abi_accepted)
    with tempfile.TemporaryDirectory() as tmp:
        _check("setter override resolves alternate name", lambda: test_setter_override_resolves_alternate_name(pathlib.Path(tmp)))
    _check("unknown struct dotted row is skipped", test_unknown_struct_dotted_row_is_skipped)

    if _fails:
        print(f"test_check_binding_parity: {len(_fails)} failure(s)")
        for line in _fails:
            print(f"  {line}")
        return 1
    print("test_check_binding_parity: all 12 cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
