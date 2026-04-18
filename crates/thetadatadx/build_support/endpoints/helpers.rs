//! Cross-cutting helpers shared by every renderer.
//!
//! These are the pure mapping, naming, and classification utilities: param
//! filters (`method_params` / `builder_params` / `is_*`), casing conversion,
//! per-language type tables, arg declaration formatters, arg literal
//! formatters, builder-binding helpers, and CLI/validator name derivations.
//!
//! Anything that emits a multi-line chunk of target-language code belongs in
//! `render/`, not here.

use std::collections::HashSet;

use super::model::{GeneratedEndpoint, GeneratedParam};

// ───────────────────────── Param classification ────────────────────────────

pub(super) fn method_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect()
}

pub(super) fn builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect()
}

pub(super) fn collect_builder_params(endpoints: &[GeneratedEndpoint]) -> Vec<GeneratedParam> {
    let mut seen = HashSet::new();
    let mut params = Vec::new();
    for endpoint in endpoints {
        for param in builder_params(endpoint) {
            if seen.insert(param.name.clone()) {
                params.push(param.clone());
            }
        }
    }
    params
}

pub(super) fn is_simple_list_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "list"
}

pub(super) fn is_streaming_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "stream"
}

pub(super) fn is_method_call_param(param: &GeneratedParam) -> bool {
    param.binding == "method"
}

pub(super) fn is_symbols_param(param: &GeneratedParam) -> bool {
    param.param_type == "Symbols"
}

pub(super) fn call_arg_name(param: &GeneratedParam) -> String {
    if is_symbols_param(param) {
        "&symbol_refs".into()
    } else {
        param.name.clone()
    }
}

// ───────────────────────── Runtime dispatch getters ─────────────────────────

pub(super) fn required_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Symbol" => "required_symbol",
        "Symbols" => "required_symbols",
        "Date" => "required_date",
        "Expiration" => "required_expiration",
        "Strike" => "required_strike",
        "Interval" => "required_interval",
        "Right" => "required_right",
        "Int" => "required_int32",
        "Float" => "required_float64",
        "Bool" => "required_bool",
        "Year" => "required_year",
        _ => "required_str",
    }
}

pub(super) fn optional_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Date" => "optional_date",
        "Expiration" => "optional_expiration",
        "Strike" => "optional_strike",
        "Int" => "optional_int32",
        "Float" => "optional_float64",
        "Bool" => "optional_bool",
        _ => "optional_str",
    }
}

// ───────────────────────── Casing ────────────────────────────────────────────

pub(super) fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<String>()
}

pub(super) fn go_segment_pascal(segment: &str) -> String {
    match segment {
        "eod" => "EOD".into(),
        "ohlc" => "OHLC".into(),
        "iv" => "IV".into(),
        "dte" => "DTE".into(),
        "nbbo" => "NBBO".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

pub(super) fn to_go_exported_name(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(go_segment_pascal)
        .collect::<String>()
}

pub(super) fn to_camel_case(value: &str) -> String {
    let pascal = to_go_exported_name(value);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ───────────────────────── Direct (Rust) client type maps ──────────────────

pub(super) fn direct_method_arg_name(
    endpoint: &GeneratedEndpoint,
    param: &GeneratedParam,
) -> String {
    let _ = endpoint;
    param.arg_name.clone().unwrap_or_else(|| param.name.clone())
}

pub(super) fn direct_date_arg_name(
    endpoint: &GeneratedEndpoint,
    param: &GeneratedParam,
) -> Option<String> {
    match param.name.as_str() {
        "date" | "start_date" | "end_date" => Some(direct_method_arg_name(endpoint, param)),
        _ => None,
    }
}

pub(super) fn direct_required_kind(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "str_vec"
    } else {
        "str"
    }
}

pub(super) fn direct_optional_kind_and_default(param: &GeneratedParam) -> (&'static str, String) {
    if let Some(default) = param.default.as_deref() {
        return match param.param_type.as_str() {
            "Str" => ("string", format!("{default:?}.to_string()")),
            "Int" => {
                let value = default.parse::<i32>().unwrap_or_else(|_| {
                    panic!(
                        "invalid int default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_i32", format!("Some({value})"))
            }
            "Float" => {
                let value = default.parse::<f64>().unwrap_or_else(|_| {
                    panic!(
                        "invalid float default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_f64", format!("Some({value:?})"))
            }
            "Bool" => {
                let value = default.parse::<bool>().unwrap_or_else(|_| {
                    panic!(
                        "invalid bool default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_bool", format!("Some({value})"))
            }
            other => panic!(
                "unsupported default for parameter '{}' with type '{}'",
                param.name, other
            ),
        };
    }
    match param.param_type.as_str() {
        "Int" => ("opt_i32", "None".into()),
        "Float" => ("opt_f64", "None".into()),
        "Bool" => ("opt_bool", "None".into()),
        _ => ("opt_str", "None".into()),
    }
}

pub(super) fn direct_optional_rust_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "Option<i32>",
        "opt_f64" => "Option<f64>",
        "opt_bool" => "Option<bool>",
        "string" => "String",
        _ => "Option<String>",
    }
}

pub(super) fn direct_optional_setter_arg_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "i32",
        "opt_f64" => "f64",
        "opt_bool" => "bool",
        "string" => "&str",
        _ => "&str",
    }
}

pub(super) fn direct_optional_setter_assign_expr(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" | "opt_f64" | "opt_bool" => "Some(v)",
        "string" => "v.to_string()",
        _ => "Some(v.to_string())",
    }
}

pub(super) fn direct_required_field_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "Vec<String>"
    } else {
        "String"
    }
}

pub(super) fn direct_required_param_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "&[&str]"
    } else {
        "&str"
    }
}

pub(super) fn direct_required_store_expr(
    endpoint: &GeneratedEndpoint,
    param: &GeneratedParam,
) -> String {
    let arg_name = direct_method_arg_name(endpoint, param);
    if param.param_type == "Symbols" {
        format!("{arg_name}.iter().map(|s| s.to_string()).collect()")
    } else {
        format!("{arg_name}.to_string()")
    }
}

/// Map a collection return type (e.g. `TradeTicks`) to the per-chunk tick type
/// (e.g. `TradeTick`) used by generated direct streaming builders.
pub(super) fn direct_stream_tick_type(return_type: &str) -> &'static str {
    match return_type {
        "TradeTicks" => "TradeTick",
        "QuoteTicks" => "QuoteTick",
        other => panic!("unsupported streaming tick type: {other}"),
    }
}

pub(super) fn direct_return_type(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "Vec<EodTick>",
        "OhlcTicks" => "Vec<OhlcTick>",
        "TradeTicks" => "Vec<TradeTick>",
        "QuoteTicks" => "Vec<QuoteTick>",
        "TradeQuoteTicks" => "Vec<TradeQuoteTick>",
        "OpenInterestTicks" => "Vec<OpenInterestTick>",
        "MarketValueTicks" => "Vec<MarketValueTick>",
        "GreeksTicks" => "Vec<GreeksTick>",
        "IvTicks" => "Vec<IvTick>",
        "PriceTicks" => "Vec<PriceTick>",
        "CalendarDays" => "Vec<CalendarDay>",
        "InterestRateTicks" => "Vec<InterestRateTick>",
        "OptionContracts" => "Vec<OptionContract>",
        other => panic!("unsupported direct return type: {other}"),
    }
}

pub(super) fn direct_parser_name(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "decode::parse_eod_ticks",
        "OhlcTicks" => "decode::parse_ohlc_ticks",
        "TradeTicks" => "decode::parse_trade_ticks",
        "QuoteTicks" => "decode::parse_quote_ticks",
        "TradeQuoteTicks" => "decode::parse_trade_quote_ticks",
        "OpenInterestTicks" => "decode::parse_open_interest_ticks",
        "MarketValueTicks" => "decode::parse_market_value_ticks",
        "GreeksTicks" => "decode::parse_greeks_ticks",
        "IvTicks" => "decode::parse_iv_ticks",
        "PriceTicks" => "decode::parse_price_ticks",
        "CalendarDays" => "decode::parse_calendar_days_v3",
        "InterestRateTicks" => "decode::parse_interest_rate_ticks",
        "OptionContracts" => "decode::parse_option_contracts_v3",
        other => panic!("unsupported parser return type: {other}"),
    }
}

// ───────────────────────── Per-language type tables ─────────────────────────

pub(super) fn python_optional_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "Option<i32>",
        "Float" => "Option<f64>",
        "Bool" => "Option<bool>",
        _ => "Option<&str>",
    }
}

pub(super) fn go_result_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "[]string",
        "EodTicks" => "[]EodTick",
        "OhlcTicks" => "[]OhlcTick",
        "TradeTicks" => "[]TradeTick",
        "QuoteTicks" => "[]QuoteTick",
        "TradeQuoteTicks" => "[]TradeQuoteTick",
        "OpenInterestTicks" => "[]OpenInterestTick",
        "MarketValueTicks" => "[]MarketValueTick",
        "GreeksTicks" => "[]GreeksTick",
        "IvTicks" => "[]IVTick",
        "PriceTicks" => "[]PriceTick",
        "CalendarDays" => "[]CalendarDay",
        "InterestRateTicks" => "[]InterestRateTick",
        "OptionContracts" => "[]OptionContract",
        other => panic!("unsupported Go result type: {other}"),
    }
}

pub(super) fn go_converter_name(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "convertEodTicks",
        "OhlcTicks" => "convertOhlcTicks",
        "TradeTicks" => "convertTradeTicks",
        "QuoteTicks" => "convertQuoteTicks",
        "TradeQuoteTicks" => "convertTradeQuoteTicks",
        "OpenInterestTicks" => "convertOpenInterestTicks",
        "MarketValueTicks" => "convertMarketValueTicks",
        "GreeksTicks" => "convertGreeksTicks",
        "IvTicks" => "convertIvTicks",
        "PriceTicks" => "convertPriceTicks",
        "CalendarDays" => "convertCalendarDays",
        "InterestRateTicks" => "convertInterestRateTicks",
        "OptionContracts" => "convertOptionContracts",
        other => panic!("unsupported Go converter type: {other}"),
    }
}

pub(super) fn ffi_array_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "TdxStringArray",
        "EodTicks" => "TdxEodTickArray",
        "OhlcTicks" => "TdxOhlcTickArray",
        "TradeTicks" => "TdxTradeTickArray",
        "QuoteTicks" => "TdxQuoteTickArray",
        "TradeQuoteTicks" => "TdxTradeQuoteTickArray",
        "OpenInterestTicks" => "TdxOpenInterestTickArray",
        "MarketValueTicks" => "TdxMarketValueTickArray",
        "GreeksTicks" => "TdxGreeksTickArray",
        "IvTicks" => "TdxIvTickArray",
        "PriceTicks" => "TdxPriceTickArray",
        "CalendarDays" => "TdxCalendarDayArray",
        "InterestRateTicks" => "TdxInterestRateTickArray",
        "OptionContracts" => "TdxOptionContractArray",
        other => panic!("unsupported FFI array type: {other}"),
    }
}

pub(super) fn ffi_array_empty_expr(return_type: &str) -> &'static str {
    match return_type {
        "OptionContracts" => {
            "TdxOptionContractArray {\n        data: ptr::null(),\n        len: 0,\n    }"
        }
        _ => "ARRAY_EMPTY",
    }
}

pub(super) fn ffi_output_variant(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "StringList",
        "EodTicks" => "EodTicks",
        "OhlcTicks" => "OhlcTicks",
        "TradeTicks" => "TradeTicks",
        "QuoteTicks" => "QuoteTicks",
        "TradeQuoteTicks" => "TradeQuoteTicks",
        "OpenInterestTicks" => "OpenInterestTicks",
        "MarketValueTicks" => "MarketValueTicks",
        "GreeksTicks" => "GreeksTicks",
        "IvTicks" => "IvTicks",
        "PriceTicks" => "PriceTicks",
        "CalendarDays" => "CalendarDays",
        "InterestRateTicks" => "InterestRateTicks",
        "OptionContracts" => "OptionContracts",
        other => panic!("unsupported endpoint output variant: {other}"),
    }
}

/// Returns the `#[repr(C)]` array type name for the given `EndpointOutput`
/// variant (e.g. `TdxEodTickArray`). The emitter wraps `<type>::from_vec(...)`
/// — which returns `Result<Self, NulError>` — in an inline match that routes
/// interior-NUL failures through the FFI error slot.
pub(super) fn ffi_from_vec_array_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "TdxStringArray",
        "OptionContracts" => "TdxOptionContractArray",
        "EodTicks" => "TdxEodTickArray",
        "OhlcTicks" => "TdxOhlcTickArray",
        "TradeTicks" => "TdxTradeTickArray",
        "QuoteTicks" => "TdxQuoteTickArray",
        "TradeQuoteTicks" => "TdxTradeQuoteTickArray",
        "OpenInterestTicks" => "TdxOpenInterestTickArray",
        "MarketValueTicks" => "TdxMarketValueTickArray",
        "GreeksTicks" => "TdxGreeksTickArray",
        "IvTicks" => "TdxIvTickArray",
        "PriceTicks" => "TdxPriceTickArray",
        "CalendarDays" => "TdxCalendarDayArray",
        "InterestRateTicks" => "TdxInterestRateTickArray",
        other => panic!("unsupported FFI from_vec return type: {other}"),
    }
}

pub(super) fn ffi_header_return_type(return_type: &str) -> &'static str {
    match return_type {
        "OptionContracts" => "TdxOptionContractArray",
        "StringList" => "TdxStringArray",
        "EodTicks" | "OhlcTicks" | "TradeTicks" | "QuoteTicks" | "TradeQuoteTicks"
        | "OpenInterestTicks" | "MarketValueTicks" | "GreeksTicks" | "IvTicks" | "PriceTicks"
        | "CalendarDays" | "InterestRateTicks" => "TdxTickArray",
        other => panic!("unsupported Go/C header return type: {other}"),
    }
}

pub(super) fn ffi_free_fn(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "C.tdx_eod_tick_array_free",
        "OhlcTicks" => "C.tdx_ohlc_tick_array_free",
        "TradeTicks" => "C.tdx_trade_tick_array_free",
        "QuoteTicks" => "C.tdx_quote_tick_array_free",
        "TradeQuoteTicks" => "C.tdx_trade_quote_tick_array_free",
        "OpenInterestTicks" => "C.tdx_open_interest_tick_array_free",
        "MarketValueTicks" => "C.tdx_market_value_tick_array_free",
        "GreeksTicks" => "C.tdx_greeks_tick_array_free",
        "IvTicks" => "C.tdx_iv_tick_array_free",
        "PriceTicks" => "C.tdx_price_tick_array_free",
        "CalendarDays" => "C.tdx_calendar_day_array_free",
        "InterestRateTicks" => "C.tdx_interest_rate_tick_array_free",
        "OptionContracts" => "C.tdx_option_contract_array_free",
        other => panic!("unsupported FFI free fn for Go: {other}"),
    }
}

pub(super) fn cpp_value_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "std::string",
        "EodTicks" => "EodTick",
        "OhlcTicks" => "OhlcTick",
        "TradeTicks" => "TradeTick",
        "QuoteTicks" => "QuoteTick",
        "TradeQuoteTicks" => "TradeQuoteTick",
        "OpenInterestTicks" => "OpenInterestTick",
        "MarketValueTicks" => "MarketValueTick",
        "GreeksTicks" => "GreeksTick",
        "IvTicks" => "IvTick",
        "PriceTicks" => "PriceTick",
        "CalendarDays" => "CalendarDay",
        "InterestRateTicks" => "InterestRateTick",
        "OptionContracts" => "OptionContract",
        other => panic!("unsupported C++ value type: {other}"),
    }
}

pub(super) fn cpp_converter_expr(return_type: &str) -> String {
    match return_type {
        "StringList" => "return detail::check_string_array(arr);".into(),
        "OptionContracts" => "return detail::option_contract_array_to_vector(arr);".into(),
        other => {
            // Check `tdx_last_error_raw` before converting: success-empty
            // and failure (e.g. timeout) both return `{nullptr, 0}` arrays,
            // so we have to consult the error slot directly. The generated
            // Client method `tdx_clear_error()`s before the FFI call so a
            // stale error from a prior call isn't misattributed.
            let free_fn = ffi_free_fn(other).trim_start_matches("C.").to_string();
            format!(
                "{{\n        const std::string err = detail::last_ffi_error_raw();\n        if (!err.empty()) {{\n            {free_fn}(arr);\n            throw std::runtime_error(\"thetadatadx: \" + err);\n        }}\n    }}\n    auto result = detail::to_vector(arr.data, arr.len);\n    {free_fn}(arr);\n    return result;"
            )
        }
    }
}

pub(super) fn python_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_tick_to_dict",
        "OhlcTicks" => "ohlc_tick_to_dict",
        "TradeTicks" => "trade_tick_to_dict",
        "QuoteTicks" => "quote_tick_to_dict",
        "TradeQuoteTicks" => "trade_quote_tick_to_dict",
        "OpenInterestTicks" => "open_interest_tick_to_dict",
        "MarketValueTicks" => "market_value_tick_to_dict",
        "GreeksTicks" => "greeks_tick_to_dict",
        "IvTicks" => "iv_tick_to_dict",
        "PriceTicks" => "price_tick_to_dict",
        "CalendarDays" => "calendar_day_to_dict",
        "InterestRateTicks" => "interest_rate_tick_to_dict",
        "OptionContracts" => "option_contract_to_dict",
        other => panic!("unsupported Python converter: {other}"),
    }
}

pub(super) fn python_columnar_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_ticks_to_columnar",
        "OhlcTicks" => "ohlc_ticks_to_columnar",
        "TradeTicks" => "trade_ticks_to_columnar",
        "QuoteTicks" => "quote_ticks_to_columnar",
        "TradeQuoteTicks" => "trade_quote_ticks_to_columnar",
        "OpenInterestTicks" => "open_interest_ticks_to_columnar",
        "MarketValueTicks" => "market_value_ticks_to_columnar",
        "GreeksTicks" => "greeks_ticks_to_columnar",
        "IvTicks" => "iv_ticks_to_columnar",
        "PriceTicks" => "price_ticks_to_columnar",
        "CalendarDays" => "calendar_days_to_columnar",
        "InterestRateTicks" => "interest_rate_ticks_to_columnar",
        "OptionContracts" => "option_contracts_to_columnar",
        other => panic!("unsupported Python columnar converter: {other}"),
    }
}

/// Name of the generated `*_to_pyclass_list` converter for a given tick
/// return type. This is the PRIMARY return path for Python historical
/// endpoints — typed `#[pyclass]` objects matching Rust/TS/Go/C++ SDKs.
/// See `build_support/ticks.rs::render_python_tick_classes`.
pub(super) fn python_pyclass_list_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_ticks_to_pyclass_list",
        "OhlcTicks" => "ohlc_ticks_to_pyclass_list",
        "TradeTicks" => "trade_ticks_to_pyclass_list",
        "QuoteTicks" => "quote_ticks_to_pyclass_list",
        "TradeQuoteTicks" => "trade_quote_ticks_to_pyclass_list",
        "OpenInterestTicks" => "open_interest_ticks_to_pyclass_list",
        "MarketValueTicks" => "market_value_ticks_to_pyclass_list",
        "GreeksTicks" => "greeks_ticks_to_pyclass_list",
        "IvTicks" => "iv_ticks_to_pyclass_list",
        "PriceTicks" => "price_ticks_to_pyclass_list",
        "CalendarDays" => "calendar_days_to_pyclass_list",
        "InterestRateTicks" => "interest_rate_ticks_to_pyclass_list",
        "OptionContracts" => "option_contracts_to_pyclass_list",
        other => panic!("unsupported Python pyclass-list converter: {other}"),
    }
}

/// Map a collection return type (e.g. `TradeTicks`) to the generated
/// `#[napi(object)]` struct name emitted in `tick_classes.rs`. The TS SDK
/// binds each Rust tick struct (from `tdbe::types::tick`) to this flat
/// napi-object variant so `Vec<T>` surfaces as `T[]` in `index.d.ts`.
pub(super) fn ts_class_name(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "EodTick",
        "OhlcTicks" => "OhlcTick",
        "TradeTicks" => "TradeTick",
        "QuoteTicks" => "QuoteTick",
        "TradeQuoteTicks" => "TradeQuoteTick",
        "OpenInterestTicks" => "OpenInterestTick",
        "MarketValueTicks" => "MarketValueTick",
        "GreeksTicks" => "GreeksTick",
        "IvTicks" => "IvTick",
        "PriceTicks" => "PriceTick",
        "CalendarDays" => "CalendarDay",
        "InterestRateTicks" => "InterestRateTick",
        "OptionContracts" => "OptionContract",
        other => panic!("unsupported TypeScript class name: {other}"),
    }
}

/// Map a collection return type to the generated
/// `{tick}_to_class_vec` factory name. Complements `ts_class_name`.
pub(super) fn ts_class_vec_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_ticks_to_class_vec",
        "OhlcTicks" => "ohlc_ticks_to_class_vec",
        "TradeTicks" => "trade_ticks_to_class_vec",
        "QuoteTicks" => "quote_ticks_to_class_vec",
        "TradeQuoteTicks" => "trade_quote_ticks_to_class_vec",
        "OpenInterestTicks" => "open_interest_ticks_to_class_vec",
        "MarketValueTicks" => "market_value_ticks_to_class_vec",
        "GreeksTicks" => "greeks_ticks_to_class_vec",
        "IvTicks" => "iv_ticks_to_class_vec",
        "PriceTicks" => "price_ticks_to_class_vec",
        "CalendarDays" => "calendar_days_to_class_vec",
        "InterestRateTicks" => "interest_rate_ticks_to_class_vec",
        "OptionContracts" => "option_contracts_to_class_vec",
        other => panic!("unsupported TypeScript class-vec converter: {other}"),
    }
}

pub(super) fn ts_columnar_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_ticks_to_columnar",
        "OhlcTicks" => "ohlc_ticks_to_columnar",
        "TradeTicks" => "trade_ticks_to_columnar",
        "QuoteTicks" => "quote_ticks_to_columnar",
        "TradeQuoteTicks" => "trade_quote_ticks_to_columnar",
        "OpenInterestTicks" => "open_interest_ticks_to_columnar",
        "MarketValueTicks" => "market_value_ticks_to_columnar",
        "GreeksTicks" => "greeks_ticks_to_columnar",
        "IvTicks" => "iv_ticks_to_columnar",
        "PriceTicks" => "price_ticks_to_columnar",
        "CalendarDays" => "calendar_days_to_columnar",
        "InterestRateTicks" => "interest_rate_ticks_to_columnar",
        "OptionContracts" => "option_contracts_to_columnar",
        other => panic!("unsupported TypeScript columnar converter: {other}"),
    }
}

// ───────────────────────── Builder / FFI option tables ─────────────────────

pub(super) fn builder_value_type_name(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Float" => "double",
        "Bool" => "bool",
        _ => "std::string",
    }
}

pub(super) fn builder_copy_expr(param: &GeneratedParam, source: &str) -> String {
    match param.param_type.as_str() {
        "Int" => format!("{} = {}", param.name, source),
        "Float" => format!("{} = {}", param.name, source),
        "Bool" => format!("{} = {}", param.name, source),
        _ => format!("{} = std::move({})", param.name, source),
    }
}

pub(super) fn ffi_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" | "Bool" => "i32",
        "Float" => "f64",
        _ => "*const c_char",
    }
}

pub(super) fn c_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Bool" => "int32_t",
        "Float" => "double",
        _ => "const char*",
    }
}

pub(super) fn ffi_option_insert_expr(param: &GeneratedParam) -> String {
    match param.param_type.as_str() {
        "Int" => format!(
            "        insert_int_arg(args, {:?}, options.{});",
            param.name, param.name
        ),
        "Float" => {
            format!(
                "        insert_float_arg(args, {:?}, options.{});",
                param.name, param.name
            )
        }
        "Bool" => format!(
            "        insert_bool_arg(args, {:?}, options.{})?;",
            param.name, param.name
        ),
        _ => format!(
            "    insert_optional_str_arg(args, {:?}, options.{})?;",
            param.name, param.name
        ),
    }
}

pub(super) fn ffi_option_has_flag(param: &GeneratedParam) -> bool {
    matches!(param.param_type.as_str(), "Int" | "Float" | "Bool")
}

// ───────────────────────── SDK method arg declarations ─────────────────────

pub(super) fn sdk_method_arg_name(param: &GeneratedParam) -> String {
    if param.param_type == "Symbols" {
        "symbols".into()
    } else {
        param.name.clone()
    }
}

pub(super) fn go_method_arg_decl(param: &GeneratedParam) -> String {
    let name = to_camel_case(&sdk_method_arg_name(param));
    if param.param_type == "Symbols" {
        format!("{name} []string")
    } else {
        format!("{name} string")
    }
}

pub(super) fn python_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("{name}: Vec<String>")
    } else {
        format!("{name}: &str")
    }
}

pub(super) fn cpp_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("const std::vector<std::string>& {name}")
    } else {
        format!("const std::string& {name}")
    }
}

pub(super) fn go_c_var_name(param: &GeneratedParam) -> String {
    format!("c{}", to_go_exported_name(&sdk_method_arg_name(param)))
}

// ───────────────────────── Validator arg literals ──────────────────────────

/// Render a single arg string as a Python literal expression, taking the
/// param's wire type into account so `Symbols` becomes a list.
pub(super) fn python_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("[\"{value}\"]"),
        _ => format!("\"{value}\""),
    }
}

/// Render a single arg string as a Go literal expression.
pub(super) fn go_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("[]string{{\"{value}\"}}"),
        _ => format!("\"{value}\""),
    }
}

/// Render a single arg string as a C++ literal expression.
pub(super) fn cpp_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("std::vector<std::string>{{\"{value}\"}}"),
        _ => format!("\"{value}\""),
    }
}

/// Look up a builder-bound `GeneratedParam` on the endpoint by name.
fn builder_param_for<'a>(
    endpoint: &'a GeneratedEndpoint,
    name: &str,
) -> Option<&'a GeneratedParam> {
    endpoint
        .params
        .iter()
        .find(|p| p.name == name && !is_method_call_param(p))
}

/// Render a Python kwarg value (`key=value`) for a builder-bound param,
/// preserving the param's wire type (Bool → `True`/`False`, Int/Float → bare,
/// Str → quoted).
pub(super) fn python_builder_kwarg(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let literal = match param.param_type.as_str() {
        "Bool" => match value {
            "true" => "True".to_string(),
            "false" => "False".to_string(),
            other => panic!("python_builder_kwarg: bool override {other:?} must be true/false"),
        },
        "Int" | "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!("{name}={literal}"))
}

/// Render a Go `WithXxx(value)` option for a builder-bound param. The
/// generated validate.go lives in the same `package thetadatadx` as the
/// `WithXxx` ctors, so no package qualifier is needed.
pub(super) fn go_builder_option(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let with_name = go_with_name_from_param(name);
    let literal = match param.param_type.as_str() {
        "Bool" => value.to_string(),
        "Int" => format!("int32({value})"),
        "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!("{with_name}({literal})"))
}

/// Convert a snake_case param name to the Go `WithXxx` exported ctor, keeping
/// the `DTE`/`NBBO` acronym casing used in the existing hand-rolled options.
pub(super) fn go_with_name_from_param(name: &str) -> String {
    let exported = name.split('_').map(go_segment_pascal).collect::<String>();
    format!("With{exported}")
}

/// Render a C++ `.with_<name>(value)` chained setter for a builder-bound param.
pub(super) fn cpp_builder_setter(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let literal = match param.param_type.as_str() {
        "Bool" => value.to_string(),
        "Int" => value.to_string(),
        "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!(".with_{name}({literal})"))
}

// ───────────────────────── CLI / validator scaffolding ─────────────────────

pub(super) fn cli_command_name(endpoint: &GeneratedEndpoint) -> String {
    match endpoint.category.as_str() {
        "stock" | "option" | "index" | "calendar" => endpoint
            .name
            .strip_prefix(&format!("{}_", endpoint.category))
            .expect("endpoint name should match category prefix")
            .into(),
        "rate" => endpoint
            .name
            .strip_prefix("interest_rate_")
            .expect("rate endpoint should use interest_rate_ prefix")
            .into(),
        other => panic!("unsupported CLI endpoint category: {other}"),
    }
}

pub(super) fn cli_command_tokens_for_mode(
    endpoint: &GeneratedEndpoint,
    mode: &super::modes::TestMode,
) -> Vec<String> {
    let mut tokens = vec![
        match endpoint.category.as_str() {
            "rate" => "rate".into(),
            other => other.into(),
        },
        cli_command_name(endpoint),
    ];
    tokens.extend(mode.args.iter().cloned());
    tokens
}
