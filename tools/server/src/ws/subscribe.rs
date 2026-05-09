//! Subscribe / unsubscribe request handling from WebSocket clients.

use axum::extract::ws::WebSocket;
use sonic_rs::prelude::*;

use tdbe::time::is_valid_yyyymmdd;
use tdbe::types::enums::SecType;
use thetadatadx::fpss::protocol::Contract;

use crate::state::AppState;
use crate::validation;

use super::session::send_response;

/// Max accepted payload size for a single client `Message::Text` frame.
///
/// A legitimate subscribe / stop envelope is <200 bytes; 4 KiB leaves
/// comfortable headroom for long ticker lists or extra fields while
/// rejecting a multi-megabyte JSON bomb before `sonic_rs::from_str`
/// touches the bytes.
pub(super) const WS_MAX_TEXT_BYTES: usize = 4 * 1024;

/// Inclusive lower bound on option expiration dates (YYYYMMDD). Any
/// earlier value is an attacker probing the contract keyspace.
pub(super) const MIN_OPTION_EXP: i32 = 19000101;

/// Inclusive upper bound on option expiration dates (YYYYMMDD). ThetaData
/// supports LEAPS out a few decades; 2100 is a hard ceiling that the
/// underlying MDDS would reject anyway.
pub(super) const MAX_OPTION_EXP: i32 = 21000101;

/// Return `true` iff `exp` is within the accepted YYYYMMDD range for an
/// option subscription. Cheap integer check — no allocations.
pub(super) fn is_valid_yyyymmdd_range(exp: i32) -> bool {
    (MIN_OPTION_EXP..=MAX_OPTION_EXP).contains(&exp)
}

/// Parse and handle a client subscription command.
pub(super) async fn handle_client_message(state: &AppState, text: &str, socket: &mut WebSocket) {
    // Reject oversize Text frames BEFORE handing bytes to the JSON parser.
    // Without this, a 2 GB subscribe envelope would stream into
    // `sonic_rs::from_str` and could OOM the process long before
    // validation kicked in. 4 KiB is strictly above any legitimate
    // subscribe envelope emitted by the Python SDK / terminal clients
    // (observed ~120-200 bytes).
    if text.len() > WS_MAX_TEXT_BYTES {
        tracing::warn!(
            bytes = text.len(),
            limit = WS_MAX_TEXT_BYTES,
            "WS client Text frame exceeds cap; rejecting"
        );
        let err_msg = format!(
            "text frame exceeds maximum of {WS_MAX_TEXT_BYTES} bytes (got {})",
            text.len()
        );
        let resp = sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": "ERROR",
                "req_id": 0,
                "error": err_msg.as_str(),
            }
        });
        send_response(socket, &resp, "oversize_text_reply").await;
        return;
    }

    let obj: sonic_rs::Value = match sonic_rs::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("invalid WebSocket JSON: {}", text);
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": 0
                }
            });
            send_response(socket, &resp, "invalid_json_reply").await;
            return;
        }
    };

    let null_val = sonic_rs::Value::default();
    let msg_type_val = obj.get("msg_type").unwrap_or(&null_val);
    let msg_type = msg_type_val.as_str().unwrap_or("").to_uppercase();

    let id_val = obj.get("id").unwrap_or(&null_val);
    let req_id = id_val.as_i64().unwrap_or(0);

    if msg_type == "STOP" {
        tracing::info!("WebSocket client requested STOP");
        let resp = sonic_rs::json!({
            "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
        });
        send_response(socket, &resp, "stop_reply").await;
        return;
    }

    let add_val = obj.get("add").unwrap_or(&null_val);
    let is_add = add_val.as_bool().unwrap_or(true);

    let sec_type_val = obj.get("sec_type").unwrap_or(&null_val);
    let sec_type = sec_type_val.as_str().unwrap_or("").to_uppercase();

    let req_type_val = obj.get("req_type").unwrap_or(&null_val);
    let req_type = req_type_val.as_str().unwrap_or("").to_uppercase();

    let contract_obj = obj.get("contract").unwrap_or(&null_val);
    // Accept the v3 `"symbol"` key first, fall back to legacy `"root"`
    // so existing consumers keep working without an envelope rewrite.
    // The two keys are mutually exclusive in practice; downstream
    // validation runs against whichever the caller sent.
    let symbol_val = contract_obj
        .get("symbol")
        .or_else(|| contract_obj.get("root"))
        .unwrap_or(&null_val);
    let symbol = symbol_val.as_str().unwrap_or("");

    // Bound the client-supplied ticker symbol length BEFORE the string
    // flows into `Contract::stock(symbol)` /
    // `Contract::option_raw(symbol, ...)`. Without this a malicious
    // client can send a multi-megabyte `"symbol"` value in the JSON
    // subscribe envelope, triggering allocation inside the FPSS
    // contract map keyed by that string. Mirrors the REST validation
    // performed in `handler::build_endpoint_args`.
    if let Err(e) = validation::validate_symbol(symbol, "symbol") {
        tracing::warn!(error = %e, "WS subscribe: symbol failed length validation");
        let resp = sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": "ERROR",
                "req_id": req_id,
                "error": e.message.as_str(),
            }
        });
        send_response(socket, &resp, "bad_request_reply").await;
        return;
    }

    tracing::info!(
        msg_type = %msg_type,
        sec_type = %sec_type,
        req_type = %req_type,
        req_id = req_id,
        symbol = %symbol,
        add = is_add,
        "WebSocket subscription command"
    );

    let contract = if sec_type == "OPTION" {
        // Reject externally-sourced values that don't fit `i32`. Silent
        // narrowing (`as i32`) on client input is a principle violation:
        // a caller sending `strike = 9_000_000_000` would have wrapped
        // to a garbage ThetaData contract instead of surfacing the bad
        // request. Both `expiration` and `strike` are parsed fallibly.
        let exp_val = contract_obj.get("expiration").unwrap_or(&null_val);
        let exp_i64 = match exp_val.as_i64() {
            Some(v) => v,
            None => {
                tracing::warn!("WS subscribe: option expiration missing or not an integer");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": "expiration must be an integer",
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        let exp = match i32::try_from(exp_i64) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    expiration = exp_i64,
                    "WS subscribe: option expiration out of i32 range"
                );
                let err_msg = format!("expiration {exp_i64} exceeds i32 range");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str(),
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        // Range-check expiration in the YYYYMMDD domain. Any value outside
        // 1900-01-01..=2100-01-01 is a format error or an attacker probing
        // the FPSS contract-map keyspace with garbage ids. `i32::try_from`
        // already fenced the width; this fences the semantic range before
        // it reaches `Contract::option_raw`.
        //
        // PR #514 LOW-001: range-check ALONE accepts impossible Gregorian
        // dates like 20260230 (Feb 30) or 20260431 (Apr 31). Run the
        // canonical `tdbe::time::is_valid_yyyymmdd` validator alongside
        // so the WS subscribe path enforces the same calendar discipline
        // the REST validator does on the historical endpoints.
        // Both checks must pass: the bounds gate is the cheap precheck
        // (single comparison), the calendar gate catches the bad-day-of-
        // month classes the bounds check cannot see.
        if !is_valid_yyyymmdd_range(exp) {
            tracing::warn!(
                expiration = exp,
                "WS subscribe: option expiration out of YYYYMMDD range"
            );
            let err_msg = format!(
                "'exp' out of range (expected YYYYMMDD {}..={}; got {exp})",
                MIN_OPTION_EXP, MAX_OPTION_EXP
            );
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": req_id,
                    "error": err_msg.as_str(),
                }
            });
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
        if !is_valid_yyyymmdd(exp) {
            tracing::warn!(
                expiration = exp,
                "WS subscribe: option expiration is not a real Gregorian date"
            );
            let err_msg = format!(
                "'exp' is not a valid YYYYMMDD calendar date (got {exp}; \
                 reject reason: month/day decomposition is not a real \
                 Gregorian day, e.g. Feb 30 or Apr 31)"
            );
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": req_id,
                    "error": err_msg.as_str(),
                }
            });
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
        let strike_val = contract_obj.get("strike").unwrap_or(&null_val);
        let strike_i64 = match strike_val.as_i64() {
            Some(v) => v,
            None => {
                tracing::warn!("WS subscribe: option strike missing or not an integer");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": "strike must be an integer",
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        let strike = match i32::try_from(strike_i64) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    strike = strike_i64,
                    "WS subscribe: option strike out of i32 range"
                );
                let err_msg = format!("strike {strike_i64} exceeds i32 range");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str(),
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        // Strike is an OPRA-encoded price (thousandths of a dollar). A
        // non-positive value is never legal — zero is impossible and
        // negatives would wrap to a garbage key in `Contract::option_raw`.
        if strike <= 0 {
            tracing::warn!(
                strike = strike,
                "WS subscribe: option strike must be positive"
            );
            let err_msg = format!("'strike' must be positive (got {strike})");
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": req_id,
                    "error": err_msg.as_str(),
                }
            });
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
        let right_val = contract_obj.get("right").unwrap_or(&null_val);
        let is_call = match right_val.as_str().map(str::trim) {
            Some(r) if r.eq_ignore_ascii_case("C") || r.eq_ignore_ascii_case("CALL") => true,
            Some(r) if r.eq_ignore_ascii_case("P") || r.eq_ignore_ascii_case("PUT") => false,
            got => {
                let got_str = got.unwrap_or("<missing>");
                tracing::warn!(
                    right = got_str,
                    "WS subscribe: option 'right' must be one of C/CALL/P/PUT"
                );
                let err_msg =
                    format!("'right' must be one of 'C' / 'CALL' / 'P' / 'PUT' (got {got_str:?})");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str(),
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        Contract::option_raw(symbol, exp, is_call, strike)
    } else {
        Contract::stock(symbol)
    };

    let tdx = state.tdx();
    if tdx.is_streaming() {
        use thetadatadx::fpss::protocol::{
            FullSubscriptionKind, Subscription, SubscriptionKind,
        };
        let kind = match req_type.as_str() {
            "QUOTE" => Some(SubscriptionKind::Quote),
            "TRADE" => Some(SubscriptionKind::Trade),
            "OPEN_INTEREST" => Some(SubscriptionKind::OpenInterest),
            _ => None,
        };
        let result = if is_add {
            if let Some(k) = kind {
                tdx.subscribe(Subscription::Contract {
                    contract: contract.clone(),
                    kind: k,
                })
            } else if req_type == "FULL_TRADES" {
                let st = match sec_type.as_str() {
                    "OPTION" => SecType::Option,
                    "INDEX" => SecType::Index,
                    _ => SecType::Stock,
                };
                tdx.subscribe(Subscription::Full {
                    sec_type: st,
                    kind: FullSubscriptionKind::Trades,
                })
            } else {
                tracing::warn!(req_type = %req_type, "unknown req_type for subscription");
                Ok(())
            }
        } else if let Some(k) = kind {
            tdx.unsubscribe(Subscription::Contract {
                contract: contract.clone(),
                kind: k,
            })
        } else {
            Ok(())
        };

        let resp = match result {
            Ok(_) => sonic_rs::json!({
                "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
            }),
            Err(e) => {
                tracing::warn!(error = %e, "FPSS subscription failed");
                let err_msg = e.to_string();
                sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str()
                    }
                })
            }
        };
        send_response(socket, &resp, "subscription_reply").await;
    } else {
        tracing::warn!("FPSS streaming not started, subscription command ignored");
        let resp = sonic_rs::json!({
            "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
        });
        send_response(socket, &resp, "streaming_off_reply").await;
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    //  M4 — exp / strike bounds
    // -----------------------------------------------------------------------

    #[test]
    fn yyyymmdd_accepts_realistic_range() {
        assert!(is_valid_yyyymmdd_range(20260420));
        assert!(is_valid_yyyymmdd_range(20000101));
        assert!(is_valid_yyyymmdd_range(19500615));
    }

    #[test]
    fn yyyymmdd_accepts_boundaries() {
        assert!(is_valid_yyyymmdd_range(MIN_OPTION_EXP));
        assert!(is_valid_yyyymmdd_range(MAX_OPTION_EXP));
    }

    #[test]
    fn yyyymmdd_rejects_below_range() {
        assert!(!is_valid_yyyymmdd_range(0));
        assert!(!is_valid_yyyymmdd_range(18991231));
        assert!(!is_valid_yyyymmdd_range(MIN_OPTION_EXP - 1));
        assert!(!is_valid_yyyymmdd_range(-20260420));
    }

    #[test]
    fn yyyymmdd_rejects_above_range() {
        assert!(!is_valid_yyyymmdd_range(21000102));
        assert!(!is_valid_yyyymmdd_range(99999999));
        assert!(!is_valid_yyyymmdd_range(MAX_OPTION_EXP + 1));
        assert!(!is_valid_yyyymmdd_range(i32::MAX));
    }

    // -----------------------------------------------------------------------
    //  PR #514 LOW-001 — canonical Gregorian validator on the WS path
    // -----------------------------------------------------------------------

    /// The  Gregorian validator now runs on the WS option-subscribe
    /// path. Impossible calendar dates like 20260230 (Feb 30) or
    /// 20260431 (Apr 31) used to leak through because the WS path only
    /// applied the cheap `is_valid_yyyymmdd_range` bounds check; the
    /// REST handlers ran the full `tdbe::time::is_valid_yyyymmdd`
    /// calendar check alongside. Pin both behaviours here.
    #[test]
    fn ws_canonical_validator_rejects_impossible_gregorian_dates() {
        // Bounds-check passes — these dates are inside the 1900-2100
        // window the WS path accepts as "plausible YYYYMMDD".
        assert!(is_valid_yyyymmdd_range(20260230));
        assert!(is_valid_yyyymmdd_range(20260431));
        assert!(is_valid_yyyymmdd_range(20260132)); // Jan 32
        assert!(is_valid_yyyymmdd_range(20251301)); // month 13

        // ...but the canonical Gregorian validator (the REAL gate) rejects
        // them because no such calendar day exists.
        assert!(!is_valid_yyyymmdd(20260230));
        assert!(!is_valid_yyyymmdd(20260431));
        assert!(!is_valid_yyyymmdd(20260132));
        assert!(!is_valid_yyyymmdd(20251301));

        // Sanity: a real date passes both gates.
        assert!(is_valid_yyyymmdd_range(20260417));
        assert!(is_valid_yyyymmdd(20260417));

        // Leap-year semantics: 2024 is a leap year (Feb 29 is real),
        // 2026 is not.
        assert!(is_valid_yyyymmdd(20240229));
        assert!(!is_valid_yyyymmdd(20260229));
    }

    // -----------------------------------------------------------------------
    //  C1 — WS text frame cap
    // -----------------------------------------------------------------------

    #[test]
    fn ws_text_cap_is_tight() {
        // Sanity: the cap must be exactly 4 KiB. Hard number instead of
        // a self-referential assertion so a future typo in the constant
        // trips this test. A larger value would re-open the 2 GB text
        // frame OOM vector this cap was introduced to close.
        assert_eq!(WS_MAX_TEXT_BYTES, 4 * 1024);
    }
}
