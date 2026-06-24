// Historical tuning setters + getters on thetadatadx::Config.
//
// Offline test pinning the contract that the historical tuning setters
// and readback getters on the `thetadatadx::Config` C++ wrapper
// round-trip through the underlying C ABI.

#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.h"
#include "thetadatadx.hpp"

TEST_CASE("Config historical request_timeout_secs setter + getter round-trip",
          "[config][pool_sizing][offline]") {
    auto cfg = thetadatadx::Config::production();
    // The readback getter mirrors the Python `Config.request_timeout_secs`
    // and TypeScript `requestTimeoutSecs` surfaces, so a value set
    // through the C++ wrapper reads back through the same wrapper.
    // Production seeds the 300s default per-request deadline.
    REQUIRE(cfg.get_request_timeout_secs() == 300u);
    cfg.set_request_timeout_secs(45);
    REQUIRE(cfg.get_request_timeout_secs() == 45u);
    cfg.set_request_timeout_secs(600);
    REQUIRE(cfg.get_request_timeout_secs() == 600u);
    // 0 disables the default deadline.
    cfg.set_request_timeout_secs(0);
    REQUIRE(cfg.get_request_timeout_secs() == 0u);
}

TEST_CASE("Config historical_host / historical_port setters + getters round-trip",
          "[config][historical][offline]") {
    // The historical endpoint overrides mirror the Python `Config.historical_host` /
    // `.historical_port` advanced knobs, so a value set through the C++ wrapper
    // reads back through the same wrapper.
    auto cfg = thetadatadx::Config::production();

    // A production config has a non-empty default host.
    REQUIRE_FALSE(cfg.get_historical_host().empty());

    cfg.set_historical_host("127.0.0.1");
    REQUIRE(cfg.get_historical_host() == "127.0.0.1");

    cfg.set_historical_port(50051);
    REQUIRE(cfg.get_historical_port() == 50051u);
}

TEST_CASE("Config environment getters read back the selected clusters",
          "[config][environment][offline]") {
    // The environment readbacks mirror the Python
    // `Config.historical_environment` / `.streaming_environment` and the
    // TypeScript `historicalEnvironment` / `streamingEnvironment` getters.
    // The two channels are selected independently: the stage preset moves
    // the historical channel to staging while streaming stays on
    // production, and the dev preset moves the streaming channel to dev
    // while historical stays on production.
    REQUIRE(thetadatadx::Config::stage().get_historical_environment() == "STAGE");
    REQUIRE(thetadatadx::Config::stage().get_streaming_environment() == "PROD");
    REQUIRE(thetadatadx::Config::dev().get_historical_environment() == "PROD");
    REQUIRE(thetadatadx::Config::dev().get_streaming_environment() == "DEV");
    REQUIRE(thetadatadx::Config::production().get_historical_environment() == "PROD");
    REQUIRE(thetadatadx::Config::production().get_streaming_environment() == "PROD");
}
