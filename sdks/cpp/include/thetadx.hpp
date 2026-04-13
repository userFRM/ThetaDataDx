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

// Check a TdxStringArray for errors (empty may be an error).
inline std::vector<std::string> check_string_array(TdxStringArray arr) {
    // Note: empty array is valid (no results), not an error.
    // Errors are signaled by tdx_last_error().
    return string_array_to_vector(arr);
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

private:
    std::unique_ptr<TdxFpssHandle, FpssHandleDeleter> handle_;
};

// ── Standalone Greeks functions ──

#include "utilities.hpp.inc"

} // namespace tdx

#endif /* THETADX_HPP */
