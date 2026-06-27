// Async historical query surface tests.
//
// Every buffered historical / snapshot query carries an `<endpoint>_async`
// companion returning a `std::future<std::vector<Row>>` so callers can run
// the request off the calling thread without managing their own threads.
// The offline half pins the static shape (return type + that the call
// compiles and a future is produced) without a network round-trip; the
// live half guards that `.get()` yields the same rows as the blocking call
// and that a typed error surfaces on `.get()`.

#include <atomic>
#include <cstdlib>
#include <future>
#include <memory>
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

namespace detail {

// Detects whether `<obj>.stock_list_symbols_async()` is well-formed for a given
// value category of the object. The `_async` companions co-own the FFI handle
// (the closure captures a copy of the object), so the future may safely outlive
// the object — the call is well-formed on BOTH an lvalue and an rvalue.
template <typename T, typename = void>
struct async_lvalue : std::false_type {};
template <typename T>
struct async_lvalue<T, std::void_t<decltype(std::declval<T&>().stock_list_symbols_async())>>
    : std::true_type {};

template <typename T, typename = void>
struct async_rvalue : std::false_type {};
template <typename T>
struct async_rvalue<T, std::void_t<decltype(std::declval<T&&>().stock_list_symbols_async())>>
    : std::true_type {};

template <typename T>
inline constexpr bool async_callable_on_lvalue = async_lvalue<T>::value;
template <typename T>
inline constexpr bool async_callable_on_rvalue = async_rvalue<T>::value;

} // namespace detail

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

    // Shared-ownership safety. The `_async` companions capture a copy of the
    // object (`self = *this`), which co-owns the FFI handle, so the future
    // keeps the handle alive on its own. The natural-looking
    // `client.historical().foo_async(...)` — where the `Historical` view is a
    // temporary — is therefore SOUND, not a use-after-free, so the call is
    // well-formed on both an lvalue and an rvalue object (no `&&`-deleted
    // overload). The runtime lifetime test below proves the handle outlives a
    // destroyed originating object.
    STATIC_REQUIRE(detail::async_callable_on_lvalue<View>);
    STATIC_REQUIRE(detail::async_callable_on_rvalue<View>);
    // Same on the dedicated historical client.
    STATIC_REQUIRE(detail::async_callable_on_lvalue<Hist>);
    STATIC_REQUIRE(detail::async_callable_on_rvalue<Hist>);
}

// ── Lifetime regression: a future may outlive the object it was launched from ──
//
// The real `HistoricalClient` / `Historical` can only be constructed by a live
// `connect()` (private ctor + gRPC handshake), so a true destroy-mid-flight
// against a live FFI handle is a `[live]` scenario (last test below). This
// offline case proves the load-bearing mechanism the generated `_async`
// companions now rely on, with NO network: an owner that holds its resource by
// `shared_ptr` and whose `_async`-shaped method captures a COPY of itself
// (`self = *this`) keeps the resource alive for the future's whole lifetime,
// even when the originating object is destroyed while the future is still
// pending. A regression to capturing a raw `this` (or a non-owning copy) would
// free the resource at the originator's destruction and fail these assertions
// (and trip ASan on the dangling access inside the task).
namespace {

// Free-counting resource standing in for the FFI client handle.
struct Resource {
    int value;
};

// Mirrors the generated owner: holds the handle by `shared_ptr` (single-free
// deleter) and exposes a `_async`-shaped method that captures a copy of itself.
struct OwningStub {
    std::shared_ptr<Resource> handle;

    explicit OwningStub(std::atomic<int>* free_counter)
        : handle(new Resource{42}, [free_counter](Resource* r) {
              free_counter->fetch_add(1, std::memory_order_relaxed);
              delete r;
          }) {}

    // The blocking body the future runs — touches the co-owned resource.
    int read() const { return handle->value; }

    // `_async` shape: capture a copy of `*this`, gated on `start` so the caller
    // can destroy the originator before the body runs (destroy-mid-flight).
    std::future<int> read_async(std::shared_future<void> start) const {
        return std::async(std::launch::async, [self = *this, start]() {
            start.wait();           // block until the originator is destroyed
            return self.read();     // co-owned resource still alive
        });
    }
};

} // namespace

TEST_CASE("async future outlives the destroyed originating object",
          "[history][async][offline]") {
    std::atomic<int> frees{0};
    std::promise<void> gate;
    std::shared_future<void> start = gate.get_future().share();

    std::future<int> fut;
    {
        OwningStub owner(&frees);
        fut = owner.read_async(start);
        // Owner (and its shared_ptr reference) drops here, while the task is
        // parked on `start` — i.e. the future is still pending. The resource
        // must NOT be freed yet: the captured copy inside the task co-owns it.
    }
    REQUIRE(frees.load() == 0);

    gate.set_value();           // release the task; it reads the co-owned resource
    REQUIRE(fut.get() == 42);   // correct result, no use-after-free

    fut = {};                   // drop the last owner (the future's captured copy)
    REQUIRE(frees.load() == 1); // freed exactly once, only now
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

TEST_CASE("async query from a destroyed Historical view resolves safely",
          "[history][async][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::Client::connect(creds, config);

    // Launch the async query directly on the temporary `client.historical()`
    // view — the view is destroyed at the end of the full expression, while the
    // detached task is still in flight. With shared handle ownership the task's
    // captured copy keeps the handle alive, so `.get()` returns the correct
    // rows instead of dereferencing a freed handle. (Pre-fix this was a
    // deliberately-deleted `&&` overload; the UAF it guarded is now gone.)
    std::future<std::vector<thetadatadx::EodTick>> fut =
        client.historical().stock_history_eod_async("AAPL", "20240101", "20240131");
    auto rows = fut.get();
    REQUIRE_FALSE(rows.empty());
}
