# Codex Summary

| Finding | Status | Commit | Notes |
|---|---|---|---|
| C1 | fixed | bb677e94 | Confirmed napi-rs holds the `aborted` read lock across blocking TSFN calls; release now wakes before taking the write lock. Checks: `cargo test --manifest-path thetadatadx-ts/Cargo.toml teardown_deadlock_tests --lib`; `cargo build --manifest-path thetadatadx-ts/Cargo.toml`; `cargo clippy --manifest-path thetadatadx-ts/Cargo.toml --all-targets -- -D warnings`. |
| H1 | fixed | 97fa2280 | Confirmed decoded in-session `Disconnected { reason }` was published but did not break the inner loop. The read loop now carries reconnectable decoded reasons into the reconnect classifier. Checks: `cargo test --manifest-path thetadatadx-rs/Cargo.toml decoded_disconnect_reason_reaches_reconnect_classifier --lib`; `cargo build --manifest-path thetadatadx-rs/Cargo.toml`; `cargo clippy --manifest-path thetadatadx-rs/Cargo.toml --all-targets --features __internal -- -D warnings`. |
