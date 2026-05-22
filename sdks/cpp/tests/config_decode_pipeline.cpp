// MDDS two-stage decode pipeline setters on tdx::Config
// (Phase 3 of 3, follow-up to PR #587 / #588).
//
// Offline tests pin the contract that the two new setters exposed by
// the `tdx::Config` C++ wrapper -- `set_decode_threads` and
// `set_decode_queue_depth` -- accept both `std::nullopt` (auto-size
// sentinel) and an explicit `std::size_t` (pinned worker count /
// queue depth) without throwing.
//
// The round-trip is observable through the FFI handle's underlying
// `MddsConfig` field, which the offline C++ test cannot reach
// directly. The Rust-side `ffi::auth::decode_pipeline_tests` exercises
// the actual round-trip; the C++ test here pins only that:
//
//   * Both setters accept `std::nullopt`, `std::optional{0}`,
//     `std::optional{1}`, large explicit values without throwing.
//   * The setters do NOT touch `tdx_last_error()` on success (so a
//     subsequent error-classifier call observes the cleared state).
//   * The setters compose with the legacy pool-sizing knobs on the
//     same `Config` instance.

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
}

TEST_CASE("Config::set_decode_threads accepts explicit values",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    for (std::size_t n : {std::size_t{0}, std::size_t{1}, std::size_t{2},
                          std::size_t{4}, std::size_t{8}, std::size_t{16},
                          std::size_t{32}, std::size_t{4096}}) {
        REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{n}));
        REQUIRE(last_error_text().empty());
    }
}

TEST_CASE("Config::set_decode_threads round-trips nullopt after explicit",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{8}));
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::nullopt));
    REQUIRE_NOTHROW(cfg.set_decode_threads(std::optional<std::size_t>{16}));
    REQUIRE(last_error_text().empty());
}

TEST_CASE("Config::set_decode_queue_depth accepts nullopt (auto-size sentinel)",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(cfg.set_decode_queue_depth(std::nullopt));
    REQUIRE(last_error_text().empty());
}

TEST_CASE("Config::set_decode_queue_depth accepts explicit values",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    for (std::size_t n : {std::size_t{0}, std::size_t{1}, std::size_t{64},
                          std::size_t{128}, std::size_t{512}, std::size_t{2048},
                          std::size_t{8192}, std::size_t{65536}}) {
        REQUIRE_NOTHROW(
            cfg.set_decode_queue_depth(std::optional<std::size_t>{n}));
        REQUIRE(last_error_text().empty());
    }
}

TEST_CASE("Config::set_decode_queue_depth round-trips nullopt after explicit",
          "[config][decode_pipeline][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    REQUIRE_NOTHROW(
        cfg.set_decode_queue_depth(std::optional<std::size_t>{1024}));
    REQUIRE_NOTHROW(cfg.set_decode_queue_depth(std::nullopt));
    REQUIRE_NOTHROW(
        cfg.set_decode_queue_depth(std::optional<std::size_t>{4096}));
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
    REQUIRE(last_error_text().empty());
}
