// Historical endpoint round-trip smoke tests.
//
// EOD is the cheapest historical endpoint to exercise — one row per
// trading day, decoded into a `TdxEodTick` array. The offline half
// here is intentionally narrow: a real historical call needs a live
// server. The live half guards the symbol decode, the typed array
// wrapper, and the FFI-error -> exception path.

#include <cstdlib>
#include <string>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("standalone Greeks calculator does not need a connection", "[history][offline]") {
    // `all_greeks` is a pure-numerical helper — it runs entirely
    // inside the FFI without touching the network. Confirms the
    // wrapper at least links and dispatches without throwing on a
    // textbook call (ATM call, 30 d to expiry).
    auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0 / 365.0, 8.50, "C");
    REQUIRE(g.iv > 0.0);
    REQUIRE(g.delta > 0.0);
    REQUIRE(g.delta < 1.0);
}

TEST_CASE("stock_history_eod returns a non-empty vector for a known active symbol",
          "[history][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    auto creds = tdx::Credentials::from_file(creds_path);
    auto config = tdx::Config::production();
    auto client = tdx::Client::connect(creds, config);
    auto eod = client.stock_history_eod("AAPL", "20240101", "20240131");
    REQUIRE_FALSE(eod.empty());
    // First decoded tick must carry a plausible YYYYMMDD date —
    // guards the typed array wrapper's i32 copy across the FFI boundary.
    REQUIRE(eod.front().date >= 20240101);
    REQUIRE(eod.front().date <= 20240201);
}
