#!/usr/bin/env bash
# Repo-root entry point for the generator byte-identical check.

set -euo pipefail

exec bash crates/thetadatadx/tests/regen_byte_identical.sh "$@"
