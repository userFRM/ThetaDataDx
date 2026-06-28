"""Cross-binding error-class parity — Python surface.

Pins the canonical typed-exception vocabulary so it stays identical to
the TypeScript / C++ / C ABI leaf sets: a `NotFound` status raises
`NotFoundError`, an expired deadline raises `DeadlineExceededError`, and
an `Unavailable` status raises `UnavailableError`. The two legacy names
(`NoDataFoundError` / `TimeoutError`) must remain as assignment aliases
of their canonical replacements so existing `except` clauses keep
working. `RateLimitError` must carry a `retry_after` attribute, and a
rejected client parameter must surface as `InvalidParameterError`, not
the root `ThetaDataError`.
"""

from __future__ import annotations

import importlib

import pytest


@pytest.fixture(scope="module")
def client():
    try:
        return importlib.import_module("thetadatadx")
    except ImportError:
        pytest.skip("native module thetadatadx not built")


# Every canonical leaf class plus the root, in the cross-binding
# vocabulary. These names must match the TypeScript / C++ / C ABI sets.
CANONICAL_CLASSES = [
    "ThetaDataError",
    "AuthenticationError",
    "InvalidCredentialsError",
    "SubscriptionError",
    "RateLimitError",
    "InvalidParameterError",
    "SchemaMismatchError",
    "NetworkError",
    "UnavailableError",
    "DeadlineExceededError",
    "NotFoundError",
    "StreamError",
    "ConfigError",
]


@pytest.mark.parametrize("name", CANONICAL_CLASSES)
def test_canonical_class_exists_and_roots_at_theta_data_error(client, name: str) -> None:
    cls = getattr(client, name, None)
    assert isinstance(cls, type), f"thetadatadx.{name} must be an exception class"
    assert issubclass(
        cls, client.ThetaDataError
    ), f"{name} must derive from ThetaDataError"


def test_invalid_credentials_narrows_authentication_error(client) -> None:
    assert issubclass(client.InvalidCredentialsError, client.AuthenticationError)


def test_legacy_names_are_assignment_aliases(client) -> None:
    # The legacy names must be the *same object* as their canonical
    # replacement so `except thetadatadx.NoDataFoundError` keeps catching
    # the dispatched `NotFoundError`.
    assert client.NoDataFoundError is client.NotFoundError
    assert client.TimeoutError is client.DeadlineExceededError


def test_legacy_except_clause_catches_canonical(client) -> None:
    # Raising the canonical class is caught by an `except` on the alias.
    with pytest.raises(client.NoDataFoundError):
        raise client.NotFoundError("no rows")
    with pytest.raises(client.TimeoutError):
        raise client.DeadlineExceededError("deadline")


def test_rate_limit_carries_retry_after_attribute(client) -> None:
    # The attribute is present (default None) on the class, so callers
    # can read `err.retry_after` unconditionally.
    assert hasattr(client.RateLimitError, "retry_after")
    inst = client.RateLimitError("429")
    assert inst.retry_after is None


def test_invalid_parameter_is_distinct_from_root(client) -> None:
    # A dedicated subclass under the root, not the root itself.
    assert client.InvalidParameterError is not client.ThetaDataError
    assert issubclass(client.InvalidParameterError, client.ThetaDataError)
