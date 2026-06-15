//! Subscribe / unsubscribe request handling from WebSocket clients.

use axum::extract::ws::WebSocket;
use sonic_rs::prelude::*;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::time::is_valid_yyyymmdd;
use thetadatadx::SecType;

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

    let contracts = if sec_type == "OPTION" {
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
        // Range-check ALONE accepts impossible Gregorian dates like
        // 20260230 (Feb 30) or 20260431 (Apr 31). Run the canonical
        // `thetadatadx::time::is_valid_yyyymmdd` validator alongside so the WS
        // subscribe path enforces the same calendar discipline the
        // REST validator does on the historical endpoints.
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
        // `strike` is the price in dollars — the same unit every other
        // public surface speaks. The wire's fixed-point conversion is
        // the SDK's job (`parse_strike_dollars` scales to thousandths
        // internally before `Contract::option_raw`).
        let strike_val = contract_obj.get("strike").unwrap_or(&null_val);
        let strike = match parse_strike_dollars(strike_val.as_f64()) {
            Ok(v) => v,
            Err(err_msg) => {
                tracing::warn!(error = %err_msg, "WS subscribe: invalid option 'strike'");
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
        let right_val = contract_obj.get("right").unwrap_or(&null_val);
        let sides = match parse_right_sides(right_val.as_str().map(str::trim)) {
            Ok(sides) => sides,
            Err(err_msg) => {
                tracing::warn!(error = %err_msg, "WS subscribe: invalid option 'right'");
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
        // `Both` / `*` fans out into one contract per side — the FPSS
        // wire addresses single-side contracts only, so the wildcard
        // becomes two subscribe dispatches here at the SDK boundary.
        sides
            .iter()
            .map(|&is_call| Contract::option_raw(symbol, exp, is_call, strike))
            .collect::<Vec<_>>()
    } else {
        vec![Contract::stock(symbol)]
    };

    let subscriptions = match subscription_plan(&req_type, &sec_type, &contracts) {
        Ok(subs) => subs,
        Err(err_msg) => {
            tracing::warn!(req_type = %req_type, "WS subscribe: unsupported req_type");
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

    let stream = state.client().stream();
    if stream.is_streaming() {
        let result = if is_add {
            stream.subscribe_many(subscriptions)
        } else {
            stream.unsubscribe_many(subscriptions)
        };

        let resp = match result {
            Ok(()) => sonic_rs::json!({
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
//  Pure command -> subscription planning (testable without a socket)
// ---------------------------------------------------------------------------

/// `req_type` values accepted on the subscribe path. Echoed in the ERROR
/// diagnostic for unknown values so clients can discover the vocabulary
/// without consulting the docs.
const ACCEPTED_REQ_TYPES: &[&str] = &[
    "QUOTE",
    "TRADE",
    "OHLC",
    "OPEN_INTEREST",
    "FULL_TRADES",
    "FULL_OPEN_INTEREST",
];

/// Parse the option `strike` field (dollars, JSON number) into the
/// FPSS wire's fixed-point thousandths integer.
///
/// Accepts any positive finite number; rejects missing / non-numeric
/// values, non-positive strikes, and values whose thousandths scaling
/// leaves `i32`. Dollars are the only accepted unit — clients holding
/// the wire integer divide by 1000 before subscribing.
fn parse_strike_dollars(raw: Option<f64>) -> Result<i32, String> {
    let dollars = raw.ok_or_else(|| {
        "'strike' must be a number in dollars (e.g. 550 or 550.5), got: <missing or non-numeric>"
            .to_string()
    })?;
    if !dollars.is_finite() || dollars <= 0.0 {
        return Err(format!(
            "'strike' must be a positive number in dollars (got {dollars})"
        ));
    }
    let scaled = (dollars * 1000.0).round();
    if scaled > f64::from(i32::MAX) {
        return Err(format!(
            "'strike' {dollars} exceeds the representable range after fixed-point scaling"
        ));
    }
    // Reason: bounds checked above; positivity checked above.
    #[allow(clippy::cast_possible_truncation)]
    Ok(scaled as i32)
}

/// Parse the option `right` field into the contract sides to subscribe.
///
/// Routes through the same `thetadatadx::greeks::parse_right` parser the REST
/// validators use, so the two surfaces accept one vocabulary:
/// `call` / `put` / `both` / `C` / `P` / `*`, case-insensitive. `Both`
/// (and `*`) yields both sides — the FPSS wire addresses single-side
/// contracts only, so the wildcard fans out into one dispatch per side.
fn parse_right_sides(raw: Option<&str>) -> Result<Vec<bool>, String> {
    let raw = raw.ok_or_else(|| {
        "'right' must be one of: 'call', 'put', 'both', 'C', 'P', '*' (case-insensitive), got: <missing>".to_string()
    })?;
    let parsed = thetadatadx::greeks::parse_right(raw).map_err(|_| {
        format!(
            "'right' must be one of: 'call', 'put', 'both', 'C', 'P', '*' (case-insensitive), got: '{raw}'"
        )
    })?;
    Ok(match parsed.as_is_call() {
        Some(is_call) => vec![is_call],
        // `Both` has no single FPSS side; subscribe the call AND the put.
        None => vec![true, false],
    })
}

/// Translate a validated subscribe command into the FPSS subscriptions
/// to install (or remove).
///
/// - `QUOTE` / `TRADE` / `OPEN_INTEREST` map to one per-contract
///   subscription per contract side.
/// - `OHLC` maps to the per-contract Trade stream: OHLCVC bars are
///   derived from trades, so the bar feed flows once the trade
///   subscription is installed. Previously this arm silently returned
///   OK without installing anything.
/// - `FULL_TRADES` / `FULL_OPEN_INTEREST` map to the security-type-wide
///   full stream.
/// - Anything else is an error listing the accepted vocabulary.
fn subscription_plan(
    req_type: &str,
    sec_type: &str,
    contracts: &[thetadatadx::fpss::protocol::Contract],
) -> Result<Vec<thetadatadx::fpss::protocol::Subscription>, String> {
    use thetadatadx::fpss::protocol::{FullSubscriptionKind, Subscription, SubscriptionKind};

    let per_contract = |kind: SubscriptionKind| {
        contracts
            .iter()
            .map(|contract| Subscription::Contract {
                contract: contract.clone(),
                kind,
            })
            .collect::<Vec<_>>()
    };

    let full = |kind: FullSubscriptionKind| {
        let st = match sec_type {
            "OPTION" => SecType::Option,
            "INDEX" => SecType::Index,
            _ => SecType::Stock,
        };
        vec![Subscription::Full { sec_type: st, kind }]
    };

    match req_type {
        "QUOTE" => Ok(per_contract(SubscriptionKind::Quote)),
        "TRADE" => Ok(per_contract(SubscriptionKind::Trade)),
        // OHLCVC bars are derived from the trade stream; subscribing the
        // underlying Trade feed is what makes the OHLC events flow.
        "OHLC" => Ok(per_contract(SubscriptionKind::Trade)),
        "OPEN_INTEREST" => Ok(per_contract(SubscriptionKind::OpenInterest)),
        "FULL_TRADES" => Ok(full(FullSubscriptionKind::Trades)),
        "FULL_OPEN_INTEREST" => Ok(full(FullSubscriptionKind::OpenInterest)),
        other => Err(format!(
            "unknown req_type: '{other}' (accepted: {})",
            ACCEPTED_REQ_TYPES.join(", ")
        )),
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
    //  Canonical Gregorian validator on the WS path
    // -----------------------------------------------------------------------

    /// The  Gregorian validator now runs on the WS option-subscribe
    /// path. Impossible calendar dates like 20260230 (Feb 30) or
    /// 20260431 (Apr 31) used to leak through because the WS path only
    /// applied the cheap `is_valid_yyyymmdd_range` bounds check; the
    /// REST handlers ran the full `thetadatadx::time::is_valid_yyyymmdd`
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
    //  right vocabulary — shared with the REST validator
    // -----------------------------------------------------------------------

    #[test]
    fn right_accepts_rest_vocabulary_single_sides() {
        for (raw, expected) in [
            ("C", vec![true]),
            ("c", vec![true]),
            ("CALL", vec![true]),
            ("Call", vec![true]),
            ("P", vec![false]),
            ("put", vec![false]),
            ("PUT", vec![false]),
        ] {
            assert_eq!(parse_right_sides(Some(raw)).unwrap(), expected, "{raw}");
        }
    }

    /// `Both` / `*` fan out into both sides — one subscribe dispatch per
    /// side, since the FPSS wire addresses single-side contracts only.
    #[test]
    fn right_both_and_wildcard_fan_out_to_two_sides() {
        for raw in ["Both", "both", "BOTH", "*"] {
            assert_eq!(
                parse_right_sides(Some(raw)).unwrap(),
                vec![true, false],
                "{raw}"
            );
        }
    }

    #[test]
    fn right_rejects_garbage_with_rest_error_shape() {
        for raw in [Some("xyz"), Some(""), Some("CP"), None] {
            let err = parse_right_sides(raw).unwrap_err();
            assert!(
                err.contains("'call', 'put', 'both', 'C', 'P', '*'"),
                "diagnostic must list the REST vocabulary: {err}"
            );
        }
    }

    // -----------------------------------------------------------------------
    //  req_type -> subscription planning
    // -----------------------------------------------------------------------

    use thetadatadx::fpss::protocol::{
        Contract, FullSubscriptionKind, Subscription, SubscriptionKind,
    };

    fn stock_contracts() -> Vec<Contract> {
        vec![Contract::stock("AAPL")]
    }

    #[test]
    fn plan_maps_quote_trade_open_interest_per_contract() {
        for (req, kind) in [
            ("QUOTE", SubscriptionKind::Quote),
            ("TRADE", SubscriptionKind::Trade),
            ("OPEN_INTEREST", SubscriptionKind::OpenInterest),
        ] {
            let plan = subscription_plan(req, "STOCK", &stock_contracts()).unwrap();
            assert_eq!(plan.len(), 1, "{req}");
            assert!(
                matches!(&plan[0], Subscription::Contract { kind: k, .. } if *k == kind),
                "{req} maps to {kind:?}"
            );
        }
    }

    /// OHLCVC bars are derived from trades: `req_type=OHLC` installs the
    /// per-contract Trade subscription so the bar feed flows. The old
    /// arm returned OK without installing anything.
    #[test]
    fn plan_maps_ohlc_to_trade_subscription() {
        let plan = subscription_plan("OHLC", "STOCK", &stock_contracts()).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(matches!(
            &plan[0],
            Subscription::Contract {
                kind: SubscriptionKind::Trade,
                ..
            }
        ));
    }

    #[test]
    fn plan_maps_full_streams_to_sec_type_scope() {
        let plan = subscription_plan("FULL_TRADES", "OPTION", &[]).unwrap();
        assert_eq!(
            plan,
            vec![Subscription::Full {
                sec_type: SecType::Option,
                kind: FullSubscriptionKind::Trades,
            }]
        );

        let plan = subscription_plan("FULL_OPEN_INTEREST", "OPTION", &[]).unwrap();
        assert_eq!(
            plan,
            vec![Subscription::Full {
                sec_type: SecType::Option,
                kind: FullSubscriptionKind::OpenInterest,
            }]
        );
    }

    /// `right=Both` produces two contracts; the plan installs one
    /// subscription per side.
    #[test]
    fn plan_fans_out_both_sides_for_options() {
        let sides = parse_right_sides(Some("Both")).unwrap();
        let contracts: Vec<Contract> = sides
            .iter()
            .map(|&is_call| Contract::option_raw("SPY", 20260620, is_call, 550_000))
            .collect();
        let plan = subscription_plan("QUOTE", "OPTION", &contracts).unwrap();
        assert_eq!(plan.len(), 2, "one subscription per option side");
        let kinds_ok = plan.iter().all(|sub| {
            matches!(
                sub,
                Subscription::Contract {
                    kind: SubscriptionKind::Quote,
                    ..
                }
            )
        });
        assert!(kinds_ok, "both sides subscribe the same tick kind");
    }

    /// Unknown `req_type` values return ERROR with the accepted set —
    /// the silent-OK-without-subscribing behavior is gone.
    #[test]
    fn plan_rejects_unknown_req_type_with_accepted_list() {
        let err = subscription_plan("OHLCVC", "STOCK", &stock_contracts()).unwrap_err();
        assert!(err.contains("'OHLCVC'"), "echoes the value: {err}");
        for accepted in ACCEPTED_REQ_TYPES {
            assert!(err.contains(accepted), "lists {accepted}: {err}");
        }
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
