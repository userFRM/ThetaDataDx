//! MDDS (Market Data Distribution Server) gRPC client.
//!
//! [`MddsClient`] authenticates against the Nexus HTTP API, opens a gRPC
//! channel to the MDDS server, and exposes typed methods for every historical
//! data endpoint. Macro-driven builder patterns (`list_endpoint!`,
//! `parsed_endpoint!`) live in [`crate::macros`] and are applied here via
//! generated code (`include!`) from `endpoint_surface.toml`.
//!
//! # Architecture
//!
//! ```text
//! Credentials --> nexus::authenticate() --> AuthResponse.session_id
//!                                              |
//!              +-------------------------------+
//!              |
//!       MddsClient
//!        |-- mdds_stub: BetaThetaTerminalClient  (gRPC, historical data)
//!        \-- session_uuid: String                (UUID in every QueryInfo)
//! ```
//!
//! Every MDDS request wraps parameters in a `QueryInfo` that carries the session
//! UUID obtained from Nexus auth. Responses are `stream ResponseData` — zstd-
//! compressed `DataTable` payloads decoded by [`crate::decode`].
//!
//! # Layout
//!
//! This module mirrors the [`crate::fpss`] layout for symmetry between the two
//! upstream services:
//!
//! - [`client`] — `MddsClient` struct, `connect`, transport/session state
//! - [`stream`] — gRPC response stream helpers (`collect_stream`, `for_each_chunk`)
//! - [`validate`] — runtime parameter validators invoked by generated macros
//! - [`endpoints`] — generated endpoint method bodies (`include!` sites);
//!   wire-format canonicalizers (`normalize_interval`, `normalize_time_of_day`,
//!   `contract_spec!`) live at the top of that file. The cross-cutting wire
//!   helpers (`normalize_expiration`, `wire_strike_opt`, `wire_right_opt`)
//!   live in [`crate::wire_semantics`].

mod client;
mod endpoints;
mod stream;
mod validate;

pub use client::MddsClient;

#[cfg(test)]
mod tests {
    use crate::decode;
    use crate::proto;

    #[test]
    fn parse_eod_handles_empty_table() {
        let table = proto::DataTable {
            headers: vec!["ms_of_day".into(), "open".into(), "date".into()],
            data_table: vec![],
        };
        let ticks = decode::parse_eod_ticks(&table).unwrap();
        assert!(ticks.is_empty());
    }

    #[test]
    fn parse_eod_handles_number_typed_columns() {
        let table = proto::DataTable {
            headers: vec![
                "ms_of_day".into(),
                "open".into(),
                "close".into(),
                "date".into(),
            ],
            data_table: vec![proto::DataValueList {
                values: vec![
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(34200000)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(15000)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(15100)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(20240301)),
                    },
                ],
            }],
        };
        let ticks = decode::parse_eod_ticks(&table).unwrap();
        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].ms_of_day, 34200000);
        assert!((ticks[0].open - 15000.0).abs() < 1e-10);
        assert!((ticks[0].close - 15100.0).abs() < 1e-10);
        assert_eq!(ticks[0].date, 20240301);
    }
}
