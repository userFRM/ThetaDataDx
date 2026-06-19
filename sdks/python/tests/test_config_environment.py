"""Round-trip tests for the Config.environment readback getter."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built; run `maturin develop` from sdks/python/",
        allow_module_level=True,
    )


def test_production_reads_back_prod():
    cfg = client.Config.production()
    assert cfg.environment == "PROD"


def test_stage_reads_back_stage():
    cfg = client.Config.stage()
    assert cfg.environment == "STAGE"


def test_environment_is_read_only():
    cfg = client.Config.production()
    with pytest.raises(AttributeError):
        cfg.environment = "STAGE"
