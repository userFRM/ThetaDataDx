"""thetadatadx: native Rust SDK for direct ThetaData market-data access."""

from .thetadatadx import *  # noqa: F401, F403
from . import thetadatadx as _ext

__doc__ = _ext.__doc__
if hasattr(_ext, "__all__"):
    __all__ = _ext.__all__
