# Codex Summary

| Finding | Status | Commit | Notes |
|---|---|---|---|
| C1 | fixed | bb677e94 | Confirmed napi-rs holds the `aborted` read lock across blocking TSFN calls; release now wakes before taking the write lock. Checks: `cargo test --manifest-path thetadatadx-ts/Cargo.toml teardown_deadlock_tests --lib`; `cargo build --manifest-path thetadatadx-ts/Cargo.toml`; `cargo clippy --manifest-path thetadatadx-ts/Cargo.toml --all-targets -- -D warnings`. |
