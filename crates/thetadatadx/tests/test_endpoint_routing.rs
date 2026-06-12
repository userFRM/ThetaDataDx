//! Endpoint-to-parser routing regression tests.
//!
//! The sibling `test_eod_greeks_schema.rs` and
//! `test_trade_greeks_schema.rs` suites prove that the per-tick
//! *parsers* (`parse_greeks_eod_ticks`, `parse_trade_greeks_*_ticks`,
//! `parse_index_price_at_time_ticks`) preserve every wire column when
//! fed a hand-built `DataTable`. They do NOT prove that the high-level
//! `MddsClient::<endpoint>` dispatch method routes the response through
//! that parser -- the heuristic that picks `parse: decode::parse_<tick>`
//! for each endpoint inside `build_support/endpoints/proto_parser.rs`
//! can drift silently and the per-parser regressions would still pass.
//!
//! These tests close the routing gap end-to-end. For each of the
//! seven endpoints whose silent mis-routing previously dropped the
//! trade-side execution / EOD trade-quote columns, the harness:
//!
//!   1. Loads the verified-live capture fixture as a raw
//!      `proto::ResponseData`.
//!   2. Spins up the in-process `grpc_mock_server` mock and serves
//!      that single `ResponseData` chunk under the gRPC stub path
//!      the real `MddsClient::<endpoint>` builder dispatches to.
//!   3. Builds an `MddsClient` against the mock via the
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
use thetadatadx::mdds::MddsClient;
use thetadatadx::wire as proto;
use thetadatadx::DirectConfig;

#[path = "grpc_mock_server.rs"]
mod mock;

#[path = "common/capture_loader.rs"]
mod capture_loader;

use capture_loader::load_response_data as load_response;

/// Stand up an in-process gRPC mock that serves one
/// `proto::ResponseData` chunk and a clean `grpc-status: 0`, then
/// return an `MddsClient` wired to that mock via the
/// `__test-helpers`-gated `for_endpoint_routing_test` constructor. The mock
/// handle stays alive as long as the test owning it.
async fn client_for_response(response: proto::ResponseData) -> (mock::MockServer, MddsClient) {
    let server = mock::MockServer::spawn(vec![response], 0).await;
    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect to mock");
    let pool = ChannelPool::from_channels(vec![channel]);
    let cfg = DirectConfig::production();
    let sem = Arc::new(Semaphore::new(4));
    let client = MddsClient::for_endpoint_routing_test(cfg, pool, sem);
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
    let ticks: Vec<GreeksEodTick> = client
        .option_history_greeks_eod("SPY", "20240621", "20240614", "20240614")
        .await
        .expect("option_history_greeks_eod via mock");

    assert!(!ticks.is_empty(), "mock served a non-empty fixture");
    let first = ticks.first().expect("non-empty ticks");
    // EOD trade-quote columns the earlier routing dropped. Pin values
    // from the verified-live capture (terminal jar `202605221`,
    // fixture meta `first_row_*`).
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

    let ticks: Vec<IndexPriceAtTimeTick> = client
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

    let ticks: Vec<TradeGreeksAllTick> = client
        .option_history_trade_greeks_all("SPY", "20240621", "20240614")
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

    let ticks: Vec<TradeGreeksFirstOrderTick> = client
        .option_history_trade_greeks_first_order("SPY", "20240621", "20240614")
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

    let ticks: Vec<TradeGreeksSecondOrderTick> = client
        .option_history_trade_greeks_second_order("SPY", "20240621", "20240614")
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

    let ticks: Vec<TradeGreeksThirdOrderTick> = client
        .option_history_trade_greeks_third_order("SPY", "20240621", "20240614")
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

    let ticks: Vec<TradeGreeksImpliedVolatilityTick> = client
        .option_history_trade_greeks_implied_volatility("SPY", "20240621", "20240614")
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
