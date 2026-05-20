// Pull-iter delivery — the C++ RAII helper closes the gap where TS
// and Python had `streamingIter()` / `streaming_iter()` context
// managers but C++ users had to drive `start_streaming_iter()`,
// `stop_streaming()`, and `await_drain()` by hand.
//
// `UnifiedFpssIterSession`'s destructor pairs `close()` on the
// iterator with `stop_streaming()` + `await_drain(5000)` on the
// parent so the consumer thread is guaranteed to have stopped
// pushing into the queue before any captured state goes out of
// scope.

#include <chrono>
#include <cstdlib>
#include <optional>
#include <string>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("UnifiedFpssIterSession is move-only", "[streaming-iter][offline]") {
    STATIC_REQUIRE(std::is_move_constructible_v<tdx::UnifiedFpssIterSession>);
    STATIC_REQUIRE(std::is_move_assignable_v<tdx::UnifiedFpssIterSession>);
    STATIC_REQUIRE_FALSE(std::is_copy_constructible_v<tdx::UnifiedFpssIterSession>);
    STATIC_REQUIRE_FALSE(std::is_copy_assignable_v<tdx::UnifiedFpssIterSession>);
}

TEST_CASE("UnifiedFpssIterSession exposes the documented surface",
          "[streaming-iter][offline]") {
    using namespace std::chrono_literals;
    using Sess = tdx::UnifiedFpssIterSession;

    // next(timeout) -> std::optional<TdxFpssEvent>
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<Sess&>().next(50ms)),
        std::optional<TdxFpssEvent>>);
    // try_next() -> std::optional<TdxFpssEvent>
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<Sess&>().try_next()),
        std::optional<TdxFpssEvent>>);
    // ended() / close()
    STATIC_REQUIRE(std::is_same_v<
        decltype(std::declval<const Sess&>().ended()), bool>);
    STATIC_REQUIRE(std::is_invocable_v<decltype(&Sess::close), Sess&>);
}

TEST_CASE("UnifiedFpssIterSession construct / iterate / destruct",
          "[streaming-iter][live]") {
    const auto creds_path = env_or_empty("THETADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADX_LIVE_CREDS not set");
    }
    auto creds = tdx::Credentials::from_file(creds_path);
    auto config = tdx::Config::production();
    auto client = tdx::UnifiedClient::connect(creds, config);

    {
        auto session = client.streaming_iter_session();
        client.subscribe(tdx::Contract::stock("SPY").quote());

        // Drain a few events to confirm the queue is being fed.
        // Outside market hours we still get the Connected /
        // LoginSuccess sequence so the loop terminates promptly.
        uint64_t observed = 0;
        for (int i = 0; i < 16; ++i) {
            auto event = session.next(std::chrono::milliseconds(200));
            if (event.has_value()) {
                ++observed;
            }
            if (observed >= 1) break;
        }
        REQUIRE(observed >= 1);
        // Destructor runs here: close() + stop_streaming() + drain.
    }

    // After destruction, the streaming session has been stopped and
    // the drain barrier has returned. The unified client itself is
    // healthy — historical / re-streaming-iter can be issued.
    REQUIRE_FALSE(client.is_streaming());
}
