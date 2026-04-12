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
//  Historical endpoints (generated)
// ═══════════════════════════════════════════════════════════════

#include "generated_historical.cpp.inc"

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
