# Thin task runner over the repository gate suite and release/dev scripts.
# Each recipe just calls the matching script; the gate suite has a single
# dispatcher at scripts/ci.py.

# Run the whole gate suite (mirrors the CI invariant jobs).
ci:
    python3 scripts/ci.py all

# Run one named gate, e.g. `just check binding_parity`. `just check list`
# prints the available gate names.
check name:
    python3 scripts/ci.py {{name}}

# Bump every user-visible version pin in lockstep.
bump-version version:
    python3 scripts/release/bump_version.py {{version}}

# Run the local live smoke checks (needs creds.txt in the repo root).
smoke creds="creds.txt":
    python3 scripts/dev/live_smoke.py {{creds}}
    python3 scripts/dev/fpss_smoke.py {{creds}}
