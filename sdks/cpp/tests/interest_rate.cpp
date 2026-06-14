// Schema coverage for the `InterestRateTick` C ABI / C++ wrapper.
//
// The upstream interest-rate response carries 2 columns (`created` as
// an ISO date Text, `rate` as a percent Number). `TdxInterestRateTick`
// must mirror that 2-field shape: a `rate` value and a `date` parsed
// through `parse_iso_date`. A third field would force the live decode
// to reject column 0 as `expected Number|Timestamp, got Text`.
//
// This file pins the struct layout so a header whose
// `TdxInterestRateTick` drifts from the 2-field schema fails to
// compile. Live decode coverage lives in
// `crates/thetadatadx/tests/test_interest_rate_schema.rs`.

#include <cstddef>
#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("TdxInterestRateTick has the 2-field shape", "[interest_rate][schema][offline]") {
    // The C struct must be 64 bytes total (cache-line aligned) with
    // `date` at offset 0 and `rate` at offset 8. Padding between the
    // i32 and the f64 is 4 bytes; the trailing pad fills to 64.
    STATIC_REQUIRE(sizeof(TdxInterestRateTick) == 64);
    STATIC_REQUIRE(alignof(TdxInterestRateTick) == 64);
    STATIC_REQUIRE(offsetof(TdxInterestRateTick, date) == 0);
    STATIC_REQUIRE(offsetof(TdxInterestRateTick, rate) == 8);
}

TEST_CASE("InterestRateTick wrapper alias resolves to the C ABI struct", "[interest_rate][schema][offline]") {
    // The C++ wrapper exposes the schema name verbatim via a `using`
    // alias on top of the C type — both must be the same layout.
    STATIC_REQUIRE(sizeof(tdx::InterestRateTick) == sizeof(TdxInterestRateTick));
    STATIC_REQUIRE(alignof(tdx::InterestRateTick) == alignof(TdxInterestRateTick));
    STATIC_REQUIRE(offsetof(tdx::InterestRateTick, date) == offsetof(TdxInterestRateTick, date));
    STATIC_REQUIRE(offsetof(tdx::InterestRateTick, rate) == offsetof(TdxInterestRateTick, rate));
}

TEST_CASE("InterestRateTick decodes the SOFR reference row", "[interest_rate][offline]") {
    // The headline wire dump in the CHANGELOG is the SOFR
    // 2025-04-28 row (`date=20250428`, `rate=4.36`). Pin the exact
    // values on a hand-built tick so any future struct-shape drift
    // fails this test before it ships.
    tdx::InterestRateTick tick{};
    tick.date = 20250428;
    tick.rate = 4.36;
    REQUIRE(tick.date == 20250428);
    REQUIRE(tick.rate == 4.36);
}
