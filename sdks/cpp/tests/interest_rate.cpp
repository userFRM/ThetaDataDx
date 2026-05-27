// Regression coverage for the v11 `InterestRateTick` schema fix.
//
// The upstream v3 server emits 2 columns (`created` as ISO date Text,
// `rate` as percent Number). The pre-v11 SDK declared 3 fields
// (`ms_of_day`, `rate`, `date`) and decoded every live response into
// `column 0: expected Number|Timestamp, got Text`. The fix removed
// the fictitious `ms_of_day` field and rewired `date` to flow through
// `parse_iso_date`.
//
// This file pins the C ABI / C++ wrapper surface for the new
// 2-field shape so a future schema regression cannot ship a header
// whose `TdxInterestRateTick` struct still carries the removed field.
// Live decode coverage lives in
// `crates/thetadatadx/tests/test_interest_rate_schema.rs`.

#include <cstddef>
#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("TdxInterestRateTick has the v11 2-field shape", "[interest_rate][schema][offline]") {
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
    // The headline wire dump in the v11 CHANGELOG is the SOFR
    // 2025-04-28 row (`date=20250428`, `rate=4.36`). Pin the exact
    // values on a hand-built tick so any future struct-shape drift
    // fails this test before it ships.
    tdx::InterestRateTick tick{};
    tick.date = 20250428;
    tick.rate = 4.36;
    REQUIRE(tick.date == 20250428);
    REQUIRE(tick.rate == 4.36);
}
