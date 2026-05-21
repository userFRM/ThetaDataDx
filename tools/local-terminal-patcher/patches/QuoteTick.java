/*
 * QuoteTick.java — patched 2026-04-27
 *
 * Original location: net/thetadata/types/tick/QuoteTick.java (decompiled with CFR 0.152)
 *
 * Bug fixed: hard IllegalArgumentException on rows whose data[] length is not
 * exactly 11. Production traffic on /v3/option/history/greeks/* for 2022-era
 * options has been observed returning rows with data.length == 6, which the
 * original constructor rejects without diagnostic context. The whole HTTP
 * response then fails with a generic "Internal server error" and no row-level
 * information for the server team to triage.
 *
 * Two changes:
 *
 * 1. Lenient upcast for the legacy 6-field schema. The pre-extension NBBO
 *    quote layout was [ms_of_day, bid_size, bid, ask_size, ask, date].
 *    The post-extension layout is the current 11-field shape. When a 6-field
 *    row arrives, we upcast it to 11 fields by zero-filling the missing
 *    columns: bid_exg, bid_condition, ask_exg, ask_condition, price_type.
 *    Zeroes are the canonical "unknown / default" sentinel for those fields
 *    everywhere else in the codebase, so downstream consumers behave
 *    consistently.
 *
 * 2. Diagnostic on failure. When the row really is malformed (any length
 *    other than 6 or 11), the thrown exception now includes the actual
 *    array contents so the server team can identify the upstream record.
 *    This replaces the original "expecting 11, got N" with a message that
 *    is actionable.
 *
 * The cosmetic surface of the class — field accessors, msOfDay(), bid(),
 * ask(), getDateTimeMessage(), and so on — is unchanged. The clone() method
 * is unchanged. Wire compatibility with downstream code is preserved.
 *
 * To apply: drop this file in place of the original at
 * net/thetadata/types/tick/QuoteTick.java and rebuild the terminal.
 */
package net.thetadata.types.tick;

import java.time.ZoneId;
import java.util.Arrays;
import net.thetadata.generated.Price;
import net.thetadata.generated.TimeZone;
import net.thetadata.generated.ZonedDateTime;
import net.thetadata.types.tick.Tick;
import net.thetadata.utils.TimeUtils;

public class QuoteTick extends Tick {

    /** Length of the post-extension NBBO quote schema (current). */
    private static final int FIELD_COUNT_V3 = 11;

    /** Length of the pre-extension NBBO quote schema observed on 2022-era
     *  history rows. Layout: [ms_of_day, bid_size, bid, ask_size, ask, date]. */
    private static final int FIELD_COUNT_V2_LEGACY = 6;

    public QuoteTick(Tick t2) {
        super(QuoteTick.normalizeData(t2.data()));
        // After normalize, data.length is guaranteed 11. Guard anyway so any
        // future regression in normalize() throws here instead of silently
        // producing a misshaped tick.
        if (this.data.length != FIELD_COUNT_V3) {
            throw new IllegalArgumentException(
                "QuoteTick.normalize() returned unexpected length=" + this.data.length
                + ", expected " + FIELD_COUNT_V3
                + ", source data=" + Arrays.toString(t2.data()));
        }
    }

    /**
     * Upcast legacy 6-field rows to the current 11-field shape. Throws with
     * a diagnostic payload when the input length is neither.
     *
     * Legacy layout (length 6): [ms_of_day, bid_size, bid, ask_size, ask, date]
     * Current layout (length 11): [ms_of_day, bid_size, bid_exg, bid, bid_cond,
     *                              ask_size, ask_exg, ask, ask_cond, price_type, date]
     *
     * The upcast zero-fills bid_exg, bid_cond, ask_exg, ask_cond, price_type.
     * Zero is the canonical "unknown" value for these fields elsewhere in the
     * tick stack, which keeps downstream behaviour consistent.
     */
    private static int[] normalizeData(int[] data) {
        if (data == null) {
            throw new IllegalArgumentException("QuoteTick: data array is null");
        }
        if (data.length == FIELD_COUNT_V3) {
            return data;
        }
        if (data.length == FIELD_COUNT_V2_LEGACY) {
            int[] upcast = new int[FIELD_COUNT_V3];
            // Source: [ms, bid_size, bid, ask_size, ask, date]
            // Target: [ms, bid_size, bid_exg, bid, bid_cond,
            //          ask_size, ask_exg, ask, ask_cond, price_type, date]
            upcast[0]  = data[0];      // ms_of_day
            upcast[1]  = data[1];      // bid_size
            upcast[2]  = 0;            // bid_exg (unknown in legacy schema)
            upcast[3]  = data[2];      // bid
            upcast[4]  = 0;            // bid_condition (unknown)
            upcast[5]  = data[3];      // ask_size
            upcast[6]  = 0;            // ask_exg (unknown)
            upcast[7]  = data[4];      // ask
            upcast[8]  = 0;            // ask_condition (unknown)
            upcast[9]  = 0;            // price_type (default = 0; same as
                                       //             pre-extension implicit type)
            upcast[10] = data[5];      // date
            return upcast;
        }
        // Genuine corruption — surface a diagnostic payload so the server
        // team can identify the upstream record.
        throw new IllegalArgumentException(
            "QuoteTick: malformed row, expected length " + FIELD_COUNT_V3
            + " (current schema) or " + FIELD_COUNT_V2_LEGACY + " (legacy 2022-era schema), "
            + "got length=" + data.length
            + ", contents=" + Arrays.toString(data));
    }

    public int msOfDay() {
        return this.data[0];
    }

    public int bidSize() {
        return this.data[1];
    }

    public int bidExg() {
        return this.data[2];
    }

    public int bid() {
        return this.data[3];
    }

    public Price bidPrice() {
        return Price.newBuilder().setValue(this.bid()).setType(this.priceType()).build();
    }

    public int bidCondition() {
        return this.data[4];
    }

    public int askSize() {
        return this.data[5];
    }

    public int askExg() {
        return this.data[6];
    }

    public int ask() {
        return this.data[7];
    }

    public Price askPrice() {
        return Price.newBuilder().setValue(this.ask()).setType(this.priceType()).build();
    }

    public int askCondition() {
        return this.data[8];
    }

    public int priceType() {
        return this.data[9];
    }

    public int date() {
        return this.data[10];
    }

    public int[] getDataArray() {
        return this.data;
    }

    public int midPointValue() {
        return this.bid() / 2 + this.ask() / 2 + (this.bid() % 2 + this.ask() % 2) / 2;
    }

    public Price midPoint() {
        return Price.newBuilder().setValue(this.midPointValue()).setType(this.priceType()).build();
    }

    @Override
    public QuoteTick clone() {
        return new QuoteTick(super.clone());
    }

    public ZonedDateTime getDateTimeMessage() {
        java.time.ZonedDateTime timestamp = TimeUtils.getZonedDateTime(
            this.date(), this.msOfDay(), ZoneId.of("America/New_York"));
        return ZonedDateTime.newBuilder()
            .setEpochMs(timestamp.toInstant().toEpochMilli())
            .setZone(TimeZone.NEW_YORK)
            .build();
    }
}
