//! The `option_list_dates` filters must reach the wire.
//!
//! The vendor narrows this route by `strike` and `right`, and the terminal
//! returns only the matching dates. This SDK sent the contract spec with
//! `strike = "*"` and `right = "both"` pinned, whatever the caller asked for,
//! so a filtered request came back as the unfiltered list.
//!
//! The mock captures the request body the client actually wrote, so the
//! assertion is on the encoded `ContractSpec`, not on a value the builder
//! happens to hold.

#![cfg(feature = "__test-helpers")]

use std::sync::{Arc, Mutex};

use prost::Message;
use tokio::sync::Semaphore;

use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::mdds::MarketDataClient;
use thetadatadx::DirectConfig;

/// The slice of `OptionListDatesRequest` this test reads back.
///
/// The request protos are `pub(crate)`, and the vendor's `.proto` carries no
/// comments on these two messages, so re-exporting them for the test would
/// trip `missing_docs` on generated code. These mirror the wire tags instead.
/// A tag that drifts from the vendor's contract decodes to an empty field and
/// fails the assertions below rather than passing quietly.
#[derive(Clone, PartialEq, prost::Message)]
struct CapturedContractSpec {
    #[prost(string, tag = "1")]
    symbol: String,
    #[prost(string, tag = "2")]
    expiration: String,
    #[prost(string, optional, tag = "3")]
    strike: Option<String>,
    #[prost(string, optional, tag = "4")]
    right: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CapturedQuery {
    #[prost(string, tag = "1")]
    request_type: String,
    #[prost(message, optional, tag = "2")]
    contract_spec: Option<CapturedContractSpec>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CapturedRequest {
    #[prost(message, optional, tag = "2")]
    params: Option<CapturedQuery>,
}

#[path = "grpc_mock_server.rs"]
mod mock;

/// Serve one empty response and hand back the request bytes the client wrote.
async fn client_capturing_request() -> (mock::MockServer, MarketDataClient, Arc<Mutex<Vec<u8>>>) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let behaviour = mock::MockBehaviour {
        capture_request_bytes: Some(Arc::clone(&captured)),
        ..mock::MockBehaviour::default()
    };
    let server = mock::MockServer::spawn_with_behaviour(
        vec![mock::make_response_data(&["2025-03-03"])],
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

/// Decode the captured request into its contract spec.
fn captured_contract_spec(captured: &Arc<Mutex<Vec<u8>>>) -> CapturedContractSpec {
    let bytes = captured.lock().expect("capture lock").clone();
    assert!(!bytes.is_empty(), "mock captured no request body");
    let request = CapturedRequest::decode(bytes.as_slice())
        .expect("captured bytes decode as the list-dates request");
    let params = request.params.expect("request carries its params");
    assert_eq!(params.request_type, "trade", "request_type was not sent");
    params.contract_spec.expect("params carry a contract spec")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_strike_and_right_filter_reach_the_wire() {
    let (_mock, client, captured) = client_capturing_request().await;

    let _ = client
        .option_list_dates("trade", "SPY", "20250321")
        .strike("580")
        .right("call")
        .await;

    let spec = captured_contract_spec(&captured);
    assert_eq!(spec.strike.as_deref(), Some("580"), "strike was not sent");
    assert_eq!(spec.right.as_deref(), Some("call"), "right was not sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfiltered_call_still_sends_the_vendor_defaults() {
    let (_mock, client, captured) = client_capturing_request().await;

    let _ = client.option_list_dates("trade", "SPY", "20250321").await;

    let spec = captured_contract_spec(&captured);
    assert_eq!(spec.strike.as_deref(), Some("*"), "default strike changed");
    assert_eq!(spec.right.as_deref(), Some("both"), "default right changed");
}
