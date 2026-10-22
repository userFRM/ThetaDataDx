//! Optional query parameters must reach the wire as the terminal sends them.
//!
//! Several parameters are optional on the vendor's query messages, yet the
//! terminal fills a value in on every request: 101 `binomial_steps`,
//! `perf_boost_intraday = false`, `current_day_only = false`,
//! `option_use_nbbo = true` and `use_time_based_trade_quote = true`. An
//! omitted field is therefore not the same request the terminal makes, and
//! the difference is invisible to the caller: the server applies whatever
//! default it holds and answers normally. The values come from a declared default in
//! `endpoint_surface.toml`, so losing one leaves the surface compiling, the
//! parity gate green and the request quietly different.
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

/// Every request message carries its query as field 2. Reading it as bytes
/// lets each check below decode only the slice of the query it reads.
#[derive(Clone, PartialEq, prost::Message)]
struct CapturedRequest {
    #[prost(bytes = "vec", optional, tag = "2")]
    params: Option<Vec<u8>>,
}

// The request protos are `pub(crate)`, so the tags of the fields read back are
// mirrored here; a tag that drifts from the vendor's contract decodes to an
// absent field and fails the assertions below rather than passing quietly.

/// `OptionHistoryBinomialTradeGreeksAllRequestQuery`.
#[derive(Clone, PartialEq, prost::Message)]
struct BinomialTradeGreeksQuery {
    #[prost(int32, optional, tag = "9")]
    binomial_steps: Option<i32>,
    #[prost(bool, optional, tag = "15")]
    perf_boost_intraday: Option<bool>,
}

/// `OptionHistoryTradeGreeksAllRequestQuery`: `perf_boost_intraday` sits one
/// tag lower, having no `binomial_steps` before it.
#[derive(Clone, PartialEq, prost::Message)]
struct TradeGreeksQuery {
    #[prost(bool, optional, tag = "14")]
    perf_boost_intraday: Option<bool>,
}

/// `OptionSnapshotQuoteRequestQuery`.
#[derive(Clone, PartialEq, prost::Message)]
struct OptionSnapshotQuoteQuery {
    #[prost(bool, optional, tag = "6")]
    current_day_only: Option<bool>,
}

/// `OptionSnapshotGreeksAllRequestQuery`.
#[derive(Clone, PartialEq, prost::Message)]
struct OptionSnapshotGreeksAllQuery {
    #[prost(double, optional, tag = "13")]
    annual_dividend_amount: Option<f64>,
}

/// `OptionHistoryGreeksEodRequestQuery`.
#[derive(Clone, PartialEq, prost::Message)]
struct OptionHistoryGreeksEodQuery {
    #[prost(bool, optional, tag = "12")]
    option_use_nbbo: Option<bool>,
}

/// `StockHistoryTradeQuoteRequestQuery`.
#[derive(Clone, PartialEq, prost::Message)]
struct StockHistoryTradeQuoteQuery {
    #[prost(bool, optional, tag = "9")]
    use_time_based_trade_quote: Option<bool>,
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

/// Decode the query the client wrote as `Q`.
fn sent_query<Q: Message + Default>(captured: &Arc<Mutex<Vec<u8>>>) -> Q {
    let bytes = captured.lock().expect("capture lock").clone();
    assert!(!bytes.is_empty(), "mock captured no request body");
    let params = CapturedRequest::decode(bytes.as_slice())
        .expect("captured bytes decode as a request")
        .params
        .expect("request carries its params");
    Q::decode(params.as_slice()).expect("params decode as the expected query")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_parameters_carry_the_terminal_defaults_or_the_callers_values() {
    // binomial_steps and perf_boost_intraday unset: the terminal's values.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_binomial_trade_greeks_all("SPY", "20250321")
        .await;
    let query: BinomialTradeGreeksQuery = sent_query(&captured);
    assert_eq!(query.binomial_steps, Some(101));
    assert_eq!(query.perf_boost_intraday, Some(false));

    // Both set: the caller's values instead.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_binomial_trade_greeks_all("SPY", "20250321")
        .binomial_steps(51)
        .perf_boost_intraday(true)
        .await;
    let query: BinomialTradeGreeksQuery = sent_query(&captured);
    assert_eq!(query.binomial_steps, Some(51));
    assert_eq!(query.perf_boost_intraday, Some(true));

    // The non-binomial trade greeks route carries the same flag.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_trade_greeks_all("SPY", "20250321")
        .await;
    let query: TradeGreeksQuery = sent_query(&captured);
    assert_eq!(query.perf_boost_intraday, Some(false));

    // current_day_only on a snapshot route, unset and then set.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client.option_snapshot_quote("SPY", "20250321").await;
    let query: OptionSnapshotQuoteQuery = sent_query(&captured);
    assert_eq!(query.current_day_only, Some(false));

    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_snapshot_quote("SPY", "20250321")
        .current_day_only(true)
        .await;
    let query: OptionSnapshotQuoteQuery = sent_query(&captured);
    assert_eq!(query.current_day_only, Some(true));

    // annual_dividend_amount has no terminal default: the terminal sends it
    // only when the caller set it, and the server refuses it beside a
    // non-zero annual_dividend, so an unset value must stay off the wire.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client.option_snapshot_greeks_all("SPY", "20250321").await;
    let query: OptionSnapshotGreeksAllQuery = sent_query(&captured);
    assert_eq!(query.annual_dividend_amount, None);

    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_snapshot_greeks_all("SPY", "20250321")
        .annual_dividend_amount(0.96)
        .await;
    let query: OptionSnapshotGreeksAllQuery = sent_query(&captured);
    assert_eq!(query.annual_dividend_amount, Some(0.96));

    // option_use_nbbo on the end-of-day Greeks route.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .option_history_greeks_eod("SPY", "20250321", "20250303", "20250303")
        .await;
    let query: OptionHistoryGreeksEodQuery = sent_query(&captured);
    assert_eq!(query.option_use_nbbo, Some(true));

    // use_time_based_trade_quote on the trade_quote route.
    let (_mock, client, captured) = client_capturing_request().await;
    let _ = client
        .stock_history_trade_quote("AAPL")
        .date("20250303")
        .await;
    let query: StockHistoryTradeQuoteQuery = sent_query(&captured);
    assert_eq!(query.use_time_based_trade_quote, Some(true));
}
