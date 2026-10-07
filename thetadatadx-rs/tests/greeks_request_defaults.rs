//! `binomial_steps` and `perf_boost_intraday` must reach the wire as the
//! terminal sends them.
//!
//! Both parameters are optional on the vendor's query messages, and the
//! terminal fills a value in for each on every request: 101 steps, and
//! `perf_boost_intraday = false`. An omitted field is therefore not the same
//! request the terminal makes, and the difference is invisible to the caller:
//! the server applies whatever default it holds and answers normally. The
//! values come from a declared default in `endpoint_surface.toml`, so losing
//! one leaves the surface compiling, the parity gate green and the request
//! quietly different.
//!
//! The mock captures the request body the client actually wrote, so the
//! assertions are on the encoded query message rather than on a value the
//! builder happens to hold.

#![cfg(feature = "__test-helpers")]

use std::sync::{Arc, Mutex};

use prost::Message;
use tokio::sync::Semaphore;

use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::mdds::MarketDataClient;
use thetadatadx::DirectConfig;

/// The slice of `OptionHistoryBinomialTradeGreeksAllRequestQuery` this test
/// reads back. The request protos are `pub(crate)`, so the tags are mirrored
/// here; a tag that drifts from the vendor's contract decodes to an absent
/// field and fails the assertions below rather than passing quietly.
#[derive(Clone, PartialEq, prost::Message)]
struct CapturedBinomialTradeGreeksQuery {
    #[prost(int32, optional, tag = "9")]
    binomial_steps: Option<i32>,
    #[prost(bool, optional, tag = "15")]
    perf_boost_intraday: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CapturedBinomialTradeGreeksRequest {
    #[prost(message, optional, tag = "2")]
    params: Option<CapturedBinomialTradeGreeksQuery>,
}

/// The slice of `OptionHistoryTradeGreeksAllRequestQuery` this test reads
/// back: the non-binomial route carries `perf_boost_intraday` one tag lower,
/// having no `binomial_steps` before it.
#[derive(Clone, PartialEq, prost::Message)]
struct CapturedTradeGreeksQuery {
    #[prost(bool, optional, tag = "14")]
    perf_boost_intraday: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CapturedTradeGreeksRequest {
    #[prost(message, optional, tag = "2")]
    params: Option<CapturedTradeGreeksQuery>,
}

#[path = "common/grpc_mock.rs"]
pub mod mock;

/// Serve one empty response and hand back the request bytes the client wrote.
async fn client_capturing_request() -> (mock::MockServer, MarketDataClient, Arc<Mutex<Vec<u8>>>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let behaviour = mock::MockBehaviour {
        capture_request_bytes: Some(Arc::clone(&captured)),
        ..mock::MockBehaviour::default()
    };
    let server = mock::MockServer::spawn_with_behaviour(
        vec![mock::make_response_data(&[])],
        0,
        String::new(),
        behaviour,
    )
    .await;
    let channel = Channel::connect_h2c("127.0.0.1", server.addr.port())
        .await
        .expect("h2c connect to mock");
    let pool = ChannelPool::from_channels(vec![channel]);
    let client = MarketDataClient::for_endpoint_routing_test(
        DirectConfig::production(),
        pool,
        Arc::new(Semaphore::new(4)),
    );
    (server, client, captured)
}

fn captured_bytes(captured: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
    let bytes = captured.lock().expect("capture lock").clone();
    assert!(!bytes.is_empty(), "mock captured no request body");
    bytes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn binomial_steps_and_perf_boost_carry_the_terminal_defaults_or_the_callers_values() {
    // Neither parameter set: the terminal's values must still reach the wire.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_binomial_trade_greeks_all("SPY", "20250321")
        .await;
    let params = CapturedBinomialTradeGreeksRequest::decode(captured_bytes(&captured).as_slice())
        .expect("captured bytes decode as the binomial trade greeks request")
        .params
        .expect("request carries its params");
    assert_eq!(
        params.binomial_steps,
        Some(101),
        "default binomial_steps did not reach the wire"
    );
    assert_eq!(
        params.perf_boost_intraday,
        Some(false),
        "default perf_boost_intraday did not reach the wire"
    );

    // Both parameters set: the caller's values must reach the wire instead.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_binomial_trade_greeks_all("SPY", "20250321")
        .binomial_steps(51)
        .perf_boost_intraday(true)
        .await;
    let params = CapturedBinomialTradeGreeksRequest::decode(captured_bytes(&captured).as_slice())
        .expect("captured bytes decode as the binomial trade greeks request")
        .params
        .expect("request carries its params");
    assert_eq!(
        params.binomial_steps,
        Some(51),
        "caller's binomial_steps did not reach the wire"
    );
    assert_eq!(
        params.perf_boost_intraday,
        Some(true),
        "caller's perf_boost_intraday did not reach the wire"
    );

    // The non-binomial trade greeks route carries the same flag, one tag lower.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_trade_greeks_all("SPY", "20250321")
        .await;
    let params = CapturedTradeGreeksRequest::decode(captured_bytes(&captured).as_slice())
        .expect("captured bytes decode as the trade greeks request")
        .params
        .expect("request carries its params");
    assert_eq!(
        params.perf_boost_intraday,
        Some(false),
        "default perf_boost_intraday did not reach the wire"
    );
}
