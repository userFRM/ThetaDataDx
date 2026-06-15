// MDDS pool-sizing setter + getter on thetadatadx::Config.
//
// Offline test pinning the contract that `set_concurrent_requests` and
// the `get_concurrent_requests()` readback getter on the `thetadatadx::Config`
// C++ wrapper round-trip through the underlying C ABI.

#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("Config concurrent_requests setter + getter round-trip", "[config][pool_sizing][offline]") {
    auto cfg = thetadatadx::Config::production();
    // The readback getter mirrors the Python `Config.concurrent_requests`
    // and TypeScript `concurrentRequests` surfaces, so a value set
    // through the C++ wrapper reads back through the same wrapper.
    cfg.set_concurrent_requests(0);
    REQUIRE(cfg.get_concurrent_requests() == 0u);
    cfg.set_concurrent_requests(1);
    REQUIRE(cfg.get_concurrent_requests() == 1u);
    cfg.set_concurrent_requests(8);
    REQUIRE(cfg.get_concurrent_requests() == 8u);
    cfg.set_concurrent_requests(32);
    REQUIRE(cfg.get_concurrent_requests() == 32u);
}

TEST_CASE("Config mdds_host / mdds_port setters + getters round-trip",
          "[config][mdds][offline]") {
    // The MDDS endpoint overrides mirror the Python `Config.mdds_host` /
    // `.mdds_port` advanced knobs, so a value set through the C++ wrapper
    // reads back through the same wrapper.
    auto cfg = thetadatadx::Config::production();

    // A production config has a non-empty default host.
    REQUIRE_FALSE(cfg.get_mdds_host().empty());

    cfg.set_mdds_host("127.0.0.1");
    REQUIRE(cfg.get_mdds_host() == "127.0.0.1");

    cfg.set_mdds_port(50051);
    REQUIRE(cfg.get_mdds_port() == 50051u);
}
