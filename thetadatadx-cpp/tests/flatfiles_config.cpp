// FlatFilesConfig setters on thetadatadx::Config — C++ binding parity
// with Python / TypeScript / FFI.
//
// Pins the C++ surface contract for `set_flatfiles_max_attempts`,
// `set_flatfiles_initial_backoff_secs`,
// `set_flatfiles_max_backoff_secs`, `set_flatfiles_connect_timeout_secs`,
// and `set_flatfiles_read_timeout_secs`. The Rust core enforces the
// `[1, 10]` range on `max_attempts` and the
// `max_backoff >= initial_backoff` invariant at
// `DirectConfig::validate` time, not at the C ABI setter; this file
// pins only that the wrapper forwards the inputs without crashing
// and that the getter round-trips the value.

#include <cstdint>
#include <limits>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.h"
#include "thetadatadx.hpp"

TEST_CASE("Config exposes FlatFilesConfig production defaults",
          "[config][flatfiles][offline]") {
    auto cfg = thetadatadx::Config::production();
    REQUIRE(cfg.get_flatfiles_max_attempts() == 10u);
    REQUIRE(cfg.get_flatfiles_initial_backoff_secs() == 1u);
    REQUIRE(cfg.get_flatfiles_max_backoff_secs() == 30u);
    REQUIRE(cfg.get_flatfiles_connect_timeout_secs() == 10u);
    REQUIRE(cfg.get_flatfiles_read_timeout_secs() == 60u);
}

TEST_CASE("Config::set_flatfiles_max_attempts round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = thetadatadx::Config::production();
    for (std::uint32_t n : {0u, 1u, 3u, 5u, 10u, 100u, 1000u}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_max_attempts(n));
        REQUIRE(cfg.get_flatfiles_max_attempts() == n);
    }
}

TEST_CASE("Config::set_flatfiles_initial_backoff_secs round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = thetadatadx::Config::production();
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
    auto cfg = thetadatadx::Config::production();
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{4}, std::uint64_t{60},
                               std::uint64_t{3600}, std::uint64_t{86'400}}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_max_backoff_secs(secs));
        REQUIRE(cfg.get_flatfiles_max_backoff_secs() == secs);
    }
}

TEST_CASE("Config::set_flatfiles_connect_timeout_secs round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = thetadatadx::Config::production();
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{4}, std::uint64_t{10},
                               std::uint64_t{60}, std::uint64_t{3600}}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_connect_timeout_secs(secs));
        REQUIRE(cfg.get_flatfiles_connect_timeout_secs() == secs);
    }
}

TEST_CASE("Config::set_flatfiles_read_timeout_secs round-trips via getter",
          "[config][flatfiles][offline]") {
    auto cfg = thetadatadx::Config::production();
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{4}, std::uint64_t{60},
                               std::uint64_t{3600}, std::uint64_t{86'400}}) {
        REQUIRE_NOTHROW(cfg.set_flatfiles_read_timeout_secs(secs));
        REQUIRE(cfg.get_flatfiles_read_timeout_secs() == secs);
    }
}

TEST_CASE("FlatFiles setters compose with historical tuning setters",
          "[config][flatfiles][offline]") {
    // Interleaved flatfiles setter and historical tuning setter calls
    // on the same `thetadatadx::Config` must not interfere with each
    // other.
    auto cfg = thetadatadx::Config::production();
    REQUIRE_NOTHROW(cfg.set_flatfiles_max_attempts(7));
    REQUIRE_NOTHROW(cfg.set_flatfiles_initial_backoff_secs(3));
    REQUIRE_NOTHROW(cfg.set_flatfiles_max_backoff_secs(12));
    REQUIRE_NOTHROW(cfg.set_flatfiles_connect_timeout_secs(20));
    REQUIRE_NOTHROW(cfg.set_flatfiles_read_timeout_secs(45));
    REQUIRE_NOTHROW(cfg.set_warn_on_buffered_threshold_bytes(8 * 1024 * 1024));

    REQUIRE(cfg.get_flatfiles_max_attempts() == 7u);
    REQUIRE(cfg.get_flatfiles_initial_backoff_secs() == 3u);
    REQUIRE(cfg.get_flatfiles_max_backoff_secs() == 12u);
    REQUIRE(cfg.get_flatfiles_connect_timeout_secs() == 20u);
    REQUIRE(cfg.get_flatfiles_read_timeout_secs() == 45u);
    REQUIRE(cfg.get_warn_on_buffered_threshold_bytes() == 8u * 1024u * 1024u);
}
