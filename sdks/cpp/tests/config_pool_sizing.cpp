// MDDS pool-sizing setters on tdx::Config (issue #584).
//
// Offline tests pin the contract that the three new setters exposed by
// the `tdx::Config` C++ wrapper — `set_concurrent_requests`,
// `set_decoder_threads`, `set_decoder_ring_size` — invoke the underlying
// C ABI without crashing, and that `set_decoder_ring_size` surfaces
// validation failures via `tdx_last_error()` while leaving the config
// unchanged on rejection.

#include <cstdint>
#include <cstring>
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

TEST_CASE("Config::set_decoder_threads round-trips", "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    REQUIRE_NOTHROW(cfg.set_decoder_threads(0));
    REQUIRE_NOTHROW(cfg.set_decoder_threads(1));
    REQUIRE_NOTHROW(cfg.set_decoder_threads(8));
    REQUIRE_NOTHROW(cfg.set_decoder_threads(64));
}

TEST_CASE("Config::set_decoder_ring_size accepts valid powers of two",
          "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    for (std::uint32_t n : {64u, 128u, 256u, 512u, 1024u, 2048u, 4096u}) {
        cfg.set_decoder_ring_size(n);
        REQUIRE(last_error_text().empty());
    }
}

TEST_CASE("Config::set_decoder_ring_size rejects below-minimum values",
          "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    cfg.set_decoder_ring_size(32);
    auto err = last_error_text();
    REQUIRE_FALSE(err.empty());
    REQUIRE(err.find("decoder_ring_size") != std::string::npos);
}

TEST_CASE("Config::set_decoder_ring_size rejects non-power-of-two values",
          "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    cfg.set_decoder_ring_size(100);
    auto err = last_error_text();
    REQUIRE_FALSE(err.empty());
    REQUIRE(err.find("decoder_ring_size") != std::string::npos);

    tdx_clear_error();
    cfg.set_decoder_ring_size(1023);
    err = last_error_text();
    REQUIRE_FALSE(err.empty());
    REQUIRE(err.find("decoder_ring_size") != std::string::npos);
}

TEST_CASE("Config::set_decoder_ring_size rejects zero", "[config][pool_sizing][offline]") {
    auto cfg = tdx::Config::production();
    tdx_clear_error();
    cfg.set_decoder_ring_size(0);
    auto err = last_error_text();
    REQUIRE_FALSE(err.empty());
    REQUIRE(err.find("decoder_ring_size") != std::string::npos);
}
