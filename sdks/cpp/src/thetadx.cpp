/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadx.hpp"

#include <stdexcept>
#include <sstream>

namespace tdx {

namespace detail {

// Build a JSON array string from a vector of strings: ["a","b","c"]
static std::string build_json_array(const std::vector<std::string>& items) {
    std::string json = "[";
    for (size_t i = 0; i < items.size(); ++i) {
        if (i > 0) json += ",";
        json += "\"" + items[i] + "\"";
    }
    json += "]";
    return json;
}

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

// Helper macro for snapshot endpoints (symbols -> JSON array)
#define TDX_SNAPSHOT(arr_type, tick_type, free_fn, ffi_fn) \
    do { \
        auto json = detail::build_json_array(symbols); \
        arr_type arr = ffi_fn(handle_.get(), json.c_str()); \
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
    auto result = detail::to_vector(arr.data, arr.len);
    // NOTE: We copy the data but do NOT free the strings inside — they are still
    // owned by the Rust FFI array. The caller's OptionContract.root pointers
    // will be invalidated when we free the array. For safe use, copy root strings
    // before freeing. This is a known limitation of the zero-copy approach.
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

std::vector<GreeksTick> Client::option_history_greeks_eod(const std::string& s, const std::string& e, const std::string& k, const std::string& r, const std::string& sd, const std::string& ed) const {
    TDX_TYPED_ARRAY(TdxGreeksTickArray, GreeksTick, tdx_greeks_tick_array_free,
        tdx_option_history_greeks_eod(handle_.get(), s.c_str(), e.c_str(), k.c_str(), r.c_str(), sd.c_str(), ed.c_str()));
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
//  FPSS (streaming) — unchanged, still JSON-based
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
int FpssClient::unsubscribe_quotes(const std::string& symbol) { return tdx_fpss_unsubscribe_quotes(handle_.get(), symbol.c_str()); }
int FpssClient::unsubscribe_open_interest(const std::string& symbol) { return tdx_fpss_unsubscribe_open_interest(handle_.get(), symbol.c_str()); }
int FpssClient::unsubscribe_trades(const std::string& symbol) { return tdx_fpss_unsubscribe_trades(handle_.get(), symbol.c_str()); }

bool FpssClient::is_authenticated() const { return tdx_fpss_is_authenticated(handle_.get()) != 0; }

std::optional<std::string> FpssClient::contract_lookup(int id) const {
    detail::FfiString result(tdx_fpss_contract_lookup(handle_.get(), id));
    if (!result.ok()) return std::nullopt;
    return result.str();
}

std::string FpssClient::active_subscriptions() const {
    detail::FfiString result(tdx_fpss_active_subscriptions(handle_.get()));
    return result.ok() ? result.str() : "[]";
}

std::string FpssClient::next_event(uint64_t timeout_ms) {
    detail::FfiString result(tdx_fpss_next_event(handle_.get(), timeout_ms));
    return result.ok() ? result.str() : "";
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

// Minimal JSON parser for the Greeks JSON object
namespace detail {

static double json_double(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\":";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return 0.0;
    pos += needle.size();
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
    return std::stod(json.substr(pos));
}

} // namespace detail

Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, bool is_call) {
    detail::FfiString result(tdx_all_greeks(spot, strike, rate, div_yield, tte, option_price, is_call ? 1 : 0));
    if (!result.ok()) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    auto json = result.str();
    return Greeks{
        detail::json_double(json, "value"),
        detail::json_double(json, "delta"),
        detail::json_double(json, "gamma"),
        detail::json_double(json, "theta"),
        detail::json_double(json, "vega"),
        detail::json_double(json, "rho"),
        detail::json_double(json, "iv"),
        detail::json_double(json, "iv_error"),
        detail::json_double(json, "vanna"),
        detail::json_double(json, "charm"),
        detail::json_double(json, "vomma"),
        detail::json_double(json, "veta"),
        detail::json_double(json, "speed"),
        detail::json_double(json, "zomma"),
        detail::json_double(json, "color"),
        detail::json_double(json, "ultima"),
        detail::json_double(json, "d1"),
        detail::json_double(json, "d2"),
        detail::json_double(json, "dual_delta"),
        detail::json_double(json, "dual_gamma"),
        detail::json_double(json, "epsilon"),
        detail::json_double(json, "lambda"),
    };
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
