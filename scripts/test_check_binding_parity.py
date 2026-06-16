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
_total = 0


def _check(label: str, fn) -> None:
    """Run a test closure under a label; record failures."""
    global _total
    _total += 1
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
            "name": "HistoricalConfig.host",
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
            "name": "StreamingConfig.timeout_ms",
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
            "name": "StreamingConfig.timeout_ms",
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
            "name": "StreamingConfig.timeout_ms",
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
    """FFI emits `thetadatadx_config_set_worker_threads_explicit` for the
    widened `(has_value, n)` ABI shape; the parity row uses the bare
    public `worker_threads` name. The gate must accept the `_explicit`
    suffix as equivalent.
    """
    rows = [
        {
            "name": "RuntimeConfig.worker_threads",
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    ffi_setters = {"worker_threads_explicit", "worker_threads"}
    py_setters = {"worker_threads"}
    ts_setters = {"worker_threads"}
    cpp_setters = {"worker_threads"}
    errors = cbp._check_dotted_rows(
        rows, py_setters, ts_setters, cpp_setters, ffi_setters
    )
    assert errors == [], f"_explicit suffix must satisfy; got {errors!r}"


# ─── Positive: per-row `setter` override resolves alternate name ───


def test_setter_override_resolves_alternate_name(tmpdir: pathlib.Path) -> None:
    """`HistoricalConfig.host` binds on Python as `set_historical_host`
    (with the `historical_` prefix). The row's `setter = "historical_host"`
    field overrides the default `host` derivation and the gate accepts
    the binding.
    """
    rows = [
        {
            "name": "HistoricalConfig.host",
            "python": True,
            "typescript": False,
            "cpp": False,
            "setter": "historical_host",
        }
    ]
    py_setters = {"historical_host"}
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


# ─── Public-surface vocabulary guard ────────────────────────────────


def test_surface_vocab_flags_embedded_impl_token() -> None:
    """A public identifier embedding one of OUR impl tokens (tokio)
    trips the guard, even though `\\btokio\\b` would not match it.
    """
    errors = cbp._check_public_surface_vocab(
        {"Config"}, set(), set(),
        {"tokio_worker_threads"}, set(), set(), set(),
        {}, {}, {},
    )
    assert any("tokio" in e for e in errors), (
        f"embedded tokio identifier must trip; got {errors!r}"
    )


def test_surface_vocab_allows_vendor_protocol_names() -> None:
    """Vendor protocol names (mdds / fpss) are allow-listed; the
    `HistoricalClient` class and `historical_host` / `streaming_ring_size`
    setters must NOT trip.
    """
    errors = cbp._check_public_surface_vocab(
        {"HistoricalClient", "StreamingClient"}, set(), set(),
        {"historical_host", "historical_port", "streaming_ring_size"}, set(), set(), set(),
        {}, {}, {},
    )
    assert errors == [], f"vendor protocol names must be clean; got {errors!r}"


def test_surface_vocab_allows_neutral_worker_threads() -> None:
    """The renamed neutral knob is clean on every binding spelling."""
    errors = cbp._check_public_surface_vocab(
        {"WorkerThreadsSetting"}, set(), set(),
        {"worker_threads"}, {"worker_threads_explicit"},
        {"worker_threads_explicit"}, {"worker_threads_explicit"},
        {}, {}, {},
    )
    assert errors == [], f"neutral worker_threads must be clean; got {errors!r}"


# ─── Client-facing setter-SET parity ────────────────────────────────


def test_setter_set_parity_normalizes_and_matches() -> None:
    """Per-binding idioms (`_explicit`, `flat_files`↔`flatfiles`) fold
    away; equal sets are silent.
    """
    py = {"worker_threads", "flatfiles_jitter"}
    ts = {"worker_threads_explicit", "flat_files_jitter", "flatfiles_jitter"}
    cpp = {"worker_threads_explicit", "flatfiles_jitter"}
    ffi = {"worker_threads_explicit", "flatfiles_jitter"}
    errors = cbp._check_setter_set_parity(py, ts, cpp, ffi, exempt={})
    assert errors == [], f"normalized-equal sets must be silent; got {errors!r}"


def test_setter_set_parity_missing_on_ts_trips() -> None:
    """A knob bound on Python/C++/FFI but absent from TS trips — the
    `derive_ohlcvc`-missing-on-TS defect class.
    """
    errors = cbp._check_setter_set_parity(
        {"derive_ohlcvc"}, set(), {"derive_ohlcvc"}, {"derive_ohlcvc"}, exempt={}
    )
    assert any("derive_ohlcvc" in e and "typescript" in e for e in errors), (
        f"missing-on-TS knob must trip; got {errors!r}"
    )


def test_setter_set_parity_exemption_honoured() -> None:
    """A Python-only knob in the exemption map does NOT trip."""
    errors = cbp._check_setter_set_parity(
        {"historical_host", "shared"}, {"shared"}, {"shared"}, {"shared"},
        exempt={"historical_host": "Python-only advanced override"},
    )
    assert errors == [], f"exempted Python-only knob must not trip; got {errors!r}"


def test_setter_set_parity_live_sources_clean() -> None:
    """The shipped exemptions are live against the real binding
    sources: the full setter-set parity gate is clean.
    """
    py = cbp._collect_python_setters(cbp.PY_SRC)
    ts = cbp._collect_typescript_setters(cbp.TS_SRC)
    cpp = cbp._collect_cpp_setters(cbp.CPP_HPP, cbp.CPP_H)
    ffi = cbp._collect_ffi_setters(cbp.FFI_SRC)
    errors = cbp._check_setter_set_parity(py, ts, cpp, ffi)
    assert errors == [], f"live setter-set parity must be clean; got {errors!r}"


# ─── Config getter-SET parity (read side of the knob roster) ───────


def test_getter_set_parity_normalizes_and_matches() -> None:
    """Per-binding idioms fold; equal getter sets are silent."""
    py = {"reconnect_wait_ms", "worker_threads"}
    ts = {"reconnect_wait_ms", "worker_threads_explicit"}
    cpp = {"reconnect_wait_ms", "worker_threads_explicit"}
    ffi = {"reconnect_wait_ms", "worker_threads_explicit"}
    errors = cbp._check_getter_set_parity(py, ts, cpp, ffi, exempt={})
    assert errors == [], f"normalized-equal getter sets must be silent; got {errors!r}"


def test_getter_set_parity_missing_on_ffi_trips() -> None:
    """A read-back getter bound on Python/TS/C++ but absent from the C
    ABI trips — the read-side missing-binding defect class.
    """
    errors = cbp._check_getter_set_parity(
        {"streaming_ring_size"}, {"streaming_ring_size"}, {"streaming_ring_size"}, set(), exempt={}
    )
    assert any("streaming_ring_size" in e and "ffi" in e for e in errors), (
        f"getter missing on FFI must trip; got {errors!r}"
    )


def test_getter_set_parity_live_sources_clean() -> None:
    """The live Config getter roster is symmetric across all four
    bindings: every read-back accessor present in one is present in all.
    """
    py = cbp._collect_python_getters(cbp.PY_SRC)
    ts = cbp._collect_typescript_getters(cbp.TS_SRC)
    cpp = cbp._collect_cpp_getters(cbp.CPP_HPP)
    ffi = cbp._collect_ffi_getters(cbp.FFI_SRC)
    errors = cbp._check_getter_set_parity(py, ts, cpp, ffi)
    assert errors == [], f"live getter-set parity must be clean; got {errors!r}"


def test_getter_collectors_scope_to_config() -> None:
    """The getter collectors harvest only `impl Config` bodies, so a
    getter on an unrelated pyclass is not swept into the knob roster.
    """
    text = (
        "#[pymethods]\nimpl Config {\n    #[getter] fn get_streaming_ring_size(&self) -> usize { 0 }\n}\n"
        "#[pymethods]\nimpl QuoteTick {\n    #[getter] fn bid_price(&self) -> f64 { 0.0 }\n}\n"
    )
    bodies = cbp._iter_impl_config_bodies(text)
    assert len(bodies) == 1, f"only the Config impl body must be picked; got {bodies!r}"
    assert "get_streaming_ring_size" in bodies[0] and "bid_price" not in bodies[0]


# ─── Subscription-kind label parity ────────────────────────────────


def test_subscription_kind_parity_positive() -> None:
    full = set(cbp.CANONICAL_SUBSCRIPTION_KINDS)
    errors = cbp._check_subscription_kind_parity(full, full, full, full, full)
    assert errors == [], f"all-canonical kind sets must be silent; got {errors!r}"


def test_subscription_kind_missing_label_trips() -> None:
    full = set(cbp.CANONICAL_SUBSCRIPTION_KINDS)
    errors = cbp._check_subscription_kind_parity(
        full, full, full, full - {"market_value"}, full
    )
    assert any("cpp" in e and "missing" in e for e in errors), (
        f"a dropped kind label must trip; got {errors!r}"
    )


def test_subscription_kind_fictitious_label_trips() -> None:
    """The C++ `full_quote` / `full_market_value` defect class: a
    full-stream kind that does not exist on the wire trips.
    """
    full = set(cbp.CANONICAL_SUBSCRIPTION_KINDS)
    errors = cbp._check_subscription_kind_parity(
        full, full, full, full | {"full_quote"}, full
    )
    assert any("cpp" in e and "non-canonical" in e for e in errors), (
        f"a fictitious kind label must trip; got {errors!r}"
    )


def test_subscription_kind_live_sources_clean() -> None:
    """Every live binding emits exactly the canonical kind roster."""
    errors = cbp._check_subscription_kind_parity(
        cbp._collect_rust_subscription_kinds(cbp.SUBSCRIPTION_RS),
        cbp._collect_binding_subscription_kinds(cbp.PY_FLUENT_RS),
        cbp._collect_binding_subscription_kinds(cbp.TS_FLUENT_RS),
        cbp._collect_cpp_subscription_kinds(cbp.CPP_HPP),
        cbp._collect_ffi_subscription_kinds(cbp.CPP_H),
    )
    assert errors == [], f"live subscription-kind parity must be clean; got {errors!r}"


# ─── Error-leaf mapping parity ─────────────────────────────────────


def test_error_leaf_parity_positive() -> None:
    leaves = set(cbp.CANONICAL_ERROR_LEAVES)
    codes = dict(cbp.CANONICAL_ERROR_CODES)
    errors = cbp._check_error_leaf_parity(
        leaves, leaves, leaves, codes, set(codes), codes
    )
    assert errors == [], f"symmetric error mapping must be silent; got {errors!r}"


def test_error_leaf_missing_on_python_trips() -> None:
    """A leaf invisible on Python (the missing-`ConfigError` /
    `FlatFilesUnavailable`-routed-nowhere defect class) trips.
    """
    leaves = set(cbp.CANONICAL_ERROR_LEAVES)
    codes = dict(cbp.CANONICAL_ERROR_CODES)
    errors = cbp._check_error_leaf_parity(
        leaves - {"ConfigError"}, leaves, leaves, codes, set(codes), codes
    )
    assert any("python" in e and "ConfigError" in e for e in errors), (
        f"a leaf missing on Python must trip; got {errors!r}"
    )


def test_error_leaf_code_renumber_trips() -> None:
    leaves = set(cbp.CANONICAL_ERROR_LEAVES)
    codes = dict(cbp.CANONICAL_ERROR_CODES)
    bad = dict(codes)
    bad["THETADATADX_ERR_STREAM"] = 99
    errors = cbp._check_error_leaf_parity(
        leaves, leaves, leaves, bad, set(codes), bad
    )
    assert any("ffi" in e and "THETADATADX_ERR_STREAM" in e for e in errors), (
        f"a renumbered FFI code must trip; got {errors!r}"
    )


def test_error_leaf_header_drift_trips() -> None:
    """A C ABI header `#define` disagreeing with the FFI Rust constant
    (invisible to `cargo build`) trips.
    """
    leaves = set(cbp.CANONICAL_ERROR_LEAVES)
    codes = dict(cbp.CANONICAL_ERROR_CODES)
    header = dict(codes)
    header["THETADATADX_ERR_CONFIG"] = 42
    errors = cbp._check_error_leaf_parity(
        leaves, leaves, leaves, codes, set(codes), header
    )
    assert any("cpp header" in e and "THETADATADX_ERR_CONFIG" in e for e in errors), (
        f"a C-header code drift must trip; got {errors!r}"
    )


def test_error_leaf_live_sources_symmetric() -> None:
    """The live error mapping is symmetric across all four bindings."""
    errors = cbp._check_error_leaf_parity(
        cbp._collect_python_error_leaves(cbp.PY_ERRORS_RS),
        cbp._collect_typescript_error_leaves(cbp.TS_LIB_RS),
        cbp._collect_cpp_error_leaves(cbp.CPP_HPP),
        cbp._collect_ffi_error_codes(cbp.FFI_ERROR_RS),
        cbp._collect_ffi_error_codes_dispatched(cbp.FFI_ERROR_RS),
        cbp._collect_cpp_error_codes(cbp.CPP_H),
    )
    assert errors == [], f"live error-leaf parity must be symmetric; got {errors!r}"


# ─── Utility-roster parity ─────────────────────────────────────────


def test_utility_ffi_name_override_resolves() -> None:
    """A row whose C ABI symbol carries a disambiguating prefix resolves
    through `ffi_name`.
    """
    rows = [
        {
            "name": "is_cancel",
            "ffi_name": "condition_is_cancel",
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    errors = cbp._check_utility_rows(
        rows, {"is_cancel"}, {"is_cancel"}, {"is_cancel"}, {"condition_is_cancel"}
    )
    assert errors == [], f"ffi_name override must resolve; got {errors!r}"


def test_utility_binding_specific_asserts_booleans() -> None:
    """A `binding_specific` row still asserts the declared per-binding
    presence — a Python-only util appearing on TS trips.
    """
    rows = [
        {
            "name": "split_date_range",
            "binding_specific": "Python-only",
            "python": True,
            "typescript": False,
            "cpp": False,
            "ffi": False,
        }
    ]
    ok = cbp._check_utility_rows(rows, {"split_date_range"}, set(), set(), set())
    assert ok == [], f"correct binding-specific state must be silent; got {ok!r}"
    drift = cbp._check_utility_rows(
        rows, {"split_date_range"}, {"split_date_range"}, set(), set()
    )
    assert any("split_date_range" in e and "typescript" in e for e in drift), (
        f"a binding-specific util appearing elsewhere must trip; got {drift!r}"
    )


def test_utility_roster_orphan_trips() -> None:
    rows = [{"name": "all_greeks"}]
    errors = cbp._check_utility_roster_complete(
        rows, {"all_greeks", "calendar_status_name"}, {"all_greeks"}
    )
    assert any("calendar_status_name" in e for e in errors), (
        f"an untracked utility must trip; got {errors!r}"
    )


def test_ts_utility_surface_filters_internal() -> None:
    surface = cbp._ts_utility_surface(
        {"all_greeks", "quote_tick_to_arrow_ipc", "bigint_to_i32"},
        {"Util": {"conditionName", "isCancel"}},
    )
    assert {"all_greeks", "condition_name", "is_cancel"} <= surface
    assert "quote_tick_to_arrow_ipc" not in surface
    assert "bigint_to_i32" not in surface


def test_utility_roster_live_complete() -> None:
    """Every standalone utility on the live Python / TypeScript surfaces
    is named by a `[[utility]]` row.
    """
    import tomllib

    data = tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    rows = data.get("utility", [])
    py = cbp._collect_python_utility_functions(cbp.PY_SRC)
    ts = cbp._ts_utility_surface(
        cbp._collect_typescript_utility_functions(cbp.TS_SRC),
        cbp._collect_typescript_class_methods(cbp.TS_SRC),
    )
    errors = cbp._check_utility_roster_complete(rows, py, ts)
    assert errors == [], f"live utility roster must be complete; got {errors!r}"


def test_from_file_parity_all_bound_passes() -> None:
    rows = [
        {
            "name": "HistoricalClient",
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    errors = cbp._check_from_file_rows(
        rows, {"HistoricalClient"}, {"HistoricalClient"}, {"HistoricalClient"}, {"historical"}
    )
    assert errors == [], f"all-bound from_file row must be silent; got {errors!r}"


def test_from_file_missing_on_ffi_trips() -> None:
    rows = [
        {
            "name": "StreamingClient",
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    errors = cbp._check_from_file_rows(
        rows, {"StreamingClient"}, {"StreamingClient"}, {"StreamingClient"}, set()
    )
    assert any("ffi" in e and "missing" in e for e in errors), (
        f"missing C ABI from_file symbol must trip; got {errors!r}"
    )


def test_from_file_untracked_client_trips() -> None:
    errors = cbp._check_from_file_rows(
        [], {"Client"}, set(), set(), set()
    )
    assert any(
        "Client" in e and "no [[from_file]] row" in e for e in errors
    ), f"untracked file-construction client must trip; got {errors!r}"


def test_from_file_ffi_stem_maps_class_name() -> None:
    rows = [
        {
            "name": "Client",
            "python": False,
            "typescript": False,
            "cpp": False,
            "ffi": True,
        }
    ]
    # The class name itself must not satisfy the row; only the mapped stem.
    wrong = cbp._check_from_file_rows(rows, set(), set(), set(), {"theta_data_dx"})
    assert any("ffi" in e for e in wrong), (
        f"class-name stem must not satisfy the row; got {wrong!r}"
    )
    right = cbp._check_from_file_rows(rows, set(), set(), set(), {"client"})
    assert right == [], f"mapped `client` stem must satisfy the row; got {right!r}"


def test_from_file_parity_live_sources_clean() -> None:
    """Every client the live `[[from_file]]` rows declare exposes the
    idiomatic file-construction entry point on the claimed bindings.
    """
    import tomllib

    data = tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    rows = data.get("from_file", [])
    assert rows, "live parity.toml must declare [[from_file]] rows"
    py_methods = cbp._collect_python_class_methods(cbp.PY_SRC)
    ts_methods = cbp._collect_typescript_class_methods(cbp.TS_SRC)
    cpp_methods = cbp._collect_cpp_class_methods(cbp.CPP_HPP)
    errors = cbp._check_from_file_rows(
        rows,
        cbp._collect_python_from_file_classes(py_methods),
        cbp._collect_typescript_from_file_classes(ts_methods),
        cbp._collect_cpp_from_file_classes(cpp_methods),
        cbp._collect_ffi_from_file_stems(cbp.FFI_SRC),
    )
    assert errors == [], f"live from_file sources must be clean; got {errors!r}"


# ─── Client view-accessor reverse-orphan ───────────────────────────


def test_client_view_accessors_all_enrolled_passes() -> None:
    """Every view accessor present on `Client` has an enrolling row."""
    rows = [
        {"class": "Client", "name": "historical", "python": True, "typescript": True, "cpp": True},
        {"class": "Client", "name": "stream", "python": True, "typescript": True, "cpp": True},
        {"class": "Client", "name": "flatFiles", "python": True, "typescript": True, "cpp": True},
    ]
    py_methods = {"Client": {"historical", "stream", "flat_files"}}
    ts_methods = {"Client": {"historical", "stream", "flatFiles"}}
    cpp_methods = {"Client": {"historical", "stream", "flat_files"}}
    errors = cbp._check_method_rows(rows, py_methods, ts_methods, cpp_methods)
    assert errors == [], f"fully-enrolled view accessors must pass; got {errors!r}"


def test_client_view_accessor_orphan_trips() -> None:
    """A view accessor on `Client` with no enrolling row trips the gate."""
    rows = [
        {"class": "Client", "name": "historical", "python": True, "typescript": True, "cpp": True},
        {"class": "Client", "name": "stream", "python": True, "typescript": True, "cpp": True},
        # `flatFiles` deliberately omitted — present on the bindings below.
    ]
    py_methods = {"Client": {"historical", "stream", "flat_files"}}
    ts_methods = {"Client": {"historical", "stream", "flatFiles"}}
    cpp_methods = {"Client": {"historical", "stream", "flat_files"}}
    errors = cbp._check_method_rows(rows, py_methods, ts_methods, cpp_methods)
    assert any("flatFiles" in e and "no Client [[method]] row" in e for e in errors), (
        f"an unenrolled view accessor must trip the reverse-orphan scan; got {errors!r}"
    )


def test_client_view_accessors_live_sources_enrolled() -> None:
    """The live `Client` view accessors are all enrolled and symmetric."""
    import tomllib

    data = tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    rows = data.get("method", [])
    py_methods = cbp._collect_python_class_methods(cbp.PY_SRC)
    ts_methods = cbp._collect_typescript_class_methods(cbp.TS_SRC)
    cpp_methods = cbp._collect_cpp_class_methods(cbp.CPP_HPP)
    errors = cbp._check_method_rows(rows, py_methods, ts_methods, cpp_methods)
    accessor_errors = [e for e in errors if "view accessor" in e]
    assert accessor_errors == [], (
        f"live Client view accessors must be enrolled; got {accessor_errors!r}"
    )


# ─── Historical Rust surface + buffered base family ────────────────


def test_rust_buffered_endpoints_from_registry() -> None:
    """The registry of record yields exactly the 61 buffered endpoints —
    every `[[endpoints]]` entry except the four `*_stream` FPSS subscription
    endpoints.
    """
    rust = cbp._collect_rust_buffered_endpoints(cbp.ENDPOINT_SURFACE_TOML)
    assert len(rust) == 61, f"registry must yield 61 buffered endpoints; got {len(rust)}"
    assert not any(n.endswith("_stream") for n in rust), (
        f"the `*_stream` FPSS endpoints must be excluded; got {sorted(rust)!r}"
    )


def test_rust_streaming_mirror_equals_generated_python() -> None:
    """The Rust streaming classification (registry-of-record mirror of the
    build's `endpoint_streams` SSOT) equals the live generated Python
    `fn stream` surface — both emitted from the same registry, so any desync
    is a real drift.
    """
    rust_stream = cbp._collect_rust_streaming_endpoints(cbp.ENDPOINT_SURFACE_TOML)
    py_stream = cbp._collect_python_streaming_endpoints(cbp.PY_SRC)
    assert rust_stream == py_stream, (
        f"Rust streaming mirror must equal the generated Python stream "
        f"surface; rust-only={sorted(rust_stream - py_stream)!r}, "
        f"py-only={sorted(py_stream - rust_stream)!r}"
    )


def test_cabi_base_matches_registry() -> None:
    """The shipped C-ABI header declares one `_with_options` base symbol per
    buffered endpoint, matching the Rust registry exactly.
    """
    rust = cbp._collect_rust_buffered_endpoints(cbp.ENDPOINT_SURFACE_TOML)
    cabi = cbp._collect_cabi_base_endpoints(cbp.ENDPOINT_WITH_OPTIONS_INC)
    assert rust == cabi, (
        f"C-ABI base set must equal the registry buffered set; "
        f"rust-only={sorted(rust - cabi)!r}, cabi-only={sorted(cabi - rust)!r}"
    )


def test_cabi_header_matches_ffi_source() -> None:
    """The shipped header and the `ffi/src` source declare / define the same
    base symbols — a stale regenerated header that drifted from the source of
    truth is caught.
    """
    cabi = cbp._collect_cabi_base_endpoints(cbp.ENDPOINT_WITH_OPTIONS_INC)
    ffi = cbp._collect_ffi_base_endpoints(cbp.FFI_SRC)
    assert cabi == ffi, (
        f"shipped header must agree with ffi/src base symbols; "
        f"header-only={sorted(cabi - ffi)!r}, source-only={sorted(ffi - cabi)!r}"
    )


def test_historical_async_rust_column_missing_trips() -> None:
    """An async row claiming Rust presence the registry does not back trips —
    the dropped / renamed Rust endpoint defect class.
    """
    rows = [
        {
            "name": "stock_history_eod",
            "rust": True,
            "python": True,
            "typescript": True,
            "cpp": True,
        }
    ]
    bound = {"stock_history_eod"}
    errors = cbp._check_historical_async_rows(rows, set(), bound, bound, bound)
    assert any("rust" in e and "missing" in e for e in errors), (
        f"a dropped Rust async endpoint must trip; got {errors!r}"
    )


def test_historical_streaming_rust_column_missing_trips() -> None:
    """A streaming row claiming Rust presence the registry does not back
    trips.
    """
    rows = [
        {
            "name": "option_history_trade",
            "rust": True,
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    bound = {"option_history_trade"}
    errors = cbp._check_historical_streaming_rows(
        rows, set(), bound, bound, bound, bound
    )
    assert any("rust" in e and "missing" in e for e in errors), (
        f"a dropped Rust streaming endpoint must trip; got {errors!r}"
    )


def test_historical_base_missing_on_cabi_trips() -> None:
    """A base row claiming the C-ABI `_with_options` symbol the shipped
    header does not declare trips — the 61-symbol blind spot.
    """
    rows = [
        {
            "name": "stock_history_eod",
            "rust": True,
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    s = {"stock_history_eod"}
    errors = cbp._check_historical_base_rows(rows, s, s, s, s, set(), set())
    assert any("ffi" in e and "missing" in e for e in errors), (
        f"missing C-ABI base symbol must trip; got {errors!r}"
    )


def test_historical_base_header_source_divergence_trips() -> None:
    """A shipped header that dropped a base symbol the `ffi/src` source still
    defines (a stale regenerated header) trips, independent of any per-row
    column.
    """
    rows = [
        {
            "name": "stock_history_eod",
            "rust": True,
            "python": True,
            "typescript": True,
            "cpp": True,
            "ffi": True,
        }
    ]
    s = {"stock_history_eod"}
    cabi = {"stock_history_eod"}
    ffi = {"stock_history_eod", "option_history_eod"}
    errors = cbp._check_historical_base_rows(rows, s, s, s, s, cabi, ffi)
    assert any("option_history_eod" in e and "stale header" in e for e in errors), (
        f"a header/source divergence must trip; got {errors!r}"
    )


def test_historical_base_untracked_orphan_trips() -> None:
    """An endpoint present on a surface but with no row at all trips the
    reverse-direction orphan scan.
    """
    errors = cbp._check_historical_base_rows(
        [], {"stock_history_eod"}, set(), set(), set(), set(), set()
    )
    assert any(
        "stock_history_eod" in e and "no [[historical_base]] row" in e
        for e in errors
    ), f"untracked base endpoint must trip; got {errors!r}"


def test_historical_base_live_sources_clean() -> None:
    """The live buffered base surface is symmetric across all five surfaces:
    every one of the 61 endpoints present on Rust / Python / TypeScript /
    C++ / the C-ABI base, with the shipped header, the `ffi/src` source, and
    the Rust registry in agreement.
    """
    import tomllib

    data = tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    rows = data.get("historical_base", [])
    assert rows, "live parity.toml must declare [[historical_base]] rows"
    ts_methods = cbp._collect_typescript_class_methods(cbp.TS_SRC)
    cpp_methods = cbp._collect_cpp_class_methods(cbp.CPP_HPP)
    errors = cbp._check_historical_base_rows(
        rows,
        cbp._collect_rust_buffered_endpoints(cbp.ENDPOINT_SURFACE_TOML),
        cbp._collect_python_buffered_endpoints(cbp.PY_SRC),
        cbp._collect_typescript_async_endpoints(ts_methods),
        cpp_methods.get(cbp._cpp_class_for("HistoricalView"), set()),
        cbp._collect_cabi_base_endpoints(cbp.ENDPOINT_WITH_OPTIONS_INC),
        cbp._collect_ffi_base_endpoints(cbp.FFI_SRC),
    )
    assert errors == [], f"live buffered base surface must be clean; got {errors!r}"


def test_historical_families_live_rust_column_clean() -> None:
    """The live `[[historical_async]]` / `[[historical_streaming]]` rows with
    their new `rust` column resolve clean against the registry of record.
    """
    import tomllib

    data = tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    rust_buffered = cbp._collect_rust_buffered_endpoints(cbp.ENDPOINT_SURFACE_TOML)
    rust_stream = cbp._collect_rust_streaming_endpoints(cbp.ENDPOINT_SURFACE_TOML)
    ts_methods = cbp._collect_typescript_class_methods(cbp.TS_SRC)
    cpp_methods = cbp._collect_cpp_class_methods(cbp.CPP_HPP)
    async_errors = cbp._check_historical_async_rows(
        data.get("historical_async", []),
        rust_buffered,
        cbp._collect_python_async_endpoints(cbp.PY_SRC),
        cbp._collect_typescript_async_endpoints(ts_methods),
        cbp._collect_cpp_async_endpoints(cpp_methods),
    )
    stream_errors = cbp._check_historical_streaming_rows(
        data.get("historical_streaming", []),
        rust_stream,
        cbp._collect_python_streaming_endpoints(cbp.PY_SRC),
        cbp._collect_typescript_streaming_endpoints(ts_methods),
        cbp._collect_cpp_streaming_endpoints(cpp_methods),
        cbp._collect_ffi_streaming_endpoints(cbp.FFI_SRC),
    )
    assert async_errors == [], f"live async rust column must be clean; got {async_errors!r}"
    assert stream_errors == [], f"live streaming rust column must be clean; got {stream_errors!r}"


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
    _check("surface-vocab flags embedded impl token", test_surface_vocab_flags_embedded_impl_token)
    _check("surface-vocab allows vendor protocol names", test_surface_vocab_allows_vendor_protocol_names)
    _check("surface-vocab allows neutral worker_threads", test_surface_vocab_allows_neutral_worker_threads)
    _check("setter-set normalizes and matches", test_setter_set_parity_normalizes_and_matches)
    _check("setter-set missing-on-TS trips", test_setter_set_parity_missing_on_ts_trips)
    _check("setter-set exemption honoured", test_setter_set_parity_exemption_honoured)
    _check("setter-set live sources clean", test_setter_set_parity_live_sources_clean)
    _check("from-file all-bound passes", test_from_file_parity_all_bound_passes)
    _check("from-file missing-on-FFI trips", test_from_file_missing_on_ffi_trips)
    _check("from-file untracked client trips", test_from_file_untracked_client_trips)
    _check("from-file FFI stem maps class name", test_from_file_ffi_stem_maps_class_name)
    _check("from-file live sources clean", test_from_file_parity_live_sources_clean)
    _check("client view accessors all-enrolled passes", test_client_view_accessors_all_enrolled_passes)
    _check("client view accessor orphan trips", test_client_view_accessor_orphan_trips)
    _check("client view accessors live sources enrolled", test_client_view_accessors_live_sources_enrolled)
    _check("rust buffered endpoints from registry (61)", test_rust_buffered_endpoints_from_registry)
    _check("rust streaming mirror equals generated python", test_rust_streaming_mirror_equals_generated_python)
    _check("c-abi base matches registry", test_cabi_base_matches_registry)
    _check("c-abi header matches ffi source", test_cabi_header_matches_ffi_source)
    _check("historical-async rust column missing trips", test_historical_async_rust_column_missing_trips)
    _check("historical-streaming rust column missing trips", test_historical_streaming_rust_column_missing_trips)
    _check("historical-base missing on C-ABI trips", test_historical_base_missing_on_cabi_trips)
    _check("historical-base header/source divergence trips", test_historical_base_header_source_divergence_trips)
    _check("historical-base untracked orphan trips", test_historical_base_untracked_orphan_trips)
    _check("historical-base live sources clean", test_historical_base_live_sources_clean)
    _check("historical families live rust column clean", test_historical_families_live_rust_column_clean)

    if _fails:
        print(f"test_check_binding_parity: {len(_fails)} failure(s)")
        for line in _fails:
            print(f"  {line}")
        return 1
    print(f"test_check_binding_parity: all {_total} cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
