"""Round-trip tests for Config.wait_mode and Config.park_interval_us."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from thetadatadx-py/",
        allow_module_level=True,
    )


def test_defaults_are_spin_and_1000_us():
    cfg = client.Config.production()
    assert cfg.wait_mode == "spin"
    assert cfg.park_interval_us == 1000


def test_wait_mode_round_trips_every_variant():
    cfg = client.Config.production()
    for mode in ("busyspin", "park", "backoff", "spin"):
        cfg.wait_mode = mode
        assert cfg.wait_mode == mode


def test_wait_mode_is_case_insensitive_and_normalises_lowercase():
    cfg = client.Config.production()
    cfg.wait_mode = "BACKOFF"
    assert cfg.wait_mode == "backoff"


def test_wait_mode_rejects_unknown():
    cfg = client.Config.production()
    with pytest.raises(Exception):
        cfg.wait_mode = "block"


def test_park_interval_us_round_trips():
    cfg = client.Config.production()
    cfg.park_interval_us = 250
    assert cfg.park_interval_us == 250
