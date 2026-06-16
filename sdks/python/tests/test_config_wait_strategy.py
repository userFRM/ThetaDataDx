"""Round-trip tests for Config.wait_strategy + tuning + consumer_cpu."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from sdks/python/",
        allow_module_level=True,
    )


def test_default_wait_strategy_is_low_latency():
    cfg = client.Config.production()
    assert cfg.wait_strategy == "low_latency"


@pytest.mark.parametrize(
    "value", ["low_latency", "balanced", "efficient", "busy_spin"]
)
def test_set_wait_strategy_round_trips(value):
    cfg = client.Config.production()
    cfg.wait_strategy = value
    assert cfg.wait_strategy == value


def test_set_wait_strategy_case_insensitive():
    cfg = client.Config.production()
    cfg.wait_strategy = "BUSY_SPIN"
    assert cfg.wait_strategy == "busy_spin"


def test_set_wait_strategy_invalid_raises_value_error():
    cfg = client.Config.production()
    with pytest.raises(ValueError, match="low_latency.*busy_spin"):
        cfg.wait_strategy = "spin_forever"


def test_default_wait_tuning():
    cfg = client.Config.production()
    assert cfg.wait_spin_iters == 100
    assert cfg.wait_yield_iters == 10
    assert cfg.wait_park_us == 50


def test_wait_tuning_round_trips():
    cfg = client.Config.production()
    cfg.wait_spin_iters = 16
    cfg.wait_yield_iters = 2
    cfg.wait_park_us = 200
    assert cfg.wait_spin_iters == 16
    assert cfg.wait_yield_iters == 2
    assert cfg.wait_park_us == 200


def test_default_consumer_cpu_is_none():
    cfg = client.Config.production()
    assert cfg.consumer_cpu is None


def test_consumer_cpu_round_trips():
    cfg = client.Config.production()
    cfg.consumer_cpu = 3
    assert cfg.consumer_cpu == 3
    cfg.consumer_cpu = None
    assert cfg.consumer_cpu is None
