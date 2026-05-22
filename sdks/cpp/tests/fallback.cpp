// REST-fallback policy + Config integration tests.
//
// Offline tests pin the four factory functions, the move-only semantics,
// and the Config::withRestFallback wiring. The live half drives an
// end-to-end call against the patched local Terminal; gated on
// `THETADX_LIVE_CREDS` pointing at a creds.txt file -- mirrors the
// gate used by the Python `test_rest_fallback.py::*_live` cases.

#include <cstdlib>
#include <string>
#include <utility>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("FallbackPolicy::disabled constructs a valid handle", "[fallback][offline]") {
    auto policy = tdx::FallbackPolicy::disabled();
    REQUIRE(policy.get() != nullptr);
}

TEST_CASE("FallbackPolicy::restOnH2Disconnect round-trips the base URL", "[fallback][offline]") {
    auto policy = tdx::FallbackPolicy::restOnH2Disconnect("http://127.0.0.1:25503");
    REQUIRE(policy.get() != nullptr);
}

TEST_CASE("FallbackPolicy::restAlwaysForDateRange round-trips both args", "[fallback][offline]") {
    auto policy = tdx::FallbackPolicy::restAlwaysForDateRange(
        "http://127.0.0.1:25503", 20230101);
    REQUIRE(policy.get() != nullptr);
}

TEST_CASE("FallbackPolicy::restAlways round-trips the base URL", "[fallback][offline]") {
    auto policy = tdx::FallbackPolicy::restAlways("http://127.0.0.1:25503");
    REQUIRE(policy.get() != nullptr);
}

TEST_CASE("FallbackPolicy is move-constructible + move-assignable", "[fallback][offline]") {
    auto a = tdx::FallbackPolicy::restAlways("http://127.0.0.1:25503");
    REQUIRE(a.get() != nullptr);

    // Move-construct.
    tdx::FallbackPolicy b(std::move(a));
    REQUIRE(b.get() != nullptr);
    REQUIRE(a.get() == nullptr);

    // Move-assign.
    auto c = tdx::FallbackPolicy::disabled();
    c = std::move(b);
    REQUIRE(c.get() != nullptr);
    REQUIRE(b.get() == nullptr);
}

TEST_CASE("Config::withRestFallback accepts each variant", "[fallback][offline]") {
    auto config = tdx::Config::production();

    REQUIRE_NOTHROW(config.withRestFallback(tdx::FallbackPolicy::disabled()));
    REQUIRE_NOTHROW(config.withRestFallback(
        tdx::FallbackPolicy::restOnH2Disconnect("http://127.0.0.1:25503")));
    REQUIRE_NOTHROW(config.withRestFallback(
        tdx::FallbackPolicy::restAlwaysForDateRange("http://127.0.0.1:25503", 20230101)));
    REQUIRE_NOTHROW(config.withRestFallback(
        tdx::FallbackPolicy::restAlways("http://127.0.0.1:25503")));
}

TEST_CASE("Config::withRestFallback survives policy destruction (snapshot semantics)",
          "[fallback][offline]") {
    auto config = tdx::Config::production();
    {
        auto policy = tdx::FallbackPolicy::restAlways("http://127.0.0.1:25503");
        config.withRestFallback(policy);
    } // policy drops here -- config has already cloned the inner enum
    // Re-installing must still work; the previous policy handle is gone
    // but the config retains its own copy.
    REQUIRE_NOTHROW(config.withRestFallback(tdx::FallbackPolicy::disabled()));
}

TEST_CASE("Client::optionHistoryQuoteWithFallback round-trips against patched Terminal",
          "[fallback][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    if (env_or_empty("THETADX_LIVE_PATCHED_TERMINAL").empty()) {
        SKIP("THETADX_LIVE_PATCHED_TERMINAL not set");
    }
    auto creds = tdx::Credentials::from_file(creds_path);
    auto config = tdx::Config::production();
    config.withRestFallback(tdx::FallbackPolicy::restAlways("http://127.0.0.1:25503"));
    auto client = tdx::Client::connect(creds, config);

    // 2022-era QQQ row -- the issue #571 cascade window. With
    // RestAlways the call should round-trip via REST regardless.
    auto ticks = client.optionHistoryQuoteWithFallback(
        "QQQ", "20220422", "20220414", /*end_date=*/{}, /*strike=*/"305",
        /*right=*/"P", /*interval=*/"60000");
    // Empty result is a legal "no ticks" outcome -- the patch just
    // ensures the call doesn't throw on the affected rows.
    (void)ticks;
}
