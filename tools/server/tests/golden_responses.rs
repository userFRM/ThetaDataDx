//! The server's answer on every route, in every format, against committed bytes.
//!
//! Each case in `golden/corpus.txt` names an endpoint and a query. Its recorded
//! upstream response (`golden/<case>/response.pb`, plus `golden/<case>/status`
//! when the upstream refused the request) is served by an in-process gRPC mock,
//! and the request runs through the server's own path: validation, the real
//! dispatch and decode, and the renderer. The status, headers and body must
//! equal `golden/<case>/<format>`. [`EDGES`] adds tables live data rarely shows,
//! each derived from a recorded case.
//!
//! A registry route's JSON bodies are compared with each object's members
//! sorted by key, as the expected files store them; every other byte, and
//! every other body, is compared as written.
//!
//! A flat file's `<endpoint>_to_path` call in the SDK must write exactly the
//! body its route serves, in every format, and leave nothing behind when the
//! download fails.
//!
//! `GOLDEN_UPDATE=1 cargo test golden` rewrites the expected files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prost::Message;
use thetadatadx::grpc::{Channel, ChannelPool};
use thetadatadx::wire::{
    CompressionAlgo, CompressionDescription, DataTable, DataValue, ResponseData, data_value,
};
use thetadatadx::{DirectConfig, EndpointMeta, MarketDataClient};

use crate::{flatfile_routes, handler};

#[path = "../../../thetadatadx-rs/tests/common/grpc_mock.rs"]
pub mod mock;

/// Every value the server's `format` parameter maps to a distinct renderer.
const FORMATS: [&str; 5] = ["csv", "json", "json_legacy", "ndjson", "html"];

/// A change to each chunk of a recorded response.
type Edit = fn(&mut DataTable);

/// `(case, recorded case it is derived from, edit)`.
const EDGES: &[(&str, &str, Edit)] = &[
    // A null in a column the vendor never leaves empty.
    (
        "stock_history_eod_null_volume",
        "stock_history_eod",
        |table| {
            let volume = column(table, "volume");
            table.data_table[0].values[volume] = null();
        },
    ),
    // A nullable column the response does not carry.
    (
        "stock_history_quote_without_bid_condition",
        "stock_history_quote",
        |table| {
            let condition = column(table, "bid_condition");
            table.headers.remove(condition);
            for row in &mut table.data_table {
                row.values.remove(condition);
            }
        },
    ),
    // Headers and no rows.
    (
        "stock_history_trade_empty",
        "stock_history_trade",
        |table| {
            table.data_table.clear();
        },
    ),
    // A list with a null cell.
    (
        "option_list_expirations_null",
        "option_list_expirations",
        |table| {
            table.data_table[1].values[0] = null();
        },
    ),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn golden_responses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let update = std::env::var_os("GOLDEN_UPDATE").is_some();
    let corpus = std::fs::read_to_string(dir.join("corpus.txt")).expect("read corpus");
    let recorded: Vec<(&str, &str, &str)> = corpus
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split_whitespace();
            let case = fields.next().expect("case");
            let endpoint = fields
                .next()
                .unwrap_or_else(|| panic!("{case}: no endpoint"));
            (case, endpoint, fields.next().unwrap_or(""))
        })
        .collect();
    let uncovered: Vec<&str> = thetadatadx::ENDPOINTS
        .iter()
        .map(|ep| ep.name)
        .filter(|name| !recorded.iter().any(|&(_, endpoint, _)| endpoint == *name))
        .collect();
    assert!(
        uncovered.is_empty(),
        "routes with no case in corpus.txt: {uncovered:?}"
    );

    let mut cases = Vec::new();
    for &(case, endpoint, query) in &recorded {
        cases.push((case, endpoint, query, Recording::load(&dir.join(case))));
    }
    for &(case, base, edit) in EDGES {
        let &(_, endpoint, query) = recorded
            .iter()
            .find(|(name, ..)| *name == base)
            .unwrap_or_else(|| panic!("{case}: no recorded case {base}"));
        let mut recording = Recording::load(&dir.join(base));
        recording.chunks = recording
            .chunks
            .iter()
            .map(|chunk| {
                let mut table = thetadatadx::decode::decode_data_table(&mut chunk.clone())
                    .unwrap_or_else(|e| panic!("{case}: decode {base}: {e}"));
                edit(&mut table);
                let compressed_data = table.encode_to_vec();
                ResponseData {
                    compression_description: Some(CompressionDescription {
                        algo: CompressionAlgo::None as i32,
                        level: 0,
                    }),
                    original_size: i32::try_from(compressed_data.len()).expect("small table"),
                    compressed_data,
                }
            })
            .collect();
        cases.push((case, endpoint, query, recording));
    }

    let mut mismatches = Vec::new();
    for (case, endpoint, query, recording) in &cases {
        let ep = thetadatadx::find(endpoint).unwrap_or_else(|| panic!("{case}: no {endpoint}"));
        for (format, actual) in FORMATS.into_iter().zip(answers(ep, query, recording).await) {
            let path = dir.join(case).join(format);
            if update {
                std::fs::create_dir_all(dir.join(case)).expect("create case directory");
                std::fs::write(&path, &actual).expect("write golden file");
            } else if std::fs::read_to_string(&path).ok().as_deref() != Some(actual.as_str()) {
                mismatches.push(format!("{case} {format} ({})", path.display()));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} responses differ from their golden files (GOLDEN_UPDATE=1 rewrites them):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

/// What the upstream answered one case's request with.
struct Recording {
    chunks: Vec<ResponseData>,
    /// gRPC status code, the service's `http_status_code` (0 when absent) and
    /// message; `0 0` and no message for a clean end.
    status: (u32, u16, String),
}

impl Recording {
    fn load(case_dir: &Path) -> Self {
        let read = |name: &str| -> PathBuf { case_dir.join(name) };
        let bytes = std::fs::read(read("response.pb"))
            .unwrap_or_else(|e| panic!("read {}: {e}", read("response.pb").display()));
        let mut rest = bytes.as_slice();
        let mut chunks = Vec::new();
        while !rest.is_empty() {
            chunks.push(ResponseData::decode_length_delimited(&mut rest).expect("recorded chunk"));
        }
        let status = match std::fs::read_to_string(read("status")) {
            Ok(text) => {
                let (codes, message) = text.split_once('\n').expect("status has two lines");
                let (code, http) = codes.split_once(' ').expect("status codes");
                (
                    code.parse().expect("gRPC code"),
                    http.parse().expect("HTTP status"),
                    message.trim_end().to_owned(),
                )
            }
            Err(_) => (0, 0, String::new()),
        };
        Self { chunks, status }
    }
}

/// The server's answer to `query` in each of [`FORMATS`] with `recording`
/// upstream, each as the status line, every header, a blank line and the body.
///
/// One mock connection serves every format: new connections are paced by a
/// process-wide budget, so one per answer would take minutes.
async fn answers(ep: &'static EndpointMeta, query: &str, recording: &Recording) -> Vec<String> {
    let (code, http_status_code, message) = recording.status.clone();
    let behaviour = mock::MockBehaviour {
        http_status_code: (http_status_code != 0).then_some(http_status_code),
        ..mock::MockBehaviour::default()
    };
    let upstream =
        mock::MockServer::spawn_with_behaviour(recording.chunks.clone(), code, message, behaviour)
            .await;
    let channel = Channel::connect_h2c("127.0.0.1", upstream.addr.port())
        .await
        .expect("connect to the mock");
    let client = Arc::new(MarketDataClient::for_endpoint_routing_test(
        DirectConfig::production(),
        ChannelPool::from_channels(vec![channel]),
        Arc::new(tokio::sync::Semaphore::new(1)),
    ));
    let mut answers = Vec::new();
    for format in FORMATS {
        answers.push(answer(&client, ep, query, format).await);
    }
    if ep.subcategory == "flat_file" {
        for (format, answer) in FORMATS.into_iter().zip(&answers) {
            let served = answer
                .starts_with("200 OK\n")
                .then(|| answer.split_once("\n\n").expect("headers end").1);
            assert_eq!(
                written(&client, ep, query, format).await.as_deref(),
                served,
                "{} {format}: the file written is not the file served",
                ep.name
            );
        }
    }
    answers
}

/// The file the SDK writes for a flat file in `format`, or `None` when the
/// call fails. Either way the call leaves nothing else behind.
async fn written(
    client: &MarketDataClient,
    ep: &EndpointMeta,
    query: &str,
    format: &str,
) -> Option<String> {
    let dir = std::env::temp_dir().join(format!(
        "golden-{}-{}-{format}",
        std::process::id(),
        ep.name
    ));
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let date = query
        .strip_prefix("date=")
        .expect("a flat file takes a date");
    let format = format.parse().expect("a flat-file format");
    let file = match client
        .flat_file_to_path(ep.name, date, &dir.join("file"), format)
        .await
    {
        Ok(path) => Some(std::fs::read_to_string(path).expect("read the written file")),
        Err(_) => None,
    };
    let left = std::fs::read_dir(&dir)
        .expect("list scratch directory")
        .count();
    std::fs::remove_dir_all(&dir).expect("remove scratch directory");
    assert_eq!(
        left,
        usize::from(file.is_some()),
        "{}: a call leaves its file and nothing else",
        ep.name
    );
    file
}

async fn answer(
    client: &Arc<MarketDataClient>,
    ep: &'static EndpointMeta,
    query: &str,
    format: &str,
) -> String {
    let pairs: Vec<(String, String)> =
        serde_urlencoded::from_str(&format!("{query}&format={format}")).expect("query");
    let params = handler::fold_query_pairs(ep, pairs);
    let response = if ep.subcategory == "flat_file" {
        flatfile_routes::respond(Arc::clone(client), ep, params).await
    } else {
        handler::respond(Arc::clone(client), ep, &params).await
    };

    let mut out = format!("{}\n", response.status());
    for (name, value) in response.headers() {
        let _ = writeln!(out, "{name}: {}", value.to_str().expect("ASCII header"));
    }
    out.push('\n');
    let json = ep.subcategory != "flat_file"
        && response.status().is_success()
        && (format.starts_with("json") || format == "ndjson");
    match axum::body::to_bytes(response.into_body(), usize::MAX).await {
        Ok(body) => {
            let body = std::str::from_utf8(&body).expect("UTF-8 body");
            if json {
                for line in body.split_inclusive('\n') {
                    let value = line.trim_end_matches('\n');
                    out.push_str(&sorted_members(value));
                    out.push_str(&line[value.len()..]);
                }
            } else {
                out.push_str(body);
            }
        }
        Err(error) => {
            let _ = write!(out, "<body aborted: {error}>");
        }
    }
    out
}

/// `json` with each object's members sorted by key, every value spelled as
/// written.
fn sorted_members(json: &str) -> String {
    if json.starts_with('{') {
        let mut members: Vec<(String, String)> = sonic_rs::to_object_iter(json)
            .map(|member| {
                let (key, value) = member.expect("JSON object member");
                (
                    sonic_rs::to_string(&*key).expect("JSON key"),
                    sorted_members(value.as_raw_str()),
                )
            })
            .collect();
        members.sort();
        let members: Vec<String> = members.into_iter().map(|(k, v)| k + ":" + &v).collect();
        format!("{{{}}}", members.join(","))
    } else if json.starts_with('[') {
        let items: Vec<String> = sonic_rs::to_array_iter(json)
            .map(|item| sorted_members(item.expect("JSON array item").as_raw_str()))
            .collect();
        format!("[{}]", items.join(","))
    } else {
        json.to_owned()
    }
}

fn column(table: &DataTable, name: &str) -> usize {
    table
        .headers
        .iter()
        .position(|header| header == name)
        .unwrap_or_else(|| panic!("no {name} column in {:?}", table.headers))
}

fn null() -> DataValue {
    DataValue {
        data_type: Some(data_value::DataType::NullValue(0)),
    }
}
