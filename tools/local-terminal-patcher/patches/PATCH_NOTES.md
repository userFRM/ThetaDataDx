# ThetaTerminal patches — root-cause analysis + fixes

These patches address the recurring `Wrong number of data fields, expecting 11, got 6` error reported in `#api-help` on April 26-27, 2026, on `/v3/option/history/greeks/implied_volatility` and `/v3/option/history/greeks/first_order` queries against 2022-era options data (AMC, ORCL, SPXW observed).

## Where the error fires

**File:** `net/thetadata/types/tick/QuoteTick.java`

**Original constructor** (decompiled via CFR 0.152):

```java
public QuoteTick(Tick t2) {
    super(t2.data());
    if (this.data.length != 11) {
        throw new IllegalArgumentException(
            "Wrong number of data fields, expecting 11, got " + this.data.length);
    }
}
```

The constructor accepts a `Tick` whose `data()` returns an `int[]`. It validates that the array is exactly 11 elements long and throws `IllegalArgumentException` otherwise. When this exception bubbles up through the request handler, the server returns a generic 500 with no row-level diagnostic.

## Why `got 6` is happening

The current 11-field layout for an NBBO quote tick is:

```
data[0]  ms_of_day
data[1]  bid_size
data[2]  bid_exg
data[3]  bid
data[4]  bid_condition
data[5]  ask_size
data[6]  ask_exg
data[7]  ask
data[8]  ask_condition
data[9]  price_type
data[10] date
```

The 6-field rows arriving from 2022 historical match the pre-extension legacy layout:

```
data[0]  ms_of_day
data[1]  bid_size
data[2]  bid
data[3]  ask_size
data[4]  ask
data[5]  date
```

This is the NBBO quote shape from before the schema was extended to carry exchange codes, conditions, and price-type discriminators. Pre-2023 historical rows on some symbols still live in this format on the storage backend, and the Greeks endpoints — which reach back to those rows to attach the underlying NBBO at each interval — surface them up to the terminal unchanged.

The terminal's QuoteTick constructor was not written to handle the legacy shape, so it throws on every hit. This matches the symptom set reported by users: it is reproducible only on 2022-era data (`hossy`'s observation), it affects multiple symbols (AMC, ORCL, SPXW), and the same query sometimes succeeds (when no 2022 row needs to be referenced) and sometimes fails (when one does).

## The fix

`patches/QuoteTick.java` in this directory contains a drop-in replacement that:

1. **Upcasts legacy 6-field rows to the current 11-field shape** by zero-filling the absent columns (bid_exg, bid_condition, ask_exg, ask_condition, price_type). Zero is the canonical "unknown" sentinel for these fields in the rest of the tick stack, so downstream consumers behave consistently. The mapping is documented in the patched constructor's `normalizeData()` helper.

2. **Throws a diagnostic exception on genuine corruption** (any length other than 6 or 11). The new message includes the actual length and the contents of the array, which gives the server team something they can grep for in the storage tier.

The cosmetic surface of the class is unchanged: every accessor (`msOfDay()`, `bid()`, `bidExg()`, `priceType()`, etc.) returns the same value when a current-shape row arrives, and zero for the legacy fields when a pre-extension row arrives. `clone()`, `getDateTimeMessage()`, `midPoint()` are byte-for-byte identical.

## Bonus fix — OhlcTick

`patches/OhlcTick.java` fixes a cosmetic typo in the same package. The original throws `IllegalArgumentException("OHLC tick data length must be 10")` when the array length is not 9. The check is correct (the OHLC layout is 9 ints); only the message text was wrong. The patched version throws a diagnostic message with the actual length and contents.

## Server-side fix to consider in parallel

The terminal patch keeps clients running, but the underlying issue is upstream. Two options for the storage layer:

- **Migrate** the legacy 6-field rows in place to the 11-field shape by zero-filling on read or on a one-shot rewrite. This makes the upcast unnecessary on the wire and avoids any future client having to know about the legacy schema.
- **Tag** rows with a schema-version byte and let the wire layer signal which layout is in use. The terminal then dispatches to the correct decoder rather than guessing from length.

If neither happens, the patched terminal continues to absorb the legacy rows correctly; the ratio of zero-filled fields will simply track the proportion of pre-extension data being touched.

## How to apply

1. Copy `patches/QuoteTick.java` over `net/thetadata/types/tick/QuoteTick.java` in the terminal source tree.
2. Copy `patches/OhlcTick.java` over `net/thetadata/types/tick/OhlcTick.java`.
3. Rebuild the terminal jar (`mvn package` or the existing build pipeline).
4. Smoke-test against the failing reproducers from the support tickets:

```
http://127.0.0.1:25503/v3/option/history/greeks/implied_volatility?symbol=AMC&expiration=20220414&strike=27.000&right=call&start_date=20220228&end_date=20220320&interval=1h&format=html
http://127.0.0.1:25503/v3/option/history/greeks/first_order?symbol=ORCL&expiration=20220225&date=20220225&interval=1m&format=json
```

Both should now return data instead of a 500. Rows that previously triggered the exception will arrive with zero-filled exchange / condition / price_type fields.

5. If any genuinely malformed row remains in the storage tier (length other than 6 or 11), the new diagnostic exception will identify it by its actual contents, which lets the server team run a targeted repair.

## Files in this patch set

- `patches/QuoteTick.java` — drop-in replacement for the affected file
- `patches/OhlcTick.java` — bonus typo fix on the OHLC tick
- `patches/PATCH_NOTES.md` — this document

The original decompiled files are untouched at `src-java/net/thetadata/types/tick/`.
