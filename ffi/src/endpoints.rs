//! Options-aware historical endpoint wrappers emitted by the generator.
//!
//! The generated `endpoint_request_options.rs` declares `TdxEndpointRequestOptions`
//! and the private helper `apply_endpoint_request_options`. The generated
//! `endpoint_with_options.rs` declares the 61 `tdx_<endpoint>_with_options`
//! entry points. Both are `include!`'d here so the helper is in scope when
//! the endpoint wrappers expand.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{set_error, set_error_from};
use crate::runtime;
use crate::types::{
    insert_bool_arg, insert_float_arg, insert_int_arg, insert_optional_str_arg,
    TdxCalendarDayArray, TdxClient, TdxEodTickArray, TdxGreeksAllTickArray, TdxGreeksEodTickArray,
    TdxGreeksFirstOrderTickArray, TdxGreeksSecondOrderTickArray, TdxGreeksThirdOrderTickArray,
    TdxIndexPriceAtTimeTickArray, TdxInterestRateTickArray, TdxIvTickArray,
    TdxMarketValueTickArray, TdxOhlcTickArray, TdxOpenInterestTickArray, TdxOptionContractArray,
    TdxPriceTickArray, TdxQuoteTickArray, TdxStringArray, TdxTradeGreeksAllTickArray,
    TdxTradeGreeksFirstOrderTickArray, TdxTradeGreeksImpliedVolatilityTickArray,
    TdxTradeGreeksSecondOrderTickArray, TdxTradeGreeksThirdOrderTickArray, TdxTradeQuoteTickArray,
    TdxTradeTickArray,
};

include!("endpoint_request_options.rs");
include!("endpoint_with_options.rs");
