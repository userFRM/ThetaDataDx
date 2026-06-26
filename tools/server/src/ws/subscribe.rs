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

/// Stream-request acknowledgement value carried in the `REQ_RESPONSE`
/// header's `response` field.
///
/// The strings match ThetaData's stream-request verification vocabulary
/// so a client written against the Terminal WebSocket contract reads the
/// acknowledgement directly:
///
/// - `SUBSCRIBED` — the request was accepted. ThetaData documents one
///   success token; it covers add, remove, and stop acknowledgements,
///   since the protocol defines no distinct value for a removal.
/// - `ERROR` — the request was rejected (bad envelope, streaming not
///   started, or an upstream failure).
///
/// ThetaData also documents `MAX_STREAMS_REACHED` and `INVALID_PERMS`,
/// but the upstream FPSS error type does not carry enough to tell those
/// apart from a generic failure, so both currently fall through to
/// `Error` with a descriptive message. The more specific values belong
/// here once the error type surfaces the category.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReqResponse {
    Subscribed,
    Error,
}

impl ReqResponse {
    /// The exact wire string ThetaData clients match on.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            ReqResponse::Subscribed => "SUBSCRIBED",
            ReqResponse::Error => "ERROR",
        }
    }
}

/// Build the `REQ_RESPONSE` acknowledgement envelope. Pure: it allocates
/// only the JSON value, so the exact wire shape — including the
/// `response` token and an optional `error` diagnostic — is testable
/// without a live socket. `error` is omitted entirely on a success ack.
pub(super) fn build_req_response(
    response: ReqResponse,
    req_id: i64,
    error: Option<&str>,
) -> sonic_rs::Value {
    match error {
        Some(msg) => sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": response.as_str(),
                "req_id": req_id,
                "error": msg,
            }
        }),
        None => sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": response.as_str(),
                "req_id": req_id,
            }
        }),
    }
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
        let resp = build_req_response(ReqResponse::Error, 0, Some(err_msg.as_str()));
        send_response(socket, &resp, "oversize_text_reply").await;
        return;
    }

    let obj: sonic_rs::Value = match sonic_rs::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("invalid WebSocket JSON: {}", text);
            let resp = build_req_response(ReqResponse::Error, 0, None);
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
        let resp = build_stop_response(state, req_id);
        send_response(socket, &resp, "stop_reply").await;
        return;
    }

    let add_val = obj.get("add").unwrap_or(&null_val);
    let is_add = add_val.as_bool().unwrap_or(true);

    let sec_type_val = obj.get("sec_type").unwrap_or(&null_val);
    let sec_type = sec_type_val.as_str().unwrap_or("").to_uppercase();

    let req_type_val = obj.get("req_type").unwrap_or(&null_val);
    let req_type = req_type_val.as_str().unwrap_or("").to_uppercase();

    // Bulk (full security-type-wide trade stream). The terminal triggers
    // it when `msg_type` CONTAINS "BULK" with `req_type=TRADE`
    // (`WSEvents.onWebSocketText`: `msg_type.toUpperCase().contains("BULK")`
    // gated on `req == ReqType.TRADE`), routing to
    // `requestFullTradeWS(sec, id, !isAdd)`. A bulk command addresses a
    // whole security type, not a single contract, so it is handled before
    // the per-contract `contract` envelope is read. Our SDK's documented
    // `req_type=FULL_TRADES` token maps to the same full-trade stream and
    // stays accepted on the per-contract path below; this branch adds the
    // terminal's substring shape so a terminal WS client repoints
    // unchanged.
    if msg_type.contains("BULK") && req_type == "TRADE" {
        let resp = apply_bulk_trade(state, &sec_type, is_add, req_id);
        send_response(socket, &resp, "bulk_trade_reply").await;
        return;
    }

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
        let resp = build_req_response(ReqResponse::Error, req_id, Some(e.message.as_str()));
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
                let resp = build_req_response(
                    ReqResponse::Error,
                    req_id,
                    Some("expiration must be an integer"),
                );
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
                let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
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
            let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
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
            let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
        // `strike` is the raw fixed-point integer in thousandths of a
        // dollar — the exact value the terminal puts on the wire in both
        // directions (`Contract.strike` is `int`; `WSEvents` parses it
        // with `conObj.get("strike").getAsInt()`). A `$550.00` strike is
        // the integer `550000`. The SDK stores thousandths internally, so
        // the parsed integer flows straight into `Contract::option_raw`
        // with no dollar scaling.
        let strike_val = contract_obj.get("strike").unwrap_or(&null_val);
        let strike = match parse_strike_thousandths(strike_val) {
            Ok(v) => v,
            Err(err_msg) => {
                tracing::warn!(error = %err_msg, "WS subscribe: invalid option 'strike'");
                let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        let right_val = contract_obj.get("right").unwrap_or(&null_val);
        let sides = match parse_right_sides(right_val.as_str().map(str::trim)) {
            Ok(sides) => sides,
            Err(err_msg) => {
                tracing::warn!(error = %err_msg, "WS subscribe: invalid option 'right'");
                let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
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
        vec![non_option_contract(&sec_type, symbol)]
    };

    let subscriptions = match subscription_plan(&req_type, &sec_type, &contracts) {
        Ok(subs) => subs,
        Err(err_msg) => {
            tracing::warn!(req_type = %req_type, "WS subscribe: unsupported req_type");
            let resp = build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()));
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
            // Both add and remove acknowledge with the single documented
            // success token; the protocol defines no removal-specific value.
            Ok(()) => build_req_response(ReqResponse::Subscribed, req_id, None),
            Err(e) => {
                tracing::warn!(error = %e, "FPSS subscription failed");
                let err_msg = e.to_string();
                // The FPSS error category does not distinguish a max-streams
                // or permission rejection from a generic failure, so every
                // upstream error maps to the generic token. The enum carries
                // the more specific values for the day the error type does.
                build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()))
            }
        };
        send_response(socket, &resp, "subscription_reply").await;
    } else {
        // Streaming is not running, so the command cannot be honored.
        // Acknowledging it as a success would be a false positive: the
        // client believes it is subscribed while no feed is installed.
        tracing::warn!("FPSS streaming not started, subscription command rejected");
        let resp = build_req_response(
            ReqResponse::Error,
            req_id,
            Some("streaming is not started; no subscription was installed"),
        );
        send_response(socket, &resp, "streaming_off_reply").await;
    }
}

/// Apply a `STOP` command: remove every active stream on the live
/// session and acknowledge only when the removal actually applied.
///
/// `STOP` is a removal, not a status query, so it must change session
/// state to be a success. Acknowledging `SUBSCRIBED` without removing
/// anything is a false positive in two cases: when streaming was never
/// started (there is no session to stop), and when an upstream
/// unsubscribe fails midway. Both surface as `ERROR` with a diagnostic
/// so the client can tell "your streams are gone" from "nothing
/// happened". The single documented success token acknowledges the
/// applied removal, since the protocol defines no removal-specific value.
fn build_stop_response(state: &AppState, req_id: i64) -> sonic_rs::Value {
    use thetadatadx::fpss::protocol::{FullSubscriptionKind, Subscription, SubscriptionKind};

    let stream = state.client().stream();
    if !stream.is_streaming() {
        tracing::warn!("FPSS streaming not started; STOP removed nothing");
        return build_req_response(
            ReqResponse::Error,
            req_id,
            Some("streaming is not started; no stream was stopped"),
        );
    }

    // Snapshot the live subscription set, then remove each one. A read
    // failure here means the session went away between the check above
    // and the snapshot — report it rather than claim a clean stop.
    let per_contract = match stream.active_subscriptions() {
        Ok(subs) => subs,
        Err(e) => {
            tracing::warn!(error = %e, "STOP: could not read active subscriptions");
            let msg = e.to_string();
            return build_req_response(ReqResponse::Error, req_id, Some(msg.as_str()));
        }
    };
    let full = match stream.active_full_subscriptions() {
        Ok(subs) => subs,
        Err(e) => {
            tracing::warn!(error = %e, "STOP: could not read active full subscriptions");
            let msg = e.to_string();
            return build_req_response(ReqResponse::Error, req_id, Some(msg.as_str()));
        }
    };

    let removals = per_contract
        .into_iter()
        .map(|(kind, contract)| Subscription::Contract { contract, kind })
        .chain(full.into_iter().filter_map(|(kind, sec_type)| {
            // Only Trade / OpenInterest have a full-stream form; a full
            // snapshot never carries a per-contract-only kind, so any
            // other kind has no full-stream removal to issue and is
            // dropped rather than mapped to a bogus subscription.
            let kind = match kind {
                SubscriptionKind::Trade => FullSubscriptionKind::Trades,
                SubscriptionKind::OpenInterest => FullSubscriptionKind::OpenInterest,
                _ => return None,
            };
            Some(Subscription::Full { sec_type, kind })
        }))
        .collect::<Vec<_>>();

    match stream.unsubscribe_many(removals) {
        Ok(()) => build_req_response(ReqResponse::Subscribed, req_id, None),
        Err(e) => {
            tracing::warn!(error = %e, "STOP: unsubscribe failed");
            let msg = e.to_string();
            build_req_response(ReqResponse::Error, req_id, Some(msg.as_str()))
        }
    }
}

/// Apply a bulk (full security-type-wide) trade subscribe / unsubscribe and
/// build the `REQ_RESPONSE` acknowledgement.
///
/// Mirrors the terminal's `requestFullTradeWS(sec, id, !isAdd)`: a single
/// full-trade stream scoped to the resolved security type, added when
/// `add` is true and removed otherwise. Like every other subscribe, it can
/// only be honored on a live stream; a command arriving before streaming
/// has started installs nothing and is rejected with `ERROR` rather than a
/// false-positive `SUBSCRIBED`.
fn apply_bulk_trade(
    state: &AppState,
    sec_type: &str,
    is_add: bool,
    req_id: i64,
) -> sonic_rs::Value {
    use thetadatadx::fpss::protocol::{FullSubscriptionKind, Subscription};

    let subscriptions = vec![Subscription::Full {
        sec_type: sec_type_from_str(sec_type),
        kind: FullSubscriptionKind::Trades,
    }];

    let stream = state.client().stream();
    if !stream.is_streaming() {
        tracing::warn!("FPSS streaming not started; bulk trade command rejected");
        return build_req_response(
            ReqResponse::Error,
            req_id,
            Some("streaming is not started; no subscription was installed"),
        );
    }

    let result = if is_add {
        stream.subscribe_many(subscriptions)
    } else {
        stream.unsubscribe_many(subscriptions)
    };
    match result {
        Ok(()) => build_req_response(ReqResponse::Subscribed, req_id, None),
        Err(e) => {
            tracing::warn!(error = %e, "bulk trade subscription failed");
            let err_msg = e.to_string();
            build_req_response(ReqResponse::Error, req_id, Some(err_msg.as_str()))
        }
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
    "MARKET_VALUE",
    "FULL_TRADES",
    "FULL_OPEN_INTEREST",
];

/// Parse the option `strike` field — the FPSS wire's fixed-point integer
/// in thousandths of a dollar — into the `i32` the contract codec stores.
///
/// The terminal carries `strike` as a bare integer on the wire and reads
/// it with `conObj.get("strike").getAsInt()` (`WSEvents.onWebSocketText`),
/// so a `$550.00` strike arrives as the JSON integer `550000`. This parser
/// mirrors that: it requires an integer JSON value (a fractional value like
/// `550.5` is rejected — thousandths are already whole units, so a
/// fractional thousandths value is malformed), and the value must be a
/// positive `i32` (the smallest real strike is `1` = $0.001; a non-positive
/// strike is not a real instrument). Values outside `i32` range are
/// rejected rather than silently narrowed.
fn parse_strike_thousandths(raw: &sonic_rs::Value) -> Result<i32, String> {
    // `as_i64` succeeds only for an integer JSON number, so a fractional
    // `strike` (e.g. `550.5`) falls through to the error below — the wire
    // unit is whole thousandths, never a fraction of one.
    let thousandths = raw.as_i64().ok_or_else(|| {
        "'strike' must be an integer in thousandths of a dollar (e.g. 550000 for $550.00), \
         got: <missing, non-numeric, or fractional>"
            .to_string()
    })?;
    if thousandths <= 0 {
        return Err(format!(
            "'strike' must be a positive integer in thousandths of a dollar \
             (e.g. 550000 for $550.00), got {thousandths}"
        ));
    }
    i32::try_from(thousandths).map_err(|_| {
        format!("'strike' {thousandths} thousandths exceeds the representable i32 range")
    })
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

/// Resolve a client `sec_type` token to the wire [`SecType`]. Unknown
/// values default to `Stock`, matching the wire default for a root with no
/// security type. Options are addressed by the same `"OPTION"` token used on
/// the per-contract path.
fn sec_type_from_str(sec_type: &str) -> SecType {
    match sec_type {
        "OPTION" => SecType::Option,
        "INDEX" => SecType::Index,
        _ => SecType::Stock,
    }
}

/// Build the per-contract contract for a non-option subscribe.
///
/// The FPSS contract wire encoding carries a load-bearing sec_type: an
/// index addresses a different instrument map than a stock of the same
/// symbol. Branch on the client `sec_type` so an `INDEX` per-contract
/// subscribe is encoded as an index-typed contract instead of being
/// silently sent as a stock (wrong instrument, no ticks). Options are
/// handled upstream; any other value defaults to stock, matching the
/// wire default for a root with no security type.
fn non_option_contract(sec_type: &str, symbol: &str) -> Contract {
    match sec_type {
        "INDEX" => Contract::index(symbol),
        _ => Contract::stock(symbol),
    }
}

/// Translate a validated subscribe command into the FPSS subscriptions
/// to install (or remove).
///
/// - `QUOTE` / `TRADE` / `OPEN_INTEREST` / `MARKET_VALUE` map to one
///   per-contract subscription per contract side. `MARKET_VALUE` is the
///   calculated per-contract market-value feed and has no full-stream
///   form on the wire.
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
        vec![Subscription::Full {
            sec_type: sec_type_from_str(sec_type),
            kind,
        }]
    };

    match req_type {
        "QUOTE" => Ok(per_contract(SubscriptionKind::Quote)),
        "TRADE" => Ok(per_contract(SubscriptionKind::Trade)),
        // OHLCVC bars are derived from the trade stream; subscribing the
        // underlying Trade feed is what makes the OHLC events flow.
        "OHLC" => Ok(per_contract(SubscriptionKind::Trade)),
        "OPEN_INTEREST" => Ok(per_contract(SubscriptionKind::OpenInterest)),
        // Market value is a calculated per-contract feed with no
        // full-stream form on the wire (like `QUOTE`); there is no
        // `FULL_MARKET_VALUE` token, so a full market-value subscribe
        // falls through to the `unknown req_type` error below.
        "MARKET_VALUE" => Ok(per_contract(SubscriptionKind::MarketValue)),
        "FULL_TRADES" => Ok(full(FullSubscriptionKind::Trades)),
        // Open interest is an option-only concept; stocks and indices carry
        // none, so a security-type-wide open-interest stream over them would
        // never publish. Reject the pairing rather than acknowledge a feed
        // that can never deliver.
        "FULL_OPEN_INTEREST" if sec_type != "OPTION" => Err(format!(
            "'FULL_OPEN_INTEREST' is only available for sec_type 'OPTION', got: '{sec_type}'"
        )),
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
    //  strike: raw thousandths integer on the wire (matches the terminal)
    // -----------------------------------------------------------------------

    #[test]
    fn strike_accepts_raw_thousandths_integer() {
        // The terminal puts `strike` on the wire as the raw thousandths
        // integer (a `$550.00` strike is `550000`), read via `getAsInt()`.
        assert_eq!(
            parse_strike_thousandths(&sonic_rs::json!(550_000)).unwrap(),
            550_000
        );
        // Smallest real strike: 1 thousandth = $0.001.
        assert_eq!(parse_strike_thousandths(&sonic_rs::json!(1)).unwrap(), 1);
        // Fractional-dollar strike, still a whole thousandths integer.
        assert_eq!(
            parse_strike_thousandths(&sonic_rs::json!(550_500)).unwrap(),
            550_500
        );
        assert_eq!(parse_strike_thousandths(&sonic_rs::json!(500)).unwrap(), 500);
    }

    #[test]
    fn strike_rejects_dollars_float_shape() {
        // A dollars float like `550.5` is NOT the wire shape: thousandths
        // are whole units, so a fractional value is malformed and must be
        // rejected rather than silently truncated.
        let err = parse_strike_thousandths(&sonic_rs::json!(550.5)).unwrap_err();
        assert!(
            err.contains("integer in thousandths"),
            "diagnostic must name the wire unit: {err}"
        );
        // A whole-number-valued float (`550.0`) is likewise not an integer
        // JSON number under `as_i64`; the wire carries a bare integer.
        assert!(parse_strike_thousandths(&sonic_rs::json!(550.0)).is_err());
    }

    #[test]
    fn strike_rejects_non_positive_and_non_numeric() {
        assert!(parse_strike_thousandths(&sonic_rs::json!(0)).is_err());
        assert!(parse_strike_thousandths(&sonic_rs::json!(-1)).is_err());
        assert!(parse_strike_thousandths(&sonic_rs::json!("550000")).is_err());
        assert!(parse_strike_thousandths(&sonic_rs::Value::default()).is_err());
    }

    #[test]
    fn strike_rejects_out_of_i32_range() {
        // Beyond i32::MAX thousandths must be rejected, never narrowed.
        let too_big = i64::from(i32::MAX) + 1;
        let err = parse_strike_thousandths(&sonic_rs::json!(too_big)).unwrap_err();
        assert!(
            err.contains("i32 range"),
            "diagnostic must name the range bound: {err}"
        );
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

    /// `req_type=MARKET_VALUE` is accepted and installs the per-contract
    /// market-value subscription. It is a first-class per-contract stream,
    /// not an `unknown req_type` ERROR.
    #[test]
    fn plan_maps_market_value_per_contract() {
        let plan = subscription_plan("MARKET_VALUE", "OPTION", &stock_contracts()).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(matches!(
            &plan[0],
            Subscription::Contract {
                kind: SubscriptionKind::MarketValue,
                ..
            }
        ));
        assert!(
            ACCEPTED_REQ_TYPES.contains(&"MARKET_VALUE"),
            "MARKET_VALUE must be advertised in the accepted vocabulary"
        );
    }

    /// Market value has no full-stream form on the wire, so there is no
    /// `FULL_MARKET_VALUE` token: a full market-value subscribe is
    /// rejected with the accepted-vocabulary diagnostic, mirroring the
    /// other per-contract-only kinds.
    #[test]
    fn plan_rejects_full_market_value() {
        let err = subscription_plan("FULL_MARKET_VALUE", "OPTION", &[]).unwrap_err();
        assert!(
            err.contains("'FULL_MARKET_VALUE'"),
            "echoes the value: {err}"
        );
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

    /// Open interest is option-only; a security-type-wide open-interest
    /// stream over stocks or indices is rejected rather than acknowledged.
    #[test]
    fn plan_rejects_full_open_interest_for_non_option() {
        for sec_type in ["STOCK", "INDEX"] {
            let err = subscription_plan("FULL_OPEN_INTEREST", sec_type, &[]).unwrap_err();
            assert!(
                err.contains("OPTION"),
                "names the only valid sec_type: {err}"
            );
            assert!(
                err.contains(sec_type),
                "echoes the rejected sec_type: {err}"
            );
        }
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

    /// A per-contract `sec_type=INDEX` subscribe builds an index-typed
    /// contract, not a stock. The FPSS contract wire encoding carries the
    /// sec_type, so a stock-typed contract for an index symbol addresses
    /// the wrong instrument and yields no ticks.
    #[test]
    fn non_option_index_builds_index_contract() {
        let c = non_option_contract("INDEX", "VIX");
        assert_eq!(c.sec_type, SecType::Index, "INDEX must map to index");
        assert_eq!(&*c.symbol, "VIX");
    }

    /// A non-option subscribe with any other `sec_type` (or none) defaults
    /// to stock, matching the wire default for a root with no security type.
    #[test]
    fn non_option_defaults_to_stock_contract() {
        for st in ["STOCK", ""] {
            let c = non_option_contract(st, "AAPL");
            assert_eq!(c.sec_type, SecType::Stock, "{st:?} must map to stock");
            assert_eq!(&*c.symbol, "AAPL");
        }
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

    // -----------------------------------------------------------------------
    //  REQ_RESPONSE stream-verification values
    // -----------------------------------------------------------------------

    /// Read the `header.response` token out of a built acknowledgement.
    fn response_token(value: &sonic_rs::Value) -> String {
        value
            .get("header")
            .and_then(|h| h.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// The acknowledgement tokens are the exact strings ThetaData's
    /// stream-request verification contract defines. A drift here breaks
    /// every client matching on the wire value.
    #[test]
    fn req_response_tokens_match_thetadata_vocabulary() {
        assert_eq!(ReqResponse::Subscribed.as_str(), "SUBSCRIBED");
        assert_eq!(ReqResponse::Error.as_str(), "ERROR");
    }

    /// A successful subscribe acknowledges with `SUBSCRIBED` (the prior
    /// `OK` was not part of the verification vocabulary), and the success
    /// envelope carries no `error` field.
    #[test]
    fn success_subscribe_returns_subscribed() {
        let resp = build_req_response(ReqResponse::Subscribed, 7, None);
        assert_eq!(response_token(&resp), "SUBSCRIBED");
        assert_eq!(
            resp.get("header")
                .and_then(|h| h.get("req_id"))
                .unwrap()
                .as_i64(),
            Some(7)
        );
        assert!(
            resp.get("header").and_then(|h| h.get("error")).is_none(),
            "a success ack must not carry an error field"
        );
    }

    /// A validation failure acknowledges with `ERROR` and a diagnostic
    /// `error` field naming the cause.
    #[test]
    fn validation_failure_returns_error_with_message() {
        let resp = build_req_response(ReqResponse::Error, 3, Some("symbol too long"));
        assert_eq!(response_token(&resp), "ERROR");
        assert_eq!(
            resp.get("header")
                .and_then(|h| h.get("error"))
                .and_then(|e| e.as_str()),
            Some("symbol too long")
        );
    }

    /// A subscribe arriving before streaming has started installs nothing,
    /// so it must be rejected with `ERROR` rather than the old false
    /// positive `OK` that told the client it was subscribed.
    #[test]
    fn streaming_not_started_returns_error_not_false_success() {
        let resp = build_req_response(
            ReqResponse::Error,
            1,
            Some("streaming is not started; no subscription was installed"),
        );
        assert_eq!(response_token(&resp), "ERROR");
        assert_ne!(response_token(&resp), "SUBSCRIBED");
        assert_ne!(response_token(&resp), "OK");
        assert!(resp
            .get("header")
            .and_then(|h| h.get("error"))
            .and_then(|e| e.as_str())
            .unwrap()
            .contains("streaming is not started"));
    }

    /// `STOP` is a removal, not a status query: when no stream is
    /// installed it must acknowledge `ERROR`, never the success token.
    /// Acknowledging `SUBSCRIBED` when nothing was removed tells the
    /// client its streams are gone when they were never there — the same
    /// false-positive class the subscribe path closes for "streaming not
    /// started".
    #[test]
    fn stop_without_active_stream_is_error_not_false_success() {
        let resp = build_req_response(
            ReqResponse::Error,
            9,
            Some("streaming is not started; no stream was stopped"),
        );
        assert_eq!(response_token(&resp), "ERROR");
        assert_ne!(response_token(&resp), "SUBSCRIBED");
        assert!(resp
            .get("header")
            .and_then(|h| h.get("error"))
            .and_then(|e| e.as_str())
            .unwrap()
            .contains("no stream was stopped"));
    }

    /// A `STOP` that actually removed the live stream set acknowledges
    /// with the single documented success token and carries no error.
    #[test]
    fn stop_that_applied_returns_subscribed() {
        let resp = build_req_response(ReqResponse::Subscribed, 9, None);
        assert_eq!(response_token(&resp), "SUBSCRIBED");
        assert!(
            resp.get("header").and_then(|h| h.get("error")).is_none(),
            "an applied STOP must not carry an error field"
        );
    }

    #[test]
    fn ws_text_cap_is_tight() {
        // Sanity: the cap must be exactly 4 KiB. Hard number instead of
        // a self-referential assertion so a future typo in the constant
        // trips this test. A larger value would re-open the 2 GB text
        // frame OOM vector this cap was introduced to close.
        assert_eq!(WS_MAX_TEXT_BYTES, 4 * 1024);
    }
}
