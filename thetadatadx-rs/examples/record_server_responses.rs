//! Record the upstream responses the local server's golden test replays.
//!
//! Usage:
//!     cargo run -p thetadatadx-rs --features __test-helpers \
//!         --example record_server_responses -- <creds file> <golden dir> [case ...]
//!
//! Reads `<golden dir>/corpus.txt` (`<case> <endpoint> <query>` per line), sends
//! each case's request to production through the server's own request path,
//! and writes what came back under `<golden dir>/<case>/`:
//!
//! * `response.pb`: the `ResponseData` chunks, each length-delimited. A
//!   response is cut to its first [`MAX_CHUNKS`] chunks and each chunk to its
//!   first [`MAX_ROWS`] rows, re-encoded as a valid table, so the fixtures stay
//!   small while multi-chunk framing stays covered. A flat file is the whole
//!   universe for a date, so only its first chunks are downloaded.
//! * `status`, only when the call failed upstream: the gRPC code and the
//!   service's `http_status_code` (0 when absent) on the first line, the
//!   message on the second.
//!
//! Naming cases records only those; otherwise every case is recorded.

use std::fmt::Write as _;
use std::future::ready;
use std::ops::ControlFlow;
use std::path::Path;

use prost::Message;
use thetadatadx::decode::{decode_data_table, record_responses};
use thetadatadx::endpoint::{EndpointArgs, invoke_endpoint_tables};
use thetadatadx::wire::{CompressionAlgo, ResponseData};
use thetadatadx::{Credentials, DirectConfig, Error, MarketDataClient};

/// Chunks kept per response.
const MAX_CHUNKS: usize = 2;
/// Rows kept per chunk.
const MAX_ROWS: usize = 20;

type BoxError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut args = std::env::args().skip(1);
    let (Some(creds), Some(dir)) = (args.next(), args.next()) else {
        return Err("usage: record_server_responses <creds file> <golden dir> [case ...]".into());
    };
    let only: Vec<String> = args.collect();
    let dir = Path::new(&dir);
    let corpus = std::fs::read_to_string(dir.join("corpus.txt"))?;

    let client =
        MarketDataClient::connect(&Credentials::from_file(&creds)?, DirectConfig::production())
            .await?;

    for line in corpus.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(case), Some(endpoint)) = (fields.next(), fields.next()) else {
            return Err(format!("corpus line without an endpoint: {line}").into());
        };
        if !only.is_empty() && !only.iter().any(|name| name == case) {
            continue;
        }
        let ep =
            thetadatadx::find(endpoint).ok_or_else(|| format!("unknown endpoint {endpoint}"))?;
        let mut call = EndpointArgs::new();
        for pair in fields
            .next()
            .unwrap_or("")
            .split('&')
            .filter(|p| !p.is_empty())
        {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("{case}: bad pair {pair}"))?;
            let param = ep
                .params
                .iter()
                .find(|param| param.name == name)
                .ok_or_else(|| format!("{case}: {endpoint} takes no {name}"))?;
            call.insert_raw(name, param.param_type, value)?;
        }

        record_responses(Some(Vec::new()));
        let outcome: Result<(), Error> = if ep.subcategory == "flat_file" {
            let mut chunks = 0;
            client
                .stream_flat_file(endpoint, call.required_date("date")?, |_, _| {
                    chunks += 1;
                    ready(if chunks < MAX_CHUNKS {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(())
                    })
                })
                .await
        } else {
            invoke_endpoint_tables(&client, endpoint, &call, |_, _| {
                ready(ControlFlow::Continue(()))
            })
            .await
            .map_err(Error::from)
        };
        let recorded = record_responses(None).unwrap_or_default();

        let case_dir = dir.join(case);
        std::fs::create_dir_all(&case_dir)?;
        let mut file = Vec::new();
        for chunk in recorded.iter().take(MAX_CHUNKS) {
            trim(chunk)?.encode_length_delimited(&mut file)?;
        }
        std::fs::write(case_dir.join("response.pb"), file)?;
        let status = case_dir.join("status");
        let mut summary = format!(
            "{case}: {} of {} chunks",
            recorded.len().min(MAX_CHUNKS),
            recorded.len()
        );
        if status.exists() {
            std::fs::remove_file(&status)?;
        }
        match outcome {
            Ok(()) => {}
            Err(Error::Grpc {
                kind,
                message,
                http_status_code,
                ..
            }) => {
                std::fs::write(
                    status,
                    format!(
                        "{} {}\n{message}\n",
                        kind as u32,
                        http_status_code.unwrap_or(0)
                    ),
                )?;
                let _ = write!(summary, ", upstream {kind} {http_status_code:?}: {message}");
            }
            // A failure on this side (decoding, transport) is not part of the
            // upstream's answer: the chunks replay it, or the case needs a rerun.
            Err(error) => {
                let _ = write!(summary, ", failed here: {error}");
            }
        }
        println!("{summary}");
    }
    Ok(())
}

/// `chunk` cut to its first [`MAX_ROWS`] rows, compressed as it arrived.
fn trim(chunk: &ResponseData) -> Result<ResponseData, BoxError> {
    let mut table = decode_data_table(&mut chunk.clone())?;
    table.data_table.truncate(MAX_ROWS);
    let bytes = table.encode_to_vec();
    let algo = chunk.compression_description.as_ref().map_or(0, |c| c.algo);
    let compressed_data = if algo == CompressionAlgo::Zstd as i32 {
        zstd::bulk::compress(&bytes, 0)?
    } else {
        bytes.clone()
    };
    Ok(ResponseData {
        compression_description: chunk.compression_description,
        original_size: i32::try_from(bytes.len())?,
        compressed_data,
    })
}
