---
title: 2020+ schema variant investigation (issue #577)
---

# Wire-level schema variant investigation — issue #577

Follow-up investigation for the post-Feb-2020 `option_history_quote`
h2 cascade reported in [#577](https://github.com/userFRM/ThetaDataDx/issues/577).
Pinned here so the next contributor does not repeat the search.

## Investigation method

1. Read every `*.java` file under
   `theta-terminal-re/actual-terminal/net/thetadata/types/tick/` from the
   freshly decompiled March 2026 ThetaTerminal jar.
2. Read the patched terminal source in
   `theta-terminal-re/patches/QuoteTick.java`.
3. Mapped each strict-length check to the corresponding Rust decode
   path in `crates/thetadatadx/src/mdds/decode/`.

## All strict-length checks on the Java side

Every tick class in the upstream jar validates `data.length` against a
single integer and throws `IllegalArgumentException` on any other length.

| Tick type             | Required length | Wire concern         |
|-----------------------|-----------------|----------------------|
| `QuoteTick`           | 11              | **legacy 6-field**   |
| `MarketValueTick`     | 11 (via super)  | inherits QuoteTick   |
| `TradeQuoteTick`      | 25              | inherits 11-quote tail |
| `TradeTick`           | 16              | (no legacy)          |
| `SnapshotTradeTick`   | 7               | (no legacy)          |
| `OhlcTick`            | 9               | (no legacy)          |
| `EodTick`             | 18              | (no legacy)          |
| `OpenInterestTick`    | 3               | (no legacy)          |

The patched terminal source contains exactly **two** patches:

- `QuoteTick.java` -- `normalizeData()` upcasts 6-field rows to the
  11-field shape by zero-filling the absent exchange / condition /
  price_type columns.
- `OhlcTick.java` -- cosmetic typo fix on the error message text.

**There is no intermediate 8-field or 9-column variant** anywhere in
the source. The hypothesis floated in issue #577 (an intermediate
extension layout introduced between the 6-field legacy and 11-field
current shapes) is not supported by source evidence.

## How the h2 cascade reaches the SDK

1. Server-side storage returns a `Tick` with `data.length == 6` for a
   pre-2023 NBBO row.
2. The unpatched MDDS `QuoteTick(Tick t)` constructor runs
   `if (this.data.length != 11) throw ...;` and aborts the request
   handler.
3. The Java gRPC handler swallows the exception without writing a
   `grpc-status` trailer -- the h2 stream closes with no error frame.
4. The SDK observes
   `Error::Transport { kind: ConnectionClosed, .. }` mid-response.

The Rust decoder's behaviour on the same row is irrelevant: the bytes
never reach it.

## Why post-Feb-2020 days cascade but earlier days work

The two NBBO storage tiers transitioned at different cutover dates per
symbol. The empirical evidence in #577 shows:

- 2019-05-24 to 2020-02-24: storage row is already 11-field for QQQ --
  no cascade.
- 2020-02-25 onward: storage row is 6-field for QQQ daily-expiry
  contracts -- cascades on most days.
- Post-2022: nearly every day cascades.

This is a **vendor-side storage-cutover date**, not a SDK schema issue.
The 6-field shape itself is unchanged; only the date range over which
it is encountered has expanded as PR #573's smoke-test runtime exposed
more storage tiers.

## Rust decoder coverage today

The build-time generator in `crates/thetadatadx/build_support/ticks/parser.rs`
emits `find_header(name)` calls for every column declared in
`tick_schema.toml`. Columns not in the `required` list resolve through
`opt_number(idx_opt)` which returns `0` when `idx_opt == None`. The
`QuoteTick` schema declares only `["ms_of_day", "bid", "ask"]` as
required; the four exchange / condition columns are optional and
zero-fill when absent.

This means the Rust decoder **already handles the 6-of-11 column row
shape correctly when the bytes reach it** (via the REST transport),
matching the patched Terminal's `normalizeData()` upcast verbatim.

Pinning tests on main from #573:

- `quote_tick_decodes_legacy_six_field_shape_with_zero_fill`
- `quote_tick_decodes_current_eleven_field_shape_unchanged`

Both live in `crates/thetadatadx/src/mdds/decode/tests.rs`.

## Conclusion

There is no second wire variant. The remaining #577 work is
purely **transport-layer** -- making the SDK survive the
unpatched-server cascade and recover automatically. The follow-up
work this branch lands:

1. **Auto-recycle the gRPC channel on `ConnectionClosed`.** Today the
   dead channel stays in the pool and every subsequent RPC on it
   returns the same error. The fix detects connection-level death,
   tears down the broken channel, opens a fresh one in its place, and
   retries the request once. See `crates/thetadatadx/src/grpc/pool.rs`.

2. **Mirror the `*_with_fallback` shim onto the streaming
   builders.** PR #573 wired fallback only into the buffered `.await`
   path; the streaming `.stream(handler)` path observed the same
   cascade on multi-million-row responses with no recovery. The
   streaming-flavour shim re-issues the call over REST when gRPC
   raises `ConnectionClosed` mid-stream.

3. **Extend fallback shims to the greeks endpoints.** The REST module
   already exposes `option_history_greeks_implied_volatility` and
   `option_history_greeks_first_order` builders; this branch adds the
   matching `*_with_fallback` shims on `ThetaDataDxClient` so users
   do not have to call REST manually for those endpoints.

## Where strict checks would live in Rust (none today)

The Rust side is already lenient by construction. The generator emits
`opt_number(idx_opt)` for every non-required column; nothing
length-checks the row array. Adding new tick types in the future
should keep that pattern -- declare load-bearing columns in `required`,
leave everything else optional, and the generator handles the rest.
