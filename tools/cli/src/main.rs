use std::process;

use clap::{Arg, ArgMatches, Command};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, ContentArrangement, Table};
use thetadatadx::endpoint::{invoke_endpoint, EndpointArgs, EndpointOutput};
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

            let mut seen_params = std::collections::HashSet::new();
            let mut saw_optional = false;
            for p in ep.params {
                if seen_params.insert(p.name) {
                    // Once we see an optional param, all subsequent must be optional
                    // (clap positional args don't allow required after optional).
                    if !p.required {
                        saw_optional = true;
                    }
                    let required = p.required && !saw_optional;
                    sub_cmd = sub_cmd.arg(Arg::new(p.name).required(required).help(p.description));
                }
            }

            cat_cmd = cat_cmd.subcommand(sub_cmd);
        }

        app = app.subcommand(cat_cmd);
    }

    app
}

/// Extract a string arg from clap matches, or panic with a clear message.
fn get_arg<'a>(m: &'a ArgMatches, name: &str) -> &'a str {
    m.get_one::<String>(name).map_or_else(
        || panic!("missing required argument: {name}"),
        std::string::String::as_str,
    )
}

/// Build validated endpoint arguments from clap matches and registry metadata.
fn build_endpoint_args(
    ep: &EndpointMeta,
    m: &ArgMatches,
) -> Result<EndpointArgs, thetadatadx::Error> {
    let mut args = EndpointArgs::new();
    for param in ep.params {
        match m.get_one::<String>(param.name) {
            Some(raw) => args.insert_raw(param.name, param.param_type, raw)?,
            None if param.required => {
                return Err(thetadatadx::Error::Config(format!(
                    "missing required argument: {}",
                    param.name
                )));
            }
            None => {}
        }
    }
    Ok(args)
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
    let s = format!("{value:.6}");
    // Trim trailing zeros but keep at least 2 decimal places.
    let dot = s.find('.').unwrap();
    let min_len = dot + 3;
    let trimmed = s.trim_end_matches('0');
    if trimmed.len() < min_len {
        s[..min_len].to_string()
    } else {
        trimmed.to_string()
    }
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

fn string_list_header(ep: &EndpointMeta) -> &'static str {
    if ep.name.ends_with("_list_symbols") {
        "symbol"
    } else if ep.name.ends_with("_list_dates") {
        "date"
    } else if ep.name.ends_with("_list_expirations") {
        "expiration"
    } else if ep.name.ends_with("_list_strikes") {
        "strike"
    } else {
        "value"
    }
}

/// Render a shared endpoint runtime result using the CLI formatters.
fn render_output(ep: &EndpointMeta, output: EndpointOutput, fmt: &OutputFormat) {
    match output {
        EndpointOutput::StringList(items) => {
            render_string_list(&items, string_list_header(ep), fmt)
        }
        EndpointOutput::EodTicks(ticks) => render_eod(&ticks, fmt),
        EndpointOutput::OhlcTicks(ticks) => render_ohlc(&ticks, fmt),
        EndpointOutput::TradeTicks(ticks) => render_trades(&ticks, fmt),
        EndpointOutput::QuoteTicks(ticks) => render_quotes(&ticks, fmt),
        EndpointOutput::TradeQuoteTicks(ticks) => render_trade_quotes(&ticks, fmt),
        EndpointOutput::OpenInterestTicks(ticks) => render_open_interest(&ticks, fmt),
        EndpointOutput::MarketValueTicks(ticks) => render_market_value(&ticks, fmt),
        EndpointOutput::GreeksTicks(ticks) => render_greeks(&ticks, fmt),
        EndpointOutput::IvTicks(ticks) => render_iv(&ticks, fmt),
        EndpointOutput::PriceTicks(ticks) => render_price(&ticks, fmt),
        EndpointOutput::CalendarDays(days) => render_calendar(&days, fmt),
        EndpointOutput::InterestRateTicks(ticks) => render_interest_rates(&ticks, fmt),
        EndpointOutput::OptionContracts(contracts) => render_option_contracts(&contracts, fmt),
    }
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
                let args = build_endpoint_args(ep, sub_m)?;
                let output = invoke_endpoint(&client, ep.name, &args).await?;
                render_output(ep, output, &fmt);
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
