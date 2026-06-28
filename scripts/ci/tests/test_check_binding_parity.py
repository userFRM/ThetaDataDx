#!/usr/bin/env python3
"""Test suite for `scripts/ci/check_binding_parity.py` (Gate 2 / #595).

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

    python3 scripts/ci/tests/test_check_binding_parity.py

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
GATE_DIR = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(GATE_DIR))
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


# ─── ClientBuilder fluent-setter parity ────────────────────────────


def test_client_builder_setter_parity_positive() -> None:
    """Matching Rust/C++ builder setter rosters are silent."""
    errors = cbp._check_client_builder_setter_parity(
        {"api_key", "environment", "from_dotenv"},
        {"api_key", "environment", "from_dotenv"},
        exempt={},
    )
    assert errors == [], f"matching builder setter sets must be silent; got {errors!r}"


def test_client_builder_setter_parity_missing_on_cpp_trips() -> None:
    """A Rust builder setter missing from C++ trips."""
    errors = cbp._check_client_builder_setter_parity(
        {"api_key", "environment"},
        {"api_key"},
        exempt={},
    )
    assert any("environment" in e and "cpp" in e for e in errors), (
        f"a dropped C++ builder setter must trip; got {errors!r}"
    )


def test_client_builder_setter_parity_live_sources_clean() -> None:
    """The shipped Rust and C++ builder setter rosters match."""
    errors = cbp._check_client_builder_setter_parity(
        cbp._collect_rust_client_builder_setters(cbp.RUST_CLIENT_BUILDER_RS),
        cbp._collect_cpp_client_builder_setters(cbp.CPP_HPP),
    )
    assert errors == [], f"live builder setter parity must be clean; got {errors!r}"


# ─── TypeScript connectWith option-field roster ────────────────────


def test_connect_with_field_collector_camelizes() -> None:
    """The collector lifts Rust snake_case fields to JS camelCase."""
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        lib = pathlib.Path(tmp) / "lib.rs"
        lib.write_text(
            "#[napi(object)]\n"
            "pub struct ClientConnectOptions {\n"
            "    pub api_key_from_env: Option<bool>,\n"
            "    pub credentials_file: Option<String>,\n"
            "}\n",
            encoding="utf-8",
        )
        fields = cbp._collect_typescript_connect_with_fields(lib)
    assert fields == {"apiKeyFromEnv", "credentialsFile"}, (
        f"collector must camelCase fields; got {fields!r}"
    )


def test_connect_with_field_roster_missing_field_trips() -> None:
    """A dropped/renamed connectWith field trips."""
    actual = set(cbp.TYPESCRIPT_CONNECT_WITH_FIELD_ROSTER) - {"historicalType"}
    errors = cbp._check_typescript_connect_with_field_roster(actual)
    assert any("historicalType" in e and "missing" in e for e in errors), (
        f"a missing connectWith field must trip; got {errors!r}"
    )


def test_connect_with_field_roster_live_source_clean() -> None:
    """The shipped `ClientConnectOptions` fields equal the pinned roster."""
    errors = cbp._check_typescript_connect_with_field_roster(
        cbp._collect_typescript_connect_with_fields(cbp.TS_LIB_RS)
    )
    assert errors == [], f"live connectWith field roster must be clean; got {errors!r}"


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
    assert any(
        "Client.flatFiles" in e and "no `[[method]]` row" in e for e in errors
    ), (
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


# ─── napi balanced-paren collector (G2) ────────────────────────────


def test_napi_callback_typed_setter_detected(tmp: pathlib.Path) -> None:
    """A callback-typed napi setter (`ts_args_type = "cb: (e) => void",
    js_name = "setX"`) must be collected. The old `#[napi(...[^)]*...)]`
    scan stopped at the `)` inside `(e)` and read it as ABSENT — the G2
    bypass. The balanced-paren scan must see it.
    """
    src = (
        "#[napi]\n"
        "impl Config {\n"
        '    #[napi(ts_args_type = "cb: (e: Event) => void", '
        'js_name = "setStreamCallback")]\n'
        "    pub fn set_stream_callback(&mut self, cb: ThreadsafeFunction)"
        " -> napi::Result<()> { Ok(()) }\n"
        "}\n"
    )
    (tmp / "config.rs").write_text(src, encoding="utf-8")
    setters = cbp._collect_typescript_setters(tmp)
    assert any("stream_callback" in s for s in setters), (
        "callback-typed napi setter was not collected (the balanced-paren "
        f"scan regressed); got {sorted(setters)!r}"
    )


def test_napi_callback_typed_method_detected(tmp: pathlib.Path) -> None:
    """A callback-typed napi class method must be collected — the same
    inner-paren bypass on the class-method collector.
    """
    src = (
        "#[napi]\n"
        "impl StreamView {\n"
        '    #[napi(ts_args_type = "cb: (e: Event) => void", '
        'js_name = "onEvent")]\n'
        "    pub fn on_event(&self, cb: ThreadsafeFunction)"
        " -> napi::Result<()> { Ok(()) }\n"
        "}\n"
    )
    (tmp / "sv.rs").write_text(src, encoding="utf-8")
    methods = cbp._collect_typescript_class_methods(tmp)
    assert "onEvent" in methods.get("StreamView", set()), (
        "callback-typed napi method was not collected on StreamView; got "
        f"{sorted(methods.get('StreamView', set()))!r}"
    )


def test_napi_getter_order_independent(tmp: pathlib.Path) -> None:
    """A `#[napi(js_name = "x", getter)]` (getter flag AFTER js_name) must
    be collected — the balanced scan checks for both tokens regardless of
    order, where the old regex required `getter` before `js_name`.
    """
    src = (
        "#[napi]\n"
        "impl Config {\n"
        '    #[napi(js_name = "fooBar", getter)]\n'
        "    pub fn foo_bar(&self) -> u32 { 0 }\n"
        "}\n"
    )
    (tmp / "g.rs").write_text(src, encoding="utf-8")
    getters = cbp._collect_typescript_getters(tmp)
    assert any("foo_bar" in g for g in getters), (
        f"order-independent getter not collected; got {sorted(getters)!r}"
    )


# ─── C++ method decl-position collector (G11) ──────────────────────


def test_cpp_method_call_site_not_counted_as_decl(tmp: pathlib.Path) -> None:
    """A C++ method called inside another inline body (`return request(...)`)
    must NOT count as a declaration. Deleting the `request` DECLARATION
    while leaving its call sites must make the collector drop `request` —
    the G11 bypass (a deleted decl masked by in-class call sites).
    """
    with_decl = (
        "class FlatFiles {\n"
        "public:\n"
        "    FlatFileRowList request(const std::string& a) const {\n"
        "        return FlatFileRowList(do_it(a));\n"
        "    }\n"
        "    FlatFileRowList option_eod(const std::string& d) const {\n"
        '        return request("OPTION");\n'
        "    }\n"
        "};\n"
    )
    decl_deleted = (
        "class FlatFiles {\n"
        "public:\n"
        "    FlatFileRowList option_eod(const std::string& d) const {\n"
        '        return request("OPTION");\n'
        "    }\n"
        "};\n"
    )
    hpp = tmp / "thetadatadx.hpp"

    hpp.write_text(with_decl, encoding="utf-8")
    methods = cbp._collect_cpp_class_methods(hpp).get("FlatFiles", set())
    assert "request" in methods, (
        f"declared method `request` not collected; got {sorted(methods)!r}"
    )

    hpp.write_text(decl_deleted, encoding="utf-8")
    methods = cbp._collect_cpp_class_methods(hpp).get("FlatFiles", set())
    assert "request" not in methods, (
        "a deleted C++ method declaration was still reported because its "
        "in-class call sites kept the name alive (the G11 bypass); got "
        f"{sorted(methods)!r}"
    )
    # The real surviving accessor must still be collected.
    assert "option_eod" in methods, (
        f"a genuine method declaration was dropped; got {sorted(methods)!r}"
    )


def test_cpp_member_field_not_counted_as_method(tmp: pathlib.Path) -> None:
    """A trailing-underscore member field (`handle_`) must not be collected
    as a method — only `<return-type> name(` declarations count.
    """
    src = (
        "class Foo {\n"
        "public:\n"
        "    int32_t value() const { return handle_; }\n"
        "private:\n"
        "    Handle* handle_;\n"
        "};\n"
    )
    hpp = tmp / "thetadatadx.hpp"
    hpp.write_text(src, encoding="utf-8")
    methods = cbp._collect_cpp_class_methods(hpp).get("Foo", set())
    assert "value" in methods, f"real method dropped; got {sorted(methods)!r}"
    assert "handle_" not in methods, (
        f"a member field was counted as a method; got {sorted(methods)!r}"
    )


# ─── Route-B method signature infrastructure (engaged on real specs) ──


def test_sig_type_map_forward_and_sanction() -> None:
    """The TYPE_MAP is forward-only: a canonical type is satisfied by its
    accepted binding spellings (incl. the `usize`→napi `f64` widening), and a
    spelling outside the cell or an unknown canonical name fails closed."""
    assert cbp._sig_type_agrees("usize", "f64", "ts_napi")
    assert cbp._sig_type_agrees("usize", "size_t", "cpp")
    assert cbp._sig_type_agrees("Option<u64>", "std::optional<uint64_t>", "cpp")
    assert not cbp._sig_type_agrees("i32", "f64", "ts_napi")
    assert not cbp._sig_type_agrees("Mystery", "size_t", "cpp")


def test_sig_extractors_read_each_binding(tmp: pathlib.Path) -> None:
    """Each of the five extractors reads the correct `(params, return)` from a
    synthetic source for its binding view."""
    (tmp / "py").mkdir()
    (tmp / "py" / "m.rs").write_text(
        "#[pymethods]\nimpl W {\n"
        "    pub fn f(&self, py: Python<'_>, n: usize) -> PyResult<()> { Ok(()) }\n}\n",
        encoding="utf-8",
    )
    assert cbp._sig_extract_python(tmp / "py", "W", "f") == (["usize"], "PyResult<()>")

    (tmp / "ts").mkdir()
    (tmp / "ts" / "l.rs").write_text(
        "#[napi]\nimpl W {\n    #[napi]\n    pub fn do_it(&self, n: f64) -> napi::Result<()> { Ok(()) }\n}\n",
        encoding="utf-8",
    )
    assert cbp._sig_extract_ts_napi(tmp / "ts", "W", "doIt") == (["f64"], "napi::Result<()>")

    dts = tmp / "i.d.ts"
    dts.write_text("export class W {\n  doIt(n: number): void\n}\n", encoding="utf-8")
    assert cbp._sig_extract_ts_dts(dts, "W", "doIt") == (["number"], "void")

    hpp = tmp / "w.hpp"
    hpp.write_text(
        "class W {\npublic:\n    void f(size_t n, const std::string& s);\n};\n",
        encoding="utf-8",
    )
    assert cbp._sig_extract_cpp(hpp, "W", "f") == (["size_t", "const std::string&"], "void")

    client = tmp / "client.rs"
    client.write_text(
        "impl W {\n    pub async fn f(&self, n: usize) -> Result<()> { Ok(()) }\n}\n",
        encoding="utf-8",
    )
    assert cbp._sig_extract_rust(client, "W", "f") == (["usize"], "Result<()>")

    (tmp / "ffi").mkdir()
    (tmp / "ffi" / "f.rs").write_text(
        'pub extern "C" fn thetadatadx_w_f(n: usize) -> i32 { 0 }\n', encoding="utf-8"
    )
    assert cbp._sig_extract_ffi(tmp / "ffi", "w_f") == (["usize"], "i32")


def _sig_tree(tmp: pathlib.Path, *, py_params: str, cpp_ret: str = "void") -> dict:
    (tmp / "py").mkdir()
    (tmp / "py" / "m.rs").write_text(
        f"#[pymethods]\nimpl W {{\n    pub fn resize(&self, {py_params}) -> PyResult<()> {{ Ok(()) }}\n}}\n",
        encoding="utf-8",
    )
    (tmp / "ts").mkdir()
    (tmp / "ts" / "l.rs").write_text(
        "#[napi]\nimpl W {\n    #[napi]\n    pub fn resize(&self, n: f64) -> napi::Result<()> { Ok(()) }\n}\n",
        encoding="utf-8",
    )
    hpp = tmp / "w.hpp"
    hpp.write_text(
        f"class W {{\npublic:\n    {cpp_ret} resize(size_t n);\n}};\n", encoding="utf-8"
    )
    (tmp / "ffi").mkdir()
    (tmp / "ffi" / "f.rs").write_text(
        'pub extern "C" fn thetadatadx_w_resize(n: usize) -> i32 { 0 }\n', encoding="utf-8"
    )
    return dict(py_src=tmp / "py", ts_src=tmp / "ts", ts_dts=tmp / "none.d.ts",
                cpp_hpp=hpp, client_rs=tmp / "none.rs", ffi_src=tmp / "ffi")


def _sig_resize_row() -> list:
    return [{
        "class": "W", "name": "resize",
        "python": True, "typescript": True, "cpp": True,
        "ffi_symbol": "w_resize",
        "signature": {"params": ["usize"], "returns": "()",
                      "ts_napi_params": ["f64"]},
    }]


def test_sig_orchestrator_clean_passes(tmp: pathlib.Path) -> None:
    """Matching sources (with the napi f64 sanction) leave the gate silent."""
    paths = _sig_tree(tmp, py_params="n: usize")
    errs = cbp._sig_check_method_signatures(_sig_resize_row(), **paths)
    assert errs == [], f"clean signatures must pass; got {errs!r}"


def test_sig_orchestrator_type_drift_trips(tmp: pathlib.Path) -> None:
    """A Python param type drift (`usize`→`bool`) trips the gate."""
    paths = _sig_tree(tmp, py_params="n: bool")
    errs = cbp._sig_check_method_signatures(_sig_resize_row(), **paths)
    assert any("python" in e and "type mismatch" in e for e in errs), errs


def test_sig_orchestrator_arity_drift_trips(tmp: pathlib.Path) -> None:
    """An extra Python param trips the arity check."""
    paths = _sig_tree(tmp, py_params="n: usize, extra: bool")
    errs = cbp._sig_check_method_signatures(_sig_resize_row(), **paths)
    assert any("python" in e and "arity mismatch" in e for e in errs), errs


def test_sig_orchestrator_return_drift_trips(tmp: pathlib.Path) -> None:
    """A C++ return drift (`void`→`int32_t`) trips the return check."""
    paths = _sig_tree(tmp, py_params="n: usize", cpp_ret="int32_t")
    errs = cbp._sig_check_method_signatures(_sig_resize_row(), **paths)
    assert any("cpp" in e and "return mismatch" in e for e in errs), errs


def test_sig_name_only_fails_closed(tmp: pathlib.Path) -> None:
    """Fail-closed enrollment: a `[[method]]` row without `[method.signature]`
    FAILS unless it is in `NAME_ONLY_METHOD_ALLOWLIST`; the allowlisted row
    passes. No new row can be silently name-only."""
    paths = _sig_tree(tmp, py_params="n: bool")
    rows = [{"class": "W", "name": "resize", "python": True}]
    errs = cbp._sig_check_method_signatures(rows, **paths)
    assert any("neither a `[method.signature]`" in e for e in errs), errs
    cbp.NAME_ONLY_METHOD_ALLOWLIST[("W", "resize")] = "test fixture"
    try:
        assert cbp._sig_check_method_signatures(rows, **paths) == []
    finally:
        del cbp.NAME_ONLY_METHOD_ALLOWLIST[("W", "resize")]


def test_sig_extractor_getter_prefix_and_decl_position() -> None:
    """The extractor fixes the first real exercise surfaced: a `get_`-prefixed
    pyo3/C++ readback resolves against the bare row name; a C++ elaborated-type
    parameter (`const class X&`) is not mistaken for the class def; an in-body
    member-access call (`handle_.get()`) does not shadow the real declaration."""
    (tmp_root := cbp.pathlib.Path(__import__("tempfile").mkdtemp()))
    try:
        (tmp_root / "py").mkdir()
        (tmp_root / "py" / "m.rs").write_text(
            "#[pymethods]\nimpl Config {\n    #[getter]\n"
            "    fn get_worker_threads(&self) -> Option<usize> { None }\n}\n",
            encoding="utf-8",
        )
        # get_ prefix on Python.
        assert cbp._sig_extract_python(tmp_root / "py", "Config", "worker_threads") == (
            [], "Option<usize>"
        )
        hpp = tmp_root / "h.hpp"
        hpp.write_text(
            # An elaborated-type param usage with its own body brace BEFORE the
            # real class def; and an in-body `.get()` call before the real decl.
            "class Other {\npublic:\n    void use(const class Rows& r) const { (void)r; }\n};\n"
            "class Rows {\npublic:\n"
            "    size_t size() const { return handle_ ? count(handle_.get()) : 0; }\n"
            "    const Handle* get() const noexcept { return handle_.get(); }\n"
            "    std::optional<size_t> get_worker_threads() const;\n};\n",
            encoding="utf-8",
        )
        assert cbp._sig_extract_cpp(hpp, "Rows", "get") == ([], "const Handle*")
        assert cbp._sig_extract_cpp(hpp, "Rows", "worker_threads") == (
            [], "std::optional<size_t>"
        )
    finally:
        __import__("shutil").rmtree(tmp_root, ignore_errors=True)


def test_sig_ffi_opaque_and_return_normalization() -> None:
    """The FFI lang compares unmapped opaque pointers / C-ABI structs by exact
    spelling; a return is unwrapped of its fallible-result wrapper and folded of
    lifetimes / the napi prelude path before the type compare."""
    assert cbp._sig_type_agrees(
        "*mut ThetaDataDxRecordBatchStream", "*mut ThetaDataDxRecordBatchStream", "ffi"
    )
    assert not cbp._sig_type_agrees(
        "*const ThetaDataDxClient", "*const ThetaDataDxClient", "python"
    )
    # `&'static str` folds to satisfy the String canonical in return position.
    assert cbp._sig_compare_one("X.k", ([], "String"), ([], "&'static str"), "python") == []
    # `napi::Result<Option<u32>>` unwraps to the structural Option compare.
    assert cbp._sig_compare_one(
        "X.w", ([], "Option<usize>"), ([], "napi::Result<Option<u32>>"), "ts_napi"
    ) == []


def test_sig_skip_langs_and_ffi_symbol_signature() -> None:
    """`skip_langs` opts a present-but-not-napi-fn binding out; an
    `[[ffi_symbol]]` row's `[ffi_symbol.signature]` is extracted + compared, and
    a drift in the pinned C shape trips."""
    tmp_root = cbp.pathlib.Path(__import__("tempfile").mkdtemp())
    try:
        (tmp_root / "ffi").mkdir()
        (tmp_root / "ffi" / "f.rs").write_text(
            'pub unsafe extern "C" fn thetadatadx_client_batches_open('
            "h: *const ThetaDataDxClient, n: usize) "
            "-> *mut ThetaDataDxRecordBatchStream { core::ptr::null_mut() }\n",
            encoding="utf-8",
        )
        syms = {"client_batches_open"}
        good = [{"name": "client_batches_open", "signature": {
            "params": ["*const ThetaDataDxClient", "usize"],
            "returns": "*mut ThetaDataDxRecordBatchStream"}}]
        assert cbp._check_ffi_symbol_rows(good, syms, tmp_root / "ffi") == []
        bad = [{"name": "client_batches_open", "signature": {
            "params": ["*const ThetaDataDxClient", "usize"],
            "returns": "*mut ThetaDataDxArrowBytes"}}]
        assert any(
            "return mismatch" in e
            for e in cbp._check_ffi_symbol_rows(bad, syms, tmp_root / "ffi")
        )
    finally:
        __import__("shutil").rmtree(tmp_root, ignore_errors=True)
    # skip_langs silences a lang whose extractor finds nothing.
    sig = {"params": ["usize"], "returns": "()", "skip_langs": ["ts_napi"]}
    assert cbp._sig_spec_for(sig, "ts_napi") is None
    assert cbp._sig_spec_for(sig, "python") == (["usize"], "()")


def test_sig_live_surface_engaged_and_clean() -> None:
    """The live parity.toml now carries real `[method.signature]` /
    `[ffi_symbol.signature]` specs (Phase 4a engages the gate); the signature
    check extracts every enrolled binding from the real sources and must find
    them all satisfying the specs."""
    data = cbp.tomllib.loads(cbp.PARITY_TOML.read_text(encoding="utf-8"))
    method_rows = data.get("method", [])
    assert any(r.get("signature") for r in method_rows), (
        "Phase 4a engages the gate: live [[method]] rows must carry "
        "[method.signature] sub-tables"
    )
    errs = cbp._sig_check_method_signatures(
        method_rows, py_src=cbp.PY_SRC, ts_src=cbp.TS_SRC, ts_dts=cbp.TS_DTS,
        cpp_hpp=cbp.CPP_HPP, client_rs=cbp.CORE_CLIENT_RS, ffi_src=cbp.FFI_SRC,
    )
    assert errs == [], f"live method signature gate must be clean; got {errs!r}"
    ffi_rows = data.get("ffi_symbol", [])
    assert any(r.get("signature") for r in ffi_rows)
    ffi_errs = cbp._check_ffi_symbol_rows(
        ffi_rows, cbp._collect_ffi_all_symbols(cbp.FFI_SRC), cbp.FFI_SRC
    )
    assert ffi_errs == [], f"live FFI signature gate must be clean; got {ffi_errs!r}"


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
    # napi balanced-paren collector (G2) + C++ decl-position collector (G11)
    with tempfile.TemporaryDirectory() as tmp:
        _check(
            "napi callback-typed setter detected",
            lambda: test_napi_callback_typed_setter_detected(pathlib.Path(tmp)),
        )
    with tempfile.TemporaryDirectory() as tmp:
        _check(
            "napi callback-typed method detected",
            lambda: test_napi_callback_typed_method_detected(pathlib.Path(tmp)),
        )
    with tempfile.TemporaryDirectory() as tmp:
        _check(
            "napi getter order-independent",
            lambda: test_napi_getter_order_independent(pathlib.Path(tmp)),
        )
    with tempfile.TemporaryDirectory() as tmp:
        _check(
            "cpp method call-site not counted as decl",
            lambda: test_cpp_method_call_site_not_counted_as_decl(pathlib.Path(tmp)),
        )
    with tempfile.TemporaryDirectory() as tmp:
        _check(
            "cpp member field not counted as method",
            lambda: test_cpp_member_field_not_counted_as_method(pathlib.Path(tmp)),
        )

    # Route-B method-signature infrastructure (Phase 3).
    _check("sig type-map forward + sanction + fail-closed", test_sig_type_map_forward_and_sanction)
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig extractors read each binding", lambda: test_sig_extractors_read_each_binding(pathlib.Path(tmp)))
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig orchestrator clean passes", lambda: test_sig_orchestrator_clean_passes(pathlib.Path(tmp)))
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig orchestrator TYPE drift trips", lambda: test_sig_orchestrator_type_drift_trips(pathlib.Path(tmp)))
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig orchestrator ARITY drift trips", lambda: test_sig_orchestrator_arity_drift_trips(pathlib.Path(tmp)))
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig orchestrator RETURN drift trips", lambda: test_sig_orchestrator_return_drift_trips(pathlib.Path(tmp)))
    with tempfile.TemporaryDirectory() as tmp:
        _check("sig name-only fails closed unless allowlisted", lambda: test_sig_name_only_fails_closed(pathlib.Path(tmp)))
    # Route-B signature specs ENGAGED (Phase 4a): extractor fixes + opaque/FFI
    # type handling + skip_langs + the live engaged-and-clean surface.
    _check("sig extractor get_ prefix + decl position", test_sig_extractor_getter_prefix_and_decl_position)
    _check("sig FFI opaque + return normalization", test_sig_ffi_opaque_and_return_normalization)
    _check("sig skip_langs + ffi_symbol signature", test_sig_skip_langs_and_ffi_symbol_signature)
    _check("sig live surface engaged + clean", test_sig_live_surface_engaged_and_clean)

    if _fails:
        print(f"test_check_binding_parity: {len(_fails)} failure(s)")
        for line in _fails:
            print(f"  {line}")
        return 1
    print(f"test_check_binding_parity: all {_total} cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
