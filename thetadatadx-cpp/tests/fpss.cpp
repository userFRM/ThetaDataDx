// FPSS standalone-client smoke tests.
//
// `StreamingClient::connect` is the dedicated streaming entry point —
// distinct from the unified handle covered by `unified_client.cpp`.
// Offline tests cover only the move-semantics surface; the live half
// exercises a connect -> set_callback -> stop_streaming cycle.

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <optional>
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

TEST_CASE("StreamingClient binds the observability surface",
          "[fpss][offline]") {
    // Pin the diagnostic accessors so a delete or rename fires at compile
    // time. The standalone client uses the same `dropped_event_count()`
    // spelling as the unified `Stream` view (the counter is identical on
    // both surfaces).
    using SC = thetadatadx::StreamingClient;
    // dropped_event_count() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().dropped_event_count()), uint64_t>);
    // ring_occupancy() / ring_capacity() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().ring_occupancy()), uint64_t>);
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().ring_capacity()), uint64_t>);
    // panic_count() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().panic_count()), uint64_t>);
    // is_streaming() -> bool (evened up with the unified Stream view and
    // the Python / TypeScript standalone surface)
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().is_streaming()), bool>);
    // is_authenticated() -> bool
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().is_authenticated()), bool>);
    // active_full_subscriptions() -> std::vector<FullSubscription>
    // (evened up with the unified Stream view and the Python / TypeScript
    // standalone surface)
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SC&>().active_full_subscriptions()),
        std::vector<thetadatadx::FullSubscription>>);
}

TEST_CASE("StreamingClient teardown without a callback completes promptly",
          "[fpss][offline]") {
    // `thetadatadx_streaming_connect` only allocates a handle and stashes the
    // connection params; the FPSS TLS connection + consumer thread are created
    // lazily at `set_callback`. So a StreamingClient can be constructed offline
    // and, with no callback ever installed, has no consumer thread to drain.
    // Teardown must NOT enter the retired-session reclaimer (whose poll runs
    // for up to kReclaimQuiescenceCap = 300 s): with `callback_` null the
    // members destruct synchronously. Regression guard for a teardown that
    // hot-spun ~300 s and then leaked the credential-bearing handle.
    auto creds = thetadatadx::Credentials::from_api_key("offline-test-key");
    auto config = thetadatadx::Config::production();

    const auto start = std::chrono::steady_clock::now();
    {
        thetadatadx::StreamingClient client(creds, config);
        // Intentionally never call set_callback: no consumer thread, no ctx.
    } // destructor runs here
    const auto elapsed = std::chrono::steady_clock::now() - start;

    // Well under a second; the pre-fix hot-spin path took ~300 s.
    REQUIRE(elapsed < std::chrono::seconds(1));
}

TEST_CASE("StreamingClient teardown after a throwing set_callback completes promptly",
          "[fpss][offline]") {
    // Offline, `set_callback` opens the FPSS connection, fails to reach the
    // server, and throws — leaving `callback_` null because the adopt happens
    // only after the FFI reports success. Teardown of such a client must also
    // skip the reclaimer (nothing was ever wired) and destruct synchronously.
    auto creds = thetadatadx::Credentials::from_api_key("offline-test-key");
    auto config = thetadatadx::Config::production();

    // Optional so the destructor can be timed in isolation: the connect inside
    // set_callback may take a bounded moment to fail offline, and only the
    // teardown must be fast. Reset() below runs the destructor under the clock.
    auto client = std::make_optional<thetadatadx::StreamingClient>(creds, config);
    // No server reachable offline, so the connect inside set_callback must fail
    // and throw; the callback is NOT adopted on the throw path (callback_ stays
    // null).
    REQUIRE_THROWS(client->set_callback(
        [](const thetadatadx::StreamEvent& /*event*/) {}));

    const auto start = std::chrono::steady_clock::now();
    client.reset(); // destructor runs here, with callback_ still null
    const auto elapsed = std::chrono::steady_clock::now() - start;

    // Well under a second; the pre-fix hot-spin path took ~300 s.
    REQUIRE(elapsed < std::chrono::seconds(1));
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
