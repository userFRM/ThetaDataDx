// C++ SDK lifecycle smoke tests.
//
// Confirms the connect / disconnect path against the production
// server (live-only) and the type-level surface that does not need
// credentials (offline). The live half is gated on
// `THETADX_LIVE_CREDS` pointing at a `creds.txt` file with the
// account email on line 1 and the password on line 2.

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

TEST_CASE("Config::production builds without network access", "[lifecycle][offline]") {
    auto config = tdx::Config::production();
    REQUIRE(config.get() != nullptr);
}

TEST_CASE("Config setters do not throw on a fresh config handle", "[lifecycle][offline]") {
    auto config = tdx::Config::production();
    REQUIRE_NOTHROW(config.set_reconnect_policy(0));
    REQUIRE_NOTHROW(config.set_reconnect_max_attempts(5));
    REQUIRE_NOTHROW(config.set_reconnect_max_rate_limited_attempts(50));
    REQUIRE_NOTHROW(config.set_reconnect_stable_window_secs(120));
    REQUIRE_NOTHROW(config.set_flush_mode(0));
    REQUIRE_NOTHROW(config.set_derive_ohlcvc(true));
}

TEST_CASE("Config flush_mode / derive_ohlcvc getters round-trip", "[lifecycle][offline]") {
    // The readback getters mirror the Python `Config.flush_mode` /
    // `.derive_ohlcvc` and TypeScript `flushMode` / `deriveOhlcvc`
    // surfaces, so a value set through the C++ wrapper reads back
    // through the same wrapper.
    auto config = tdx::Config::production();

    config.set_flush_mode(1);
    REQUIRE(config.flush_mode() == 1);
    config.set_flush_mode(0);
    REQUIRE(config.flush_mode() == 0);

    config.set_derive_ohlcvc(false);
    REQUIRE(config.derive_ohlcvc() == false);
    config.set_derive_ohlcvc(true);
    REQUIRE(config.derive_ohlcvc() == true);
}

TEST_CASE("MddsClient::connect succeeds against the production server", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    auto creds = tdx::Credentials::from_file(creds_path);
    auto config = tdx::Config::production();
    REQUIRE_NOTHROW(tdx::MddsClient::connect(creds, config));
}
