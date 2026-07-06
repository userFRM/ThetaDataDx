"""Round-trip tests for the per-channel Config environment readback getters."""
from __future__ import annotations

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built; run `maturin develop` from thetadatadx-py/",
        allow_module_level=True,
    )


def test_production_reads_back_prod_on_both_channels():
    cfg = client.Config.production()
    assert cfg.market_data_environment == "PROD"
    assert cfg.streaming_environment == "PROD"


def test_stage_selects_market_data_staging_and_leaves_streaming_on_prod():
    cfg = client.Config.stage()
    assert cfg.market_data_environment == "STAGE"
    assert cfg.streaming_environment == "PROD"


def test_dev_selects_streaming_dev_and_leaves_market_data_on_prod():
    cfg = client.Config.dev()
    assert cfg.market_data_environment == "PROD"
    assert cfg.streaming_environment == "DEV"


def test_environment_getters_are_read_only():
    cfg = client.Config.production()
    with pytest.raises(AttributeError):
        cfg.market_data_environment = "STAGE"
    with pytest.raises(AttributeError):
        cfg.streaming_environment = "DEV"
