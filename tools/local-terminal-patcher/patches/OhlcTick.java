/*
 * OhlcTick.java — patched 2026-04-27
 *
 * Original location: net/thetadata/types/tick/OhlcTick.java (decompiled with CFR 0.152)
 *
 * Bug fixed: cosmetic. Original line 18 checks `data.length != 9` but the
 * thrown IllegalArgumentException reads "OHLC tick data length must be 10".
 * The check is correct (the layout is 9 ints); only the message is wrong.
 * Diagnostic value-add: include the actual length and the contents on failure
 * so the server team can identify a malformed row rather than guess.
 *
 * To apply: drop this file in place of the original at
 * net/thetadata/types/tick/OhlcTick.java and rebuild the terminal.
 */
package net.thetadata.types.tick;

import java.time.ZoneId;
import java.util.Arrays;
import net.thetadata.generated.Price;
import net.thetadata.generated.TimeZone;
import net.thetadata.generated.ZonedDateTime;
import net.thetadata.types.tick.Tick;
import net.thetadata.utils.TimeUtils;

public class OhlcTick {
    private static final int FIELD_COUNT = 9;
    private final int[] data;

    public OhlcTick(Tick tick) {
        this.data = tick.data();
        if (this.data.length != FIELD_COUNT) {
            throw new IllegalArgumentException(
                "OhlcTick: malformed row, expected length " + FIELD_COUNT
                + ", got length=" + this.data.length
                + ", contents=" + Arrays.toString(this.data));
        }
    }

    public int msOfDay() {
        return this.data[0];
    }

    public int open() {
        return this.data[1];
    }

    public Price openPrice() {
        return Price.newBuilder().setValue(this.open()).setType(this.priceType()).build();
    }

    public int high() {
        return this.data[2];
    }

    public Price highPrice() {
        return Price.newBuilder().setValue(this.high()).setType(this.priceType()).build();
    }

    public int low() {
        return this.data[3];
    }

    public Price lowPrice() {
        return Price.newBuilder().setValue(this.low()).setType(this.priceType()).build();
    }

    public int close() {
        return this.data[4];
    }

    public Price closePrice() {
        return Price.newBuilder().setValue(this.close()).setType(this.priceType()).build();
    }

    public int volume() {
        return this.data[5];
    }

    public int count() {
        return this.data[6];
    }

    public int priceType() {
        return this.data[7];
    }

    public int date() {
        return this.data[8];
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
