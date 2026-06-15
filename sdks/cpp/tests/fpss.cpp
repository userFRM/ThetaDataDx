// FPSS standalone-client smoke tests.
//
// `StreamingClient::connect` is the dedicated streaming entry point —
// distinct from the unified handle covered by `unified_client.cpp`.
// Offline tests cover only the move-semantics surface; the live half
// exercises a connect -> set_callback -> stop_streaming cycle.

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <string>
#include <thread>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("StreamingClient is move-constructible", "[fpss][offline]") {
    // We can't actually construct an StreamingClient without a live
    // connection, but the type must at least be move-constructible
    // and move-assignable per the API contract. Verifying via type
    // traits keeps the test offline.
    STATIC_REQUIRE(std::is_move_constructible_v<thetadatadx::StreamingClient>);
    STATIC_REQUIRE(std::is_move_assignable_v<thetadatadx::StreamingClient>);
    STATIC_REQUIRE_FALSE(std::is_copy_constructible_v<thetadatadx::StreamingClient>);
    STATIC_REQUIRE_FALSE(std::is_copy_assignable_v<thetadatadx::StreamingClient>);
}

TEST_CASE("StreamingClient registers a callback and receives at least one event",
          "[fpss][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    thetadatadx::StreamingClient client(creds, config);

    std::atomic<uint64_t> events{0};
    client.set_callback([&](const thetadatadx::StreamEvent& /*event*/) {
        events.fetch_add(1, std::memory_order_relaxed);
    });

    // Subscribe to a single liquid symbol's quote stream so the
    // callback observes at least one Connected event plus (during
    // market hours) live quote frames. Outside market hours we still
    // get the Connected/LoginSuccess sequence, so the assertion is
    // tolerant.
    client.subscribe(thetadatadx::Contract::stock("SPY").quote());
    std::this_thread::sleep_for(std::chrono::seconds(2));
    REQUIRE(events.load(std::memory_order_relaxed) >= 1);
}
