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
#include <memory>
#include <optional>
#include <string>
#include <thread>
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

// Every data variant carries an embedded `TdxContract contract` as
// the first member. On LP64 (x86_64 / aarch64 Linux, macOS),
// `TdxContract` is 32 bytes {
//   const char *root         offset  0, size 8
//   int32_t sec_type         offset  8, size 4
//   bool has_exp_date        offset 12, size 1
//   int32_t exp_date         offset 16, size 4 (3 bytes pad after has_exp_date)
//   bool has_is_call         offset 20, size 1
//   bool is_call             offset 21, size 1
//   bool has_strike          offset 22, size 1
//   int32_t strike           offset 24, size 4 (1 byte tail pad)
// }
// Removed the wire-internal `contract_id` field from every
// data variant; identity rides on `contract.symbol` (and the
// option-only `expiration` / `strike` / `is_call` flags) instead.
// Numbers below recomputed from the exact struct under `-O2` with the
// generated `fpss_event_structs.h.inc` types on an LP64 host; CI
// re-validates the asserts on every build.

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

// ══════════════════════════════════════════════════════════════════════════
// Typed exception hierarchy
// ══════════════════════════════════════════════════════════════════════════
//
// Every FFI failure surfaces as a leaf in this hierarchy, rooted at
// `ThetaDataError` (itself a `std::runtime_error`). Callers writing
// generic `catch (const std::runtime_error&)` continue to observe
// the failure unchanged; callers that want structured handling can
// `catch (const SubscriptionError&)` for a tier / permission error
// or `catch (const RateLimitError&)` for a 429-shaped response,
// matching the Python and TypeScript leaf sets one-for-one.
//
// The dispatcher [`detail::throw_for_grpc_kind`] reads
// `tdx_last_error_code()` (typed discriminant set inside the FFI
// boundary) to pick the right leaf without parsing the formatted
// message. Pre-B4 throw sites that still emit
// `std::runtime_error("thetadatadx: ...")` are backward-compatible:
// new typed-throw sites route through this hierarchy while legacy
// sites stay as plain `runtime_error` and will be migrated as the
// FFI surface expands.

/// gRPC canonical status kind. Mirror of [`Rust::GrpcStatusKind`].
/// Enum values match the gRPC wire codes one-for-one (RFC 5234) so
/// pattern-matching is portable across bindings.
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
class RateLimitError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
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

/// FPSS streaming protocol / state-machine failure.
class StreamError : public ThetaDataError {
public:
    using ThetaDataError::ThetaDataError;
};

// ── RAII typed array wrappers ──

namespace detail {

/// Throw the [`ThetaDataError`] leaf that matches the typed C ABI
/// discriminant `code` (one of the `TDX_ERR_*` constants in
/// `thetadx.h`). Used by every wrapper that already has the formatted
/// message in hand and wants the right leaf class without re-parsing.
[[noreturn]] inline void throw_for_code(int32_t code, const std::string& message) {
    switch (code) {
        case TDX_ERR_AUTHENTICATION:
            throw AuthenticationError("thetadatadx: " + message);
        case TDX_ERR_INVALID_CREDENTIALS:
            throw InvalidCredentialsError("thetadatadx: " + message);
        case TDX_ERR_SUBSCRIPTION:
            throw SubscriptionError("thetadatadx: " + message);
        case TDX_ERR_RATE_LIMIT:
            throw RateLimitError("thetadatadx: " + message);
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
        case TDX_ERR_STREAM:
            throw StreamError("thetadatadx: " + message);
        case TDX_ERR_OTHER:
        case TDX_ERR_CONFIG:
        case TDX_ERR_NONE:
        default:
            throw ThetaDataError("thetadatadx: " + message);
    }
}

/// Dispatcher keyed on the canonical gRPC kind. Used in tests that
/// want to verify the routing without actually round-tripping through
/// the FFI; production wrappers go through [`throw_for_code`] which
/// reads `tdx_last_error_code()` directly.
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
    const char* err = tdx_last_error();
    return err ? std::string(err) : "unknown error";
}

/// Combined read-and-throw helper: snapshot the thread-local error
/// string AND the typed code, then throw the matching leaf. Returns
/// only if no error is set — the caller is responsible for proving
/// it has an error to surface (typically a null return from an FFI
/// call). Pre-B4 sites that throw `std::runtime_error` directly
/// should migrate to this so the leaf set stays current.
[[noreturn]] inline void throw_last_ffi_error() {
    const std::string message = last_ffi_error();
    const int32_t code = tdx_last_error_code();
    throw_for_code(code, message);
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
        const int32_t code = tdx_last_error_code();
        tdx_string_array_free(arr);
        throw_for_code(code, err);
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
        const int32_t code = tdx_last_error_code();
        free_fn(arr);
        throw_for_code(code, err);
    }
    auto result = convert(arr);
    free_fn(arr);
    return result;
}

inline std::vector<Subscription> subscription_array_to_vector(TdxSubscriptionArray* arr) {
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

    /** Set the per-class transient-failure attempt budget. Default 3. */
    void set_reconnect_max_attempts(uint32_t max_attempts) {
        tdx_config_set_reconnect_max_attempts(handle_.get(), max_attempts);
    }

    /** Set the rate-limited (TooManyRequests) attempt budget. Default 100. */
    void set_reconnect_max_rate_limited_attempts(uint32_t max_rate_limited_attempts) {
        tdx_config_set_reconnect_max_rate_limited_attempts(handle_.get(),
                                                            max_rate_limited_attempts);
    }

    /** Set the stable-window timer (seconds) after which the auto-reconnect
     *  attempt counters reset. Default 60. */
    void set_reconnect_stable_window_secs(uint64_t secs) {
        tdx_config_set_reconnect_stable_window_secs(handle_.get(), secs);
    }

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
//
// Replaced the flat `TdxFpssControl { kind, id, detail }`
// envelope with one typed C struct per `FpssControl::*` Rust variant.
// Consumers dispatch via `event.kind` and read the matching
// `event.<variant>` payload (`event.login_success.permissions`,
// `event.disconnected.reason`, etc.). The aliases below mirror every
// generated type so C++ users can stay in the `tdx::` namespace.

using FpssEventKind = TdxFpssEventKind;
using FpssQuote = TdxFpssQuote;
using FpssTrade = TdxFpssTrade;
using FpssOpenInterest = TdxFpssOpenInterest;
using FpssOhlcvc = TdxFpssOhlcvc;
// Typed control variants — one alias per `FpssControl::*` Rust variant.
using FpssConnected = TdxFpssConnected;
using FpssContractAssigned = TdxFpssContractAssigned;
using FpssDisconnected = TdxFpssDisconnected;
using FpssError = TdxFpssError;
using FpssLoginSuccess = TdxFpssLoginSuccess;
using FpssMarketClose = TdxFpssMarketClose;
using FpssMarketOpen = TdxFpssMarketOpen;
using FpssPing = TdxFpssPing;
using FpssReconnected = TdxFpssReconnected;
using FpssReconnectedServer = TdxFpssReconnectedServer;
using FpssReconnecting = TdxFpssReconnecting;
using FpssReqResponse = TdxFpssReqResponse;
using FpssRestart = TdxFpssRestart;
using FpssServerError = TdxFpssServerError;
using FpssUnknownControl = TdxFpssUnknownControl;
using FpssUnknownFrame = TdxFpssUnknownFrame;
using FpssEvent = TdxFpssEvent;

// ── FPSS real-time streaming client ──
//
// Event delivery is callback-driven via `set_callback(fn)`. Events flow
// `FPSS reader -> LMAX Disruptor ring -> consumer thread ->
// catch_unwind(fn)`. The reader thread never blocks on user code; on
// ring overflow events are dropped and counted via `dropped_events()`.
//
// The Client owns the `std::function`. A free `extern "C"` shim retrieves
// the stored function from the registered `void* ctx` and invokes it with
// the event reference. The shim converts `const TdxFpssEvent*` (the C ABI
// payload type) to `const FpssEvent&` (the C++ alias) at the boundary.
// Callback storage outlives any FPSS reader / Disruptor consumer thread
// because the destruction path always routes through `tdx_fpss_free`,
// which performs an internal drain barrier (5 s timeout) so the
// consumer has stopped firing the callback before the storage is
// released.

class FpssClient {
public:
    #include "fpss.hpp.inc"

    /// Polymorphic subscribe — primary fluent entry point. Forward
    /// declared here; the inline implementation appears below the
    /// fluent type definitions.
    inline void subscribe(const class FluentSubscription& sub) const;
    inline void subscribe_many(std::initializer_list<class FluentSubscription> subs) const;
    inline void unsubscribe(const class FluentSubscription& sub) const;
    inline void unsubscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    ~FpssClient();

    FpssClient(const FpssClient&) = delete;
    FpssClient& operator=(const FpssClient&) = delete;
    FpssClient(FpssClient&& other) noexcept
        // Initialiser order MUST follow declaration order; see the
        // ordering invariant comment above the member declarations.
        : callback_(std::move(other.callback_)),
          handle_(std::move(other.handle_)) {}
    /** Move-assign. The receiver may already hold a live FPSS handle
     *  with a registered callback whose `ctx` points into our existing
     *  `callback_` storage. We must drain that wiring on the C ABI side
     *  BEFORE destroying the old `callback_`, otherwise the Rust
     *  Disruptor consumer could invoke through a dangling `void*` ctx.
     *  `tdx_fpss_shutdown` returns asynchronously, so we follow it with
     *  `tdx_fpss_await_drain` (5 s budget, matching the free contract)
     *  to confirm the consumer thread has stopped firing the callback
     *  before releasing the storage.
     *
     *  Drain timeout (rare, indicates a wedged user callback): we MUST
     *  NOT reset `callback_` synchronously because a still-firing
     *  consumer would invoke through a dangling ctx. Instead we detach
     *  the callback storage onto a helper thread that holds it for an
     *  extra 30 s grace window before dropping it. The Rust-side detach
     *  helper bounds the consumer's worst-case lifetime to its own ring
     *  drain, so 30 s is a generous upper bound and lets the move
     *  proceed without observable liveness loss to the caller. */
    FpssClient& operator=(FpssClient&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                tdx_fpss_shutdown(handle_.get());
                // Block until the consumer thread quiesces. The 5 s
                // budget matches `tdx_fpss_free`'s internal barrier.
                int drained = tdx_fpss_await_drain(handle_.get(), 5000);
                if (drained == 0) {
                    // Drain barrier timed out: the Disruptor consumer
                    // may still be firing through `callback_`'s
                    // storage. Detach storage to a helper thread for
                    // a 30 s grace window so destruction happens off
                    // the move path; the consumer is bounded by its
                    // own ring drain and will quiesce well within
                    // that window even on a heavily backlogged ring.
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

    /** Register an FPSS callback and open the FPSS connection.
     *  `fn` runs on the LMAX Disruptor consumer thread under
     *  `catch_unwind`, never on the FPSS reader. The reader thread
     *  cannot be blocked by user code: on ring overflow events are
     *  dropped and counted via `dropped_events()`. Throws on
     *  registration failure.
     *
     *  ## Callback storage + thread affinity
     *
     *  The wrapper owns a `std::unique_ptr<std::function>` whose
     *  address is what the Rust Disruptor consumer receives as `ctx`.
     *  That address must outlive every consumer-thread invocation;
     *  destruction routes through `tdx_fpss_free`, which performs the
     *  shutdown + drain barrier internally, and move-assign calls
     *  `tdx_fpss_shutdown` followed by `tdx_fpss_await_drain` (5 s
     *  budget) before releasing the storage — so no thread can
     *  observe a dangling ctx. The consumer invokes `fn` serially on
     *  a single thread, so no internal locks are needed for
     *  callback-private state.
     *
     *  ## Lifecycle contract (FPSS one-shot rule)
     *
     *  The C ABI permits exactly one successful callback registration
     *  per handle, and rejects every register / reconnect / shutdown
     *  call after `tdx_fpss_shutdown`. A second call on a still-live
     *  handle returns -1 and KEEPS the previously installed
     *  (callback, ctx) wired into the Rust dispatcher. We therefore
     *  stage the new `std::function` into a local `unique_ptr`,
     *  attempt the FFI registration with the staged address, and only
     *  adopt it into `callback_` after the FFI reports success. On
     *  failure the existing `callback_` is left untouched so the
     *  still-live Rust registration keeps pointing at valid storage. */
    void set_callback(std::function<void(const FpssEvent&)> fn) {
        auto staged = std::make_unique<std::function<void(const FpssEvent&)>>(std::move(fn));
        int rc = tdx_fpss_set_callback(handle_.get(), &FpssClient::callback_shim, staged.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
        callback_ = std::move(staged);
    }

    /** Cumulative count of FPSS events the TLS reader could not publish
     *  into the LMAX Disruptor ring because the consumer fell behind
     *  and the ring was full. Returns 0 when no callback has been
     *  installed yet. Safe to call on a moved-from client. */
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

    // ── Member ordering invariant (do not reorder) ──
    //
    // C++ destructs members in REVERSE declaration order. The C ABI
    // contract for `tdx_fpss_free` is "drain the user-callback path
    // before this call returns" — the FFI's deleter runs that drain
    // barrier internally (5 s budget). For the barrier to be safe the
    // `std::function` storage backing the registered `void* ctx` MUST
    // still be alive while `tdx_fpss_free` is polling the drain flag,
    // because the Disruptor consumer may still be invoking through it.
    //
    // We therefore declare `handle_` AFTER `callback_`: reverse-order
    // destruction destroys `handle_` first → `tdx_fpss_free` runs and
    // its drain barrier returns → `callback_` storage is then released.
    // Reordering these two members reintroduces the use-after-free.
    //
    // `callback_` is a `unique_ptr<std::function<...>>` so the address
    // handed to the C ABI as `ctx` is stable across moves of the owning
    // `FpssClient`.
    std::unique_ptr<std::function<void(const FpssEvent&)>> callback_;
    std::unique_ptr<TdxFpssHandle, FpssHandleDeleter> handle_;
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

struct FlatFileRowListDeleter {
    void operator()(TdxFlatFileRowList* p) const {
        if (p) tdx_flatfile_rowlist_free(p);
    }
};

/// RAII wrapper around an opaque `TdxFlatFileRowList*`. Move-only.
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
        return handle_ ? tdx_flatfile_rows_count(handle_.get()) : 0;
    }

    /// Serialise the rows as Arrow IPC stream bytes. Throws on
    /// schema-inference / serialisation failure. The returned vector
    /// owns its memory; the underlying FFI buffer is freed before
    /// return so the caller never has to invoke `tdx_flatfile_bytes_free`.
    std::vector<uint8_t> to_arrow_ipc() const {
        if (!handle_) {
            throw std::runtime_error("thetadatadx: FlatFileRowList moved-from");
        }
        TdxFlatFileBytes raw = tdx_flatfile_rows_to_arrow_ipc(handle_.get());
        if (raw.data == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<uint8_t> out(raw.data, raw.data + raw.len);
        tdx_flatfile_bytes_free(raw);
        return out;
    }

    /// Raw handle accessor for advanced consumers that want to call
    /// the C ABI directly (e.g. zero-copy bridges into custom Arrow
    /// converters). Ownership remains with this object.
    const TdxFlatFileRowList* get() const noexcept { return handle_.get(); }

private:
    friend class FlatFiles;
    explicit FlatFileRowList(TdxFlatFileRowList* h) : handle_(h) {}
    std::unique_ptr<TdxFlatFileRowList, FlatFileRowListDeleter> handle_;
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
        TdxFlatFileRowList* h = tdx_flatfile_request_decoded(
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
        int rc = tdx_flatfile_request_to_path(
            handle_, sec_type.c_str(), req_type.c_str(),
            date.c_str(), path.c_str(), format.c_str());
        if (rc != 0) {
            detail::throw_last_ffi_error();
        }
    }

private:
    friend class UnifiedClient;
    explicit FlatFiles(const TdxUnified* h) : handle_(h) {}
    const TdxUnified* handle_;
};

struct UnifiedDeleter {
    void operator()(TdxUnified* p) const {
        if (p) tdx_unified_free(p);
    }
};

/// Full-stream subscription descriptor returned by
/// `UnifiedClient::active_full_subscriptions`. `sec_type` carries the
/// security-type discriminant (`"Stock"` / `"Option"` / `"Index"`) the
/// full-stream subscription is bound to; `kind` is the subscription
/// kind (`"Trade"` / `"OpenInterest"` / `"Quote"`).
struct FullSubscription {
    std::string kind;
    std::string sec_type;
};

/// RAII wrapper around a unified client handle (`TdxUnified*`).
/// The unified handle owns both the historical (gRPC/MDDS) and
/// streaming (FPSS) sub-clients; the C++ wrapper exposes the
/// FLATFILES surface, the polymorphic `subscribe(spec)` /
/// `unsubscribe(spec)` API, the `set_callback`-driven push delivery
/// path, the `streaming_iter_session()` RAII helper around pull-iter
/// delivery, and the lifecycle methods (`stop_streaming`,
/// `reconnect`, `await_drain`, `dropped_event_count`, `is_streaming`,
/// `active_subscriptions`, `active_full_subscriptions`). For
/// pure-historical gRPC use, `Client` remains the recommended entry
/// point.
class UnifiedClient {
public:
    /// Connect a unified client. Throws on auth / handshake failure.
    static UnifiedClient connect(const Credentials& creds, const Config& config) {
        TdxUnified* h = tdx_unified_connect(creds.get(), config.get());
        if (h == nullptr) {
            detail::throw_last_ffi_error();
        }
        return UnifiedClient(h);
    }

    UnifiedClient(const UnifiedClient&) = delete;
    UnifiedClient& operator=(const UnifiedClient&) = delete;
    UnifiedClient(UnifiedClient&& other) noexcept
        // Initialiser order MUST follow declaration order; see the
        // ordering invariant above the member declarations below.
        : callback_(std::move(other.callback_)),
          handle_(std::move(other.handle_)) {}
    /** Move-assign. The receiver may already hold a live streaming
     *  session whose Disruptor consumer is invoking through the
     *  `callback_` storage. Drain the consumer before releasing the
     *  storage — same discipline as `FpssClient::operator=`. On drain
     *  timeout, detach the callback storage onto a helper thread for a
     *  30 s grace window so destruction happens off the move path. */
    UnifiedClient& operator=(UnifiedClient&& other) noexcept {
        if (this != &other) {
            if (handle_) {
                tdx_unified_stop_streaming(handle_.get());
                int drained = tdx_unified_await_drain(handle_.get(), 5000);
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

    /// Polymorphic subscribe — primary fluent entry point. Defined
    /// inline below the fluent class declarations.
    inline void subscribe(const class FluentSubscription& sub) const;

    /// Bulk-subscribe an initializer list of `Subscription` values.
    /// Stops at the first error and throws.
    inline void subscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    inline void unsubscribe(const class FluentSubscription& sub) const;

    /// Bulk-unsubscribe an initializer list of `Subscription` values.
    inline void unsubscribe_many(std::initializer_list<class FluentSubscription> subs) const;

    /// Raw handle for advanced consumers that want to call the C ABI
    /// directly. Ownership remains with this object.
    const TdxUnified* get() const noexcept { return handle_.get(); }

    /// Start FPSS streaming in pull-iter delivery mode. Returns a
    /// move-only [`EventIterator`] handle; iterate with
    /// `while (auto event = it.next(timeout)) { ... }` or use the
    /// STL-iterator adapters `it.begin()` / `it.end()` for a
    /// range-for loop.
    ///
    /// Mutually exclusive with `set_callback(...)` on the same handle;
    /// switch by stopping streaming and starting again. Throws
    /// `std::runtime_error` on connection / state failure.
    inline class EventIterator start_streaming_iter() const;

    /// Open a context-managed pull-iter streaming session. The
    /// returned [`UnifiedFpssIterSession`] holds the
    /// [`EventIterator`] and pairs its destructor with
    /// `close()` + `stop_streaming()` + `await_drain(5000)`, mirroring
    /// the Python `with tdx.streaming_iter() as it:` shape.
    ///
    /// Mutually exclusive with `set_callback(...)` on the same handle.
    /// Throws on connection / state failure.
    inline class UnifiedFpssIterSession streaming_iter_session() const;

    /** Register an FPSS push callback and open the streaming session.
     *  `fn` runs on the LMAX Disruptor consumer thread under
     *  `catch_unwind`, never on the FPSS reader. The reader thread
     *  cannot be blocked by user code: on ring overflow events are
     *  dropped and counted via `dropped_event_count()`. Throws on
     *  registration failure.
     *
     *  ## Callback storage + thread affinity
     *
     *  The wrapper owns a `std::unique_ptr<std::function>` whose
     *  address is the `void* ctx` registered with the Rust dispatcher.
     *  That address must outlive every consumer-thread invocation;
     *  destruction routes through `tdx_unified_free`, which performs
     *  the shutdown + drain barrier internally, and move-assign /
     *  replacement calls `tdx_unified_stop_streaming` followed by
     *  `tdx_unified_await_drain(5000)` before releasing the storage
     *  — so no thread can observe a dangling ctx.
     *
     *  ## Lifecycle contract (unified replace-allowed rule)
     *
     *  Unlike `FpssClient::set_callback` (one-shot), the unified path
     *  permits stop+register as a normal user flow: after
     *  `stop_streaming()` another `set_callback` REPLACES the saved
     *  `(callback, ctx)`. `reconnect()` is built on top of this.
     *  Calling `set_callback` on a live (running) session also
     *  replaces — the previous (callback, ctx) is drained out before
     *  the new one is wired in, with the same `await_drain(5000)`
     *  budget. */
    void set_callback(std::function<void(const FpssEvent&)> fn) {
        // Drain the existing wiring first so the Disruptor consumer
        // stops invoking through the old `callback_` storage before
        // we release it. Matches the C ABI's replace-allowed contract:
        // a successful replacement registration leaves the old `ctx`
        // observable only inside the drain barrier window.
        if (callback_) {
            tdx_unified_stop_streaming(handle_.get());
            int drained = tdx_unified_await_drain(handle_.get(), 5000);
            if (drained == 0) {
                // Drain barrier timed out: detach old storage to a
                // helper thread for a 30 s grace window so destruction
                // happens off the registration path; the consumer is
                // bounded by its own ring drain and will quiesce well
                // within that window.
                std::thread([cb = std::move(callback_)]() mutable {
                    std::this_thread::sleep_for(std::chrono::seconds(30));
                }).detach();
            } else {
                callback_.reset();
            }
        }
        auto staged = std::make_unique<std::function<void(const FpssEvent&)>>(std::move(fn));
        int rc = tdx_unified_set_callback(handle_.get(), &UnifiedClient::callback_shim, staged.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
        callback_ = std::move(staged);
    }

    /// Stop FPSS streaming. Historical access remains available. Pair
    /// with `await_drain()` if you need to confirm the consumer
    /// thread has finished firing the registered callback before
    /// dropping any captured state.
    void stop_streaming() {
        if (handle_) {
            tdx_unified_stop_streaming(handle_.get());
        }
    }

    /// Reconnect FPSS streaming and re-apply every previously active
    /// subscription. Returns true on full success. Throws on failure
    /// — the wrapped C ABI sets the last-error slot on `-1` return.
    void reconnect() {
        int rc = tdx_unified_reconnect(handle_.get());
        if (rc < 0) {
            detail::throw_last_ffi_error();
        }
    }

    /// Block until the previous Disruptor consumer thread has
    /// finished firing the registered callback. Returns true on
    /// drain, false on timeout. Pass the same 5 s budget the FFI free
    /// path uses unless you have a specific reason to deviate.
    bool await_drain(std::chrono::milliseconds timeout) {
        const uint64_t ms = timeout.count() < 0
                                ? 0
                                : static_cast<uint64_t>(timeout.count());
        return tdx_unified_await_drain(handle_.get(), ms) == 1;
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the LMAX Disruptor ring because the consumer fell
    /// behind and the ring was full. Returns 0 when no callback has
    /// been installed yet. Safe to call on a moved-from client.
    uint64_t dropped_event_count() const {
        return handle_ ? tdx_unified_dropped_events(handle_.get()) : 0;
    }

    /// `true` iff the FPSS streaming session is currently live (set_callback
    /// or start_streaming_iter has been invoked and stop_streaming /
    /// terminal close has not).
    bool is_streaming() const {
        return handle_ && tdx_unified_is_streaming(handle_.get()) == 1;
    }

    /// Snapshot the currently-active per-contract subscriptions.
    /// Throws on FFI error.
    std::vector<Subscription> active_subscriptions() const {
        TdxSubscriptionArray* arr = tdx_unified_active_subscriptions(handle_.get());
        if (arr == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<Subscription> out;
        if (arr->data != nullptr && arr->len > 0) {
            out.reserve(arr->len);
            for (size_t i = 0; i < arr->len; ++i) {
                const TdxSubscription& s = arr->data[i];
                out.push_back(Subscription{
                    s.kind ? std::string(s.kind) : std::string(),
                    s.contract ? std::string(s.contract) : std::string(),
                });
            }
        }
        tdx_subscription_array_free(arr);
        return out;
    }

    /// Snapshot the currently-active full-stream subscriptions
    /// (the entire universe for a given sec_type + kind, not bound
    /// to a single contract). Throws on FFI error.
    std::vector<FullSubscription> active_full_subscriptions() const {
        TdxSubscriptionArray* arr = tdx_unified_active_full_subscriptions(handle_.get());
        if (arr == nullptr) {
            detail::throw_last_ffi_error();
        }
        std::vector<FullSubscription> out;
        if (arr->data != nullptr && arr->len > 0) {
            out.reserve(arr->len);
            for (size_t i = 0; i < arr->len; ++i) {
                const TdxSubscription& s = arr->data[i];
                out.push_back(FullSubscription{
                    s.kind ? std::string(s.kind) : std::string(),
                    s.contract ? std::string(s.contract) : std::string(),
                });
            }
        }
        tdx_subscription_array_free(arr);
        return out;
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

    explicit UnifiedClient(TdxUnified* h) : handle_(h) {}

    // ── Member ordering invariant (do not reorder) ──
    //
    // C++ destructs members in REVERSE declaration order. The C ABI
    // contract for `tdx_unified_free` is "drain the user-callback path
    // before this call returns" — the FFI's deleter runs that drain
    // barrier internally (5 s budget). For the barrier to be safe the
    // `std::function` storage backing the registered `void* ctx` MUST
    // still be alive while `tdx_unified_free` is polling the drain
    // flag, because the Disruptor consumer may still be invoking
    // through it.
    //
    // We therefore declare `handle_` AFTER `callback_`: reverse-order
    // destruction destroys `handle_` first → `tdx_unified_free` runs
    // and its drain barrier returns → `callback_` storage is then
    // released. Reordering these two members reintroduces the
    // use-after-free.
    std::unique_ptr<std::function<void(const FpssEvent&)>> callback_;
    std::unique_ptr<TdxUnified, UnifiedDeleter> handle_;
};

// ══════════════════════════════════════════════════════════════════════════
// Pull-iter delivery — RAII wrapper around `TdxFpssEventIterator*`
// ══════════════════════════════════════════════════════════════════════════
//
// Sibling of the push-callback path on `UnifiedClient`. Drains the
// per-client bounded queue on the caller's own thread; each `next()`
// blocks up to a user-supplied timeout for the next typed
// `TdxFpssEvent`. The class is move-only — copying would silently
// fan out queue draining across multiple consumers, which is not the
// design.
//
// Two surfaces:
//   * Explicit polling: `auto event = it.next(std::chrono::seconds(1));`
//     — `std::optional<TdxFpssEvent>` so the caller can branch on
//     timeout vs. terminal end-of-stream.
//   * Range-for: `for (auto& event : it) { ... }` — uses the
//     STL-iterator adapters below; the implicit timeout is "block
//     indefinitely until terminal end-of-stream", which matches the
//     idiomatic Python `for event in iter:` shape.
//
// The borrowed pointer fields inside `TdxFpssEvent` (`Contract.symbol`,
// payload byte slices, etc.) reference heap memory owned by the
// iterator handle. They are valid until the next `next()` call OR
// until the iterator is destroyed. Copy any fields the consumer
// wants to outlive the next pop.

struct EventIteratorDeleter {
    void operator()(TdxFpssEventIterator* p) const {
        if (p) tdx_fpss_event_iter_free(p);
    }
};

class EventIterator {
public:
    EventIterator(EventIterator&&) noexcept = default;
    EventIterator& operator=(EventIterator&&) noexcept = default;
    EventIterator(const EventIterator&) = delete;
    EventIterator& operator=(const EventIterator&) = delete;

    /// Pop the next event with a deadline. Returns `std::nullopt` on
    /// timeout (non-terminal — the upstream is still live and the
    /// caller can re-poll) and on terminal end-of-stream (the
    /// streaming session has shut down and the queue is drained).
    /// Distinguish via [`Self::ended`] after the call: `ended()` flips
    /// to `true` ONLY on terminal close (C ABI rc `-1`), never on
    /// timeout (rc `1`). A loop that re-polls on timeout therefore
    /// will not falsely terminate on a quiet-but-live upstream.
    std::optional<TdxFpssEvent> next(std::chrono::milliseconds timeout) {
        TdxFpssEvent out{};
        const int32_t ms = timeout.count() < 0
                               ? 0
                               : static_cast<int32_t>(std::min<long long>(
                                     timeout.count(), static_cast<long long>(INT32_MAX)));
        const int rc = tdx_fpss_event_iter_next(handle_.get(), &out, ms);
        if (rc == 0) {
            return out;
        }
        // rc == 1 — timeout; rc == -1 — terminal end-of-stream.
        // Only the terminal case latches `ended_`; the timeout case
        // is a soft re-poll signal the caller can act on.
        if (rc == -1) {
            ended_ = true;
        }
        return std::nullopt;
    }

    /// Non-blocking pop. Returns `std::nullopt` immediately on either
    /// an empty-but-live queue (rc `1`, soft re-poll signal) or a
    /// terminal end-of-stream (rc `-1`, queue drained on a stopped
    /// session). Distinguish via [`Self::ended`] after the call:
    /// `ended()` flips to `true` ONLY on terminal close, never on the
    /// quiet-but-live empty path. A polling integration should
    /// therefore loop on `try_next()` returning `nullopt` while
    /// `!ended()` and exit cleanly when `ended()` flips. Earlier the
    /// underlying core's `try_next()` returned `Option<FpssEvent>` and
    /// overloaded `None` to mean both, so the C ABI mapped every empty
    /// poll to `Timeout` (rc `1`) and a C++ caller draining after
    /// `stop_streaming()` would never observe `ended() == true`.
    std::optional<TdxFpssEvent> try_next() {
        TdxFpssEvent out{};
        const int rc = tdx_fpss_event_iter_next(handle_.get(), &out, 0);
        if (rc == 0) {
            return out;
        }
        // rc == 1 — empty-but-live; rc == -1 — terminal end-of-stream.
        // Only the terminal case latches `ended_`; mirrors the
        // `next(timeout)` code path so the two entry points have a
        // consistent end-of-stream contract.
        if (rc == -1) {
            ended_ = true;
        }
        return std::nullopt;
    }

    /// Whether the iterator has observed terminal end-of-stream.
    /// Once `true`, subsequent `next()` calls always return
    /// `std::nullopt`.
    bool ended() const noexcept { return ended_; }

    /// Mark the iterator closed. Subsequent `next()` calls return
    /// `std::nullopt` once the residual queue drains, without
    /// shutting down the underlying streaming session.
    void close() {
        if (handle_) {
            tdx_fpss_event_iter_close(handle_.get());
        }
    }

    /// STL-compatible input-iterator adapter. Not bidirectional or
    /// random-access — single-pass over the streaming queue.
    class Sentinel {};
    class IterAdapter {
    public:
        IterAdapter(EventIterator* parent, std::chrono::milliseconds timeout)
            : parent_(parent), timeout_(timeout) {
            advance();
        }
        const TdxFpssEvent& operator*() const { return current_; }
        const TdxFpssEvent* operator->() const { return &current_; }
        IterAdapter& operator++() {
            advance();
            return *this;
        }
        bool operator!=(const Sentinel&) const { return !done_; }

    private:
        void advance() {
            // Re-poll on timeout — `next()` returns `std::nullopt`
            // for both timeout and terminal close, but only the
            // terminal case latches `parent_->ended()`. A `for (auto&
            // event : iter)` loop must keep advancing on timeout so
            // a quiet-but-live upstream doesn't falsely end the
            // iteration (earlier the C ABI conflated the two,
            // which is what made this distinction necessary).
            for (;;) {
                auto evt = parent_->next(timeout_);
                if (evt.has_value()) {
                    current_ = *evt;
                    done_ = false;
                    return;
                }
                if (parent_->ended()) {
                    done_ = true;
                    return;
                }
                // Timeout — upstream still live. Continue waiting on
                // the next slice.
            }
        }
        EventIterator* parent_;
        std::chrono::milliseconds timeout_;
        TdxFpssEvent current_{};
        bool done_ = false;
    };

    /// `for (const auto& event : it)` adapter. Uses a 1-second
    /// per-pop timeout so a stalled upstream surfaces as a soft
    /// re-poll rather than blocking the iteration forever; callers
    /// who need a different cadence drive `next()` directly.
    IterAdapter begin() { return IterAdapter(this, std::chrono::milliseconds(1000)); }
    Sentinel end() { return Sentinel{}; }

    /// Raw handle for advanced consumers. Ownership stays with this
    /// object.
    TdxFpssEventIterator* get() const noexcept { return handle_.get(); }

private:
    friend class UnifiedClient;
    explicit EventIterator(TdxFpssEventIterator* h) : handle_(h) {}
    std::unique_ptr<TdxFpssEventIterator, EventIteratorDeleter> handle_;
    bool ended_ = false;
};

// Definition of `UnifiedClient::start_streaming_iter` deferred until
// after `EventIterator` is fully declared.
inline EventIterator UnifiedClient::start_streaming_iter() const {
    TdxFpssEventIterator* it = tdx_unified_start_streaming_iter(handle_.get());
    if (it == nullptr) {
        detail::throw_last_ffi_error();
    }
    return EventIterator(it);
}

// ══════════════════════════════════════════════════════════════════════════
// RAII pull-iter session
// ══════════════════════════════════════════════════════════════════════════
//
// Sibling of the Python `with tdx.streaming_iter() as it: ...` block.
// Construction opens the FPSS streaming session in pull-iter delivery
// mode; destruction pairs `close()` on the iterator with
// `stop_streaming()` + `await_drain(5000)` on the parent client so the
// consumer thread is guaranteed to have stopped pushing into the queue
// before any captured state goes out of scope.
//
// The session borrows the parent `UnifiedClient` by reference. Keep
// the parent alive for the whole session lifetime; a moved-from
// parent would dangle the borrow and the next FFI call would be
// undefined.

class UnifiedFpssIterSession {
public:
    UnifiedFpssIterSession(UnifiedFpssIterSession&&) noexcept = default;
    UnifiedFpssIterSession& operator=(UnifiedFpssIterSession&&) noexcept = default;
    UnifiedFpssIterSession(const UnifiedFpssIterSession&) = delete;
    UnifiedFpssIterSession& operator=(const UnifiedFpssIterSession&) = delete;

    ~UnifiedFpssIterSession() {
        if (iterator_.has_value()) {
            // Close the iterator first so any in-flight `next()` on a
            // helper thread bails out promptly; then stop streaming
            // and block on the drain barrier with the same 5 s budget
            // the Python / TS sessions use.
            iterator_->close();
            iterator_.reset();
            if (parent_ != nullptr) {
                tdx_unified_stop_streaming(parent_->get());
                int drained = tdx_unified_await_drain(parent_->get(), 5000);
                if (drained == 0) {
                    // The Disruptor consumer is still firing. The
                    // event-loop body has already exited (we are in
                    // destruction), so emit a diagnostic line and let
                    // the consumer drain in the background bounded by
                    // its own ring drain. Matches the warning the
                    // Python / TS RAII paths emit on drain timeout.
                    std::fprintf(stderr,
                                 "thetadatadx: UnifiedFpssIterSession drain timed out after 5000ms; "
                                 "the consumer thread may still be pushing events. "
                                 "The iterator is already closed and will stop yielding "
                                 "once the consumer exits.\n");
                }
            }
        }
    }

    /// Pop the next event with a deadline. Returns `std::nullopt` on
    /// timeout (non-terminal — the upstream is still live and the
    /// caller can re-poll) or on terminal end-of-stream. Distinguish
    /// the two via [`ended()`] after the call.
    std::optional<TdxFpssEvent> next(std::chrono::milliseconds timeout) {
        if (!iterator_.has_value()) {
            return std::nullopt;
        }
        return iterator_->next(timeout);
    }

    /// Non-blocking pop. Same semantics as
    /// [`EventIterator::try_next`].
    std::optional<TdxFpssEvent> try_next() {
        if (!iterator_.has_value()) {
            return std::nullopt;
        }
        return iterator_->try_next();
    }

    /// `true` once the underlying iterator has observed terminal
    /// end-of-stream. The session destructor itself does NOT mark
    /// the iterator ended — it shuts down the streaming session.
    bool ended() const noexcept {
        return iterator_.has_value() ? iterator_->ended() : true;
    }

    /// Mark the iterator closed without tearing down the streaming
    /// session. Subsequent `next()` calls drain residuals then return
    /// `std::nullopt`. The session destructor still runs `close()` +
    /// `stop_streaming()` + `await_drain()` — this method just lets
    /// the caller short-circuit the iterator early.
    void close() {
        if (iterator_.has_value()) {
            iterator_->close();
        }
    }

private:
    friend class UnifiedClient;
    UnifiedFpssIterSession(const UnifiedClient* parent, EventIterator iterator)
        : parent_(parent), iterator_(std::move(iterator)) {}

    const UnifiedClient* parent_;
    std::optional<EventIterator> iterator_;
};

inline UnifiedFpssIterSession UnifiedClient::streaming_iter_session() const {
    return UnifiedFpssIterSession(this, start_streaming_iter());
}

// ══════════════════════════════════════════════════════════════════════════
// Fluent contract-first API
// ══════════════════════════════════════════════════════════════════════════
//
// Mirrors the Rust target shape from
// `report/ThetaDataDxClient_API_DX_Review_Rust_Python.md`:
//
//     auto stock  = tdx::Contract::stock("AAPL");
//     auto option = tdx::Contract::option("SPY", "20260620", "550", "C");
//     client.subscribe(stock.quote());
//     client.subscribe(option.trade());
//     client.subscribe(tdx::SecType::option().full_trades());
//
// Pure-header layer over the existing C ABI subscribe entry points
// (`tdx_unified_subscribe` / `_unsubscribe`, polymorphic over
// `TdxSubscriptionRequest`). No
// new C ABI symbols — the value type just routes the existing call
// dispatch by stored kind + payload.

class FluentContract;
class FluentSubscription;
class FluentSecType;

/// Typed market-data subscription. Returned by `Contract::quote` /
/// `Contract::trade` / `Contract::open_interest` (per-contract) or by
/// `SecType::option().full_trades()` /
/// `.full_open_interest()` (full-stream). Pass into
/// `UnifiedClient::subscribe(sub)` or `subscribe_many(...)`.
class FluentSubscription {
public:
    enum class Scope { Contract, Full };
    enum class Kind { Quote, Trade, OpenInterest };

    Scope scope() const noexcept { return scope_; }
    Kind kind() const noexcept { return kind_; }
    const std::string& symbol() const noexcept { return symbol_; }
    const std::string& expiration() const noexcept { return expiration_; }
    const std::string& strike() const noexcept { return strike_; }
    const std::string& right() const noexcept { return right_; }
    const std::string& sec_type() const noexcept { return sec_type_; }
    bool is_option() const noexcept { return is_option_; }

private:
    friend class FluentContract;
    friend class FluentSecType;

    static FluentSubscription per_contract_stock(std::string symbol, Kind k) {
        FluentSubscription s;
        s.scope_ = Scope::Contract;
        s.kind_ = k;
        s.symbol_ = std::move(symbol);
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

/// Fluent contract identifier — stock or option.
class FluentContract {
public:
    /// Construct a stock contract.
    static FluentContract stock(std::string symbol) {
        return FluentContract{std::move(symbol), false, "", "", ""};
    }
    /// Construct an index contract. Routes through the stock-shape
    /// wire encoder; the C ABI layer treats them identically (no
    /// per-index subscribe call exists today).
    static FluentContract index(std::string symbol) {
        return FluentContract{std::move(symbol), false, "", "", ""};
    }
    /// Construct an option contract. `right` accepts `"C"` / `"CALL"`
    /// / `"P"` / `"PUT"` (case-insensitive).
    static FluentContract option(std::string symbol, std::string expiration,
                                  std::string strike, std::string right) {
        return FluentContract{std::move(symbol), true, std::move(expiration),
                              std::move(strike), std::move(right)};
    }

    FluentSubscription quote() const {
        if (is_option_) {
            return FluentSubscription::per_contract_option(
                symbol_, expiration_, strike_, right_,
                FluentSubscription::Kind::Quote);
        }
        return FluentSubscription::per_contract_stock(
            symbol_, FluentSubscription::Kind::Quote);
    }
    FluentSubscription trade() const {
        if (is_option_) {
            return FluentSubscription::per_contract_option(
                symbol_, expiration_, strike_, right_,
                FluentSubscription::Kind::Trade);
        }
        return FluentSubscription::per_contract_stock(
            symbol_, FluentSubscription::Kind::Trade);
    }
    FluentSubscription open_interest() const {
        if (is_option_) {
            return FluentSubscription::per_contract_option(
                symbol_, expiration_, strike_, right_,
                FluentSubscription::Kind::OpenInterest);
        }
        return FluentSubscription::per_contract_stock(
            symbol_, FluentSubscription::Kind::OpenInterest);
    }

    const std::string& symbol() const noexcept { return symbol_; }
    bool is_option() const noexcept { return is_option_; }
    const std::string& expiration() const noexcept { return expiration_; }
    const std::string& strike() const noexcept { return strike_; }
    const std::string& right() const noexcept { return right_; }

private:
    FluentContract(std::string symbol, bool is_option,
                   std::string expiration, std::string strike, std::string right)
        : symbol_(std::move(symbol)), is_option_(is_option),
          expiration_(std::move(expiration)), strike_(std::move(strike)),
          right_(std::move(right)) {}

    std::string symbol_;
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

// User-facing aliases — the documented surface (`Contract`, `SecType`)
// per `report/...md`. The class names above are prefixed `Fluent*` to
// keep them out of the namespace search path of any user code that
// might also `using namespace tdx;` together with a free-standing
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

// ── UnifiedClient::subscribe(...) inline definitions ───────────────────
//
// Implemented out-of-class so the class body can reference the fluent
// types by forward declaration, then dispatch at the call site through
// the polymorphic C ABI (`tdx_unified_subscribe` /
// `tdx_unified_unsubscribe`) which mirrors the Rust `subscribe`
// signature one-for-one.

namespace detail {

inline TdxSubscriptionRequest build_subscription_request(const FluentSubscription& sub) {
    TdxSubscriptionRequest req{};
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

inline void UnifiedClient::subscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (tdx_unified_subscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void UnifiedClient::subscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) subscribe(s);
}

inline void UnifiedClient::unsubscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (tdx_unified_unsubscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void UnifiedClient::unsubscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) unsubscribe(s);
}

// ── FpssClient::subscribe(...) inline definitions ─────────────────────

inline void FpssClient::subscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (tdx_fpss_subscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void FpssClient::subscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) subscribe(s);
}

inline void FpssClient::unsubscribe(const FluentSubscription& sub) const {
    auto req = detail::build_subscription_request(sub);
    if (tdx_fpss_unsubscribe(handle_.get(), &req) != 0) {
        detail::throw_last_ffi_error();
    }
}

inline void FpssClient::unsubscribe_many(
    std::initializer_list<FluentSubscription> subs) const {
    for (const auto& s : subs) unsubscribe(s);
}

// ── Cross-language utility helpers (issue #424) ─────────────────────────
//
// Thin std::string wrappers over the `'static` C-string accessors in
// `thetadx.h`. Each call copies the table entry once into a
// std::string so consumers don't need to think about C-string
// lifetimes. The underlying C functions are zero-cost lookups
// (no heap allocation, table-bounded).

namespace util {

/// Trade condition human-readable name. Returns "UNKNOWN" for unknown codes.
inline std::string condition_name(int32_t code) {
    return std::string(tdx_condition_name(code));
}

/// Trade condition description. Returns "" for unknown codes.
inline std::string condition_description(int32_t code) {
    return std::string(tdx_condition_description(code));
}

/// True if the trade condition code represents a cancellation.
inline bool is_cancel(int32_t code) {
    return tdx_condition_is_cancel(code);
}

/// True if the trade condition code updates the volume bar.
inline bool updates_volume(int32_t code) {
    return tdx_condition_updates_volume(code);
}

/// Quote condition human-readable name. Returns "UNKNOWN" for unknown codes.
inline std::string quote_condition_name(int32_t code) {
    return std::string(tdx_quote_condition_name(code));
}

/// Quote condition description. Returns "" for unknown codes.
inline std::string quote_condition_description(int32_t code) {
    return std::string(tdx_quote_condition_description(code));
}

/// True if the quote condition is firm (binding).
inline bool is_firm(int32_t code) {
    return tdx_quote_condition_is_firm(code);
}

/// True if the quote condition indicates a trading halt.
inline bool is_halted(int32_t code) {
    return tdx_quote_condition_is_halted(code);
}

/// Exchange human-readable name (e.g. 3 -> "NewYorkStockExchange").
/// Returns "UNKNOWN" for unknown codes.
inline std::string exchange_name(int32_t code) {
    return std::string(tdx_exchange_name(code));
}

/// Exchange MIC-like symbol (e.g. 3 -> "NYSE"). Returns "UNKNOWN" for unknown codes.
inline std::string exchange_symbol(int32_t code) {
    return std::string(tdx_exchange_symbol(code));
}

/// Convert a signed wire-encoded trade-sequence value to its unsigned
/// monotonic form. Mirrors `tdbe::sequences::signed_to_unsigned`.
inline uint64_t sequence_signed_to_unsigned(int64_t signed_value) {
    return tdx_sequence_signed_to_unsigned(signed_value);
}

/// Convert an unsigned monotonic trade-sequence value back to its
/// signed wire encoding. Mirrors `tdbe::sequences::unsigned_to_signed`.
inline int64_t sequence_unsigned_to_signed(uint64_t unsigned_value) {
    return tdx_sequence_unsigned_to_signed(unsigned_value);
}

} // namespace util

// ── Fluent accessors over the C-ABI event structs ────────────────────
//
// C++ users get the same fluent surface Python and TypeScript see —
// `strike_dollars`, the option side as a `char`, `sec_type` as a
// symbolic uppercase name, and `reason_name` for `RemoveReason`
// values — without us widening the wire `#[repr(C)]` structs (which
// would break ABI parity with the Rust mirror). These inline helpers
// take the C struct by reference and return the same shape Python /
// TypeScript bindings expose as fields.

/// Strike price in dollars. Returns `std::nullopt` for non-option
/// contracts. The wire field is an `int32_t` in thousandths of a
/// dollar (`5_400_000` for a `$5,400.00` strike); this divides by
/// `1000.0` so user code reads the dollar notation it writes when
/// calling `tdx::Contract::option(symbol, expiration, strike, right)`.
inline std::optional<double> strike_dollars(const TdxContract& c) noexcept {
    if (!c.has_strike) {
        return std::nullopt;
    }
    return static_cast<double>(c.strike) / 1000.0;
}

/// Option side as a single-character ASCII byte (`'C'` / `'P'`).
/// Returns `std::nullopt` for non-option contracts. Mirrors the
/// Python / TypeScript `right` field surface.
inline std::optional<char> right(const TdxContract& c) noexcept {
    if (!c.has_right) {
        return std::nullopt;
    }
    return c.right;
}

/// Security type as a symbolic uppercase name (`"STOCK"` /
/// `"OPTION"` / `"INDEX"` / `"RATE"` / `"UNKNOWN"`). Mirrors
/// `SecType::as_str()` on the Rust core and the Python / TypeScript
/// `sec_type` string surface. Returns `"UNKNOWN"` for unrecognised
/// discriminants so callers stay total.
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
/// ...). Mirrors `tdbe::types::enums::RemoveReason::as_str()` on the
/// Rust core and the Python / TypeScript `reason_name` field surface.
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

} // namespace tdx

#endif /* THETADX_HPP */
