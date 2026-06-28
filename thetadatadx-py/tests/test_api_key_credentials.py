"""API-key credential construction + redaction.

Mirrors the email + password constructor smoke coverage: build
credentials through every API-key entry point and confirm the repr never
exposes the key or the email.
"""

from __future__ import annotations

import importlib

import pytest


@pytest.fixture(scope="module")
def mod():
    return importlib.import_module("thetadatadx")


def test_from_api_key_builds_credentials(mod) -> None:
    creds = mod.Credentials.from_api_key("super-secret-key")
    assert creds is not None


def test_from_api_key_with_email_builds_credentials(mod) -> None:
    creds = mod.Credentials.from_api_key_with_email(
        "user@example.com", "super-secret-key"
    )
    assert creds is not None


def test_from_env_sources_strictly_from_env(mod, monkeypatch) -> None:
    monkeypatch.setenv("THETADATA_API_KEY", "env-sourced-key")
    creds = mod.Credentials.from_env()
    assert creds is not None
    # The strict env factory holds the key as secret material; the repr
    # never exposes it.
    rendered = repr(creds)
    assert "env-sourced-key" not in rendered
    assert "<redacted>" in rendered


def test_from_env_strict_raises_when_unset(mod, monkeypatch) -> None:
    # Strict: an unset THETADATA_API_KEY raises, with NO creds.txt fallback.
    # The strict-unset case is an invalid-parameter config fault, a subclass
    # of the binding's ThetaDataError base (the same base ConfigError derives
    # from), so a `except ThetaDataError` clause covers it.
    monkeypatch.delenv("THETADATA_API_KEY", raising=False)
    with pytest.raises(mod.ThetaDataError):
        mod.Credentials.from_env()


def test_from_env_or_file_sources_from_env(mod, monkeypatch) -> None:
    monkeypatch.setenv("THETADATA_API_KEY", "env-sourced-key")
    creds = mod.Credentials.from_env_or_file("/nonexistent/creds.txt")
    assert creds is not None


def test_from_env_or_file_falls_back_to_file(mod, monkeypatch, tmp_path) -> None:
    monkeypatch.delenv("THETADATA_API_KEY", raising=False)
    creds_file = tmp_path / "creds.txt"
    creds_file.write_text("user@example.com\nsuper-secret-pw\n")
    creds = mod.Credentials.from_env_or_file(str(creds_file))
    assert creds is not None


def test_from_dotenv_reads_api_key(mod, tmp_path) -> None:
    env_file = tmp_path / ".env"
    env_file.write_text('# comment\nTHETADATA_API_KEY="td_example_key"\n')
    creds = mod.Credentials.from_dotenv(str(env_file))
    assert creds is not None


def test_from_dotenv_repr_redacts_secret(mod, tmp_path) -> None:
    env_file = tmp_path / ".env"
    env_file.write_text("THETADATA_API_KEY=td_secret_value\n")
    creds = mod.Credentials.from_dotenv(str(env_file))
    rendered = repr(creds)
    assert "td_secret_value" not in rendered, (
        f"Credentials repr leaked the .env API key: {rendered}"
    )
    assert "<redacted>" in rendered


def test_api_key_repr_redacts_secret(mod) -> None:
    creds = mod.Credentials.from_api_key("super-secret-key")
    rendered = repr(creds)
    assert "super-secret-key" not in rendered, (
        f"Credentials repr leaked the API key: {rendered}"
    )
    assert "<redacted>" in rendered, (
        f"Credentials repr missing the redaction marker: {rendered}"
    )


def test_api_key_with_email_repr_redacts_both(mod) -> None:
    creds = mod.Credentials.from_api_key_with_email(
        "user@example.com", "super-secret-key"
    )
    rendered = repr(creds)
    assert "super-secret-key" not in rendered, (
        f"Credentials repr leaked the API key: {rendered}"
    )
    assert "user@example.com" not in rendered, (
        f"Credentials repr leaked the email: {rendered}"
    )
    assert "<redacted>" in rendered
