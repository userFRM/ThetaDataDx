"""Regression coverage for the `InterestRateTick` schema fix.

Before this PR the `tick_schema.toml` definition advertised three fields
(`ms_of_day`, `rate`, `date`) but the upstream v3 server actually emits
two columns: an ISO-date `created` header and a `rate` percent value.
Every live `interest_rate_history_eod` call therefore failed inside the
decoder with `expected: "Number|Timestamp", got Text`.

The fix is cross-binding by design — the Python `InterestRateTick`
pyclass is regenerated from `tick_schema.toml`, so the shape of the
pyclass itself is the single source of truth this test pins. Liveness
of the decode is covered upstream in
`crates/thetadatadx/tests/test_interest_rate_schema.rs`; here we lock
the Python-facing surface so a future schema regression cannot ship a
wheel whose `InterestRateTick(...)` constructor still accepts the
removed `ms_of_day` keyword.
"""

from __future__ import annotations

import importlib
import inspect

import pytest


@pytest.fixture(scope="module")
def InterestRateTick():
    mod = importlib.import_module("thetadatadx")
    cls = getattr(mod, "InterestRateTick", None)
    assert cls is not None, "thetadatadx.InterestRateTick must be exported"
    return cls


def test_interest_rate_tick_constructs_with_two_kw_fields(InterestRateTick) -> None:
    """The keyword-only constructor accepts `date` and `rate` (and only
    those). The reference row from the wire-format pin is SOFR
    `2025-04-28` -> `date=20250428`, `rate=4.36`."""
    tick = InterestRateTick(date=20250428, rate=4.36)
    assert tick.date == 20250428
    assert tick.rate == pytest.approx(4.36)


def test_interest_rate_tick_default_constructor_zero_fields(InterestRateTick) -> None:
    """The keyword-only `__new__` defaults every field to zero. Prior
    versions also defaulted the (now-removed) `ms_of_day` field — this
    test pins that the constructor no longer requires or accepts it."""
    tick = InterestRateTick()
    assert tick.date == 0
    assert tick.rate == 0.0


def test_interest_rate_tick_rejects_removed_ms_of_day_kw(InterestRateTick) -> None:
    """Pin the breaking change: `ms_of_day` was a fictitious field
    (server never sent it) and has been removed. A keyword call that
    targets the removed field must raise `TypeError` — the wheel is
    explicitly NOT backward-compatible with v10.x `InterestRateTick(
    ms_of_day=...)` call sites."""
    with pytest.raises(TypeError):
        InterestRateTick(ms_of_day=34_200_000, rate=4.36, date=20250428)


def test_interest_rate_tick_has_no_ms_of_day_attribute(InterestRateTick) -> None:
    """Instances no longer expose `ms_of_day`. A live decode of a SOFR
    response previously surfaced a `.ms_of_day` zero on every tick;
    callers that read it should now read `.date` (YYYYMMDD)."""
    tick = InterestRateTick(date=20250428, rate=4.36)
    assert not hasattr(tick, "ms_of_day"), (
        "InterestRateTick must not carry the fictitious `ms_of_day` field"
    )


def test_interest_rate_tick_repr_uses_two_fields(InterestRateTick) -> None:
    """`__repr__` advertises the new 2-field shape; downstream log
    scrapers / debug dumps that grep for the field set break loud rather
    than silently miss the schema change."""
    tick = InterestRateTick(date=20250428, rate=4.36)
    rep = repr(tick)
    assert "date=20250428" in rep
    assert "rate=4.36" in rep
    assert "ms_of_day" not in rep


def test_interest_rate_tick_signature_pins_two_params(InterestRateTick) -> None:
    """The synthesised `__init__` signature lists exactly `date` and
    `rate` (both keyword-only with defaults)."""
    sig = inspect.signature(InterestRateTick)
    names = list(sig.parameters)
    assert names == ["date", "rate"], (
        f"InterestRateTick constructor params drifted: {names!r}"
    )


def test_interest_rate_tick_list_round_trips(InterestRateTick) -> None:
    """The `InterestRateTickList` companion accepts the new 2-field
    pyclass and round-trips through `__getitem__`. This is the path
    that historical methods return to Python callers; it must stay
    indexable and yield instances whose `date`/`rate` survive the
    round-trip."""
    mod = importlib.import_module("thetadatadx")
    InterestRateTickList = getattr(mod, "InterestRateTickList", None)
    assert InterestRateTickList is not None, (
        "thetadatadx.InterestRateTickList must be exported"
    )

    reference_rows = [
        (20250428, 4.36),
        (20250429, 4.36),
        (20250430, 4.41),
        (20250501, 4.39),
        (20250502, 4.36),
    ]
    ticks = [InterestRateTick(date=d, rate=r) for d, r in reference_rows]
    tick_list = InterestRateTickList(ticks)
    assert len(tick_list) == len(reference_rows)
    for i, (d, r) in enumerate(reference_rows):
        got = tick_list[i]
        assert got.date == d
        assert got.rate == pytest.approx(r)
