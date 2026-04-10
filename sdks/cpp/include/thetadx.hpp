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
using SnapshotTradeTick = TdxSnapshotTradeTick;
using TradeQuoteTick = TdxTradeQuoteTick;
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

/// Optional builder parameters for registry-driven endpoint wrappers.
///
/// Rust models these as builder setters. The C++ wrapper projects the same
/// surface as a value object with `std::optional` fields.
struct EndpointRequestOptions {
    std::optional<std::string> venue;
    std::optional<std::string> min_time;
    std::optional<std::string> start_time;
    std::optional<std::string> end_time;
    std::optional<std::string> start_date;
    std::optional<std::string> end_date;
    std::optional<bool> exclusive;
    std::optional<double> annual_dividend;
    std::optional<std::string> rate_type;
    std::optional<double> rate_value;
    std::optional<double> stock_price;
    std::optional<std::string> version;
    std::optional<bool> underlyer_use_nbbo;
    std::optional<bool> use_market_value;
    std::optional<int32_t> max_dte;
    std::optional<int32_t> strike_range;
};

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

    // ═══════════════════════════════════════════════════════════════
    //  Stock — List endpoints (2)
    // ═══════════════════════════════════════════════════════════════

    /** 1. List all stock symbols. */
    std::vector<std::string> stock_list_symbols() const;

    /** 2. List available dates for a stock. */
    std::vector<std::string> stock_list_dates(const std::string& request_type,
                                               const std::string& symbol) const;

    // ═══════════════════════════════════════════════════════════════
    //  Stock — Snapshot endpoints (4)
    // ═══════════════════════════════════════════════════════════════

    /** 3. Get latest OHLC snapshot. */
    std::vector<OhlcTick> stock_snapshot_ohlc(const std::vector<std::string>& symbols) const;

    /** 4. Get latest trade snapshot. */
    std::vector<TradeTick> stock_snapshot_trade(const std::vector<std::string>& symbols) const;

    /** 5. Get latest NBBO quote snapshot. */
    std::vector<QuoteTick> stock_snapshot_quote(const std::vector<std::string>& symbols) const;

    /** 6. Get latest market value snapshot. */
    std::vector<MarketValueTick> stock_snapshot_market_value(const std::vector<std::string>& symbols) const;

    // ═══════════════════════════════════════════════════════════════
    //  Stock — History endpoints (5 + bonus)
    // ═══════════════════════════════════════════════════════════════

    /** 7. Fetch EOD stock data. */
    std::vector<EodTick> stock_history_eod(const std::string& symbol,
                                           const std::string& start_date,
                                           const std::string& end_date) const;

    /** 8. Fetch intraday OHLC bars. */
    std::vector<OhlcTick> stock_history_ohlc(const std::string& symbol,
                                             const std::string& date,
                                             const std::string& interval) const;

    /** 8b. Fetch OHLC bars across date range. */
    std::vector<OhlcTick> stock_history_ohlc_range(const std::string& symbol,
                                                    const std::string& start_date,
                                                    const std::string& end_date,
                                                    const std::string& interval) const;

    /** 9. Fetch all trades on a date. */
    std::vector<TradeTick> stock_history_trade(const std::string& symbol,
                                               const std::string& date) const;

    /** 10. Fetch NBBO quotes. */
    std::vector<QuoteTick> stock_history_quote(const std::string& symbol,
                                               const std::string& date,
                                               const std::string& interval) const;

    /** 11. Fetch combined trade + quote ticks. */
    std::vector<TradeQuoteTick> stock_history_trade_quote(const std::string& symbol,
                                                          const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Stock — At-Time endpoints (2)
    // ═══════════════════════════════════════════════════════════════

    /** 12. Fetch trade at a specific time across date range. */
    std::vector<TradeTick> stock_at_time_trade(const std::string& symbol,
                                                const std::string& start_date,
                                                const std::string& end_date,
                                                const std::string& time_of_day) const;

    /** 13. Fetch quote at a specific time across date range. */
    std::vector<QuoteTick> stock_at_time_quote(const std::string& symbol,
                                                const std::string& start_date,
                                                const std::string& end_date,
                                                const std::string& time_of_day) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — List endpoints (5)
    // ═══════════════════════════════════════════════════════════════

    /** 14. List all option underlyings. */
    std::vector<std::string> option_list_symbols() const;

    /** 15. List available dates for an option contract. */
    std::vector<std::string> option_list_dates(const std::string& request_type,
                                                const std::string& symbol,
                                                const std::string& expiration,
                                                const std::string& strike,
                                                const std::string& right) const;

    /** 16. List expiration dates. */
    std::vector<std::string> option_list_expirations(const std::string& symbol) const;

    /** 17. List strike prices. */
    std::vector<std::string> option_list_strikes(const std::string& symbol,
                                                  const std::string& expiration) const;

    /** 18. List all option contracts on a date. */
    std::vector<OptionContract> option_list_contracts(const std::string& request_type,
                                                      const std::string& symbol,
                                                      const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — Snapshot endpoints (10)
    // ═══════════════════════════════════════════════════════════════

    std::vector<OhlcTick> option_snapshot_ohlc(const std::string& symbol, const std::string& expiration,
                                               const std::string& strike, const std::string& right) const;
    std::vector<TradeTick> option_snapshot_trade(const std::string& symbol, const std::string& expiration,
                                                  const std::string& strike, const std::string& right) const;
    std::vector<QuoteTick> option_snapshot_quote(const std::string& symbol, const std::string& expiration,
                                                  const std::string& strike, const std::string& right) const;
    std::vector<OpenInterestTick> option_snapshot_open_interest(const std::string& symbol, const std::string& expiration,
                                                                const std::string& strike, const std::string& right) const;
    std::vector<MarketValueTick> option_snapshot_market_value(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right) const;
    std::vector<IvTick> option_snapshot_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                  const std::string& strike, const std::string& right) const;
    std::vector<GreeksTick> option_snapshot_greeks_all(const std::string& symbol, const std::string& expiration,
                                                       const std::string& strike, const std::string& right) const;
    std::vector<GreeksTick> option_snapshot_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right) const;
    std::vector<GreeksTick> option_snapshot_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                                const std::string& strike, const std::string& right) const;
    std::vector<GreeksTick> option_snapshot_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — History endpoints (6)
    // ═══════════════════════════════════════════════════════════════

    std::vector<EodTick> option_history_eod(const std::string& symbol, const std::string& expiration,
                                            const std::string& strike, const std::string& right,
                                            const std::string& start_date, const std::string& end_date) const;
    std::vector<OhlcTick> option_history_ohlc(const std::string& symbol, const std::string& expiration,
                                              const std::string& strike, const std::string& right,
                                              const std::string& date, const std::string& interval) const;
    std::vector<TradeTick> option_history_trade(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& date) const;
    std::vector<QuoteTick> option_history_quote(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& date, const std::string& interval) const;
    std::vector<TradeQuoteTick> option_history_trade_quote(const std::string& symbol, const std::string& expiration,
                                                           const std::string& strike, const std::string& right,
                                                           const std::string& date) const;
    std::vector<OpenInterestTick> option_history_open_interest(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right,
                                                               const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — History Greeks endpoints (11)
    // ═══════════════════════════════════════════════════════════════

    std::vector<GreeksTick> option_history_greeks_eod(const std::string& symbol, const std::string& expiration,
                                                      const std::string& strike, const std::string& right,
                                                      const std::string& start_date, const std::string& end_date,
                                                      const EndpointRequestOptions& options = {}) const;
    std::vector<GreeksTick> option_history_greeks_all(const std::string& symbol, const std::string& expiration,
                                                      const std::string& strike, const std::string& right,
                                                      const std::string& date, const std::string& interval) const;
    std::vector<GreeksTick> option_history_trade_greeks_all(const std::string& symbol, const std::string& expiration,
                                                            const std::string& strike, const std::string& right,
                                                            const std::string& date) const;
    std::vector<GreeksTick> option_history_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right,
                                                              const std::string& date, const std::string& interval) const;
    std::vector<GreeksTick> option_history_trade_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                                    const std::string& strike, const std::string& right,
                                                                    const std::string& date) const;
    std::vector<GreeksTick> option_history_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right,
                                                               const std::string& date, const std::string& interval) const;
    std::vector<GreeksTick> option_history_trade_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                                     const std::string& strike, const std::string& right,
                                                                     const std::string& date) const;
    std::vector<GreeksTick> option_history_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right,
                                                              const std::string& date, const std::string& interval) const;
    std::vector<GreeksTick> option_history_trade_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                                    const std::string& strike, const std::string& right,
                                                                    const std::string& date) const;
    std::vector<IvTick> option_history_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                 const std::string& strike, const std::string& right,
                                                                 const std::string& date, const std::string& interval) const;
    std::vector<IvTick> option_history_trade_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                       const std::string& strike, const std::string& right,
                                                                       const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — At-Time endpoints (2)
    // ═══════════════════════════════════════════════════════════════

    std::vector<TradeTick> option_at_time_trade(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& start_date, const std::string& end_date,
                                                 const std::string& time_of_day) const;
    std::vector<QuoteTick> option_at_time_quote(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& start_date, const std::string& end_date,
                                                 const std::string& time_of_day) const;

    // ═══════════════════════════════════════════════════════════════
    //  Index — List + Snapshot + History + At-Time
    // ═══════════════════════════════════════════════════════════════

    std::vector<std::string> index_list_symbols() const;
    std::vector<std::string> index_list_dates(const std::string& symbol) const;

    std::vector<OhlcTick> index_snapshot_ohlc(const std::vector<std::string>& symbols) const;
    std::vector<PriceTick> index_snapshot_price(const std::vector<std::string>& symbols) const;
    std::vector<MarketValueTick> index_snapshot_market_value(const std::vector<std::string>& symbols) const;

    std::vector<EodTick> index_history_eod(const std::string& symbol,
                                           const std::string& start_date,
                                           const std::string& end_date) const;
    std::vector<OhlcTick> index_history_ohlc(const std::string& symbol,
                                             const std::string& start_date,
                                             const std::string& end_date,
                                             const std::string& interval) const;
    std::vector<PriceTick> index_history_price(const std::string& symbol,
                                               const std::string& date,
                                               const std::string& interval) const;

    std::vector<PriceTick> index_at_time_price(const std::string& symbol,
                                               const std::string& start_date,
                                               const std::string& end_date,
                                               const std::string& time_of_day) const;

    // ═══════════════════════════════════════════════════════════════
    //  Calendar + Interest Rate
    // ═══════════════════════════════════════════════════════════════

    std::vector<CalendarDay> calendar_open_today() const;
    std::vector<CalendarDay> calendar_on_date(const std::string& date) const;
    std::vector<CalendarDay> calendar_year(const std::string& year) const;

    std::vector<InterestRateTick> interest_rate_history_eod(const std::string& symbol,
                                                            const std::string& start_date,
                                                            const std::string& end_date) const;

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
    /** Connect to FPSS streaming servers. Throws on failure. */
    FpssClient(const Credentials& creds, const Config& config);

    int subscribe_quotes(const std::string& symbol);
    int subscribe_trades(const std::string& symbol);
    int subscribe_open_interest(const std::string& symbol);
    int subscribe_full_trades(const std::string& sec_type);
    int subscribe_full_open_interest(const std::string& sec_type);
    int unsubscribe_quotes(const std::string& symbol);
    int unsubscribe_open_interest(const std::string& symbol);
    int unsubscribe_trades(const std::string& symbol);
    int unsubscribe_full_trades(const std::string& sec_type);
    int unsubscribe_full_open_interest(const std::string& sec_type);

    bool is_authenticated() const;
    std::optional<std::string> contract_lookup(int id) const;
    std::vector<Subscription> active_subscriptions() const;

    /** Poll for the next event as a typed struct. Returns nullptr on timeout. */
    FpssEventPtr next_event(uint64_t timeout_ms);

    void shutdown();
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

/** Compute all 22 Greeks + IV. Throws on failure. */
Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, bool is_call);

/** Compute implied volatility. Returns (iv, error). Throws on failure. */
std::pair<double, double> implied_volatility(double spot, double strike,
                                              double rate, double div_yield,
                                              double tte, double option_price,
                                              bool is_call);

} // namespace tdx

#endif /* THETADX_HPP */
