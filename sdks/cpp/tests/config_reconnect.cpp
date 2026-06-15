// ReconnectConfig setters on thetadatadx::Config — C++ binding parity
// with Python / TypeScript / FFI.
//
// Pins the C++ surface contract for `set_reconnect_policy`,
// `set_reconnect_max_attempts`,
// `set_reconnect_max_rate_limited_attempts`, and
// `set_reconnect_stable_window_secs`. Failure-class semantics
// (transient vs rate-limited budget split, stable-window timer
// reset) are exercised in the Rust unit tests under
// `fpss::session::tests` and
// `fpss::protocol::reconnect_delays_match_policy`; this file pins
// only that the C++ wrapper forwards the inputs without crashing
// and that invalid policy ints fall through to the documented
// default behaviour without raising.

#include <cstdint>
#include <limits>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("Config::set_reconnect_policy accepts Auto and Manual selectors",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    REQUIRE_NOTHROW(cfg.set_reconnect_policy(0)); // Auto
    REQUIRE_NOTHROW(cfg.set_reconnect_policy(1)); // Manual
}

TEST_CASE("Config::set_reconnect_policy rejects unknown selectors with InvalidParameterError",
          "[config][reconnect][offline]") {
    // An unknown selector (outside {0, 1}) is rejected with the typed
    // invalid-parameter class rather than silently coerced to Auto —
    // the cross-binding contract the Python ValueError / TypeScript
    // InvalidParameterError already honour. The leaf must narrow
    // ThetaDataError so generic handlers still observe it.
    auto cfg = thetadatadx::Config::production();
    REQUIRE_THROWS_AS(cfg.set_reconnect_policy(7), thetadatadx::InvalidParameterError);
    REQUIRE_THROWS_AS(cfg.set_reconnect_policy(-1), thetadatadx::InvalidParameterError);
    REQUIRE_THROWS_AS(cfg.set_reconnect_policy(7), thetadatadx::ThetaDataError);
}

TEST_CASE("Config::set_reconnect_max_attempts accepts representative budgets without throwing",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(0); // Auto
    for (std::uint32_t n : {0u, 1u, 3u, 10u, 100u, 1000u}) {
        REQUIRE_NOTHROW(cfg.set_reconnect_max_attempts(n));
    }
}

TEST_CASE("Config::set_reconnect_max_rate_limited_attempts accepts representative budgets without throwing",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(0); // Auto
    for (std::uint32_t n : {0u, 1u, 10u, 100u, 1000u}) {
        REQUIRE_NOTHROW(cfg.set_reconnect_max_rate_limited_attempts(n));
    }
}

TEST_CASE("Config::set_reconnect_stable_window_secs accepts u64 values",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(0); // Auto
    for (std::uint64_t secs : {std::uint64_t{0}, std::uint64_t{1},
                               std::uint64_t{60}, std::uint64_t{3600},
                               std::uint64_t{86'400},
                               std::numeric_limits<std::uint64_t>::max()}) {
        REQUIRE_NOTHROW(cfg.set_reconnect_stable_window_secs(secs));
    }
}

TEST_CASE("Reconnect setters under Manual policy are silent no-ops",
          "[config][reconnect][offline]") {
    // Matches the cross-binding contract: per-class budget setters
    // only mutate `ReconnectAttemptLimits` when the policy is
    // `Auto(limits)`. Under `Manual` the calls are silently absorbed;
    // the wrapper surface must not throw.
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(1); // Manual
    REQUIRE_NOTHROW(cfg.set_reconnect_max_attempts(5));
    REQUIRE_NOTHROW(cfg.set_reconnect_max_rate_limited_attempts(50));
    REQUIRE_NOTHROW(cfg.set_reconnect_stable_window_secs(120));
}

TEST_CASE("Reconnect setters compose with pool-sizing setters",
          "[config][reconnect][offline]") {
    // Interleaved reconnect setter and pool-sizing setter calls on
    // the same `thetadatadx::Config` must not interfere with each other.
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(0);
    cfg.set_reconnect_max_attempts(7);
    cfg.set_reconnect_max_rate_limited_attempts(77);
    cfg.set_reconnect_stable_window_secs(120);
    REQUIRE_NOTHROW(cfg.set_concurrent_requests(4));
}
