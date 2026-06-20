//! Hand-written `thetadatadx flatfile` subcommand surface.
//!
//! Wires `thetadatadx flatfile {trade_quote,open_interest,eod,stock_trade_quote,stock_eod,request}`
//! to `thetadatadx::Client::flatfile_request`. The convenience
//! subcommands cover exactly the datasets the flat-file distribution
//! serves; the generic `request` arm constrains `--sec-type` / `--req-type`
//! to the served union and rejects an unserved `(sec_type, req_type)` pair
//! with a typed invalid-parameter error. Output goes to the
//! path supplied with `-o` / `--output`; if absent, the CSV/JSONL bytes
//! are streamed to stdout via `std::io::copy` from the file just written
//! (the SDK's primary entry point writes to disk; the CLI reroutes on demand).
//!
//! Flat files are whole-universe daily blobs — they take a single
//! `YYYYMMDD` date, not a (start, end, symbol) tuple. The high-level
//! SDK methods reflect that contract; the CLI mirrors it 1:1.

use clap::{Arg, ArgMatches, Command};
use thetadatadx::flatfiles::{flat_file_serves, FlatFileFormat, ReqType, SecType, SERVED_DATASETS};
use thetadatadx::Credentials;

/// Lower-case CLI token for a security type. The flag spelling is lower-case
/// (`option` / `stock`), distinct from the upper-case `SEC=` wire token, so it
/// is sourced here rather than from the wire formatter.
fn sec_type_cli_token(sec: SecType) -> &'static str {
    match sec {
        SecType::Option => "option",
        SecType::Stock => "stock",
        SecType::Index => "index",
    }
}

/// Distinct lower-case `sec_type` flag values the flat-file service serves,
/// derived from the served matrix so the generic `request` arm can never
/// advertise a security type without a served dataset (no `index`). Order
/// follows the matrix; duplicates are skipped.
fn served_sec_type_tokens() -> Vec<&'static str> {
    let mut tokens: Vec<&'static str> = Vec::new();
    for (sec, _) in SERVED_DATASETS {
        let token = sec_type_cli_token(*sec);
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens
}

/// Distinct lower-case `req_type` flag values served as a flat file, derived
/// from the served matrix so the generic `request` arm advertises only request
/// types that exist as flat files (no per-tick `quote` / `trade` / `ohlc`).
/// `ReqType::as_str` is the canonical lower-case dataset token.
fn served_req_type_tokens() -> Vec<&'static str> {
    let mut tokens: Vec<&'static str> = Vec::new();
    for (_, req) in SERVED_DATASETS {
        let token = req.as_str();
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens
}

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
            Command::new("trade_quote").about("Option trade-quote flat file"),
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
            Command::new("stock_trade_quote").about("Stock trade-quote flat file"),
            true,
        ))
        .subcommand(common_args(
            Command::new("stock_eod").about("Stock EOD flat file"),
            true,
        ))
        .subcommand(
            Command::new("request")
                .about("Generic flatfile request over a served (sec_type, req_type) dataset")
                .arg(
                    Arg::new("sec_type")
                        .long("sec-type")
                        .required(true)
                        .value_parser(clap::builder::PossibleValuesParser::new(
                            served_sec_type_tokens(),
                        ))
                        .help("Security type with a served flat-file dataset (option or stock)"),
                )
                .arg(
                    Arg::new("req_type")
                        .long("req-type")
                        .required(true)
                        .value_parser(clap::builder::PossibleValuesParser::new(
                            served_req_type_tokens(),
                        ))
                        .help(
                            "Request type served as a flat file. Valid set depends on \
                             --sec-type (option: trade_quote, open_interest, eod; stock: \
                             trade_quote, eod)",
                        ),
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
        "trade_quote" => (SecType::Option, ReqType::TradeQuote),
        "open_interest" => (SecType::Option, ReqType::OpenInterest),
        "eod" => (SecType::Option, ReqType::Eod),
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

/// Dispatch a parsed `thetadatadx flatfile <sub>` invocation. Returns `Ok(true)`
/// when the subcommand was a `flatfile` subcommand (handled here);
/// `Ok(false)` lets the caller fall through to the registry-driven
/// dispatch in `main::run`.
///
/// # Errors
/// Returns an error when the flatfile sub-subcommand is missing or unknown,
/// when `--sec-type` / `--req-type` fail to parse, when credentials cannot be
/// loaded, when the underlying flat-file request fails, or when streaming the
/// result to stdout hits an I/O error.
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
                "missing flatfile sub-subcommand (try `thetadatadx flatfile --help`)",
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

    // Reject an unserved (sec_type, req_type) pair before loading credentials
    // or touching the network. The generic `request` arm constrains each flag
    // to the served union, but their cross-product still admits an unserved
    // pair (e.g. `--sec-type stock --req-type open_interest`); the convenience
    // subcommands are served by construction. The served matrix is the single
    // source of truth.
    if !flat_file_serves(sec_type, req_type) {
        return Err(thetadatadx::Error::config_invalid(
            "flatfile",
            format!(
                "flat-file service does not serve {sec_type} {}",
                req_type.as_str()
            ),
        ));
    }

    let creds = Credentials::from_file(creds_path)?;

    // Choose the on-disk path: explicit `-o`, otherwise a temp file we
    // stream to stdout afterwards. Flat files are large, so streaming
    // through a real file beats buffering whole-universe blobs in RAM.
    let (final_path, stream_to_stdout) = match output {
        Some(p) => (std::path::PathBuf::from(p), false),
        None => {
            let tmp = std::env::temp_dir().join(format!(
                "thetadatadx_flatfile_{}_{}_{date}.{}",
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
        // (e.g. `thetadatadx flatfile trade_quote ... -o foo.csv`) still see
        // "where it landed" without polluting the data stream.
        eprintln!("wrote {}", written.display());
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generic `request` arm advertises only security types with a served
    /// flat-file dataset — `option` and `stock`, never `index`.
    #[test]
    fn sec_type_choices_match_the_served_matrix() {
        let tokens = served_sec_type_tokens();
        assert!(tokens.contains(&"option"));
        assert!(tokens.contains(&"stock"));
        assert!(
            !tokens.contains(&"index"),
            "index has no served flat-file dataset; got {tokens:?}"
        );
    }

    /// The generic `request` arm advertises only request types served as a
    /// flat file — never per-tick `quote` / `trade` / `ohlc`.
    #[test]
    fn req_type_choices_match_the_served_matrix() {
        let tokens = served_req_type_tokens();
        for served in ["trade_quote", "open_interest", "eod"] {
            assert!(
                tokens.contains(&served),
                "req_type choices must include served `{served}`; got {tokens:?}"
            );
        }
        for unserved in ["quote", "trade", "ohlc"] {
            assert!(
                !tokens.contains(&unserved),
                "req_type choices must exclude unserved `{unserved}`; got {tokens:?}"
            );
        }
    }

    /// Every advertised choice must parse back to a variant, and every
    /// resulting same-name cross pair the matrix does not serve (e.g. stock
    /// open_interest) is caught by the `flat_file_serves` gate the dispatch
    /// applies before any network work.
    #[test]
    fn unserved_cross_pair_is_not_in_the_served_matrix() {
        // Both tokens are individually advertised, but the pair is unserved.
        let sec = parse_sec_type("stock").expect("stock parses");
        let req = parse_req_type("open_interest").expect("open_interest parses");
        assert!(
            !flat_file_serves(sec, req),
            "stock open_interest must be rejected by the served-matrix gate"
        );
    }

    /// The constrained value parsers must build a valid clap command.
    #[test]
    fn flatfile_command_builds() {
        let app = add_flatfile_command(Command::new("tdx"));
        app.debug_assert();
    }
}
