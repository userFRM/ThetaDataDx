// ReconnectConfig setters on thetadatadx::Config — C++ binding parity
// with Python / TypeScript / FFI.
//
// Pins that each C++ reconnect setter reaches the field its getter reads,
// that the budget setters leave the Manual policy in place, and that an
// unknown policy selector is rejected. Failure-class semantics
// (transient vs rate-limited budget split, stable-window timer
// reset) are exercised in the Rust unit tests under
// `fpss::session::tests` and
// `fpss::protocol::reconnect_delays_match_policy`.

#include <cstdint>
#include <limits>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.h"
#include "thetadatadx.hpp"

TEST_CASE("Config::set_reconnect_policy round-trips Auto and Manual",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(1); // Manual
    REQUIRE(cfg.get_reconnect_policy() == 1);
    cfg.set_reconnect_policy(0); // Auto
    REQUIRE(cfg.get_reconnect_policy() == 0);
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

TEST_CASE("Reconnect budgets round-trip under the Auto policy",
          "[config][reconnect][offline]") {
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(0); // Auto
    cfg.set_reconnect_max_attempts(7);
    cfg.set_reconnect_max_rate_limited_attempts(77);
    cfg.set_reconnect_stable_window_secs(120);
    REQUIRE(cfg.get_reconnect_max_attempts() == 7);
    REQUIRE(cfg.get_reconnect_max_rate_limited_attempts() == 77);
    REQUIRE(cfg.get_reconnect_stable_window_secs() == 120);

    cfg.set_reconnect_stable_window_secs(std::numeric_limits<std::uint64_t>::max());
    REQUIRE(cfg.get_reconnect_stable_window_secs() ==
            std::numeric_limits<std::uint64_t>::max());
}

TEST_CASE("Reconnect budget setters leave the Manual policy in place",
          "[config][reconnect][offline]") {
    // Matches the cross-binding contract: per-class budget setters
    // only mutate `ReconnectAttemptLimits` when the policy is
    // `Auto(limits)`. Under `Manual` they must not switch the policy.
    auto cfg = thetadatadx::Config::production();
    cfg.set_reconnect_policy(1); // Manual
    cfg.set_reconnect_max_attempts(5);
    cfg.set_reconnect_max_rate_limited_attempts(50);
    cfg.set_reconnect_stable_window_secs(120);
    REQUIRE(cfg.get_reconnect_policy() == 1);
}
