"""Smoke tests for Config.from_dotenv (.env-sourced staging selection)."""
from __future__ import annotations

import os

import pytest

try:
    import thetadatadx as client
except ImportError:
    pytest.skip(
        "thetadatadx native extension not built — run `maturin develop` from thetadatadx-py/",
        allow_module_level=True,
    )


def _write_dotenv(tmp_path, name: str, body: str) -> str:
    path = tmp_path / name
    path.write_text(body)
    return os.fspath(path)


def test_from_dotenv_stage_selects_staging_host(tmp_path):
    path = _write_dotenv(tmp_path, "stage.env", "# select staging\nTHETADATA_HISTORICAL_TYPE=STAGE\n")
    cfg = client.Config.from_dotenv(path)
    # A staging `.env` resolves to the staging historical host, distinct
    # from the production host a prod `.env` yields.
    assert cfg.historical_host == "mdds-stage.thetadata.us"


def test_from_dotenv_api_key_only_keeps_production_host(tmp_path):
    path = _write_dotenv(tmp_path, "apikey.env", "THETADATA_API_KEY=td_example_key\n")
    cfg = client.Config.from_dotenv(path)
    # No cluster selector: the production default stays in force, distinct
    # from the staging host a `STAGE` selector would produce.
    assert cfg.historical_host == "mdds-01.thetadata.us"


def test_from_dotenv_prod_and_stage_differ(tmp_path):
    prod_path = _write_dotenv(tmp_path, "prod.env", "THETADATA_HISTORICAL_TYPE=PROD\n")
    stage_path = _write_dotenv(tmp_path, "stage2.env", "THETADATA_HISTORICAL_TYPE=STAGE\n")
    prod = client.Config.from_dotenv(prod_path)
    stage = client.Config.from_dotenv(stage_path)
    assert prod.historical_host != stage.historical_host
