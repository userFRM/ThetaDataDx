// MDDS pool-sizing setter + getter on tdx::Config.
//
// Offline test pinning the contract that `set_concurrent_requests` and
// the `concurrent_requests()` readback getter on the `tdx::Config` C++
// wrapper round-trip through the underlying C ABI.

#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("Config concurrent_requests setter + getter round-trip", "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    // The readback getter mirrors the Python `Config.concurrent_requests`
    // and TypeScript `concurrentRequests` surfaces, so a value set
    // through the C++ wrapper reads back through the same wrapper.
    cfg.set_concurrent_requests(0);
    REQUIRE(cfg.concurrent_requests() == 0u);
    cfg.set_concurrent_requests(1);
    REQUIRE(cfg.concurrent_requests() == 1u);
    cfg.set_concurrent_requests(8);
    REQUIRE(cfg.concurrent_requests() == 8u);
    cfg.set_concurrent_requests(32);
    REQUIRE(cfg.concurrent_requests() == 32u);
}
