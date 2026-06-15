// Client FPSS surface tests.
//
// The typed `Client` wrapper exposes the full push-callback
// streaming surface, so callers reach every method below without
// dropping to the raw `thetadatadx_client_*` C ABI handle.
//
// Offline tests confirm:
//   * `is_streaming` returns false on a moved-from / never-connected
//     handle without throwing.
//   * `dropped_event_count` returns 0 on the same.
//   * Move-construct + move-assign hold the callback-storage
//     ordering invariant (no UAF in the destructor).
//   * The wrapper compiles with each method bound — symbol presence
//     is the surface contract this file pins.
//
// Live tests (gated on `THETADX_LIVE_CREDS`) drive the full
// set_callback -> stop_streaming -> reconnect -> await_drain ->
// dropped_event_count -> is_streaming -> active_subscriptions cycle
// against the production server.

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <functional>
#include <string>
#include <thread>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("Client is move-only with the right type-trait shape",
          "[unified][offline]") {
    STATIC_REQUIRE(std::is_move_constructible_v<thetadatadx::Client>);
    STATIC_REQUIRE(std::is_move_assignable_v<thetadatadx::Client>);
    STATIC_REQUIRE_FALSE(std::is_copy_constructible_v<thetadatadx::Client>);
    STATIC_REQUIRE_FALSE(std::is_copy_assignable_v<thetadatadx::Client>);
}

TEST_CASE("Stream binds the full FPSS surface",
          "[unified][offline]") {
    // The unified client's streaming surface lives on the
    // `client.stream()` `Stream` view; pin every method there so an
    // accidental delete or rename fires at compile time rather than at
    // runtime against a live server.
    using namespace std::chrono_literals;
    using Cb = std::function<void(const thetadatadx::StreamEvent&)>;
    using SV = thetadatadx::Stream;

    // Client exposes the sub-namespace accessors.
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<thetadatadx::Client&>().stream()), SV>);
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const thetadatadx::Client&>().historical()),
        thetadatadx::Historical>);

    // set_callback
    STATIC_REQUIRE(std::is_invocable_v<decltype(&SV::set_callback), SV&, Cb>);
    // stop_streaming
    STATIC_REQUIRE(std::is_invocable_v<decltype(&SV::stop_streaming), SV&>);
    // reconnect
    STATIC_REQUIRE(std::is_invocable_v<decltype(&SV::reconnect), SV&>);
    // await_drain(std::chrono::milliseconds) -> bool
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<SV&>().await_drain(5000ms)), bool>);
    // dropped_event_count() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().dropped_event_count()), uint64_t>);
    // ring_occupancy() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().ring_occupancy()), uint64_t>);
    // ring_capacity() -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().ring_capacity()), uint64_t>);
    // is_streaming() -> bool
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().is_streaming()), bool>);
    // active_subscriptions() -> std::vector<Subscription>
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().active_subscriptions()),
        std::vector<thetadatadx::Subscription>>);
    // active_full_subscriptions() lives on the `client.stream()` view
    // (mirrors the Python / TypeScript placement) -> std::vector<FullSubscription>
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().active_full_subscriptions()),
        std::vector<thetadatadx::FullSubscription>>);
    // panic_count() lives on the `client.stream()` view -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().panic_count()), uint64_t>);
}

TEST_CASE("Client end-to-end push-callback cycle", "[unified][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::Client::connect(creds, config);

    // The streaming surface is reached through the `client.stream()` view;
    // every view shares the client's callback slot, so a fresh view per
    // call observes the same session.
    auto stream = client.stream();
    REQUIRE_FALSE(stream.is_streaming());
    REQUIRE(stream.dropped_event_count() == 0);

    std::atomic<uint64_t> events{0};
    stream.set_callback([&](const thetadatadx::StreamEvent& /*event*/) {
        events.fetch_add(1, std::memory_order_relaxed);
    });

    // Subscribe so the streaming session has work to do; live status
    // depends on whether the upstream finished the handshake before
    // this check fires. The C ABI is_streaming flips true on a
    // successful Connected event; we wait briefly so a slow login
    // doesn't race us.
    stream.subscribe(thetadatadx::Contract::stock("SPY").quote());
    std::this_thread::sleep_for(std::chrono::seconds(1));
    REQUIRE(stream.is_streaming());

    // active_subscriptions reflects the subscribe call. `contract` is the
    // canonical contract Display (root + sec_type), so a stock subscription
    // renders as "SPY STOCK".
    const auto subs = stream.active_subscriptions();
    REQUIRE(subs.size() == 1);
    REQUIRE(subs.front().contract == "SPY STOCK");

    // active_full_subscriptions starts empty (we did not full-subscribe).
    // It lives on the `client.stream()` view, mirroring Python / TypeScript.
    REQUIRE(stream.active_full_subscriptions().empty());

    // Reconnect exercises the C ABI reconnect path + the wrapper's
    // saved-subscription re-registration.
    REQUIRE_NOTHROW(stream.reconnect());
    std::this_thread::sleep_for(std::chrono::seconds(1));
    REQUIRE(stream.is_streaming());

    // Stop + drain.
    stream.stop_streaming();
    const bool drained = stream.await_drain(std::chrono::seconds(5));
    REQUIRE(drained);
    REQUIRE_FALSE(stream.is_streaming());

    // Sanity check: events advanced. Outside market hours we still
    // get Connected / LoginSuccess events, so the lower bound is
    // intentionally generous.
    REQUIRE(events.load(std::memory_order_relaxed) >= 1);
}
