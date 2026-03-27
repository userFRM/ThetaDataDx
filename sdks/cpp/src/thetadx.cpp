/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return parsed C++ types (vectors of structs) from JSON.
 *
 * This file is a single-header-ish implementation. Include "thetadatadx.hpp" (below)
 * or compile this .cpp with the include/ header for the C types.
 */

#include "thetadatadx.hpp"

#include <stdexcept>
#include <sstream>
#include <unordered_map>

namespace tdx {

// ── JSON parsing helpers ──
// Minimal JSON parsing using string manipulation (no external dependency).
// For production use, consider nlohmann/json or simdjson.

namespace detail {

static std::string last_ffi_error() {
    const char* err = tdx_last_error();
    return err ? std::string(err) : "unknown error";
}

// Managed C string from FFI: auto-frees on destruction.
struct FfiString {
    char* ptr;
    FfiString(char* p) : ptr(p) {}
    ~FfiString() { if (ptr) tdx_string_free(ptr); }
    FfiString(const FfiString&) = delete;
    FfiString& operator=(const FfiString&) = delete;

    std::string str() const { return ptr ? std::string(ptr) : ""; }
    bool ok() const { return ptr != nullptr; }
};

// Simple JSON value extraction (numbers and strings from objects).
// This is intentionally minimal. For a real project, use a JSON library.

static double json_double(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\":";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return 0.0;
    pos += needle.size();
    // Skip whitespace
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
    return std::stod(json.substr(pos));
}

static int json_int(const std::string& json, const std::string& key) {
    std::string needle = "\"" + key + "\":";
    auto pos = json.find(needle);
    if (pos == std::string::npos) return 0;
    pos += needle.size();
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
    return std::stoi(json.substr(pos));
}

// Split a JSON array of objects into individual object strings.
static std::vector<std::string> split_json_array(const std::string& json) {
    std::vector<std::string> result;
    int depth = 0;
    size_t start = 0;
    bool in_string = false;
    bool escaped = false;

    for (size_t i = 0; i < json.size(); ++i) {
        char c = json[i];
        if (escaped) { escaped = false; continue; }
        if (c == '\\') { escaped = true; continue; }
        if (c == '"') { in_string = !in_string; continue; }
        if (in_string) continue;

        if (c == '{') {
            if (depth == 0) start = i;
            ++depth;
        } else if (c == '}') {
            --depth;
            if (depth == 0) {
                result.push_back(json.substr(start, i - start + 1));
            }
        }
    }
    return result;
}

// Parse a JSON array of strings: ["a","b","c"]
static std::vector<std::string> parse_string_array(const std::string& json) {
    std::vector<std::string> result;
    bool in_string = false;
    bool escaped = false;
    std::string current;

    for (size_t i = 0; i < json.size(); ++i) {
        char c = json[i];
        if (escaped) { current += c; escaped = false; continue; }
        if (c == '\\') { escaped = true; continue; }
        if (c == '"') {
            if (in_string) {
                result.push_back(current);
                current.clear();
            }
            in_string = !in_string;
            continue;
        }
        if (in_string) current += c;
    }
    return result;
}

static EodTick parse_eod_tick(const std::string& obj) {
    return EodTick{
        json_int(obj, "ms_of_day"),
        json_double(obj, "open"),
        json_double(obj, "high"),
        json_double(obj, "low"),
        json_double(obj, "close"),
        json_int(obj, "volume"),
        json_int(obj, "count"),
        json_double(obj, "bid"),
        json_double(obj, "ask"),
        json_int(obj, "date"),
    };
}

static OhlcTick parse_ohlc_tick(const std::string& obj) {
    return OhlcTick{
        json_int(obj, "ms_of_day"),
        json_double(obj, "open"),
        json_double(obj, "high"),
        json_double(obj, "low"),
        json_double(obj, "close"),
        json_int(obj, "volume"),
        json_int(obj, "count"),
        json_int(obj, "date"),
    };
}

static TradeTick parse_trade_tick(const std::string& obj) {
    return TradeTick{
        json_int(obj, "ms_of_day"),
        json_int(obj, "sequence"),
        json_int(obj, "condition"),
        json_int(obj, "size"),
        json_int(obj, "exchange"),
        json_double(obj, "price"),
        json_int(obj, "price_raw"),
        json_int(obj, "price_type"),
        json_int(obj, "condition_flags"),
        json_int(obj, "price_flags"),
        json_int(obj, "volume_type"),
        json_int(obj, "records_back"),
        json_int(obj, "date"),
    };
}

static QuoteTick parse_quote_tick(const std::string& obj) {
    return QuoteTick{
        json_int(obj, "ms_of_day"),
        json_int(obj, "bid_size"),
        json_int(obj, "bid_exchange"),
        json_double(obj, "bid"),
        json_int(obj, "bid_condition"),
        json_int(obj, "ask_size"),
        json_int(obj, "ask_exchange"),
        json_double(obj, "ask"),
        json_int(obj, "ask_condition"),
        json_int(obj, "date"),
    };
}

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

// Helper: call FFI, check result, return string
static std::string ffi_call_string(char* ptr) {
    FfiString result(ptr);
    if (!result.ok()) throw std::runtime_error("thetadatadx: " + last_ffi_error());
    return result.str();
}

// Helper: parse typed tick arrays
template<typename T>
using TickParser = T(*)(const std::string&);

template<typename T>
static std::vector<T> parse_tick_array(const std::string& json, TickParser<T> parser) {
    auto objects = split_json_array(json);
    std::vector<T> ticks;
    ticks.reserve(objects.size());
    for (auto& obj : objects) ticks.push_back(parser(obj));
    return ticks;
}

// ── DataTable parsing ──
// The FFI returns DataTable endpoints as: {"headers": [...], "rows": [[...], ...]}
// Each cell is either: a number (int64), a string, null, a price {"value":N,"type":T},
// or a timestamp {"epoch_ms":N,"zone":Z}.
// We build a header-to-column-index map, then extract values by name.

// A cell value from a DataTable row. May be a number, a price object, a string, or null.
struct DtCell {
    double num;          // numeric value (number or decoded price)
    std::string text;    // string value (if applicable)
    bool is_null;
};

// Decode a price object {"value":N,"type":T} into a double.
// ThetaData price encoding: value * 10^(type-10) for type>0; 0 for type=0.
static double decode_price(double value, int price_type) {
    if (price_type == 0) return 0.0;
    // type=10 => multiply by 1.0 (integer)
    // type<10 => multiply by 10^(type-10), i.e. divide
    // type>10 => multiply by 10^(type-10)
    double divisor = 1.0;
    int exp = 10 - price_type;
    if (exp > 0) {
        for (int i = 0; i < exp; ++i) divisor *= 10.0;
        return value / divisor;
    } else if (exp < 0) {
        for (int i = 0; i < -exp; ++i) divisor *= 10.0;
        return value * divisor;
    }
    return value;
}

// Extract a single JSON value starting at position `pos`. Advances `pos` past the value.
// Returns the DtCell representation.
static DtCell parse_dt_cell(const std::string& json, size_t& pos) {
    DtCell cell{0.0, "", false};
    // Skip whitespace
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t' || json[pos] == '\n' || json[pos] == '\r'))
        ++pos;
    if (pos >= json.size()) { cell.is_null = true; return cell; }

    char c = json[pos];
    if (c == 'n') {
        // null
        cell.is_null = true;
        pos += 4; // skip "null"
    } else if (c == '"') {
        // string
        ++pos;
        while (pos < json.size() && json[pos] != '"') {
            if (json[pos] == '\\') { cell.text += json[pos + 1]; pos += 2; }
            else { cell.text += json[pos]; ++pos; }
        }
        if (pos < json.size()) ++pos; // skip closing quote
    } else if (c == '{') {
        // object: either {"value":N,"type":T} (price) or {"epoch_ms":N,"zone":Z} (timestamp)
        std::string obj;
        int depth = 0;
        size_t obj_start = pos;
        for (; pos < json.size(); ++pos) {
            if (json[pos] == '{') ++depth;
            else if (json[pos] == '}') { --depth; if (depth == 0) { ++pos; break; } }
        }
        obj = json.substr(obj_start, pos - obj_start);
        // Check if it's a price object
        if (obj.find("\"value\"") != std::string::npos && obj.find("\"type\"") != std::string::npos) {
            double value = json_double(obj, "value");
            int ptype = json_int(obj, "type");
            cell.num = decode_price(value, ptype);
        } else if (obj.find("\"epoch_ms\"") != std::string::npos) {
            cell.num = json_double(obj, "epoch_ms");
        }
    } else {
        // number (int or float)
        size_t end = pos;
        while (end < json.size() && json[end] != ',' && json[end] != ']' && json[end] != '}' &&
               json[end] != ' ' && json[end] != '\n' && json[end] != '\r')
            ++end;
        std::string numstr = json.substr(pos, end - pos);
        pos = end;
        try { cell.num = std::stod(numstr); } catch (...) { cell.is_null = true; }
    }
    return cell;
}

// A parsed DataTable row: vector of cells.
using DtRow = std::vector<DtCell>;

// Parsed DataTable: headers + rows.
struct DataTable {
    std::vector<std::string> headers;
    std::vector<DtRow> rows;
    std::unordered_map<std::string, size_t> col_index;

    // Build the column index map from headers.
    void build_index() {
        for (size_t i = 0; i < headers.size(); ++i)
            col_index[headers[i]] = i;
    }

    // Get a cell by column name from a row. Returns null cell if not found.
    const DtCell& cell(const DtRow& row, const std::string& name) const {
        static const DtCell null_cell{0.0, "", true};
        auto it = col_index.find(name);
        if (it == col_index.end() || it->second >= row.size()) return null_cell;
        return row[it->second];
    }

    double num(const DtRow& row, const std::string& name) const {
        return cell(row, name).num;
    }

    int inum(const DtRow& row, const std::string& name) const {
        return static_cast<int>(cell(row, name).num);
    }

    const std::string& text(const DtRow& row, const std::string& name) const {
        static const std::string empty;
        auto it = col_index.find(name);
        if (it == col_index.end() || it->second >= row.size()) return empty;
        return row[it->second].text;
    }
};

// Parse a DataTable JSON string into a DataTable struct.
static DataTable parse_data_table(const std::string& json) {
    DataTable dt;

    // Find "headers" array
    auto hdr_pos = json.find("\"headers\"");
    if (hdr_pos == std::string::npos) return dt;
    hdr_pos = json.find('[', hdr_pos);
    if (hdr_pos == std::string::npos) return dt;
    auto hdr_end = json.find(']', hdr_pos);
    if (hdr_end == std::string::npos) return dt;
    dt.headers = parse_string_array(json.substr(hdr_pos, hdr_end - hdr_pos + 1));
    dt.build_index();

    // Find "rows" array
    auto rows_pos = json.find("\"rows\"");
    if (rows_pos == std::string::npos) return dt;
    rows_pos = json.find('[', rows_pos);
    if (rows_pos == std::string::npos) return dt;
    ++rows_pos; // skip outer '['

    // Parse each row: [...], [...], ...
    while (rows_pos < json.size()) {
        // Skip whitespace and commas
        while (rows_pos < json.size() && (json[rows_pos] == ' ' || json[rows_pos] == ',' ||
               json[rows_pos] == '\n' || json[rows_pos] == '\r' || json[rows_pos] == '\t'))
            ++rows_pos;
        if (rows_pos >= json.size() || json[rows_pos] == ']') break;
        if (json[rows_pos] != '[') break;
        ++rows_pos; // skip '['

        DtRow row;
        while (rows_pos < json.size() && json[rows_pos] != ']') {
            // Skip whitespace and commas
            while (rows_pos < json.size() && (json[rows_pos] == ' ' || json[rows_pos] == ',' ||
                   json[rows_pos] == '\n' || json[rows_pos] == '\r' || json[rows_pos] == '\t'))
                ++rows_pos;
            if (rows_pos >= json.size() || json[rows_pos] == ']') break;
            row.push_back(parse_dt_cell(json, rows_pos));
        }
        if (rows_pos < json.size()) ++rows_pos; // skip ']'
        dt.rows.push_back(std::move(row));
    }
    return dt;
}

// ── DataTable-based tick parsers ──

template<typename T>
using DtTickParser = T(*)(const DataTable&, const DtRow&);

template<typename T>
static std::vector<T> parse_dt_tick_array(const std::string& json, DtTickParser<T> parser) {
    auto dt = parse_data_table(json);
    std::vector<T> ticks;
    ticks.reserve(dt.rows.size());
    for (auto& row : dt.rows) ticks.push_back(parser(dt, row));
    return ticks;
}

static TradeQuoteTick parse_trade_quote_tick(const DataTable& dt, const DtRow& row) {
    return TradeQuoteTick{
        dt.inum(row, "ms_of_day"),
        dt.inum(row, "sequence"),
        dt.inum(row, "ext_condition1"),
        dt.inum(row, "ext_condition2"),
        dt.inum(row, "ext_condition3"),
        dt.inum(row, "ext_condition4"),
        dt.inum(row, "condition"),
        dt.inum(row, "size"),
        dt.inum(row, "exchange"),
        dt.num(row, "price"),
        dt.inum(row, "condition_flags"),
        dt.inum(row, "price_flags"),
        dt.inum(row, "volume_type"),
        dt.inum(row, "records_back"),
        dt.inum(row, "quote_ms_of_day"),
        dt.inum(row, "bid_size"),
        dt.inum(row, "bid_exchange"),
        dt.num(row, "bid"),
        dt.inum(row, "bid_condition"),
        dt.inum(row, "ask_size"),
        dt.inum(row, "ask_exchange"),
        dt.num(row, "ask"),
        dt.inum(row, "ask_condition"),
        dt.inum(row, "date"),
    };
}

static OpenInterestTick parse_open_interest_tick(const DataTable& dt, const DtRow& row) {
    return OpenInterestTick{
        dt.inum(row, "ms_of_day"),
        dt.inum(row, "open_interest"),
        dt.inum(row, "date"),
    };
}

static GreeksTick parse_greeks_tick(const DataTable& dt, const DtRow& row) {
    return GreeksTick{
        dt.inum(row, "ms_of_day"),
        dt.num(row, "value"),
        dt.num(row, "delta"),
        dt.num(row, "gamma"),
        dt.num(row, "theta"),
        dt.num(row, "vega"),
        dt.num(row, "rho"),
        dt.num(row, "implied_volatility"),
        dt.num(row, "iv_error"),
        dt.num(row, "vanna"),
        dt.num(row, "charm"),
        dt.num(row, "vomma"),
        dt.num(row, "veta"),
        dt.num(row, "speed"),
        dt.num(row, "zomma"),
        dt.num(row, "color"),
        dt.num(row, "ultima"),
        dt.num(row, "d1"),
        dt.num(row, "d2"),
        dt.num(row, "dual_delta"),
        dt.num(row, "dual_gamma"),
        dt.num(row, "epsilon"),
        dt.num(row, "lambda"),
        dt.inum(row, "date"),
    };
}

static IvTick parse_iv_tick(const DataTable& dt, const DtRow& row) {
    return IvTick{
        dt.inum(row, "ms_of_day"),
        dt.num(row, "implied_volatility"),
        dt.num(row, "iv_error"),
        dt.inum(row, "date"),
    };
}

static PriceTick parse_price_tick(const DataTable& dt, const DtRow& row) {
    return PriceTick{
        dt.inum(row, "ms_of_day"),
        dt.num(row, "price"),
        dt.inum(row, "date"),
    };
}

static MarketValueTick parse_market_value_tick(const DataTable& dt, const DtRow& row) {
    return MarketValueTick{
        dt.inum(row, "ms_of_day"),
        dt.num(row, "value"),
        dt.inum(row, "date"),
    };
}

static OptionContract parse_option_contract(const DataTable& dt, const DtRow& row) {
    return OptionContract{
        dt.text(row, "root"),
        dt.inum(row, "expiration"),
        dt.inum(row, "strike"),
        dt.text(row, "right"),
    };
}

static CalendarDay parse_calendar_day(const DataTable& dt, const DtRow& row) {
    return CalendarDay{
        dt.inum(row, "date"),
        dt.inum(row, "is_open"),
        dt.inum(row, "open_time"),
        dt.inum(row, "close_time"),
    };
}

static InterestRateTick parse_interest_rate_tick(const DataTable& dt, const DtRow& row) {
    return InterestRateTick{
        dt.num(row, "rate"),
        dt.inum(row, "date"),
    };
}

} // namespace detail

// ── Credentials ──

Credentials Credentials::from_file(const std::string& path) {
    TdxCredentials* h = tdx_credentials_from_file(path.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}

Credentials Credentials::from_email(const std::string& email, const std::string& password) {
    TdxCredentials* h = tdx_credentials_new(email.c_str(), password.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}

// ── Config ──

Config Config::production() {
    return Config(tdx_config_production());
}

Config Config::dev() {
    return Config(tdx_config_dev());
}

// ── Client ──

Client Client::connect(const Credentials& creds, const Config& config) {
    TdxClient* h = tdx_client_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Client(h);
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

std::vector<std::string> Client::stock_list_symbols() const {
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_stock_list_symbols(handle_.get())));
}

std::vector<std::string> Client::stock_list_dates(
    const std::string& request_type, const std::string& symbol) const
{
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_stock_list_dates(
            handle_.get(), request_type.c_str(), symbol.c_str())));
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — Snapshot endpoints (4)
// ═══════════════════════════════════════════════════════════════════════

std::vector<OhlcTick> Client::stock_snapshot_ohlc(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_stock_snapshot_ohlc(handle_.get(), json_arg.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<TradeTick> Client::stock_snapshot_trade(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_stock_snapshot_trade(handle_.get(), json_arg.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::stock_snapshot_quote(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_stock_snapshot_quote(handle_.get(), json_arg.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

std::vector<MarketValueTick> Client::stock_snapshot_market_value(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_stock_snapshot_market_value(handle_.get(), json_arg.c_str()));
    return detail::parse_dt_tick_array<MarketValueTick>(json, detail::parse_market_value_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — History endpoints (5 + bonus)
// ═══════════════════════════════════════════════════════════════════════

std::vector<EodTick> Client::stock_history_eod(
    const std::string& symbol, const std::string& start_date, const std::string& end_date) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_eod(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
    return detail::parse_tick_array<EodTick>(json, detail::parse_eod_tick);
}

std::vector<OhlcTick> Client::stock_history_ohlc(
    const std::string& symbol, const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_ohlc(
        handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<OhlcTick> Client::stock_history_ohlc_range(
    const std::string& symbol, const std::string& start_date,
    const std::string& end_date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_ohlc_range(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), interval.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<TradeTick> Client::stock_history_trade(
    const std::string& symbol, const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_trade(
        handle_.get(), symbol.c_str(), date.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::stock_history_quote(
    const std::string& symbol, const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_quote(
        handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

std::vector<TradeQuoteTick> Client::stock_history_trade_quote(
    const std::string& symbol, const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_stock_history_trade_quote(
        handle_.get(), symbol.c_str(), date.c_str()));
    return detail::parse_dt_tick_array<TradeQuoteTick>(json, detail::parse_trade_quote_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

std::vector<TradeTick> Client::stock_at_time_trade(
    const std::string& symbol, const std::string& start_date,
    const std::string& end_date, const std::string& time_of_day) const
{
    auto json = detail::ffi_call_string(tdx_stock_at_time_trade(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::stock_at_time_quote(
    const std::string& symbol, const std::string& start_date,
    const std::string& end_date, const std::string& time_of_day) const
{
    auto json = detail::ffi_call_string(tdx_stock_at_time_quote(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — List endpoints (5)
// ═══════════════════════════════════════════════════════════════════════

std::vector<std::string> Client::option_list_symbols() const {
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_option_list_symbols(handle_.get())));
}

std::vector<std::string> Client::option_list_dates(
    const std::string& request_type, const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_option_list_dates(
            handle_.get(), request_type.c_str(), symbol.c_str(),
            expiration.c_str(), strike.c_str(), right.c_str())));
}

std::vector<std::string> Client::option_list_expirations(const std::string& symbol) const {
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_option_list_expirations(handle_.get(), symbol.c_str())));
}

std::vector<std::string> Client::option_list_strikes(
    const std::string& symbol, const std::string& expiration) const
{
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_option_list_strikes(
            handle_.get(), symbol.c_str(), expiration.c_str())));
}

std::vector<OptionContract> Client::option_list_contracts(
    const std::string& request_type, const std::string& symbol, const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_list_contracts(
        handle_.get(), request_type.c_str(), symbol.c_str(), date.c_str()));
    return detail::parse_dt_tick_array<OptionContract>(json, detail::parse_option_contract);
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — Snapshot endpoints (10)
// ═══════════════════════════════════════════════════════════════════════

std::vector<OhlcTick> Client::option_snapshot_ohlc(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_ohlc(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<TradeTick> Client::option_snapshot_trade(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_trade(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::option_snapshot_quote(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_quote(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

std::vector<OpenInterestTick> Client::option_snapshot_open_interest(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_open_interest(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<OpenInterestTick>(json, detail::parse_open_interest_tick);
}

std::vector<MarketValueTick> Client::option_snapshot_market_value(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_market_value(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<MarketValueTick>(json, detail::parse_market_value_tick);
}

std::vector<IvTick> Client::option_snapshot_greeks_implied_volatility(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_greeks_implied_volatility(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<IvTick>(json, detail::parse_iv_tick);
}

std::vector<GreeksTick> Client::option_snapshot_greeks_all(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_greeks_all(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_snapshot_greeks_first_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_greeks_first_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_snapshot_greeks_second_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_greeks_second_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_snapshot_greeks_third_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right) const
{
    auto json = detail::ffi_call_string(tdx_option_snapshot_greeks_third_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History endpoints (6)
// ═══════════════════════════════════════════════════════════════════════

std::vector<EodTick> Client::option_history_eod(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& start_date, const std::string& end_date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_eod(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        start_date.c_str(), end_date.c_str()));
    return detail::parse_tick_array<EodTick>(json, detail::parse_eod_tick);
}

std::vector<OhlcTick> Client::option_history_ohlc(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_ohlc(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<TradeTick> Client::option_history_trade(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::option_history_quote(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_quote(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

std::vector<TradeQuoteTick> Client::option_history_trade_quote(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_quote(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<TradeQuoteTick>(json, detail::parse_trade_quote_tick);
}

std::vector<OpenInterestTick> Client::option_history_open_interest(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_open_interest(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<OpenInterestTick>(json, detail::parse_open_interest_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

std::vector<GreeksTick> Client::option_history_greeks_eod(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& start_date, const std::string& end_date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_eod(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        start_date.c_str(), end_date.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_greeks_all(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_all(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_trade_greeks_all(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_greeks_all(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_greeks_first_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_first_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_trade_greeks_first_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_greeks_first_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_greeks_second_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_second_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_trade_greeks_second_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_greeks_second_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_greeks_third_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_third_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<GreeksTick> Client::option_history_trade_greeks_third_order(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_greeks_third_order(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<GreeksTick>(json, detail::parse_greeks_tick);
}

std::vector<IvTick> Client::option_history_greeks_implied_volatility(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_option_history_greeks_implied_volatility(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<IvTick>(json, detail::parse_iv_tick);
}

std::vector<IvTick> Client::option_history_trade_greeks_implied_volatility(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& date) const
{
    auto json = detail::ffi_call_string(tdx_option_history_trade_greeks_implied_volatility(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        date.c_str()));
    return detail::parse_dt_tick_array<IvTick>(json, detail::parse_iv_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

std::vector<TradeTick> Client::option_at_time_trade(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& start_date, const std::string& end_date,
    const std::string& time_of_day) const
{
    auto json = detail::ffi_call_string(tdx_option_at_time_trade(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
    return detail::parse_tick_array<TradeTick>(json, detail::parse_trade_tick);
}

std::vector<QuoteTick> Client::option_at_time_quote(
    const std::string& symbol, const std::string& expiration,
    const std::string& strike, const std::string& right,
    const std::string& start_date, const std::string& end_date,
    const std::string& time_of_day) const
{
    auto json = detail::ffi_call_string(tdx_option_at_time_quote(
        handle_.get(), symbol.c_str(), expiration.c_str(), strike.c_str(), right.c_str(),
        start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
    return detail::parse_tick_array<QuoteTick>(json, detail::parse_quote_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

std::vector<std::string> Client::index_list_symbols() const {
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_index_list_symbols(handle_.get())));
}

std::vector<std::string> Client::index_list_dates(const std::string& symbol) const {
    return detail::parse_string_array(
        detail::ffi_call_string(tdx_index_list_dates(handle_.get(), symbol.c_str())));
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — Snapshot endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

std::vector<OhlcTick> Client::index_snapshot_ohlc(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_index_snapshot_ohlc(handle_.get(), json_arg.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<PriceTick> Client::index_snapshot_price(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_index_snapshot_price(handle_.get(), json_arg.c_str()));
    return detail::parse_dt_tick_array<PriceTick>(json, detail::parse_price_tick);
}

std::vector<MarketValueTick> Client::index_snapshot_market_value(const std::vector<std::string>& symbols) const {
    std::string json_arg = detail::build_json_array(symbols);
    auto json = detail::ffi_call_string(tdx_index_snapshot_market_value(handle_.get(), json_arg.c_str()));
    return detail::parse_dt_tick_array<MarketValueTick>(json, detail::parse_market_value_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — History endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

std::vector<EodTick> Client::index_history_eod(
    const std::string& symbol, const std::string& start_date, const std::string& end_date) const
{
    auto json = detail::ffi_call_string(tdx_index_history_eod(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
    return detail::parse_tick_array<EodTick>(json, detail::parse_eod_tick);
}

std::vector<OhlcTick> Client::index_history_ohlc(
    const std::string& symbol, const std::string& start_date,
    const std::string& end_date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_index_history_ohlc(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), interval.c_str()));
    return detail::parse_tick_array<OhlcTick>(json, detail::parse_ohlc_tick);
}

std::vector<PriceTick> Client::index_history_price(
    const std::string& symbol, const std::string& date, const std::string& interval) const
{
    auto json = detail::ffi_call_string(tdx_index_history_price(
        handle_.get(), symbol.c_str(), date.c_str(), interval.c_str()));
    return detail::parse_dt_tick_array<PriceTick>(json, detail::parse_price_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

std::vector<PriceTick> Client::index_at_time_price(
    const std::string& symbol, const std::string& start_date,
    const std::string& end_date, const std::string& time_of_day) const
{
    auto json = detail::ffi_call_string(tdx_index_at_time_price(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str(), time_of_day.c_str()));
    return detail::parse_dt_tick_array<PriceTick>(json, detail::parse_price_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

std::vector<CalendarDay> Client::calendar_open_today() const {
    auto json = detail::ffi_call_string(tdx_calendar_open_today(handle_.get()));
    return detail::parse_dt_tick_array<CalendarDay>(json, detail::parse_calendar_day);
}

std::vector<CalendarDay> Client::calendar_on_date(const std::string& date) const {
    auto json = detail::ffi_call_string(tdx_calendar_on_date(handle_.get(), date.c_str()));
    return detail::parse_dt_tick_array<CalendarDay>(json, detail::parse_calendar_day);
}

std::vector<CalendarDay> Client::calendar_year(const std::string& year) const {
    auto json = detail::ffi_call_string(tdx_calendar_year(handle_.get(), year.c_str()));
    return detail::parse_dt_tick_array<CalendarDay>(json, detail::parse_calendar_day);
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

std::vector<InterestRateTick> Client::interest_rate_history_eod(
    const std::string& symbol, const std::string& start_date, const std::string& end_date) const
{
    auto json = detail::ffi_call_string(tdx_interest_rate_history_eod(
        handle_.get(), symbol.c_str(), start_date.c_str(), end_date.c_str()));
    return detail::parse_dt_tick_array<InterestRateTick>(json, detail::parse_interest_rate_tick);
}

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone)
// ═══════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Real-time streaming client
// ═══════════════════════════════════════════════════════════════════════

FpssClient::FpssClient(const Credentials& creds, const Config& config) {
    TdxFpssHandle* h = tdx_fpss_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    handle_.reset(h);
}

int FpssClient::subscribe_quotes(const std::string& symbol) {
    int rc = tdx_fpss_subscribe_quotes(handle_.get(), symbol.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

int FpssClient::subscribe_trades(const std::string& symbol) {
    int rc = tdx_fpss_subscribe_trades(handle_.get(), symbol.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

int FpssClient::subscribe_open_interest(const std::string& symbol) {
    int rc = tdx_fpss_subscribe_open_interest(handle_.get(), symbol.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

int FpssClient::subscribe_full_trades(const std::string& sec_type) {
    int rc = tdx_fpss_subscribe_full_trades(handle_.get(), sec_type.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

int FpssClient::unsubscribe_open_interest(const std::string& symbol) {
    int rc = tdx_fpss_unsubscribe_open_interest(handle_.get(), symbol.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

int FpssClient::unsubscribe_trades(const std::string& symbol) {
    int rc = tdx_fpss_unsubscribe_trades(handle_.get(), symbol.c_str());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return rc;
}

bool FpssClient::is_authenticated() const {
    return tdx_fpss_is_authenticated(handle_.get()) != 0;
}

std::optional<std::string> FpssClient::contract_lookup(int id) const {
    char* ptr = tdx_fpss_contract_lookup(handle_.get(), id);
    if (!ptr) return std::nullopt;
    std::string result(ptr);
    tdx_string_free(ptr);
    return result;
}

std::string FpssClient::active_subscriptions() const {
    char* ptr = tdx_fpss_active_subscriptions(handle_.get());
    if (!ptr) return "[]";
    std::string result(ptr);
    tdx_string_free(ptr);
    return result;
}

std::string FpssClient::next_event(uint64_t timeout_ms) {
    detail::FfiString result(tdx_fpss_next_event(handle_.get(), timeout_ms));
    if (!result.ok()) return "";  // Timeout — not an error.
    return result.str();
}

void FpssClient::shutdown() {
    if (handle_) {
        tdx_fpss_shutdown(handle_.get());
    }
}

FpssClient::~FpssClient() {
    if (handle_) {
        tdx_fpss_shutdown(handle_.get());
    }
    // handle_ unique_ptr destructor calls tdx_fpss_free via FpssHandleDeleter
}

// ═══════════════════════════════════════════════════════════════════════
//  Standalone Greeks
// ═══════════════════════════════════════════════════════════════════════

Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, bool is_call)
{
    detail::FfiString result(tdx_all_greeks(
        spot, strike, rate, div_yield, tte, option_price, is_call ? 1 : 0));
    if (!result.ok()) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());

    auto s = result.str();
    return Greeks{
        detail::json_double(s, "value"),
        detail::json_double(s, "delta"),
        detail::json_double(s, "gamma"),
        detail::json_double(s, "theta"),
        detail::json_double(s, "vega"),
        detail::json_double(s, "rho"),
        detail::json_double(s, "iv"),
        detail::json_double(s, "iv_error"),
        detail::json_double(s, "vanna"),
        detail::json_double(s, "charm"),
        detail::json_double(s, "vomma"),
        detail::json_double(s, "veta"),
        detail::json_double(s, "speed"),
        detail::json_double(s, "zomma"),
        detail::json_double(s, "color"),
        detail::json_double(s, "ultima"),
        detail::json_double(s, "d1"),
        detail::json_double(s, "d2"),
        detail::json_double(s, "dual_delta"),
        detail::json_double(s, "dual_gamma"),
        detail::json_double(s, "epsilon"),
        detail::json_double(s, "lambda"),
    };
}

std::pair<double, double> implied_volatility(
    double spot, double strike, double rate, double div_yield,
    double tte, double option_price, bool is_call)
{
    double iv = 0.0, err = 0.0;
    int rc = tdx_implied_volatility(
        spot, strike, rate, div_yield, tte, option_price,
        is_call ? 1 : 0, &iv, &err);
    if (rc != 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return {iv, err};
}

} // namespace tdx
