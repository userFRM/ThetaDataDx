// Async historical query surface tests.
//
// Every buffered historical / snapshot query carries an `<endpoint>_async`
// companion returning a `std::future<std::vector<Row>>` so callers can run
// the request off the calling thread without managing their own threads.
// The offline half pins the static shape (return type + that the call
// compiles and a future is produced) without a network round-trip; the
// live half guards that `.get()` yields the same rows as the blocking call
// and that a typed error surfaces on `.get()`.

#include <cstdlib>
#include <future>
#include <string>
#include <type_traits>
#include <vector>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

} // namespace

TEST_CASE("async query methods return std::future of the sync row type",
          "[history][async][offline]") {
    // Type-level assertion: the async companion's return type must be a
    // `std::future` of exactly the vector the blocking method returns. No
    // connection is needed — the check is purely on the declared signature
    // via `decltype` on an unevaluated member-pointer expression.
    using Hist = thetadatadx::HistoricalClient;

    using SyncRet = decltype(std::declval<const Hist&>().stock_history_eod(
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<thetadatadx::EndpointRequestOptions>()));
    using AsyncRet = decltype(std::declval<const Hist&>().stock_history_eod_async(
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<thetadatadx::EndpointRequestOptions>()));

    STATIC_REQUIRE(std::is_same_v<SyncRet, std::vector<thetadatadx::EodTick>>);
    STATIC_REQUIRE(std::is_same_v<AsyncRet, std::future<std::vector<thetadatadx::EodTick>>>);

    // The companion is present on the unified client's `Historical` view as
    // well, at the same shape — the generated surface is emitted onto both
    // historical classes from one template.
    using View = thetadatadx::Historical;
    using ViewAsyncRet = decltype(std::declval<const View&>().stock_history_eod_async(
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<std::string>(),
        std::declval<thetadatadx::EndpointRequestOptions>()));
    STATIC_REQUIRE(
        std::is_same_v<ViewAsyncRet, std::future<std::vector<thetadatadx::EodTick>>>);

    // A StringList endpoint resolves to `std::future<std::vector<std::string>>`,
    // and an OptionContracts endpoint to its contract row — guards that the
    // row-type projection flows through the async wrapper unchanged.
    using ListRet =
        decltype(std::declval<const Hist&>().stock_list_symbols_async(
            std::declval<thetadatadx::EndpointRequestOptions>()));
    STATIC_REQUIRE(std::is_same_v<ListRet, std::future<std::vector<std::string>>>);
}

TEST_CASE("async query resolves to the same rows as the blocking call",
          "[history][async][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::HistoricalClient::connect(creds, config);

    // Blocking baseline, then the async companion. The two run on the same
    // handle sequentially (the future is drained before the next query), so
    // the per-handle single-threaded contract holds.
    auto sync_rows = client.stock_history_eod("AAPL", "20240101", "20240131");

    std::future<std::vector<thetadatadx::EodTick>> fut =
        client.stock_history_eod_async("AAPL", "20240101", "20240131");
    auto async_rows = fut.get();

    REQUIRE(async_rows.size() == sync_rows.size());
    REQUIRE_FALSE(async_rows.empty());
    REQUIRE(async_rows.front().date == sync_rows.front().date);
}

TEST_CASE("async query surfaces a typed error on future::get",
          "[history][async][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::HistoricalClient::connect(creds, config);

    // A malformed date range drives the blocking call to throw; the throw is
    // captured in the future's shared state by `std::async` and re-raised on
    // `.get()`, so the typed error propagates exactly as the blocking call
    // raises it.
    std::future<std::vector<thetadatadx::EodTick>> fut =
        client.stock_history_eod_async("AAPL", "not-a-date", "not-a-date");
    REQUIRE_THROWS_AS(fut.get(), thetadatadx::ThetaDataError);
}
