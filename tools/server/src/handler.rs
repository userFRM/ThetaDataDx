//! Generic endpoint handler and dispatch for all 61 endpoints.
//!
//! A single handler function receives the endpoint metadata via closure
//! capture, extracts and validates query params, calls `DirectClient`,
//! and returns the Java terminal JSON envelope (or CSV if `use_csv=true`).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sonic_rs::prelude::*;

use thetadatadx::registry::EndpointMeta;

use crate::format;
use crate::state::AppState;

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Build a JSON error response with the Java terminal error envelope format.
fn error_response(status: StatusCode, error_type: &str, msg: &str) -> Response {
    let body = format::error_envelope(error_type, msg);
    let json_bytes = sonic_rs::to_string(&body).unwrap_or_default();
    (
        status,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        json_bytes,
    )
        .into_response()
}

/// Extract a param by primary name, falling back to an alternate name.
fn get_param(params: &HashMap<String, String>, name: &str, alt: &str) -> String {
    params
        .get(name)
        .or_else(|| params.get(alt))
        .cloned()
        .unwrap_or_default()
}

/// Split a comma-separated symbols string into a `Vec<&str>`.
fn parse_symbols(s: &str) -> Vec<&str> {
    s.split(',')
        .map(|sym| sym.trim())
        .filter(|sym| !sym.is_empty())
        .collect()
}

/// Serialize a `sonic_rs::Value` to an axum JSON response body.
fn json_response(val: &sonic_rs::Value) -> Response {
    let json_bytes = sonic_rs::to_string(val).unwrap_or_default();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        json_bytes,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
//  Generic endpoint handler
// ---------------------------------------------------------------------------

/// Generic handler invoked for all 61 registry endpoints.
///
/// 1. Validates required query params against `EndpointMeta.params`.
/// 2. Dispatches to the correct `DirectClient` method.
/// 3. Returns JSON envelope or CSV depending on `use_csv` query param.
pub async fn generic(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    ep: &EndpointMeta,
) -> Response {
    // Validate required params.
    for p in ep.params {
        if p.required && !params.contains_key(p.name) {
            let alt = match p.name {
                "symbol" | "symbols" => "root",
                "expiration" => "exp",
                "interval" => "ivl",
                _ => "",
            };
            if alt.is_empty() || !params.contains_key(alt) {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    &format!(
                        "missing required parameter: '{}' ({})",
                        p.name, p.description
                    ),
                );
            }
        }
    }

    let use_csv = params
        .get("use_csv")
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");

    let result = dispatch(state.client(), ep, &params).await;

    match result {
        Ok(json_val) => {
            if use_csv {
                // Extract the "response" array and convert to CSV.
                if let Some(arr) = json_val
                    .get("response")
                    .and_then(|v: &sonic_rs::Value| v.as_array())
                {
                    let items: Vec<sonic_rs::Value> = arr.iter().cloned().collect();
                    if let Some(csv) = format::json_to_csv(&items) {
                        return (
                            StatusCode::OK,
                            [("content-type", "text/csv; charset=utf-8")],
                            csv,
                        )
                            .into_response();
                    }
                }
                // Fallback: empty CSV
                (
                    StatusCode::OK,
                    [("content-type", "text/csv; charset=utf-8")],
                    String::new(),
                )
                    .into_response()
            } else {
                json_response(&json_val)
            }
        }
        Err(e) => {
            tracing::warn!(endpoint = ep.name, error = %e, "request failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
//  Dispatch -- match on endpoint name -> DirectClient method call
// ---------------------------------------------------------------------------

/// Dispatch a single endpoint call. Returns the full JSON envelope on success.
async fn dispatch(
    client: &thetadatadx::direct::DirectClient,
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> Result<sonic_rs::Value, thetadatadx::Error> {
    // Shorthand closures for common param extraction.
    let sym = || get_param(params, "symbol", "root");
    let syms_str = || get_param(params, "symbols", "root");
    let start = || get_param(params, "start_date", "start_date");
    let end = || get_param(params, "end_date", "end_date");
    let date = || get_param(params, "date", "date");
    let interval = || {
        let v = get_param(params, "interval", "ivl");
        if v.is_empty() {
            "60000".to_string()
        } else {
            v
        }
    };
    let interval_quote = || {
        let v = get_param(params, "interval", "ivl");
        if v.is_empty() {
            "0".to_string()
        } else {
            v
        }
    };
    let exp = || get_param(params, "expiration", "exp");
    let strike = || get_param(params, "strike", "strike");
    let right = || get_param(params, "right", "right");
    let request_type = || get_param(params, "request_type", "request_type");
    let time_of_day = || get_param(params, "time_of_day", "time_of_day");
    let year = || get_param(params, "year", "year");

    match ep.name {
        // ── Stock List (2) ──────────────────────────────────────────
        "stock_list_symbols" => {
            let items = client.stock_list_symbols().await?;
            Ok(format::list_envelope(&items))
        }
        "stock_list_dates" => {
            let items = client.stock_list_dates(&request_type(), &sym()).await?;
            Ok(format::list_envelope(&items))
        }

        // ── Stock Snapshot (4) ──────────────────────────────────────
        "stock_snapshot_ohlc" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.stock_snapshot_ohlc(&syms).await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "stock_snapshot_trade" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.stock_snapshot_trade(&syms).await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "stock_snapshot_quote" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.stock_snapshot_quote(&syms).await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }
        "stock_snapshot_market_value" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.stock_snapshot_market_value(&syms).await?;
            Ok(format::ok_envelope(format::market_value_ticks_to_json(
                &ticks,
            )))
        }

        // ── Stock History (6) ───────────────────────────────────────
        "stock_history_eod" => {
            let ticks = client.stock_history_eod(&sym(), &start(), &end()).await?;
            Ok(format::ok_envelope(format::eod_ticks_to_json(&ticks)))
        }
        "stock_history_ohlc" => {
            let ticks = client
                .stock_history_ohlc(&sym(), &date(), &interval())
                .await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "stock_history_ohlc_range" => {
            let ticks = client
                .stock_history_ohlc_range(&sym(), &start(), &end(), &interval())
                .await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "stock_history_trade" => {
            let ticks = client.stock_history_trade(&sym(), &date()).await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "stock_history_quote" => {
            let ticks = client
                .stock_history_quote(&sym(), &date(), &interval_quote())
                .await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }
        "stock_history_trade_quote" => {
            let ticks = client.stock_history_trade_quote(&sym(), &date()).await?;
            Ok(format::ok_envelope(format::trade_quote_ticks_to_json(
                &ticks,
            )))
        }

        // ── Stock At-Time (2) ───────────────────────────────────────
        "stock_at_time_trade" => {
            let ticks = client
                .stock_at_time_trade(&sym(), &start(), &end(), &time_of_day())
                .await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "stock_at_time_quote" => {
            let ticks = client
                .stock_at_time_quote(&sym(), &start(), &end(), &time_of_day())
                .await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }

        // ── Option List (5) ─────────────────────────────────────────
        "option_list_symbols" => {
            let items = client.option_list_symbols().await?;
            Ok(format::list_envelope(&items))
        }
        "option_list_dates" => {
            let items = client
                .option_list_dates(&request_type(), &sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::list_envelope(&items))
        }
        "option_list_expirations" => {
            let items = client.option_list_expirations(&sym()).await?;
            Ok(format::list_envelope(&items))
        }
        "option_list_strikes" => {
            let items = client.option_list_strikes(&sym(), &exp()).await?;
            Ok(format::list_envelope(&items))
        }
        "option_list_contracts" => {
            let ticks = client
                .option_list_contracts(&request_type(), &sym(), &date())
                .await?;
            Ok(format::ok_envelope(format::option_contracts_to_json(
                &ticks,
            )))
        }

        // ── Option Snapshot (10) ────────────────────────────────────
        "option_snapshot_ohlc" => {
            let ticks = client
                .option_snapshot_ohlc(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "option_snapshot_trade" => {
            let ticks = client
                .option_snapshot_trade(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "option_snapshot_quote" => {
            let ticks = client
                .option_snapshot_quote(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }
        "option_snapshot_open_interest" => {
            let ticks = client
                .option_snapshot_open_interest(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::open_interest_ticks_to_json(
                &ticks,
            )))
        }
        "option_snapshot_market_value" => {
            let ticks = client
                .option_snapshot_market_value(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::market_value_ticks_to_json(
                &ticks,
            )))
        }
        "option_snapshot_greeks_implied_volatility" => {
            let ticks = client
                .option_snapshot_greeks_implied_volatility(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::iv_ticks_to_json(&ticks)))
        }
        "option_snapshot_greeks_all" => {
            let ticks = client
                .option_snapshot_greeks_all(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_snapshot_greeks_first_order" => {
            let ticks = client
                .option_snapshot_greeks_first_order(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_snapshot_greeks_second_order" => {
            let ticks = client
                .option_snapshot_greeks_second_order(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_snapshot_greeks_third_order" => {
            let ticks = client
                .option_snapshot_greeks_third_order(&sym(), &exp(), &strike(), &right())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }

        // ── Option History (6) ──────────────────────────────────────
        "option_history_eod" => {
            let ticks = client
                .option_history_eod(&sym(), &exp(), &strike(), &right(), &start(), &end())
                .await?;
            Ok(format::ok_envelope(format::eod_ticks_to_json(&ticks)))
        }
        "option_history_ohlc" => {
            let ticks = client
                .option_history_ohlc(&sym(), &exp(), &strike(), &right(), &date(), &interval())
                .await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "option_history_trade" => {
            let ticks = client
                .option_history_trade(&sym(), &exp(), &strike(), &right(), &date())
                .await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "option_history_quote" => {
            let ticks = client
                .option_history_quote(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval_quote(),
                )
                .await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }
        "option_history_trade_quote" => {
            let ticks = client
                .option_history_trade_quote(&sym(), &exp(), &strike(), &right(), &date())
                .await?;
            Ok(format::ok_envelope(format::trade_quote_ticks_to_json(
                &ticks,
            )))
        }
        "option_history_open_interest" => {
            let ticks = client
                .option_history_open_interest(&sym(), &exp(), &strike(), &right(), &date())
                .await?;
            Ok(format::ok_envelope(format::open_interest_ticks_to_json(
                &ticks,
            )))
        }

        // ── Option History Greeks (12) ──────────────────────────────
        "option_history_greeks_eod" => {
            let ticks = client
                .option_history_greeks_eod(&sym(), &exp(), &strike(), &right(), &start(), &end())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_greeks_all" => {
            let ticks = client
                .option_history_greeks_all(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_trade_greeks_all" => {
            let ticks = client
                .option_history_trade_greeks_all(&sym(), &exp(), &strike(), &right(), &date())
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_greeks_first_order" => {
            let ticks = client
                .option_history_greeks_first_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_trade_greeks_first_order" => {
            let ticks = client
                .option_history_trade_greeks_first_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_greeks_second_order" => {
            let ticks = client
                .option_history_greeks_second_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_trade_greeks_second_order" => {
            let ticks = client
                .option_history_trade_greeks_second_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_greeks_third_order" => {
            let ticks = client
                .option_history_greeks_third_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_trade_greeks_third_order" => {
            let ticks = client
                .option_history_trade_greeks_third_order(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                )
                .await?;
            Ok(format::ok_envelope(format::greeks_ticks_to_json(&ticks)))
        }
        "option_history_greeks_implied_volatility" => {
            let ticks = client
                .option_history_greeks_implied_volatility(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                    &interval(),
                )
                .await?;
            Ok(format::ok_envelope(format::iv_ticks_to_json(&ticks)))
        }
        "option_history_trade_greeks_implied_volatility" => {
            let ticks = client
                .option_history_trade_greeks_implied_volatility(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &date(),
                )
                .await?;
            Ok(format::ok_envelope(format::iv_ticks_to_json(&ticks)))
        }

        // ── Option At-Time (2) ──────────────────────────────────────
        "option_at_time_trade" => {
            let ticks = client
                .option_at_time_trade(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &start(),
                    &end(),
                    &time_of_day(),
                )
                .await?;
            Ok(format::ok_envelope(format::trade_ticks_to_json(&ticks)))
        }
        "option_at_time_quote" => {
            let ticks = client
                .option_at_time_quote(
                    &sym(),
                    &exp(),
                    &strike(),
                    &right(),
                    &start(),
                    &end(),
                    &time_of_day(),
                )
                .await?;
            Ok(format::ok_envelope(format::quote_ticks_to_json(&ticks)))
        }

        // ── Index List (2) ──────────────────────────────────────────
        "index_list_symbols" => {
            let items = client.index_list_symbols().await?;
            Ok(format::list_envelope(&items))
        }
        "index_list_dates" => {
            let items = client.index_list_dates(&sym()).await?;
            Ok(format::list_envelope(&items))
        }

        // ── Index Snapshot (3) ──────────────────────────────────────
        "index_snapshot_ohlc" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.index_snapshot_ohlc(&syms).await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "index_snapshot_price" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.index_snapshot_price(&syms).await?;
            Ok(format::ok_envelope(format::price_ticks_to_json(&ticks)))
        }
        "index_snapshot_market_value" => {
            let s = syms_str();
            let syms = parse_symbols(&s);
            let ticks = client.index_snapshot_market_value(&syms).await?;
            Ok(format::ok_envelope(format::market_value_ticks_to_json(
                &ticks,
            )))
        }

        // ── Index History (3) ───────────────────────────────────────
        "index_history_eod" => {
            let ticks = client.index_history_eod(&sym(), &start(), &end()).await?;
            Ok(format::ok_envelope(format::eod_ticks_to_json(&ticks)))
        }
        "index_history_ohlc" => {
            let ticks = client
                .index_history_ohlc(&sym(), &start(), &end(), &interval())
                .await?;
            Ok(format::ok_envelope(format::ohlc_ticks_to_json(&ticks)))
        }
        "index_history_price" => {
            let ticks = client
                .index_history_price(&sym(), &date(), &interval())
                .await?;
            Ok(format::ok_envelope(format::price_ticks_to_json(&ticks)))
        }

        // ── Index At-Time (1) ───────────────────────────────────────
        "index_at_time_price" => {
            let ticks = client
                .index_at_time_price(&sym(), &start(), &end(), &time_of_day())
                .await?;
            Ok(format::ok_envelope(format::price_ticks_to_json(&ticks)))
        }

        // ── Calendar (3) ────────────────────────────────────────────
        "calendar_open_today" => {
            let ticks = client.calendar_open_today().await?;
            Ok(format::ok_envelope(format::calendar_days_to_json(&ticks)))
        }
        "calendar_on_date" => {
            let ticks = client.calendar_on_date(&date()).await?;
            Ok(format::ok_envelope(format::calendar_days_to_json(&ticks)))
        }
        "calendar_year" => {
            let ticks = client.calendar_year(&year()).await?;
            Ok(format::ok_envelope(format::calendar_days_to_json(&ticks)))
        }

        // ── Interest Rate (1) ───────────────────────────────────────
        "interest_rate_history_eod" => {
            let ticks = client
                .interest_rate_history_eod(&sym(), &start(), &end())
                .await?;
            Ok(format::ok_envelope(format::interest_rate_ticks_to_json(
                &ticks,
            )))
        }

        _ => Err(thetadatadx::Error::Config(format!(
            "unhandled endpoint in REST dispatch: {}",
            ep.name
        ))),
    }
}

// ---------------------------------------------------------------------------
//  System endpoints
// ---------------------------------------------------------------------------

/// GET /v3/system/status -- matches Java terminal system status.
pub async fn system_status(State(state): State<AppState>) -> Response {
    let body = sonic_rs::json!({
        "status": state.mdds_status(),
        "version": env!("CARGO_PKG_VERSION")
    });
    json_response(&body)
}

/// GET /v3/system/mdds/status
pub async fn system_mdds_status(State(state): State<AppState>) -> Response {
    let body = format::ok_envelope(vec![sonic_rs::Value::from(state.mdds_status())]);
    json_response(&body)
}

/// GET /v3/system/fpss/status
pub async fn system_fpss_status(State(state): State<AppState>) -> Response {
    let body = sonic_rs::json!({
        "status": state.fpss_status(),
        "version": env!("CARGO_PKG_VERSION")
    });
    json_response(&body)
}

/// GET /v3/system/shutdown -- requires `X-Shutdown-Token` header.
pub async fn system_shutdown(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = headers
        .get("X-Shutdown-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !state.validate_shutdown_token(token) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid or missing X-Shutdown-Token header",
        );
    }

    tracing::info!("shutdown requested via REST API with valid token");
    state.shutdown();
    let body = format::ok_envelope(vec![sonic_rs::Value::from("OK")]);
    json_response(&body)
}
