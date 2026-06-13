// FlatFilesConfig setters on tdx::Config — C++ binding parity
// with Python / TypeScript / FFI.
//
// Pins the C++ surface contract for `set_flatfiles_max_attempts`,
// `set_flatfiles_initial_backoff_secs`, and
// `set_flatfiles_max_backoff_secs`. The Rust core enforces the
// `[1, 10]` range on `max_attempts` and the
// `max_backoff >= initial_backoff` invariant at
// `DirectConfig::validate` time, not at the C ABI setter; this file
// pins only that the wrapper forwards the inputs without crashing
// and that the getter round-trips the value.

#include <cstdint>
#include <limits>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("Config exposes FlatFilesConfig production defaults",
          "[config][flatfiles][offline]") {
    auto cfg = tdx::Config::production();
    REQUIRE(cfg.get_flatfiles_max_attempts() == 10u);
    REQUIRE(cfg.get_flatfiles_initial_backoff_secs() == 1u);
    REQUIRE(cfg.get_flatfiles_max_backoff_secs() == 30u);
}

TEST_CASE("Config::set_flatfiles_max_attempts round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = tdx::Config::production();
    for (std::uint32_t n : {0u, 1u, 3u, 5u, 10u, 100u, 1000u}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_max_attempts(n));
        REQUIRE(cfg.get_flatfiles_max_attempts() == n);
    }
}

TEST_CASE("Config::set_flatfiles_initial_backoff_secs round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = tdx::Config::production();
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{2}, std::uint64_t{4},
                               std::uint64_t{60}, std::uint64_t{3600},
                               std::uint64_t{86'400}}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_initial_backoff_secs(secs));
        REQUIRE(cfg.get_flatfiles_initial_backoff_secs() == secs);
    }
}

TEST_CASE("Config::set_flatfiles_max_backoff_secs round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = tdx::Config::production();
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{4}, std::uint64_t{60},
                               std::uint64_t{3600}, std::uint64_t{86'400}}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_max_backoff_secs(secs));
        REQUIRE(cfg.get_flatfiles_max_backoff_secs() == secs);
    }
}

TEST_CASE("FlatFiles setters compose with pool-sizing setters",
          "[config][flatfiles][offline]") {
    // Interleaved flatfiles setter and pool-sizing setter calls on
    // the same `tdx::Config` must not interfere with each other.
    auto cfg = tdx::Config::production();
    REQUIRE_NOTHROW(cfg.set_flatfiles_max_attempts(7));
    REQUIRE_NOTHROW(cfg.set_flatfiles_initial_backoff_secs(3));
    REQUIRE_NOTHROW(cfg.set_flatfiles_max_backoff_secs(12));
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(4));

    REQUIRE(cfg.get_flatfiles_max_attempts() == 7u);
    REQUIRE(cfg.get_flatfiles_initial_backoff_secs() == 3u);
    REQUIRE(cfg.get_flatfiles_max_backoff_secs() == 12u);
}
