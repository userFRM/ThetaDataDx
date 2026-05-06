/**
 * thetadatadx C++ SDK.
 *
 * RAII wrappers around the C FFI layer. Provides idiomatic C++ access to
 * ThetaData market data with automatic resource management.
 *
 * Tick data is returned directly as #[repr(C)] structs — no JSON parsing.
 * The C++ tick types are layout-compatible with the Rust originals.
 */

#ifndef THETADX_HPP
#define THETADX_HPP

#include "thetadx.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <vector>
#include <utility>
#include <stdexcept>

namespace tdx {

// ── Tick types (re-exported from thetadx.h for C++ convenience) ──
// These are typedef aliases to the C types defined in thetadx.h.
// They are #[repr(C)] layout-compatible with the Rust originals.

using EodTick = TdxEodTick;
using OhlcTick = TdxOhlcTick;
using TradeTick = TdxTradeTick;
using QuoteTick = TdxQuoteTick;
using GreeksAllTick = TdxGreeksAllTick;
using GreeksFirstOrderTick = TdxGreeksFirstOrderTick;
using GreeksSecondOrderTick = TdxGreeksSecondOrderTick;
using GreeksThirdOrderTick = TdxGreeksThirdOrderTick;
using IvTick = TdxIvTick;
using PriceTick = TdxPriceTick;
using OpenInterestTick = TdxOpenInterestTick;
using MarketValueTick = TdxMarketValueTick;
using CalendarDay = TdxCalendarDay;
using InterestRateTick = TdxInterestRateTick;
using TradeQuoteTick = TdxTradeQuoteTick;

// Generated layout guards for the C mirror tick structs.
#include "tick_layout_asserts.hpp.inc"

// ── FPSS event struct layout guards ──
//
// Field-level offsetof guards. These would have caught the pre-#??? bug
// where the hand-written C++ `TdxFpssEvent` ordered its Data fields as
// { quote, trade, open_interest, ohlcvc } while the Rust FFI emitted
// { ohlcvc, open_interest, quote, trade } — every `event->quote.*` read
// in the C++ SDK dereferenced memory belonging to a different struct.
// The generator now owns both the Go and C++ C header, so field order
// comes straight from `fpss_event_schema.toml`; these asserts catch any
// ABI-level drift (padding, alignment, scalar widths) the schema alone
// cannot express.

// Every data variant gained an embedded `TdxContract contract` field
// immediately after `contract_id`. On LP64 (x86_64 / aarch64 Linux,
// macOS), `TdxContract` is 32 bytes {
//   const char *root         offset  0, size 8
//   int32_t sec_type         offset  8, size 4
//   bool has_exp_date        offset 12, size 1
//   int32_t exp_date         offset 16, size 4 (3 bytes pad after has_exp_date)
//   bool has_is_call         offset 20, size 1
//   bool is_call             offset 21, size 1
//   bool has_strike          offset 22, size 1
//   int32_t strike           offset 24, size 4 (1 byte tail pad)
// }
// so every field below the embedded `contract` shifted by +32 vs the
// pre-contract layout. Numbers below recomputed from the exact struct
// under `-O2` with the generated `fpss_event_structs.h.inc` types on
// an LP64 host; CI re-validates the asserts on every build.

// Generated layout guards for the FPSS event C mirror structs.
#include "fpss_layout_asserts.hpp.inc"

// OptionContract uses std::string for symbol to avoid use-after-free.
// The C FFI TdxOptionContract uses a raw char* that is freed with the array,
// so we deep-copy the string during conversion.
struct OptionContract {
    std::string symbol;
    int32_t expiration;
    double strike;
    int32_t right;
};

/// Active FPSS subscription descriptor.
struct Subscription {
    std::string kind;
    std::string contract;
};

// ── Greeks result (from standalone tdx_all_greeks) ──

struct Greeks {
    double value;
    double delta;
    double gamma;
    double theta;
    double vega;
    double rho;
    double iv;
    double iv_error;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double vera;
    double speed;
    double zomma;
    double color;
    double ultima;
    double d1;
    double d2;
    double dual_delta;
    double dual_gamma;
    double epsilon;
    double lambda;
};

/* Generated in endpoint_options.hpp.inc. */
#include "endpoint_options.hpp.inc"

// ── RAII typed array wrappers ──

namespace detail {

static std::string last_ffi_error() {
    const char* err = tdx_last_error();
    return err ? std::string(err) : "unknown error";
}

// Raw variant: returns "" when the FFI error slot is empty. Used by
// post-call disambiguation in check_array helpers — distinguishes
// success-empty from failure-empty (e.g. timeout on a list endpoint
// returns the same `{nullptr, 0}` sentinel as a successful empty result).
// Generated `_with_options` callers MUST `tdx_clear_error()` before
// invoking the FFI so a stale error from a prior call isn't picked up.
static std::string last_ffi_error_raw() {
    const char* err = tdx_last_error();
    return err ? std::string(err) : std::string();
}

template<typename T>
std::vector<T> to_vector(const T* data, size_t len) {
    if (data == nullptr || len == 0) return {};
    return std::vector<T>(data, data + len);
}

inline std::vector<std::string> string_array_to_vector(TdxStringArray arr) {
    std::vector<std::string> result;
    if (arr.data != nullptr && arr.len > 0) {
        result.reserve(arr.len);
        for (size_t i = 0; i < arr.len; ++i) {
            result.emplace_back(arr.data[i] ? arr.data[i] : "");
        }
    }
    tdx_string_array_free(arr);
    return result;
}

// Convert a TdxStringArray to vector<string>, throwing on FFI error.
//
// Empty array is ambiguous: success-with-zero-results AND failure (e.g.
// timeout on a list endpoint) both return `{nullptr, 0}`. Disambiguate by
// reading `tdx_last_error_raw` after the call. Generated wrappers
// `tdx_clear_error()` before the FFI call so a stale error from a prior
// call isn't misattributed.
inline std::vector<std::string> check_string_array(TdxStringArray arr) {
    const std::string err = last_ffi_error_raw();
    if (!err.empty()) {
        tdx_string_array_free(arr);
        throw std::runtime_error("thetadatadx: " + err);
    }
    return string_array_to_vector(arr);
}

// Convert a typed tick array to vector<T> by passing in the converter and
// the FFI-array free fn. Throws on FFI error so callers don't mistake a
// timed-out tick endpoint for "no rows". Same contract as
// check_string_array — `tdx_clear_error()` MUST have been called before
// the FFI invocation.
template<typename T, typename Arr, typename Convert, typename Free>
std::vector<T> check_tick_array(Arr arr, Convert convert, Free free_fn) {
    const std::string err = last_ffi_error_raw();
    if (!err.empty()) {
        free_fn(arr);
        throw std::runtime_error("thetadatadx: " + err);
    }
    auto result = convert(arr);
    free_fn(arr);
    return result;
}

inline std::vector<Subscription> subscription_array_to_vector(TdxSubscriptionArray* arr) {
    if (arr == nullptr) {
        throw std::runtime_error("thetadatadx: " + last_ffi_error());
    }

    std::vector<Subscription> result;
    if (arr->data != nullptr && arr->len > 0) {
        result.reserve(arr->len);
        for (size_t i = 0; i < arr->len; ++i) {
            result.push_back(Subscription{
                arr->data[i].kind ? std::string(arr->data[i].kind) : "",
                arr->data[i].contract ? std::string(arr->data[i].contract) : "",
            });
        }
    }
    tdx_subscription_array_free(arr);
    return result;
}

/// Managed C string from FFI: auto-frees on destruction.
struct FfiString {
    char* ptr;
    FfiString(char* p) : ptr(p) {}
    ~FfiString() { if (ptr) tdx_string_free(ptr); }
    FfiString(const FfiString&) = delete;
    FfiString& operator=(const FfiString&) = delete;

    std::string str() const { return ptr ? std::string(ptr) : ""; }
    bool ok() const { return ptr != nullptr; }
};

} // namespace detail

// ── RAII deleters ──

struct CredentialsDeleter {
    void operator()(TdxCredentials* p) const { if (p) tdx_credentials_free(p); }
};

struct ConfigDeleter {
    void operator()(TdxConfig* p) const { if (p) tdx_config_free(p); }
};

struct ClientDeleter {
    void operator()(TdxClient* p) const { if (p) tdx_client_free(p); }
};

struct FpssHandleDeleter {
    void operator()(TdxFpssHandle* p) const { if (p) tdx_fpss_free(p); }
};

// ── Credentials ──

class Credentials {
public:
    /** Load credentials from a file (line 1 = email, line 2 = password). */
    static Credentials from_file(const std::string& path);

    /** Create credentials from email and password. */
    static Credentials from_email(const std::string& email, const std::string& password);

    /** Get the raw handle (for passing to Client::connect). */
    TdxCredentials* get() const { return handle_.get(); }

private:
    explicit Credentials(TdxCredentials* h) : handle_(h) {}
    std::unique_ptr<TdxCredentials, CredentialsDeleter> handle_;
};

// ── Config ──

class Config {
public:
    /** Production config (ThetaData NJ datacenter). */
    static Config production();

    /** Dev FPSS config (port 20200, infinite historical replay). */
    static Config dev();

    /** Stage FPSS config (port 20100, testing, unstable). */
    static Config stage();

    /** Set FPSS reconnect policy. 0=Auto (default), 1=Manual. */
    void set_reconnect_policy(int policy) { tdx_config_set_reconnect_policy(handle_.get(), policy); }

    /** Set FPSS flush mode. 0=Batched (default), 1=Immediate. */
    void set_flush_mode(int mode) { tdx_config_set_flush_mode(handle_.get(), mode); }

    /** Set whether to derive OHLCVC bars locally from trades. */
    void set_derive_ohlcvc(bool enabled) { tdx_config_set_derive_ohlcvc(handle_.get(), enabled ? 1 : 0); }

    /** Get the raw handle. */
    TdxConfig* get() const { return handle_.get(); }

private:
    explicit Config(TdxConfig* h) : handle_(h) {}
    std::unique_ptr<TdxConfig, ConfigDeleter> handle_;
};

// ── Client ──

class Client {
public:
    /** Connect to ThetaData servers. Throws on failure. */
    static Client connect(const Credentials& creds, const Config& config);

    #include "historical.hpp.inc"
private:
    explicit Client(TdxClient* h) : handle_(h) {}
    std::unique_ptr<TdxClient, ClientDeleter> handle_;
};

// ── FPSS event types (re-exported from thetadx.h) ──

using FpssEventKind = TdxFpssEventKind;
using FpssQuote = TdxFpssQuote;
using FpssTrade = TdxFpssTrade;
using FpssOpenInterest = TdxFpssOpenInterest;
using FpssOhlcvc = TdxFpssOhlcvc;
using FpssControl = TdxFpssControl;
using FpssRawData = TdxFpssRawData;
using FpssEvent = TdxFpssEvent;

// ── FPSS real-time streaming client ──
//
// Event delivery is callback-driven. Two paths are available:
//
// * `set_callback(fn)` — default queued path. Events flow
//   `FPSS reader -> bounded(8192) crossbeam queue -> dispatcher drain
//   thread -> user fn`. The reader thread never blocks on user code; on
//   queue overflow events are dropped and counted via `dropped_events()`.
//
// * `set_inline_callback(fn)` — power-user opt-in. `fn` fires directly
//   from the FPSS reader thread. The caller MUST guarantee `fn` returns
//   within microseconds; a slow `fn` blocks the reader, fills the kernel
//   TCP receive buffer, and causes the vendor to disconnect.
//
// The Client owns the `std::function`. A free `extern "C"` shim retrieves
// the stored function from the registered `void* ctx` and invokes it with
// the event reference. The shim converts `const TdxFpssEvent*` (the C ABI
// payload type) to `const FpssEvent&` (the C++ alias) at the boundary.
// Callback storage outlives any FPSS reader / dispatcher thread because
// `~FpssClient` calls `tdx_fpss_shutdown` before the storage is freed.

class FpssClient {
public:
    #include "fpss.hpp.inc"
    ~FpssClient();

    FpssClient(const FpssClient&) = delete;
    FpssClient& operator=(const FpssClient&) = delete;
    FpssClient(FpssClient&& other) noexcept
        : handle_(std::move(other.handle_)),
          callback_(std::move(other.callback_)) {}
    FpssClient& operator=(FpssClient&& other) noexcept {
        handle_ = std::move(other.handle_);
        callback_ = std::move(other.callback_);
        return *this;
    }

    /** Register a queued FPSS callback and open the FPSS connection.
     *  `fn` runs on the dispatcher drain thread, never on the FPSS
     *  reader. The reader thread cannot be blocked by user code: on
     *  overflow events are dropped and counted via `dropped_events()`.
     *  Throws on registration failure. */
    void set_callback(std::function<void(const FpssEvent&)> fn) {
        callback_ = std::make_unique<std::function<void(const FpssEvent&)>>(std::move(fn));
        int rc = tdx_fpss_set_callback(handle_.get(), &FpssClient::callback_shim, callback_.get());
        if (rc < 0) {
            callback_.reset();
            throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
        }
    }

    /** Register an inline FPSS callback and open the FPSS connection.
     *  `fn` fires directly from the FPSS reader thread. Caller MUST
     *  guarantee `fn` returns within microseconds; a slow `fn` stalls
     *  the reader and the vendor will drop the session. Throws on
     *  registration failure. */
    void set_inline_callback(std::function<void(const FpssEvent&)> fn) {
        callback_ = std::make_unique<std::function<void(const FpssEvent&)>>(std::move(fn));
        int rc = tdx_fpss_set_inline_callback(handle_.get(), &FpssClient::callback_shim, callback_.get());
        if (rc < 0) {
            callback_.reset();
            throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
        }
    }

    /** Cumulative count of FPSS events dropped by the streaming dispatcher
     *  because the bounded(8192) queue was full when the FPSS reader
     *  thread tried to enqueue. Returns 0 when no callback has been
     *  installed or when the inline path was taken (no queue). Safe to
     *  call on a moved-from client. */
    uint64_t dropped_events() const {
        return handle_ ? tdx_fpss_dropped_events(handle_.get()) : 0;
    }

private:
    // Free C-ABI shim that the Rust dispatcher invokes. `ctx` is the
    // `std::function*` we registered alongside the callback. The event
    // pointer is non-null and valid only for the duration of this call.
    static void callback_shim(const TdxFpssEvent* event, void* ctx) noexcept {
        auto* fn = static_cast<std::function<void(const FpssEvent&)>*>(ctx);
        if (fn == nullptr || event == nullptr) return;
        try {
            (*fn)(*event);
        } catch (...) {
            // User callbacks must not propagate exceptions across the
            // C ABI boundary — Rust would unwind into UB. Swallow.
        }
    }

    std::unique_ptr<TdxFpssHandle, FpssHandleDeleter> handle_;
    // `unique_ptr` so the address handed to the C ABI as `ctx` is stable
    // across moves of the owning `FpssClient`.
    std::unique_ptr<std::function<void(const FpssEvent&)>> callback_;
};

// ── Standalone Greeks functions ──

#include "utilities.hpp.inc"

} // namespace tdx

#endif /* THETADX_HPP */
