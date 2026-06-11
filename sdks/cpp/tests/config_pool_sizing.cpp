// MDDS pool-sizing setter on tdx::Config.
//
// Offline test pinning the contract that `set_concurrent_requests`
// on the `tdx::Config` C++ wrapper invokes the underlying C ABI
// without crashing.

#include <cstdint>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("Config::set_concurrent_requests round-trips", "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    // The setter takes a uint32_t and returns void; the round-trip is
    // observable through the FFI handle's underlying MddsConfig field,
    // which the offline test cannot reach directly. The C++ test
    // therefore asserts only that the setter accepts a range of values
    // without throwing — the actual round-trip is exercised by the
    // Rust-side `ffi::auth::pool_sizing_tests::concurrent_requests_round_trips`.
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(0));
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(1));
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(8));
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(32));
}
