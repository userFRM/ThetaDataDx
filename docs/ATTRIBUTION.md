# Attribution

ThetaDataDx ships original code under the Apache License, Version 2.0.

## Third-party content bundled in the source tree

### Per-endpoint Python docstrings

Starting with v8.0.2 the endpoint docstring field in
`crates/thetadatadx/endpoint_surface.toml` carries prose lifted verbatim
from the upstream Python SDK distributed on PyPI as `thetadata`,
which is also licensed under the Apache License, Version 2.0. The
upstream project publishes the same docstrings as part of its
`client.py` module; we consume them through our TOML SSOT so the generator
can emit identical prose into every generated surface (Python sync +
`_async`, fluent builders, TypeScript, Rust, C++, Go) without drift.

The Apache-2.0 license does not require us to reproduce the notice in
every generated file; we acknowledge the source here so downstream
readers can find the original quickly. No changes are made to the
substantive text of any docstring — only mechanical reformatting (leading
whitespace trim and a blank-line separator when merging with the
DX-native `description` field).

If you notice a docstring that should be updated to match a newer
upstream release, please open an issue on
`github.com/userFRM/ThetaDataDx` referencing the endpoint name and the
upstream version.
