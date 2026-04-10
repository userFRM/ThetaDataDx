/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadx.hpp"

#include <limits>
#include <stdexcept>
#include <sstream>

namespace tdx {

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

struct FfiEndpointRequestOptions {
    TdxEndpointRequestOptions raw{};
    std::string venue_storage;
    std::string min_time_storage;
    std::string start_time_storage;
    std::string end_time_storage;
    std::string start_date_storage;
    std::string end_date_storage;
    std::string rate_type_storage;
    std::string version_storage;

    explicit FfiEndpointRequestOptions(const EndpointRequestOptions& options) {
        raw.max_dte = -1;
        raw.strike_range = -1;
        raw.venue = nullptr;
        raw.min_time = nullptr;
        raw.start_time = nullptr;
        raw.end_time = nullptr;
        raw.start_date = nullptr;
        raw.end_date = nullptr;
        raw.exclusive = -1;
        raw.annual_dividend = std::numeric_limits<double>::quiet_NaN();
        raw.rate_type = nullptr;
        raw.rate_value = std::numeric_limits<double>::quiet_NaN();
        raw.stock_price = std::numeric_limits<double>::quiet_NaN();
        raw.version = nullptr;
        raw.underlyer_use_nbbo = -1;
        raw.use_market_value = -1;

        if (options.venue) {
            venue_storage = *options.venue;
            raw.venue = venue_storage.c_str();
        }
        if (options.min_time) {
            min_time_storage = *options.min_time;
            raw.min_time = min_time_storage.c_str();
        }
        if (options.start_time) {
            start_time_storage = *options.start_time;
            raw.start_time = start_time_storage.c_str();
        }
        if (options.end_time) {
            end_time_storage = *options.end_time;
            raw.end_time = end_time_storage.c_str();
        }
        if (options.start_date) {
            start_date_storage = *options.start_date;
            raw.start_date = start_date_storage.c_str();
        }
        if (options.end_date) {
            end_date_storage = *options.end_date;
            raw.end_date = end_date_storage.c_str();
        }
        if (options.exclusive) {
            raw.exclusive = *options.exclusive ? 1 : 0;
        }
        if (options.annual_dividend) {
            raw.annual_dividend = *options.annual_dividend;
        }
        if (options.rate_type) {
            rate_type_storage = *options.rate_type;
            raw.rate_type = rate_type_storage.c_str();
        }
        if (options.rate_value) {
            raw.rate_value = *options.rate_value;
        }
        if (options.stock_price) {
            raw.stock_price = *options.stock_price;
        }
        if (options.version) {
            version_storage = *options.version;
            raw.version = version_storage.c_str();
        }
        if (options.underlyer_use_nbbo) {
            raw.underlyer_use_nbbo = *options.underlyer_use_nbbo ? 1 : 0;
        }
        if (options.use_market_value) {
            raw.use_market_value = *options.use_market_value ? 1 : 0;
        }
        if (options.max_dte) {
            raw.max_dte = *options.max_dte;
        }
        if (options.strike_range) {
            raw.strike_range = *options.strike_range;
        }
    }
};

} // namespace detail

// ── Credentials ──

Credentials Credentials::from_file(const std::string& path) {
    auto h = tdx_credentials_from_file(path.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}

Credentials Credentials::from_email(const std::string& email, const std::string& password) {
    auto h = tdx_credentials_new(email.c_str(), password.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}

// ── Config ──

Config Config::production() { return Config(tdx_config_production()); }
Config Config::dev() { return Config(tdx_config_dev()); }
Config Config::stage() { return Config(tdx_config_stage()); }

// ── Client ──

Client Client::connect(const Credentials& creds, const Config& config) {
    auto h = tdx_client_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Client(h);
}

// ═══════════════════════════════════════════════════════════════
//  Macros for typed array endpoints (no JSON parsing)
// ═══════════════════════════════════════════════════════════════

// Helper macro: call FFI, convert to vector, free the array.
#define TDX_TYPED_ARRAY(arr_type, tick_type, free_fn, call) \
    do { \
        arr_type arr = call; \
        auto result = detail::to_vector(arr.data, arr.len); \
        free_fn(arr); \
        return result; \
    } while (0)

// Helper macro for snapshot endpoints (symbols -> borrowed C string array)
#define TDX_SNAPSHOT(arr_type, tick_type, free_fn, ffi_fn) \
    do { \
        auto symbol_ptrs = detail::string_ptrs(symbols); \
        arr_type arr = ffi_fn(handle_.get(), symbol_ptrs.data(), symbol_ptrs.size()); \
        auto result = detail::to_vector(arr.data, arr.len); \
        free_fn(arr); \
        return result; \
    } while (0)

// ═══════════════════════════════════════════════════════════════
//  Stock — List endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<std::string> Client::stock_list_symbols() const {
    return detail::check_string_array(tdx_stock_list_symbols(handle_.get()));
}

std::vector<std::string> Client::stock_list_dates(const std::string& request_type,
                                                   const std::string& symbol) const {
    return detail::check_string_array(tdx_stock_list_dates(handle_.get(), request_type.c_str(), symbol.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Stock — Snapshot endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<OhlcTick> Client::stock_snapshot_ohlc(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free, tdx_stock_snapshot_ohlc);
}

std::vector<TradeTick> Client::stock_snapshot_trade(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free, tdx_stock_snapshot_trade);
}

std::vector<QuoteTick> Client::stock_snapshot_quote(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free, tdx_stock_snapshot_quote);
}

std::vector<MarketValueTick> Client::stock_snapshot_market_value(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxMarketValueTickArray, MarketValueTick, tdx_market_value_tick_array_free, tdx_stock_snapshot_market_value);
}

// ═══════════════════════════════════════════════════════════════
//  Stock — History endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<EodTick> Client::stock_history_eod(const std::string& symbol, const std::string& start_date, const std::string& end_date) const {
    TDX_TYPED_ARRAY(TdxEodTickArray, EodTick, tdx_eod_tick_array_free,
        tdx_stock_history_eod(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
}

std::vector<OhlcTick> Client::stock_history_ohlc(const std::string& symbol, const std::string& date, const std::string& interval) const {
    TDX_TYPED_ARRAY(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free,
        tdx_stock_history_ohlc(handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
}

std::vector<OhlcTick> Client::stock_history_ohlc_range(const std::string& symbol, const std::string& start_date, const std::string& end_date, const std::string& interval) const {
    TDX_TYPED_ARRAY(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free,
        tdx_stock_history_ohlc_range(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), interval.c_str()));
}

std::vector<TradeTick> Client::stock_history_trade(const std::string& symbol, const std::string& date) const {
    TDX_TYPED_ARRAY(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free,
        tdx_stock_history_trade(handle_.get(), symbol.c_str(), date.c_str()));
}

std::vector<QuoteTick> Client::stock_history_quote(const std::string& symbol, const std::string& date, const std::string& interval) const {
    TDX_TYPED_ARRAY(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free,
        tdx_stock_history_quote(handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
}

std::vector<TradeQuoteTick> Client::stock_history_trade_quote(const std::string& symbol, const std::string& date) const {
    TDX_TYPED_ARRAY(TdxTradeQuoteTickArray, TradeQuoteTick, tdx_trade_quote_tick_array_free,
        tdx_stock_history_trade_quote(handle_.get(), symbol.c_str(), date.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Stock — At-Time endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<TradeTick> Client::stock_at_time_trade(const std::string& symbol, const std::string& start_date, const std::string& end_date, const std::string& time_of_day) const {
    TDX_TYPED_ARRAY(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free,
        tdx_stock_at_time_trade(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
}

std::vector<QuoteTick> Client::stock_at_time_quote(const std::string& symbol, const std::string& start_date, const std::string& end_date, const std::string& time_of_day) const {
    TDX_TYPED_ARRAY(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free,
        tdx_stock_at_time_quote(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Option — List endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<std::string> Client::option_list_symbols() const {
    return detail::check_string_array(tdx_option_list_symbols(handle_.get()));
}

std::vector<std::string> Client::option_list_dates(const std::string& request_type, const std::string& symbol,
                                                    const std::string& expiration, const std::string& strike,
                                                    const std::string& right) const {
    return detail::check_string_array(tdx_option_list_dates(handle_.get(), request_type.c_str(), symbol.c_str(),
                                                             expiration.c_str(), strike.c_str(), right.c_str()));
}

std::vector<std::string> Client::option_list_expirations(const std::string& symbol) const {
    return detail::check_string_array(tdx_option_list_expirations(handle_.get(), symbol.c_str()));
}

std::vector<std::string> Client::option_list_strikes(const std::string& symbol, const std::string& expiration) const {
    return detail::check_string_array(tdx_option_list_strikes(handle_.get(), symbol.c_str(), expiration.c_str()));
}

std::vector<OptionContract> Client::option_list_contracts(const std::string& request_type, const std::string& symbol, const std::string& date) const {
    TdxOptionContractArray arr = tdx_option_list_contracts(handle_.get(), request_type.c_str(), symbol.c_str(), date.c_str());
    std::vector<OptionContract> result;
    result.reserve(arr.len);
    for (size_t i = 0; i < arr.len; ++i) {
        OptionContract c;
        c.root = arr.data[i].root ? std::string(arr.data[i].root) : "";
        c.expiration = arr.data[i].expiration;
        c.strike = arr.data[i].strike;
        c.right = arr.data[i].right;
        result.push_back(std::move(c));
    }
    tdx_option_contract_array_free(arr);
    return result;
}

// ═══════════════════════════════════════════════════════════════
//  Option — Snapshot endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<OhlcTick> Client::option_snapshot_ohlc(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free,
        tdx_option_snapshot_ohlc(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<TradeTick> Client::option_snapshot_trade(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free,
        tdx_option_snapshot_trade(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<QuoteTick> Client::option_snapshot_quote(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free,
        tdx_option_snapshot_quote(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<OpenInterestTick> Client::option_snapshot_open_interest(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxOpenInterestTickArray, OpenInterestTick, tdx_open_interest_tick_array_free,
        tdx_option_snapshot_open_interest(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<MarketValueTick> Client::option_snapshot_market_value(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxMarketValueTickArray, MarketValueTick, tdx_market_value_tick_array_free,
        tdx_option_snapshot_market_value(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<IvTick> Client::option_snapshot_greeks_implied_volatility(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxIvTickArray, IvTick, tdx_iv_tick_array_free,
        tdx_option_snapshot_greeks_implied_volatility(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<GreeksTick> Client::option_snapshot_greeks_all(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_snapshot_greeks_all(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<GreeksTick> Client::option_snapshot_greeks_first_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_snapshot_greeks_first_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<GreeksTick> Client::option_snapshot_greeks_second_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_snapshot_greeks_second_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

std::vector<GreeksTick> Client::option_snapshot_greeks_third_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_snapshot_greeks_third_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Option — History endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<EodTick> Client::option_history_eod(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& sd, const std::string& ed) const {
    TDX_TYPED_ARRAY(TdxEodTickArray, EodTick, tdx_eod_tick_array_free,
        tdx_option_history_eod(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), sd.c_str(), ed.c_str()));
}

std::vector<OhlcTick> Client::option_history_ohlc(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free,
        tdx_option_history_ohlc(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<TradeTick> Client::option_history_trade(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free,
        tdx_option_history_trade(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<QuoteTick> Client::option_history_quote(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free,
        tdx_option_history_quote(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<TradeQuoteTick> Client::option_history_trade_quote(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxTradeQuoteTickArray, TradeQuoteTick, tdx_trade_quote_tick_array_free,
        tdx_option_history_trade_quote(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<OpenInterestTick> Client::option_history_open_interest(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxOpenInterestTickArray, OpenInterestTick, tdx_open_interest_tick_array_free,
        tdx_option_history_open_interest(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<GreeksTick> Client::option_history_greeks_eod(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& sd, const std::string& ed, const EndpointRequestOptions& options) const {
    detail::FfiEndpointRequestOptions ffi_options(options);
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_eod_with_options(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), sd.c_str(), ed.c_str(), &ffi_options.raw));
}

std::vector<GreeksTick> Client::option_history_greeks_all(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_all(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<GreeksTick> Client::option_history_trade_greeks_all(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_trade_greeks_all(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<GreeksTick> Client::option_history_greeks_first_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_first_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<GreeksTick> Client::option_history_trade_greeks_first_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_trade_greeks_first_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<GreeksTick> Client::option_history_greeks_second_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_second_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<GreeksTick> Client::option_history_trade_greeks_second_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_trade_greeks_second_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<GreeksTick> Client::option_history_greeks_third_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_third_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<GreeksTick> Client::option_history_trade_greeks_third_order(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_trade_greeks_third_order(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

std::vector<IvTick> Client::option_history_greeks_implied_volatility(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d, const std::string& iv) const {
    TDX_TYPED_ARRAY(TdxIvTickArray, IvTick, tdx_iv_tick_array_free,
        tdx_option_history_greeks_implied_volatility(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str(), iv.c_str()));
}

std::vector<IvTick> Client::option_history_trade_greeks_implied_volatility(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& d) const {
    TDX_TYPED_ARRAY(TdxIvTickArray, IvTick, tdx_iv_tick_array_free,
        tdx_option_history_trade_greeks_implied_volatility(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), d.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Option — At-Time endpoints
// ═══════════════════════════════════════════════════════════════

std::vector<TradeTick> Client::option_at_time_trade(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& sd, const std::string& ed, const std::string& tod) const {
    TDX_TYPED_ARRAY(TdxTradeTickArray, TradeTick, tdx_trade_tick_array_free,
        tdx_option_at_time_trade(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), sd.c_str(), ed.c_str(), tod.c_str()));
}

std::vector<QuoteTick> Client::option_at_time_quote(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& sd, const std::string& ed, const std::string& tod) const {
    TDX_TYPED_ARRAY(TdxQuoteTickArray, QuoteTick, tdx_quote_tick_array_free,
        tdx_option_at_time_quote(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), sd.c_str(), ed.c_str(), tod.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Index
// ═══════════════════════════════════════════════════════════════

std::vector<std::string> Client::index_list_symbols() const {
    return detail::check_string_array(tdx_index_list_symbols(handle_.get()));
}

std::vector<std::string> Client::index_list_dates(const std::string& symbol) const {
    return detail::check_string_array(tdx_index_list_dates(handle_.get(), symbol.c_str()));
}

std::vector<OhlcTick> Client::index_snapshot_ohlc(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free, tdx_index_snapshot_ohlc);
}

std::vector<PriceTick> Client::index_snapshot_price(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxPriceTickArray, PriceTick, tdx_price_tick_array_free, tdx_index_snapshot_price);
}

std::vector<MarketValueTick> Client::index_snapshot_market_value(const std::vector<std::string>& symbols) const {
    TDX_SNAPSHOT(TdxMarketValueTickArray, MarketValueTick, tdx_market_value_tick_array_free, tdx_index_snapshot_market_value);
}

std::vector<EodTick> Client::index_history_eod(const std::string& symbol, const std::string& start_date, const std::string& end_date) const {
    TDX_TYPED_ARRAY(TdxEodTickArray, EodTick, tdx_eod_tick_array_free,
        tdx_index_history_eod(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
}

std::vector<OhlcTick> Client::index_history_ohlc(const std::string& symbol, const std::string& start_date, const std::string& end_date, const std::string& interval) const {
    TDX_TYPED_ARRAY(TdxOhlcTickArray, OhlcTick, tdx_ohlc_tick_array_free,
        tdx_index_history_ohlc(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), interval.c_str()));
}

std::vector<PriceTick> Client::index_history_price(const std::string& symbol, const std::string& date, const std::string& interval) const {
    TDX_TYPED_ARRAY(TdxPriceTickArray, PriceTick, tdx_price_tick_array_free,
        tdx_index_history_price(handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
}

std::vector<PriceTick> Client::index_at_time_price(const std::string& symbol, const std::string& start_date, const std::string& end_date, const std::string& time_of_day) const {
    TDX_TYPED_ARRAY(TdxPriceTickArray, PriceTick, tdx_price_tick_array_free,
        tdx_index_at_time_price(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
}

// ═══════════════════════════════════════════════════════════════
//  Calendar + Interest Rate
// ═══════════════════════════════════════════════════════════════

std::vector<CalendarDay> Client::calendar_open_today() const {
    TDX_TYPED_ARRAY(TdxCalendarDayArray, CalendarDay, tdx_calendar_day_array_free,
        tdx_calendar_open_today(handle_.get()));
}

std::vector<CalendarDay> Client::calendar_on_date(const std::string& date) const {
    TDX_TYPED_ARRAY(TdxCalendarDayArray, CalendarDay, tdx_calendar_day_array_free,
        tdx_calendar_on_date(handle_.get(), date.c_str()));
}

std::vector<CalendarDay> Client::calendar_year(const std::string& year) const {
    TDX_TYPED_ARRAY(TdxCalendarDayArray, CalendarDay, tdx_calendar_day_array_free,
        tdx_calendar_year(handle_.get(), year.c_str()));
}

std::vector<InterestRateTick> Client::interest_rate_history_eod(const std::string& symbol, const std::string& start_date, const std::string& end_date) const {
    TDX_TYPED_ARRAY(TdxInterestRateTickArray, InterestRateTick, tdx_interest_rate_tick_array_free,
        tdx_interest_rate_history_eod(handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
}

#undef TDX_TYPED_ARRAY
#undef TDX_SNAPSHOT

// ═══════════════════════════════════════════════════════════════
//  FPSS (streaming) — typed #[repr(C)] events
// ═══════════════════════════════════════════════════════════════

FpssClient::FpssClient(const Credentials& creds, const Config& config) {
    auto h = tdx_fpss_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    handle_.reset(h);
}

int FpssClient::subscribe_quotes(const std::string& symbol) { return tdx_fpss_subscribe_quotes(handle_.get(), symbol.c_str()); }
int FpssClient::subscribe_trades(const std::string& symbol) { return tdx_fpss_subscribe_trades(handle_.get(), symbol.c_str()); }
int FpssClient::subscribe_open_interest(const std::string& symbol) { return tdx_fpss_subscribe_open_interest(handle_.get(), symbol.c_str()); }
int FpssClient::subscribe_full_trades(const std::string& sec_type) { return tdx_fpss_subscribe_full_trades(handle_.get(), sec_type.c_str()); }
int FpssClient::subscribe_full_open_interest(const std::string& sec_type) { return tdx_fpss_subscribe_full_open_interest(handle_.get(), sec_type.c_str()); }
int FpssClient::unsubscribe_quotes(const std::string& symbol) { return tdx_fpss_unsubscribe_quotes(handle_.get(), symbol.c_str()); }
int FpssClient::unsubscribe_open_interest(const std::string& symbol) { return tdx_fpss_unsubscribe_open_interest(handle_.get(), symbol.c_str()); }
int FpssClient::unsubscribe_trades(const std::string& symbol) { return tdx_fpss_unsubscribe_trades(handle_.get(), symbol.c_str()); }
int FpssClient::unsubscribe_full_trades(const std::string& sec_type) { return tdx_fpss_unsubscribe_full_trades(handle_.get(), sec_type.c_str()); }
int FpssClient::unsubscribe_full_open_interest(const std::string& sec_type) { return tdx_fpss_unsubscribe_full_open_interest(handle_.get(), sec_type.c_str()); }

bool FpssClient::is_authenticated() const { return tdx_fpss_is_authenticated(handle_.get()) != 0; }

std::optional<std::string> FpssClient::contract_lookup(int id) const {
    detail::FfiString result(tdx_fpss_contract_lookup(handle_.get(), id));
    if (!result.ok()) return std::nullopt;
    return result.str();
}

std::vector<Subscription> FpssClient::active_subscriptions() const {
    return detail::subscription_array_to_vector(tdx_fpss_active_subscriptions(handle_.get()));
}

FpssEventPtr FpssClient::next_event(uint64_t timeout_ms) {
    auto* raw = tdx_fpss_next_event(handle_.get(), timeout_ms);
    return FpssEventPtr(raw);
}

void FpssClient::shutdown() { tdx_fpss_shutdown(handle_.get()); }

FpssClient::~FpssClient() {
    if (handle_) {
        tdx_fpss_shutdown(handle_.get());
    }
}

// ═══════════════════════════════════════════════════════════════
//  Standalone Greeks — still JSON-based (single-value, not arrays)
// ═══════════════════════════════════════════════════════════════

Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, bool is_call) {
    TdxGreeksResult* raw = tdx_all_greeks(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        is_call ? 1 : 0
    );
    if (raw == nullptr) {
        throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    }

    Greeks result{
        raw->value,
        raw->delta,
        raw->gamma,
        raw->theta,
        raw->vega,
        raw->rho,
        raw->iv,
        raw->iv_error,
        raw->vanna,
        raw->charm,
        raw->vomma,
        raw->veta,
        raw->speed,
        raw->zomma,
        raw->color,
        raw->ultima,
        raw->d1,
        raw->d2,
        raw->dual_delta,
        raw->dual_gamma,
        raw->epsilon,
        raw->lambda,
    };
    tdx_greeks_result_free(raw);
    return result;
}

std::pair<double, double> implied_volatility(double spot, double strike,
                                              double rate, double div_yield,
                                              double tte, double option_price,
                                              bool is_call) {
    double iv = 0.0, err = 0.0;
    int rc = tdx_implied_volatility(spot, strike, rate, div_yield, tte, option_price, is_call ? 1 : 0, &iv, &err);
    if (rc != 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return {iv, err};
}

} // namespace tdx
