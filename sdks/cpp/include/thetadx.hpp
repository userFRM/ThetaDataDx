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
using GreeksTick = TdxGreeksTick;
using IvTick = TdxIvTick;
using PriceTick = TdxPriceTick;
using OpenInterestTick = TdxOpenInterestTick;
using MarketValueTick = TdxMarketValueTick;
using CalendarDay = TdxCalendarDay;
using InterestRateTick = TdxInterestRateTick;
using TradeQuoteTick = TdxTradeQuoteTick;

// Catch C header / Rust layout drift at compile time before it can corrupt arrays at runtime.
static_assert(sizeof(CalendarDay) == 64 && alignof(CalendarDay) == 64,
              "TdxCalendarDay layout drifted from Rust");
static_assert(sizeof(EodTick) == 128 && alignof(EodTick) == 64,
              "TdxEodTick layout drifted from Rust");
static_assert(sizeof(GreeksTick) == 256 && alignof(GreeksTick) == 64,
              "TdxGreeksTick layout drifted from Rust");
static_assert(sizeof(InterestRateTick) == 64 && alignof(InterestRateTick) == 64,
              "TdxInterestRateTick layout drifted from Rust");
static_assert(sizeof(IvTick) == 64 && alignof(IvTick) == 64,
              "TdxIvTick layout drifted from Rust");
static_assert(sizeof(MarketValueTick) == 64 && alignof(MarketValueTick) == 64,
              "TdxMarketValueTick layout drifted from Rust");
static_assert(sizeof(OhlcTick) == 128 && alignof(OhlcTick) == 64,
              "TdxOhlcTick layout drifted from Rust");
static_assert(sizeof(OpenInterestTick) == 64 && alignof(OpenInterestTick) == 64,
              "TdxOpenInterestTick layout drifted from Rust");
static_assert(sizeof(PriceTick) == 64 && alignof(PriceTick) == 64,
              "TdxPriceTick layout drifted from Rust");
static_assert(sizeof(QuoteTick) == 128 && alignof(QuoteTick) == 64,
              "TdxQuoteTick layout drifted from Rust");
static_assert(sizeof(TradeQuoteTick) == 192 && alignof(TradeQuoteTick) == 64,
              "TdxTradeQuoteTick layout drifted from Rust");
static_assert(sizeof(TradeTick) == 128 && alignof(TradeTick) == 64,
              "TdxTradeTick layout drifted from Rust");

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

// TdxFpssOhlcvc — alphabetized first data variant (sorted_data_events).
static_assert(offsetof(TdxFpssOhlcvc, contract_id) == 0,
              "TdxFpssOhlcvc::contract_id offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, ms_of_day) == 4,
              "TdxFpssOhlcvc::ms_of_day offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, open) == 8,
              "TdxFpssOhlcvc::open offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, high) == 16,
              "TdxFpssOhlcvc::high offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, low) == 24,
              "TdxFpssOhlcvc::low offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, close) == 32,
              "TdxFpssOhlcvc::close offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, volume) == 40,
              "TdxFpssOhlcvc::volume offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, count) == 48,
              "TdxFpssOhlcvc::count offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, date) == 56,
              "TdxFpssOhlcvc::date offset drifted");
static_assert(offsetof(TdxFpssOhlcvc, received_at_ns) == 64,
              "TdxFpssOhlcvc::received_at_ns offset drifted");
static_assert(sizeof(TdxFpssOhlcvc) == 72,
              "TdxFpssOhlcvc total size drifted");

// TdxFpssOpenInterest
static_assert(offsetof(TdxFpssOpenInterest, contract_id) == 0,
              "TdxFpssOpenInterest::contract_id offset drifted");
static_assert(offsetof(TdxFpssOpenInterest, ms_of_day) == 4,
              "TdxFpssOpenInterest::ms_of_day offset drifted");
static_assert(offsetof(TdxFpssOpenInterest, open_interest) == 8,
              "TdxFpssOpenInterest::open_interest offset drifted");
static_assert(offsetof(TdxFpssOpenInterest, date) == 12,
              "TdxFpssOpenInterest::date offset drifted");
static_assert(offsetof(TdxFpssOpenInterest, received_at_ns) == 16,
              "TdxFpssOpenInterest::received_at_ns offset drifted");
static_assert(sizeof(TdxFpssOpenInterest) == 24,
              "TdxFpssOpenInterest total size drifted");

// TdxFpssQuote
static_assert(offsetof(TdxFpssQuote, contract_id) == 0,
              "TdxFpssQuote::contract_id offset drifted");
static_assert(offsetof(TdxFpssQuote, ms_of_day) == 4,
              "TdxFpssQuote::ms_of_day offset drifted");
static_assert(offsetof(TdxFpssQuote, bid_size) == 8,
              "TdxFpssQuote::bid_size offset drifted");
static_assert(offsetof(TdxFpssQuote, bid_exchange) == 12,
              "TdxFpssQuote::bid_exchange offset drifted");
static_assert(offsetof(TdxFpssQuote, bid) == 16,
              "TdxFpssQuote::bid offset drifted");
static_assert(offsetof(TdxFpssQuote, bid_condition) == 24,
              "TdxFpssQuote::bid_condition offset drifted");
static_assert(offsetof(TdxFpssQuote, ask_size) == 28,
              "TdxFpssQuote::ask_size offset drifted");
static_assert(offsetof(TdxFpssQuote, ask_exchange) == 32,
              "TdxFpssQuote::ask_exchange offset drifted");
static_assert(offsetof(TdxFpssQuote, ask) == 40,
              "TdxFpssQuote::ask offset drifted");
static_assert(offsetof(TdxFpssQuote, ask_condition) == 48,
              "TdxFpssQuote::ask_condition offset drifted");
static_assert(offsetof(TdxFpssQuote, date) == 52,
              "TdxFpssQuote::date offset drifted");
static_assert(offsetof(TdxFpssQuote, received_at_ns) == 56,
              "TdxFpssQuote::received_at_ns offset drifted");
static_assert(sizeof(TdxFpssQuote) == 64,
              "TdxFpssQuote total size drifted");

// TdxFpssTrade
static_assert(offsetof(TdxFpssTrade, contract_id) == 0,
              "TdxFpssTrade::contract_id offset drifted");
static_assert(offsetof(TdxFpssTrade, ms_of_day) == 4,
              "TdxFpssTrade::ms_of_day offset drifted");
static_assert(offsetof(TdxFpssTrade, sequence) == 8,
              "TdxFpssTrade::sequence offset drifted");
static_assert(offsetof(TdxFpssTrade, ext_condition1) == 12,
              "TdxFpssTrade::ext_condition1 offset drifted");
static_assert(offsetof(TdxFpssTrade, ext_condition2) == 16,
              "TdxFpssTrade::ext_condition2 offset drifted");
static_assert(offsetof(TdxFpssTrade, ext_condition3) == 20,
              "TdxFpssTrade::ext_condition3 offset drifted");
static_assert(offsetof(TdxFpssTrade, ext_condition4) == 24,
              "TdxFpssTrade::ext_condition4 offset drifted");
static_assert(offsetof(TdxFpssTrade, condition) == 28,
              "TdxFpssTrade::condition offset drifted");
static_assert(offsetof(TdxFpssTrade, size) == 32,
              "TdxFpssTrade::size offset drifted");
static_assert(offsetof(TdxFpssTrade, exchange) == 36,
              "TdxFpssTrade::exchange offset drifted");
static_assert(offsetof(TdxFpssTrade, price) == 40,
              "TdxFpssTrade::price offset drifted");
static_assert(offsetof(TdxFpssTrade, condition_flags) == 48,
              "TdxFpssTrade::condition_flags offset drifted");
static_assert(offsetof(TdxFpssTrade, price_flags) == 52,
              "TdxFpssTrade::price_flags offset drifted");
static_assert(offsetof(TdxFpssTrade, volume_type) == 56,
              "TdxFpssTrade::volume_type offset drifted");
static_assert(offsetof(TdxFpssTrade, records_back) == 60,
              "TdxFpssTrade::records_back offset drifted");
static_assert(offsetof(TdxFpssTrade, date) == 64,
              "TdxFpssTrade::date offset drifted");
static_assert(offsetof(TdxFpssTrade, received_at_ns) == 72,
              "TdxFpssTrade::received_at_ns offset drifted");
static_assert(sizeof(TdxFpssTrade) == 80,
              "TdxFpssTrade total size drifted");

// TdxFpssControl — sub-type tag + id + optional detail string.
static_assert(offsetof(TdxFpssControl, kind) == 0,
              "TdxFpssControl::kind offset drifted");
static_assert(offsetof(TdxFpssControl, id) == 4,
              "TdxFpssControl::id offset drifted");
static_assert(offsetof(TdxFpssControl, detail) == 8,
              "TdxFpssControl::detail offset drifted");
static_assert(sizeof(TdxFpssControl) == 16,
              "TdxFpssControl total size drifted");

// TdxFpssRawData — unrecognized wire frame passthrough.
static_assert(offsetof(TdxFpssRawData, code) == 0,
              "TdxFpssRawData::code offset drifted");
static_assert(offsetof(TdxFpssRawData, payload) == 8,
              "TdxFpssRawData::payload offset drifted");
static_assert(offsetof(TdxFpssRawData, payload_len) == 16,
              "TdxFpssRawData::payload_len offset drifted");
static_assert(sizeof(TdxFpssRawData) == 24,
              "TdxFpssRawData total size drifted");

// TdxFpssEvent — the tagged wrapper. Field ORDER must match the Rust
// `#[repr(C)] struct TdxFpssEvent` exactly: { kind, ohlcvc, open_interest,
// quote, trade, control, raw_data }. Every field's offset is reproduced
// below so swapping two same-size variants (which would pass a sizeof
// check) still fails the build.
static_assert(offsetof(TdxFpssEvent, kind) == 0,
              "TdxFpssEvent::kind offset drifted");
static_assert(offsetof(TdxFpssEvent, ohlcvc) == 8,
              "TdxFpssEvent::ohlcvc offset drifted");
static_assert(offsetof(TdxFpssEvent, open_interest) == 80,
              "TdxFpssEvent::open_interest offset drifted");
static_assert(offsetof(TdxFpssEvent, quote) == 104,
              "TdxFpssEvent::quote offset drifted");
static_assert(offsetof(TdxFpssEvent, trade) == 168,
              "TdxFpssEvent::trade offset drifted");
static_assert(offsetof(TdxFpssEvent, control) == 248,
              "TdxFpssEvent::control offset drifted");
static_assert(offsetof(TdxFpssEvent, raw_data) == 264,
              "TdxFpssEvent::raw_data offset drifted");
static_assert(sizeof(TdxFpssEvent) == 288,
              "TdxFpssEvent total size drifted");

// OptionContract uses std::string for root to avoid use-after-free.
// The C FFI TdxOptionContract uses a raw char* that is freed with the array,
// so we deep-copy the string during conversion.
struct OptionContract {
    std::string root;
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

struct FpssEventDeleter {
    void operator()(TdxFpssEvent* p) const { if (p) tdx_fpss_event_free(p); }
};

/** Owned FPSS event pointer. Automatically freed when destroyed. */
using FpssEventPtr = std::unique_ptr<TdxFpssEvent, FpssEventDeleter>;

class FpssClient {
public:
    #include "fpss.hpp.inc"
    ~FpssClient();

    FpssClient(const FpssClient&) = delete;
    FpssClient& operator=(const FpssClient&) = delete;
    FpssClient(FpssClient&& other) noexcept : handle_(std::move(other.handle_)) {}
    FpssClient& operator=(FpssClient&& other) noexcept {
        handle_ = std::move(other.handle_);
        return *this;
    }

    /** Cumulative count of FPSS events dropped because the internal
     *  receiver was gone (channel disconnected) when the callback tried
     *  to deliver. Survives `reconnect()`. Parity with the Python
     *  `tdx.dropped_events()` / TypeScript `tdx.droppedEvents()` /
     *  Go `client.DroppedEvents()` getters. Safe to call on a moved-from
     *  client (returns 0). */
    uint64_t dropped_events() const {
        return handle_ ? tdx_fpss_dropped_events(handle_.get()) : 0;
    }

private:
    std::unique_ptr<TdxFpssHandle, FpssHandleDeleter> handle_;
};

// ── Standalone Greeks functions ──

#include "utilities.hpp.inc"

} // namespace tdx

#endif /* THETADX_HPP */
