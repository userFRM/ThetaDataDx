// C++ SDK lifecycle smoke tests.
//
// Confirms the connect / disconnect path against the production
// server (live-only) and the type-level surface that does not need
// credentials (offline). The live half is gated on
// `THETADATADX_LIVE_CREDS` pointing at a `creds.txt` file with the
// account email on line 1 and the password on line 2.

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

TEST_CASE("Config::production builds without network access", "[lifecycle][offline]") {
    auto config = thetadatadx::Config::production();
    REQUIRE(config.get() != nullptr);
}

TEST_CASE("Config setters do not throw on a fresh config handle", "[lifecycle][offline]") {
    auto config = thetadatadx::Config::production();
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
    auto config = thetadatadx::Config::production();

    config.set_flush_mode(1);
    REQUIRE(config.get_flush_mode() == 1);
    config.set_flush_mode(0);
    REQUIRE(config.get_flush_mode() == 0);

    config.set_derive_ohlcvc(false);
    REQUIRE(config.get_derive_ohlcvc() == false);
    config.set_derive_ohlcvc(true);
    REQUIRE(config.get_derive_ohlcvc() == true);
}

TEST_CASE("Config wait_strategy / tuning / consumer_cpu round-trip", "[lifecycle][offline]") {
    // Mirrors the Python `Config.wait_strategy` / `.consumer_cpu` and
    // TypeScript `waitStrategy` / `consumerCpu` surfaces: a value set
    // through the C++ wrapper reads back through the same wrapper.
    auto config = thetadatadx::Config::production();

    // Default preset is LowLatency (preserves the historical behaviour).
    REQUIRE(config.get_wait_strategy() == THETADATADX_WAIT_LOW_LATENCY);

    for (int mode : {THETADATADX_WAIT_LOW_LATENCY, THETADATADX_WAIT_BALANCED,
                     THETADATADX_WAIT_EFFICIENT, THETADATADX_WAIT_BUSY_SPIN}) {
        config.set_wait_strategy(mode);
        REQUIRE(config.get_wait_strategy() == mode);
    }

    config.set_wait_spin_iters(16);
    REQUIRE(config.get_wait_spin_iters() == 16);
    config.set_wait_yield_iters(2);
    REQUIRE(config.get_wait_yield_iters() == 2);
    config.set_wait_park_us(200);
    REQUIRE(config.get_wait_park_us() == 200);

    // Default consumer cpu is unpinned (the negative sentinel).
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
    config.set_consumer_cpu(3);
    REQUIRE(config.get_consumer_cpu() == 3);
    config.set_consumer_cpu(THETADATADX_CONSUMER_CPU_UNPINNED);
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
}

TEST_CASE("HistoricalClient::connect succeeds against the production server", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    REQUIRE_NOTHROW(thetadatadx::HistoricalClient::connect(creds, config));
}
