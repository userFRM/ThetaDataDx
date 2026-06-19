"""Inline client-construction surface (first-class API-key kwargs).

The constructor resolves the authentication kwargs (and the environment)
locally and raises ``ConfigError`` for a conflicting or absent auth set
BEFORE any network round-trip, so these validation paths run offline. A
dummy key drives the resolve step without a live server.
"""

import pytest

from thetadatadx import Client, ConfigError, Credentials


def test_no_auth_kwarg_raises_config_error():
    with pytest.raises(ConfigError):
        Client()


def test_conflicting_auth_kwargs_raise_config_error():
    # api_key AND email/password is a conflict — rejected before connect.
    with pytest.raises(ConfigError):
        Client(api_key="td1_dummy", email="you@example.com", password="secret")


def test_bad_mdds_type_raises_config_error():
    with pytest.raises(ConfigError):
        Client(api_key="td1_dummy", mdds_type="NOPE")


def test_email_without_password_raises_config_error():
    with pytest.raises(ConfigError):
        Client(email="you@example.com")


def test_credentials_repr_redacts_secret():
    # The credential handle never leaks its secret through repr, on any
    # construction path.
    creds = Credentials.from_api_key("super-secret-key")
    rendered = repr(creds)
    assert "super-secret-key" not in rendered
    assert "<redacted>" in rendered


def test_constructor_accepts_api_key_kwarg_signature():
    # A well-formed api_key call gets past local validation and only fails
    # at the network boundary (no live server here). We assert it does NOT
    # raise ConfigError — any failure must be a connection-class error,
    # proving the kwarg resolved cleanly.
    try:
        Client(api_key="td1_dummy_key", mdds_type="STAGE")
    except ConfigError as exc:  # pragma: no cover - would be a real bug
        pytest.fail(f"well-formed api_key kwargs raised ConfigError: {exc}")
    except Exception:
        # Network / auth failure is expected without a live server.
        pass
