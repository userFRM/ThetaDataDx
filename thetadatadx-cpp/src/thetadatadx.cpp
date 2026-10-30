/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadatadx.hpp"

#include <utility>

namespace thetadatadx {

namespace detail {

// Borrow `const char*` views into a stable vector<string> for C FFI calls.
static std::vector<const char*> string_ptrs(const std::vector<std::string>& items) {
    std::vector<const char*> ptrs;
    ptrs.reserve(items.size());
    for (const auto& item : items) {
        ptrs.push_back(item.c_str());
    }
    return ptrs;
}

} // namespace detail

// ── Credentials / Config / Client lifecycle ──

Credentials Credentials::from_file(const std::string& path) {
    auto h = thetadatadx_credentials_from_file(path.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_email(const std::string& email, const std::string& password) {
    auto h = thetadatadx_credentials_from_email(email.c_str(), password.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_api_key(const std::string& api_key) {
    auto h = thetadatadx_credentials_from_api_key(api_key.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_api_key_with_email(const std::string& email, const std::string& api_key) {
    auto h = thetadatadx_credentials_from_api_key_with_email(email.c_str(), api_key.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_env() {
    auto h = thetadatadx_credentials_from_env();
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_env_or_file(const std::string& path) {
    auto h = thetadatadx_credentials_from_env_or_file(path.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Credentials Credentials::from_dotenv(const std::string& path) {
    auto h = thetadatadx_credentials_from_dotenv(path.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Credentials(h);
}

Config Config::production() { return Config(thetadatadx_config_production()); }

Config Config::dev() { return Config(thetadatadx_config_dev()); }

Config Config::stage() { return Config(thetadatadx_config_stage()); }

Config Config::from_dotenv(const std::string& path) {
    auto h = thetadatadx_config_from_dotenv(path.c_str());
    if (!h) detail::throw_last_ffi_error();
    return Config(h);
}

MarketDataClient MarketDataClient::connect(const Credentials& creds, const Config& config) {
    auto h = thetadatadx_market_data_connect(creds.get(), config.get());
    if (!h) detail::throw_last_ffi_error();
    return MarketDataClient(h);
}

MarketDataClient MarketDataClient::from_file(const std::string& path, const Config& config) {
    auto h = thetadatadx_market_data_connect_from_file(path.c_str(), config.get());
    if (!h) detail::throw_last_ffi_error();
    return MarketDataClient(h);
}

// ═══════════════════════════════════════════════════════════════
//  Market-data endpoints (generated)
// ═══════════════════════════════════════════════════════════════

#include "market_data.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  Market-data server-stream endpoints (generated)
// ═══════════════════════════════════════════════════════════════

#include "market_data_stream.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  Streaming client
// ═══════════════════════════════════════════════════════════════

std::vector<Subscription> StreamingClient::active_subscriptions() const {
    return detail::subscription_array_to_vector(thetadatadx_streaming_active_subscriptions(handle_.get()));
}

void StreamingClient::reconnect() {
    int rc = thetadatadx_streaming_reconnect(handle_.get());
    if (rc < 0) detail::throw_last_ffi_error();
}

void StreamingClient::shutdown() { thetadatadx_streaming_shutdown(handle_.get()); }

bool StreamingClient::is_authenticated() const { return thetadatadx_streaming_is_authenticated(handle_.get()) != 0; }

StreamingClient::StreamingClient(const Credentials& creds, const Config& config) {
    auto h = thetadatadx_streaming_connect(creds.get(), config.get());
    if (!h) detail::throw_last_ffi_error();
    handle_.reset(h);
}

StreamingClient StreamingClient::from_file(const std::string& path, const Config& config) {
    return StreamingClient(Credentials::from_file(path), config);
}

// Lifecycle: intentionally hand-written (C++ destructor semantics with unique_ptr).
//
// Member destruction order (REVERSE declaration order): `handle_` first,
// `callback_` second. The `handle_` deleter calls `thetadatadx_streaming_free` which
// performs an internal 5 s drain barrier — see the ordering invariant
// comment above the member declarations in the header.
//
// The body raises the shutdown signal early so the consumer thread starts
// quiescing before the deleter polls the drain flag. On drain timeout (rare,
// a wedged user callback) the consumer may still be firing through
// `callback_`'s storage, so we MUST NOT let the storage drop synchronously.
// We mirror `StreamingClient::operator=`'s rescue path exactly: hand BOTH the
// retired handle and callback storage to a reclaimer that polls the same
// drain barrier and releases them (handle first, then storage) only once the
// consumer is confirmed quiesced — never a guessed wall-clock window. If the
// callback is still wedged after `kReclaimQuiescenceCap`, the reclaimer leaks
// the storage rather than destroy a `std::function` under a live invocation
// (a bounded leak beats a use-after-free); see `reclaim_after_drain`.
StreamingClient::~StreamingClient() {
    if (handle_) {
        thetadatadx_streaming_shutdown(handle_.get());
        int drained = thetadatadx_streaming_await_drain(handle_.get(), 5000);
        // Only rescue the retired session when a callback was actually
        // installed: `callback_` is non-null only after a successful
        // `set_callback`, which is the sole path that starts the consumer
        // thread and wires a live `ctx`. With no callback there is no consumer
        // to outrun, and the members can destruct synchronously.
        if (drained == 0 && callback_) {
            const ThetaDataDxStreamHandle* raw = handle_.get();
            detail::reclaim_after_drain(
                [raw]() {
                    return thetadatadx_streaming_await_drain(
                               raw,
                               static_cast<uint64_t>(
                                   detail::kReclaimPollStep.count())) == 1;
                },
                [retired_handle = std::move(handle_),
                 retired_cb = std::move(callback_)]() mutable {
                    // Free the handle first so its internal drain barrier
                    // still observes a live `ctx`, then drop the storage.
                    // Mirrors the handle-before-callback member invariant.
                    retired_handle.reset();
                    retired_cb.reset();
                });
            // `handle_` and `callback_` are now empty; the reclaimer owns
            // the retired session.
        }
    }
}

} // namespace thetadatadx
