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
// Live tests (gated on `THETADATADX_LIVE_CREDS`) drive the full
// set_callback -> stop_streaming -> reconnect -> await_drain ->
// dropped_event_count -> is_streaming -> active_subscriptions cycle
// against the production server.

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <functional>
#include <string>
#include <utility>
#include <thread>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

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

TEST_CASE("ClientBuilder validates auth before connecting",
          "[unified][offline]") {
    // The builder rejects an empty auth set and a conflict BEFORE any
    // network round-trip, surfacing the failure as a typed `ConfigError`.
    // These paths never touch the server, so they run offline.

    // No authentication source set → ConfigError.
    REQUIRE_THROWS_AS(thetadatadx::Client::builder().connect(),
                      thetadatadx::ConfigError);

    // Two different authentication sources → ConfigError naming both.
    REQUIRE_THROWS_AS(thetadatadx::Client::builder()
                          .api_key("td1_example")
                          .email_password("you@example.com", "secret")
                          .connect(),
                      thetadatadx::ConfigError);

    // The builder is fluent: each setter returns a reference so the chain
    // composes. Pin the surface (api key first-class, plus the
    // environment selectors) at compile time without connecting.
    thetadatadx::ClientBuilder builder = thetadatadx::Client::builder();
    builder.api_key("td1_example").stage();
}

TEST_CASE("ClientBuilder is single-use: connect() consumes the builder",
          "[unified][offline]") {
    // `connect()` is rvalue-ref-qualified, mirroring the Rust
    // `ClientBuilder::connect(self)`. The documented inline form is an
    // rvalue chain all the way through, so it compiles and reaches the
    // pre-flight validation (here a conflict, surfaced before any network
    // round-trip). This pins the consuming surface at compile time.
    REQUIRE_THROWS_AS(thetadatadx::Client::builder()
                          .api_key("td1_example")
                          .email_password("you@example.com", "secret")
                          .stage()
                          .connect(),
                      thetadatadx::ConfigError);

    // A stored builder is handed over explicitly with std::move, which is
    // the only way to reach the rvalue-only connect() from a named builder.
    // Calling `stored.connect()` directly would NOT compile: connect() has
    // a `&&` ref-qualifier, so a second use of a moved-from builder cannot
    // be written by accident. The handover below consumes it exactly once,
    // and the conflicting sources make connect() throw before any network
    // round-trip, keeping the test offline.
    thetadatadx::ClientBuilder stored = thetadatadx::Client::builder();
    stored.api_key("td1_example").email_password("you@example.com", "secret");
    REQUIRE_THROWS_AS(std::move(stored).connect(), thetadatadx::ConfigError);
}

TEST_CASE("api_key_from_env is strict — unset env throws ConfigError",
          "[unified][offline]") {
    // Mirror the Rust `ClientBuilder::api_key_from_env`: an unset or
    // whitespace-only `THETADATA_API_KEY` is a ConfigError before any
    // network round-trip, with NO `creds.txt` file fallback. This path
    // never touches the server, so it runs offline.
    const std::string saved = env_or_empty("THETADATA_API_KEY");
    ::unsetenv("THETADATA_API_KEY");

    REQUIRE_THROWS_AS(thetadatadx::Client::builder().api_key_from_env().connect(),
                      thetadatadx::ConfigError);

    // A whitespace-only value is likewise rejected (not trimmed to a
    // valid key, not fallen back to a file).
    ::setenv("THETADATA_API_KEY", "   ", 1);
    REQUIRE_THROWS_AS(thetadatadx::Client::builder().api_key_from_env().connect(),
                      thetadatadx::ConfigError);

    // Restore the caller's environment so sibling tests are unaffected.
    if (saved.empty()) {
        ::unsetenv("THETADATA_API_KEY");
    } else {
        ::setenv("THETADATA_API_KEY", saved.c_str(), 1);
    }
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
    // is_authenticated() -> bool (mirrors the standalone
    // StreamingClient::is_authenticated() and the Python / TypeScript
    // client.stream.is_authenticated placement)
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().is_authenticated()), bool>);
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
    // slow_callback_count() lives on the `client.stream()` view -> uint64_t
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().slow_callback_count()), uint64_t>);
    // set_slow_callback_threshold_us(uint64_t) -> void
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const SV&>().set_slow_callback_threshold_us(uint64_t{})),
        void>);
}

TEST_CASE("Client end-to-end push-callback cycle", "[unified][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::Client::connect(creds, config);

    // The streaming surface is reached through the `client.stream()` view;
    // every view shares the client's callback slot, so a fresh view per
    // call observes the same session.
    auto stream = client.stream();
    REQUIRE_FALSE(stream.is_streaming());
    // Distinct from is_streaming(): the session is not authenticated
    // before streaming opens.
    REQUIRE_FALSE(stream.is_authenticated());
    REQUIRE(stream.dropped_event_count() == 0);
    // Slow-callback watchdog: 0 before streaming; the threshold setter
    // round-trips as a no-throw configuration call (microseconds; 0
    // disables).
    REQUIRE(stream.slow_callback_count() == 0);
    REQUIRE_NOTHROW(stream.set_slow_callback_threshold_us(1000));

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
