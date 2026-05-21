"""thetadatadx: native Rust SDK for direct ThetaData market-data access."""

from .thetadatadx import *  # noqa: F401, F403
from . import thetadatadx as _ext

__doc__ = _ext.__doc__
if hasattr(_ext, "__all__"):
    __all__ = _ext.__all__

# Resolve the package version through `importlib.metadata` so the
# string the user reads from `thetadatadx.__version__` is the same one
# the installed wheel's `METADATA` declares — driven by `Cargo.toml`
# via the maturin build. Falling back to the in-source default keeps
# `thetadatadx.__version__` readable in editable / source-tree imports
# where the wheel metadata is absent. Wrapping the import in a
# try/except guards against the (theoretical) edge case where
# `importlib.metadata` cannot resolve the distribution (unusual stale
# install), so an import-time exception cannot break the wheel.
try:
    from importlib.metadata import PackageNotFoundError as _PackageNotFoundError
    from importlib.metadata import version as _pkg_version

    try:
        __version__ = _pkg_version("thetadatadx")
    except _PackageNotFoundError:
        __version__ = "10.0.0"
except ImportError:  # pragma: no cover - importlib.metadata is in stdlib >=3.8
    __version__ = "10.0.0"
