"""Round-trip tests for Config.consumer_cpu."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from thetadatadx-py/",
        allow_module_level=True,
    )


def test_default_consumer_cpu_is_none():
    cfg = client.Config.production()
    assert cfg.consumer_cpu is None


def test_consumer_cpu_round_trips():
    cfg = client.Config.production()
    cfg.consumer_cpu = 3
    assert cfg.consumer_cpu == 3
    cfg.consumer_cpu = None
    assert cfg.consumer_cpu is None
