/**
 * thetadatadx C++ SDK.
 *
 * RAII wrappers around the C FFI layer. Provides idiomatic C++ access to
 * ThetaData market data with automatic resource management.
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

namespace tdx {

// ── Tick types ──

struct EodTick {
    int ms_of_day;
    double open;
    double high;
    double low;
    double close;
    int volume;
    int count;
    double bid;
    double ask;
    int date;
};

struct OhlcTick {
    int ms_of_day;
    double open;
    double high;
    double low;
    double close;
    int volume;
    int count;
    int date;
};

struct TradeTick {
    int ms_of_day;
    int sequence;
    int condition;
    int size;
    int exchange;
    double price;
    int price_raw;
    int price_type;
    int condition_flags;
    int price_flags;
    int volume_type;
    int records_back;
    int date;
};

struct QuoteTick {
    int ms_of_day;
    int bid_size;
    int bid_exchange;
    double bid;
    int bid_condition;
    int ask_size;
    int ask_exchange;
    double ask;
    int ask_condition;
    int date;
};

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

// ── DataTable-derived tick types ──
// These structs are parsed from the DataTable JSON format returned by the FFI layer.
// DataTable format: {"headers": ["col1", ...], "rows": [[val1, ...], ...]}
// Price cells are {"value": N, "type": T} objects; decoded to double by the parser.

/** Combined trade + quote tick (25 fields). Mirrors Rust TradeQuoteTick. */
struct TradeQuoteTick {
    int ms_of_day;
    int sequence;
    int ext_condition1;
    int ext_condition2;
    int ext_condition3;
    int ext_condition4;
    int condition;
    int size;
    int exchange;
    double price;
    int condition_flags;
    int price_flags;
    int volume_type;
    int records_back;
    int quote_ms_of_day;
    int bid_size;
    int bid_exchange;
    double bid;
    int bid_condition;
    int ask_size;
    int ask_exchange;
    double ask;
    int ask_condition;
    int date;
};

/** Open interest tick (3 fields). Mirrors Rust OpenInterestTick. */
struct OpenInterestTick {
    int ms_of_day;
    int open_interest;
    int date;
};

/** Greeks tick with timestamp. For greeks history and greeks snapshot endpoints. */
struct GreeksTick {
    int ms_of_day;
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
    int date;
};

/** Implied volatility tick. For IV-only snapshot and history endpoints. */
struct IvTick {
    int ms_of_day;
    double iv;
    double iv_error;
    int date;
};

/** Index price tick. For index price snapshot, history, and at-time endpoints. */
struct PriceTick {
    int ms_of_day;
    double price;
    int date;
};

/** Market value tick. For stock/option/index market value endpoints. */
struct MarketValueTick {
    int ms_of_day;
    double market_cap;
    int64_t shares_outstanding;
    double enterprise_value;
    double book_value;
    int64_t free_float;
    int date;
};

/** Option contract descriptor. For option_list_contracts. */
struct OptionContract {
    std::string root;
    int expiration;
    int strike;
    std::string right;
};

/** Calendar day entry. For calendar endpoints. */
struct CalendarDay {
    int date;
    int is_open;
    int open_time;
    int close_time;
    std::string status;
};

/** Interest rate EOD tick. For interest_rate_history_eod. */
struct InterestRateTick {
    int ms_of_day;
    double rate;
    int date;
};

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

    /** Dev config (shorter timeouts). */
    static Config dev();

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

    /** 19. Get latest OHLC snapshot for options. */
    std::vector<OhlcTick> option_snapshot_ohlc(const std::string& symbol, const std::string& expiration,
                                               const std::string& strike, const std::string& right) const;

    /** 20. Get latest trade snapshot for options. */
    std::vector<TradeTick> option_snapshot_trade(const std::string& symbol, const std::string& expiration,
                                                  const std::string& strike, const std::string& right) const;

    /** 21. Get latest quote snapshot for options. */
    std::vector<QuoteTick> option_snapshot_quote(const std::string& symbol, const std::string& expiration,
                                                  const std::string& strike, const std::string& right) const;

    /** 22. Get latest open interest snapshot. */
    std::vector<OpenInterestTick> option_snapshot_open_interest(const std::string& symbol, const std::string& expiration,
                                                                const std::string& strike, const std::string& right) const;

    /** 23. Get latest market value snapshot for options. */
    std::vector<MarketValueTick> option_snapshot_market_value(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right) const;

    /** 24. Get IV snapshot. */
    std::vector<IvTick> option_snapshot_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                  const std::string& strike, const std::string& right) const;

    /** 25. Get all Greeks snapshot. */
    std::vector<GreeksTick> option_snapshot_greeks_all(const std::string& symbol, const std::string& expiration,
                                                       const std::string& strike, const std::string& right) const;

    /** 26. Get first-order Greeks snapshot. */
    std::vector<GreeksTick> option_snapshot_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right) const;

    /** 27. Get second-order Greeks snapshot. */
    std::vector<GreeksTick> option_snapshot_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                                const std::string& strike, const std::string& right) const;

    /** 28. Get third-order Greeks snapshot. */
    std::vector<GreeksTick> option_snapshot_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — History endpoints (6)
    // ═══════════════════════════════════════════════════════════════

    /** 29. Fetch EOD option data. */
    std::vector<EodTick> option_history_eod(const std::string& symbol, const std::string& expiration,
                                            const std::string& strike, const std::string& right,
                                            const std::string& start_date, const std::string& end_date) const;

    /** 30. Fetch intraday OHLC for options. */
    std::vector<OhlcTick> option_history_ohlc(const std::string& symbol, const std::string& expiration,
                                              const std::string& strike, const std::string& right,
                                              const std::string& date, const std::string& interval) const;

    /** 31. Fetch all trades for an option. */
    std::vector<TradeTick> option_history_trade(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& date) const;

    /** 32. Fetch quotes for an option. */
    std::vector<QuoteTick> option_history_quote(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& date, const std::string& interval) const;

    /** 33. Fetch combined trade + quote for an option. */
    std::vector<TradeQuoteTick> option_history_trade_quote(const std::string& symbol, const std::string& expiration,
                                                           const std::string& strike, const std::string& right,
                                                           const std::string& date) const;

    /** 34. Fetch open interest history. */
    std::vector<OpenInterestTick> option_history_open_interest(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right,
                                                               const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — History Greeks endpoints (11)
    // ═══════════════════════════════════════════════════════════════

    /** 35. Fetch EOD Greeks history. */
    std::vector<GreeksTick> option_history_greeks_eod(const std::string& symbol, const std::string& expiration,
                                                      const std::string& strike, const std::string& right,
                                                      const std::string& start_date, const std::string& end_date) const;

    /** 36. Fetch all Greeks history (intraday). */
    std::vector<GreeksTick> option_history_greeks_all(const std::string& symbol, const std::string& expiration,
                                                      const std::string& strike, const std::string& right,
                                                      const std::string& date, const std::string& interval) const;

    /** 37. Fetch all Greeks on each trade. */
    std::vector<GreeksTick> option_history_trade_greeks_all(const std::string& symbol, const std::string& expiration,
                                                            const std::string& strike, const std::string& right,
                                                            const std::string& date) const;

    /** 38. Fetch first-order Greeks history. */
    std::vector<GreeksTick> option_history_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right,
                                                              const std::string& date, const std::string& interval) const;

    /** 39. Fetch first-order Greeks on each trade. */
    std::vector<GreeksTick> option_history_trade_greeks_first_order(const std::string& symbol, const std::string& expiration,
                                                                    const std::string& strike, const std::string& right,
                                                                    const std::string& date) const;

    /** 40. Fetch second-order Greeks history. */
    std::vector<GreeksTick> option_history_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                               const std::string& strike, const std::string& right,
                                                               const std::string& date, const std::string& interval) const;

    /** 41. Fetch second-order Greeks on each trade. */
    std::vector<GreeksTick> option_history_trade_greeks_second_order(const std::string& symbol, const std::string& expiration,
                                                                     const std::string& strike, const std::string& right,
                                                                     const std::string& date) const;

    /** 42. Fetch third-order Greeks history. */
    std::vector<GreeksTick> option_history_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                              const std::string& strike, const std::string& right,
                                                              const std::string& date, const std::string& interval) const;

    /** 43. Fetch third-order Greeks on each trade. */
    std::vector<GreeksTick> option_history_trade_greeks_third_order(const std::string& symbol, const std::string& expiration,
                                                                    const std::string& strike, const std::string& right,
                                                                    const std::string& date) const;

    /** 44. Fetch IV history (intraday). */
    std::vector<IvTick> option_history_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                 const std::string& strike, const std::string& right,
                                                                 const std::string& date, const std::string& interval) const;

    /** 45. Fetch IV on each trade. */
    std::vector<IvTick> option_history_trade_greeks_implied_volatility(const std::string& symbol, const std::string& expiration,
                                                                       const std::string& strike, const std::string& right,
                                                                       const std::string& date) const;

    // ═══════════════════════════════════════════════════════════════
    //  Option — At-Time endpoints (2)
    // ═══════════════════════════════════════════════════════════════

    /** 46. Fetch trade at a specific time for an option. */
    std::vector<TradeTick> option_at_time_trade(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& start_date, const std::string& end_date,
                                                 const std::string& time_of_day) const;

    /** 47. Fetch quote at a specific time for an option. */
    std::vector<QuoteTick> option_at_time_quote(const std::string& symbol, const std::string& expiration,
                                                 const std::string& strike, const std::string& right,
                                                 const std::string& start_date, const std::string& end_date,
                                                 const std::string& time_of_day) const;

    // ═══════════════════════════════════════════════════════════════
    //  Index — List endpoints (2)
    // ═══════════════════════════════════════════════════════════════

    /** 48. List all index symbols. */
    std::vector<std::string> index_list_symbols() const;

    /** 49. List available dates for an index. */
    std::vector<std::string> index_list_dates(const std::string& symbol) const;

    // ═══════════════════════════════════════════════════════════════
    //  Index — Snapshot endpoints (3)
    // ═══════════════════════════════════════════════════════════════

    /** 50. Get latest OHLC snapshot for indices. */
    std::vector<OhlcTick> index_snapshot_ohlc(const std::vector<std::string>& symbols) const;

    /** 51. Get latest price snapshot for indices. */
    std::vector<PriceTick> index_snapshot_price(const std::vector<std::string>& symbols) const;

    /** 52. Get latest market value for indices. */
    std::vector<MarketValueTick> index_snapshot_market_value(const std::vector<std::string>& symbols) const;

    // ═══════════════════════════════════════════════════════════════
    //  Index — History endpoints (3)
    // ═══════════════════════════════════════════════════════════════

    /** 53. Fetch EOD index data. */
    std::vector<EodTick> index_history_eod(const std::string& symbol,
                                           const std::string& start_date,
                                           const std::string& end_date) const;

    /** 54. Fetch intraday OHLC for an index. */
    std::vector<OhlcTick> index_history_ohlc(const std::string& symbol,
                                             const std::string& start_date,
                                             const std::string& end_date,
                                             const std::string& interval) const;

    /** 55. Fetch intraday price history. */
    std::vector<PriceTick> index_history_price(const std::string& symbol,
                                               const std::string& date,
                                               const std::string& interval) const;

    // ═══════════════════════════════════════════════════════════════
    //  Index — At-Time endpoints (1)
    // ═══════════════════════════════════════════════════════════════

    /** 56. Fetch index price at a specific time. */
    std::vector<PriceTick> index_at_time_price(const std::string& symbol,
                                               const std::string& start_date,
                                               const std::string& end_date,
                                               const std::string& time_of_day) const;

    // ═══════════════════════════════════════════════════════════════
    //  Calendar endpoints (3)
    // ═══════════════════════════════════════════════════════════════

    /** 57. Check whether the market is open today. */
    std::vector<CalendarDay> calendar_open_today() const;

    /** 58. Get calendar for a specific date. */
    std::vector<CalendarDay> calendar_on_date(const std::string& date) const;

    /** 59. Get calendar for a year. */
    std::vector<CalendarDay> calendar_year(const std::string& year) const;

    // ═══════════════════════════════════════════════════════════════
    //  Interest Rate endpoints (1)
    // ═══════════════════════════════════════════════════════════════

    /** 60. Fetch EOD interest rate history. */
    std::vector<InterestRateTick> interest_rate_history_eod(const std::string& symbol,
                                                            const std::string& start_date,
                                                            const std::string& end_date) const;

private:
    explicit Client(TdxClient* h) : handle_(h) {}
    std::unique_ptr<TdxClient, ClientDeleter> handle_;
};

// ── FPSS real-time streaming client ──

class FpssClient {
public:
    /** Connect to FPSS streaming servers. Throws on failure. */
    FpssClient(const Credentials& creds, const Config& config);

    /** Subscribe to quote data for a stock symbol. Returns request ID. */
    int subscribe_quotes(const std::string& symbol);

    /** Subscribe to trade data for a stock symbol. Returns request ID. */
    int subscribe_trades(const std::string& symbol);

    /** Subscribe to open interest data for a stock symbol. Returns request ID. */
    int subscribe_open_interest(const std::string& symbol);

    /** Subscribe to all trades for a security type ("STOCK", "OPTION", "INDEX"). Returns request ID. */
    int subscribe_full_trades(const std::string& sec_type);

    /** Unsubscribe from quote data. Returns request ID. */
    int unsubscribe_quotes(const std::string& symbol);

    /** Unsubscribe from open interest data. Returns request ID. */
    int unsubscribe_open_interest(const std::string& symbol);

    /** Unsubscribe from trade data. Returns request ID. */
    int unsubscribe_trades(const std::string& symbol);

    /** Check if the client is currently authenticated. */
    bool is_authenticated() const;

    /** Look up a contract by server-assigned ID. Returns empty optional if not found. */
    std::optional<std::string> contract_lookup(int id) const;

    /** Get active subscriptions as a JSON array string. */
    std::string active_subscriptions() const;

    /** Poll for the next event. Returns JSON string, or empty string on timeout. */
    std::string next_event(uint64_t timeout_ms);

    /** Shut down the FPSS client. */
    void shutdown();

    /** Destructor: shuts down and frees the handle. */
    ~FpssClient();

    // Non-copyable, movable.
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
