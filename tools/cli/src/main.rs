use std::process;

use clap::{Arg, ArgMatches, Command};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, ContentArrangement, Table};
use thetadatadx::registry::{self, EndpointMeta};

// ═══════════════════════════════════════════════════════════════════════════
//  CLI construction from endpoint registry
// ═══════════════════════════════════════════════════════════════════════════

/// Build the full CLI tree dynamically from the endpoint registry.
///
/// Structure: `tdx [global opts] <category> <endpoint-subcmd> [args...]`
///
/// Categories (stock, option, index, rate, calendar) are auto-populated.
/// The `auth`, `greeks`, and `iv` commands remain hand-written since they
/// don't map to `DirectClient` endpoints.
// Reason: CLI builder registers all subcommands in a single function; splitting would lose cohesion.
#[allow(clippy::too_many_lines)]
fn build_cli() -> Command {
    let mut app = Command::new("tdx")
        .version(env!("CARGO_PKG_VERSION"))
        .about("ThetaDataDx CLI — query ThetaData from your terminal")
        .long_about(
            "Native CLI for ThetaData market data. No JVM required.\n\n\
             Requires a creds.txt file (email on line 1, password on line 2).",
        )
        .arg(
            Arg::new("creds")
                .long("creds")
                .global(true)
                .default_value("creds.txt")
                .help("Path to credentials file (email + password, one per line)"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .global(true)
                .default_value("production")
                .value_parser(["production", "dev"])
                .help("Server configuration preset"),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .global(true)
                .default_value("table")
                .value_parser(["table", "json", "csv"])
                .help("Output format"),
        );

    // Hand-written: auth
    app = app.subcommand(Command::new("auth").about("Test authentication and print session info"));

    // Hand-written: greeks (offline)
    app = app.subcommand(
        Command::new("greeks")
            .about("Compute Black-Scholes Greeks (offline, no server needed)")
            .arg(Arg::new("spot").required(true).help("Spot price"))
            .arg(Arg::new("strike").required(true).help("Strike price"))
            .arg(
                Arg::new("rate")
                    .required(true)
                    .help("Risk-free rate (e.g. 0.05)"),
            )
            .arg(
                Arg::new("dividend")
                    .required(true)
                    .help("Dividend yield (e.g. 0.015)"),
            )
            .arg(
                Arg::new("time")
                    .required(true)
                    .help("Time to expiration in years (e.g. 0.082 for ~30 days)"),
            )
            .arg(Arg::new("option_price").required(true).help("Option price"))
            .arg(
                Arg::new("right")
                    .required(true)
                    .value_parser(["call", "put"])
                    .help("Option type: call or put"),
            ),
    );

    // Hand-written: iv (offline)
    app = app.subcommand(
        Command::new("iv")
            .about("Compute implied volatility only (offline, no server needed)")
            .arg(Arg::new("spot").required(true).help("Spot price"))
            .arg(Arg::new("strike").required(true).help("Strike price"))
            .arg(Arg::new("rate").required(true).help("Risk-free rate"))
            .arg(Arg::new("dividend").required(true).help("Dividend yield"))
            .arg(
                Arg::new("time")
                    .required(true)
                    .help("Time to expiration in years"),
            )
            .arg(Arg::new("option_price").required(true).help("Option price"))
            .arg(
                Arg::new("right")
                    .required(true)
                    .value_parser(["call", "put"])
                    .help("Option type: call or put"),
            ),
    );

    // Dynamic: build category subcommands from ENDPOINTS
    for &cat in registry::CATEGORIES {
        let cat_endpoints = registry::by_category(cat);
        let cat_about = match cat {
            "stock" => "Stock data commands",
            "option" => "Option data commands",
            "index" => "Index data commands",
            "rate" => "Interest rate data commands",
            "calendar" => "Market calendar commands",
            _ => "Data commands",
        };

        let mut cat_cmd = Command::new(cat).about(cat_about);

        for ep in &cat_endpoints {
            // Subcmd name: strip the category prefix (e.g. "stock_history_eod" -> "history_eod")
            let sub_name = ep
                .name
                .strip_prefix(&format!("{cat}_"))
                // For "interest_rate_history_eod" under "rate" category
                .or_else(|| ep.name.strip_prefix("interest_rate_"))
                .unwrap_or(ep.name);

            let mut sub_cmd = Command::new(sub_name).about(ep.description);

            for p in ep.params {
                sub_cmd = sub_cmd.arg(Arg::new(p.name).required(p.required).help(p.description));
            }

            cat_cmd = cat_cmd.subcommand(sub_cmd);
        }

        app = app.subcommand(cat_cmd);
    }

    app
}

// ═══════════════════════════════════════════════════════════════════════════
//  Dynamic dispatch — calls the right DirectClient method based on endpoint name
// ═══════════════════════════════════════════════════════════════════════════

/// Extract a string arg from clap matches, or panic with a clear message.
fn get_arg<'a>(m: &'a ArgMatches, name: &str) -> &'a str {
    m.get_one::<String>(name).map_or_else(
        || panic!("missing required argument: {name}"),
        std::string::String::as_str,
    )
}

/// Parse comma-separated symbols into a `Vec<&str>`, filtering out empty entries.
fn parse_symbols(s: &str) -> Vec<&str> {
    s.split(',')
        .map(str::trim)
        .filter(|sym| !sym.is_empty())
        .collect()
}

/// Normalize option right to the uppercase single-letter format the API expects.
fn normalize_right(s: &str) -> Result<&'static str, thetadatadx::Error> {
    match s.to_ascii_uppercase().as_str() {
        "C" | "CALL" => Ok("C"),
        "P" | "PUT" => Ok("P"),
        _ => Err(thetadatadx::Error::Fpss(format!(
            "invalid option right '{s}': expected C, P, call, or put"
        ))),
    }
}

/// Dispatch a single endpoint call based on its registry metadata.
///
/// This is the core of the dynamic dispatch system: given an `EndpointMeta`
/// and parsed `ArgMatches`, it calls the right `ThetaDataDx` method (via
/// Deref to `DirectClient`) and renders the result in the requested format.
// Reason: endpoint dispatcher matches over 61 endpoints; cannot be meaningfully split.
#[allow(clippy::too_many_lines)]
async fn dispatch_endpoint(
    ep: &EndpointMeta,
    m: &ArgMatches,
    client: &thetadatadx::ThetaDataDx,
    fmt: &OutputFormat,
) -> Result<(), thetadatadx::Error> {
    // The match is on the exact endpoint name from the registry.
    // This is unavoidable because DirectClient methods have heterogeneous signatures.
    match ep.name {
        // ── Stock List ──────────────────────────────────────────────
        "stock_list_symbols" => {
            let symbols = client.stock_list_symbols().await?;
            render_string_list(&symbols, "symbol", fmt);
        }
        "stock_list_dates" => {
            let rt = get_arg(m, "request_type");
            let sym = get_arg(m, "symbol");
            let dates = client.stock_list_dates(rt, sym).await?;
            render_string_list(&dates, "date", fmt);
        }

        // ── Stock Snapshot ──────────────────────────────────────────
        "stock_snapshot_ohlc" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.stock_snapshot_ohlc(&syms).await?;
            render_ohlc(&ticks, fmt);
        }
        "stock_snapshot_trade" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.stock_snapshot_trade(&syms).await?;
            render_trades(&ticks, fmt);
        }
        "stock_snapshot_quote" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.stock_snapshot_quote(&syms).await?;
            render_quotes(&ticks, fmt);
        }
        "stock_snapshot_market_value" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.stock_snapshot_market_value(&syms).await?;
            render_market_value(&ticks, fmt);
        }

        // ── Stock History ───────────────────────────────────────────
        "stock_history_eod" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let ticks = client.stock_history_eod(sym, start, end).await?;
            render_eod(&ticks, fmt);
        }
        "stock_history_ohlc" => {
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client.stock_history_ohlc(sym, date, interval).await?;
            render_ohlc(&ticks, fmt);
        }
        "stock_history_ohlc_range" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .stock_history_ohlc_range(sym, start, end, interval)
                .await?;
            render_ohlc(&ticks, fmt);
        }
        "stock_history_trade" => {
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let ticks = client.stock_history_trade(sym, date).await?;
            render_trades(&ticks, fmt);
        }
        "stock_history_quote" => {
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client.stock_history_quote(sym, date, interval).await?;
            render_quotes(&ticks, fmt);
        }
        "stock_history_trade_quote" => {
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let ticks = client.stock_history_trade_quote(sym, date).await?;
            render_trade_quotes(&ticks, fmt);
        }

        // ── Stock At-Time ───────────────────────────────────────────
        "stock_at_time_trade" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let tod = get_arg(m, "time_of_day");
            let ticks = client.stock_at_time_trade(sym, start, end, tod).await?;
            render_trades(&ticks, fmt);
        }
        "stock_at_time_quote" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let tod = get_arg(m, "time_of_day");
            let ticks = client.stock_at_time_quote(sym, start, end, tod).await?;
            render_quotes(&ticks, fmt);
        }

        // ── Option List ─────────────────────────────────────────────
        "option_list_symbols" => {
            let symbols = client.option_list_symbols().await?;
            render_string_list(&symbols, "symbol", fmt);
        }
        "option_list_dates" => {
            let rt = get_arg(m, "request_type");
            let sym = get_arg(m, "symbol");
            let exp = get_arg(m, "expiration");
            let strike = get_arg(m, "strike");
            let right = normalize_right(get_arg(m, "right"))?;
            let dates = client
                .option_list_dates(rt, sym, exp, strike, right)
                .await?;
            render_string_list(&dates, "date", fmt);
        }
        "option_list_expirations" => {
            let sym = get_arg(m, "symbol");
            let exps = client.option_list_expirations(sym).await?;
            render_string_list(&exps, "expiration", fmt);
        }
        "option_list_strikes" => {
            let sym = get_arg(m, "symbol");
            let exp = get_arg(m, "expiration");
            let strikes = client.option_list_strikes(sym, exp).await?;
            render_string_list(&strikes, "strike", fmt);
        }
        "option_list_contracts" => {
            let rt = get_arg(m, "request_type");
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let contracts = client.option_list_contracts(rt, sym, date).await?;
            render_option_contracts(&contracts, fmt);
        }

        // ── Option Snapshot ─────────────────────────────────────────
        "option_snapshot_ohlc" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client.option_snapshot_ohlc(sym, exp, strike, right).await?;
            render_ohlc(&ticks, fmt);
        }
        "option_snapshot_trade" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_trade(sym, exp, strike, right)
                .await?;
            render_trades(&ticks, fmt);
        }
        "option_snapshot_quote" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_quote(sym, exp, strike, right)
                .await?;
            render_quotes(&ticks, fmt);
        }
        "option_snapshot_open_interest" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_open_interest(sym, exp, strike, right)
                .await?;
            render_open_interest(&ticks, fmt);
        }
        "option_snapshot_market_value" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_market_value(sym, exp, strike, right)
                .await?;
            render_market_value(&ticks, fmt);
        }
        "option_snapshot_greeks_implied_volatility" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_greeks_implied_volatility(sym, exp, strike, right)
                .await?;
            render_iv(&ticks, fmt);
        }
        "option_snapshot_greeks_all" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_greeks_all(sym, exp, strike, right)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_snapshot_greeks_first_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_greeks_first_order(sym, exp, strike, right)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_snapshot_greeks_second_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_greeks_second_order(sym, exp, strike, right)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_snapshot_greeks_third_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let ticks = client
                .option_snapshot_greeks_third_order(sym, exp, strike, right)
                .await?;
            render_greeks(&ticks, fmt);
        }

        // ── Option History ──────────────────────────────────────────
        "option_history_eod" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let ticks = client
                .option_history_eod(sym, exp, strike, right, start, end)
                .await?;
            render_eod(&ticks, fmt);
        }
        "option_history_ohlc" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_ohlc(sym, exp, strike, right, date, interval)
                .await?;
            render_ohlc(&ticks, fmt);
        }
        "option_history_trade" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade(sym, exp, strike, right, date)
                .await?;
            render_trades(&ticks, fmt);
        }
        "option_history_quote" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_quote(sym, exp, strike, right, date, interval)
                .await?;
            render_quotes(&ticks, fmt);
        }
        "option_history_trade_quote" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_quote(sym, exp, strike, right, date)
                .await?;
            render_trade_quotes(&ticks, fmt);
        }
        "option_history_open_interest" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_open_interest(sym, exp, strike, right, date)
                .await?;
            render_open_interest(&ticks, fmt);
        }

        // ── Option History Greeks ───────────────────────────────────
        "option_history_greeks_eod" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let ticks = client
                .option_history_greeks_eod(sym, exp, strike, right, start, end)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_greeks_all" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_greeks_all(sym, exp, strike, right, date, interval)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_trade_greeks_all" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_greeks_all(sym, exp, strike, right, date)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_greeks_first_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_greeks_first_order(sym, exp, strike, right, date, interval)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_trade_greeks_first_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_greeks_first_order(sym, exp, strike, right, date)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_greeks_second_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_greeks_second_order(sym, exp, strike, right, date, interval)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_trade_greeks_second_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_greeks_second_order(sym, exp, strike, right, date)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_greeks_third_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_greeks_third_order(sym, exp, strike, right, date, interval)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_trade_greeks_third_order" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_greeks_third_order(sym, exp, strike, right, date)
                .await?;
            render_greeks(&ticks, fmt);
        }
        "option_history_greeks_implied_volatility" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client
                .option_history_greeks_implied_volatility(sym, exp, strike, right, date, interval)
                .await?;
            render_iv(&ticks, fmt);
        }
        "option_history_trade_greeks_implied_volatility" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let date = get_arg(m, "date");
            let ticks = client
                .option_history_trade_greeks_implied_volatility(sym, exp, strike, right, date)
                .await?;
            render_iv(&ticks, fmt);
        }

        // ── Option At-Time ──────────────────────────────────────────
        "option_at_time_trade" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let tod = get_arg(m, "time_of_day");
            let ticks = client
                .option_at_time_trade(sym, exp, strike, right, start, end, tod)
                .await?;
            render_trades(&ticks, fmt);
        }
        "option_at_time_quote" => {
            let (sym, exp, strike, right) = option_contract_args(m)?;
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let tod = get_arg(m, "time_of_day");
            let ticks = client
                .option_at_time_quote(sym, exp, strike, right, start, end, tod)
                .await?;
            render_quotes(&ticks, fmt);
        }

        // ── Index List ──────────────────────────────────────────────
        "index_list_symbols" => {
            let symbols = client.index_list_symbols().await?;
            render_string_list(&symbols, "symbol", fmt);
        }
        "index_list_dates" => {
            let sym = get_arg(m, "symbol");
            let dates = client.index_list_dates(sym).await?;
            render_string_list(&dates, "date", fmt);
        }

        // ── Index Snapshot ──────────────────────────────────────────
        "index_snapshot_ohlc" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.index_snapshot_ohlc(&syms).await?;
            render_ohlc(&ticks, fmt);
        }
        "index_snapshot_price" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.index_snapshot_price(&syms).await?;
            render_price(&ticks, fmt);
        }
        "index_snapshot_market_value" => {
            let syms = parse_symbols(get_arg(m, "symbols"));
            let ticks = client.index_snapshot_market_value(&syms).await?;
            render_market_value(&ticks, fmt);
        }

        // ── Index History ───────────────────────────────────────────
        "index_history_eod" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let ticks = client.index_history_eod(sym, start, end).await?;
            render_eod(&ticks, fmt);
        }
        "index_history_ohlc" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let interval = get_arg(m, "interval");
            let ticks = client.index_history_ohlc(sym, start, end, interval).await?;
            render_ohlc(&ticks, fmt);
        }
        "index_history_price" => {
            let sym = get_arg(m, "symbol");
            let date = get_arg(m, "date");
            let interval = get_arg(m, "interval");
            let ticks = client.index_history_price(sym, date, interval).await?;
            render_price(&ticks, fmt);
        }

        // ── Index At-Time ───────────────────────────────────────────
        "index_at_time_price" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let tod = get_arg(m, "time_of_day");
            let ticks = client.index_at_time_price(sym, start, end, tod).await?;
            render_price(&ticks, fmt);
        }

        // ── Calendar ────────────────────────────────────────────────
        "calendar_open_today" => {
            let days = client.calendar_open_today().await?;
            render_calendar(&days, fmt);
        }
        "calendar_on_date" => {
            let date = get_arg(m, "date");
            let days = client.calendar_on_date(date).await?;
            render_calendar(&days, fmt);
        }
        "calendar_year" => {
            let year = get_arg(m, "year");
            let days = client.calendar_year(year).await?;
            render_calendar(&days, fmt);
        }

        // ── Interest Rate ───────────────────────────────────────────
        "interest_rate_history_eod" => {
            let sym = get_arg(m, "symbol");
            let start = get_arg(m, "start_date");
            let end = get_arg(m, "end_date");
            let ticks = client.interest_rate_history_eod(sym, start, end).await?;
            render_interest_rates(&ticks, fmt);
        }

        other => {
            return Err(thetadatadx::Error::Config(format!(
                "unhandled endpoint in dispatch: {other}"
            )));
        }
    }

    Ok(())
}

/// Extract the 4 standard option contract args from clap matches.
fn option_contract_args(
    m: &ArgMatches,
) -> Result<(&str, &str, &str, &'static str), thetadatadx::Error> {
    let sym = get_arg(m, "symbol");
    let exp = get_arg(m, "expiration");
    let strike = get_arg(m, "strike");
    let right = normalize_right(get_arg(m, "right"))?;
    Ok((sym, exp, strike, right))
}

// ═══════════════════════════════════════════════════════════════════════════
//  Output format enum
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
enum OutputFormat {
    Table,
    Json,
    Csv,
}

impl OutputFormat {
    fn from_str(s: &str) -> Self {
        match s {
            "json" => Self::Json,
            "csv" => Self::Csv,
            _ => Self::Table,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Formatting helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Format `ms_of_day` as HH:MM:SS.mmm
fn format_ms(ms: i32) -> String {
    if ms < 0 {
        return "N/A".into();
    }
    let total_ms = u64::try_from(ms).unwrap_or(0);
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1_000;
    let ms_frac = total_ms % 1_000;
    format!("{h:02}:{m:02}:{s:02}.{ms_frac:03}")
}

/// Format a decoded f64 price for display.
fn format_price_f64(value: f64) -> String {
    if value == 0.0 {
        return "0.00".into();
    }
    format!("{value:.2}")
}

/// Format a YYYYMMDD integer date to a readable string.
fn format_date(date: i32) -> String {
    if date == 0 {
        return "N/A".into();
    }
    let y = date / 10000;
    let m = (date % 10000) / 100;
    let d = date % 100;
    format!("{y:04}-{m:02}-{d:02}")
}

// ═══════════════════════════════════════════════════════════════════════════
//  Output renderers — one generic system for table / json / csv
// ═══════════════════════════════════════════════════════════════════════════

/// A row-oriented data structure that all output formatters consume.
struct TabularData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl TabularData {
    fn new(headers: Vec<&str>) -> Self {
        Self {
            headers: headers
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect(),
            rows: Vec::new(),
        }
    }

    fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    fn render(&self, fmt: &OutputFormat) {
        match fmt {
            OutputFormat::Table => self.render_table(),
            OutputFormat::Json => self.render_json(),
            OutputFormat::Csv => self.render_csv(),
        }
    }

    fn render_table(&self) {
        if self.rows.is_empty() {
            eprintln!("0 rows");
            return;
        }
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL_CONDENSED)
            .set_content_arrangement(ContentArrangement::Dynamic)
            .set_header(self.headers.iter().map(Cell::new));

        for row in &self.rows {
            // For table display, nulls render as empty string.
            table.add_row(row.iter().map(|cell| {
                if cell == NULL_SENTINEL {
                    Cell::new("")
                } else {
                    Cell::new(cell)
                }
            }));
        }
        println!("{table}");
        eprintln!("{} rows", self.rows.len());
    }

    fn render_json(&self) {
        let arr: Vec<sonic_rs::Value> = self
            .rows
            .iter()
            .map(|row| {
                let mut obj = sonic_rs::Object::new();
                for (i, h) in self.headers.iter().enumerate() {
                    let val = row.get(i).cloned().unwrap_or_default();
                    // Null sentinel -> JSON null
                    if val == NULL_SENTINEL {
                        obj.insert(&h, sonic_rs::Value::new_null());
                    } else if let Ok(n) = val.parse::<f64>() {
                        // Try to parse as number for cleaner JSON
                        obj.insert(
                            &h,
                            sonic_rs::Value::from(
                                sonic_rs::Number::from_f64(n)
                                    .unwrap_or_else(|| sonic_rs::Number::from(0)),
                            ),
                        );
                    } else {
                        obj.insert(&h, sonic_rs::Value::from(val.as_str()));
                    }
                }
                sonic_rs::Value::from(obj)
            })
            .collect();
        println!(
            "{}",
            sonic_rs::to_string_pretty(&arr).unwrap_or_else(|_| "[]".into())
        );
    }

    fn render_csv(&self) {
        println!("{}", self.headers.join(","));
        for row in &self.rows {
            let escaped: Vec<String> = row
                .iter()
                .map(|cell| {
                    // Null sentinel -> empty (CSV has no native null)
                    if cell == NULL_SENTINEL {
                        String::new()
                    } else if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
                        format!("\"{}\"", cell.replace('"', "\"\""))
                    } else {
                        cell.clone()
                    }
                })
                .collect();
            println!("{}", escaped.join(","));
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  DataTable renderer — for raw_endpoint results
// ═══════════════════════════════════════════════════════════════════════════

/// Sentinel value used internally to distinguish null values from empty strings
/// in `DataTable` cells. For table display, nulls render as empty; for JSON/CSV,
/// they render as proper `null`.
const NULL_SENTINEL: &str = "\x00NULL\x00";

// ═══════════════════════════════════════════════════════════════════════════
//  Client construction helper
// ═══════════════════════════════════════════════════════════════════════════

async fn connect(
    creds_path: &str,
    preset: &str,
) -> Result<thetadatadx::ThetaDataDx, thetadatadx::Error> {
    let creds = thetadatadx::Credentials::from_file(creds_path)?;
    let config = match preset {
        "dev" => thetadatadx::DirectConfig::dev(),
        _ => thetadatadx::DirectConfig::production(),
    };
    thetadatadx::ThetaDataDx::connect(&creds, config).await
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tick rendering helpers — reduce repetition across subcommands
// ═══════════════════════════════════════════════════════════════════════════

fn render_eod(ticks: &[tdbe::types::tick::EodTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "ms_of_day2",
        "open",
        "high",
        "low",
        "close",
        "volume",
        "count",
        "bid_size",
        "bid_exchange",
        "bid",
        "bid_condition",
        "ask_size",
        "ask_exchange",
        "ask",
        "ask_condition",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format_ms(t.ms_of_day2),
            format_price_f64(t.open),
            format_price_f64(t.high),
            format_price_f64(t.low),
            format_price_f64(t.close),
            format!("{}", t.volume),
            format!("{}", t.count),
            format!("{}", t.bid_size),
            format!("{}", t.bid_exchange),
            format_price_f64(t.bid),
            format!("{}", t.bid_condition),
            format!("{}", t.ask_size),
            format!("{}", t.ask_exchange),
            format_price_f64(t.ask),
            format!("{}", t.ask_condition),
        ]);
    }
    td.render(fmt);
}

fn render_ohlc(ticks: &[tdbe::types::tick::OhlcTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date", "time", "open", "high", "low", "close", "volume", "count",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format_price_f64(t.open),
            format_price_f64(t.high),
            format_price_f64(t.low),
            format_price_f64(t.close),
            format!("{}", t.volume),
            format!("{}", t.count),
        ]);
    }
    td.render(fmt);
}

fn render_trades(ticks: &[tdbe::types::tick::TradeTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "time",
        "price",
        "size",
        "exchange",
        "condition",
        "sequence",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format_price_f64(t.price),
            format!("{}", t.size),
            format!("{}", t.exchange),
            format!("{}", t.condition),
            format!("{}", t.sequence),
        ]);
    }
    td.render(fmt);
}

fn render_quotes(ticks: &[tdbe::types::tick::QuoteTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "bid_size",
        "bid_exchange",
        "bid",
        "bid_condition",
        "ask_size",
        "ask_exchange",
        "ask",
        "ask_condition",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{}", t.bid_size),
            format!("{}", t.bid_exchange),
            format_price_f64(t.bid),
            format!("{}", t.bid_condition),
            format!("{}", t.ask_size),
            format!("{}", t.ask_exchange),
            format_price_f64(t.ask),
            format!("{}", t.ask_condition),
        ]);
    }
    td.render(fmt);
}

fn render_string_list(items: &[String], header: &str, fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![header]);
    for s in items {
        td.push(vec![s.clone()]);
    }
    td.render(fmt);
}

fn render_trade_quotes(ticks: &[tdbe::types::tick::TradeQuoteTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "time",
        "price",
        "size",
        "exchange",
        "condition",
        "sequence",
        "quote_time",
        "bid",
        "bid_size",
        "ask",
        "ask_size",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format_price_f64(t.price),
            format!("{}", t.size),
            format!("{}", t.exchange),
            format!("{}", t.condition),
            format!("{}", t.sequence),
            format_ms(t.quote_ms_of_day),
            format_price_f64(t.bid),
            format!("{}", t.bid_size),
            format_price_f64(t.ask),
            format!("{}", t.ask_size),
        ]);
    }
    td.render(fmt);
}

fn render_open_interest(ticks: &[tdbe::types::tick::OpenInterestTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "open_interest"]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{}", t.open_interest),
        ]);
    }
    td.render(fmt);
}

fn render_market_value(ticks: &[tdbe::types::tick::MarketValueTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "market_cap",
        "shares_outstanding",
        "enterprise_value",
        "book_value",
        "free_float",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{}", t.market_cap),
            format!("{}", t.shares_outstanding),
            format!("{}", t.enterprise_value),
            format!("{}", t.book_value),
            format!("{}", t.free_float),
        ]);
    }
    td.render(fmt);
}

fn render_greeks(ticks: &[tdbe::types::tick::GreeksTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "iv",
        "delta",
        "gamma",
        "theta",
        "vega",
        "rho",
    ]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{:.6}", t.implied_volatility),
            format!("{:.6}", t.delta),
            format!("{:.6}", t.gamma),
            format!("{:.6}", t.theta),
            format!("{:.6}", t.vega),
            format!("{:.6}", t.rho),
        ]);
    }
    td.render(fmt);
}

fn render_iv(ticks: &[tdbe::types::tick::IvTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "implied_volatility", "iv_error"]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{:.6}", t.implied_volatility),
            format!("{:.6}", t.iv_error),
        ]);
    }
    td.render(fmt);
}

fn render_price(ticks: &[tdbe::types::tick::PriceTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "price"]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format_price_f64(t.price),
        ]);
    }
    td.render(fmt);
}

fn render_calendar(days: &[tdbe::types::tick::CalendarDay], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "is_open", "open_time", "close_time", "status"]);
    for d in days {
        td.push(vec![
            format_date(d.date),
            format!("{}", d.is_open),
            format_ms(d.open_time),
            format_ms(d.close_time),
            format!("{}", d.status),
        ]);
    }
    td.render(fmt);
}

fn render_interest_rates(ticks: &[tdbe::types::tick::InterestRateTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "rate"]);
    for t in ticks {
        td.push(vec![
            format_date(t.date),
            format_ms(t.ms_of_day),
            format!("{:.6}", t.rate),
        ]);
    }
    td.render(fmt);
}

fn render_option_contracts(contracts: &[tdbe::types::tick::OptionContract], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["root", "expiration", "strike", "right"]);
    for c in contracts {
        td.push(vec![
            c.root.clone(),
            format!("{}", c.expiration),
            format_price_f64(c.strike),
            format!("{}", c.right),
        ]);
    }
    td.render(fmt);
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() {
    let matches = build_cli().get_matches();

    if let Err(e) = run(matches).await {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

// Reason: top-level CLI dispatch across auth, greeks, IV, streaming, and endpoint commands.
#[allow(clippy::too_many_lines)]
async fn run(matches: ArgMatches) -> Result<(), thetadatadx::Error> {
    let fmt = OutputFormat::from_str(
        matches
            .get_one::<String>("format")
            .map_or("table", std::string::String::as_str),
    );
    let creds_path = matches
        .get_one::<String>("creds")
        .map_or("creds.txt", std::string::String::as_str);
    let config_preset = matches
        .get_one::<String>("config")
        .map_or("production", std::string::String::as_str);

    match matches.subcommand() {
        // ── Auth (hand-written) ─────────────────────────────────────
        Some(("auth", _)) => {
            let creds = thetadatadx::Credentials::from_file(creds_path)?;
            let resp = thetadatadx::auth::authenticate(&creds).await?;
            let mut td = TabularData::new(vec![
                "session_id",
                "email",
                "stock_tier",
                "options_tier",
                "indices_tier",
                "rate_tier",
                "created",
            ]);
            let user = resp.user.as_ref();
            let redacted_session = if resp.session_id.len() >= 8 {
                format!("{}...", &resp.session_id[..8])
            } else {
                resp.session_id.clone()
            };
            td.push(vec![
                redacted_session,
                user.and_then(|u| u.email.clone()).unwrap_or_default(),
                user.and_then(|u| u.stock_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.options_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.indices_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                user.and_then(|u| u.interest_rate_subscription)
                    .map(|t| format!("{t}"))
                    .unwrap_or_default(),
                resp.session_created.unwrap_or_default(),
            ]);
            td.render(&fmt);
        }

        // ── Greeks (offline, hand-written) ──────────────────────────
        Some(("greeks", sub_m)) => {
            let spot: f64 = get_arg(sub_m, "spot")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid spot price: {e}")))?;
            let strike: f64 = get_arg(sub_m, "strike")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid strike price: {e}")))?;
            let rate: f64 = get_arg(sub_m, "rate")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid rate: {e}")))?;
            let dividend: f64 = get_arg(sub_m, "dividend")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid dividend: {e}")))?;
            let time: f64 = get_arg(sub_m, "time")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid time: {e}")))?;
            let option_price: f64 = get_arg(sub_m, "option_price")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid option_price: {e}")))?;
            let is_call = get_arg(sub_m, "right") == "call";

            let g =
                tdbe::greeks::all_greeks(spot, strike, rate, dividend, time, option_price, is_call);
            let mut td = TabularData::new(vec!["greek", "value"]);
            let rows = [
                ("value", g.value),
                ("iv", g.iv),
                ("iv_error", g.iv_error),
                ("delta", g.delta),
                ("gamma", g.gamma),
                ("theta", g.theta),
                ("vega", g.vega),
                ("rho", g.rho),
                ("d1", g.d1),
                ("d2", g.d2),
                ("vanna", g.vanna),
                ("charm", g.charm),
                ("vomma", g.vomma),
                ("veta", g.veta),
                ("speed", g.speed),
                ("zomma", g.zomma),
                ("color", g.color),
                ("ultima", g.ultima),
                ("dual_delta", g.dual_delta),
                ("dual_gamma", g.dual_gamma),
                ("epsilon", g.epsilon),
                ("lambda", g.lambda),
            ];
            for (name, val) in rows {
                td.push(vec![name.to_string(), format!("{val:.8}")]);
            }
            td.render(&fmt);
        }

        // ── IV (offline, hand-written) ──────────────────────────────
        Some(("iv", sub_m)) => {
            let spot: f64 = get_arg(sub_m, "spot")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid spot price: {e}")))?;
            let strike: f64 = get_arg(sub_m, "strike")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid strike price: {e}")))?;
            let rate: f64 = get_arg(sub_m, "rate")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid rate: {e}")))?;
            let dividend: f64 = get_arg(sub_m, "dividend")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid dividend: {e}")))?;
            let time: f64 = get_arg(sub_m, "time")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid time: {e}")))?;
            let option_price: f64 = get_arg(sub_m, "option_price")
                .parse()
                .map_err(|e| thetadatadx::Error::Config(format!("invalid option_price: {e}")))?;
            let is_call = get_arg(sub_m, "right") == "call";

            let (iv, iv_error) = tdbe::greeks::implied_volatility(
                spot,
                strike,
                rate,
                dividend,
                time,
                option_price,
                is_call,
            );
            let mut td = TabularData::new(vec!["iv", "iv_error"]);
            td.push(vec![format!("{iv:.8}"), format!("{iv_error:.8}")]);
            td.render(&fmt);
        }

        // ── Dynamic category dispatch (registry-driven) ────────────
        Some((cat, cat_m)) => {
            // Find which endpoint sub-command was invoked
            if let Some((sub_name, sub_m)) = cat_m.subcommand() {
                // Reconstruct the full endpoint name
                let full_name = if cat == "rate" {
                    format!("interest_rate_{sub_name}")
                } else {
                    format!("{cat}_{sub_name}")
                };

                let ep = registry::find(&full_name).ok_or_else(|| {
                    thetadatadx::Error::Config(format!("unknown endpoint: {full_name}"))
                })?;

                let client = connect(creds_path, config_preset).await?;
                dispatch_endpoint(ep, sub_m, &client, &fmt).await?;
            } else {
                // No sub-command: print help for this category
                let mut cmd = build_cli();
                let _ = cmd.find_subcommand_mut(cat).map(clap::Command::print_help);
            }
        }

        None => {
            build_cli().print_help().ok();
        }
    }

    Ok(())
}
