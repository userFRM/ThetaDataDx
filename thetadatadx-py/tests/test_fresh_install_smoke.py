"""Fresh-install smoke test.

Acts like a first-time user typing `pip install thetadatadx` then
`from thetadatadx import <X>` for every documented top-level export.
Catches "documented but unreachable" regressions — a name that the docs
promise but the compiled module never registers.

The CI workflow runs this against a wheel freshly installed into an
isolated venv — see `python.yml::build`. The test deliberately uses
`importlib.import_module("thetadatadx")` rather than `from thetadatadx
import ...` so a missing name fails on the assertion, not on the
import-statement itself (which gives a less actionable error).
"""

from __future__ import annotations

import importlib

import pytest


# Top-level names the user-facing docs (lib.rs module doc, README.md,
# the published API reference) advertise as importable directly off the
# package namespace. Grouped for readability; the test parametrises
# across the flat union.
PUBLIC_CLASSES = [
    # Credentials + config
    "Credentials",
    "Config",
    # Sync + async clients
    "Client",
    "AsyncClient",
    "StreamingClient",
    "MarketDataClient",
    # Streaming sessions
    "StreamingSession",
    # Fluent surface
    "Contract",
    "Subscription",
    "SecType",
    # FPSS event payload classes — the high-traffic ones used in every
    # streaming example. Full coverage of the 21 event variants lives
    # in `test_module_exports.py`.
    "Quote",
    "Trade",
    "Ohlcvc",
    "OpenInterest",
    "MarketValue",
    "ContractRef",
    "Connected",
    "Disconnected",
    "LoginSuccess",
    # FlatFiles namespace
    "FlatFilesNamespace",
    "FlatFileRowList",
    # Typed exceptions — names mirror `thetadatadx-py/src/errors.rs`
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
    # Back-compatibility aliases of the canonical classes above.
    "NoDataFoundError",
    "TimeoutError",
]

PUBLIC_FUNCTIONS = [
    "decode_response_bytes",
    "split_date_range",
]


@pytest.fixture(scope="module")
def mod():
    return importlib.import_module("thetadatadx")


@pytest.mark.parametrize("name", PUBLIC_CLASSES)
def test_class_importable(mod, name: str) -> None:
    obj = getattr(mod, name, None)
    assert obj is not None, (
        f"`from thetadatadx import {name}` would raise ImportError — "
        f"documented public class is missing from the installed wheel."
    )
    # Belt-and-braces: catch the case where something registers as the
    # name but is the wrong kind of object (e.g. a stub `None` or a
    # placeholder string left over from a half-removed export).
    assert isinstance(obj, type) or callable(obj), (
        f"`thetadatadx.{name}` exists but is not a class or callable "
        f"(got {type(obj).__name__})."
    )


@pytest.mark.parametrize("name", PUBLIC_FUNCTIONS)
def test_function_importable(mod, name: str) -> None:
    fn = getattr(mod, name, None)
    assert fn is not None, (
        f"`from thetadatadx import {name}` would raise ImportError — "
        f"documented public function is missing from the installed wheel."
    )
    assert callable(fn), f"`thetadatadx.{name}` exists but is not callable."


def test_documented_fluent_example_runs(mod) -> None:
    """End-to-end exercise of the doc-comment example from `fluent.rs`."""
    stock = mod.Contract.stock("AAPL")
    assert stock.symbol == "AAPL"
    quote_sub = stock.quote()
    assert quote_sub.kind == "quote"
    assert quote_sub.contract is not None

    option = mod.Contract.option("SPY", expiration="20260620", strike="550", right="C")
    assert option.symbol == "SPY"


def test_version_attribute_exposed(mod) -> None:
    """PEP 396: `thetadatadx.__version__` must be a non-empty string.

    A fresh install of the wheel resolves the version through
    `importlib.metadata.version("thetadatadx")`; the source-tree
    fallback returns the in-source default. Either way, the attribute
    must exist and be a non-empty string — downstream packagers,
    `pip show`, and bug-report scripts rely on it.
    """
    assert hasattr(mod, "__version__"), (
        "`thetadatadx.__version__` is missing — PEP 396 requires every "
        "distributable Python package to expose this attribute."
    )
    version = mod.__version__
    assert isinstance(version, str), (
        f"`thetadatadx.__version__` must be a string, got {type(version).__name__}."
    )
    assert version, "`thetadatadx.__version__` must be a non-empty string."
