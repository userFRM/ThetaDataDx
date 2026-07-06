//! Endpoint-to-parser routing regression tests.
//!
//! The sibling `test_eod_greeks_schema.rs` and
//! `test_trade_greeks_schema.rs` suites prove that the per-tick
//! *parsers* (`parse_greeks_eod_ticks`, `parse_trade_greeks_*_ticks`,
//! `parse_index_price_at_time_ticks`) preserve every wire column when
//! fed a hand-built `DataTable`. They do NOT prove that the high-level
//! `MarketDataClient::<endpoint>` dispatch method routes the response through
//! that parser -- the heuristic that picks `parse: decode::parse_<tick>`
//! for each endpoint inside `build_support/endpoints/proto_parser.rs`
//! can drift silently and the per-parser regressions would still pass.
//!
//! These tests close the routing gap end-to-end. For each of the
//! seven endpoints whose silent mis-routing previously dropped the
//! trade-side execution / EOD trade-quote columns, the harness:
//!
//!   1. Loads the captured wire fixture as a raw
//!      `proto::ResponseData`.
//!   2. Spins up the in-process `grpc_mock_server` mock and serves
//!      that single `ResponseData` chunk under the gRPC stub path
//!      the real `MarketDataClient::<endpoint>` builder dispatches to.
//!   3. Builds an `MarketDataClient` against the mock via the
//!      `__test-helpers`-gated `for_endpoint_routing_test` constructor.
//!   4. Awaits the real builder (`client.<endpoint>(...).await`) and
//!      asserts the returned `Vec<X>` carries the concrete tick type
//!      (compile-time type-binding) AND that the trade-side
//!      execution / EOD trade-quote columns the silent-routing
//!      dropped are populated on the first row.
//!
//! Pinning routing AND parsing in one assertion: a future drift in
//! `proto_parser.rs` that reverts `option_history_greeks_eod` back to
//! `parse_greeks_all_ticks` (or any other wrong parser) would either
//! type-fail at compile time (wrong `Vec<Tick>` shape) or runtime-fail
//! the column-population asserts here.

#![cfg(feature = "__test-helpers")]

use std::sync::Arc;

use thetadatadx::{
    GreeksEodTick, IndexPriceAtTimeTick, TradeGreeksAllTick, TradeGreeksFirstOrderTick,
    TradeGreeksImpliedVolatilityTick, TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick,
};
use tokio::sync::Semaphore;

use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::mdds::MarketDataClient;
use thetadatadx::wire as proto;
use thetadatadx::DirectConfig;

#[path = "grpc_mock_server.rs"]
mod mock;

#[path = "common/capture_loader.rs"]
mod capture_loader;

use capture_loader::load_response_data as load_response;

/// Stand up an in-process gRPC mock that serves one
/// `proto::ResponseData` chunk and a clean `grpc-status: 0`, then
/// return an `MarketDataClient` wired to that mock via the
/// `__test-helpers`-gated `for_endpoint_routing_test` constructor. The mock
/// handle stays alive as long as the test owning it.
async fn client_for_response(
    response: proto::ResponseData,
) -> (mock::MockServer, MarketDataClient) {
    let server = mock::MockServer::spawn(vec![response], 0).await;
    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect to mock");
    let pool = ChannelPool::from_channels(vec![channel]);
    let cfg = DirectConfig::production();
    let sem = Arc::new(Semaphore::new(4));
    let client = MarketDataClient::for_endpoint_routing_test(cfg, pool, sem);
    (server, client)
}

// ────────────────────────────────────────────────────────────────────
// option_history_greeks_eod -> GreeksEodTick
// ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_greeks_eod_routes_to_greeks_eod_parser() {
    let response = load_response("option_history_greeks_eod");
    let (_mock, client) = client_for_response(response).await;

    // Compile-time type binding: this `let` would not type-check if
    // the dispatch returned `Vec<GreeksAllTick>` (the silent
    // mis-routing target) or any other tick shape.
    let ticks: thetadatadx::Ticks<GreeksEodTick> = client
        .option_history_greeks_eod("SPY", "20240621", "20240614", "20240614")
        .await
        .expect("option_history_greeks_eod via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    // EOD trade-quote columns the earlier routing dropped. Pin values
    // from the captured wire fixture (fixture meta `first_row_*`).
    assert!(
        (first.open - 41.71).abs() < 1e-4,
        "open dropped or wrong (got {})",
        first.open
    );
    assert!(
        (first.high - 42.78).abs() < 1e-4,
        "high dropped or wrong (got {})",
        first.high
    );
    assert!(
        (first.low - 40.48).abs() < 1e-4,
        "low dropped or wrong (got {})",
        first.low
    );
    assert!(
        (first.close - 42.78).abs() < 1e-4,
        "close dropped or wrong (got {})",
        first.close
    );
    assert_eq!(first.volume, 683, "volume dropped or wrong");
    assert_eq!(first.count, 99, "count dropped or wrong");
}

// ────────────────────────────────────────────────────────────────────
// index_at_time_price -> IndexPriceAtTimeTick
// ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn index_at_time_price_routes_to_index_price_at_time_parser() {
    let response = load_response("index_at_time_price");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<IndexPriceAtTimeTick> = client
        .index_at_time_price("SPX", "20240614", "20240614", "16:00:00")
        .await
        .expect("index_at_time_price via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    // Seven trade-side execution columns the earlier routing dropped,
    // including the SIP-source `exchange` attribution. Sequencing /
    // sizing fields are tick-shape fingerprints — any non-trade
    // routing zero-fills them.
    assert!(
        first.sequence != 0 || first.size != 0 || first.exchange != 0,
        "trade-side execution columns all zero — routing reverted to PriceTick"
    );
}

// ────────────────────────────────────────────────────────────────────
// option_history_trade_greeks_* -> TradeGreeks*Tick
//
// Five endpoints whose silent reroute through the interval-sampled
// `Greeks*Tick` parsers dropped the nine trade-side execution
// columns (`sequence`, `ext_condition1..4`, `condition`, `size`,
// `exchange`, `price`). Each test pins both routing (compile-time
// type binding) AND parsing (the trade-side columns survive).
// ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_trade_greeks_all_routes_to_trade_greeks_all_parser() {
    let response = load_response("option_history_trade_greeks_all");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<TradeGreeksAllTick> = client
        .option_history_trade_greeks_all("SPY", "20240621")
        .date("20240614")
        .await
        .expect("option_history_trade_greeks_all via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    assert!(
        first.size != 0 || first.exchange != 0 || first.sequence != 0,
        "trade-side cols all zero — routing reverted to non-trade Greeks parser"
    );
    assert!(
        first.price != 0.0,
        "price column dropped — routing reverted to non-trade Greeks parser"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_trade_greeks_first_order_routes_to_first_order_parser() {
    let response = load_response("option_history_trade_greeks_first_order");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<TradeGreeksFirstOrderTick> = client
        .option_history_trade_greeks_first_order("SPY", "20240621")
        .date("20240614")
        .await
        .expect("option_history_trade_greeks_first_order via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    assert!(
        first.size != 0 || first.exchange != 0 || first.sequence != 0,
        "trade-side cols all zero — routing reverted to non-trade Greeks parser"
    );
    assert!(
        first.price != 0.0,
        "price column dropped — routing reverted to non-trade Greeks parser"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_trade_greeks_second_order_routes_to_second_order_parser() {
    let response = load_response("option_history_trade_greeks_second_order");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<TradeGreeksSecondOrderTick> = client
        .option_history_trade_greeks_second_order("SPY", "20240621")
        .date("20240614")
        .await
        .expect("option_history_trade_greeks_second_order via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    assert!(
        first.size != 0 || first.exchange != 0 || first.sequence != 0,
        "trade-side cols all zero — routing reverted to non-trade Greeks parser"
    );
    assert!(
        first.price != 0.0,
        "price column dropped — routing reverted to non-trade Greeks parser"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_trade_greeks_third_order_routes_to_third_order_parser() {
    let response = load_response("option_history_trade_greeks_third_order");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<TradeGreeksThirdOrderTick> = client
        .option_history_trade_greeks_third_order("SPY", "20240621")
        .date("20240614")
        .await
        .expect("option_history_trade_greeks_third_order via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    assert!(
        first.size != 0 || first.sequence != 0,
        "trade-side cols all zero — routing reverted to non-trade Greeks parser"
    );
    assert!(
        first.price != 0.0,
        "price column dropped — routing reverted to non-trade Greeks parser"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn option_history_trade_greeks_implied_volatility_routes_to_iv_parser() {
    let response = load_response("option_history_trade_greeks_implied_volatility");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<TradeGreeksImpliedVolatilityTick> = client
        .option_history_trade_greeks_implied_volatility("SPY", "20240621")
        .date("20240614")
        .await
        .expect("option_history_trade_greeks_implied_volatility via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    assert!(
        first.size != 0 || first.sequence != 0,
        "trade-side cols all zero — routing reverted to non-trade IV parser"
    );
    assert!(
        first.price != 0.0,
        "price column dropped — routing reverted to non-trade IV parser"
    );
}

// ────────────────────────────────────────────────────────────────────
// End-to-end column projection through the buffered `Ticks<T>` return
//
// The whole point of the projection: a real gRPC response, decoded through
// the direct-client `.await` path, yields a `Ticks<T>` whose `.to_arrow()`
// emits exactly the wire's columns. This exercises the full chain (mock
// transport -> decode -> WireColumns::present_columns at the macro seam ->
// Ticks -> to_arrow_projected), not the builder in isolation.
// ────────────────────────────────────────────────────────────────────

#[cfg(feature = "arrow")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stock_trade_quote_await_projects_columns_end_to_end() {
    use thetadatadx::frames::TicksArrowExt;

    let response = load_response("stock_history_trade_quote");
    let (_mock, client) = client_for_response(response).await;

    let ticks: thetadatadx::Ticks<thetadatadx::TradeQuoteTick> = client
        .stock_history_trade_quote("AAPL")
        .date("20240102")
        .await
        .expect("stock_history_trade_quote via mock");
    assert!(!ticks.is_empty(), "mock served a non-empty fixture");

    // Terminal-exact: the buffered `Ticks::to_arrow` projects to the wire.
    let batch = ticks.to_arrow().expect("projected arrow");
    let cols: Vec<String> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    for absent in [
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ] {
        assert!(
            !cols.contains(&absent.to_string()),
            "stock trade_quote gRPC path must not emit {absent}; got {cols:?}"
        );
    }
    for kept in ["ms_of_day", "quote_ms_of_day", "bid", "ask", "price"] {
        assert!(
            cols.contains(&kept.to_string()),
            "missing {kept} in {cols:?}"
        );
    }

    // The full-schema slice builder still emits every column — the
    // hand-built default is unchanged by the projection.
    let full = ticks.as_slice().to_arrow().expect("full arrow");
    let full_cols: Vec<String> = full
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert!(
        full_cols.contains(&"condition_flags".to_string()),
        "full slice builder must keep every column; got {full_cols:?}"
    );
}

// List endpoints honor the per-request deadline
//
// List RPCs (`*_list_*`) must apply the same deadline contract as the
// builder endpoints: the configured `request_timeout_secs` bounds the
// plain method, and the generated `<name>_with_deadline` overload accepts
// an explicit per-call deadline. Without it a live-but-silent stream hangs
// the request forever (the gRPC keepalive PING only detects a fully dead
// peer, not a stalled one).
// ────────────────────────────────────────────────────────────────────

use std::time::Duration;

use thetadatadx::Error;

/// Build a `MarketDataClient` wired to a mock that delays its response by
/// `pre_response_delay`, with `request_timeout_secs` set on the config so
/// the default-deadline path can be exercised deterministically.
async fn client_for_delayed_mock(
    response: proto::ResponseData,
    pre_response_delay: Duration,
    request_timeout_secs: u64,
) -> (mock::MockServer, MarketDataClient) {
    let server = mock::MockServer::spawn_with_behaviour(
        vec![response],
        0,
        String::new(),
        mock::MockBehaviour {
            pre_response_delay: Some(pre_response_delay),
            ..mock::MockBehaviour::default()
        },
    )
    .await;
    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect to mock");
    let pool = ChannelPool::from_channels(vec![channel]);
    let mut cfg = DirectConfig::production();
    cfg.market_data.request_timeout_secs = request_timeout_secs;
    let sem = Arc::new(Semaphore::new(4));
    let client = MarketDataClient::for_endpoint_routing_test(cfg, pool, sem);
    (server, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_endpoint_plain_call_is_bounded_by_configured_default() {
    // The mock holds the response 5s; the configured default deadline is
    // 1s. The plain `stock_list_symbols()` (no explicit deadline) must
    // surface `Error::Timeout` from the configured default rather than
    // hang on the silent stream.
    let (_mock, client) = client_for_delayed_mock(
        mock::make_response_data(&["AAPL"]),
        Duration::from_secs(5),
        1,
    )
    .await;

    let result = client.stock_list_symbols().await;
    match result {
        Err(Error::Timeout { duration_ms }) => {
            assert!(
                duration_ms <= 1_000,
                "default deadline carried the wrong bound; got {duration_ms}ms"
            );
        }
        other => panic!("expected Error::Timeout from the configured default, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_endpoint_with_deadline_overload_bounds_the_call() {
    // The generated `<name>_with_deadline` overload exists (compile-time)
    // and an explicit short deadline wins: the mock delays 5s, the call
    // bounds at 100ms. `request_timeout_secs` is left high to prove the
    // explicit per-call deadline — not the default — drives the timeout.
    let (_mock, client) = client_for_delayed_mock(
        mock::make_response_data(&["AAPL"]),
        Duration::from_secs(5),
        300,
    )
    .await;

    let result = client
        .stock_list_symbols_with_deadline(Duration::from_millis(100))
        .await;
    match result {
        Err(Error::Timeout { duration_ms }) => {
            assert!(
                duration_ms <= 100,
                "explicit deadline carried the wrong bound; got {duration_ms}ms"
            );
        }
        other => panic!("expected Error::Timeout from the explicit deadline, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_endpoint_unset_deadline_applies_configured_default() {
    // The four generated `*_stream` builders must route an unset deadline
    // through `effective_deadline`, exactly like the parsed builders: with
    // no explicit `with_deadline`, the configured `request_timeout_secs`
    // (1s here) must bound the call so a live-but-silent server cannot hang
    // it forever and starve the request-semaphore. The mock holds the
    // response 5s.
    let (_mock, client) = client_for_delayed_mock(
        mock::make_response_data(&["AAPL"]),
        Duration::from_secs(5),
        1,
    )
    .await;

    let result = client
        .stock_history_trade_stream("AAPL")
        .date("20240102")
        .stream(|_ticks| {})
        .await;
    match result {
        Err(Error::Timeout { duration_ms }) => {
            assert!(
                duration_ms <= 1_000,
                "unset-deadline stream applied the wrong default bound; got {duration_ms}ms"
            );
        }
        other => panic!("expected Error::Timeout from the configured default, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_endpoint_completes_within_deadline() {
    // A prompt response well inside the deadline returns `Ok`, proving the
    // deadline wrapper is transparent on the success path (the in-flight
    // call completes and is not cancelled). The mock responds immediately,
    // far inside the 10s bound.
    let (_mock, client) = client_for_delayed_mock(
        mock::make_response_data(&["AAPL", "MSFT"]),
        Duration::from_millis(0),
        300,
    )
    .await;

    let result = client
        .stock_list_symbols_with_deadline(Duration::from_secs(10))
        .await;
    assert!(
        result.is_ok(),
        "prompt list call must complete within the deadline, got {result:?}"
    );
}
