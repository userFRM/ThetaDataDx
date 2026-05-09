//! Cross-language utility helpers — TypeScript / napi-rs bindings (issue #424).
//!
//! Wraps `tdbe::{conditions, exchange, sequences}` lookup tables and
//! exposes them under a `Util` JS namespace:
//!
//! ```ts
//! import { Util } from 'thetadatadx';
//! Util.conditionName(0);          // "REGULAR"
//! Util.exchangeName(3);           // "NewYorkStockExchange"
//! Util.exchangeSymbol(3);         // "NYSE"
//! Util.sequenceSignedToUnsigned(BigInt(-1));
//! ```
//!
//! Hand-written; the function set is finite. The TypeScript side gets
//! camelCase names via `napi(js_name = ...)` to match Node convention,
//! while the underlying Rust functions stay snake_case for parity with
//! Python.

use napi::bindgen_prelude::BigInt;

#[napi(js_name = "Util")]
pub struct Util;

#[napi]
impl Util {
    #[napi(js_name = "conditionName")]
    pub fn condition_name(code: i32) -> String {
        tdbe::conditions::condition_name(code).to_string()
    }

    #[napi(js_name = "conditionDescription")]
    pub fn condition_description(code: i32) -> String {
        tdbe::conditions::condition_description(code).to_string()
    }

    #[napi(js_name = "isCancel")]
    pub fn is_cancel(code: i32) -> bool {
        tdbe::conditions::is_cancel(code)
    }

    #[napi(js_name = "updatesVolume")]
    pub fn updates_volume(code: i32) -> bool {
        tdbe::conditions::updates_volume(code)
    }

    #[napi(js_name = "quoteConditionName")]
    pub fn quote_condition_name(code: i32) -> String {
        tdbe::conditions::quote_condition_name(code).to_string()
    }

    #[napi(js_name = "quoteConditionDescription")]
    pub fn quote_condition_description(code: i32) -> String {
        tdbe::conditions::quote_condition_description(code).to_string()
    }

    #[napi(js_name = "isFirm")]
    pub fn is_firm(code: i32) -> bool {
        tdbe::conditions::is_firm(code)
    }

    #[napi(js_name = "isHalted")]
    pub fn is_halted(code: i32) -> bool {
        tdbe::conditions::is_halted(code)
    }

    #[napi(js_name = "exchangeName")]
    pub fn exchange_name(code: i32) -> String {
        tdbe::exchange::exchange_name(code).to_string()
    }

    #[napi(js_name = "exchangeSymbol")]
    pub fn exchange_symbol(code: i32) -> String {
        tdbe::exchange::exchange_symbol(code).to_string()
    }

    /// Convert a signed wire-encoded trade-sequence value to its unsigned
    /// monotonic form. Mirrors `tdbe::sequences::signed_to_unsigned`.
    /// Accepts a JS BigInt in the **i32 wire range**
    /// (`-2_147_483_648 ..= 2_147_483_647`) — the upstream Java
    /// terminal encodes trade sequences as i32; the SDK widens to
    /// i64 internally, but the meaningful round-trip is the i32
    /// range. Returns a JS BigInt because the unsigned monotonic
    /// sequence id can exceed `Number.MAX_SAFE_INTEGER`. Inputs
    /// outside the i32 wire range throw so silent coercion cannot
    /// produce a look-correct-but-wrong sequence id downstream.
    #[napi(js_name = "sequenceSignedToUnsigned")]
    pub fn sequence_signed_to_unsigned(signed_value: BigInt) -> napi::Result<BigInt> {
        let signed: i64 = bigint_to_i32(&signed_value).map(i64::from).ok_or_else(|| {
            napi::Error::from_reason(
                "sequenceSignedToUnsigned: BigInt outside the i32 wire range \
                 (-2_147_483_648 ..= 2_147_483_647)",
            )
        })?;
        Ok(BigInt::from(tdbe::sequences::signed_to_unsigned(signed)))
    }

    /// Convert an unsigned monotonic trade-sequence value back to its
    /// signed wire encoding. Mirrors `tdbe::sequences::unsigned_to_signed`.
    /// Accepts a JS BigInt in the unsigned wire range
    /// (`0 ..= SEQUENCE_RANGE - 1`, i.e. `0 ..= 2^32 - 1`); returns a
    /// JS BigInt for symmetry with `sequenceSignedToUnsigned`.
    /// Negative inputs and inputs above the wire range throw — the
    /// unsigned monotonic sequence id is always non-negative and
    /// never wider than the i32 wire range.
    #[napi(js_name = "sequenceUnsignedToSigned")]
    pub fn sequence_unsigned_to_signed(unsigned_value: BigInt) -> napi::Result<BigInt> {
        if unsigned_value.sign_bit && !unsigned_value.words.iter().all(|w| *w == 0) {
            return Err(napi::Error::from_reason(
                "sequenceUnsignedToSigned: negative BigInt rejected; the unsigned \
                 monotonic sequence id is always non-negative",
            ));
        }
        if unsigned_value.words.len() > 1 {
            return Err(napi::Error::from_reason(
                "sequenceUnsignedToSigned: BigInt above the wire range \
                 (0 ..= 2^32 - 1)",
            ));
        }
        let value = unsigned_value.words.first().copied().unwrap_or(0);
        if value > u32::MAX as u64 {
            return Err(napi::Error::from_reason(
                "sequenceUnsignedToSigned: BigInt above the wire range \
                 (0 ..= 2^32 - 1)",
            ));
        }
        Ok(BigInt::from(tdbe::sequences::unsigned_to_signed(value)))
    }
}

/// Decode a napi `BigInt` into the i32 wire range, accepting the
/// asymmetric `i32::MIN` boundary. Returns `None` for any value
/// outside `[i32::MIN, i32::MAX]`.
fn bigint_to_i32(value: &BigInt) -> Option<i32> {
    if value.words.len() > 1 {
        return None;
    }
    let magnitude = value.words.first().copied().unwrap_or(0);
    if value.sign_bit {
        if magnitude == 0 {
            Some(0)
        } else if magnitude <= i32::MAX as u64 {
            // SAFETY: `magnitude` fits in i32 here.
            Some(-(magnitude as i32))
        } else if magnitude == (i32::MAX as u64) + 1 {
            Some(i32::MIN)
        } else {
            None
        }
    } else {
        i32::try_from(magnitude).ok()
    }
}
