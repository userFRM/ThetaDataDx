"""Offline coverage for the FallbackPolicy pyclass + Config wiring.

The live `_with_fallback` end-to-end tests run against the patched
Terminal and are gated by `THETADX_LIVE_PATCHED_TERMINAL`; this module
keeps the pyclass contract covered in CI without that gate.
"""
from __future__ import annotations

import os

import pytest

import thetadatadx as m


# ---------------------------------------------------------------------------
# FallbackPolicy pyclass construction + introspection.
# ---------------------------------------------------------------------------


def test_disabled_constructor() -> None:
    p = m.FallbackPolicy.disabled()
    assert p.variant == "Disabled"
    assert p.base_url is None
    assert "disabled" in repr(p)


def test_rest_on_h2_disconnect_constructor() -> None:
    p = m.FallbackPolicy.rest_on_h2_disconnect("http://127.0.0.1:25503")
    assert p.variant == "RestOnH2Disconnect"
    assert p.base_url == "http://127.0.0.1:25503"
    assert "rest_on_h2_disconnect" in repr(p)


def test_rest_always_for_date_range_constructor() -> None:
    p = m.FallbackPolicy.rest_always_for_date_range(
        "http://127.0.0.1:25503", before=20230101
    )
    assert p.variant == "RestAlwaysForDateRange"
    assert p.base_url == "http://127.0.0.1:25503"
    assert "20230101" in repr(p)


def test_rest_always_constructor() -> None:
    p = m.FallbackPolicy.rest_always("http://127.0.0.1:25503")
    assert p.variant == "RestAlways"
    assert p.base_url == "http://127.0.0.1:25503"


def test_default_rest_base_url_constant_exposed() -> None:
    # Mirrors `thetadatadx::config::DEFAULT_REST_BASE_URL`.
    assert m.DEFAULT_REST_BASE_URL == "http://127.0.0.1:25503"


# ---------------------------------------------------------------------------
# Config.with_rest_fallback round-trips.
# ---------------------------------------------------------------------------


def test_config_defaults_to_disabled_fallback() -> None:
    cfg = m.Config.production()
    assert cfg.fallback_variant == "Disabled"


def test_config_with_rest_fallback_round_trips_all_variants() -> None:
    for builder, expected in [
        (lambda: m.FallbackPolicy.disabled(), "Disabled"),
        (
            lambda: m.FallbackPolicy.rest_on_h2_disconnect(m.DEFAULT_REST_BASE_URL),
            "RestOnH2Disconnect",
        ),
        (
            lambda: m.FallbackPolicy.rest_always_for_date_range(
                m.DEFAULT_REST_BASE_URL, before=20230101
            ),
            "RestAlwaysForDateRange",
        ),
        (
            lambda: m.FallbackPolicy.rest_always(m.DEFAULT_REST_BASE_URL),
            "RestAlways",
        ),
    ]:
        cfg = m.Config.production()
        cfg.with_rest_fallback(builder())
        assert cfg.fallback_variant == expected


def test_config_with_rest_fallback_rejects_non_policy_argument() -> None:
    cfg = m.Config.production()
    with pytest.raises(TypeError):
        cfg.with_rest_fallback("disabled")  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# End-to-end against the live patched Terminal -- gated.
# ---------------------------------------------------------------------------


LIVE_GATE = "THETADX_LIVE_PATCHED_TERMINAL"


def _live_gate_enabled() -> bool:
    return bool(os.environ.get(LIVE_GATE, "").strip())


@pytest.mark.skipif(
    not _live_gate_enabled(), reason=f"set {LIVE_GATE}=1 to run live tests"
)
def test_option_history_quote_with_fallback_live() -> None:
    """End-to-end: 2022-era request routes to REST and returns ticks."""
    creds = m.Credentials.from_file(os.environ.get("THETADX_CREDS", "creds.txt"))
    cfg = m.Config.production()
    cfg.with_rest_fallback(
        m.FallbackPolicy.rest_always_for_date_range(
            m.DEFAULT_REST_BASE_URL, before=20230101
        )
    )
    tdx = m.ThetaDataDxClient(creds, cfg)

    ticks = tdx.option_history_quote_with_fallback(
        symbol="QQQ",
        expiration="20220415",
        start_date="20220414",
        strike="345",
        right="call",
    )
    assert len(ticks) > 0
