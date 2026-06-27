/**
 * thetadatadx C++ SDK.
 *
 * RAII wrappers around the C FFI layer. Provides idiomatic C++ access to
 * ThetaData market data with automatic resource management.
 *
 * Tick data is returned directly as fixed-layout structs — no JSON parsing.
 * The C++ tick types are layout-compatible with the C ABI structs.
 */

#ifndef THETADATADX_HPP
#define THETADATADX_HPP

#include "thetadatadx.h"

#include <chrono>
#include <cctype>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <functional>
#include <future>
#include <memory>
#include <optional>
#include <ostream>
#include <sstream>
#include <string>
#include <thread>
#include <vector>
#include <utility>
#include <stdexcept>
#include <type_traits>

#if defined(__cpp_lib_span) && __cpp_lib_span >= 202002L
#include <span>
#endif

// The pull-based Arrow `RecordBatch` reader (`Stream::batches(..)`) returns a
// native `arrow::RecordBatchReader`, so it requires arrow-cpp. Gate it behind
// `THETADATADX_CPP_ARROW` (set by the CMake `THETADATADX_CPP_ARROW` option) so
// the rest of the SDK still builds for users who do not link arrow-cpp — the
// per-tick `*_to_arrow_ipc` terminals already hand back raw IPC bytes for that
// audience.
#ifdef THETADATADX_CPP_ARROW
#include <cstring>
#include <arrow/array.h>
#include <arrow/buffer.h>
#include <arrow/io/memory.h>
#include <arrow/ipc/reader.h>
#include <arrow/record_batch.h>
#include <arrow/result.h>
#include <arrow/status.h>
#include <arrow/type.h>
#endif

namespace thetadatadx {

// ── Chunk view for the server-stream callbacks ──
//
// `Span<const Tick>` is the borrowed, contiguous view the `_stream` methods
// hand each decoded chunk through. Under C++20 it is exactly `std::span`; on
// a C++17 toolchain (the SDK's baseline standard) it degrades to a minimal
// pointer + length view with the same `data()` / `size()` / iteration surface,
// so streaming callers compile unchanged on either standard. The view never
// owns the rows — they belong to the decoder and are freed before the next
// chunk, so a caller that needs rows beyond the callback must copy them out.
#if defined(__cpp_lib_span) && __cpp_lib_span >= 202002L
template <typename T>
using Span = std::span<T>;
#else
/// C++17 fallback for `std::span`: a non-owning pointer + length view over a
/// contiguous run of `T`, exposing the `data()` / `size()` / iteration subset
/// the streaming callbacks rely on.
template <typename T>
class Span {
public:
    using element_type = T;
    using value_type = std::remove_cv_t<T>;
    using size_type = std::size_t;
    using pointer = T*;
    using reference = T&;
    using iterator = T*;
    using const_iterator = const T*;

    constexpr Span() noexcept : data_(nullptr), size_(0) {}
    constexpr Span(T* data, std::size_t size) noexcept : data_(data), size_(size) {}

    constexpr T* data() const noexcept { return data_; }
    constexpr std::size_t size() const noexcept { return size_; }
    constexpr bool empty() const noexcept { return size_ == 0; }

    constexpr T& operator[](std::size_t i) const noexcept { return data_[i]; }
    constexpr T* begin() const noexcept { return data_; }
    constexpr T* end() const noexcept { return data_ + size_; }

private:
    T* data_;
    std::size_t size_;
};
#endif

// ── Tick types (re-exported from thetadatadx.h for C++ convenience) ──
// These are typedef aliases to the C types defined in thetadatadx.h.
// They are fixed-layout and ABI-compatible with those C structs.

using EodTick = ThetaDataDxEodTick;
using OhlcTick = ThetaDataDxOhlcTick;
using TradeTick = ThetaDataDxTradeTick;
using QuoteTick = ThetaDataDxQuoteTick;
using GreeksAllTick = ThetaDataDxGreeksAllTick;
using GreeksEodTick = ThetaDataDxGreeksEodTick;
using GreeksFirstOrderTick = ThetaDataDxGreeksFirstOrderTick;
using GreeksSecondOrderTick = ThetaDataDxGreeksSecondOrderTick;
using GreeksThirdOrderTick = ThetaDataDxGreeksThirdOrderTick;
using TradeGreeksAllTick = ThetaDataDxTradeGreeksAllTick;
using TradeGreeksFirstOrderTick = ThetaDataDxTradeGreeksFirstOrderTick;
using TradeGreeksSecondOrderTick = ThetaDataDxTradeGreeksSecondOrderTick;
using TradeGreeksThirdOrderTick = ThetaDataDxTradeGreeksThirdOrderTick;
using TradeGreeksImpliedVolatilityTick = ThetaDataDxTradeGreeksImpliedVolatilityTick;
using IvTick = ThetaDataDxIvTick;
using PriceTick = ThetaDataDxPriceTick;
using IndexPriceAtTimeTick = ThetaDataDxIndexPriceAtTimeTick;
using OpenInterestTick = ThetaDataDxOpenInterestTick;
using MarketValueTick = ThetaDataDxMarketValueTick;
using CalendarDay = ThetaDataDxCalendarDay;
using InterestRateTick = ThetaDataDxInterestRateTick;
using TradeQuoteTick = ThetaDataDxTradeQuoteTick;

// Generated layout guards for the C mirror tick structs.
#include "tick_layout_asserts.hpp.inc"

// Generated boolean flag-word accessors (`thetadatadx::is_cancelled(...)`, ...)
// decoded from the integer condition / flag columns. Free functions
// because the tick types above are C struct aliases with no member
// methods; mirrors the Python computed properties and TypeScript
// precomputed fields from the same schema rows.
#include "tick_flag_accessors.hpp.inc"

// ── streaming event struct layout guards ──
//
// Field-level offsetof guards. The `ThetaDataDxStreamEvent` data-variant field
// order is generated from `fpss_event_schema.toml` — the same schema
// every binding is emitted from — so the C++ consumer and the data
// producer agree on member order by construction rather than by
// hand-kept convention. These asserts catch any ABI-level drift
// (padding, alignment, scalar widths) the schema alone cannot express.

// Every data variant carries an embedded `ThetaDataDxContract contract` as
// the first member. On LP64 (x86_64 / aarch64 Linux, macOS),
// `ThetaDataDxContract` is 40 bytes {
//   const char *symbol        offset  0, size 8
//   int32_t sec_type          offset  8, size 4
//   bool has_expiration       offset 12, size 1
//   int32_t expiration        offset 16, size 4 (3 bytes pad after has_expiration)
//   bool has_right            offset 20, size 1
//   char right                offset 21, size 1
//   bool has_strike           offset 22, size 1
//   double strike             offset 24, size 8 (1 byte pad after has_strike)
//   int32_t strike_thousandths offset 32, size 4 (4 bytes tail pad)
// }
// Data variants carry no wire-internal `contract_id` field; identity
// rides on `contract.symbol` (and the option-only `expiration` /
// `strike` / `right` fields) instead. The numbers below are the exact
// struct layout under `-O2` with the generated `fpss_event_structs.h.inc`
// types on an LP64 host; CI re-validates the asserts on every build.

// Generated layout guards for the streaming event C mirror structs.
#include "fpss_layout_asserts.hpp.inc"

// Layout guards for the hand-written ABI structs (option contract,
// subscription, arrow / flat-file byte buffers, string + array
// wrappers) that no schema generator covers.
#include "abi_struct_layout_asserts.hpp.inc"

// OptionContract uses std::string for symbol to avoid use-after-free.
// The C FFI ThetaDataDxOptionContract uses a raw char* that is freed with the array,
// so we deep-copy the string during conversion.
struct OptionContract {
    std::string symbol;
    int32_t expiration;
    double strike;
    char right;
};

/// Active streaming subscription descriptor.
struct Subscription {
    std::string kind;
    std::string contract;
};

/// Full-stream subscription descriptor returned by
/// `Stream::active_full_subscriptions` /
/// `StreamingClient::active_full_subscriptions`. `sec_type` carries the
/// security-type discriminant (`"Stock"` / `"Option"` / `"Index"`) the
/// full-stream subscription is bound to; `kind` is the snake_case
/// full-stream kind label (`"full_trades"` / `"full_open_interest"`),
/// matching the Python / TypeScript `Subscription.kind` accessor.
struct FullSubscription {
    std::string kind;
    std::string sec_type;
};

// ── Greeks result (from standalone thetadatadx_all_greeks) ──

/// Full set of option Greeks and Black-Scholes intermediates returned by the
/// standalone `thetadatadx_all_greeks` computation, alongside the implied volatility
/// solve result (`iv`, `iv_error`).
struct GreeksResult {
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

// ══════════════════════════════════════════════════════════════════════════
// Typed exception hierarchy
// ══════════════════════════════════════════════════════════════════════════
//
// Every FFI failure surfaces as a leaf in this hierarchy, rooted at
// `ThetaDataError` (itself a `std::runtime_error`). Callers writing
// generic `catch (const std::runtime_error&)` continue to observe
// the failure unchanged; callers that want structured handling can
// `catch (const SubscriptionError&)` for a tier / permission error
// or `catch (const RateLimitError&)` for a 429-shaped response. The
// canonical leaf names (`NotFoundError`, `DeadlineExceededError`,
// `UnavailableError`, `InvalidParameterError`, ...) are identical to
// the Python and TypeScript leaf sets, so a handler ports across
// bindings by name. Python additionally ships two back-compat aliases
// (`NoDataFoundError` / `TimeoutError`) that have no C++ equivalent.
//
// The dispatcher [`detail::throw_for_code`] reads
// `thetadatadx_last_error_code()` (typed discriminant set inside the FFI
// boundary) to pick the right leaf without parsing the formatted
// message. Throw sites that emit a plain
// `std::runtime_error("thetadatadx: ...")` remain compatible because
// every leaf derives from `ThetaDataError` (a `std::runtime_error`):
// a generic `catch (const std::runtime_error&)` observes both the
// typed leaves and any plain-`runtime_error` site unchanged.

/// Root of the typed exception hierarchy; every FFI failure surfaces as this
/// class or one of its leaves. Derives from `std::runtime_error` so generic
/// `catch (const std::runtime_error&)` handlers still observe every failure.
class ThetaDataError : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

/// Authentication failure (Nexus 401, gRPC `Unauthenticated`).
class AuthenticationError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Bad credentials specifically (subset of `AuthenticationError`).
class InvalidCredentialsError : public AuthenticationError {
public:
    using AuthenticationError::AuthenticationError;
};

/// Tier / plan does not cover the requested endpoint (gRPC
/// `PermissionDenied`).
class SubscriptionError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Rate limit / quota (gRPC `ResourceExhausted`, HTTP 429).
///
/// Carries the server-supplied minimum back-off in seconds when the
/// upstream attached a `google.rpc.RetryInfo` detail, so a caller can
/// honour the cooldown as a value (`retry_after()`) instead of parsing
/// the message text. `retry_after()` is `std::nullopt` when no hint was
/// supplied.
class RateLimitError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;

    RateLimitError(const std::string& message, std::optional<double> retry_after_seconds)
        : ThetaDataError(message), retry_after_(retry_after_seconds) {}

    /// Server-supplied minimum back-off in seconds, or `std::nullopt`
    /// when the upstream attached no `RetryInfo` hint.
    std::optional<double> retry_after() const { return retry_after_; }

private:
    std::optional<double> retry_after_;
};

/// Empty result / unknown contract (gRPC `NotFound`,
/// `Error::NoData`).
class NotFoundError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Per-request deadline elapsed (gRPC `DeadlineExceeded`,
/// `with_deadline` / `timeout_ms` wrappers).
class DeadlineExceededError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Upstream unavailable (gRPC `Unavailable`, often retryable).
class UnavailableError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Transport-layer failure (TCP / TLS / IO).
class NetworkError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Decoder schema mismatch — usually a proto bump on the server
/// before the SDK is refreshed.
class SchemaMismatchError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// A client-side parameter was rejected by input validation (a bad
/// value, an out-of-range number, a missing required field). Distinct
/// from the root `ThetaDataError` so a malformed-but-rejected argument
/// is distinguishable by catch type from an unrelated configuration
/// fault (config-file I/O, TOML parse), which stays on the root class.
class InvalidParameterError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Streaming protocol / state-machine failure.
class StreamError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Environmental configuration fault — a config-file read failure, a
/// TOML parse error, or an internal config invariant. Distinct from
/// `InvalidParameterError` (a rejected user-supplied argument): a
/// `ConfigError` is the environment, not the call site. Pinned to the
/// reserved `THETADATADX_ERR_CONFIG` discriminant so a `catch (const
/// ConfigError&)` clause catches the same conditions the Python
/// `ConfigError` and the C ABI config code surface.
class ConfigError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

// Generated request-options bag. Included after the exception hierarchy
// so its `with_deadline` setter can throw the complete
// `InvalidParameterError` type when handed a negative deadline.
/* Generated in endpoint_options.hpp.inc. */
#include "endpoint_options.hpp.inc"

// ── RAII typed array wrappers ──

namespace detail {

/// Best-effort wipe of a `std::string` holding secret material.
///
/// Overwrites the live buffer through a `volatile` byte pointer (so the
/// compiler may not elide the store as dead) and then clears the string.
/// This is best-effort: the C ABI boundary takes `const char*`, so the
/// authoritative copy is the one the Rust core holds in zeroizing memory;
/// this only shortens the lifetime of the transient C++-side plaintext.
/// A reallocation earlier in the string's life may have left an untouched
/// copy elsewhere, which no portable C++ can reach.
inline void secure_wipe(std::string& secret) {
    if (!secret.empty()) {
        volatile char* p = const_cast<volatile char*>(secret.data());
        for (std::size_t i = 0; i < secret.size(); ++i) {
            p[i] = '\0';
        }
    }
    secret.clear();
}

/// Throw the [`ThetaDataError`] leaf that matches the typed C ABI
/// discriminant `code` (one of the `THETADATADX_ERR_*` constants in
/// `thetadatadx.h`). Used by every wrapper that already has the formatted
/// message in hand and wants the right leaf class without re-parsing.
/// Read the thread-local rate-limit back-off hint and convert it to
/// seconds, or `std::nullopt` when the FFI slot carries no hint (`-1`).
inline std::optional<double> last_ffi_retry_after_seconds() {
    const int64_t ms = thetadatadx_last_error_retry_after_ms();
    if (ms < 0) {
        return std::nullopt;
    }
    return static_cast<double>(ms) / 1000.0;
}

[[noreturn]] inline void throw_for_code(int32_t code, const std::string& message) {
    switch (code) {
        case THETADATADX_ERR_AUTHENTICATION:
            throw AuthenticationError("thetadatadx: " + message);
        case THETADATADX_ERR_INVALID_CREDENTIALS:
            throw InvalidCredentialsError("thetadatadx: " + message);
        case THETADATADX_ERR_SUBSCRIPTION:
            throw SubscriptionError("thetadatadx: " + message);
        case THETADATADX_ERR_RATE_LIMIT:
            // Carry the server back-off hint (if any) so the caller can
            // read `RateLimitError::retry_after()` as a value.
            throw RateLimitError("thetadatadx: " + message, last_ffi_retry_after_seconds());
        case THETADATADX_ERR_NOT_FOUND:
            throw NotFoundError("thetadatadx: " + message);
        case THETADATADX_ERR_DEADLINE_EXCEEDED:
            throw DeadlineExceededError("thetadatadx: " + message);
        case THETADATADX_ERR_UNAVAILABLE:
            throw UnavailableError("thetadatadx: " + message);
        case THETADATADX_ERR_NETWORK:
            throw NetworkError("thetadatadx: " + message);
        case THETADATADX_ERR_SCHEMA_MISMATCH:
            throw SchemaMismatchError("thetadatadx: " + message);
        case THETADATADX_ERR_INVALID_PARAMETER:
            throw InvalidParameterError("thetadatadx: " + message);
        case THETADATADX_ERR_STREAM:
            throw StreamError("thetadatadx: " + message);
        case THETADATADX_ERR_CONFIG:
            throw ConfigError("thetadatadx: " + message);
        case THETADATADX_ERR_OTHER:
        case THETADATADX_ERR_NONE:
        default:
            throw ThetaDataError("thetadatadx: " + message);
    }
}

// Snapshot the thread-local FFI error string. With `raw == false` an empty
// slot reads as the `"unknown error"` placeholder (the throw path always has
// a message to surface); with `raw == true` it reads as `""` so the array
// helpers can distinguish success-empty from failure-empty (a timeout on a
// list endpoint returns the same `{nullptr, 0}` sentinel as a successful
// empty result). Generated `_with_options` callers MUST
// `thetadatadx_clear_error()` before invoking the FFI so a stale error from a
// prior call isn't picked up by the `raw` read.
static std::string last_ffi_error(bool raw = false) {
    const char* err = thetadatadx_last_error();
    if (err) return std::string(err);
    return raw ? std::string() : std::string("unknown error");
}

/// Combined read-and-throw helper: snapshot the thread-local error
/// string AND the typed code, then throw the matching leaf. Always
/// throws (never returns); the caller invokes it only after proving it
/// has an error to surface (typically a null return from an FFI call).
/// The canonical throw path so every failure surfaces as the typed
/// leaf its error code selects rather than a plain
/// `std::runtime_error`.
[[noreturn]] inline void throw_last_ffi_error() {
    const std::string message = last_ffi_error();
    const int32_t code = thetadatadx_last_error_code();
    throw_for_code(code, message);
}

/// Throw a `ConfigError` for a malformed client-construction argument
/// (conflicting or absent builder auth sources). A local, pre-network
/// configuration fault — surfaced as the same typed leaf the C ABI
/// selects for `THETADATADX_ERR_CONFIG`, matching the other bindings.
[[noreturn]] inline void throw_config_error(const std::string& message) {
    throw_for_code(THETADATADX_ERR_CONFIG, message);
}

// Empty slot reads as "" (vs the `"unknown error"` placeholder) for the
// post-call success-empty / failure-empty disambiguation in the array helpers.
static std::string last_ffi_error_raw() { return last_ffi_error(true); }

template<typename T>
std::vector<T> to_vector(const T* data, size_t len) {
    if (data == nullptr || len == 0) return {};
    return std::vector<T>(data, data + len);
}

// Convert an FFI array to `vector<T>`, throwing on FFI error.
//
// `convert` maps the array to rows (it must NOT free — this helper owns the
// free); `free_fn` releases the FFI allocation on every exit path. An empty
// array is ambiguous: success-with-zero-results AND failure (e.g. timeout on
// a list endpoint) both return the `{nullptr, 0}` sentinel, so the error slot
// is consulted first. Generated wrappers `thetadatadx_clear_error()` before the
// FFI call so a stale error from a prior call isn't misattributed.
template<typename T, typename Arr, typename Convert, typename Free>
std::vector<T> check_tick_array(Arr arr, Convert convert, Free free_fn) {
    const std::string err = last_ffi_error_raw();
    if (!err.empty()) {
        const int32_t code = thetadatadx_last_error_code();
        free_fn(arr);
        throw_for_code(code, err);
    }
    auto result = convert(arr);
    free_fn(arr);
    return result;
}

// Pure converter (no free): the `Convert` callback for a `ThetaDataDxStringArray`.
inline std::vector<std::string> string_array_to_vector(ThetaDataDxStringArray arr) {
    std::vector<std::string> result;
    if (arr.data != nullptr && arr.len > 0) {
        result.reserve(arr.len);
        for (size_t i = 0; i < arr.len; ++i) {
            result.emplace_back(arr.data[i] ? arr.data[i] : "");
        }
    }
    return result;
}

inline std::vector<std::string> check_string_array(ThetaDataDxStringArray arr) {
    return check_tick_array<std::string>(arr, string_array_to_vector, thetadatadx_string_array_free);
}

inline std::vector<Subscription> subscription_array_to_vector(ThetaDataDxSubscriptionArray* arr) {
    if (arr == nullptr) {
        throw_last_ffi_error();
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
    thetadatadx_subscription_array_free(arr);
    return result;
}

/// Managed C string from FFI: auto-frees on destruction.
struct FfiString {
    char* ptr;
    FfiString(char* p) : ptr(p) {}
    ~FfiString() { if (ptr) thetadatadx_string_free(ptr); }
    FfiString(const FfiString&) = delete;
    FfiString& operator=(const FfiString&) = delete;

    std::string str() const { return ptr ? std::string(ptr) : ""; }
    bool ok() const { return ptr != nullptr; }
};

/// Poll cadence for the retired-callback reclaimer. Each step asks the FFI
/// drain barrier to wait at most this long, so the loop re-checks the
/// quiescence flag at roughly this interval. Small enough that a callback
/// that finishes promptly is reclaimed within a few milliseconds of its
/// last invocation.
inline constexpr std::chrono::milliseconds kReclaimPollStep{50};

/// Generous upper bound on how long the reclaimer waits for confirmed
/// consumer quiescence before releasing the retired callback node anyway.
///
/// The reclaimer drops the retired node the instant the FFI drain barrier
/// reports quiescence (typically single-digit milliseconds after the
/// consumer's last invocation), so this cap is reached only by a
/// pathologically stuck user callback that never returns. The cap is what
/// bounds the reclaimer thread and the retired node: a wedged callback
/// cannot leak either unboundedly. Releasing on the cap mirrors the C ABI
/// free contract, which also proceeds with destruction after its own
/// bounded drain wait when a callback will not return; in that case the
/// node is released while the callback may still be running, which is the
/// documented residual of a non-returning callback and not a regression
/// from the previous fixed-timer behavior.
inline constexpr std::chrono::seconds kReclaimQuiescenceCap{300};

/// Release a retired push-callback node off the calling thread, gated on
/// the consumer thread's confirmed quiescence rather than a wall-clock
/// guess.
///
/// When a callback is replaced or a streaming-owning wrapper is move-assigned
/// while the consumer thread may still be firing through the retired node's
/// registered `&fn`, the node must outlive the consumer's last dereference
/// of it. The ordering that establishes "the consumer has stopped firing"
/// is the same FFI drain barrier `stop_streaming` / free rely on: it flips
/// once the I/O and dispatch threads have joined and the user callback is
/// guaranteed to have stopped. `is_drained` wraps that barrier (one of the
/// `thetadatadx_*_await_drain` entry points bound to the retired session's
/// handle) and returns true once quiescence is confirmed.
///
/// This detaches a helper thread that polls `is_drained` until it reports
/// quiescence (or the bounded cap elapses) and only then runs `release`,
/// which drops the retired node and, where the helper owns it, frees the
/// retired handle. `release` therefore happens-after the consumer's final
/// dereference, so the dropped node is never read after free. `is_drained`
/// and `release` run on the detached helper, never on the consumer thread,
/// so the barrier cannot wait on work it is itself blocking.
///
/// Both callables are moved into the helper so a move-only retained handle
/// or storage can be carried in. The function returns immediately; the
/// calling (move / replace) path never blocks on the drain.
template <typename IsDrained, typename Release>
inline void reclaim_after_drain(IsDrained is_drained, Release release) {
    std::thread([is_drained = std::move(is_drained),
                 release = std::move(release)]() mutable {
        const auto deadline = std::chrono::steady_clock::now() + kReclaimQuiescenceCap;
        while (!is_drained()) {
            if (std::chrono::steady_clock::now() >= deadline) {
                break;
            }
        }
        // Confirmed quiescence (or the bounded cap): the consumer has
        // finished its last dereference of the retired node, so dropping it
        // now cannot be observed as a use-after-free.
        release();
    }).detach();
}

} // namespace detail

// ── RAII deleters ──

/// `unique_ptr` deleter that releases a `ThetaDataDxCredentials*` via `thetadatadx_credentials_free`.
struct CredentialsDeleter {
    void operator()(ThetaDataDxCredentials* p) const { if (p) thetadatadx_credentials_free(p); }
};

/// `unique_ptr` deleter that releases a `ThetaDataDxConfig*` via `thetadatadx_config_free`.
struct ConfigDeleter {
    void operator()(ThetaDataDxConfig* p) const { if (p) thetadatadx_config_free(p); }
};

/// `unique_ptr` deleter that releases a `ThetaDataDxHistoricalClient*` via `thetadatadx_historical_free`.
struct HistoricalClientDeleter {
    void operator()(ThetaDataDxHistoricalClient* p) const { if (p) thetadatadx_historical_free(p); }
};

/// `unique_ptr` deleter that releases a `ThetaDataDxStreamHandle*` via `thetadatadx_streaming_free`.
struct StreamingHandleDeleter {
    void operator()(ThetaDataDxStreamHandle* p) const { if (p) thetadatadx_streaming_free(p); }
};

// ── Credentials ──

/// RAII holder for a ThetaData credentials handle (`ThetaDataDxCredentials*`), freed
/// automatically on destruction. Constructed via `from_file` or `from_email`.
class Credentials {
public:
    /** Load credentials from a file.
     *  @param path File whose first line is the email and second line
     *         the password.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the file is unreadable or malformed. */
    static Credentials from_file(const std::string& path);

    /** Create credentials from an email and password pair.
     *  @param email Account email.
     *  @param password Account password.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the credentials cannot be built. */
    static Credentials from_email(const std::string& email, const std::string& password);

    /** Authenticate with an API key instead of an email and password.
     *  @param api_key API key; trimmed and held as secret material.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the credentials cannot be built. */
    static Credentials from_api_key(const std::string& api_key);

    /** Authenticate with an API key paired with an account email.
     *  @param email Account email (lowercased and trimmed; an empty email is dropped).
     *  @param api_key API key; trimmed and held as secret material.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the credentials cannot be built. */
    static Credentials from_api_key_with_email(const std::string& email, const std::string& api_key);

    /** Source credentials strictly from the `THETADATA_API_KEY`
     *  environment variable. Strict: an unset or whitespace-only value
     *  throws rather than falling back, and there is no `creds.txt` file
     *  fallback. Use `from_env_or_file` when a file fallback is wanted.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if `THETADATA_API_KEY` is unset or empty. */
    static Credentials from_env();

    /** Source credentials from the environment, falling back to a file.
     *  When `THETADATA_API_KEY` is set and non-empty an API key is used;
     *  otherwise the two-line file at `path` is read.
     *  @param path Path to the fallback credentials file.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the fallback file is unreadable or malformed. */
    static Credentials from_env_or_file(const std::string& path);

    /** Source credentials from a `.env`-format file.
     *  Each line is a `KEY=VALUE` assignment, with optional `export`
     *  prefix, `#` comment lines, and optional matching quotes around the
     *  value. `THETADATA_API_KEY` selects an API key; otherwise
     *  `THETADATA_EMAIL` + `THETADATA_PASSWORD` build email + password
     *  credentials.
     *  @param path Path to the `.env` file.
     *  @return An owning `Credentials` holder.
     *  @throws thetadatadx::ThetaDataError if the file is unreadable or defines none of the recognized keys. */
    static Credentials from_dotenv(const std::string& path);

    /** Borrow the underlying `ThetaDataDxCredentials*` for a connect call.
     *  @return A non-owning handle; ownership stays with this object. */
    ThetaDataDxCredentials* get() const { return handle_.get(); }

private:
    explicit Credentials(ThetaDataDxCredentials* h) : handle_(h) {}
    std::unique_ptr<ThetaDataDxCredentials, CredentialsDeleter> handle_;
};

// ── Config ──

/// RAII holder for a client configuration handle (`ThetaDataDxConfig*`), freed
/// automatically on destruction. Built from a named preset (`production` /
/// `dev` / `stage`) and tuned through the reconnect, streaming, retry, historical, and
/// metrics setters below.
class Config {
public:
    /** Build the production configuration (ThetaData NJ datacenter).
     *  @return An owning `Config` holder seeded with production defaults. */
    static Config production();

    /** Build the dev streaming configuration (port 20200, infinite
     *  historical replay).
     *  @return An owning `Config` holder seeded with dev defaults. */
    static Config dev();

    /** Build the historical-staging configuration (historical staging cluster +
     *  auth marker; streaming stays on production). Testing, unstable.
     *  @return An owning `Config` holder seeded with stage defaults. */
    static Config stage();

    /** Source a configuration from a `.env`-format file.
     *  Starts from the production configuration and applies the cluster
     *  keys carried by the file: `THETADATA_HISTORICAL_TYPE` (`PROD` / `STAGE`)
     *  selects the historical environment and `THETADATA_STREAMING_TYPE`
     *  (`PROD` / `DEV`) selects the streaming environment (both
     *  case-insensitive, selected independently), and the optional
     *  `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST` keys
     *  override the hosts (an explicit host wins over the environment
     *  default). This reads the same file format and keys as
     *  `Credentials::from_dotenv`, so one `.env` can carry
     *  `THETADATA_API_KEY`, `THETADATA_HISTORICAL_TYPE`, and `THETADATA_STREAMING_TYPE`.
     *  @param path Path to the `.env` file.
     *  @return An owning `Config` holder.
     *  @throws thetadatadx::ThetaDataError if the file is unreadable. */
    static Config from_dotenv(const std::string& path);

    /** Set streaming reconnect policy. 0=Auto (default), 1=Manual. Throws
     *  @c thetadatadx::InvalidParameterError when @p policy is outside the
     *  documented `{0, 1}` set, matching the Python `ValueError` /
     *  TypeScript `InvalidParameterError` rather than silently coercing
     *  an unknown value to Auto. */
    void set_reconnect_policy(int policy) {
        if (thetadatadx_config_set_reconnect_policy(handle_.get(), policy) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    #include "config_accessors.hpp.inc"

    /** Current reconnect policy selector: 0=Auto, 1=Manual, 2=Custom. */
    int32_t get_reconnect_policy() const {
        int32_t out{};
        thetadatadx_config_get_reconnect_policy(handle_.get(), &out);
        return out;
    }

    /** Install a custom reconnect policy driven by a C callback.
     *  Permanent disconnect reasons never reach the callback; it runs
     *  on the SDK's streaming I/O thread and must be thread-safe.
     *  Return the delay in milliseconds or a negative value to stop.
     *  Pass nullptr to restore the default Auto policy. */
    int32_t set_reconnect_callback(ThetaDataDxReconnectCallback cb, void* user_data) {
        return thetadatadx_config_set_reconnect_callback(handle_.get(), cb, user_data);
    }

    /** Set the streaming event ring size (slots). Must be a power of two
     *  >= 64; invalid values are rejected (thetadatadx_last_error). Default
     *  131_072. */
    void set_streaming_ring_size(size_t n) {
        thetadatadx_config_set_streaming_ring_size(handle_.get(), n);
    }

    /** Set the async worker-thread count using the (has_value, n) shape
     *  that preserves an explicit 0 across the C boundary. has_value=false
     *  defers to the default sizing. The async worker pool is
     *  process-global: it is built once, from the config of the first
     *  client connected in the process, so this is honoured when the first
     *  client in the process is created; later clients share the
     *  already-built pool and setting it again has no effect. */
    int32_t set_worker_threads(bool has_value, size_t n) {
        return thetadatadx_config_set_worker_threads(handle_.get(), has_value, n);
    }

    /** Read worker_threads back. Returns @c std::nullopt for the unset
     *  (auto-size) sentinel; returns the wrapped count when set. */
    std::optional<size_t> get_worker_threads() const {
        bool has_value = false;
        size_t n = 0;
        thetadatadx_config_get_worker_threads(handle_.get(), &has_value, &n);
        return has_value ? std::optional<size_t>{n} : std::nullopt;
    }

    // `retry.initial_delay` / `retry.max_delay` (ms) getters and the
    // `auth.nexus_url` / `auth.client_type` string accessors are generated
    // into config_accessors.hpp.inc from config_surface.toml.

    /** Target historical environment carried by this configuration:
     *  `"PROD"` for the production cluster or `"STAGE"` for staging. The
     *  historical and streaming environments are selected independently;
     *  the production / stage / dev presets (and the `THETADATA_HISTORICAL_TYPE`
     *  dotenv key) set the historical channel, and this is the readback of
     *  that selection. Returns an empty string if the FFI getter returns
     *  null (null handle). */
    std::string get_historical_environment() const {
        detail::FfiString s(thetadatadx_config_get_historical_environment(handle_.get()));
        return s.str();
    }

    /** Target streaming environment carried by this configuration:
     *  `"PROD"` for the production cluster or `"DEV"` for the dev cluster.
     *  The streaming and historical environments are selected
     *  independently; the production / stage / dev presets (and the
     *  `THETADATA_STREAMING_TYPE` dotenv key) set the streaming channel, and
     *  this is the readback of that selection. Returns an empty string if
     *  the FFI getter returns null (null handle). */
    std::string get_streaming_environment() const {
        detail::FfiString s(thetadatadx_config_get_streaming_environment(handle_.get()));
        return s.str();
    }

    /** Set the wait-strategy spin iteration count. Throws on a null handle. */
    void set_wait_spin_iters(uint32_t iters) {
        if (thetadatadx_config_set_wait_spin_iters(handle_.get(), iters) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Read the wait-strategy spin iteration count. Returns `0` on a null handle. */
    uint32_t get_wait_spin_iters() const {
        uint32_t iters = 0;
        thetadatadx_config_get_wait_spin_iters(handle_.get(), &iters);
        return iters;
    }

    /** Set the wait-strategy yield iteration count. Throws on a null handle. */
    void set_wait_yield_iters(uint32_t iters) {
        if (thetadatadx_config_set_wait_yield_iters(handle_.get(), iters) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Read the wait-strategy yield iteration count. Returns `0` on a null handle. */
    uint32_t get_wait_yield_iters() const {
        uint32_t iters = 0;
        thetadatadx_config_get_wait_yield_iters(handle_.get(), &iters);
        return iters;
    }

    /** Set the wait-strategy park interval (microseconds; Balanced /
     *  Efficient). Throws on a null handle. */
    void set_wait_park_us(uint64_t park_us) {
        if (thetadatadx_config_set_wait_park_us(handle_.get(), park_us) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Read the wait-strategy park interval in microseconds. Returns `0`
     *  on a null handle. */
    uint64_t get_wait_park_us() const {
        uint64_t park_us = 0;
        thetadatadx_config_get_wait_park_us(handle_.get(), &park_us);
        return park_us;
    }

    /** Pin the streaming consumer thread to a CPU core. A negative
     *  @p core (@c THETADATADX_CONSUMER_CPU_UNPINNED) means unpinned (the
     *  default). Throws on a null handle. */
    void set_consumer_cpu(int64_t core) {
        if (thetadatadx_config_set_consumer_cpu(handle_.get(), core) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Read the streaming consumer-thread CPU pin, or
     *  @c THETADATADX_CONSUMER_CPU_UNPINNED (-1) when unpinned. Returns
     *  the sentinel on a null handle. */
    int64_t get_consumer_cpu() const {
        int64_t core = THETADATADX_CONSUMER_CPU_UNPINNED;
        thetadatadx_config_get_consumer_cpu(handle_.get(), &core);
        return core;
    }

    // `historical_host` (string) is generated into config_accessors.hpp.inc.

    /** Get the raw handle. */
    ThetaDataDxConfig* get() const { return handle_.get(); }

private:
    explicit Config(ThetaDataDxConfig* h) : handle_(h) {}
    std::unique_ptr<ThetaDataDxConfig, ConfigDeleter> handle_;
};

// ── HistoricalClient ──

/// RAII wrapper around a historical gRPC client handle
/// (`ThetaDataDxHistoricalClient*`), freed automatically on destruction. The recommended
/// entry point for pure-historical access; the generated historical query
/// methods are mixed in from `historical.hpp.inc`.
class HistoricalClient {
public:
    /** Connect a historical client to ThetaData servers.
     *  @param creds Authenticated credentials.
     *  @param config Client configuration.
     *  @return A connected, owning `HistoricalClient`.
     *  @throws thetadatadx::ThetaDataError (or a typed leaf) on connection or
     *          authentication failure. */
    static HistoricalClient connect(const Credentials& creds, const Config& config);

    /** Connect a historical client, loading credentials from a
     *  file. One-call equivalent of `Credentials::from_file(path)`
     *  followed by `connect`.
     *  @param path File whose first line is the email and second line
     *         the password.
     *  @param config Client configuration; defaults to the production
     *         preset.
     *  @return A connected, owning `HistoricalClient`.
     *  @throws thetadatadx::ThetaDataError (or a typed leaf) on an unreadable
     *          credentials file or a connection / authentication failure. */
    static HistoricalClient from_file(const std::string& path, const Config& config = Config::production());

    #include "historical.hpp.inc"

private:
    explicit HistoricalClient(ThetaDataDxHistoricalClient* h) : handle_(h) {}

    /// Resolve the historical sub-handle the generated buffered query
    /// definitions call into. On the standalone client this is the owned
    /// handle directly; the unified client's `Historical` view derives it
    /// from `thetadatadx_client_historical`. Naming the accessor uniformly lets
    /// the generator emit one definition body for both classes.
    const ThetaDataDxHistoricalClient* historical_handle() const { return handle_.get(); }

    std::unique_ptr<ThetaDataDxHistoricalClient, HistoricalClientDeleter> handle_;
};

// ── streaming event types (re-exported from thetadatadx.h) ──
//
// Each control variant has its own typed C struct rather than a single
// flat `{ kind, id, detail }` envelope. Consumers dispatch via
// `event.kind` and read the matching `event.<variant>` payload
// (`event.login_success.permissions`, `event.disconnected.reason`,
// etc.). The aliases below mirror every generated type so C++ users can
// stay in the `thetadatadx::` namespace.

using StreamEventKind = ThetaDataDxStreamEventKind;
using StreamQuote = ThetaDataDxStreamQuote;
using StreamTrade = ThetaDataDxStreamTrade;
using StreamOpenInterest = ThetaDataDxStreamOpenInterest;
using StreamOhlcvc = ThetaDataDxStreamOhlcvc;
using StreamMarketValue = ThetaDataDxStreamMarketValue;
// Typed control variants — one alias per control event type.
using StreamConnected = ThetaDataDxStreamConnected;
using StreamContractAssigned = ThetaDataDxStreamContractAssigned;
using StreamDisconnected = ThetaDataDxStreamDisconnected;
using StreamParseError = ThetaDataDxStreamParseError;
using StreamLoginSuccess = ThetaDataDxStreamLoginSuccess;
using StreamMarketClose = ThetaDataDxStreamMarketClose;
using StreamMarketOpen = ThetaDataDxStreamMarketOpen;
using StreamPing = ThetaDataDxStreamPing;
using StreamReconnected = ThetaDataDxStreamReconnected;
using StreamReconnectedServer = ThetaDataDxStreamReconnectedServer;
using StreamReconnecting = ThetaDataDxStreamReconnecting;
using StreamReconnectsExhausted = ThetaDataDxStreamReconnectsExhausted;
using StreamReqResponse = ThetaDataDxStreamReqResponse;
using StreamRestart = ThetaDataDxStreamRestart;
using StreamServerError = ThetaDataDxStreamServerError;
using StreamUnknownControl = ThetaDataDxStreamUnknownControl;
using StreamUnknownFrame = ThetaDataDxStreamUnknownFrame;
using StreamEvent = ThetaDataDxStreamEvent;

// ── Real-time streaming client ──
//
// Event delivery is callback-driven via `set_callback(fn)`. Events flow
// from the streaming reader through a bounded ring to a dedicated consumer
// thread, which invokes `fn` inside an isolation boundary. The reader
// thread never blocks on user code; on ring overflow events are dropped
// and counted via `dropped_event_count()`.
//
// The StreamingClient owns the `std::function`. A static-member shim retrieves
// the stored function from the registered `void* ctx` and invokes it with
// the event reference. The shim keeps C++ language linkage (a member cannot be
// `extern "C"`) but matches the C-linkage `ThetaDataDxStreamCallback` typedef,
// an assignment every mainstream ABI accepts. The shim converts
// `const ThetaDataDxStreamEvent*` (the C ABI
// payload type) to `const StreamEvent&` (the C++ alias) at the boundary.
// Callback storage outlives the consumer thread because the destruction
// path always routes through `thetadatadx_streaming_free`, which performs an internal
// drain barrier (5 s timeout) so the consumer has stopped firing the
// callback before the storage is released.

class StreamingClient {
public:
    #include "fpss.hpp.inc"

    /// Polymorphic subscribe — primary fluent entry point. Forward
    /// declared here; the inline implementation appears below the
    /// fluent type definitions.
    inline void subscribe(const class FluentSubscription& sub) const;
    inline void subscribe_many(std::initializer_list<class FluentSubscription> subs) const;
    inline void unsubscribe(const class FluentSubscription& sub) const;
    inline void unsubscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    ~StreamingClient();

    StreamingClient(const StreamingClient&) = delete;
    StreamingClient& operator=(const StreamingClient&) = delete;
    StreamingClient(StreamingClient&& other) noexcept
        // Initialiser order MUST follow declaration order; see the
        // ordering invariant comment above the member declarations.
        : callback_(std::move(other.callback_)),
          handle_(std::move(other.handle_)) {}
    /** Move-assign. The receiver may already hold a live streaming
     *  handle with a registered callback whose `ctx` points into our
     *  existing `callback_` storage. We must drain that wiring on the
     *  C ABI side BEFORE destroying the old `callback_`, otherwise
     *  the consumer thread could invoke through a dangling `void*`
     *  ctx. `thetadatadx_streaming_shutdown` returns asynchronously, so we follow
     *  it with `thetadatadx_streaming_await_drain` (5 s budget, matching the free
     *  contract) to confirm the consumer thread has stopped firing
     *  the callback before releasing the storage.
     *
     *  Drain timeout (rare, indicates a wedged user callback): we MUST
     *  NOT reset `callback_` synchronously because a still-firing
     *  consumer would invoke through a dangling ctx. Nor may we free the
     *  handle whose drain barrier that consumer rides. We hand BOTH the
     *  retired handle and the retired callback storage to a helper thread
     *  that polls the same drain barrier and releases them only once the
     *  consumer is confirmed quiesced (handle first so its free-time
     *  barrier still sees a live ctx, then the storage), with a bounded cap
     *  so a wedged callback cannot leak either. The move proceeds without
     *  observable liveness loss to the caller. */
    StreamingClient& operator=(StreamingClient&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                thetadatadx_streaming_shutdown(handle_.get());
                // Block until the consumer thread quiesces. The 5 s
                // budget matches `thetadatadx_streaming_free`'s internal barrier.
                int drained = thetadatadx_streaming_await_drain(handle_.get(), 5000);
                if (drained == 0) {
                    // Drain barrier timed out: the consumer may still be
                    // firing through `callback_`'s storage. Hand the retired
                    // handle and storage to a reclaimer that drops them only
                    // after the consumer is confirmed quiesced, polling the
                    // same barrier rather than guessing a wall-clock window.
                    // Borrow the raw handle for the poll; ownership stays with
                    // `retired_handle` inside the reclaimer.
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
                            // Free the handle first so its internal drain
                            // barrier still observes a live `ctx`, then drop
                            // the storage. Mirrors the handle-before-callback
                            // member destruction invariant.
                            retired_handle.reset();
                            retired_cb.reset();
                        });
                    // `handle_` and `callback_` are now empty; the reclaimer
                    // owns the retired session.
                } else {
                    callback_.reset();
                }
            } else {
                callback_.reset();
            }
            handle_ = std::move(other.handle_);
            callback_ = std::move(other.callback_);
        }
        return *this;
    }

    /** Register a streaming callback and open the streaming connection.
     *  `fn` runs on the consumer thread inside an isolation boundary, never
     *  on the streaming reader. The reader thread cannot be blocked by
     *  user code: on ring overflow events are dropped and counted via
     *  `dropped_event_count()`. Throws on registration failure.
     *
     *  ## Callback storage + thread affinity
     *
     *  The wrapper owns a `std::unique_ptr<std::function>` whose
     *  address is what the consumer thread receives as `ctx`. That
     *  address must outlive every consumer-thread invocation;
     *  destruction routes through `thetadatadx_streaming_free`, which performs the
     *  shutdown + drain barrier internally, and move-assign calls
     *  `thetadatadx_streaming_shutdown` followed by `thetadatadx_streaming_await_drain` (5 s
     *  budget) before releasing the storage — so no thread can
     *  observe a dangling ctx. The consumer invokes `fn` serially on
     *  a single thread, so no internal locks are needed for
     *  callback-private state.
     *
     *  ## Lifecycle contract (one-shot rule)
     *
     *  The C ABI permits exactly one successful callback registration
     *  per handle, and rejects every register / reconnect / shutdown
     *  call after `thetadatadx_streaming_shutdown`. A second call on a still-live
     *  handle returns -1 and KEEPS the previously installed
     *  (callback, ctx) wired into the dispatcher. We therefore
     *  stage the new `std::function` into a local `unique_ptr`,
     *  attempt the FFI registration with the staged address, and only
     *  adopt it into `callback_` after the FFI reports success. On
     *  failure the existing `callback_` is left untouched so the
     *  still-live registration keeps pointing at valid storage. */
    void set_callback(std::function<void(const StreamEvent&)> fn) {
        auto staged = std::make_unique<std::function<void(const StreamEvent&)>>(std::move(fn));
        int rc = thetadatadx_streaming_set_callback(handle_.get(), &StreamingClient::callback_shim, staged.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
        callback_ = std::move(staged);
    }

    /** Cumulative count of streaming events the TLS reader could not
     *  publish into the bounded ring because the consumer fell behind
     *  and the ring was full. Returns 0 when no callback has been
     *  installed yet. Safe to call on a moved-from client. Mirrors the
     *  unified Stream::dropped_event_count() spelling so the same counter
     *  reads identically on both C++ streaming surfaces. */
    uint64_t dropped_event_count() const {
        return handle_ ? thetadatadx_streaming_dropped_events(handle_.get()) : 0;
    }

    /** Point-in-time count of streaming events published into the
     *  event ring but not yet drained into the registered callback —
     *  the in-flight depth between the I/O thread and the dispatcher.
     *  Rising occupancy that approaches ring_capacity() predicts
     *  drops before dropped_event_count() moves; sampling never blocks the
     *  feed and is safe from any thread. Returns 0 when no session is
     *  live. Safe to call on a moved-from client. */
    uint64_t ring_occupancy() const {
        return handle_ ? thetadatadx_streaming_ring_occupancy(handle_.get()) : 0;
    }

    /** Configured capacity of the streaming event ring in slots (the
     *  streaming_ring_size setting, a power of two) — the fixed
     *  denominator for ring_occupancy(). Returns 0 when no session is
     *  live. Safe to call on a moved-from client. */
    uint64_t ring_capacity() const {
        return handle_ ? thetadatadx_streaming_ring_capacity(handle_.get()) : 0;
    }

    /** Cumulative count of user-callback failures contained by the
     *  per-invocation isolation boundary since the current stream
     *  started. If the callback aborts on a given event, the failure is
     *  contained, recorded here, and does not stop event delivery — the
     *  next event continues normally. Returns 0 when no callback has been
     *  installed yet. Safe to call from any thread without blocking. */
    uint64_t panic_count() const {
        return handle_ ? thetadatadx_streaming_panic_count(handle_.get()) : 0;
    }

    /** Set the slow-callback wall-clock threshold in microseconds. When a
     *  callback invocation runs longer than threshold_us,
     *  slow_callback_count() increments and a rate-limited warning is
     *  logged. Pass 0 to disable. Observability-only: the watchdog never
     *  cancels the callback. No-op on a moved-from or shut-down client. */
    void set_slow_callback_threshold_us(uint64_t threshold_us) const {
        if (handle_) {
            thetadatadx_streaming_set_slow_callback_threshold_us(handle_.get(), threshold_us);
        }
    }

    /** Cumulative count of user-callback invocations whose wall-clock
     *  duration exceeded the threshold set via
     *  set_slow_callback_threshold_us(). Returns 0 when the watchdog is
     *  disabled or no session is live. Safe to call on a moved-from
     *  client. */
    uint64_t slow_callback_count() const {
        return handle_ ? thetadatadx_streaming_slow_callback_count(handle_.get()) : 0;
    }

    /** Milliseconds since the most recent inbound streaming frame of
     *  any kind. Returns 0 on success with the value in *out_ms, 1
     *  when no session is live or no frame has been received yet, -1
     *  on a null handle. */
    int32_t millis_since_last_event(uint64_t* out_ms) const {
        return handle_ ? thetadatadx_streaming_millis_since_last_event(handle_.get(), out_ms) : -1;
    }

    /** UNIX-nanosecond receive timestamp of the most recent inbound
     *  streaming frame. 0 when no session is live or no frame has
     *  arrived yet. */
    int64_t last_event_received_at_unix_nanos() const {
        return handle_ ? thetadatadx_streaming_last_event_received_at_unix_nanos(handle_.get()) : 0;
    }

    /** Address (host:port) of the server the current session is
     *  connected to, following the session across auto-reconnects.
     *  Empty when no session is live. */
    std::string last_connected_addr() const {
        if (!handle_) return {};
        char* raw = thetadatadx_streaming_last_connected_addr(handle_.get());
        if (!raw) return {};
        std::string out(raw);
        thetadatadx_string_free(raw);
        return out;
    }

    /** `true` iff the streaming connection is currently open. Distinct
     *  from is_authenticated(): the connection can be open yet briefly
     *  unauthenticated mid-reconnect. Returns false on a moved-from or
     *  shut-down client. Mirrors the unified Stream::is_streaming() and
     *  the Python / TypeScript `is_streaming` getter so the same status
     *  reads identically on both C++ streaming surfaces. */
    bool is_streaming() const {
        return handle_ && thetadatadx_streaming_is_streaming(handle_.get()) == 1;
    }

    /** Snapshot the currently-active full-stream subscriptions (the
     *  entire universe for a given sec_type + kind, not bound to a
     *  single contract). Throws on FFI error. Mirrors the unified
     *  Stream::active_full_subscriptions() and the Python / TypeScript
     *  `active_full_subscriptions` placement. */
    std::vector<FullSubscription> active_full_subscriptions() const {
        ThetaDataDxSubscriptionArray* arr =
            thetadatadx_streaming_active_full_subscriptions(handle_.get());
        if (arr == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<FullSubscription> out;
        if (arr->data != nullptr && arr->len > 0) {
            out.reserve(arr->len);
            for (size_t i = 0; i < arr->len; ++i) {
                const ThetaDataDxSubscription& s = arr->data[i];
                out.push_back(FullSubscription{
                    s.kind ? std::string(s.kind) : std::string(),
                    s.contract ? std::string(s.contract) : std::string(),
                });
            }
        }
        thetadatadx_subscription_array_free(arr);
        return out;
    }


private:
    // Static-member shim that the dispatcher invokes. It keeps C++ language
    // linkage (a member cannot be `extern "C"`) but its signature matches the
    // C-linkage `ThetaDataDxStreamCallback` typedef, an assignment every
    // mainstream ABI accepts. `ctx` is the `std::function*` we registered
    // alongside the callback. The event pointer is non-null and valid only for
    // the duration of this call.
    static void callback_shim(const ThetaDataDxStreamEvent* event, void* ctx) noexcept {
        auto* fn = static_cast<std::function<void(const StreamEvent&)>*>(ctx);
        if (fn == nullptr || event == nullptr) return;
        try {
            (*fn)(*event);
        } catch (...) {
            // User callbacks must not propagate exceptions across the
            // C ABI boundary — unwinding across it is undefined behavior.
            // Swallow.
        }
    }

    // ── Member ordering invariant (do not reorder) ──
    //
    // C++ destructs members in REVERSE declaration order. The C ABI
    // contract for `thetadatadx_streaming_free` is "drain the user-callback path
    // before this call returns" — the FFI's deleter runs that drain
    // barrier internally (5 s budget). For the barrier to be safe the
    // `std::function` storage backing the registered `void* ctx` MUST
    // still be alive while `thetadatadx_streaming_free` is polling the drain flag,
    // because the consumer thread may still be invoking through it.
    //
    // We therefore declare `handle_` AFTER `callback_`: reverse-order
    // destruction destroys `handle_` first → `thetadatadx_streaming_free` runs and
    // its drain barrier returns → `callback_` storage is then released.
    // Reordering these two members reintroduces the use-after-free.
    //
    // `callback_` is a `unique_ptr<std::function<...>>` so the address
    // handed to the C ABI as `ctx` is stable across moves of the owning
    // `StreamingClient`.
    std::unique_ptr<std::function<void(const StreamEvent&)>> callback_;
    std::unique_ptr<ThetaDataDxStreamHandle, StreamingHandleDeleter> handle_;
};

// ── Standalone Greeks functions ──

#include "utilities.hpp.inc"

// ── FLATFILES surface ────────────────────────────────────────────────
//
// Thin RAII wrappers over the C ABI in `thetadatadx.h`. The dynamic schema
// (one column set per (sec_type, req_type)) is opaque on the C++ side
// — typed access is via the Arrow IPC bytes returned by
// `FlatFileRowList::to_arrow_ipc()`. Pair with arrow-cpp on the
// consumer side to materialise an `arrow::Table`.

/// `unique_ptr` deleter that releases a `ThetaDataDxFlatFileRowList*` via
/// `thetadatadx_flatfile_rowlist_free`.
struct FlatFileRowListDeleter {
    void operator()(ThetaDataDxFlatFileRowList* p) const {
        if (p) thetadatadx_flatfile_rowlist_free(p);
    }
};

/// RAII wrapper around an opaque `ThetaDataDxFlatFileRowList*`. Move-only.
/// Built by `FlatFiles::request(...)`; expose either Arrow IPC bytes
/// via `to_arrow_ipc()` or use the free `to_path` variant on the
/// owning `Client`.
class FlatFileRowList {
public:
    FlatFileRowList(FlatFileRowList&&) = default;
    FlatFileRowList& operator=(FlatFileRowList&&) = default;
    FlatFileRowList(const FlatFileRowList&) = delete;
    FlatFileRowList& operator=(const FlatFileRowList&) = delete;

    /// Number of decoded rows. 0 on a moved-from / null handle.
    size_t size() const noexcept {
        return handle_ ? thetadatadx_flatfile_rows_count(handle_.get()) : 0;
    }

    /// Serialise the rows as Arrow IPC stream bytes. Throws on
    /// schema-inference / serialisation failure. The returned vector
    /// owns its memory; the underlying FFI buffer is freed before
    /// return so the caller never has to invoke `thetadatadx_flatfile_bytes_free`.
    std::vector<uint8_t> to_arrow_ipc() const {
        if (!handle_) {
            throw std::runtime_error("thetadatadx: FlatFileRowList moved-from");
        }
        ThetaDataDxFlatFileBytes raw = thetadatadx_flatfile_rows_to_arrow_ipc(handle_.get());
        if (raw.data == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<uint8_t> out(raw.data, raw.data + raw.len);
        thetadatadx_flatfile_bytes_free(raw);
        return out;
    }

    /// Raw handle accessor for advanced consumers that want to call
    /// the C ABI directly (e.g. zero-copy bridges into custom Arrow
    /// converters). Ownership remains with this object.
    const ThetaDataDxFlatFileRowList* get() const noexcept { return handle_.get(); }

private:
    friend class FlatFiles;
    explicit FlatFileRowList(ThetaDataDxFlatFileRowList* h) : handle_(h) {}
    std::unique_ptr<ThetaDataDxFlatFileRowList, FlatFileRowListDeleter> handle_;
};

/// Namespace handle exposing the FLATFILES surface for a connected
/// unified client. Cheap to construct — borrows the parent handle.
class FlatFiles {
public:
    /// Generic dispatcher. `sec_type` is "OPTION" / "STOCK" / "INDEX";
    /// `req_type` is "EOD" / "QUOTE" / "OPEN_INTEREST" / "OHLC" /
    /// "TRADE" / "TRADE_QUOTE"; `date` is "YYYYMMDD". A `(sec_type,
    /// req_type)` pair the flat-file distribution does not serve throws
    /// a typed invalid-parameter error before any network round-trip.
    FlatFileRowList request(const std::string& sec_type,
                            const std::string& req_type,
                            const std::string& date) const {
        ThetaDataDxFlatFileRowList* h = thetadatadx_flatfile_request_decoded(
            handle_, sec_type.c_str(), req_type.c_str(), date.c_str());
        if (h == nullptr) {
            detail::throw_last_ffi_error();
        }
        return FlatFileRowList(h);
    }

    // Convenience accessors cover exactly the datasets the flat-file
    // distribution serves — option trade_quote / open_interest / eod and
    // stock trade_quote / eod. Other request types are reachable via the
    // historical surface, not as flat files.
    FlatFileRowList option_trade_quote(const std::string& date) const {
        return request("OPTION", "TRADE_QUOTE", date);
    }
    FlatFileRowList option_open_interest(const std::string& date) const {
        return request("OPTION", "OPEN_INTEREST", date);
    }
    FlatFileRowList option_eod(const std::string& date) const {
        return request("OPTION", "EOD", date);
    }
    FlatFileRowList stock_trade_quote(const std::string& date) const {
        return request("STOCK", "TRADE_QUOTE", date);
    }
    FlatFileRowList stock_eod(const std::string& date) const {
        return request("STOCK", "EOD", date);
    }

    /// Pull a flat-file blob and write the requested vendor format
    /// (`csv` / `jsonl`) directly to `path`. Throws on FFI failure.
    void to_path(const std::string& sec_type,
                 const std::string& req_type,
                 const std::string& date,
                 const std::string& path,
                 const std::string& format = "csv") const {
        int rc = thetadatadx_flatfile_request_to_path(
            handle_, sec_type.c_str(), req_type.c_str(),
            date.c_str(), path.c_str(), format.c_str());
        if (rc != 0) {
            detail::throw_last_ffi_error();
        }
    }

private:
    friend class Client;
    explicit FlatFiles(const ThetaDataDxClient* h) : handle_(h) {}
    const ThetaDataDxClient* handle_;
};

/// `unique_ptr` deleter that releases a `ThetaDataDxClient*` via `thetadatadx_client_free`.
struct UnifiedDeleter {
    void operator()(ThetaDataDxClient* p) const {
        if (p) thetadatadx_client_free(p);
    }
};

/// Historical-data sub-namespace returned by `Client::historical()`.
///
/// Borrows the unified `ThetaDataDxClient*` and derives the historical
/// sub-handle (`thetadatadx_client_historical`) on each call, so constructing it
/// performs no auth round-trip and opens no second connection. Exposes the
/// full buffered historical query surface (mixed in from
/// `historical.hpp.inc`) and the server-stream companions
/// (`historical_stream.hpp.inc`) generated identically with the standalone
/// `HistoricalClient`. The view is non-owning: its lifetime is bounded by
/// the parent `Client`.
class Historical {
public:
    /// Generated buffered historical query declarations
    /// (`stock_history_eod`, `option_snapshot_quote`, …).
    #include "historical.hpp.inc"

    /// Generated server-stream historical method declarations
    /// (`<endpoint>_stream`) plus the shared `stream_chunk_shim`
    /// trampoline. Each drains a large historical result chunk-by-chunk,
    /// bounding peak memory to a single chunk.
    #include "historical_stream.hpp.inc"

private:
    friend class Client;
    explicit Historical(const ThetaDataDxClient* h) : handle_(h) {}

    /// Resolve the historical sub-handle the generated query definitions
    /// call into. Derives it from the unified handle via
    /// `thetadatadx_client_historical`; throws on the (unexpected) null result so
    /// the failure surfaces as a typed error rather than a null deref.
    const ThetaDataDxHistoricalClient* historical_handle() const {
        const ThetaDataDxHistoricalClient* hist = thetadatadx_client_historical(handle_);
        if (hist == nullptr) {
            detail::throw_last_ffi_error();
        }
        return hist;
    }

    const ThetaDataDxClient* handle_;
};

/// Backing node for a single push-callback registration. The
/// `std::function` lives at a fixed heap address for the node's whole
/// life, so the `void* ctx` registered with the dispatcher (`&fn`) stays
/// valid for every consumer-thread invocation that captured it. A node is
/// never mutated in place once registered: replacing a callback installs a
/// FRESH node (see `CallbackState`) and registers the new node's distinct
/// `&fn`, so a consumer thread still firing through an earlier
/// registration always dereferences its own node's stable, unchanged `fn`.
struct CallbackSlot {
    std::function<void(const StreamEvent&)> fn;
};

/// Shared indirection between a `Client` and the `Stream` views it hands
/// out. Both reference one `CallbackState` by `shared_ptr`; the state owns
/// the currently-registered `CallbackSlot`. The dispatcher `ctx` is the
/// address of `fn` inside whichever node `slot` currently points at.
///
/// Why the extra level of indirection: when a callback is replaced while a
/// previous consumer is still firing (the drain-timeout path of
/// `Stream::set_callback`), the replacement must register a node at a
/// DIFFERENT address than the one the still-running consumer captured, and
/// the old node must stay alive and unmodified until that consumer
/// quiesces. The replacement therefore installs a fresh node into `slot`
/// (a new `&fn` for the new registration) and keeps the retired node alive
/// off the replacement path. Because `Client` and `Stream` share the one
/// `CallbackState`, both observe the new node afterward, so a later
/// `Client` move or destruction operates on the right node. `slot` is only
/// ever read or repointed on the owning (user) thread; the consumer thread
/// holds the raw `&fn` it was registered with and never reads `slot`, so no
/// synchronization on `slot` is required against the consumer.
///
/// `slot` carries no synchronization against a SECOND owner thread either:
/// the lifecycle surface is single-threaded per client, so calling
/// `set_callback` concurrently from two `Stream` views over the same client
/// is API misuse and a data race on `slot`. Drive the streaming lifecycle
/// from one thread (the same precondition the C ABI states for its own
/// register / stop / reconnect calls).
struct CallbackState {
    std::shared_ptr<CallbackSlot> slot = std::make_shared<CallbackSlot>();
};

/// Backpressure policy for the pull-based Arrow `RecordBatch` reader
/// (`Stream::batches(..)`).
///
/// `Block` (default) is lossless and applies backpressure to the wire;
/// `DropOldest` keeps a bounded buffer and drops the oldest batch on
/// overflow, counted by `RecordBatchStream::dropped()`.
enum class Backpressure {
    /// Lossless: block until the reader catches up. The default.
    Block,
    /// Bounded buffer: drop the oldest batch on overflow, count it.
    DropOldest,
};

#ifdef THETADATADX_CPP_ARROW
/// Pull-based columnar reader over the live stream — a sibling to the
/// per-event `Stream::set_callback`.
///
/// A concrete `arrow::RecordBatchReader`: `ReadNext(&batch)` yields the next
/// batch (and sets `batch` to `nullptr` at clean end of stream), `schema()`
/// reports the fixed schema, and `dropped()` reports the drop-oldest count.
/// Held by `std::shared_ptr` (see `Stream::batches`); the reader closes —
/// unsubscribing and tearing the streaming session down — when the last reference
/// drops (RAII). Every batch carries the identical schema, so batches are
/// concat-safe.
///
/// Each batch crosses the C ABI as an Arrow IPC stream and is decoded here
/// with arrow-cpp's IPC reader, the same wire format the per-tick
/// `*_to_arrow_ipc` terminals use.
///
/// Thread-safety: `ReadNext` is the single blocking consumer and is not
/// itself re-entrant, but `close()` and destruction are safe to call from a
/// different thread while a `ReadNext` is parked: the underlying reader is
/// reference-counted across the C ABI, so a teardown wakes the parked pull
/// (which returns clean end of stream) and the reader is not torn down until
/// the in-flight pull completes. This matches the standard
/// `arrow::RecordBatchReader` handoff where a worker drains the reader while
/// the owner may release the last `shared_ptr` or call `close()`.
class RecordBatchStream : public arrow::RecordBatchReader {
public:
    RecordBatchStream(const RecordBatchStream&) = delete;
    RecordBatchStream& operator=(const RecordBatchStream&) = delete;

    ~RecordBatchStream() override {
        if (handle_ != nullptr) {
            // Safe to free even if another thread is parked in `ReadNext`: the
            // C ABI handle is reference-counted, so this free signals teardown
            // (waking the parked pull) and the underlying reader is not torn
            // down until that in-flight pull returns. See the class-level
            // thread-safety note above.
            thetadatadx_record_batch_stream_free(handle_);
            handle_ = nullptr;
        }
    }

    /// The fixed Arrow schema every batch carries. Decoded once from the
    /// schema-only IPC buffer and cached. Never null after construction.
    std::shared_ptr<arrow::Schema> schema() const override {
        return schema_;
    }

    /// Read the next batch. Sets `*batch` to the next `RecordBatch`, or to
    /// `nullptr` at clean end of stream. Returns a non-OK status on error.
    arrow::Status ReadNext(std::shared_ptr<arrow::RecordBatch>* batch) override {
        if (batch == nullptr) {
            return arrow::Status::Invalid("ReadNext: null out-parameter");
        }
        *batch = nullptr;
        if (handle_ == nullptr) {
            return arrow::Status::OK(); // closed -> end of stream
        }
        ThetaDataDxArrowBytes bytes{};
        const int32_t rc = thetadatadx_record_batch_stream_next_ipc(handle_, &bytes);
        if (rc == 1) {
            return arrow::Status::OK(); // clean end of stream
        }
        if (rc < 0) {
            const char* err = thetadatadx_last_error();
            return arrow::Status::IOError(err != nullptr ? err
                                                         : "record batch stream pull failed");
        }
        // rc == 0: decode the one batch carried by this IPC buffer.
        arrow::Status status = decode_one(bytes, batch);
        thetadatadx_arrow_bytes_free(bytes);
        return status;
    }

    /// Number of batches dropped so far under the `DropOldest` policy. Always
    /// 0 under `Block`.
    uint64_t dropped() const {
        return handle_ != nullptr ? thetadatadx_record_batch_stream_dropped(handle_) : 0;
    }

    /// Stop the reader: unsubscribe and tear the streaming session down.
    /// Idempotent; subsequent reads return end of stream.
    ///
    /// Safe to call from another thread while a `ReadNext` is in flight: it
    /// signals close through the reference-counted C ABI (waking the parked
    /// pull, which then returns clean end of stream) without freeing the
    /// handle. The handle is released by the destructor, so a `close()` here
    /// followed by destruction frees exactly once.
    void close() {
        if (handle_ != nullptr) {
            thetadatadx_record_batch_stream_close(handle_);
        }
    }

private:
    friend class Stream;

    /// Build a reader over an owned C ABI handle, decoding the fixed schema
    /// up front. Throws on a schema-decode failure (and frees the handle).
    static std::shared_ptr<RecordBatchStream> create(ThetaDataDxRecordBatchStream* handle) {
        ThetaDataDxArrowBytes schema_bytes{};
        const int32_t rc = thetadatadx_record_batch_stream_schema_ipc(handle, &schema_bytes);
        if (rc < 0) {
            thetadatadx_record_batch_stream_free(handle);
            detail::throw_last_ffi_error();
        }
        std::shared_ptr<arrow::Schema> schema;
        arrow::Status status = decode_schema(schema_bytes, &schema);
        thetadatadx_arrow_bytes_free(schema_bytes);
        if (!status.ok()) {
            thetadatadx_record_batch_stream_free(handle);
            throw std::runtime_error("failed to decode streaming schema: " + status.ToString());
        }
        return std::shared_ptr<RecordBatchStream>(new RecordBatchStream(handle, std::move(schema)));
    }

    RecordBatchStream(ThetaDataDxRecordBatchStream* handle, std::shared_ptr<arrow::Schema> schema)
        : handle_(handle), schema_(std::move(schema)) {}

    /// Decode a single-batch Arrow IPC buffer into `*batch`.
    ///
    /// Called only for a `next_ipc` return of 0, which promises one batch in
    /// the buffer. A null batch from the IPC reader (a truncated or malformed
    /// frame) is therefore a decode error, not end of stream: surfacing it as
    /// an error avoids silently truncating a live stream, since the
    /// `arrow::RecordBatchReader` contract reads a null batch with an OK
    /// status as end of stream.
    static arrow::Status decode_one(const ThetaDataDxArrowBytes& bytes,
                                    std::shared_ptr<arrow::RecordBatch>* batch) {
        ARROW_ASSIGN_OR_RAISE(auto reader, open_ipc(bytes));
        ARROW_ASSIGN_OR_RAISE(*batch, reader->Next());
        if (*batch == nullptr) {
            return arrow::Status::IOError(
                "streaming batch IPC buffer contained no record batch");
        }
        return arrow::Status::OK();
    }

    /// Decode the schema from a schema-only Arrow IPC buffer.
    static arrow::Status decode_schema(const ThetaDataDxArrowBytes& bytes,
                                       std::shared_ptr<arrow::Schema>* schema) {
        ARROW_ASSIGN_OR_RAISE(auto reader, open_ipc(bytes));
        *schema = reader->schema();
        return arrow::Status::OK();
    }

    /// Open an arrow-cpp IPC stream reader over a COPY of the FFI byte
    /// buffer.
    ///
    /// The copy is deliberate: the decoded `RecordBatch` can alias the input
    /// buffer's memory zero-copy, but the FFI buffer (`bytes`) is freed by
    /// the caller as soon as decode returns. Copying into an arrow-owned
    /// buffer hands the batch a lifetime tied to the arrow buffer's
    /// refcount, not to the FFI allocation, so the returned batch stays valid
    /// after the FFI buffer is freed. A per-batch copy is the correct,
    /// leak-free ownership boundary here.
    static arrow::Result<std::shared_ptr<arrow::ipc::RecordBatchStreamReader>> open_ipc(
        const ThetaDataDxArrowBytes& bytes) {
        ARROW_ASSIGN_OR_RAISE(auto buffer,
                              arrow::AllocateBuffer(static_cast<int64_t>(bytes.len)));
        if (bytes.len > 0 && bytes.data != nullptr) {
            std::memcpy(buffer->mutable_data(), bytes.data, bytes.len);
        }
        std::shared_ptr<arrow::Buffer> shared_buffer = std::move(buffer);
        auto input = std::make_shared<arrow::io::BufferReader>(shared_buffer);
        return arrow::ipc::RecordBatchStreamReader::Open(input);
    }

    ThetaDataDxRecordBatchStream* handle_;
    std::shared_ptr<arrow::Schema> schema_;
};
#endif // THETADATADX_CPP_ARROW

/// Real-time-streaming sub-namespace returned by `Client::stream()`.
///
/// Borrows the unified `ThetaDataDxClient*` for the duration of the borrow
/// and shares the parent `Client`'s `CallbackState`, so `set_callback` /
/// `stop_streaming` / `reconnect` observe the same registration the
/// unified client manages. The handle pointer is borrowed and must not
/// outlive the parent `Client` (`Client::stream()` is lvalue-only, so the
/// borrow cannot bind a temporary), but the callback state is held by
/// shared ownership: the registered node stays alive and at a fixed
/// address while either the `Client` or this view references it, so a
/// `Client` move never dangles the registered callback `ctx`. Replacing
/// the callback installs a fresh node in the shared state, so both the
/// `Client` and this view observe the new registration.
class Stream {
public:
    Stream(const Stream&) = delete;
    Stream& operator=(const Stream&) = delete;
    Stream(Stream&&) = default;
    Stream& operator=(Stream&&) = delete;

    /** Register a streaming push callback and open the streaming session.
     *  `fn` runs on the consumer thread inside an isolation boundary, never
     *  on the streaming reader. The reader thread cannot be blocked by
     *  user code: on ring overflow events are dropped and counted via
     *  `dropped_event_count()`. Throws on registration failure.
     *
     *  ## Callback storage + thread affinity
     *
     *  The parent `Client` and this view share a `CallbackState` by
     *  `shared_ptr`. The state owns the currently-registered `CallbackSlot`
     *  node; the address of that node's `fn` member is the `void* ctx`
     *  registered with the dispatcher. A registered node is never mutated
     *  in place: it lives at a fixed heap address for its whole life, so the
     *  ctx stays valid across a `Client` move and every consumer-thread
     *  invocation. Destruction routes through `thetadatadx_client_free` on
     *  the parent, which performs the shutdown + drain barrier internally,
     *  and replacement here calls `thetadatadx_client_stop_streaming`
     *  followed by `thetadatadx_client_await_drain(5000)` before retiring
     *  the old node, so no thread can observe a dangling ctx.
     *
     *  ## Lifecycle contract (unified replace-allowed rule)
     *
     *  Unlike `StreamingClient::set_callback` (one-shot), the unified path
     *  permits stop+register as a normal user flow: after
     *  `stop_streaming()` another `set_callback` REPLACES the saved
     *  `(callback, ctx)`. `reconnect()` is built on top of this. Calling
     *  `set_callback` on a live (running) session also replaces — the
     *  previous (callback, ctx) is drained out before the new one is wired
     *  in, with the same `await_drain(5000)` budget.
     *
     *  A replacement always installs a FRESH node and registers that node's
     *  distinct `&fn`; it never reuses or mutates the previously-registered
     *  node's storage. If the drain barrier times out (a wedged previous
     *  consumer still firing), the old node is kept alive off this path
     *  until that consumer quiesces, so the still-firing consumer keeps
     *  dereferencing its own node's stable, unchanged `fn` and never
     *  observes a torn or null function. Because the new node is installed
     *  into the shared `CallbackState`, both the `Client` and this view see
     *  it afterward. */
    void set_callback(std::function<void(const StreamEvent&)> fn) {
        // Replacing a live registration: stop the session and wait for the
        // consumer thread to stop firing through the old node. Matches the
        // C ABI's replace-allowed contract, which requires that a fresh
        // callback's storage must not alias a still-running previous
        // registration.
        if (callback_->slot->fn) {
            thetadatadx_client_stop_streaming(handle_.get());
            int drained = thetadatadx_client_await_drain(handle_.get(), 5000);
            if (drained == 0) {
                // Drain barrier timed out: the previous consumer may still
                // be invoking through the old node's `&fn`. We must NOT
                // mutate or free that node here. Hand the retired node (held
                // by `shared_ptr`) to a helper thread that drops it only
                // after the consumer is CONFIRMED quiesced, polling the same
                // drain barrier rather than guessing a wall-clock window.
                //
                // The reclaimer takes SHARED ownership of both the handle
                // and the callback state, matching the move-assign
                // reclaimers above. This is the one path that can outlive
                // this view AND an API-legal single-threaded `Client`
                // destruction: once `set_callback` returns, nothing pins the
                // borrowed handle, so a reclaimer holding only a raw pointer
                // would poll the drain barrier (and the handle's `_free`
                // would later read its own state) on freed memory. Holding a
                // `shared_ptr` to the handle defers `thetadatadx_client_free`
                // until the reclaimer also releases it; holding a
                // `shared_ptr` to the callback state keeps the CURRENT node
                // alive, so when that deferred free runs its drain barrier
                // still observes a live registered `ctx`. `raw` is borrowed
                // for the poll from the handle reference the `release`
                // closure keeps alive for the reclaimer's whole life.
                const ThetaDataDxClient* raw = handle_.get();
                detail::reclaim_after_drain(
                    [raw]() {
                        return thetadatadx_client_await_drain(
                                   raw,
                                   static_cast<uint64_t>(
                                       detail::kReclaimPollStep.count())) == 1;
                    },
                    [retired = callback_->slot,
                     retired_handle = handle_,
                     retired_state = callback_]() mutable {
                        // Confirmed quiescence. Drop the retired node first
                        // (the old consumer has stopped firing through it).
                        // Then release this reclaimer's handle reference: if
                        // it is the last one, the unified deleter's drain
                        // barrier runs here while the current node, held by
                        // `retired_state`, is still alive. Drop the callback
                        // state last so that barrier always sees a live
                        // registered `ctx` — the handle-before-callback
                        // ordering the member layout encodes.
                        retired.reset();
                        retired_handle.reset();
                        retired_state.reset();
                    });
            }
            // Install a fresh node so the new registration below gets a
            // distinct `&fn` the retired consumer never captured. On the
            // drained path the old node has no live reader, so its
            // `shared_ptr` simply drops here with no detach and no leak.
            callback_->slot = std::make_shared<CallbackSlot>();
        }
        // Stage the new callback into the fresh node, then register its
        // fixed `&fn` address as the dispatcher `ctx`. On failure the node's
        // `fn` is left cleared so no stale registration lingers.
        callback_->slot->fn = std::move(fn);
        int rc = thetadatadx_client_set_callback(handle_.get(), &Stream::callback_shim, &callback_->slot->fn);
        if (rc < 0) {
            callback_->slot->fn = nullptr;
            detail::throw_last_ffi_error();
        }
    }

    /// Polymorphic subscribe — primary fluent entry point. Defined inline
    /// below the fluent class declarations.
    inline void subscribe(const class FluentSubscription& sub) const;

    /// Bulk-subscribe an initializer list of `Subscription` values.
    /// Stops at the first error and throws.
    inline void subscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    inline void unsubscribe(const class FluentSubscription& sub) const;

    /// Bulk-unsubscribe an initializer list of `Subscription` values.
    inline void unsubscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    /// Stop streaming. Historical access remains available. Pair with
    /// `await_drain()` if you need to confirm the consumer thread has
    /// finished firing the registered callback before dropping any
    /// captured state.
    void stop_streaming() {
        if (handle_) {
            thetadatadx_client_stop_streaming(handle_.get());
        }
    }

    /// Reconnect streaming and re-apply every previously active
    /// subscription. Throws on failure — the wrapped C ABI sets the
    /// last-error slot on `-1` return.
    void reconnect() {
        int rc = thetadatadx_client_reconnect(handle_.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
    }

    /// Block until the previous consumer thread has finished firing the
    /// registered callback. Returns true on drain, false on timeout. Pass
    /// the same 5 s budget the FFI free path uses unless you have a
    /// specific reason to deviate.
    bool await_drain(std::chrono::milliseconds timeout) {
        const uint64_t ms = timeout.count() < 0
                                ? 0
                                : static_cast<uint64_t>(timeout.count());
        return thetadatadx_client_await_drain(handle_.get(), ms) == 1;
    }

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the bounded ring because the consumer fell behind and
    /// the ring was full. Returns 0 when no callback has been installed
    /// yet.
    uint64_t dropped_event_count() const {
        return handle_ ? thetadatadx_client_dropped_events(handle_.get()) : 0;
    }

    /// Point-in-time count of streaming events published into the event
    /// ring but not yet drained into the registered callback — the
    /// in-flight depth between the I/O thread and the dispatcher. Rising
    /// occupancy that approaches ring_capacity() predicts drops before
    /// dropped_event_count() moves; sampling never blocks the feed and is
    /// safe from any thread. Returns 0 when no callback has been installed
    /// yet.
    uint64_t ring_occupancy() const {
        return handle_ ? thetadatadx_client_ring_occupancy(handle_.get()) : 0;
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// streaming_ring_size setting, a power of two) — the fixed denominator for
    /// ring_occupancy(). Returns 0 when no callback has been installed yet.
    uint64_t ring_capacity() const {
        return handle_ ? thetadatadx_client_ring_capacity(handle_.get()) : 0;
    }

    /** Milliseconds since the most recent inbound streaming frame of any
     *  kind. Returns 0 on success with the value in *out_ms, 1 when
     *  streaming has not started or no frame has been received yet, -1 on a
     *  null handle. */
    int32_t millis_since_last_event(uint64_t* out_ms) const {
        return handle_ ? thetadatadx_client_millis_since_last_event(handle_.get(), out_ms) : -1;
    }

    /** UNIX-nanosecond receive timestamp of the most recent inbound
     *  streaming frame. 0 when streaming has not started or no frame has
     *  arrived yet. */
    int64_t last_event_received_at_unix_nanos() const {
        return handle_ ? thetadatadx_client_last_event_received_at_unix_nanos(handle_.get()) : 0;
    }

    /** Address (host:port) of the streaming server the current session is
     *  connected to, following the session across auto-reconnects. Empty
     *  when streaming has not started. */
    std::string last_connected_addr() const {
        if (!handle_) return {};
        char* raw = thetadatadx_client_last_connected_addr(handle_.get());
        if (!raw) return {};
        std::string out(raw);
        thetadatadx_string_free(raw);
        return out;
    }

    /// `true` iff the streaming session is currently live (set_callback ran
    /// and stop_streaming / terminal close has not).
    bool is_streaming() const {
        return handle_ && thetadatadx_client_is_streaming(handle_.get()) == 1;
    }

    /// `true` iff the live streaming session is currently authenticated.
    /// Distinct from is_streaming(): the session can be live yet briefly
    /// unauthenticated mid-reconnect. Mirrors the Python / TypeScript
    /// `client.stream.is_authenticated` placement and the standalone
    /// `StreamingClient::is_authenticated()`.
    bool is_authenticated() const {
        return handle_ && thetadatadx_client_is_authenticated(handle_.get()) == 1;
    }

    /// Snapshot the currently-active per-contract subscriptions. Throws on
    /// FFI error.
    std::vector<Subscription> active_subscriptions() const {
        return detail::subscription_array_to_vector(thetadatadx_client_active_subscriptions(handle_.get()));
    }

    /// Cumulative count of user-callback failures contained by the
    /// per-invocation isolation boundary since the current stream started.
    /// If the callback aborts on a given event, the failure is contained,
    /// recorded here, and does not stop event delivery — the next event
    /// continues normally. Returns 0 when no callback has been installed
    /// yet. Safe to call from any thread without blocking. Mirrors the
    /// Python / TypeScript `client.stream.panic_count` placement.
    uint64_t panic_count() const {
        return handle_ ? thetadatadx_client_panic_count(handle_.get()) : 0;
    }

    /// Set the slow-callback wall-clock threshold in microseconds. When a
    /// callback invocation runs longer than threshold_us,
    /// slow_callback_count() increments and a rate-limited warning is logged.
    /// Pass 0 to disable. Observability-only: the watchdog never cancels the
    /// callback. No-op when no callback has been installed yet. Mirrors the
    /// Python / TypeScript `client.stream.set_slow_callback_threshold_us`
    /// placement.
    void set_slow_callback_threshold_us(uint64_t threshold_us) const {
        if (handle_) {
            thetadatadx_client_set_slow_callback_threshold_us(handle_.get(), threshold_us);
        }
    }

    /// Cumulative count of user-callback invocations whose wall-clock duration
    /// exceeded the threshold set via set_slow_callback_threshold_us(). Returns
    /// 0 when the watchdog is disabled or no callback has been installed yet.
    /// Mirrors the Python / TypeScript `client.stream.slow_callback_count`
    /// placement.
    uint64_t slow_callback_count() const {
        return handle_ ? thetadatadx_client_slow_callback_count(handle_.get()) : 0;
    }

    /// Snapshot the currently-active full-stream subscriptions (the entire
    /// universe for a given sec_type + kind, not bound to a single
    /// contract). Throws on FFI error. Mirrors the Python / TypeScript
    /// `client.stream.active_full_subscriptions` placement.
    std::vector<FullSubscription> active_full_subscriptions() const {
        ThetaDataDxSubscriptionArray* arr = thetadatadx_client_active_full_subscriptions(handle_.get());
        if (arr == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<FullSubscription> out;
        if (arr->data != nullptr && arr->len > 0) {
            out.reserve(arr->len);
            for (size_t i = 0; i < arr->len; ++i) {
                const ThetaDataDxSubscription& s = arr->data[i];
                out.push_back(FullSubscription{
                    s.kind ? std::string(s.kind) : std::string(),
                    s.contract ? std::string(s.contract) : std::string(),
                });
            }
        }
        thetadatadx_subscription_array_free(arr);
        return out;
    }

#ifdef THETADATADX_CPP_ARROW
    /// Open a pull-based columnar reader over the live stream — a sibling to
    /// the per-event `set_callback`.
    ///
    /// Returns a `std::shared_ptr<arrow::RecordBatchReader>` (a
    /// `RecordBatchStream`): `ReadNext(&batch)` yields the next batch and
    /// sets `batch` to `nullptr` at clean end of stream, `schema()` reports
    /// the fixed schema, and `dropped()` (on the concrete
    /// `thetadatadx::RecordBatchStream`) reports the drop-oldest count. The
    /// reader closes (unsubscribe + tear down) when the last reference drops
    /// (RAII). The same subscriptions feed it; subscribe first, then open.
    ///
    /// `batch_size` rows per batch (default 65536). `linger` flushes a
    /// partial batch on a quiet stream (default 50 ms). `backpressure`
    /// selects lossless block (default) or bounded drop-oldest with
    /// `capacity` buffered batches.
    ///
    /// Only available when the SDK is built with `THETADATADX_CPP_ARROW`
    /// (which links arrow-cpp). Throws on a connect / start failure.
    std::shared_ptr<RecordBatchStream> batches(
        std::size_t batch_size = 65536,
        std::chrono::milliseconds linger = std::chrono::milliseconds(50),
        Backpressure backpressure = Backpressure::Block,
        std::size_t capacity = 4) const {
        const int32_t bp = backpressure == Backpressure::DropOldest
                               ? THETADATADX_BACKPRESSURE_DROP_OLDEST
                               : THETADATADX_BACKPRESSURE_BLOCK;
        const uint64_t linger_ms = linger.count() < 0
                                       ? 0
                                       : static_cast<uint64_t>(linger.count());
        ThetaDataDxRecordBatchStream* raw = thetadatadx_client_batches_open(
            handle_.get(), batch_size, linger_ms, bp, capacity);
        if (raw == nullptr) {
            detail::throw_last_ffi_error();
        }
        return RecordBatchStream::create(raw);
    }
#endif // THETADATADX_CPP_ARROW

private:
    friend class Client;
    Stream(std::shared_ptr<ThetaDataDxClient> h, std::shared_ptr<CallbackState> callback)
        : handle_(std::move(h)), callback_(std::move(callback)) {}

    // Static-member shim that the dispatcher invokes. It keeps C++ language
    // linkage (a member cannot be `extern "C"`) but its signature matches the
    // C-linkage `ThetaDataDxStreamCallback` typedef, an assignment every
    // mainstream ABI accepts. `ctx` is the `std::function*` we registered
    // alongside the callback. The event pointer is non-null and valid only for
    // the duration of this call.
    static void callback_shim(const ThetaDataDxStreamEvent* event, void* ctx) noexcept {
        auto* fn = static_cast<std::function<void(const StreamEvent&)>*>(ctx);
        if (fn == nullptr || event == nullptr) return;
        try {
            (*fn)(*event);
        } catch (...) {
            // User callbacks must not propagate exceptions across the C ABI
            // boundary — unwinding across it is undefined behavior. Swallow.
        }
    }

    // Shared ownership of the parent `Client`'s handle. Sharing (rather than
    // borrowing a raw pointer) lets the drain-timeout reclaimer in
    // `set_callback` keep the handle alive past this view and past a
    // single-threaded `Client` destruction, so the reclaimer's drain-barrier
    // poll never reads a freed handle. The unified deleter still frees the
    // handle exactly once, when the last reference drops.
    std::shared_ptr<ThetaDataDxClient> handle_;
    // Shared ownership of the parent `Client`'s callback state. The state
    // owns the currently-registered node, which lives at a fixed heap
    // address, so the registered dispatcher `ctx` (`&callback_->slot->fn`)
    // stays valid across a `Client` move and for this view's whole
    // lifetime. A replacement installs a fresh node into the shared state,
    // which both the `Client` and this view then observe.
    std::shared_ptr<CallbackState> callback_;
};

/// Forward declaration for `Client::builder()`; defined immediately after
/// `Client` so it can reach the public `Client::connect`.
class ClientBuilder;

/// RAII wrapper around a unified client handle (`ThetaDataDxClient*`).
/// The unified handle owns both the historical (gRPC) and
/// streaming sub-clients. Historical queries are reached through
/// `client.historical()` (the `Historical` view) and the real-time
/// streaming surface through `client.stream()` (the `Stream` view); the
/// FLATFILES surface stays on the client directly via `flat_files()`. For
/// pure-historical gRPC use, `HistoricalClient` remains the recommended
/// entry point.
class Client {
public:
    /// Connect a unified client (historical + streaming through one
    /// handle).
    /// @param creds Authenticated credentials.
    /// @param config Client configuration.
    /// @return A connected, owning `Client`.
    /// @throws thetadatadx::ThetaDataError (or a typed leaf) on an
    ///         authentication or handshake failure.
    static Client connect(const Credentials& creds, const Config& config) {
        ThetaDataDxClient* h = thetadatadx_client_connect(creds.get(), config.get());
        if (h == nullptr) {
            detail::throw_last_ffi_error();
        }
        return Client(h);
    }

    /// Connect a unified client, loading credentials from a file.
    /// One-call equivalent of `Credentials::from_file(path)` followed
    /// by `connect`.
    /// @param path File whose first line is the email and second line
    ///        the password.
    /// @param config Client configuration; defaults to the production
    ///        preset.
    /// @return A connected, owning `Client`.
    /// @throws thetadatadx::ThetaDataError (or a typed leaf) on an unreadable
    ///         credentials file or an authentication / handshake failure.
    static Client from_file(const std::string& path,
                                       const Config& config = Config::production()) {
        ThetaDataDxClient* h = thetadatadx_client_connect_from_file(path.c_str(), config.get());
        if (h == nullptr) {
            detail::throw_last_ffi_error();
        }
        return Client(h);
    }

    /// Start a fluent `ClientBuilder` — the headline ergonomic for
    /// constructing a client with the API key (or email + password) and
    /// the target environment selected inline.
    ///
    /// The API key is a first-class, directly-passed argument
    /// (`ClientBuilder::api_key` and its env / `.env` siblings),
    /// distinct from the email + password pair. The lower-level typed
    /// path `Client::connect(creds, config)` stays available for power
    /// users; the builder composes the `Credentials` + `Config` and calls
    /// it.
    ///
    /// ```cpp
    /// auto client = thetadatadx::Client::builder()
    ///     .api_key("td1_example_key")
    ///     .stage()
    ///     .connect();
    /// ```
    ///
    /// Defined out-of-line below `ClientBuilder`.
    static ClientBuilder builder();

    Client(const Client&) = delete;
    Client& operator=(const Client&) = delete;
    Client(Client&& other) noexcept
        // Initialiser order MUST follow declaration order; see the
        // ordering invariant above the member declarations below.
        : callback_(std::move(other.callback_)),
          handle_(std::move(other.handle_)) {}
    /** Move-assign. The receiver may already hold a live streaming
     *  session whose consumer thread is invoking through the callback
     *  node. Drain the consumer before releasing the node, the same
     *  discipline as `StreamingClient::operator=`.
     *
     *  On drain timeout the consumer may still be firing through the
     *  retired node's registered `ctx`, so neither the node nor the handle
     *  backing the drain barrier may be released on this path. We hand BOTH
     *  the retired handle and the retired callback state to a helper thread
     *  that polls the same drain barrier and releases them only once the
     *  consumer is confirmed quiesced (handle first so its free-time barrier
     *  still sees a live ctx, then the node), with a bounded cap so a wedged
     *  callback cannot leak either. The node is held by shared ownership, so
     *  this keeps any outstanding `Stream` view's registered `ctx` valid for
     *  the consumer's remaining invocations. */
    Client& operator=(Client&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                thetadatadx_client_stop_streaming(handle_.get());
                int drained = thetadatadx_client_await_drain(handle_.get(), 5000);
                if (drained == 0) {
                    // Borrow the raw handle for the quiescence poll; ownership
                    // stays with `retired_handle` inside the reclaimer, which
                    // frees it only after the consumer has stopped firing.
                    const ThetaDataDxClient* raw = handle_.get();
                    detail::reclaim_after_drain(
                        [raw]() {
                            return thetadatadx_client_await_drain(
                                       raw,
                                       static_cast<uint64_t>(
                                           detail::kReclaimPollStep.count())) == 1;
                        },
                        [retired_handle = std::move(handle_),
                         retired_cb = std::move(callback_)]() mutable {
                            // Free the handle first so its internal drain
                            // barrier still observes a live `ctx`, then drop
                            // the callback state. Past confirmed quiescence
                            // both are no-ops in ordering terms; on the cap
                            // path this preserves the handle-before-callback
                            // invariant the member declaration order encodes.
                            retired_handle.reset();
                            retired_cb.reset();
                        });
                    // `handle_` and `callback_` are now empty; the
                    // reclaimer owns the retired session.
                }
            }
            callback_.reset();
            handle_ = std::move(other.handle_);
            callback_ = std::move(other.callback_);
        }
        return *this;
    }

    /// Namespace handle for the FLATFILES surface. Cheap — borrows the
    /// underlying C ABI handle, so the returned `FlatFiles` value borrows
    /// `*this` and must not outlive it.
    ///
    /// Lvalue-only: the accessor is ref-qualified to `const&`, so calling
    /// it on a temporary is a compile error. Bind the client to a variable
    /// first (`auto& c = ...; auto ff = c.flat_files();`); the view then
    /// cannot outlive its client.
    FlatFiles flat_files() const& { return FlatFiles(handle_.get()); }

    /// Historical-data sub-namespace: `client.historical().stock_history_eod(...)`.
    ///
    /// Returns a `Historical` view borrowing this client's handle. No auth
    /// round-trip, no second connection; the view borrows `*this` and must
    /// not outlive it.
    ///
    /// Lvalue-only: the accessor is ref-qualified to `const&`, so calling
    /// it on a temporary is a compile error. Bind the client to a variable
    /// first; the view then cannot outlive its client.
    Historical historical() const& { return Historical(handle_.get()); }

    /// Real-time-streaming sub-namespace: `client.stream().subscribe(...)`,
    /// `client.stream().set_callback(cb)`, …
    ///
    /// Returns a `Stream` view borrowing this client's handle and sharing
    /// this client's callback node, so the streaming lifecycle observed
    /// through the view is the one this client owns. The handle is borrowed
    /// and must not outlive `*this`; the callback node is held by shared
    /// ownership, so it survives a `Client` move.
    ///
    /// Lvalue-only: the accessor is ref-qualified to `&`, so calling it on
    /// a temporary is a compile error. Bind the client to a variable first;
    /// the view's borrowed handle then cannot outlive its client.
    Stream stream() & { return Stream(handle_, callback_); }

    /// Raw handle for advanced consumers that want to call the C ABI
    /// directly. Ownership remains with this object.
    const ThetaDataDxClient* get() const noexcept { return handle_.get(); }

private:
    explicit Client(ThetaDataDxClient* h)
        : callback_(std::make_shared<CallbackState>()),
          handle_(h, UnifiedDeleter{}) {}

    // ── Member ordering invariant (do not reorder) ──
    //
    // C++ destructs members in REVERSE declaration order. The C ABI
    // contract for `thetadatadx_client_free` is "drain the user-callback path
    // before this call returns" — the FFI's deleter runs that drain
    // barrier internally (5 s budget). For the barrier to be safe the
    // `std::function` storage backing the registered `void* ctx` MUST
    // still be alive while `thetadatadx_client_free` is polling the drain
    // flag, because the consumer thread may still be invoking through
    // it.
    //
    // We therefore declare `handle_` AFTER `callback_`: reverse-order
    // destruction destroys `handle_` first → `thetadatadx_client_free` runs
    // and its drain barrier returns → this client's reference to the
    // callback state (and the registered node it owns) is then released.
    // Reordering these two members reintroduces the use-after-free.
    //
    // The state is held by `shared_ptr` and owns the registered node, which
    // lives at a fixed address: a `Client` move transfers the reference
    // without moving the node, and a `Stream` view holds its own reference
    // to the same state. The registered `ctx` (`&CallbackSlot::fn`)
    // therefore stays valid across any move and for as long as any holder
    // lives. A callback replacement installs a fresh node into the shared
    // state, so a later move or destruction operates on the current node.
    //
    // The handle is held by `shared_ptr` (with the unified deleter) rather
    // than `unique_ptr`: a `Stream` view and a drain-timeout reclaimer can
    // hold their own references, so `thetadatadx_client_free` runs exactly
    // once, when the LAST reference drops. When that last reference is this
    // member during `~Client`, the deleter's drain barrier runs while
    // `callback_` (the current node) is still alive — the ordering this
    // member layout encodes. When instead a reclaimer outlives `~Client`,
    // the reclaimer also holds a `callback_` reference, so the deferred free
    // still observes a live current node (see `Stream::set_callback`).
    std::shared_ptr<CallbackState> callback_;
    std::shared_ptr<ThetaDataDxClient> handle_;
};

/// Fluent builder for `Client`, mirroring the Rust `ClientBuilder`.
///
/// The API key is a first-class, directly-passed argument: `api_key`,
/// `api_key_from_env`, and `api_key_from_dotenv` are distinct from the
/// `email_password` pair and from `credentials_file`. Set exactly one
/// authentication source plus an optional environment, then call
/// `connect()`.
///
/// ```cpp
/// auto client = thetadatadx::Client::builder()
///     .api_key("td1_example_key")
///     .stage()
///     .connect();
/// ```
///
/// Setting no authentication source, or two different ones, throws a
/// `ConfigError` from `connect()` before any network round-trip. The
/// builder holds the chosen source by value and resolves it (env / file
/// reads, then the gRPC handshake) only when `connect()` is called.
///
/// `connect()` consumes the builder: it is rvalue-ref-qualified and so is
/// single-use, mirroring the Rust `ClientBuilder::connect(self)`. Chain it
/// inline as above, or, for a stored builder, hand it over with
/// `std::move(builder).connect()`.
class ClientBuilder {
public:
    // The builder is move-only. Copying would duplicate the inline secret
    // material (`auth_a_` / `auth_b_`) and would let a copy observe the
    // moved-from credential / config state after `connect()` ran on its
    // sibling, since `connect()` moves out of the shared members. Deleting
    // the copy operations makes that class of bug impossible: the fluent
    // rvalue chain (`Client::builder().api_key(k).stage().connect()`) is
    // unaffected, and a stored builder is handed over with
    // `std::move(b).connect()`.
    ClientBuilder(const ClientBuilder&) = delete;
    ClientBuilder& operator=(const ClientBuilder&) = delete;
    ClientBuilder(ClientBuilder&&) = default;
    ClientBuilder& operator=(ClientBuilder&&) = default;

    /// Best-effort wipe of any inline secret material the builder still
    /// holds, so a builder that is destroyed without `connect()` (or after
    /// it) does not leave plaintext credentials in process memory longer
    /// than necessary.
    ~ClientBuilder() {
        detail::secure_wipe(auth_a_);
        detail::secure_wipe(auth_b_);
        detail::secure_wipe(env_path_);
    }

    // Each fluent setter mutates the builder in place and returns it with
    // the value category preserved: an lvalue overload (`&`) returns
    // `ClientBuilder&` so a named builder keeps chaining, and an rvalue
    // overload (`&&`) returns `ClientBuilder&&` so a chain that starts from
    // `Client::builder()` stays an rvalue all the way into the
    // rvalue-only `connect()`. Preserving the category is what lets the
    // documented `Client::builder().api_key(k).stage().connect()` form
    // compile while keeping the single-use guarantee.

    /// Authenticate with an inline API key — the primary, directly-passed
    /// auth argument.
    ClientBuilder& api_key(const std::string& key) & {
        set_auth(AuthKind::ApiKey, key, std::string(), "api_key");
        return *this;
    }
    ClientBuilder&& api_key(const std::string& key) && {
        set_auth(AuthKind::ApiKey, key, std::string(), "api_key");
        return std::move(*this);
    }

    /// Source the API key from the `THETADATA_API_KEY` environment
    /// variable, read at `connect()` time. Strict: an unset or
    /// whitespace-only value throws `ConfigError` from `connect()`; there
    /// is no `creds.txt` file fallback.
    ClientBuilder& api_key_from_env() & {
        set_auth(AuthKind::ApiKeyFromEnv, std::string(), std::string(),
                 "api_key_from_env");
        return *this;
    }
    ClientBuilder&& api_key_from_env() && {
        set_auth(AuthKind::ApiKeyFromEnv, std::string(), std::string(),
                 "api_key_from_env");
        return std::move(*this);
    }

    /// Source the credential from a `.env`-format file at `connect()`
    /// time. `THETADATA_API_KEY` selects an API key, otherwise
    /// `THETADATA_EMAIL` + `THETADATA_PASSWORD` build email + password
    /// credentials.
    ClientBuilder& api_key_from_dotenv(const std::string& path) & {
        set_auth(AuthKind::Dotenv, path, std::string(),
                 "api_key_from_dotenv / from_dotenv");
        return *this;
    }
    ClientBuilder&& api_key_from_dotenv(const std::string& path) && {
        set_auth(AuthKind::Dotenv, path, std::string(),
                 "api_key_from_dotenv / from_dotenv");
        return std::move(*this);
    }

    /// Authenticate with an inline email + password pair.
    ClientBuilder& email_password(const std::string& email, const std::string& password) & {
        set_auth(AuthKind::EmailPassword, email, password, "email_password");
        return *this;
    }
    ClientBuilder&& email_password(const std::string& email, const std::string& password) && {
        set_auth(AuthKind::EmailPassword, email, password, "email_password");
        return std::move(*this);
    }

    /// Authenticate from a two-line `creds.txt` file (line 1 = email,
    /// line 2 = password), read at `connect()` time.
    ClientBuilder& credentials_file(const std::string& path) & {
        set_auth(AuthKind::CredentialsFile, path, std::string(), "credentials_file");
        return *this;
    }
    ClientBuilder&& credentials_file(const std::string& path) && {
        set_auth(AuthKind::CredentialsFile, path, std::string(), "credentials_file");
        return std::move(*this);
    }

    /// Authenticate with a pre-built `Credentials` value — the escape
    /// hatch that covers every existing factory.
    ClientBuilder& credentials(Credentials creds) & {
        set_credentials(std::move(creds));
        return *this;
    }
    ClientBuilder&& credentials(Credentials creds) && {
        set_credentials(std::move(creds));
        return std::move(*this);
    }

    /// Select the historical environment by its binding label
    /// (`"PROD"` or `"STAGE"`, case-insensitive). The historical and
    /// streaming channels are chosen independently, so this composes with a
    /// streaming selection — `.streaming_environment(..).historical_environment(..)`
    /// keeps both. For example, to target historical staging and streaming dev
    /// in one builder (the explicit form of the `.stage().dev()` shorthand):
    ///
    /// ```cpp
    /// auto client = thetadatadx::Client::builder()
    ///                   .historical_environment("STAGE")
    ///                   .streaming_environment("DEV")
    ///                   .connect();
    /// ```
    ClientBuilder& historical_environment(const std::string& environment) & {
        set_historical_environment(environment);
        return *this;
    }
    ClientBuilder&& historical_environment(const std::string& environment) && {
        set_historical_environment(environment);
        return std::move(*this);
    }

    /// Select the streaming environment by its binding label
    /// (`"PROD"` or `"DEV"`, case-insensitive). Composes with a historical
    /// selection.
    ClientBuilder& streaming_environment(const std::string& environment) & {
        set_streaming_environment(environment);
        return *this;
    }
    ClientBuilder&& streaming_environment(const std::string& environment) && {
        set_streaming_environment(environment);
        return std::move(*this);
    }

    /// Target the historical staging cluster (streaming stays on
    /// production). Shorthand for `historical_environment("STAGE")`.
    ClientBuilder& stage() & {
        select_historical(HistoricalKind::Stage);
        return *this;
    }
    ClientBuilder&& stage() && {
        select_historical(HistoricalKind::Stage);
        return std::move(*this);
    }

    /// Target the streaming dev-replay cluster (historical stays on
    /// production). Shorthand for `streaming_environment("DEV")`.
    ClientBuilder& dev() & {
        select_streaming(StreamingKind::Dev);
        return *this;
    }
    ClientBuilder&& dev() && {
        select_streaming(StreamingKind::Dev);
        return std::move(*this);
    }

    /// Target production on both channels (the default).
    ClientBuilder& production() & {
        select_historical(HistoricalKind::Production);
        select_streaming(StreamingKind::Production);
        return *this;
    }
    ClientBuilder&& production() && {
        select_historical(HistoricalKind::Production);
        select_streaming(StreamingKind::Production);
        return std::move(*this);
    }

    /// Use a fully built `Config` verbatim. The config and the per-channel
    /// environment setters resolve in call order, last one wins: this config
    /// replaces an earlier `stage()` / `dev()` / `production()` /
    /// `historical_environment(..)` / `streaming_environment(..)` selection,
    /// and a later such preset setter replaces this config.
    ClientBuilder& config(Config cfg) & {
        set_config(std::move(cfg));
        return *this;
    }
    ClientBuilder&& config(Config cfg) && {
        set_config(std::move(cfg));
        return std::move(*this);
    }

    /// Source both the credential and the target environment from a
    /// `.env`-format file. Reuses `Credentials::from_dotenv` and
    /// `Config::from_dotenv`, so one file can carry both
    /// `THETADATA_API_KEY` and `THETADATA_HISTORICAL_TYPE`.
    ClientBuilder& from_dotenv(const std::string& path) & {
        set_auth(AuthKind::Dotenv, path, std::string(),
                 "api_key_from_dotenv / from_dotenv");
        set_env_from_dotenv(path);
        return *this;
    }
    ClientBuilder&& from_dotenv(const std::string& path) && {
        set_auth(AuthKind::Dotenv, path, std::string(),
                 "api_key_from_dotenv / from_dotenv");
        set_env_from_dotenv(path);
        return std::move(*this);
    }

    /// Build the `Credentials` + `Config` and connect.
    ///
    /// Consumes the builder: `connect()` is rvalue-ref-qualified, so it can
    /// only be invoked on a temporary or a `std::move`-d builder, and a
    /// named builder is single-use. This mirrors the Rust `ClientBuilder`,
    /// whose `connect(self)` takes the builder by value. Call it inline,
    /// `Client::builder().api_key(k).stage().connect()`, or on a stored
    /// builder via `std::move(b).connect()`.
    ///
    /// @return A connected, owning `Client`.
    /// @throws thetadatadx::ConfigError when no authentication source was
    ///         set or when two different sources were set (a conflict),
    ///         before any network round-trip. Otherwise throws on a
    ///         credential-resolution or handshake failure.
    Client connect() && {
        if (conflict_) {
            detail::throw_config_error(
                "conflicting authentication sources: " + first_label_ + " and " +
                second_label_ + " were both set; set exactly one");
        }
        if (auth_kind_ == AuthKind::Unset) {
            detail::throw_config_error(
                "no authentication source set — call one of api_key, api_key_from_env, "
                "api_key_from_dotenv, from_dotenv, email_password, credentials_file, "
                "or credentials");
        }
        // The builder is an about-to-expire rvalue, so the resolvers move
        // the chosen credential and config straight out of the members. The
        // builder is move-only (copy is deleted), so no sibling copy can
        // observe a moved-from source.
        Credentials creds = resolve_credentials();
        // The credential now lives in the `Credentials` handle, whose Rust
        // core holds the authoritative secret in zeroizing memory. Wipe the
        // transient inline plaintext immediately rather than waiting for the
        // destructor, shortening its lifetime to the minimum.
        detail::secure_wipe(auth_a_);
        detail::secure_wipe(auth_b_);
        Config cfg = resolve_config();
        detail::secure_wipe(env_path_);
        return Client::connect(creds, cfg);
    }

private:
    friend class Client;
    ClientBuilder() = default;

    enum class AuthKind {
        Unset,
        ApiKey,
        ApiKeyFromEnv,
        Dotenv,
        EmailPassword,
        CredentialsFile,
        Prebuilt,
    };
    /// Environment SOURCE mode. `Preset` composes the per-channel
    /// selections below on top of the production defaults; `Config` uses a
    /// caller-supplied `Config` verbatim; `Dotenv` reads the environment
    /// from a `.env` file at connect time. The source and the per-channel
    /// preset setters resolve in call order, last one wins on the kind:
    /// a later `config()` / `from_dotenv()` replaces a preset selection,
    /// and a later preset setter (`stage()` / `dev()` /
    /// `historical_environment(..)` / `streaming_environment(..)`) replaces
    /// a `config()` / `from_dotenv()` source.
    enum class EnvKind { Preset, Config, Dotenv };

    /// Per-channel preset selections, mirroring the independent historical
    /// and streaming channels. Both default to production.
    enum class HistoricalKind { Production, Stage };
    enum class StreamingKind { Production, Dev };

    /// Record an auth source, rejecting a second different one. Re-stating
    /// the same kind overwrites; a different kind latches a conflict that
    /// `connect()` reports.
    void set_auth(AuthKind kind, const std::string& a, const std::string& b,
                  const char* label) {
        record_auth(kind, label);
        if (!conflict_) {
            auth_kind_ = kind;
            auth_a_ = a;
            auth_b_ = b;
            prebuilt_.reset();
        }
    }

    /// Store a pre-built `Credentials` source, latching a conflict if a
    /// different source was already chosen.
    void set_credentials(Credentials creds) {
        record_auth(AuthKind::Prebuilt, "credentials");
        if (!conflict_) {
            auth_kind_ = AuthKind::Prebuilt;
            prebuilt_ = std::make_shared<Credentials>(std::move(creds));
        }
    }

    /// Store a fully built `Config`, selecting the verbatim-config source.
    void set_config(Config cfg) {
        env_kind_ = EnvKind::Config;
        detail::secure_wipe(env_path_);
        config_ = std::make_shared<Config>(std::move(cfg));
    }

    /// Select the historical channel by its string label
    /// (`"PROD"` / `"STAGE"`), rejecting anything else as a
    /// client-construction config error. The streaming channel is left
    /// untouched.
    void set_historical_environment(const std::string& environment) {
        select_historical(parse_historical_kind(environment));
    }

    /// Select the streaming channel by its string label
    /// (`"PROD"` / `"DEV"`), rejecting anything else as a
    /// client-construction config error. The historical channel is left
    /// untouched.
    void set_streaming_environment(const std::string& environment) {
        select_streaming(parse_streaming_kind(environment));
    }

    /// Store a `.env` file as the environment source; the same `path`
    /// is also valid for the auth source when `from_dotenv(...)` chooses
    /// the `.env` credential path.
    void set_env_from_dotenv(const std::string& path) {
        env_kind_ = EnvKind::Dotenv;
        env_path_ = path;
        config_.reset();
    }

    /// Record a historical-channel preset selection, switching the source
    /// to the preset mode and clearing any prior verbatim-config or `.env`
    /// override while preserving the streaming-channel selection.
    void select_historical(HistoricalKind kind) {
        switch_to_preset();
        historical_ = kind;
    }

    /// Record a streaming-channel preset selection, switching the source to
    /// the preset mode and clearing any prior verbatim-config or `.env`
    /// override while preserving the historical-channel selection.
    void select_streaming(StreamingKind kind) {
        switch_to_preset();
        streaming_ = kind;
    }

    /// Switch to the preset environment source, clearing any prior
    /// verbatim-config or `.env` override. The per-channel selections are
    /// preserved so `.stage().dev()` composes to historical-staging +
    /// streaming-dev.
    void switch_to_preset() {
        env_kind_ = EnvKind::Preset;
        detail::secure_wipe(env_path_);
        config_.reset();
    }

    /// Trim and upper-case a string environment label.
    static std::string normalize_label(const std::string& environment) {
        const auto first = environment.find_first_not_of(" \t\r\n");
        if (first == std::string::npos) {
            return std::string();
        }
        const auto last = environment.find_last_not_of(" \t\r\n");
        std::string normalized = environment.substr(first, last - first + 1);
        for (char& ch : normalized) {
            ch = static_cast<char>(std::toupper(static_cast<unsigned char>(ch)));
        }
        return normalized;
    }

    /// Parse a historical channel label (`"PROD"` / `"STAGE"`).
    static HistoricalKind parse_historical_kind(const std::string& environment) {
        const std::string normalized = normalize_label(environment);
        if (normalized == "PROD") {
            return HistoricalKind::Production;
        }
        if (normalized == "STAGE") {
            return HistoricalKind::Stage;
        }
        detail::throw_config_error(
            "historical environment must be PROD or STAGE; got \"" + environment + "\"");
    }

    /// Parse a streaming channel label (`"PROD"` / `"DEV"`).
    static StreamingKind parse_streaming_kind(const std::string& environment) {
        const std::string normalized = normalize_label(environment);
        if (normalized == "PROD") {
            return StreamingKind::Production;
        }
        if (normalized == "DEV") {
            return StreamingKind::Dev;
        }
        detail::throw_config_error(
            "streaming environment must be PROD or DEV; got \"" + environment + "\"");
    }

    /// Track the auth-source label so a second, different source can be
    /// reported as a conflict naming both sides.
    void record_auth(AuthKind kind, const char* label) {
        if (auth_kind_ == AuthKind::Unset && !conflict_) {
            first_label_ = label;
            return;
        }
        if (conflict_) {
            return;
        }
        // Same auth kind re-stated → not a conflict (overwrite).
        // Different auth kind → latch the conflict naming both sides.
        if (auth_kind_ != kind) {
            conflict_ = true;
            second_label_ = label;
        }
    }

    /// Non-const: the `Prebuilt` arm moves the stored `Credentials` out of
    /// the shared member, so this runs only from the rvalue-qualified
    /// `connect()` on an about-to-expire builder.
    Credentials resolve_credentials() {
        switch (auth_kind_) {
            case AuthKind::ApiKey:
                return Credentials::from_api_key(auth_a_);
            case AuthKind::ApiKeyFromEnv:
                return resolve_api_key_from_env();
            case AuthKind::Dotenv:
                return Credentials::from_dotenv(auth_a_);
            case AuthKind::EmailPassword:
                return Credentials::from_email(auth_a_, auth_b_);
            case AuthKind::CredentialsFile:
                return Credentials::from_file(auth_a_);
            case AuthKind::Prebuilt:
                // `connect()` only reaches here with a non-null prebuilt.
                return std::move(*prebuilt_);
            case AuthKind::Unset:
            default:
                detail::throw_config_error("no authentication source set");
                // throw_config_error never returns; satisfy the compiler.
                return Credentials::from_api_key("");
        }
    }

    /// Strict `THETADATA_API_KEY` env resolver, mirroring the Rust
    /// `ClientBuilder::api_key_from_env`. An unset or whitespace-only value
    /// is a configuration error rather than a silent fallback, because the
    /// caller explicitly asked for the environment source. No `creds.txt`
    /// file fallback.
    ///
    /// The transient `std::string` holding the env value is wiped through
    /// `detail::secure_wipe` before this returns: the authoritative secret
    /// then lives only in the `Credentials` handle, whose Rust core keeps it
    /// in zeroizing memory. The standalone strict resolver is also reachable
    /// as `Credentials::from_env()` (over the `thetadatadx_credentials_from_env`
    /// C ABI symbol) for callers outside the builder.
    static Credentials resolve_api_key_from_env() {
        const char* raw = std::getenv("THETADATA_API_KEY");
        if (raw == nullptr) {
            detail::throw_config_error(
                "THETADATA_API_KEY is not set in the environment");
        }
        std::string value(raw);
        const auto first = value.find_first_not_of(" \t\r\n");
        if (first == std::string::npos) {
            detail::secure_wipe(value);
            detail::throw_config_error("THETADATA_API_KEY is set but empty");
        }
        const auto last = value.find_last_not_of(" \t\r\n");
        // Build the credential (the Rust core takes the authoritative
        // zeroizing copy), then wipe both the trimmed key and the full env
        // value so the C++-side plaintext does not outlive this call.
        std::string trimmed = value.substr(first, last - first + 1);
        Credentials creds = Credentials::from_api_key(trimmed);
        detail::secure_wipe(trimmed);
        detail::secure_wipe(value);
        return creds;
    }

    /// Non-const: the `Config` arm moves the stored `Config` out of the
    /// shared member, so this runs only from the rvalue-qualified
    /// `connect()` on an about-to-expire builder.
    Config resolve_config() {
        switch (env_kind_) {
            case EnvKind::Config:
                return std::move(*config_);
            case EnvKind::Dotenv:
                return Config::from_dotenv(env_path_);
            case EnvKind::Preset:
            default: {
                // Compose the two independently-selected channels on top of
                // the production defaults, mirroring the Rust builder: the
                // historical and streaming environments are applied in turn,
                // so any combination (including historical-staging +
                // streaming-dev) is reachable. A channel left on production
                // is a no-op.
                Config cfg = Config::production();
                if (historical_ == HistoricalKind::Stage) {
                    if (thetadatadx_config_with_historical_environment(cfg.get(), 1) != 0) {
                        detail::throw_last_ffi_error();
                    }
                }
                if (streaming_ == StreamingKind::Dev) {
                    if (thetadatadx_config_with_streaming_environment(cfg.get(), 1) != 0) {
                        detail::throw_last_ffi_error();
                    }
                }
                return cfg;
            }
        }
    }

    AuthKind auth_kind_ = AuthKind::Unset;
    std::string auth_a_;
    std::string auth_b_;
    std::shared_ptr<Credentials> prebuilt_;

    EnvKind env_kind_ = EnvKind::Preset;
    HistoricalKind historical_ = HistoricalKind::Production;
    StreamingKind streaming_ = StreamingKind::Production;
    std::string env_path_;
    std::shared_ptr<Config> config_;

    bool conflict_ = false;
    std::string first_label_;
    std::string second_label_;
};

inline ClientBuilder Client::builder() { return ClientBuilder(); }

// ══════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════
// Fluent contract-first API
// ══════════════════════════════════════════════════════════════════════════
//
// The fluent contract-first surface mirrored across every binding:
//
//     auto stock  = thetadatadx::Contract::stock("AAPL");
//     auto option = thetadatadx::Contract::option("SPY", "20260620", "550", "C");
//     client.subscribe(stock.quote());
//     client.subscribe(option.trade());
//     client.subscribe(thetadatadx::SecType::option().full_trades());
//
// Pure-header layer over the existing C ABI subscribe entry points
// (`thetadatadx_client_subscribe` / `_unsubscribe`, polymorphic over
// `ThetaDataDxSubscriptionRequest`). No
// new C ABI symbols — the value type just routes the existing call
// dispatch by stored kind + payload.

/// Forward declaration of the fluent stock / option contract identifier.
class FluentContract;
/// Forward declaration of the typed market-data subscription value type.
class FluentSubscription;
/// Forward declaration of the fluent security-type accessor for full-stream subscriptions.
class FluentSecType;

/// Typed market-data subscription. Returned by `Contract::quote` /
/// `Contract::trade` / `Contract::open_interest` (per-contract) or by
/// `SecType::option().full_trades()` /
/// `.full_open_interest()` (full-stream). Pass into
/// `Client::subscribe(sub)` or `subscribe_many(...)`.
class FluentSubscription {
public:
    /// Whether the subscription targets a single contract or the full
    /// per-sec-type universe.
    enum class Scope { Contract, Full };
    /// The market-data feed kind the subscription carries.
    enum class Kind { Quote, Trade, OpenInterest, MarketValue };

    Scope scope() const noexcept { return scope_; }
    Kind kind() const noexcept { return kind_; }
    const std::string& symbol() const noexcept { return symbol_; }
    const std::string& expiration() const noexcept { return expiration_; }
    const std::string& strike() const noexcept { return strike_; }
    const std::string& right() const noexcept { return right_; }
    const std::string& sec_type() const noexcept { return sec_type_; }
    bool is_option() const noexcept { return is_option_; }

    /// Stable snake_case wire-kind label, identical to the Python /
    /// TypeScript `Subscription.kind` accessor and the C ABI
    /// active-subscription `kind` field. Per-contract subscriptions
    /// return `"quote"` / `"trade"` / `"open_interest"` /
    /// `"market_value"`; full-stream subscriptions carry the `full_`
    /// prefix (`"full_trades"` / `"full_open_interest"`) so a full-stream
    /// open-interest subscription never reads the same as a per-contract
    /// one.
    std::string kind_string() const {
        if (scope_ == Scope::Full) {
            // Only Trade and OpenInterest have a full-stream broadcast on
            // the streaming wire; Quote and MarketValue are per-contract only.
            // A full-stream `FluentSubscription` is therefore only ever
            // built for those two kinds (see `FluentSecType::full_trades`
            // / `full_open_interest`), and the label set is the same two
            // strings the Python / TypeScript `Subscription.kind` accessors
            // emit. The
            // remaining kinds fall through to the trade label so the
            // method never invents a non-canonical string.
            switch (kind_) {
                case Kind::OpenInterest: return "full_open_interest";
                case Kind::Trade:
                case Kind::Quote:
                case Kind::MarketValue:
                    break;
            }
            return "full_trades";
        }
        switch (kind_) {
            case Kind::Quote:        return "quote";
            case Kind::Trade:        return "trade";
            case Kind::OpenInterest: return "open_interest";
            case Kind::MarketValue:  return "market_value";
        }
        return "quote";
    }

private:
    friend class FluentContract;
    friend class FluentSecType;

    static FluentSubscription per_contract_stock(std::string symbol, std::string sec_type, Kind k) {
        FluentSubscription s;
        s.scope_ = Scope::Contract;
        s.kind_ = k;
        s.symbol_ = std::move(symbol);
        s.sec_type_ = std::move(sec_type);
        s.is_option_ = false;
        return s;
    }
    static FluentSubscription per_contract_option(
        std::string symbol, std::string expiration,
        std::string strike, std::string right, Kind k) {
        FluentSubscription s;
        s.scope_ = Scope::Contract;
        s.kind_ = k;
        s.symbol_ = std::move(symbol);
        s.expiration_ = std::move(expiration);
        s.strike_ = std::move(strike);
        s.right_ = std::move(right);
        s.is_option_ = true;
        return s;
    }
    static FluentSubscription full_stream(std::string sec_type, Kind k) {
        FluentSubscription s;
        s.scope_ = Scope::Full;
        s.kind_ = k;
        s.sec_type_ = std::move(sec_type);
        return s;
    }

    Scope scope_{Scope::Contract};
    Kind kind_{Kind::Quote};
    std::string symbol_;
    std::string expiration_;
    std::string strike_;
    std::string right_;
    std::string sec_type_;
    bool is_option_{false};
};

/// The expiration / strike / right of an option leg, passed to
/// `FluentContract::option(symbol, leg)` by name.
///
/// All three are strings, so a positional `(expiration, strike, right)`
/// argument list lets a transposed pair compile silently. Passing them as
/// named members — ideally via designated initialisers,
/// `thetadatadx::OptionLeg{.expiration = "20260620", .strike = "550", .right =
/// "C"}` — makes the contract identity non-transposable.
struct OptionLeg {
    /// Expiration date as `YYYYMMDD` (e.g. `"20260620"`).
    std::string expiration;
    /// Strike price in dollars (e.g. `"550"` or `"550.50"`).
    std::string strike;
    /// Option right: `"C"` / `"CALL"` / `"P"` / `"PUT"`
    /// (case-insensitive).
    std::string right;
};

/// Fluent contract identifier — stock or option.
class FluentContract {
public:
    /// Construct a stock contract.
    static FluentContract stock(std::string symbol) {
        return FluentContract{std::move(symbol), "STOCK", false, "", "", ""};
    }
    /// Construct an index contract. Routes through the stock-shape
    /// wire encoder; the C ABI layer treats them identically (no
    /// per-index subscribe call exists today). The security type is
    /// retained for rendering so an index contract reads `"INDEX"`
    /// rather than `"STOCK"`.
    static FluentContract index(std::string symbol) {
        return FluentContract{std::move(symbol), "INDEX", false, "", "", ""};
    }
    /// Construct an option contract. The expiration / strike / right
    /// travel in a single `OptionLeg` with named members —
    /// `Contract::option("SPY", {.expiration = "20260620", .strike =
    /// "550", .right = "C"})` — rather than as adjacent positional
    /// strings, so a swapped expiration/strike/right pair cannot pass
    /// silently. `right` accepts `"C"` / `"CALL"` / `"P"` / `"PUT"`
    /// (case-insensitive).
    static FluentContract option(std::string symbol, OptionLeg leg) {
        return FluentContract{std::move(symbol), "OPTION", true, std::move(leg.expiration),
                              std::move(leg.strike), std::move(leg.right)};
    }

    FluentSubscription quote() const {
        return make_subscription(FluentSubscription::Kind::Quote);
    }
    FluentSubscription trade() const {
        return make_subscription(FluentSubscription::Kind::Trade);
    }
    FluentSubscription open_interest() const {
        return make_subscription(FluentSubscription::Kind::OpenInterest);
    }
    /// Per-contract market-value subscription.
    FluentSubscription market_value() const {
        return make_subscription(FluentSubscription::Kind::MarketValue);
    }

    const std::string& symbol() const noexcept { return symbol_; }
    bool is_option() const noexcept { return is_option_; }
    /// Security type as a symbolic name (`"STOCK"` / `"INDEX"` /
    /// `"OPTION"`), retained so renderings distinguish an index contract
    /// from a stock one.
    const std::string& sec_type() const noexcept { return sec_type_; }
    const std::string& expiration() const noexcept { return expiration_; }
    const std::string& strike() const noexcept { return strike_; }
    const std::string& right() const noexcept { return right_; }

private:
    FluentContract(std::string symbol, std::string sec_type, bool is_option,
                   std::string expiration, std::string strike, std::string right)
        : symbol_(std::move(symbol)), sec_type_(std::move(sec_type)),
          is_option_(is_option), expiration_(std::move(expiration)),
          strike_(std::move(strike)), right_(std::move(right)) {}

    FluentSubscription make_subscription(FluentSubscription::Kind k) const {
        if (is_option_) {
            return FluentSubscription::per_contract_option(
                symbol_, expiration_, strike_, right_, k);
        }
        return FluentSubscription::per_contract_stock(symbol_, sec_type_, k);
    }

    std::string symbol_;
    std::string sec_type_;
    bool is_option_;
    std::string expiration_;
    std::string strike_;
    std::string right_;
};

/// Fluent security-type accessor for full-stream subscriptions.
class FluentSecType {
public:
    static FluentSecType stock()  { return FluentSecType{"STOCK"}; }
    static FluentSecType option() { return FluentSecType{"OPTION"}; }
    static FluentSecType index()  { return FluentSecType{"INDEX"}; }
    static FluentSecType rate()   { return FluentSecType{"RATE"}; }

    FluentSubscription full_trades() const {
        return FluentSubscription::full_stream(sec_type_,
            FluentSubscription::Kind::Trade);
    }
    FluentSubscription full_open_interest() const {
        return FluentSubscription::full_stream(sec_type_,
            FluentSubscription::Kind::OpenInterest);
    }

    const std::string& name() const noexcept { return sec_type_; }

private:
    explicit FluentSecType(std::string s) : sec_type_(std::move(s)) {}
    std::string sec_type_;
};

// ── String rendering for the fluent value types ─────────────────────────
//
// Python renders every fluent type via `__repr__` / `__str__` and
// TypeScript gains `toString()`; these give C++ the same so a value is
// not opaque when streamed or logged. `operator<<` is the idiomatic C++
// hook (`std::cout << contract`); `str()` returns the same text for
// callers that need a `std::string` (logging frameworks, test asserts).
// The rendered shapes match the Python surface: a contract prints as
// `"<symbol> <SEC_TYPE> <expiration> <right> <strike>"` for options and
// `"<symbol> <SEC_TYPE>"` otherwise.

inline std::ostream& operator<<(std::ostream& os, const FluentSecType& sec_type) {
    return os << sec_type.name();
}

inline std::string str(const FluentSecType& sec_type) { return sec_type.name(); }

inline std::ostream& operator<<(std::ostream& os, const FluentContract& contract) {
    os << contract.symbol();
    if (contract.is_option()) {
        os << " OPTION " << contract.expiration() << ' ' << contract.right() << ' '
           << contract.strike();
    } else {
        os << ' ' << contract.sec_type();
    }
    return os;
}

inline std::string str(const FluentContract& contract) {
    std::ostringstream os;
    os << contract;
    return os.str();
}

inline std::ostream& operator<<(std::ostream& os, const FluentSubscription& sub) {
    const char* kind_name = "Quote";
    switch (sub.kind()) {
        case FluentSubscription::Kind::Quote:        kind_name = "Quote"; break;
        case FluentSubscription::Kind::Trade:        kind_name = "Trade"; break;
        case FluentSubscription::Kind::OpenInterest: kind_name = "OpenInterest"; break;
        case FluentSubscription::Kind::MarketValue:  kind_name = "MarketValue"; break;
    }
    if (sub.scope() == FluentSubscription::Scope::Full) {
        return os << "Subscription(full " << kind_name << ", " << sub.sec_type() << ')';
    }
    os << "Subscription(" << kind_name << ", " << sub.symbol();
    if (sub.is_option()) {
        os << " OPTION " << sub.expiration() << ' ' << sub.right() << ' ' << sub.strike();
    } else {
        os << ' ' << sub.sec_type();
    }
    return os << ')';
}

inline std::string str(const FluentSubscription& sub) {
    std::ostringstream os;
    os << sub;
    return os.str();
}

// User-facing aliases — the documented surface (`Contract`, `SecType`).
// The class names above are prefixed `Fluent*` to keep them out of the
// namespace search path of any user code that might also
// `using namespace thetadatadx;` together with a free-standing
// `Contract` from another library.
using Contract = FluentContract;
// `Subscription` is already taken in this namespace by the active-
// subscription descriptor — alias the fluent value type with a
// distinct name. Users still write `client.subscribe(c.quote())`
// because `subscribe(...)` accepts the `FluentSubscription` value
// directly; the alias is only needed when the type name needs to
// appear at a call site.
using SubscriptionRef = FluentSubscription;
using SecType = FluentSecType;

// ── Client::subscribe(...) inline definitions ───────────────────
//
// Implemented out-of-class so the class body can reference the fluent
// types by forward declaration, then dispatch at the call site through
// the polymorphic C ABI (`thetadatadx_client_subscribe` /
// `thetadatadx_client_unsubscribe`), the same subscribe surface every binding
// exposes.

namespace detail {

inline ThetaDataDxSubscriptionRequest build_subscription_request(const FluentSubscription& sub) {
    ThetaDataDxSubscriptionRequest req{};
    req.symbol = nullptr;
    req.expiration = nullptr;
    req.strike = nullptr;
    req.right = nullptr;
    req.sec_type = nullptr;
    using K = FluentSubscription::Kind;
    switch (sub.kind()) {
        case K::Quote:        req.kind = THETADATADX_SUB_KIND_QUOTE;         break;
        case K::Trade:        req.kind = THETADATADX_SUB_KIND_TRADE;         break;
        case K::OpenInterest: req.kind = THETADATADX_SUB_KIND_OPEN_INTEREST; break;
        case K::MarketValue:  req.kind = THETADATADX_SUB_KIND_MARKET_VALUE;  break;
    }
    if (sub.scope() == FluentSubscription::Scope::Full) {
        req.scope = THETADATADX_SUB_SCOPE_FULL;
        req.sec_type = sub.sec_type().c_str();
    } else {
        req.scope = THETADATADX_SUB_SCOPE_CONTRACT;
        req.symbol = sub.symbol().c_str();
        if (sub.is_option()) {
            req.expiration = sub.expiration().c_str();
            req.strike = sub.strike().c_str();
            req.right = sub.right().c_str();
        }
    }
    return req;
}

} // namespace detail

inline void Stream::subscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (thetadatadx_client_subscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void Stream::subscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) subscribe(s);
}

inline void Stream::unsubscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (thetadatadx_client_unsubscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void Stream::unsubscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) unsubscribe(s);
}

// ── StreamingClient::subscribe(...) inline definitions ─────────────────────

inline void StreamingClient::subscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (thetadatadx_streaming_subscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void StreamingClient::subscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) subscribe(s);
}

inline void StreamingClient::unsubscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (thetadatadx_streaming_unsubscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void StreamingClient::unsubscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) unsubscribe(s);
}

// Generated Arrow-IPC terminals on the history result vectors
// (`thetadatadx::eod_ticks_to_arrow_ipc(...)`, ...). Placed after the `detail`
// namespace and the tick-type aliases so the wrappers can throw through
// `detail::throw_last_ffi_error` on a serialisation failure. Mirrors
// `FlatFileRowList::to_arrow_ipc()` and the Python columnar exit.
#include "tick_arrow_ipc.hpp.inc"

// ── Cross-language utility helpers ──────────────────────────────────────
//
// Thin std::string wrappers over the process-lifetime C-string accessors
// in `thetadatadx.h`. Each call copies the table entry once into a
// std::string so consumers don't need to think about C-string
// lifetimes. The underlying C functions are zero-cost lookups
// (no heap allocation, table-bounded).

namespace util {

/// Trade condition human-readable name. Returns "UNKNOWN" for unknown codes.
inline std::string condition_name(int32_t code) {
    return std::string(thetadatadx_condition_name(code));
}

/// Trade condition description. Returns "" for unknown codes.
inline std::string condition_description(int32_t code) {
    return std::string(thetadatadx_condition_description(code));
}

/// True if the trade condition code represents a cancellation.
inline bool is_cancel(int32_t code) {
    return thetadatadx_condition_is_cancel(code);
}

/// True if the trade condition code updates the volume bar.
inline bool updates_volume(int32_t code) {
    return thetadatadx_condition_updates_volume(code);
}

/// Quote condition human-readable name. Returns "UNKNOWN" for unknown codes.
inline std::string quote_condition_name(int32_t code) {
    return std::string(thetadatadx_quote_condition_name(code));
}

/// Quote condition description. Returns "" for unknown codes.
inline std::string quote_condition_description(int32_t code) {
    return std::string(thetadatadx_quote_condition_description(code));
}

/// True if the quote condition is firm (binding).
inline bool is_firm(int32_t code) {
    return thetadatadx_quote_condition_is_firm(code);
}

/// True if the quote condition indicates a trading halt.
inline bool is_halted(int32_t code) {
    return thetadatadx_quote_condition_is_halted(code);
}

/// Exchange human-readable name (e.g. 3 -> "NewYorkStockExchange").
/// Returns "UNKNOWN" for unknown codes.
inline std::string exchange_name(int32_t code) {
    return std::string(thetadatadx_exchange_name(code));
}

/// Exchange MIC-like symbol (e.g. 3 -> "NYSE"). Returns "UNKNOWN" for unknown codes.
inline std::string exchange_symbol(int32_t code) {
    return std::string(thetadatadx_exchange_symbol(code));
}

/// Convert a signed wire-encoded trade-sequence value to its unsigned
/// monotonic form. `signed_value` must lie in the 32-bit signed wire range
/// (`-2'147'483'648 ..= 2'147'483'647`); a value outside that domain
/// throws @c thetadatadx::InvalidParameterError rather than being silently
/// reinterpreted, matching the Python `ValueError` / TypeScript
/// `InvalidParameterError` for the same input.
inline uint64_t sequence_signed_to_unsigned(int64_t signed_value) {
    uint64_t out = 0;
    if (thetadatadx_sequence_signed_to_unsigned(signed_value, &out) != 0) {
        detail::throw_last_ffi_error();
    }
    return out;
}

/// Convert an unsigned monotonic trade-sequence value back to its
/// signed wire encoding. `unsigned_value` must lie in the unsigned wire range
/// (`0 ..= 2^32 - 1`); a value above that domain throws
/// @c thetadatadx::InvalidParameterError rather than being silently
/// reinterpreted.
inline int64_t sequence_unsigned_to_signed(uint64_t unsigned_value) {
    int64_t out = 0;
    if (thetadatadx_sequence_unsigned_to_signed(unsigned_value, &out) != 0) {
        detail::throw_last_ffi_error();
    }
    return out;
}

} // namespace util

// ── Fluent accessors over the C-ABI event structs ────────────────────
//
// C++ users get the same fluent surface Python and TypeScript see —
// the strike in dollars, the option side as a `char`, `sec_type` as a
// symbolic uppercase name, and `reason_name` for disconnect-reason
// values. These inline helpers take the C struct by reference and
// return the same shape Python / TypeScript bindings expose as fields.

/// Strike price in dollars. Returns `std::nullopt` for non-option
/// contracts. `ThetaDataDxContract.strike` already carries dollars — this
/// helper only folds the `has_strike` presence flag into
/// `std::optional`, so user code reads the dollar notation it writes
/// when calling `thetadatadx::Contract::option(symbol, expiration, strike,
/// right)`.
inline std::optional<double> strike(const ThetaDataDxContract& c) noexcept {
    if (!c.has_strike) {
        return std::nullopt;
    }
    return c.strike;
}

/// Option side as a single-character ASCII byte (`'C'` / `'P'`).
/// Returns `std::nullopt` for non-option contracts. Mirrors the
/// Python / TypeScript `right` field surface.
inline std::optional<char> right(const ThetaDataDxContract& c) noexcept {
    if (!c.has_right) {
        return std::nullopt;
    }
    return c.right;
}

/// Vendor vocabulary text for a `ThetaDataDxCalendarDay.status` code
/// (`"open"` / `"early_close"` / `"full_close"` / `"weekend"`;
/// `"UNKNOWN"` otherwise). Process-lifetime string — never freed.
inline std::string_view calendar_status_name(int32_t status) noexcept {
    return thetadatadx_calendar_status_name(status);
}

/// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day
/// into Unix epoch milliseconds (UTC, DST-aware). Usable with any
/// `(date, *_ms_of_day)` pair on the tick structs. Returns
/// `std::nullopt` when `date` is absent (`0`) or either input is out
/// of domain — mirrors the Python / TypeScript `*_timestamp_ms()` accessors.
inline std::optional<int64_t> timestamp_ms(int32_t date, int32_t ms_of_day) noexcept {
    const int64_t epoch = thetadatadx_timestamp_ms(date, ms_of_day);
    if (epoch < 0) {
        return std::nullopt;
    }
    return epoch;
}

/// Security type as a symbolic uppercase name (`"STOCK"` /
/// `"OPTION"` / `"INDEX"` / `"RATE"` / `"UNKNOWN"`). Mirrors the
/// Python / TypeScript `sec_type` string surface. Returns `"UNKNOWN"`
/// for unrecognised discriminants so callers stay total.
inline std::string_view sec_type_name(int32_t sec_type) noexcept {
    switch (sec_type) {
        case 0:
            return "STOCK";
        case 1:
            return "OPTION";
        case 2:
            return "INDEX";
        case 3:
            return "RATE";
        default:
            return "UNKNOWN";
    }
}

/// Disconnect reason name (`"TooManyRequests"`, `"InvalidCredentials"`,
/// ...). Mirrors the Python / TypeScript `reason_name` field surface.
/// Returns `"Unspecified"` for unrecognised discriminants so callers
/// stay total.
inline std::string_view reason_name(int32_t reason) noexcept {
    switch (reason) {
        case 0:
            return "InvalidCredentials";
        case 1:
            return "InvalidLoginValues";
        case 2:
            return "InvalidLoginSize";
        case 3:
            return "GeneralValidationError";
        case 4:
            return "TimedOut";
        case 5:
            return "ClientForcedDisconnect";
        case 6:
            return "AccountAlreadyConnected";
        case 7:
            return "SessionTokenExpired";
        case 8:
            return "InvalidSessionToken";
        case 9:
            return "FreeAccount";
        case 12:
            return "TooManyRequests";
        case 13:
            return "NoStartDate";
        case 14:
            return "LoginTimedOut";
        case 15:
            return "ServerRestarting";
        case 16:
            return "SessionTokenNotFound";
        case 17:
            return "ServerUserDoesNotExist";
        case 18:
            return "InvalidCredentialsNullUser";
        default:
            return "Unspecified";
    }
}

} // namespace thetadatadx

#endif /* THETADATADX_HPP */
