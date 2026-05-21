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
    REQUIRE_NOTHROW(config.set_flush_mode(0));
    REQUIRE_NOTHROW(config.set_derive_ohlcvc(true));
}

TEST_CASE("Client::connect succeeds against the production server", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    auto creds = tdx::Credentials::from_file(creds_path);
    auto config = tdx::Config::production();
    REQUIRE_NOTHROW(tdx::Client::connect(creds, config));
}
