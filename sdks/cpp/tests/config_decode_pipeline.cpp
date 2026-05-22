// MDDS two-stage decode pipeline setters on tdx::Config.
//
// Offline tests pin the contract that the two setters exposed by the
// `tdx::Config` C++ wrapper -- `set_decode_threads` and
// `set_decode_queue_depth` -- accept both `std::nullopt` (auto-size
// sentinel) and an explicit `std::size_t` (pinned worker count /
// queue depth), and that the explicit values round-trip through the
// matching `get_decode_threads` / `get_decode_queue_depth` getters.
//
// The round-trip getter pins the behaviour the FFI's
// `tdx_config_set_decode_threads_explicit` widens: an explicit
// `std::optional{0}` survives the C boundary as `Some(0)` (the
// pool clamps to 1 at construction but the config preserves the
// caller's intent) — matches the Python / TS bindings.

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

namespace {

/// Read the current TLS error string. Returns the empty string when no
/// error has been recorded since the last `tdx_clear_error()` (or
/// since process start).
std::string last_error_text() {
    const char* raw = tdx_last_error();
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("Config::set_decode_threads accepts nullopt (auto-size sentinel)",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::nullopt));
    REQUIRE(last_error_text().empty());
    REQUIRE(cfg.get_decode_threads() == std::nullopt);
}

TEST_CASE("Config::set_decode_threads round-trips explicit values",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    for (std::size_t n : {std::size_t{0}, std::size_t{1}, std::size_t{2},
                          std::size_t{4}, std::size_t{8}, std::size_t{16},
                          std::size_t{32}, std::size_t{4096}}) {
        REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{n}));
        REQUIRE(last_error_text().empty());
        // Explicit `Some(n)` round-trips verbatim, including n=0 —
        // the pool clamps at construction but the config preserves
        // the caller-supplied value.
        REQUIRE(cfg.get_decode_threads() == std::optional<std::size_t>{n});
    }
}

TEST_CASE("Config::set_decode_threads round-trips nullopt after explicit",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{8}));
    REQUIRE(cfg.get_decode_threads() == std::optional<std::size_t>{8});
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::nullopt));
    REQUIRE(cfg.get_decode_threads() == std::nullopt);
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{16}));
    REQUIRE(cfg.get_decode_threads() == std::optional<std::size_t>{16});
    REQUIRE(last_error_text().empty());
}

TEST_CASE("Config::set_decode_queue_depth accepts nullopt (auto-size sentinel)",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(cfg.set_decode_queue_depth(std::nullopt));
    REQUIRE(last_error_text().empty());
    REQUIRE(cfg.get_decode_queue_depth() == std::nullopt);
}

TEST_CASE("Config::set_decode_queue_depth round-trips explicit values",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    for (std::size_t n : {std::size_t{0}, std::size_t{1}, std::size_t{64},
                          std::size_t{128}, std::size_t{512}, std::size_t{2048},
                          std::size_t{8192}, std::size_t{65536}}) {
        REQUIRE_NOTHROW(
            cfg.set_decode_queue_depth(std::optional<std::size_t>{n}));
        REQUIRE(last_error_text().empty());
        REQUIRE(cfg.get_decode_queue_depth() == std::optional<std::size_t>{n});
    }
}

TEST_CASE("Config::set_decode_queue_depth round-trips nullopt after explicit",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(
        cfg.set_decode_queue_depth(std::optional<std::size_t>{1024}));
    REQUIRE(cfg.get_decode_queue_depth() == std::optional<std::size_t>{1024});
    REQUIRE_NOTHROW(cfg.set_decode_queue_depth(std::nullopt));
    REQUIRE(cfg.get_decode_queue_depth() == std::nullopt);
    REQUIRE_NOTHROW(
        cfg.set_decode_queue_depth(std::optional<std::size_t>{4096}));
    REQUIRE(cfg.get_decode_queue_depth() == std::optional<std::size_t>{4096});
    REQUIRE(last_error_text().empty());
}

TEST_CASE("Config two-stage pipeline setters compose with legacy pool-sizing",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    cfg.set_concurrent_requests(8);
    cfg.set_decoder_threads(4);
    cfg.set_decoder_ring_size(1024);
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{16}));
    REQUIRE_NOTHROW(
        cfg.set_decode_queue_depth(std::optional<std::size_t>{4096}));
    REQUIRE(cfg.get_decode_threads() == std::optional<std::size_t>{16});
    REQUIRE(cfg.get_decode_queue_depth() == std::optional<std::size_t>{4096});
    REQUIRE(last_error_text().empty());
}
