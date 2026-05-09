// Hand-written FLATFILES subcommand surface (issue #433).
//
// Wires `tdx flatfile {quotes,trades,trade_quote,ohlc,open_interest,eod,request}`
// to `thetadatadx::ThetaDataDxClient::flatfile_request`. Output goes to the
// path supplied with `-o` / `--output`; if absent, the CSV/JSONL bytes
// are streamed to stdout via `std::io::copy` from the file we just wrote
// (the SDK's primary entry point writes to disk; we reroute on demand).
//
// Flat files are whole-universe daily blobs — they take a single
// `YYYYMMDD` date, NOT a (start, end, symbol) tuple. The high-level
// SDK methods reflect that contract; the CLI mirrors it 1:1.

use clap::{Arg, ArgMatches, Command};
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Credentials;

/// Add the top-level `flatfile` subcommand group.
pub(crate) fn add_flatfile_command(app: Command) -> Command {
    let common_args = |c: Command, with_date: bool| {
        let mut c = c;
        if with_date {
            c = c.arg(
                Arg::new("date")
                    .required(true)
                    .help("Single trading date in YYYYMMDD form, e.g. 20260428"),
            );
        }
        c.arg(
            Arg::new("format")
                .long("format")
                .short('f')
                .default_value("csv")
                .value_parser(["csv", "jsonl"])
                .help("On-disk format. csv = vendor byte-format CSV, jsonl = JSON Lines"),
        )
        .arg(Arg::new("output").long("output").short('o').help(
            "Output file path. If omitted, the CSV / JSONL bytes are written to a \
                     temporary file then streamed to stdout. With `-o`, the path is taken \
                     verbatim and the format extension is auto-appended if absent.",
        ))
    };

    let group = Command::new("flatfile")
        .about(
            "FLATFILES surface — whole-universe daily blobs (CSV / JSONL). \
             Takes a single YYYYMMDD date, not a (start, end) range.",
        )
        .subcommand_required(true)
        .subcommand(common_args(
            Command::new("quotes").about("Option quote flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("trades").about("Option trade flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("trade_quote").about("Option trade-quote flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("ohlc").about("Option OHLC flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("open_interest").about("Option open-interest flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("eod").about("Option EOD flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("stock_quotes").about("Stock quote flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("stock_trades").about("Stock trade flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("stock_trade_quote").about("Stock trade-quote flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("stock_eod").about("Stock EOD flat file"),
            true,
        ))
        .subcommand(
            Command::new("request")
                .about("Generic flatfile request (any sec_type / req_type)")
                .arg(
                    Arg::new("sec_type")
                        .long("sec-type")
                        .required(true)
                        .value_parser(["option", "stock", "index"])
                        .help("Security type: option, stock, or index"),
                )
                .arg(
                    Arg::new("req_type")
                        .long("req-type")
                        .required(true)
                        .value_parser([
                            "eod",
                            "quote",
                            "open_interest",
                            "ohlc",
                            "trade",
                            "trade_quote",
                        ])
                        .help("Request type"),
                )
                .arg(
                    Arg::new("date")
                        .long("date")
                        .required(true)
                        .help("Trading date in YYYYMMDD form"),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .short('f')
                        .default_value("csv")
                        .value_parser(["csv", "jsonl"]),
                )
                .arg(Arg::new("output").long("output").short('o')),
        );

    app.subcommand(group)
}

/// Map the CLI subcommand name to the `(SecType, ReqType)` tuple fed to
/// `flatfile_request`. Returns `None` for the generic `request` arm
/// (which carries explicit `--sec-type` / `--req-type` flags).
fn sec_req_for_subcommand(name: &str) -> Option<(SecType, ReqType)> {
    Some(match name {
        "quotes" => (SecType::Option, ReqType::Quote),
        "trades" => (SecType::Option, ReqType::Trade),
        "trade_quote" => (SecType::Option, ReqType::TradeQuote),
        "ohlc" => (SecType::Option, ReqType::Ohlc),
        "open_interest" => (SecType::Option, ReqType::OpenInterest),
        "eod" => (SecType::Option, ReqType::Eod),
        "stock_quotes" => (SecType::Stock, ReqType::Quote),
        "stock_trades" => (SecType::Stock, ReqType::Trade),
        "stock_trade_quote" => (SecType::Stock, ReqType::TradeQuote),
        "stock_eod" => (SecType::Stock, ReqType::Eod),
        _ => return None,
    })
}

fn parse_format(s: &str) -> FlatFileFormat {
    match s {
        "jsonl" => FlatFileFormat::Jsonl,
        _ => FlatFileFormat::Csv,
    }
}

fn parse_sec_type(s: &str) -> Result<SecType, thetadatadx::Error> {
    match s {
        "option" => Ok(SecType::Option),
        "stock" => Ok(SecType::Stock),
        "index" => Ok(SecType::Index),
        other => Err(thetadatadx::Error::config_invalid(
            "sec_type",
            format!("unknown sec_type: {other}"),
        )),
    }
}

fn parse_req_type(s: &str) -> Result<ReqType, thetadatadx::Error> {
    match s {
        "eod" => Ok(ReqType::Eod),
        "quote" => Ok(ReqType::Quote),
        "open_interest" => Ok(ReqType::OpenInterest),
        "ohlc" => Ok(ReqType::Ohlc),
        "trade" => Ok(ReqType::Trade),
        "trade_quote" => Ok(ReqType::TradeQuote),
        other => Err(thetadatadx::Error::config_invalid(
            "req_type",
            format!("unknown req_type: {other}"),
        )),
    }
}

/// Dispatch a parsed `tdx flatfile <sub>` invocation. Returns `Ok(true)`
/// when the subcommand was a `flatfile` subcommand (handled here);
/// `Ok(false)` lets the caller fall through to the registry-driven
/// dispatch in `main::run`.
pub(crate) async fn try_dispatch(
    matches: &ArgMatches,
    creds_path: &str,
) -> Result<bool, thetadatadx::Error> {
    let Some(("flatfile", ff_m)) = matches.subcommand() else {
        return Ok(false);
    };

    let (sub_name, sub_m) = match ff_m.subcommand() {
        Some(pair) => pair,
        None => {
            return Err(thetadatadx::Error::config_invalid(
                "flatfile",
                "missing flatfile sub-subcommand (try `tdx flatfile --help`)",
            ));
        }
    };

    let (sec_type, req_type, date, format, output) = if sub_name == "request" {
        let sec = parse_sec_type(
            sub_m
                .get_one::<String>("sec_type")
                .map_or("", String::as_str),
        )?;
        let req = parse_req_type(
            sub_m
                .get_one::<String>("req_type")
                .map_or("", String::as_str),
        )?;
        let date = sub_m
            .get_one::<String>("date")
            .map(String::as_str)
            .unwrap_or("");
        let fmt = parse_format(
            sub_m
                .get_one::<String>("format")
                .map_or("csv", String::as_str),
        );
        let out = sub_m.get_one::<String>("output").cloned();
        (sec, req, date.to_string(), fmt, out)
    } else if let Some((sec, req)) = sec_req_for_subcommand(sub_name) {
        let date = sub_m
            .get_one::<String>("date")
            .map(String::as_str)
            .unwrap_or("");
        let fmt = parse_format(
            sub_m
                .get_one::<String>("format")
                .map_or("csv", String::as_str),
        );
        let out = sub_m.get_one::<String>("output").cloned();
        (sec, req, date.to_string(), fmt, out)
    } else {
        return Err(thetadatadx::Error::config_invalid(
            "flatfile",
            format!("unknown flatfile sub-subcommand: {sub_name}"),
        ));
    };

    let creds = Credentials::from_file(creds_path)?;

    // Choose the on-disk path: explicit `-o`, otherwise a temp file we
    // stream to stdout afterwards. Flat files are large, so streaming
    // through a real file beats buffering whole-universe blobs in RAM.
    let (final_path, stream_to_stdout) = match output {
        Some(p) => (std::path::PathBuf::from(p), false),
        None => {
            let tmp = std::env::temp_dir().join(format!(
                "tdx_flatfile_{}_{}_{date}.{}",
                sec_type,
                req_type as u32,
                format.extension(),
            ));
            (tmp, true)
        }
    };

    let written = thetadatadx::flatfiles::flatfile_request(
        &creds,
        sec_type,
        req_type,
        &date,
        &final_path,
        format,
    )
    .await?;

    if stream_to_stdout {
        // Stream bytes to stdout without re-buffering. Writes go straight
        // through to the terminal / pipe target. On error we propagate
        // an `Error::Io` so the CLI exits with a meaningful message.
        let mut input = std::fs::File::open(&written).map_err(thetadatadx::Error::from)?;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        std::io::copy(&mut input, &mut handle).map_err(thetadatadx::Error::from)?;
        // Best-effort cleanup of the temp file. Ignore errors — the OS
        // will sweep `/tmp` eventually and we'd rather complete the call
        // than fail at the very end on a stat() race.
        let _ = std::fs::remove_file(&written);
    } else {
        // Echo the written path to stderr so scripts capturing stdout
        // (e.g. `tdx flatfile quotes ... -o foo.csv`) still see "where
        // it landed" without polluting the data stream.
        eprintln!("wrote {}", written.display());
    }

    Ok(true)
}
