// REST-routing policy + `optionHistory*WithFallback` shims.
//
// Hand-written -- not part of the SDK-surface generator. Mirrors the
// Python `FallbackPolicy` pyclass + `Config.with_rest_fallback` +
// `option_history_*_with_fallback` methods one-for-one.

// ── FallbackPolicy factories ─────────────────────────────────────────

FallbackPolicy FallbackPolicy::disabled() {
    auto h = tdx_fallback_policy_disabled();
    if (!h) detail::throw_last_ffi_error();
    return FallbackPolicy(h);
}

FallbackPolicy FallbackPolicy::restAlways(const std::string& base_url) {
    auto h = tdx_fallback_policy_rest_always(base_url.c_str());
    if (!h) detail::throw_last_ffi_error();
    return FallbackPolicy(h);
}

// ── Config integration ───────────────────────────────────────────────

void Config::withRestFallback(const FallbackPolicy& policy) {
    tdx_clear_error();
    int rc = tdx_config_with_rest_fallback(handle_.get(), policy.get());
    if (rc != 0) {
        detail::throw_last_ffi_error();
    }
}

// ── Client _with_fallback shims ──────────────────────────────────────
//
// Empty-string sentinels for the optional `Option<&str>` args -- pass
// `c_str()` if the caller-supplied string is non-empty, NULL otherwise.
// Matches the Python and TS conventions: empty string is treated as
// "use the upstream default" (i.e. omit the argument).

namespace {
inline const char* optional_cstr(const std::string& s) {
    return s.empty() ? nullptr : s.c_str();
}
} // namespace

std::vector<QuoteTick> Client::optionHistoryQuoteWithFallback(
    const std::string& symbol, const std::string& expiration,
    const std::string& start_date, const std::string& end_date,
    const std::string& strike, const std::string& right,
    const std::string& interval) const {
    tdx_clear_error();
    auto arr = tdx_option_history_quote_with_fallback(
        handle_.get(), symbol.c_str(), expiration.c_str(), start_date.c_str(),
        optional_cstr(end_date), optional_cstr(strike), optional_cstr(right),
        optional_cstr(interval));
    {
        const std::string err = detail::last_ffi_error_raw();
        if (!err.empty()) {
            const int32_t code = tdx_last_error_code();
            tdx_quote_tick_array_free(arr);
            detail::throw_for_code(code, err);
        }
    }
    auto result = detail::to_vector(arr.data, arr.len);
    tdx_quote_tick_array_free(arr);
    return result;
}

std::vector<TradeQuoteTick> Client::optionHistoryTradeQuoteWithFallback(
    const std::string& symbol, const std::string& expiration,
    const std::string& start_date, const std::string& end_date,
    const std::string& strike, const std::string& right) const {
    tdx_clear_error();
    auto arr = tdx_option_history_trade_quote_with_fallback(
        handle_.get(), symbol.c_str(), expiration.c_str(), start_date.c_str(),
        optional_cstr(end_date), optional_cstr(strike), optional_cstr(right));
    {
        const std::string err = detail::last_ffi_error_raw();
        if (!err.empty()) {
            const int32_t code = tdx_last_error_code();
            tdx_trade_quote_tick_array_free(arr);
            detail::throw_for_code(code, err);
        }
    }
    auto result = detail::to_vector(arr.data, arr.len);
    tdx_trade_quote_tick_array_free(arr);
    return result;
}

std::vector<IvTick> Client::optionHistoryGreeksImpliedVolatilityWithFallback(
    const std::string& symbol, const std::string& expiration,
    const std::string& start_date, const std::string& end_date,
    const std::string& strike, const std::string& right,
    const std::string& interval) const {
    tdx_clear_error();
    auto arr = tdx_option_history_greeks_implied_volatility_with_fallback(
        handle_.get(), symbol.c_str(), expiration.c_str(), start_date.c_str(),
        optional_cstr(end_date), optional_cstr(strike), optional_cstr(right),
        optional_cstr(interval));
    {
        const std::string err = detail::last_ffi_error_raw();
        if (!err.empty()) {
            const int32_t code = tdx_last_error_code();
            tdx_iv_tick_array_free(arr);
            detail::throw_for_code(code, err);
        }
    }
    auto result = detail::to_vector(arr.data, arr.len);
    tdx_iv_tick_array_free(arr);
    return result;
}

std::vector<GreeksFirstOrderTick> Client::optionHistoryGreeksFirstOrderWithFallback(
    const std::string& symbol, const std::string& expiration,
    const std::string& start_date, const std::string& end_date,
    const std::string& strike, const std::string& right,
    const std::string& interval) const {
    tdx_clear_error();
    auto arr = tdx_option_history_greeks_first_order_with_fallback(
        handle_.get(), symbol.c_str(), expiration.c_str(), start_date.c_str(),
        optional_cstr(end_date), optional_cstr(strike), optional_cstr(right),
        optional_cstr(interval));
    {
        const std::string err = detail::last_ffi_error_raw();
        if (!err.empty()) {
            const int32_t code = tdx_last_error_code();
            tdx_greeks_first_order_tick_array_free(arr);
            detail::throw_for_code(code, err);
        }
    }
    auto result = detail::to_vector(arr.data, arr.len);
    tdx_greeks_first_order_tick_array_free(arr);
    return result;
}
