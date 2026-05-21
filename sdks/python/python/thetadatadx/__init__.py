"""thetadatadx: native Rust SDK for direct ThetaData market-data access."""

from .thetadatadx import *  # noqa: F401, F403
from . import thetadatadx as _ext

__doc__ = _ext.__doc__
if hasattr(_ext, "__all__"):
    __all__ = _ext.__all__

# Resolve the package version through `importlib.metadata` so the
# string the user reads from `thetadatadx.__version__` is the same one
# the installed wheel's `METADATA` declares — driven by `Cargo.toml`
# via the maturin build. The "unknown" fallback applies only when
# `importlib.metadata` cannot resolve the distribution (editable /
# source-tree imports where the wheel metadata is absent, unusual
# stale installs). Using a literal sentinel here — rather than a
# hardcoded version string — keeps the staleness immediately visible
# to operators inspecting `__version__`, instead of silently lying
# with a stale number that drifted away from `Cargo.toml`.
from importlib.metadata import PackageNotFoundError as _PackageNotFoundError
from importlib.metadata import version as _pkg_version

try:
    __version__ = _pkg_version("thetadatadx")
except _PackageNotFoundError:
    __version__ = "unknown"
