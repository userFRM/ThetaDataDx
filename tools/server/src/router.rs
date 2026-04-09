//! Route generation from the endpoint registry.
//!
//! Iterates `ENDPOINTS` at startup and registers a handler for every one of
//! the 61 endpoints, plus system routes. Each endpoint is mapped to a REST
//! path following the ThetaData v3 API convention. Paths are generated in the
//! core registry so the REST server does not re-derive them heuristically.

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use thetadatadx::registry::{EndpointMeta, ENDPOINTS};

use crate::handler;
use crate::state::AppState;

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
        let ep_arc: &'static EndpointMeta = ep;
        let ep_shared = Arc::new(ep_arc);
        let handler_fn = move |s: axum::extract::State<AppState>,
                               q: axum::extract::Query<
            std::collections::HashMap<String, String>,
        >| {
            let ep = Arc::clone(&ep_shared);
            async move { handler::generic(s, q, &ep).await }
        };
        app = app.route(ep.rest_path, get(handler_fn));
        registered += 1;
        tracing::debug!(endpoint = ep.name, path = ep.rest_path, "registered route");
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
