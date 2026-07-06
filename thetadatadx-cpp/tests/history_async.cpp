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
#include <chrono>
#include <cstdlib>
#include <functional>
#include <future>
#include <memory>
#include <string>
#include <thread>
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
    using Hist = thetadatadx::MarketDataClient;

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
    // `client.market_data().foo_async(...)` — where the `Historical` view is a
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
// The real `MarketDataClient` / `Historical` can only be constructed by a live
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

// ── Lifetime regression: unified client destroyed mid-future WHILE streaming ──
//
// This pins the exact structural invariant the unified `Client` documents: an
// entity that can become the last owner of the FFI handle must ALSO co-own the
// callback state, so the streaming-stop drain barrier the handle's deleter runs
// always sees a live registered node. The real `Client` can only be built by a
// live `connect()`, so the end-to-end version is a `[live]` test; this offline
// case proves the mechanism with no network, deterministically, under ASan.
//
// The hazard, modelled exactly: a pending `historical().<endpoint>_async`
// future captures a copy of the `Historical` view, co-owning BOTH the handle
// and the callback node. The view is the LAST owner of both, so dropping the
// future destroys the view's members in REVERSE declaration order — and that
// order is the whole bug. The view must declare `callback_` FIRST and `handle_`
// SECOND (as the shipped `Client` does): then `handle_` is released first, its
// deleter stops streaming and JOINS the consumer (the drain barrier), and only
// after that returns is `callback_` (the node the consumer reads) released. A
// view that declares `handle_` first instead releases the callback node BEFORE
// the deleter's stop+join — so the still-live consumer fires through freed node
// storage: a heap-use-after-free. The reproduction is exactly that swap: flip
// `MarketDataViewStub`'s two members to handle-first below and ASan reports
// heap-use-after-free; the shipped callback-first order is clean. The consumer
// spins in a TIGHT loop with no sleep so the read lands inside the narrow
// free-node / stop-join window every run.
namespace {

// Registered callback node — stands in for `CallbackSlot`/`CallbackState`. The
// consumer thread reads `value` through the co-owned pointer on every tick; a
// premature free turns that read into a heap-use-after-free.
struct CallbackNodeStub {
    int value = 7;
};

// Stands in for the unified FFI handle. The deleter mirrors
// `thetadatadx_client_free`: stopping streaming and JOINING the consumer
// (the drain barrier) before the handle storage goes away. Because the handle
// is held by `shared_ptr`, this runs only when the LAST owner drops — which,
// for a pending future, is AFTER `~ClientStub`.
struct HandleStub {
    std::atomic<bool>* stop;
    std::thread* consumer;
    std::atomic<int>* frees;

    HandleStub(std::atomic<bool>* s, std::thread* c, std::atomic<int>* f)
        : stop(s), consumer(c), frees(f) {}
    // Single-free: the deleter must run exactly once, on the one heap object.
    // Deleting copy/move makes an accidental temporary copy (which would run
    // the stop+join deleter early and corrupt the test) a compile error.
    HandleStub(const HandleStub&) = delete;
    HandleStub& operator=(const HandleStub&) = delete;

    ~HandleStub() {
        stop->store(true, std::memory_order_release); // stop streaming
        if (consumer->joinable()) consumer->join();   // drain barrier
        frees->fetch_add(1, std::memory_order_relaxed);
    }
};

// Mirrors the unified `Client`: co-owns the handle AND the callback node, in
// the same reverse-destruct order (callback declared first → destroyed last).
// `historical()` hands out a view that co-owns BOTH, exactly as the shipped
// `Client::market_data()` does.
struct ClientStub {
    // Declaration order mirrors `Client`: callback_ first so it is destroyed
    // AFTER handle_ (whose deleter stops + drains the consumer).
    std::shared_ptr<CallbackNodeStub> callback_;
    std::shared_ptr<HandleStub> handle_;

    // The view the async future captures a copy of. Co-owns both members.
    // Member order mirrors the shipped `Historical`/`Stream` views (and
    // `ClientStub` above): `callback_` declared FIRST so reverse-destruct
    // releases `handle_` first — its stop+join deleter runs while the node is
    // still alive. Flipping these two to handle-first is the regression this
    // case gates: it frees the node before the deleter joins the consumer.
    struct MarketDataViewStub {
        std::shared_ptr<CallbackNodeStub> callback_;
        std::shared_ptr<HandleStub> handle_;

        // `_async` shape: capture a copy of the view (`self = *this`), gated on
        // `start` so the caller can destroy the client before the body reads
        // the co-owned node — the destroy-mid-flight window.
        std::future<int> read_async(std::shared_future<void> start) const {
            return std::async(std::launch::async, [self = *this, start]() {
                start.wait();
                return self.callback_->value; // co-owned node still alive
            });
        }
    };

    MarketDataViewStub historical() const {
        // Share BOTH, like the fix. Positional init follows the member
        // declaration order: callback_ first, then handle_.
        return MarketDataViewStub{callback_, handle_};
    }
};

} // namespace

TEST_CASE("async future from a unified client outlives destruction while streaming",
          "[history][async][offline]") {
    // Repeat the whole destroy-while-streaming cycle many times so a rare
    // member-order UAF is caught reliably under ASan rather than slipping past
    // on a lucky interleaving. With the shipped callback-first order every
    // iteration is clean; flip `MarketDataViewStub` to handle-first and ASan
    // reports heap-use-after-free within the first handful.
    constexpr int kIterations = 5000;
    for (int i = 0; i < kIterations; ++i) {
        std::atomic<int> frees{0};
        std::atomic<bool> stop{false};
        std::promise<void> gate;
        std::shared_future<void> start = gate.get_future().share();

        std::future<int> fut;
        std::thread consumer;
        {
            auto callback = std::make_shared<CallbackNodeStub>();

            // Live "streaming" consumer: fires through the registered node in a
            // TIGHT loop (no sleep) until streaming is stopped. Captures the raw
            // node pointer it was registered with (the real consumer holds
            // `&slot->fn`), so a freed node is a UAF. The unbroken spin keeps a
            // read landing inside the narrow free-node / stop-join window.
            const CallbackNodeStub* registered = callback.get();
            consumer = std::thread([registered, &stop]() {
                volatile int sink = 0;
                while (!stop.load(std::memory_order_acquire)) {
                    sink += registered->value; // UAF here if the node is freed early
                }
                (void)sink;
            });

            ClientStub client{
                callback,
                std::make_shared<HandleStub>(&stop, &consumer, &frees),
            };

            // Stored future launched off the view — co-owns handle + callback.
            fut = client.market_data().read_async(start);

            // `client` is destroyed HERE while the future is pending and the
            // consumer is still firing. Its members drop in reverse order
            // (callback_ declared first → handle_ first), but the future co-owns
            // both, so neither deleter runs yet.
        }

        // The handle's deleter (stop + join) has NOT run yet: the future still
        // co-owns the handle, so streaming is still live and the consumer fires.
        REQUIRE(frees.load() == 0);

        gate.set_value();          // release the future body; it reads the node
        REQUIRE(fut.get() == 7);   // correct value, no use-after-free

        // Drop the last owner (the future's captured view). Reverse-destruct
        // releases handle_ FIRST — its deleter stops streaming and joins the
        // consumer (drain barrier) — THEN releases callback_, the node the now
        // -joined consumer was reading. Handle-first member order would free the
        // node before the join, firing the live consumer through freed storage.
        fut = {};
        REQUIRE(frees.load() == 1);
        // Consumer was joined inside the deleter; nothing left to clean up.
        REQUIRE_FALSE(consumer.joinable());
    }
}

TEST_CASE("async query resolves to the same rows as the blocking call",
          "[history][async][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    auto client = thetadatadx::MarketDataClient::connect(creds, config);

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
    auto client = thetadatadx::MarketDataClient::connect(creds, config);

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

    // Launch the async query directly on the temporary `client.market_data()`
    // view — the view is destroyed at the end of the full expression, while the
    // detached task is still in flight. With shared handle ownership the task's
    // captured copy keeps the handle alive, so `.get()` returns the correct
    // rows instead of dereferencing a freed handle. (Pre-fix this was a
    // deliberately-deleted `&&` overload; the UAF it guarded is now gone.)
    std::future<std::vector<thetadatadx::EodTick>> fut =
        client.market_data().stock_history_eod_async("AAPL", "20240101", "20240131");
    auto rows = fut.get();
    REQUIRE_FALSE(rows.empty());
}
