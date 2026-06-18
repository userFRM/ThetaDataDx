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
