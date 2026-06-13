"""Cross-binding error-class parity — Python surface (issue #681).

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
def tdx():
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
def test_canonical_class_exists_and_roots_at_theta_data_error(tdx, name: str) -> None:
    cls = getattr(tdx, name, None)
    assert isinstance(cls, type), f"thetadatadx.{name} must be an exception class"
    assert issubclass(
        cls, tdx.ThetaDataError
    ), f"{name} must derive from ThetaDataError"


def test_invalid_credentials_narrows_authentication_error(tdx) -> None:
    assert issubclass(tdx.InvalidCredentialsError, tdx.AuthenticationError)


def test_legacy_names_are_assignment_aliases(tdx) -> None:
    # The legacy names must be the *same object* as their canonical
    # replacement so `except thetadatadx.NoDataFoundError` keeps catching
    # the dispatched `NotFoundError`.
    assert tdx.NoDataFoundError is tdx.NotFoundError
    assert tdx.TimeoutError is tdx.DeadlineExceededError


def test_legacy_except_clause_catches_canonical(tdx) -> None:
    # Raising the canonical class is caught by an `except` on the alias.
    with pytest.raises(tdx.NoDataFoundError):
        raise tdx.NotFoundError("no rows")
    with pytest.raises(tdx.TimeoutError):
        raise tdx.DeadlineExceededError("deadline")


def test_rate_limit_carries_retry_after_attribute(tdx) -> None:
    # The attribute is present (default None) on the class, so callers
    # can read `err.retry_after` unconditionally.
    assert hasattr(tdx.RateLimitError, "retry_after")
    inst = tdx.RateLimitError("429")
    assert inst.retry_after is None


def test_invalid_parameter_is_distinct_from_root(tdx) -> None:
    # A dedicated subclass under the root, not the root itself.
    assert tdx.InvalidParameterError is not tdx.ThetaDataError
    assert issubclass(tdx.InvalidParameterError, tdx.ThetaDataError)
