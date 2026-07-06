// Market-data endpoint round-trip smoke tests.
//
// EOD is the cheapest market-data endpoint to exercise — one row per
// trading day, decoded into a `ThetaDataDxEodTick` array. A real historical
// call needs a live server; the live half guards the symbol decode, the
// typed array wrapper, and the FFI-error -> exception path.

#include <cstdlib>
#include <string>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("stock_history_eod returns a non-empty vector for a known active symbol",
          "[history][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::MarketDataClient::connect(creds, config);
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240131");
    REQUIRE_FALSE(eod.empty());
    // First decoded tick must carry a plausible YYYYMMDD date —
    // guards the typed array wrapper's i32 copy across the FFI boundary.
    REQUIRE(eod.front().date >= 20240101);
    REQUIRE(eod.front().date <= 20240201);
}
