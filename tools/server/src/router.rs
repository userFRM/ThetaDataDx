//! Route generation from the endpoint registry.
//!
//! Iterates `ENDPOINTS` at startup and registers a handler for every one of
//! the 61 endpoints, plus system routes. Each endpoint is mapped to a REST
//! path following the ThetaData v3 API convention.
//!
//! v3 API format (from https://docs.thetadata.us/openapiv3.yaml):
//!   Base: /v3
//!   Pattern: /v3/{category}/{subcategory}/{what}
//!   Port: 25503

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use thetadatadx::registry::{EndpointMeta, ENDPOINTS};

use crate::handler;
use crate::state::AppState;

// ---------------------------------------------------------------------------
//  Path generation — v3 API format
// ---------------------------------------------------------------------------

/// Derive a REST path from an endpoint, matching ThetaData v3 OpenAPI spec.
///
/// v3 paths follow: /v3/{category}/{action}/{detail}
///
/// Examples:
/// - stock_list_symbols              -> /v3/stock/list/symbols
/// - stock_history_eod               -> /v3/stock/history/eod
/// - option_list_expirations         -> /v3/option/list/expirations
/// - option_snapshot_greeks_all      -> /v3/option/snapshot/greeks/all
/// - option_history_trade_greeks_all -> /v3/option/history/trade_greeks/all
/// - calendar_open_today             -> /v3/calendar/open_today
/// - interest_rate_history_eod       -> /v3/rate/history/eod
pub fn endpoint_to_path(ep: &EndpointMeta) -> Option<String> {
    let name = ep.name;
    let cat = ep.category;

    // Strip category prefix to get the rest: "stock_history_eod" -> "history_eod"
    let rest = if cat == "rate" {
        name.strip_prefix("interest_rate_")?
    } else {
        name.strip_prefix(&format!("{cat}_"))?
    };

    // Convert underscores to path segments based on known patterns
    // The v3 API is: /v3/{category}/{rest_as_path}
    // where rest uses / as separators: history_eod -> history/eod
    //                                  snapshot_greeks_all -> snapshot/greeks/all
    //                                  list_symbols -> list/symbols

    // Special cases for nested subcategories
    let path_rest = if rest.starts_with("history_trade_greeks_") {
        let what = rest.strip_prefix("history_trade_greeks_")?;
        format!("history/trade_greeks/{what}")
    } else if rest.starts_with("history_greeks_") {
        let what = rest.strip_prefix("history_greeks_")?;
        format!("history/greeks/{what}")
    } else if rest.starts_with("snapshot_greeks_") {
        let what = rest.strip_prefix("snapshot_greeks_")?;
        format!("snapshot/greeks/{what}")
    } else if rest.starts_with("history_") {
        let what = rest.strip_prefix("history_")?;
        format!("history/{what}")
    } else if rest.starts_with("snapshot_") {
        let what = rest.strip_prefix("snapshot_")?;
        format!("snapshot/{what}")
    } else if rest.starts_with("list_") {
        let what = rest.strip_prefix("list_")?;
        format!("list/{what}")
    } else if rest.starts_with("at_time_") {
        let what = rest.strip_prefix("at_time_")?;
        format!("at_time/{what}")
    } else {
        // Calendar endpoints: open_today, on_date, year
        rest.to_string()
    };

    Some(format!("/v3/{cat}/{path_rest}"))
}

// ---------------------------------------------------------------------------
//  Router construction
// ---------------------------------------------------------------------------

/// Build the full REST API router with routes dynamically generated from
/// the endpoint registry plus hand-written system routes.
///
/// Default port: 25503 (matching ThetaData v3 terminal).
pub fn build(state: AppState) -> Router {
    let mut app = Router::new();
    let mut registered = 0usize;

    for ep in ENDPOINTS {
        if let Some(path) = endpoint_to_path(ep) {
            let ep_arc: &'static EndpointMeta = ep;
            let ep_shared = Arc::new(ep_arc);
            let handler_fn = move |s: axum::extract::State<AppState>,
                                   q: axum::extract::Query<
                std::collections::HashMap<String, String>,
            >| {
                let ep = Arc::clone(&ep_shared);
                async move { handler::generic(s, q, &ep).await }
            };
            app = app.route(&path, get(handler_fn));
            registered += 1;
            tracing::debug!(endpoint = ep.name, path = %path, "registered route");
        } else {
            tracing::warn!(
                endpoint = ep.name,
                category = ep.category,
                subcategory = ep.subcategory,
                "could not generate REST path"
            );
        }
    }

    tracing::info!(
        count = registered,
        "registered endpoint routes from registry"
    );

    // System routes
    app = app
        .route("/v3/system/status", get(handler::system_status))
        .route("/v3/system/mdds/status", get(handler::system_mdds_status))
        .route("/v3/system/fpss/status", get(handler::system_fpss_status))
        .route("/v3/system/shutdown", get(handler::system_shutdown));

    app.with_state(state)
}
