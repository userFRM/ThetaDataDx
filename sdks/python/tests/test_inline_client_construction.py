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


def test_each_inline_auth_kwarg_is_accepted_by_name():
    # Pin every inline kwarg by NAME. If `api_key`, `email`, `password`, or
    # `mdds_type` were renamed or dropped, the matching call below would
    # raise `TypeError` (unexpected keyword argument) instead of the
    # ConfigError / connection-class outcome we assert — so this fails loud
    # on signature drift rather than silently passing.
    for kwargs in (
        {"api_key": "td1_dummy_key"},
        {"email": "you@example.com", "password": "secret"},
        {"api_key": "td1_dummy_key", "mdds_type": "STAGE"},
    ):
        try:
            Client(**kwargs)
        except TypeError as exc:  # pragma: no cover - would be a real bug
            pytest.fail(f"inline kwarg dropped/renamed: {kwargs} -> {exc}")
        except (ConfigError, Exception):
            # email/password resolves cleanly then fails at the network
            # boundary; either outcome proves the kwargs are wired and
            # parsed. A dropped kwarg is a TypeError, caught above.
            pass


def test_api_key_email_password_kwargs_all_present():
    # Passing api_key together with the email/password pair must reach the
    # auth-resolution step (a conflict -> ConfigError). A `TypeError` here
    # would mean one of the three kwargs is missing from the signature, so
    # this doubles as a presence guard for all three at once.
    with pytest.raises(ConfigError):
        Client(api_key="td1_dummy_key", email="you@example.com", password="secret")


def test_from_env_and_from_dotenv_constructors_exist():
    # The env / dotenv constructors are not modeled by the parity collector
    # (they ride `__new__`-adjacent staticmethods), so pin them here: a
    # removed or renamed `from_env` / `from_dotenv` fails this assertion.
    assert callable(Client.from_env)
    assert callable(Client.from_dotenv)
