"""Round-trip tests for Config.flush_mode (set + get, valid + invalid)."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from thetadatadx-py/",
        allow_module_level=True,
    )


def test_default_flush_mode_is_batched():
    cfg = client.Config.production()
    assert cfg.flush_mode == "batched"


def test_set_flush_mode_immediate_round_trips():
    cfg = client.Config.production()
    cfg.flush_mode = "immediate"
    assert cfg.flush_mode == "immediate"


def test_set_flush_mode_batched_round_trips():
    cfg = client.Config.production()
    cfg.flush_mode = "immediate"
    cfg.flush_mode = "batched"
    assert cfg.flush_mode == "batched"


def test_set_flush_mode_case_insensitive():
    cfg = client.Config.production()
    cfg.flush_mode = "IMMEDIATE"
    assert cfg.flush_mode == "immediate"


def test_set_flush_mode_invalid_raises_value_error():
    cfg = client.Config.production()
    with pytest.raises(ValueError, match="batched.*immediate"):
        cfg.flush_mode = "instant"
