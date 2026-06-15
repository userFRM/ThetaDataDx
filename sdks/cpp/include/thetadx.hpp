/**
 * thetadatadx C++ SDK.
 *
 * RAII wrappers around the C FFI layer. Provides idiomatic C++ access to
 * ThetaData market data with automatic resource management.
 *
 * Tick data is returned directly as fixed-layout structs — no JSON parsing.
 * The C++ tick types are layout-compatible with the C ABI structs.
 */

#ifndef THETADX_HPP
#define THETADX_HPP

#include "thetadx.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <functional>
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

// ── Tick types (re-exported from thetadx.h for C++ convenience) ──
// These are typedef aliases to the C types defined in thetadx.h.
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

// ── FPSS event struct layout guards ──
//
// Field-level offsetof guards. The `ThetaDataDxStreamEvent` data-variant field
// order is generated from `fpss_event_schema.toml` — the same schema
// every binding is emitted from — so the C++ consumer and the data
// producer agree on member order by construction rather than by
// hand-kept convention. These asserts catch any ABI-level drift
// (padding, alignment, scalar widths) the schema alone cannot express.

// Every data variant carries an embedded `ThetaDataDxContract contract` as
// the first member. On LP64 (x86_64 / aarch64 Linux, macOS),
// `ThetaDataDxContract` is 32 bytes {
//   const char *root         offset  0, size 8
//   int32_t sec_type         offset  8, size 4
//   bool has_exp_date        offset 12, size 1
//   int32_t exp_date         offset 16, size 4 (3 bytes pad after has_exp_date)
//   bool has_is_call         offset 20, size 1
//   bool is_call             offset 21, size 1
//   bool has_strike          offset 22, size 1
//   int32_t strike           offset 24, size 4 (1 byte tail pad)
// }
// Data variants carry no wire-internal `contract_id` field; identity
// rides on `contract.symbol` (and the option-only `expiration` /
// `strike` / `is_call` flags) instead. The numbers below are the exact
// struct layout under `-O2` with the generated `fpss_event_structs.h.inc`
// types on an LP64 host; CI re-validates the asserts on every build.

// Generated layout guards for the FPSS event C mirror structs.
#include "fpss_layout_asserts.hpp.inc"

// OptionContract uses std::string for symbol to avoid use-after-free.
// The C FFI ThetaDataDxOptionContract uses a raw char* that is freed with the array,
// so we deep-copy the string during conversion.
struct OptionContract {
    std::string symbol;
    int32_t expiration;
    double strike;
    char right;
};

/// Active FPSS subscription descriptor.
struct Subscription {
    std::string kind;
    std::string contract;
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
// The dispatcher [`detail::throw_for_grpc_kind`] reads
// `thetadatadx_last_error_code()` (typed discriminant set inside the FFI
// boundary) to pick the right leaf without parsing the formatted
// message. Throw sites that emit a plain
// `std::runtime_error("thetadatadx: ...")` remain compatible because
// every leaf derives from `ThetaDataError` (a `std::runtime_error`):
// a generic `catch (const std::runtime_error&)` observes both the
// typed leaves and any plain-`runtime_error` site unchanged.

/// gRPC canonical status kind. Enum values match the gRPC wire codes
/// one-for-one (RFC 5234) so pattern-matching is portable across bindings.
enum class GrpcStatusKind : uint32_t {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
};

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

/// FPSS streaming protocol / state-machine failure.
class StreamError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

/// Environmental configuration fault — a config-file read failure, a
/// TOML parse error, or an internal config invariant. Distinct from
/// `InvalidParameterError` (a rejected user-supplied argument): a
/// `ConfigError` is the environment, not the call site. Pinned to the
/// reserved `TDX_ERR_CONFIG` discriminant so a `catch (const
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

/// Throw the [`ThetaDataError`] leaf that matches the typed C ABI
/// discriminant `code` (one of the `TDX_ERR_*` constants in
/// `thetadx.h`). Used by every wrapper that already has the formatted
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
        case TDX_ERR_AUTHENTICATION:
            throw AuthenticationError("thetadatadx: " + message);
        case TDX_ERR_INVALID_CREDENTIALS:
            throw InvalidCredentialsError("thetadatadx: " + message);
        case TDX_ERR_SUBSCRIPTION:
            throw SubscriptionError("thetadatadx: " + message);
        case TDX_ERR_RATE_LIMIT:
            // Carry the server back-off hint (if any) so the caller can
            // read `RateLimitError::retry_after()` as a value.
            throw RateLimitError("thetadatadx: " + message, last_ffi_retry_after_seconds());
        case TDX_ERR_NOT_FOUND:
            throw NotFoundError("thetadatadx: " + message);
        case TDX_ERR_DEADLINE_EXCEEDED:
            throw DeadlineExceededError("thetadatadx: " + message);
        case TDX_ERR_UNAVAILABLE:
            throw UnavailableError("thetadatadx: " + message);
        case TDX_ERR_NETWORK:
            throw NetworkError("thetadatadx: " + message);
        case TDX_ERR_SCHEMA_MISMATCH:
            throw SchemaMismatchError("thetadatadx: " + message);
        case TDX_ERR_INVALID_PARAMETER:
            throw InvalidParameterError("thetadatadx: " + message);
        case TDX_ERR_STREAM:
            throw StreamError("thetadatadx: " + message);
        case TDX_ERR_CONFIG:
            throw ConfigError("thetadatadx: " + message);
        case TDX_ERR_OTHER:
        case TDX_ERR_NONE:
        default:
            throw ThetaDataError("thetadatadx: " + message);
    }
}

/// Dispatcher keyed on the canonical gRPC kind. Used in tests that
/// want to verify the routing without actually round-tripping through
/// the FFI; production wrappers go through [`throw_for_code`] which
/// reads `thetadatadx_last_error_code()` directly.
[[noreturn]] inline void throw_for_grpc_kind(GrpcStatusKind kind, const std::string& message) {
    switch (kind) {
        case GrpcStatusKind::Unauthenticated:
            throw AuthenticationError("thetadatadx: " + message);
        case GrpcStatusKind::PermissionDenied:
            throw SubscriptionError("thetadatadx: " + message);
        case GrpcStatusKind::ResourceExhausted:
            throw RateLimitError("thetadatadx: " + message);
        case GrpcStatusKind::NotFound:
            throw NotFoundError("thetadatadx: " + message);
        case GrpcStatusKind::DeadlineExceeded:
            throw DeadlineExceededError("thetadatadx: " + message);
        case GrpcStatusKind::Unavailable:
            throw UnavailableError("thetadatadx: " + message);
        default:
            throw ThetaDataError("thetadatadx: " + message);
    }
}

static std::string last_ffi_error() {
    const char* err = thetadatadx_last_error();
    return err ? std::string(err) : "unknown error";
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

// Raw variant: returns "" when the FFI error slot is empty. Used by
// post-call disambiguation in check_array helpers — distinguishes
// success-empty from failure-empty (e.g. timeout on a list endpoint
// returns the same `{nullptr, 0}` sentinel as a successful empty result).
// Generated `_with_options` callers MUST `thetadatadx_clear_error()` before
// invoking the FFI so a stale error from a prior call isn't picked up.
static std::string last_ffi_error_raw() {
    const char* err = thetadatadx_last_error();
    return err ? std::string(err) : std::string();
}

template<typename T>
std::vector<T> to_vector(const T* data, size_t len) {
    if (data == nullptr || len == 0) return {};
    return std::vector<T>(data, data + len);
}

inline std::vector<std::string> string_array_to_vector(ThetaDataDxStringArray arr) {
    std::vector<std::string> result;
    if (arr.data != nullptr && arr.len > 0) {
        result.reserve(arr.len);
        for (size_t i = 0; i < arr.len; ++i) {
            result.emplace_back(arr.data[i] ? arr.data[i] : "");
        }
    }
    thetadatadx_string_array_free(arr);
    return result;
}

// Convert a ThetaDataDxStringArray to vector<string>, throwing on FFI error.
//
// Empty array is ambiguous: success-with-zero-results AND failure (e.g.
// timeout on a list endpoint) both return `{nullptr, 0}`. Disambiguate by
// reading `thetadatadx_last_error()` after the call. Generated wrappers
// `thetadatadx_clear_error()` before the FFI call so a stale error from a prior
// call isn't misattributed.
inline std::vector<std::string> check_string_array(ThetaDataDxStringArray arr) {
    const std::string err = last_ffi_error_raw();
    if (!err.empty()) {
        const int32_t code = thetadatadx_last_error_code();
        thetadatadx_string_array_free(arr);
        throw_for_code(code, err);
    }
    return string_array_to_vector(arr);
}

// Convert a typed tick array to vector<T> by passing in the converter and
// the FFI-array free fn. Throws on FFI error so callers don't mistake a
// timed-out tick endpoint for "no rows". Same contract as
// check_string_array — `thetadatadx_clear_error()` MUST have been called before
// the FFI invocation.
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
/// `dev` / `stage`) and tuned through the reconnect, FPSS, retry, MDDS, and
/// metrics setters below.
class Config {
public:
    /** Build the production configuration (ThetaData NJ datacenter).
     *  @return An owning `Config` holder seeded with production defaults. */
    static Config production();

    /** Build the dev FPSS configuration (port 20200, infinite
     *  historical replay).
     *  @return An owning `Config` holder seeded with dev defaults. */
    static Config dev();

    /** Build the stage FPSS configuration (port 20100, testing,
     *  unstable).
     *  @return An owning `Config` holder seeded with stage defaults. */
    static Config stage();

    /** Set FPSS reconnect policy. 0=Auto (default), 1=Manual. Throws
     *  @c thetadatadx::InvalidParameterError when @p policy is outside the
     *  documented `{0, 1}` set, matching the Python `ValueError` /
     *  TypeScript `InvalidParameterError` rather than silently coercing
     *  an unknown value to Auto. */
    void set_reconnect_policy(int policy) {
        if (thetadatadx_config_set_reconnect_policy(handle_.get(), policy) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Set the per-class transient-failure attempt budget. Default 30. */
    void set_reconnect_max_attempts(uint32_t max_attempts) {
        thetadatadx_config_set_reconnect_max_attempts(handle_.get(), max_attempts);
    }

    /** Set the rate-limited (TooManyRequests) attempt budget. Default 100. */
    void set_reconnect_max_rate_limited_attempts(uint32_t max_rate_limited_attempts) {
        thetadatadx_config_set_reconnect_max_rate_limited_attempts(handle_.get(),
                                                            max_rate_limited_attempts);
    }

    /** Set the stable-window timer (seconds) after which the auto-reconnect
     *  attempt counters reset. Default 60. */
    void set_reconnect_stable_window_secs(uint64_t secs) {
        thetadatadx_config_set_reconnect_stable_window_secs(handle_.get(), secs);
    }

    /** Set the reconnect delay (ms) honoured for generic transient
     *  disconnects (TimedOut, ServerRestarting, Unspecified, ...).
     *  Default 250. */
    void set_reconnect_wait_ms(uint64_t ms) {
        thetadatadx_config_set_reconnect_wait_ms(handle_.get(), ms);
    }

    /** Current reconnect wait_ms (default 250). Returns the default on
     *  a null config handle. */
    uint64_t get_reconnect_wait_ms() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_wait_ms(handle_.get(), &out);
        return out;
    }

    /** Set the reconnect delay (ms) honoured for `TooManyRequests`
     *  rate-limited disconnects. Default 130_000. */
    void set_reconnect_wait_rate_limited_ms(uint64_t ms) {
        thetadatadx_config_set_reconnect_wait_rate_limited_ms(handle_.get(), ms);
    }

    /** Current reconnect wait_rate_limited_ms (default 130_000). */
    uint64_t get_reconnect_wait_rate_limited_ms() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_wait_rate_limited_ms(handle_.get(), &out);
        return out;
    }

    /** Current reconnect policy selector: 0=Auto, 1=Manual, 2=Custom. */
    int32_t get_reconnect_policy() const {
        int32_t out{};
        thetadatadx_config_get_reconnect_policy(handle_.get(), &out);
        return out;
    }

    /** Current generic-transient reconnect attempt budget (default 30). */
    uint32_t get_reconnect_max_attempts() const {
        uint32_t out{};
        thetadatadx_config_get_reconnect_max_attempts(handle_.get(), &out);
        return out;
    }

    /** Current rate-limited reconnect attempt budget (default 100). */
    uint32_t get_reconnect_max_rate_limited_attempts() const {
        uint32_t out{};
        thetadatadx_config_get_reconnect_max_rate_limited_attempts(handle_.get(), &out);
        return out;
    }

    /** Set the ServerRestarting reconnect attempt budget. Default 60. */
    void set_reconnect_max_server_restart_attempts(uint32_t n) {
        thetadatadx_config_set_reconnect_max_server_restart_attempts(handle_.get(), n);
    }

    /** Current ServerRestarting reconnect attempt budget (default 60). */
    uint32_t get_reconnect_max_server_restart_attempts() const {
        uint32_t out{};
        thetadatadx_config_get_reconnect_max_server_restart_attempts(handle_.get(), &out);
        return out;
    }

    /** Current stable-window reset interval in seconds (default 60). */
    uint64_t get_reconnect_stable_window_secs() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_stable_window_secs(handle_.get(), &out);
        return out;
    }

    /** Set the wall-clock reconnect envelope (seconds) for the
     *  generic-transient and server-restart classes. 0 disables the
     *  envelope (attempt budgets only). Default 300. */
    void set_reconnect_max_elapsed_secs(uint64_t secs) {
        thetadatadx_config_set_reconnect_max_elapsed_secs(handle_.get(), secs);
    }

    /** Current wall-clock reconnect envelope in seconds (default 300;
     *  0 = disabled). */
    uint64_t get_reconnect_max_elapsed_secs() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_max_elapsed_secs(handle_.get(), &out);
        return out;
    }

    /** Set the cap (ms) on the exponential generic-transient reconnect
     *  ladder. Default 30_000. */
    void set_reconnect_wait_max_ms(uint64_t ms) {
        thetadatadx_config_set_reconnect_wait_max_ms(handle_.get(), ms);
    }

    /** Current reconnect wait_max_ms (default 30_000). */
    uint64_t get_reconnect_wait_max_ms() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_wait_max_ms(handle_.get(), &out);
        return out;
    }

    /** Set the flat reconnect cadence (ms) for ServerRestarting
     *  disconnects. Default 5_000. */
    void set_reconnect_wait_server_restart_ms(uint64_t ms) {
        thetadatadx_config_set_reconnect_wait_server_restart_ms(handle_.get(), ms);
    }

    /** Current reconnect wait_server_restart_ms (default 5_000). */
    uint64_t get_reconnect_wait_server_restart_ms() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_wait_server_restart_ms(handle_.get(), &out);
        return out;
    }

    /** Set the reconnect jitter mode: 0=Full (default), 1=Equal,
     *  2=Decorrelated, 3=None. Throws @c thetadatadx::InvalidParameterError on
     *  an out-of-domain mode (and @c thetadatadx::ThetaDataError on a null
     *  handle), routing through the typed leaf the FFI error code
     *  selects. */
    void set_reconnect_jitter(int32_t mode) {
        if (thetadatadx_config_set_reconnect_jitter(handle_.get(), mode) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Current reconnect jitter mode (same encoding as the setter). */
    int32_t get_reconnect_jitter() const {
        int32_t out{};
        thetadatadx_config_get_reconnect_jitter(handle_.get(), &out);
        return out;
    }

    /** Set the subscription-replay burst size used after an
     *  auto-reconnect. Minimum 1 (validated at connect). Default 50. */
    void set_reconnect_replay_burst_size(uint32_t n) {
        thetadatadx_config_set_reconnect_replay_burst_size(handle_.get(), n);
    }

    /** Current replay_burst_size (default 50). */
    uint32_t get_reconnect_replay_burst_size() const {
        uint32_t out{};
        thetadatadx_config_get_reconnect_replay_burst_size(handle_.get(), &out);
        return out;
    }

    /** Set the pause (ms) between subscription-replay bursts. 0
     *  removes the pause. Default 5. */
    void set_reconnect_replay_pace_ms(uint64_t ms) {
        thetadatadx_config_set_reconnect_replay_pace_ms(handle_.get(), ms);
    }

    /** Current replay_pace_ms (default 5). */
    uint64_t get_reconnect_replay_pace_ms() const {
        uint64_t out{};
        thetadatadx_config_get_reconnect_replay_pace_ms(handle_.get(), &out);
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

    /** Set the FPSS read timeout (ms): the no-frames deadline after
     *  which the streaming I/O loop reconnects. Default 3_000;
     *  validated to [100, 60_000] at connect. */
    void set_fpss_timeout_ms(uint64_t ms) {
        thetadatadx_config_set_fpss_timeout_ms(handle_.get(), ms);
    }

    /** Current fpss timeout_ms (default 3_000). */
    uint64_t get_fpss_timeout_ms() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_timeout_ms(handle_.get(), &out);
        return out;
    }

    /** Set the per-server connect timeout (ms) for the streaming
     *  connection. Default 2_000; validated to [1_000, 60_000] at
     *  connect. */
    void set_fpss_connect_timeout_ms(uint64_t ms) {
        thetadatadx_config_set_fpss_connect_timeout_ms(handle_.get(), ms);
    }

    /** Current fpss connect_timeout_ms (default 2_000). */
    uint64_t get_fpss_connect_timeout_ms() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_connect_timeout_ms(handle_.get(), &out);
        return out;
    }

    /** Set the FPSS heartbeat ping interval (ms). Default 250;
     *  validated to [100, 300_000] at connect. */
    void set_fpss_ping_interval_ms(uint64_t ms) {
        thetadatadx_config_set_fpss_ping_interval_ms(handle_.get(), ms);
    }

    /** Current fpss ping_interval_ms (default 250). */
    uint64_t get_fpss_ping_interval_ms() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_ping_interval_ms(handle_.get(), &out);
        return out;
    }

    /** Set the per-iteration blocking-read slice (ms) for the
     *  streaming I/O loop. Default 25; validated to [10, 500]. */
    void set_fpss_io_read_slice_ms(uint64_t ms) {
        thetadatadx_config_set_fpss_io_read_slice_ms(handle_.get(), ms);
    }

    /** Current fpss io_read_slice_ms (default 25). */
    uint64_t get_fpss_io_read_slice_ms() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_io_read_slice_ms(handle_.get(), &out);
        return out;
    }

    /** Set the last-frame watchdog (ms); 0 disables. Default 30_000. */
    void set_fpss_data_watchdog_ms(uint64_t ms) {
        thetadatadx_config_set_fpss_data_watchdog_ms(handle_.get(), ms);
    }

    /** Current fpss data_watchdog_ms (default 30_000; 0 = disabled). */
    uint64_t get_fpss_data_watchdog_ms() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_data_watchdog_ms(handle_.get(), &out);
        return out;
    }

    /** Set the TCP keepalive idle time (seconds). Default 5; validated
     *  to [1, 7_200] at connect. */
    void set_fpss_keepalive_idle_secs(uint64_t secs) {
        thetadatadx_config_set_fpss_keepalive_idle_secs(handle_.get(), secs);
    }

    /** Current fpss keepalive_idle_secs (default 5). */
    uint64_t get_fpss_keepalive_idle_secs() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_keepalive_idle_secs(handle_.get(), &out);
        return out;
    }

    /** Set the TCP keepalive probe interval (seconds). Default 2;
     *  validated to [1, 75] at connect. */
    void set_fpss_keepalive_interval_secs(uint64_t secs) {
        thetadatadx_config_set_fpss_keepalive_interval_secs(handle_.get(), secs);
    }

    /** Current fpss keepalive_interval_secs (default 2). */
    uint64_t get_fpss_keepalive_interval_secs() const {
        uint64_t out{};
        thetadatadx_config_get_fpss_keepalive_interval_secs(handle_.get(), &out);
        return out;
    }

    /** Set the TCP keepalive probe count before the kernel declares
     *  the peer dead. Default 2; validated to [1, 10] at connect. */
    void set_fpss_keepalive_retries(uint32_t n) {
        thetadatadx_config_set_fpss_keepalive_retries(handle_.get(), n);
    }

    /** Current fpss keepalive_retries (default 2). */
    uint32_t get_fpss_keepalive_retries() const {
        uint32_t out{};
        thetadatadx_config_get_fpss_keepalive_retries(handle_.get(), &out);
        return out;
    }

    /** Set the FPSS event ring size (slots). Must be a power of two
     *  >= 64; invalid values are rejected (thetadatadx_last_error). Default
     *  131_072. */
    void set_fpss_ring_size(size_t n) {
        thetadatadx_config_set_fpss_ring_size(handle_.get(), n);
    }

    /** Current fpss ring_size (default 131_072). */
    size_t get_fpss_ring_size() const {
        size_t out{};
        thetadatadx_config_get_fpss_ring_size(handle_.get(), &out);
        return out;
    }

    /** Set the FPSS host-selection policy: 0=Shuffled (default),
     *  1=FixedOrder. Throws @c thetadatadx::InvalidParameterError on an
     *  out-of-domain policy (and @c thetadatadx::ThetaDataError on a null
     *  handle), routing through the typed leaf the FFI error code
     *  selects. */
    void set_fpss_host_selection(int32_t policy) {
        if (thetadatadx_config_set_fpss_host_selection(handle_.get(), policy) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Current FPSS host-selection policy (same encoding as the
     *  setter). */
    int32_t get_fpss_host_selection() const {
        int32_t out{};
        thetadatadx_config_get_fpss_host_selection(handle_.get(), &out);
        return out;
    }

    /** Set the FPSS host-shuffle seed using the (has_value, seed)
     *  shape. has_value=false derives a fresh per-client seed;
     *  has_value=true makes the shuffled order deterministic. */
    int32_t set_fpss_host_shuffle_seed(bool has_value, uint64_t seed) {
        return thetadatadx_config_set_fpss_host_shuffle_seed(handle_.get(), has_value, seed);
    }

    /** Read the FPSS host-shuffle seed back. Returns @c std::nullopt for
     *  the per-client-entropy sentinel (no pinned seed); returns the
     *  wrapped seed when the shuffled order is deterministic. */
    std::optional<uint64_t> get_fpss_host_shuffle_seed() const {
        bool has_value = false;
        uint64_t seed = 0;
        thetadatadx_config_get_fpss_host_shuffle_seed(handle_.get(), &has_value, &seed);
        return has_value ? std::optional<uint64_t>{seed} : std::nullopt;
    }

    /** Set the wall-clock envelope (seconds) for one
     *  historical-channel retry sequence. 0 disables. Default 300. */
    void set_retry_max_elapsed_secs(uint64_t secs) {
        thetadatadx_config_set_retry_max_elapsed_secs(handle_.get(), secs);
    }

    /** Current retry max_elapsed in seconds (default 300; 0 = disabled). */
    uint64_t get_retry_max_elapsed_secs() const {
        uint64_t out{};
        thetadatadx_config_get_retry_max_elapsed_secs(handle_.get(), &out);
        return out;
    }

    /** Toggle AWS-style full jitter on the flatfile retry ladder.
     *  Default true. */
    void set_flatfiles_jitter(bool jitter) {
        thetadatadx_config_set_flatfiles_jitter(handle_.get(), jitter);
    }

    /** Current flatfiles jitter setting (default true). */
    bool get_flatfiles_jitter() const {
        bool out{};
        thetadatadx_config_get_flatfiles_jitter(handle_.get(), &out);
        return out;
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

    // ── RetryPolicy field setters/getters ──

    /** Initial backoff delay (ms) for the MDDS retry policy. Default 250. */
    void set_retry_initial_delay_ms(uint64_t ms) {
        thetadatadx_config_set_retry_initial_delay_ms(handle_.get(), ms);
    }
    uint64_t get_retry_initial_delay_ms() const {
        uint64_t out{};
        thetadatadx_config_get_retry_initial_delay_ms(handle_.get(), &out);
        return out;
    }

    /** Upper-bound backoff delay (ms). Default 30_000 (30 s). */
    void set_retry_max_delay_ms(uint64_t ms) {
        thetadatadx_config_set_retry_max_delay_ms(handle_.get(), ms);
    }
    uint64_t get_retry_max_delay_ms() const {
        uint64_t out{};
        thetadatadx_config_get_retry_max_delay_ms(handle_.get(), &out);
        return out;
    }

    /** Total attempt budget. 1 disables retry. Default 20. */
    void set_retry_max_attempts(uint32_t n) {
        thetadatadx_config_set_retry_max_attempts(handle_.get(), n);
    }
    uint32_t get_retry_max_attempts() const {
        uint32_t out{};
        thetadatadx_config_get_retry_max_attempts(handle_.get(), &out);
        return out;
    }

    /** AWS-style full jitter toggle. Default true. */
    void set_retry_jitter(bool jitter) {
        thetadatadx_config_set_retry_jitter(handle_.get(), jitter);
    }
    bool get_retry_jitter() const {
        bool out{};
        thetadatadx_config_get_retry_jitter(handle_.get(), &out);
        return out;
    }

    // ── FlatFilesConfig field setters/getters ──

    /** Total attempt budget for the flatfile driver retry loop.
     *  1 disables retry. Default 10. Validated to [1, 100]. */
    void set_flatfiles_max_attempts(uint32_t n) {
        thetadatadx_config_set_flatfiles_max_attempts(handle_.get(), n);
    }
    uint32_t get_flatfiles_max_attempts() const {
        uint32_t out{};
        thetadatadx_config_get_flatfiles_max_attempts(handle_.get(), &out);
        return out;
    }

    /** Initial backoff delay (seconds). Doubles per attempt up to
     *  max_backoff_secs. Default 1. */
    void set_flatfiles_initial_backoff_secs(uint64_t secs) {
        thetadatadx_config_set_flatfiles_initial_backoff_secs(handle_.get(), secs);
    }
    uint64_t get_flatfiles_initial_backoff_secs() const {
        uint64_t out{};
        thetadatadx_config_get_flatfiles_initial_backoff_secs(handle_.get(), &out);
        return out;
    }

    /** Upper-bound backoff delay (seconds). Default 30. Must be >=
     *  initial_backoff_secs (rejected at connect-time validate). */
    void set_flatfiles_max_backoff_secs(uint64_t secs) {
        thetadatadx_config_set_flatfiles_max_backoff_secs(handle_.get(), secs);
    }
    uint64_t get_flatfiles_max_backoff_secs() const {
        uint64_t out{};
        thetadatadx_config_get_flatfiles_max_backoff_secs(handle_.get(), &out);
        return out;
    }

    // ── AuthConfig field setters/getters ──

    /**
     * Set the Nexus auth URL. Default is the upstream production
     * endpoint; redirect at a staging cluster for testing.
     *
     * Throws a @c thetadatadx::ThetaDataError leaf if the FFI rejects the value
     * (null handle or non-UTF-8 input), routing through the typed class
     * the FFI error code selects.
     */
    void set_nexus_url(const std::string& url) {
        if (thetadatadx_config_set_nexus_url(handle_.get(), url.c_str()) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Current @c auth.nexus_url. Returns an empty string if the FFI
     *  getter returns null (null handle or interior-NUL value). */
    std::string get_nexus_url() const {
        detail::FfiString s(thetadatadx_config_get_nexus_url(handle_.get()));
        return s.str();
    }

    /**
     * Set the QueryInfo.client_type identifier. Default is
     * @c "rust-thetadatadx"; override to identify a deployment fleet
     * in server-side dashboards.
     *
     * Throws a @c thetadatadx::ThetaDataError leaf if the FFI rejects the value
     * (null handle or non-UTF-8 input), routing through the typed class
     * the FFI error code selects.
     */
    void set_client_type(const std::string& client_type) {
        if (thetadatadx_config_set_client_type(handle_.get(), client_type.c_str()) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Current @c auth.client_type. Returns an empty string if the FFI
     *  getter returns null (null handle or interior-NUL value). */
    std::string get_client_type() const {
        detail::FfiString s(thetadatadx_config_get_client_type(handle_.get()));
        return s.str();
    }

    // ── MetricsConfig field setter/getter ──

    /**
     * Set the Prometheus exporter port. Pass @c std::nullopt to leave
     * the exporter disabled (the @c None default); pass an explicit
     * @c std::uint16_t to bind an HTTP listener on @c 0.0.0.0:<port>
     * when the @c metrics-prometheus feature is compiled in.
     *
     * Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,
     * routing through the typed class the FFI error code selects.
     */
    void set_metrics_port(std::optional<std::uint16_t> port) {
        const bool has_value = port.has_value();
        const std::uint16_t arg = port.value_or(0);
        if (thetadatadx_config_set_metrics_port(handle_.get(), has_value, arg) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /**
     * Read the current @c metrics.port setting. Returns
     * @c std::nullopt for the disabled sentinel; returns the wrapped
     * @c std::uint16_t when an explicit port is set.
     *
     * Throws a @c thetadatadx::ThetaDataError leaf on a null-handle FFI failure,
     * routing through the typed class the FFI error code selects.
     */
    std::optional<std::uint16_t> get_metrics_port() const {
        bool has_value = false;
        std::uint16_t port = 0;
        if (thetadatadx_config_get_metrics_port(handle_.get(), &has_value, &port) != 0) {
            detail::throw_last_ffi_error();
        }
        return has_value ? std::optional<std::uint16_t>{port} : std::nullopt;
    }

    /** Set FPSS flush mode. 0=Batched (default), 1=Immediate. Throws
     *  @c thetadatadx::InvalidParameterError when @p mode is outside the
     *  documented `{0, 1}` set (and @c thetadatadx::ThetaDataError on a null
     *  handle), routing through the typed leaf the FFI error code
     *  selects. */
    void set_flush_mode(int mode) {
        if (thetadatadx_config_set_flush_mode(handle_.get(), mode) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Read the current FPSS flush mode. Same encoding as
     *  @c set_flush_mode: `0` = Batched, `1` = Immediate. Returns `0`
     *  (Batched) on a null handle (matching the C ABI's `-1` failure
     *  mapping at the boundary). */
    int get_flush_mode() const {
        int32_t mode = 0;
        thetadatadx_config_get_flush_mode(handle_.get(), &mode);
        return mode;
    }

    /** Set whether to derive OHLCVC bars locally from trades. */
    void set_derive_ohlcvc(bool enabled) { thetadatadx_config_set_derive_ohlcvc(handle_.get(), enabled); }

    /** Read the current OHLCVC-derivation flag. Returns `false` on a
     *  null handle (matching the C ABI's `-1` failure mapping at the
     *  boundary). */
    bool get_derive_ohlcvc() const {
        bool enabled = false;
        thetadatadx_config_get_derive_ohlcvc(handle_.get(), &enabled);
        return enabled;
    }

    // ── MDDS endpoint ──

    /** Set the historical (MDDS) gRPC host. Defaults to the upstream
     *  production endpoint; redirect the historical channel at a known
     *  host for testing. Throws a @c thetadatadx::ThetaDataError leaf if the FFI
     *  rejects the value (null handle or non-UTF-8 input). */
    void set_mdds_host(const std::string& host) {
        if (thetadatadx_config_set_mdds_host(handle_.get(), host.c_str()) != 0) {
            detail::throw_last_ffi_error();
        }
    }

    /** Current historical (MDDS) gRPC host. Returns an empty string if
     *  the FFI getter returns null (null handle or interior-NUL value). */
    std::string get_mdds_host() const {
        detail::FfiString s(thetadatadx_config_get_mdds_host(handle_.get()));
        return s.str();
    }

    /** Set the historical (MDDS) gRPC port. Companion to
     *  @c set_mdds_host. */
    void set_mdds_port(std::uint16_t port) {
        thetadatadx_config_set_mdds_port(handle_.get(), port);
    }

    /** Current historical (MDDS) gRPC port. Returns 0 on a null handle. */
    std::uint16_t get_mdds_port() const {
        std::uint16_t port = 0;
        thetadatadx_config_get_mdds_port(handle_.get(), &port);
        return port;
    }

    // ── MDDS pool sizing ──

    /**
     * Set the number of concurrent in-flight gRPC requests.
     *
     * @p n = 0 (default) auto-detects from the Nexus subscription tier
     * (Free=1 / Value=2 / Standard=4 / Pro=8). Explicit values above
     * the tier cap are clamped at connect time with a warn.
     */
    void set_concurrent_requests(std::uint32_t n) {
        thetadatadx_config_set_concurrent_requests(handle_.get(), n);
    }

    /**
     * Read the current concurrent in-flight gRPC request count.
     *
     * Returns the configured value (`0` = auto-detect from the tier),
     * or `0` on a null handle (matching the C ABI's `-1` failure
     * mapping at the boundary).
     */
    std::uint32_t get_concurrent_requests() const {
        std::uint32_t n = 0;
        thetadatadx_config_get_concurrent_requests(handle_.get(), &n);
        return n;
    }

    /**
     * Set the warn_on_buffered_threshold_bytes ceiling.
     *
     * Buffered (non-streaming) endpoints log a warning when a response's
     * decoded total exceeds this threshold, guiding users to the streaming
     * variant. The payload is still delivered.
     *
     * @p n = 0 disables the warning entirely.
     * Default is `100 * 1024 * 1024` (100 MiB).
     */
    void set_warn_on_buffered_threshold_bytes(std::size_t n) {
        thetadatadx_config_set_warn_on_buffered_threshold_bytes(handle_.get(), n);
    }

    /**
     * Read the current `warn_on_buffered_threshold_bytes` setting.
     *
     * Returns the configured byte count, or `0` on a null handle
     * (matching the C ABI's `-1` failure mapping at the boundary).
     */
    std::size_t get_warn_on_buffered_threshold_bytes() const {
        std::size_t n = 0;
        thetadatadx_config_get_warn_on_buffered_threshold_bytes(handle_.get(), &n);
        return n;
    }

    /** Get the raw handle. */
    ThetaDataDxConfig* get() const { return handle_.get(); }

private:
    explicit Config(ThetaDataDxConfig* h) : handle_(h) {}
    std::unique_ptr<ThetaDataDxConfig, ConfigDeleter> handle_;
};

// ── HistoricalClient ──

/// RAII wrapper around a historical (MDDS) gRPC client handle
/// (`ThetaDataDxHistoricalClient*`), freed automatically on destruction. The recommended
/// entry point for pure-historical access; the generated historical query
/// methods are mixed in from `historical.hpp.inc`.
class HistoricalClient {
public:
    /** Connect a historical (MDDS) client to ThetaData servers.
     *  @param creds Authenticated credentials.
     *  @param config Client configuration.
     *  @return A connected, owning `HistoricalClient`.
     *  @throws thetadatadx::ThetaDataError (or a typed leaf) on connection or
     *          authentication failure. */
    static HistoricalClient connect(const Credentials& creds, const Config& config);

    /** Connect a historical (MDDS) client, loading credentials from a
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

// ── FPSS event types (re-exported from thetadx.h) ──
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
// and counted via `dropped_events()`.
//
// The StreamingClient owns the `std::function`. A free `extern "C"` shim retrieves
// the stored function from the registered `void* ctx` and invokes it with
// the event reference. The shim converts `const ThetaDataDxStreamEvent*` (the C ABI
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
     *  consumer would invoke through a dangling ctx. Instead we detach
     *  the callback storage onto a helper thread that holds it for an
     *  extra 30 s grace window before dropping it. The internal detach
     *  helper bounds the consumer's worst-case lifetime to its own
     *  ring drain, so 30 s is a generous upper bound and lets the move
     *  proceed without observable liveness loss to the caller. */
    StreamingClient& operator=(StreamingClient&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                thetadatadx_streaming_shutdown(handle_.get());
                // Block until the consumer thread quiesces. The 5 s
                // budget matches `thetadatadx_streaming_free`'s internal barrier.
                int drained = thetadatadx_streaming_await_drain(handle_.get(), 5000);
                if (drained == 0) {
                    // Drain barrier timed out: the consumer may still
                    // be firing through `callback_`'s storage. Detach
                    // storage to a helper thread for a 30 s grace
                    // window so destruction happens off the move
                    // path; the consumer is bounded by its own ring
                    // drain and will quiesce well within that window
                    // even on a heavily backlogged ring.
                    std::thread([cb = std::move(callback_)]() mutable {
                        std::this_thread::sleep_for(std::chrono::seconds(30));
                        // `cb` destructs here, off the move path.
                    }).detach();
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
     *  `dropped_events()`. Throws on registration failure.
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
     *  installed yet. Safe to call on a moved-from client. */
    uint64_t dropped_events() const {
        return handle_ ? thetadatadx_streaming_dropped_events(handle_.get()) : 0;
    }

    /** Point-in-time count of streaming events published into the
     *  event ring but not yet drained into the registered callback —
     *  the in-flight depth between the I/O thread and the dispatcher.
     *  Rising occupancy that approaches ring_capacity() predicts
     *  drops before dropped_events() moves; sampling never blocks the
     *  feed and is safe from any thread. Returns 0 when no session is
     *  live. Safe to call on a moved-from client. */
    uint64_t ring_occupancy() const {
        return handle_ ? thetadatadx_streaming_ring_occupancy(handle_.get()) : 0;
    }

    /** Configured capacity of the streaming event ring in slots (the
     *  fpss_ring_size setting, a power of two) — the fixed
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


private:
    // Free C-ABI shim that the dispatcher invokes. `ctx` is the
    // `std::function*` we registered alongside the callback. The event
    // pointer is non-null and valid only for the duration of this call.
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
// Thin RAII wrappers over the C ABI in `thetadx.h`. The dynamic schema
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
    /// "TRADE" / "TRADE_QUOTE"; `date` is "YYYYMMDD".
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

    FlatFileRowList option_quote(const std::string& date) const {
        return request("OPTION", "QUOTE", date);
    }
    FlatFileRowList option_trade(const std::string& date) const {
        return request("OPTION", "TRADE", date);
    }
    FlatFileRowList option_trade_quote(const std::string& date) const {
        return request("OPTION", "TRADE_QUOTE", date);
    }
    FlatFileRowList option_ohlc(const std::string& date) const {
        return request("OPTION", "OHLC", date);
    }
    FlatFileRowList option_open_interest(const std::string& date) const {
        return request("OPTION", "OPEN_INTEREST", date);
    }
    FlatFileRowList option_eod(const std::string& date) const {
        return request("OPTION", "EOD", date);
    }
    FlatFileRowList stock_quote(const std::string& date) const {
        return request("STOCK", "QUOTE", date);
    }
    FlatFileRowList stock_trade(const std::string& date) const {
        return request("STOCK", "TRADE", date);
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

/// Full-stream subscription descriptor returned by
/// `Stream::active_full_subscriptions`. `sec_type` carries the
/// security-type discriminant (`"Stock"` / `"Option"` / `"Index"`) the
/// full-stream subscription is bound to; `kind` is the snake_case
/// full-stream kind label (`"full_trades"` / `"full_open_interest"`),
/// matching the Python / TypeScript `Subscription.kind` accessor.
struct FullSubscription {
    std::string kind;
    std::string sec_type;
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

/// Real-time-streaming sub-namespace returned by `Client::stream()`.
///
/// Borrows the unified `ThetaDataDxClient*` and a pointer to the parent `Client`'s
/// callback storage slot, so `set_callback` / `stop_streaming` /
/// `reconnect` observe the same registration the unified client manages.
/// The view is non-owning and transient: the callback `std::function`
/// storage lives on the parent `Client` (whose destructor runs the C-ABI
/// drain barrier), never on the view, so a `Stream` value may be created
/// and discarded freely without disturbing the streaming session.
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
     *  The parent `Client` owns a `std::unique_ptr<std::function>` whose
     *  address is the `void* ctx` registered with the dispatcher. That
     *  address must outlive every consumer-thread invocation; destruction
     *  routes through `thetadatadx_client_free` on the parent, which performs the
     *  shutdown + drain barrier internally, and replacement here calls
     *  `thetadatadx_client_stop_streaming` followed by
     *  `thetadatadx_client_await_drain(5000)` before releasing the storage — so no
     *  thread can observe a dangling ctx.
     *
     *  ## Lifecycle contract (unified replace-allowed rule)
     *
     *  Unlike `StreamingClient::set_callback` (one-shot), the unified path
     *  permits stop+register as a normal user flow: after
     *  `stop_streaming()` another `set_callback` REPLACES the saved
     *  `(callback, ctx)`. `reconnect()` is built on top of this. Calling
     *  `set_callback` on a live (running) session also replaces — the
     *  previous (callback, ctx) is drained out before the new one is wired
     *  in, with the same `await_drain(5000)` budget. */
    void set_callback(std::function<void(const StreamEvent&)> fn) {
        // Drain the existing wiring first so the consumer thread stops
        // invoking through the old `*callback_` storage before we release
        // it. Matches the C ABI's replace-allowed contract: a successful
        // replacement registration leaves the old `ctx` observable only
        // inside the drain barrier window.
        if (*callback_) {
            thetadatadx_client_stop_streaming(handle_);
            int drained = thetadatadx_client_await_drain(handle_, 5000);
            if (drained == 0) {
                // Drain barrier timed out: detach old storage to a helper
                // thread for a 30 s grace window so destruction happens off
                // the registration path; the consumer is bounded by its own
                // ring drain and will quiesce well within that window.
                std::thread([cb = std::move(*callback_)]() mutable {
                    std::this_thread::sleep_for(std::chrono::seconds(30));
                }).detach();
            } else {
                callback_->reset();
            }
        }
        auto staged = std::make_unique<std::function<void(const StreamEvent&)>>(std::move(fn));
        int rc = thetadatadx_client_set_callback(handle_, &Stream::callback_shim, staged.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
        *callback_ = std::move(staged);
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
            thetadatadx_client_stop_streaming(handle_);
        }
    }

    /// Reconnect streaming and re-apply every previously active
    /// subscription. Throws on failure — the wrapped C ABI sets the
    /// last-error slot on `-1` return.
    void reconnect() {
        int rc = thetadatadx_client_reconnect(handle_);
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
        return thetadatadx_client_await_drain(handle_, ms) == 1;
    }

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the bounded ring because the consumer fell behind and
    /// the ring was full. Returns 0 when no callback has been installed
    /// yet.
    uint64_t dropped_event_count() const {
        return handle_ ? thetadatadx_client_dropped_events(handle_) : 0;
    }

    /// Point-in-time count of streaming events published into the event
    /// ring but not yet drained into the registered callback — the
    /// in-flight depth between the I/O thread and the dispatcher. Rising
    /// occupancy that approaches ring_capacity() predicts drops before
    /// dropped_event_count() moves; sampling never blocks the feed and is
    /// safe from any thread. Returns 0 when no callback has been installed
    /// yet.
    uint64_t ring_occupancy() const {
        return handle_ ? thetadatadx_client_ring_occupancy(handle_) : 0;
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// fpss_ring_size setting, a power of two) — the fixed denominator for
    /// ring_occupancy(). Returns 0 when no callback has been installed yet.
    uint64_t ring_capacity() const {
        return handle_ ? thetadatadx_client_ring_capacity(handle_) : 0;
    }

    /** Milliseconds since the most recent inbound streaming frame of any
     *  kind. Returns 0 on success with the value in *out_ms, 1 when
     *  streaming has not started or no frame has been received yet, -1 on a
     *  null handle. */
    int32_t millis_since_last_event(uint64_t* out_ms) const {
        return handle_ ? thetadatadx_client_millis_since_last_event(handle_, out_ms) : -1;
    }

    /** UNIX-nanosecond receive timestamp of the most recent inbound
     *  streaming frame. 0 when streaming has not started or no frame has
     *  arrived yet. */
    int64_t last_event_received_at_unix_nanos() const {
        return handle_ ? thetadatadx_client_last_event_received_at_unix_nanos(handle_) : 0;
    }

    /** Address (host:port) of the streaming server the current session is
     *  connected to, following the session across auto-reconnects. Empty
     *  when streaming has not started. */
    std::string last_connected_addr() const {
        if (!handle_) return {};
        char* raw = thetadatadx_client_last_connected_addr(handle_);
        if (!raw) return {};
        std::string out(raw);
        thetadatadx_string_free(raw);
        return out;
    }

    /// `true` iff the streaming session is currently live (set_callback ran
    /// and stop_streaming / terminal close has not).
    bool is_streaming() const {
        return handle_ && thetadatadx_client_is_streaming(handle_) == 1;
    }

    /// Snapshot the currently-active per-contract subscriptions. Throws on
    /// FFI error.
    std::vector<Subscription> active_subscriptions() const {
        ThetaDataDxSubscriptionArray* arr = thetadatadx_client_active_subscriptions(handle_);
        if (arr == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<Subscription> out;
        if (arr->data != nullptr && arr->len > 0) {
            out.reserve(arr->len);
            for (size_t i = 0; i < arr->len; ++i) {
                const ThetaDataDxSubscription& s = arr->data[i];
                out.push_back(Subscription{
                    s.kind ? std::string(s.kind) : std::string(),
                    s.contract ? std::string(s.contract) : std::string(),
                });
            }
        }
        thetadatadx_subscription_array_free(arr);
        return out;
    }

    /// Cumulative count of user-callback failures contained by the
    /// per-invocation isolation boundary since the current stream started.
    /// If the callback aborts on a given event, the failure is contained,
    /// recorded here, and does not stop event delivery — the next event
    /// continues normally. Returns 0 when no callback has been installed
    /// yet. Safe to call from any thread without blocking. Mirrors the
    /// Python / TypeScript `client.stream.panic_count` placement.
    uint64_t panic_count() const {
        return handle_ ? thetadatadx_client_panic_count(handle_) : 0;
    }

    /// Snapshot the currently-active full-stream subscriptions (the entire
    /// universe for a given sec_type + kind, not bound to a single
    /// contract). Throws on FFI error. Mirrors the Python / TypeScript
    /// `client.stream.active_full_subscriptions` placement.
    std::vector<FullSubscription> active_full_subscriptions() const {
        ThetaDataDxSubscriptionArray* arr = thetadatadx_client_active_full_subscriptions(handle_);
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
    friend class Client;
    Stream(const ThetaDataDxClient* h,
           std::unique_ptr<std::function<void(const StreamEvent&)>>* callback)
        : handle_(h), callback_(callback) {}

    // Free C-ABI shim that the dispatcher invokes. `ctx` is the
    // `std::function*` we registered alongside the callback. The event
    // pointer is non-null and valid only for the duration of this call.
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

    const ThetaDataDxClient* handle_;
    // Borrowed pointer to the parent `Client`'s callback storage slot. The
    // parent outlives every transient `Stream` view, so the pointer is
    // always valid for the view's lifetime.
    std::unique_ptr<std::function<void(const StreamEvent&)>>* callback_;
};

/// RAII wrapper around a unified client handle (`ThetaDataDxClient*`).
/// The unified handle owns both the historical (gRPC/MDDS) and
/// streaming (FPSS) sub-clients. Historical queries are reached through
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

    Client(const Client&) = delete;
    Client& operator=(const Client&) = delete;
    Client(Client&& other) noexcept
        // Initialiser order MUST follow declaration order; see the
        // ordering invariant above the member declarations below.
        : callback_(std::move(other.callback_)),
          handle_(std::move(other.handle_)) {}
    /** Move-assign. The receiver may already hold a live streaming
     *  session whose consumer thread is invoking through the
     *  `callback_` storage. Drain the consumer before releasing the
     *  storage — same discipline as `StreamingClient::operator=`. On drain
     *  timeout, detach the callback storage onto a helper thread for a
     *  30 s grace window so destruction happens off the move path. */
    Client& operator=(Client&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                thetadatadx_client_stop_streaming(handle_.get());
                int drained = thetadatadx_client_await_drain(handle_.get(), 5000);
                if (drained == 0) {
                    std::thread([cb = std::move(callback_)]() mutable {
                        std::this_thread::sleep_for(std::chrono::seconds(30));
                    }).detach();
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

    /// Namespace handle for the FLATFILES surface. Cheap — borrows the
    /// underlying C ABI handle, so the lifetime of the returned
    /// `FlatFiles` value is bounded by `*this`.
    FlatFiles flat_files() const { return FlatFiles(handle_.get()); }

    /// Historical-data sub-namespace: `client.historical().stock_history_eod(...)`.
    ///
    /// Returns a `Historical` view borrowing this client's handle. No auth
    /// round-trip, no second connection; the view's lifetime is bounded by
    /// `*this`.
    Historical historical() const { return Historical(handle_.get()); }

    /// Real-time-streaming sub-namespace: `client.stream().subscribe(...)`,
    /// `client.stream().set_callback(cb)`, …
    ///
    /// Returns a `Stream` view borrowing this client's handle and a pointer
    /// to this client's callback storage slot, so the streaming lifecycle
    /// observed through the view is the one this client owns. The callback
    /// `std::function` storage lives on `*this` (whose destructor runs the
    /// C-ABI drain barrier), never on the transient view.
    Stream stream() { return Stream(handle_.get(), &callback_); }

    /// Raw handle for advanced consumers that want to call the C ABI
    /// directly. Ownership remains with this object.
    const ThetaDataDxClient* get() const noexcept { return handle_.get(); }

private:
    explicit Client(ThetaDataDxClient* h) : handle_(h) {}

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
    // and its drain barrier returns → `callback_` storage is then
    // released. Reordering these two members reintroduces the
    // use-after-free.
    std::unique_ptr<std::function<void(const StreamEvent&)>> callback_;
    std::unique_ptr<ThetaDataDxClient, UnifiedDeleter> handle_;
};

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
            // the FPSS wire; Quote and MarketValue are per-contract only.
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
        case K::Quote:        req.kind = TDX_SUB_KIND_QUOTE;         break;
        case K::Trade:        req.kind = TDX_SUB_KIND_TRADE;         break;
        case K::OpenInterest: req.kind = TDX_SUB_KIND_OPEN_INTEREST; break;
        case K::MarketValue:  req.kind = TDX_SUB_KIND_MARKET_VALUE;  break;
    }
    if (sub.scope() == FluentSubscription::Scope::Full) {
        req.scope = TDX_SUB_SCOPE_FULL;
        req.sec_type = sub.sec_type().c_str();
    } else {
        req.scope = TDX_SUB_SCOPE_CONTRACT;
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
    if (thetadatadx_client_subscribe(handle_, &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void Stream::subscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) subscribe(s);
}

inline void Stream::unsubscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (thetadatadx_client_unsubscribe(handle_, &req) != 0) {
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
// in `thetadx.h`. Each call copies the table entry once into a
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

#endif /* THETADX_HPP */
