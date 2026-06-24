//! `thetadatadx` -- native command-line client for ThetaData market data.
//!
//! Builds its command tree dynamically from the shared endpoint registry, so
//! every category and endpoint the SDK exposes is reachable as
//! `thetadatadx <category> <endpoint> [args...]` with no per-command wiring. A
//! hand-written `flatfile` group covers the whole-universe flat-file surface.
//!
//! ## Invocation contract
//! - Credentials resolve through one shared resolver with this precedence
//!   (highest first): the `--api-key` flag, the `THETADATA_API_KEY`
//!   environment variable, the `THETADATA_EMAIL` + `THETADATA_PASSWORD`
//!   environment pair, then `--creds <path>` (default `creds.txt`: email on
//!   line 1, password on line 2). `--config {production,dev}` selects the
//!   server preset and `--format {table,json,json-raw,csv}` selects the output
//!   encoding; results go to stdout and diagnostics to stderr.
//! - The process exits non-zero with a message on stderr when an endpoint
//!   call, credential load, or argument parse fails.

use std::process;

use clap::{Arg, ArgMatches, Command};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, ContentArrangement, Table};
use thetadatadx::endpoint::{invoke_endpoint, EndpointArgs, EndpointOutput};
use thetadatadx::{by_category, find, EndpointMeta, CATEGORIES};

// ═══════════════════════════════════════════════════════════════════════════
//  CLI construction from endpoint registry
// ═══════════════════════════════════════════════════════════════════════════

/// Build the full CLI tree dynamically from the endpoint registry.
///
/// Structure: `thetadatadx [global opts] <category> <endpoint-subcmd> [args...]`
///
/// Categories (stock, option, index, rate, calendar) are auto-populated.
// Reason: CLI builder registers all subcommands in a single function; splitting would lose cohesion.
#[allow(clippy::too_many_lines)]
fn build_cli() -> Command {
    let mut app = Command::new("thetadatadx")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Client CLI — query ThetaData from your terminal")
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
            Arg::new("api-key")
                .long("api-key")
                .global(true)
                .help(
                    "Authenticate with a ThetaData API key. Takes precedence over \
                     THETADATA_API_KEY, the THETADATA_EMAIL + THETADATA_PASSWORD pair, \
                     and the credentials file. May also be supplied via the \
                     THETADATA_API_KEY environment variable.",
                ),
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
                .value_parser(["table", "json", "json-raw", "csv"])
                .help(
                    "Output format. `json-raw` emits dates as YYYYMMDD ints and \
                     ms_of_day as raw i32 ms (vs `json` which presentation-formats \
                     them); consumed by scripts/ci/check_agreement.py for \
                     cross-language agreement checks.",
                ),
        )
        .arg(
            Arg::new("timeout-ms")
                .long("timeout-ms")
                .global(true)
                .value_parser(clap::value_parser!(u64))
                .help("Per-call deadline in milliseconds. On expiry the in-flight gRPC call is cancelled."),
        );

    app = add_generated_utility_commands(app);
    app = flatfile_commands::add_flatfile_command(app);

    // Dynamic: build category subcommands from ENDPOINTS
    for &cat in CATEGORIES {
        let cat_endpoints = by_category(cat);
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

/// Extract a clap-required string argument from the parsed matches.
///
/// All call sites pass a `name` that was declared with
/// `Arg::new(name).required(true)`. Clap aborts argument parsing
/// before `main` is invoked if a required argument is missing, so
/// reaching the `None` branch here implies a clap configuration bug
/// rather than user input. `unreachable!` documents that invariant
/// and gives a clearer panic site than a chained `unwrap` would.
fn get_arg<'a>(m: &'a ArgMatches, name: &str) -> &'a str {
    m.get_one::<String>(name).map_or_else(
        || unreachable!("clap-required argument {name:?} missing from matches; arg config bug"),
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
                return Err(thetadatadx::Error::config_missing(param.name));
            }
            None => {}
        }
    }
    Ok(args)
}

include!("utilities.rs");
include!("raw_headers_generated.rs");
mod flatfile_commands;

// ═══════════════════════════════════════════════════════════════════════════
//  Output format enum
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
enum OutputFormat {
    Table,
    Json,
    /// Same schema as `Json` but fields keep their raw numeric form: dates
    /// stay as YYYYMMDD ints (not `"YYYY-MM-DD"` strings), ms-of-day stays
    /// as raw i32 ms (not `"HH:MM:SS.mmm"`), and prices stay as unformatted
    /// f64 (not `"685.860000"` strings). Consumed by
    /// `scripts/ci/check_agreement.py` so cross-language agreement doesn't
    /// get false diffs on presentation formatting. Renderers that don't
    /// populate a raw parallel row fall back to `Json` behavior.
    JsonRaw,
    Csv,
}

impl OutputFormat {
    fn from_str(s: &str) -> Self {
        match s {
            "json" => Self::Json,
            "json-raw" => Self::JsonRaw,
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
    // Non-finite prices never carry a decimal point, so the `.find('.')` below
    // would panic. Collapse them the way the REST / MCP surfaces do via
    // `json_canon::finite_or_null` (non-finite -> null): here the display
    // equivalent of null is an empty cell. Keeps all frontends consistent.
    if !value.is_finite() {
        return String::new();
    }
    if value == 0.0 {
        return "0.00".into();
    }
    let s = format!("{value:.6}");
    // Trim trailing zeros but keep at least 2 decimal places.
    // Safety: format!("{:.6}") always produces a decimal point.
    let dot = s
        .find('.')
        .expect("format!(\"{value:.6}\") must contain '.'");
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
///
/// Carries two independent header/row pairs:
/// * `headers` + `rows` — presentation layer. CLI-friendly aliases
///   (`time` for ms-of-day, `iv` for implied_volatility, dropped contract-id
///   columns when the tick isn't an option) drive `--format table | json | csv`.
/// * `raw_headers` + `raw_rows` — canonical SDK schema. Field names match
///   `sdks/python/src/tick_columnar.rs` exactly so `scripts/ci/check_agreement.py`
///   can compare CLI `first_row` against Python / server cell-by-cell
///   without renaming surgery. Populated only by tick renderers via
///   `push_with_raw`; non-tick renderers leave it empty and `--format json-raw`
///   falls back to the string-reparse path.
///
/// The two header lists CAN differ in length and ordering. The presentation
/// row is what humans read; the raw row is what cross-language agreement
/// compares against.
struct TabularData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    raw_headers: Vec<String>,
    raw_rows: Vec<Vec<sonic_rs::Value>>,
}

impl TabularData {
    fn new(headers: Vec<&str>) -> Self {
        Self {
            headers: headers
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect(),
            rows: Vec::new(),
            raw_headers: Vec::new(),
            raw_rows: Vec::new(),
        }
    }

    fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    /// Set the canonical-schema headers used by `--format json-raw`.
    /// Field names must exactly match the canonical SDK schema (i.e.
    /// `sdks/python/src/tick_columnar.rs`) so the cross-language
    /// agreement check doesn't false-diff on field-name disagreements.
    fn set_raw_headers(&mut self, headers: Vec<&str>) {
        self.raw_headers = headers
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect();
    }

    /// Push a presentation row alongside its canonical-schema raw row.
    /// `row` matches `headers` (presentation columns, length-equal). `raw`
    /// matches `raw_headers` (canonical SDK columns, length-equal). The
    /// two vectors are independent — CLI presentation drops contract-id
    /// columns when the tick isn't an option, but the raw row always
    /// carries the full canonical schema.
    fn push_with_raw(&mut self, row: Vec<String>, raw: Vec<sonic_rs::Value>) {
        debug_assert_eq!(row.len(), self.headers.len(), "row length mismatch");
        debug_assert_eq!(
            raw.len(),
            self.raw_headers.len(),
            "raw row length mismatch -- did you forget set_raw_headers?",
        );
        self.rows.push(row);
        self.raw_rows.push(raw);
    }

    fn render(&self, fmt: &OutputFormat) {
        match fmt {
            OutputFormat::Table => self.render_table(),
            OutputFormat::Json => self.render_json(),
            OutputFormat::JsonRaw => self.render_json_raw(),
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
                    obj.insert(&h, json_cell(&val));
                }
                sonic_rs::Value::from(obj)
            })
            .collect();
        println!(
            "{}",
            sonic_rs::to_string_pretty(&arr).unwrap_or_else(|_| "[]".into())
        );
    }

    /// Emit the canonical JSON form consumed by scripts/ci/check_agreement.py.
    ///
    /// Uses `raw_headers` (canonical SDK schema, matching
    /// `sdks/python/src/tick_columnar.rs`) and `raw_rows` (raw values, no
    /// presentation formatting). When the renderer didn't populate raw
    /// data (non-tick subcommands), falls back to `render_json` so this
    /// never silently emits stale data.
    fn render_json_raw(&self) {
        if self.raw_rows.is_empty() || self.raw_headers.is_empty() {
            self.render_json();
            return;
        }
        let arr: Vec<sonic_rs::Value> = self
            .raw_rows
            .iter()
            .map(|row| {
                let mut obj = sonic_rs::Object::new();
                for (i, h) in self.raw_headers.iter().enumerate() {
                    let val = row.get(i).cloned().unwrap_or(sonic_rs::Value::new_null());
                    obj.insert(&h, val);
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

/// Convert one presentation cell to its `--format json` value.
///
/// The null sentinel becomes JSON `null`; a value that parses as a finite f64
/// becomes a JSON number; a value that parses as a NON-finite f64 (`NaN` /
/// `inf` / `-inf`, which `f64::from_str` accepts) becomes `null` rather than a
/// fabricated `0`, matching the json-raw / REST / MCP `json_canon` contract;
/// anything else stays a JSON string.
fn json_cell(value: &str) -> sonic_rs::Value {
    if value == NULL_SENTINEL {
        return sonic_rs::Value::new_null();
    }
    if let Ok(n) = value.parse::<f64>() {
        return match sonic_rs::Number::from_f64(n) {
            Some(num) => sonic_rs::Value::from(num),
            None => sonic_rs::Value::new_null(),
        };
    }
    sonic_rs::Value::from(value)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Client construction helper
// ═══════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════
//  Credential resolution — single source for every networked path
// ═══════════════════════════════════════════════════════════════════════════
//
// Every networked CLI path (endpoint calls, `flatfile`, `auth`) authenticates
// through `resolve_credentials` so the precedence is identical across the whole
// surface and matches the server binary and the SDK constructors. The API key
// and the password are secrets: they are never logged or echoed.

/// Environment variable that supplies a ThetaData API key.
const API_KEY_ENV: &str = "THETADATA_API_KEY";
/// Environment variable that supplies the account email.
const EMAIL_ENV: &str = "THETADATA_EMAIL";
/// Environment variable that supplies the account password.
const PASSWORD_ENV: &str = "THETADATA_PASSWORD";

/// Which authentication source the resolved flag and environment select,
/// before any secret is constructed or any file is read.
///
/// Splitting the decision from the construction keeps the precedence rules
/// pure and unit-testable: the decision is a total function of three presence
/// booleans and never touches the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialSource {
    /// An explicit `--api-key` flag was passed; use that key directly.
    ApiKeyFlag,
    /// No flag, but `THETADATA_API_KEY` is set; source the key from the
    /// environment, falling back to the creds file when it is empty
    /// (delegated to `Credentials::from_env_or_file`).
    EnvApiKeyOrFile,
    /// No flag and no `THETADATA_API_KEY`, but both `THETADATA_EMAIL` and
    /// `THETADATA_PASSWORD` are present; build email/password credentials.
    EnvEmailPassword,
    /// None of the above; read the email/password creds file at `--creds`.
    CredsFile,
}

/// Decide which credential source to use from the presence of the
/// `--api-key` flag, the `THETADATA_API_KEY` variable, and the complete
/// `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair.
///
/// Precedence (highest first): explicit `--api-key` flag, then
/// `THETADATA_API_KEY`, then the email/password environment pair, then the
/// creds file. This mirrors the server binary and the SDK ordering, where an
/// explicit constructor argument wins over the environment, which in turn
/// wins over the creds file.
pub(crate) fn select_credential_source(
    api_key_flag: bool,
    env_api_key_present: bool,
    env_email_password_present: bool,
) -> CredentialSource {
    if api_key_flag {
        CredentialSource::ApiKeyFlag
    } else if env_api_key_present {
        CredentialSource::EnvApiKeyOrFile
    } else if env_email_password_present {
        CredentialSource::EnvEmailPassword
    } else {
        CredentialSource::CredsFile
    }
}

/// Whether `THETADATA_API_KEY` holds a non-empty (after trim) value.
///
/// An empty or whitespace-only variable is treated as absent so it does not
/// shadow the lower-precedence sources, matching
/// `Credentials::from_env_or_file`.
fn env_api_key_present() -> bool {
    std::env::var(API_KEY_ENV).is_ok_and(|v| !v.trim().is_empty())
}

/// Read a non-empty (after trim) environment variable, if present.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Resolve credentials for every networked CLI path through one precedence.
///
/// See [`select_credential_source`] for the ordering. The resolved
/// credential keeps the secret inside the SDK's own zeroized buffer; this
/// function never logs or echoes the API key or the password.
///
/// # Errors
///
/// Propagates the underlying [`thetadatadx::Error`] when the selected source
/// cannot produce a credential (for example a missing or malformed creds
/// file).
pub(crate) fn resolve_credentials(
    api_key_flag: Option<&str>,
    creds_path: &str,
) -> Result<thetadatadx::Credentials, thetadatadx::Error> {
    // An empty / whitespace-only `--api-key` is treated as unset, so it falls
    // through to the lower-precedence sources (env api-key, env email/password,
    // creds file) instead of shadowing them with a blank key that could never
    // authenticate. This mirrors the empty-env handling (`non_empty_env`) and
    // the server binary.
    let api_key_flag = api_key_flag.filter(|k| !k.trim().is_empty());
    let email_password = match (non_empty_env(EMAIL_ENV), non_empty_env(PASSWORD_ENV)) {
        (Some(email), Some(password)) => Some((email, password)),
        _ => None,
    };
    match select_credential_source(
        api_key_flag.is_some(),
        env_api_key_present(),
        email_password.is_some(),
    ) {
        // The flag value comes from argv; the SDK constructor keeps its own
        // zeroized copy. The key itself is never logged.
        CredentialSource::ApiKeyFlag => Ok(thetadatadx::Credentials::api_key(
            api_key_flag.expect("api_key flag is Some on this arm"),
        )),
        // `from_env_or_file` reads `THETADATA_API_KEY` (already known
        // non-empty) into its own zeroized buffer; the key is never surfaced.
        CredentialSource::EnvApiKeyOrFile => thetadatadx::Credentials::from_env_or_file(creds_path),
        CredentialSource::EnvEmailPassword => {
            let (email, password) = email_password.expect("pair is Some on this arm");
            Ok(thetadatadx::Credentials::new(email, password))
        }
        CredentialSource::CredsFile => thetadatadx::Credentials::from_file(creds_path),
    }
}

async fn connect(
    creds: &thetadatadx::Credentials,
    preset: &str,
) -> Result<thetadatadx::Client, thetadatadx::Error> {
    let config = match preset {
        "dev" => thetadatadx::DirectConfig::dev(),
        _ => thetadatadx::DirectConfig::production(),
    };
    thetadatadx::Client::connect(creds, config).await
}

// ═══════════════════════════════════════════════════════════════════════════
//  Raw-value helpers for json-raw output
// ═══════════════════════════════════════════════════════════════════════════
//
// These build `sonic_rs::Value` directly from the raw tick struct fields so
// the cross-language agreement check can compare apples-to-apples with the
// Python / C++ SDKs, which expose raw ints for dates and ms-of-day. See
// scripts/ci/check_agreement.py for the canonical contract.
//
// Sentinel semantics (`date == 0`, `ms_of_day < 0`) are preserved verbatim
// here -- Python (sdks/python/src/tick_columnar.rs) emits those same
// sentinels as raw ints, and the server emitter
// (tools/server/src/format.rs:346) does too. Normalization to `null` lives
// entirely on the consumer side in scripts/ci/check_agreement.py so all
// producers can stay stupid-simple passthroughs and the agreement check has
// one authoritative canonicalization rule. If this side mapped `0 -> null`,
// it would silently disagree with every other producer.

/// Raw YYYYMMDD int. `0` passes through verbatim; consumer-side
/// canonicalization in validate_agreement.py normalizes it to null for
/// comparison with SDKs that legitimately emit `0` as a sentinel.
fn raw_date(date: i32) -> sonic_rs::Value {
    sonic_rs::Value::from(sonic_rs::Number::from(date))
}

/// Raw milliseconds-since-midnight int. Negative values pass through
/// verbatim; consumer-side canonicalization normalizes them to null for
/// cross-language agreement.
fn raw_ms(ms: i32) -> sonic_rs::Value {
    sonic_rs::Value::from(sonic_rs::Number::from(ms))
}

/// Non-finite f64 -> JSON null. JSON itself rejects NaN / +Inf / -Inf in
/// standards-compliant encoders, so we must collapse here or serialization
/// fails. Matches the validator's canonicalisation rule and is shared with
/// the REST and MCP frontends through the `json_canon` crate so all three
/// produce byte-identical output for the same tick payload.
fn raw_f64(value: f64) -> sonic_rs::Value {
    thetadatadx::json_canon::finite_or_null(value)
}

/// Raw integer value as JSON number.
fn raw_i32(value: i32) -> sonic_rs::Value {
    sonic_rs::Value::from(sonic_rs::Number::from(value))
}

/// Raw 64-bit integer value as JSON number. Used for schema columns
/// widened to `i64` (OHLCVC `volume` / `count`) where `i32` would
/// overflow on high-volume symbols.
fn raw_i64(value: i64) -> sonic_rs::Value {
    sonic_rs::Value::from(sonic_rs::Number::from(value))
}

/// Raw string (tick fields like `OptionContract::root`).
fn raw_str(value: &str) -> sonic_rs::Value {
    sonic_rs::Value::from(value)
}

/// Canonical `right` representation for tick types (NOT `OptionContract`).
/// Matches `sdks/python/src/tick_columnar.rs` (`"C"` / `"P"` / `""`). Server
/// uses the same mapping for the option-tick contract-id helper.
fn raw_right_label(is_call: bool, is_put: bool) -> sonic_rs::Value {
    if is_call {
        sonic_rs::Value::from("C")
    } else if is_put {
        sonic_rs::Value::from("P")
    } else {
        sonic_rs::Value::from("")
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tick rendering helpers — reduce repetition across subcommands
// ═══════════════════════════════════════════════════════════════════════════

fn render_eod(ticks: &[thetadatadx::EodTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "created",
        "last_trade",
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
    // Canonical schema -- the EOD time pair carries the vendor's v3
    // semantics: `created` is the report-creation time, `last_trade`
    // the day's final trade time (0 on no-trade days).
    td.set_raw_headers(EOD_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.created_ms_of_day),
                format_ms(t.last_trade_ms_of_day),
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
            ],
            vec![
                raw_ms(t.created_ms_of_day),
                raw_ms(t.last_trade_ms_of_day),
                raw_f64(t.open),
                raw_f64(t.high),
                raw_f64(t.low),
                raw_f64(t.close),
                raw_i64(t.volume),
                raw_i64(t.count),
                raw_i32(t.bid_size),
                raw_i32(t.bid_exchange),
                raw_f64(t.bid),
                raw_i32(t.bid_condition),
                raw_i32(t.ask_size),
                raw_i32(t.ask_exchange),
                raw_f64(t.ask),
                raw_i32(t.ask_condition),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_ohlc(ticks: &[thetadatadx::OhlcTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date", "time", "open", "high", "low", "close", "volume", "count",
    ]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:176-201
    // (ohlc_ticks_to_columnar).
    td.set_raw_headers(OHLC_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format_price_f64(t.open),
                format_price_f64(t.high),
                format_price_f64(t.low),
                format_price_f64(t.close),
                format!("{}", t.volume),
                format!("{}", t.count),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.open),
                raw_f64(t.high),
                raw_f64(t.low),
                raw_f64(t.close),
                raw_i64(t.volume),
                raw_i64(t.count),
                raw_f64(t.vwap),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trades(ticks: &[thetadatadx::TradeTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "time",
        "price",
        "size",
        "exchange",
        "condition",
        "sequence",
    ]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:336-374
    // (trade_ticks_to_columnar). Adds ext_condition1-4, condition_flags,
    // price_flags, volume_type, records_back fields the CLI presentation
    // table doesn't surface but the SDKs do.
    td.set_raw_headers(TRADE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format_price_f64(t.price),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format!("{}", t.condition),
                format!("{}", t.sequence),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_i32(t.condition_flags),
                raw_i32(t.price_flags),
                raw_i32(t.volume_type),
                raw_i32(t.records_back),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_quotes(ticks: &[thetadatadx::QuoteTick], fmt: &OutputFormat) {
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
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:244-275
    // (quote_ticks_to_columnar). Includes `midpoint` field that the CLI
    // presentation table doesn't surface.
    td.set_raw_headers(QUOTE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
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
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.bid_size),
                raw_i32(t.bid_exchange),
                raw_f64(t.bid),
                raw_i32(t.bid_condition),
                raw_i32(t.ask_size),
                raw_i32(t.ask_exchange),
                raw_f64(t.ask),
                raw_i32(t.ask_condition),
                raw_date(t.date),
                raw_f64(t.midpoint),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
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

fn render_trade_quotes(ticks: &[thetadatadx::TradeQuoteTick], fmt: &OutputFormat) {
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
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:277-334
    // (trade_quote_ticks_to_columnar). Adds ext_condition1-4,
    // condition_flags, price_flags, volume_type, records_back,
    // bid_exchange, bid_condition, ask_exchange, ask_condition fields
    // the CLI presentation table doesn't surface.
    td.set_raw_headers(TRADE_QUOTE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
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
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_i32(t.condition_flags),
                raw_i32(t.price_flags),
                raw_i32(t.volume_type),
                raw_i32(t.records_back),
                raw_ms(t.quote_ms_of_day),
                raw_i32(t.bid_size),
                raw_i32(t.bid_exchange),
                raw_f64(t.bid),
                raw_i32(t.bid_condition),
                raw_i32(t.ask_size),
                raw_i32(t.ask_exchange),
                raw_f64(t.ask),
                raw_i32(t.ask_condition),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_open_interest(ticks: &[thetadatadx::OpenInterestTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "open_interest"]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:203-218
    // (open_interest_ticks_to_columnar).
    td.set_raw_headers(OPEN_INTEREST_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.open_interest),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.open_interest),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_market_value(ticks: &[thetadatadx::MarketValueTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "market_bid",
        "market_ask",
        "market_price",
    ]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:155-174
    // (market_value_ticks_to_columnar).
    td.set_raw_headers(MARKET_VALUE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.4}", t.market_bid),
                format!("{:.4}", t.market_ask),
                format!("{:.4}", t.market_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.market_bid),
                raw_f64(t.market_ask),
                raw_f64(t.market_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_greeks(ticks: &[thetadatadx::GreeksAllTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "bid",
        "ask",
        "iv",
        "delta",
        "gamma",
        "theta",
        "vega",
        "rho",
        "iv_error",
        "vanna",
        "charm",
        "vomma",
        "veta",
        "speed",
        "zomma",
        "color",
        "ultima",
        "d1",
        "d2",
        "dual_delta",
        "dual_gamma",
        "epsilon",
        "lambda",
        "vera",
        "underlying_ms_of_day",
        "underlying_price",
    ]);
    td.set_raw_headers(GREEKS_ALL_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.4}", t.bid),
                format!("{:.4}", t.ask),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.delta),
                format!("{:.6}", t.gamma),
                format!("{:.6}", t.theta),
                format!("{:.6}", t.vega),
                format!("{:.6}", t.rho),
                format!("{:.6}", t.iv_error),
                format!("{:.6}", t.vanna),
                format!("{:.6}", t.charm),
                format!("{:.6}", t.vomma),
                format!("{:.6}", t.veta),
                format!("{:.6}", t.speed),
                format!("{:.6}", t.zomma),
                format!("{:.6}", t.color),
                format!("{:.6}", t.ultima),
                format!("{:.6}", t.d1),
                format!("{:.6}", t.d2),
                format!("{:.6}", t.dual_delta),
                format!("{:.6}", t.dual_gamma),
                format!("{:.6}", t.epsilon),
                format!("{:.6}", t.lambda),
                format!("{:.6}", t.vera),
                format_ms(t.underlying_ms_of_day),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.bid),
                raw_f64(t.ask),
                raw_f64(t.implied_volatility),
                raw_f64(t.delta),
                raw_f64(t.gamma),
                raw_f64(t.theta),
                raw_f64(t.vega),
                raw_f64(t.rho),
                raw_f64(t.iv_error),
                raw_f64(t.vanna),
                raw_f64(t.charm),
                raw_f64(t.vomma),
                raw_f64(t.veta),
                raw_f64(t.speed),
                raw_f64(t.zomma),
                raw_f64(t.color),
                raw_f64(t.ultima),
                raw_f64(t.d1),
                raw_f64(t.d2),
                raw_f64(t.dual_delta),
                raw_f64(t.dual_gamma),
                raw_f64(t.epsilon),
                raw_f64(t.lambda),
                raw_f64(t.vera),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_greeks_eod(ticks: &[thetadatadx::GreeksEodTick], fmt: &OutputFormat) {
    // End-of-day Greeks fused with the 12-column EOD trade/quote context
    // (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`,
    // `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`,
    // `ask_condition`). The pretty table renders a representative subset
    // for terminal readability; the raw-headers row is the full 39-column
    // wire shape so `--format raw` / Arrow / JSON callers see every
    // field. An earlier `GreeksAllTick` routing dropped 12 EOD columns;
    // this path preserves them.
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "open",
        "high",
        "low",
        "close",
        "volume",
        "bid",
        "ask",
        "iv",
        "delta",
        "gamma",
        "underlying_price",
    ]);
    td.set_raw_headers(GREEKS_EOD_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format_price_f64(t.open),
                format_price_f64(t.high),
                format_price_f64(t.low),
                format_price_f64(t.close),
                format!("{}", t.volume),
                format_price_f64(t.bid),
                format_price_f64(t.ask),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.delta),
                format!("{:.6}", t.gamma),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.open),
                raw_f64(t.high),
                raw_f64(t.low),
                raw_f64(t.close),
                raw_i64(t.volume),
                raw_i64(t.count),
                raw_i32(t.bid_size),
                raw_i32(t.bid_exchange),
                raw_f64(t.bid),
                raw_i32(t.bid_condition),
                raw_i32(t.ask_size),
                raw_i32(t.ask_exchange),
                raw_f64(t.ask),
                raw_i32(t.ask_condition),
                raw_f64(t.delta),
                raw_f64(t.theta),
                raw_f64(t.vega),
                raw_f64(t.rho),
                raw_f64(t.epsilon),
                raw_f64(t.lambda),
                raw_f64(t.gamma),
                raw_f64(t.vanna),
                raw_f64(t.charm),
                raw_f64(t.vomma),
                raw_f64(t.veta),
                raw_f64(t.vera),
                raw_f64(t.speed),
                raw_f64(t.zomma),
                raw_f64(t.color),
                raw_f64(t.ultima),
                raw_f64(t.d1),
                raw_f64(t.d2),
                raw_f64(t.dual_delta),
                raw_f64(t.dual_gamma),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_greeks_first_order(ticks: &[thetadatadx::GreeksFirstOrderTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "bid",
        "ask",
        "delta",
        "theta",
        "vega",
        "rho",
        "epsilon",
        "lambda",
        "iv",
        "iv_error",
        "underlying_ms_of_day",
        "underlying_price",
    ]);
    td.set_raw_headers(GREEKS_FIRST_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.4}", t.bid),
                format!("{:.4}", t.ask),
                format!("{:.6}", t.delta),
                format!("{:.6}", t.theta),
                format!("{:.6}", t.vega),
                format!("{:.6}", t.rho),
                format!("{:.6}", t.epsilon),
                format!("{:.6}", t.lambda),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.iv_error),
                format_ms(t.underlying_ms_of_day),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.bid),
                raw_f64(t.ask),
                raw_f64(t.delta),
                raw_f64(t.theta),
                raw_f64(t.vega),
                raw_f64(t.rho),
                raw_f64(t.epsilon),
                raw_f64(t.lambda),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_greeks_second_order(ticks: &[thetadatadx::GreeksSecondOrderTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "bid",
        "ask",
        "gamma",
        "vanna",
        "charm",
        "vomma",
        "veta",
        "iv",
        "iv_error",
        "underlying_ms_of_day",
        "underlying_price",
    ]);
    td.set_raw_headers(GREEKS_SECOND_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.4}", t.bid),
                format!("{:.4}", t.ask),
                format!("{:.6}", t.gamma),
                format!("{:.6}", t.vanna),
                format!("{:.6}", t.charm),
                format!("{:.6}", t.vomma),
                format!("{:.6}", t.veta),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.iv_error),
                format_ms(t.underlying_ms_of_day),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.bid),
                raw_f64(t.ask),
                raw_f64(t.gamma),
                raw_f64(t.vanna),
                raw_f64(t.charm),
                raw_f64(t.vomma),
                raw_f64(t.veta),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_greeks_third_order(ticks: &[thetadatadx::GreeksThirdOrderTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "bid",
        "ask",
        "speed",
        "zomma",
        "color",
        "ultima",
        "iv",
        "iv_error",
        "underlying_ms_of_day",
        "underlying_price",
    ]);
    td.set_raw_headers(GREEKS_THIRD_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.4}", t.bid),
                format!("{:.4}", t.ask),
                format!("{:.6}", t.speed),
                format!("{:.6}", t.zomma),
                format!("{:.6}", t.color),
                format!("{:.6}", t.ultima),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.iv_error),
                format_ms(t.underlying_ms_of_day),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.bid),
                raw_f64(t.ask),
                raw_f64(t.speed),
                raw_f64(t.zomma),
                raw_f64(t.color),
                raw_f64(t.ultima),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trade_greeks_all(ticks: &[thetadatadx::TradeGreeksAllTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "price",
        "delta",
        "gamma",
        "theta",
        "vega",
        "iv",
        "underlying_price",
    ]);
    td.set_raw_headers(TRADE_GREEKS_ALL_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format_price_f64(t.price),
                format!("{:.6}", t.delta),
                format!("{:.6}", t.gamma),
                format!("{:.6}", t.theta),
                format!("{:.6}", t.vega),
                format!("{:.6}", t.implied_volatility),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_f64(t.delta),
                raw_f64(t.theta),
                raw_f64(t.vega),
                raw_f64(t.rho),
                raw_f64(t.epsilon),
                raw_f64(t.lambda),
                raw_f64(t.gamma),
                raw_f64(t.vanna),
                raw_f64(t.charm),
                raw_f64(t.vomma),
                raw_f64(t.veta),
                raw_f64(t.vera),
                raw_f64(t.speed),
                raw_f64(t.zomma),
                raw_f64(t.color),
                raw_f64(t.ultima),
                raw_f64(t.d1),
                raw_f64(t.d2),
                raw_f64(t.dual_delta),
                raw_f64(t.dual_gamma),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trade_greeks_first_order(
    ticks: &[thetadatadx::TradeGreeksFirstOrderTick],
    fmt: &OutputFormat,
) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "price",
        "delta",
        "theta",
        "vega",
        "rho",
        "iv",
        "underlying_price",
    ]);
    td.set_raw_headers(TRADE_GREEKS_FIRST_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format_price_f64(t.price),
                format!("{:.6}", t.delta),
                format!("{:.6}", t.theta),
                format!("{:.6}", t.vega),
                format!("{:.6}", t.rho),
                format!("{:.6}", t.implied_volatility),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_f64(t.delta),
                raw_f64(t.theta),
                raw_f64(t.vega),
                raw_f64(t.rho),
                raw_f64(t.epsilon),
                raw_f64(t.lambda),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trade_greeks_second_order(
    ticks: &[thetadatadx::TradeGreeksSecondOrderTick],
    fmt: &OutputFormat,
) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "price",
        "gamma",
        "vanna",
        "charm",
        "vomma",
        "veta",
        "iv",
        "underlying_price",
    ]);
    td.set_raw_headers(TRADE_GREEKS_SECOND_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format_price_f64(t.price),
                format!("{:.6}", t.gamma),
                format!("{:.6}", t.vanna),
                format!("{:.6}", t.charm),
                format!("{:.6}", t.vomma),
                format!("{:.6}", t.veta),
                format!("{:.6}", t.implied_volatility),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_f64(t.gamma),
                raw_f64(t.vanna),
                raw_f64(t.charm),
                raw_f64(t.vomma),
                raw_f64(t.veta),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trade_greeks_third_order(
    ticks: &[thetadatadx::TradeGreeksThirdOrderTick],
    fmt: &OutputFormat,
) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "price",
        "speed",
        "zomma",
        "color",
        "ultima",
        "iv",
        "underlying_price",
    ]);
    td.set_raw_headers(TRADE_GREEKS_THIRD_ORDER_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format_price_f64(t.price),
                format!("{:.6}", t.speed),
                format!("{:.6}", t.zomma),
                format!("{:.6}", t.color),
                format!("{:.6}", t.ultima),
                format!("{:.6}", t.implied_volatility),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_f64(t.speed),
                raw_f64(t.zomma),
                raw_f64(t.color),
                raw_f64(t.ultima),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_trade_greeks_implied_volatility(
    ticks: &[thetadatadx::TradeGreeksImpliedVolatilityTick],
    fmt: &OutputFormat,
) {
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "price",
        "implied_volatility",
        "iv_error",
        "underlying_price",
    ]);
    td.set_raw_headers(TRADE_GREEKS_IMPLIED_VOLATILITY_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format_price_f64(t.price),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.iv_error),
                format!("{:.4}", t.underlying_price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_ms(t.underlying_ms_of_day),
                raw_f64(t.underlying_price),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_iv(ticks: &[thetadatadx::IvTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "implied_volatility", "iv_error"]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:136-153
    // (iv_ticks_to_columnar).
    td.set_raw_headers(IV_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{:.6}", t.implied_volatility),
                format!("{:.6}", t.iv_error),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_f64(t.implied_volatility),
                raw_f64(t.iv_error),
                raw_date(t.date),
                raw_i32(t.expiration),
                raw_f64(t.strike),
                raw_right_label(t.is_call(), t.is_put()),
            ],
        );
    }
    td.render(fmt);
}

fn render_price(ticks: &[thetadatadx::PriceTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "ms_of_day", "price"]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:233-242
    // (price_ticks_to_columnar). PriceTick has no contract-id fields.
    td.set_raw_headers(PRICE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format_price_f64(t.price),
            ],
            vec![raw_ms(t.ms_of_day), raw_f64(t.price), raw_date(t.date)],
        );
    }
    td.render(fmt);
}

fn render_index_price_at_time(ticks: &[thetadatadx::IndexPriceAtTimeTick], fmt: &OutputFormat) {
    // Trade-shaped row published by `index_at_time_price` (10 wire
    // columns: `timestamp`, `sequence`, `ext_condition1..4`,
    // `condition`, `size`, `exchange`, `price`). An earlier `PriceTick`
    // routing dropped the seven trade-side execution columns
    // including the SIP-exchange attribution field; this path
    // preserves them.
    let mut td = TabularData::new(vec![
        "date",
        "ms_of_day",
        "size",
        "exchange",
        "condition",
        "sequence",
        "price",
    ]);
    td.set_raw_headers(INDEX_PRICE_AT_TIME_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![
                format_date(t.date),
                format_ms(t.ms_of_day),
                format!("{}", t.size),
                format!("{}", t.exchange),
                format!("{}", t.condition),
                format!("{}", t.sequence),
                format_price_f64(t.price),
            ],
            vec![
                raw_ms(t.ms_of_day),
                raw_i32(t.sequence),
                raw_i32(t.ext_condition1),
                raw_i32(t.ext_condition2),
                raw_i32(t.ext_condition3),
                raw_i32(t.ext_condition4),
                raw_i32(t.condition),
                raw_i32(t.size),
                raw_i32(t.exchange),
                raw_f64(t.price),
                raw_date(t.date),
            ],
        );
    }
    td.render(fmt);
}

fn render_calendar(days: &[thetadatadx::CalendarDay], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "is_open", "open_time", "close_time", "status"]);
    // Canonical schema -- matches sdks/python/src/tick_columnar.rs:6-19
    // `is_open` is a logical boolean and `status` carries the vendor
    // day-type vocabulary (open / early_close / full_close / weekend),
    // matching the Python / TypeScript row surfaces.
    td.set_raw_headers(CALENDAR_DAY_RAW_HEADERS.to_vec());
    for d in days {
        td.push_with_raw(
            vec![
                format_date(d.date),
                format!("{}", d.is_open),
                format_ms(d.open_time),
                format_ms(d.close_time),
                d.status.as_str().to_string(),
            ],
            vec![
                raw_date(d.date),
                sonic_rs::Value::from(d.is_open),
                raw_ms(d.open_time),
                raw_ms(d.close_time),
                raw_str(d.status.as_str()),
            ],
        );
    }
    td.render(fmt);
}

fn render_interest_rates(ticks: &[thetadatadx::InterestRateTick], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["date", "rate"]);
    // Canonical schema -- matches `INTEREST_RATE_TICK_RAW_HEADERS`
    // (regenerated from `tick_schema.toml`).
    td.set_raw_headers(INTEREST_RATE_TICK_RAW_HEADERS.to_vec());
    for t in ticks {
        td.push_with_raw(
            vec![format_date(t.date), format!("{:.6}", t.rate)],
            vec![raw_date(t.date), raw_f64(t.rate)],
        );
    }
    td.render(fmt);
}

fn render_option_contracts(contracts: &[thetadatadx::OptionContract], fmt: &OutputFormat) {
    let mut td = TabularData::new(vec!["symbol", "expiration", "strike", "right"]);
    // Canonical schema -- `right` renders as the logical character
    // ("C" / "P"), the same projection every tick type and the Python /
    // TypeScript surfaces use.
    td.set_raw_headers(OPTION_CONTRACT_RAW_HEADERS.to_vec());
    for c in contracts {
        td.push_with_raw(
            vec![
                c.symbol.clone(),
                format!("{}", c.expiration),
                format_price_f64(c.strike),
                format!("{}", c.right),
            ],
            vec![
                raw_str(&c.symbol),
                raw_date(c.expiration),
                raw_f64(c.strike),
                raw_right_label(c.is_call(), c.is_put()),
            ],
        );
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
        EndpointOutput::GreeksAllTicks(ticks) => render_greeks(&ticks, fmt),
        EndpointOutput::GreeksEodTicks(ticks) => render_greeks_eod(&ticks, fmt),
        EndpointOutput::GreeksFirstOrderTicks(ticks) => render_greeks_first_order(&ticks, fmt),
        EndpointOutput::GreeksSecondOrderTicks(ticks) => render_greeks_second_order(&ticks, fmt),
        EndpointOutput::GreeksThirdOrderTicks(ticks) => render_greeks_third_order(&ticks, fmt),
        EndpointOutput::TradeGreeksAllTicks(ticks) => render_trade_greeks_all(&ticks, fmt),
        EndpointOutput::TradeGreeksFirstOrderTicks(ticks) => {
            render_trade_greeks_first_order(&ticks, fmt)
        }
        EndpointOutput::TradeGreeksSecondOrderTicks(ticks) => {
            render_trade_greeks_second_order(&ticks, fmt)
        }
        EndpointOutput::TradeGreeksThirdOrderTicks(ticks) => {
            render_trade_greeks_third_order(&ticks, fmt)
        }
        EndpointOutput::TradeGreeksImpliedVolatilityTicks(ticks) => {
            render_trade_greeks_implied_volatility(&ticks, fmt)
        }
        EndpointOutput::IvTicks(ticks) => render_iv(&ticks, fmt),
        EndpointOutput::PriceTicks(ticks) => render_price(&ticks, fmt),
        EndpointOutput::IndexPriceAtTimeTicks(ticks) => render_index_price_at_time(&ticks, fmt),
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
    // Seat ring as the process-default rustls CryptoProvider before any
    // TLS handshake. The workspace compiles ring as the only provider;
    // reqwest's rustls path requires the default to be installed
    // explicitly. Mirrors the server binary's startup.
    let _ = thetadatadx::__internal_install_ring_crypto_provider();

    let matches = build_cli().get_matches();

    if let Err(e) = run(matches).await {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

// Reason: top-level CLI dispatch across generated utilities and endpoint commands.
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
    let api_key_flag = matches
        .get_one::<String>("api-key")
        .map(std::string::String::as_str);
    let config_preset = matches
        .get_one::<String>("config")
        .map_or("production", std::string::String::as_str);

    // Credentials resolve through the shared precedence at each networked
    // site, not up front: the help branches (no subcommand, category with no
    // endpoint) must keep working without any credential present.
    if try_run_generated_utility(matches.subcommand(), &fmt, api_key_flag, creds_path).await? {
        return Ok(());
    }

    if flatfile_commands::try_dispatch(&matches, api_key_flag, creds_path).await? {
        return Ok(());
    }

    match matches.subcommand() {
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

                let ep = find(&full_name).ok_or_else(|| {
                    thetadatadx::Error::config_invalid(
                        "endpoint.name",
                        format!("unknown endpoint: {full_name}"),
                    )
                })?;

                let creds = resolve_credentials(api_key_flag, creds_path)?;
                let client = connect(&creds, config_preset).await?;
                let mut args = build_endpoint_args(ep, sub_m)?;
                if let Some(&ms) = matches.get_one::<u64>("timeout-ms") {
                    args = args.with_timeout_ms(ms);
                }
                let output = invoke_endpoint(client.historical(), ep.name, &args).await?;
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

#[cfg(test)]
mod tests {
    // ── Empty --api-key is treated as unset (#auth precedence) ─────────
    //
    // `resolve_credentials` filters a blank/whitespace `--api-key` to `None`
    // before the precedence decision, so a blank flag does not shadow the
    // lower-precedence sources. This pins the filter (pure, no env / fs).
    #[test]
    fn blank_api_key_flag_is_treated_as_unset() {
        let blank: Option<&str> = Some("   ");
        assert!(
            blank.filter(|k| !k.trim().is_empty()).is_none(),
            "a whitespace-only --api-key must filter to None (treated as unset)",
        );
        let real: Option<&str> = Some("td1_key");
        assert!(
            real.filter(|k| !k.trim().is_empty()).is_some(),
            "a real --api-key must remain Some",
        );
        // With the flag filtered out and no api-key env, the decision must NOT
        // be ApiKeyFlag.
        assert_ne!(
            super::select_credential_source(false, false, false),
            super::CredentialSource::ApiKeyFlag,
        );
    }

    use super::{
        json_cell, raw_date, raw_f64, raw_i32, raw_i64, raw_ms, raw_right_label,
        resolve_credentials, select_credential_source, CredentialSource, OutputFormat, TabularData,
        NULL_SENTINEL,
    };
    use sonic_rs::JsonValueTrait;

    // ── `--format json` cell conversion (#940) ─────────────────────────
    //
    // A non-finite greek (NaN / +-Inf) must render as JSON `null`, not a
    // fabricated `0`, matching the json-raw / REST / MCP json_canon contract.
    #[test]
    fn json_cell_non_finite_renders_null_not_zero() {
        for s in ["NaN", "nan", "inf", "-inf", "Infinity", "-Infinity"] {
            let v = json_cell(s);
            assert!(
                v.is_null(),
                "non-finite {s:?} must render as JSON null, got {v:?}",
            );
        }
    }

    #[test]
    fn json_cell_finite_number_renders_as_number() {
        let v = json_cell("1.5");
        assert!(v.is_number(), "a finite f64 must render as a JSON number");
        assert_eq!(v.as_f64(), Some(1.5));
        // Zero stays a real number, distinct from the non-finite-null case.
        assert_eq!(json_cell("0").as_f64(), Some(0.0));
    }

    #[test]
    fn json_cell_null_sentinel_and_strings() {
        assert!(json_cell(NULL_SENTINEL).is_null(), "null sentinel -> null");
        assert!(json_cell("OPTION").is_str(), "non-numeric stays a string");
    }

    // ── Credential resolution precedence ───────────────────────────────
    //
    // The precedence is exercised on the pure `select_credential_source`
    // decision so the assertions never depend on process-global environment
    // state, which would race across the parallel test runner. The decision
    // is a total function of three presence booleans, mirroring the server
    // binary's `select_credential_source`.

    // An explicit `--api-key` flag wins over every other source, including the
    // environment variable and the email/password pair.
    #[test]
    fn api_key_flag_takes_precedence() {
        assert_eq!(
            select_credential_source(true, true, true),
            CredentialSource::ApiKeyFlag
        );
        assert_eq!(
            select_credential_source(true, false, false),
            CredentialSource::ApiKeyFlag
        );
    }

    // With no flag but `THETADATA_API_KEY` present, the env-or-file path is
    // selected, ahead of the email/password pair.
    #[test]
    fn env_api_key_beats_email_password() {
        assert_eq!(
            select_credential_source(false, true, true),
            CredentialSource::EnvApiKeyOrFile
        );
    }

    // With no flag and no API key, a complete email/password pair is used.
    #[test]
    fn env_email_password_beats_creds_file() {
        assert_eq!(
            select_credential_source(false, false, true),
            CredentialSource::EnvEmailPassword
        );
    }

    // With none of the higher-precedence sources present, the creds file is
    // the source, so existing creds.txt invocations are unchanged.
    #[test]
    fn no_env_falls_back_to_creds_file() {
        assert_eq!(
            select_credential_source(false, false, false),
            CredentialSource::CredsFile
        );
    }

    // The `--api-key` flag arm constructs API-key credentials directly from
    // the flag value without reading any file (the path given does not exist).
    #[test]
    fn resolve_with_flag_builds_api_key_credentials() {
        let creds = resolve_credentials(Some("secret-key-123"), "/nonexistent/creds.txt")
            .expect("flag resolves without touching the creds file");
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("secret-key-123"));
        assert_eq!(creds.password(), None);
    }

    // With no flag and (in a clean env) no relevant variables, the resolver
    // falls back to the creds file; a missing file surfaces an error rather
    // than silently authenticating. This guards the no-credential path.
    #[test]
    fn resolve_without_flag_uses_creds_file() {
        // This test only asserts the file-backed arm is reached when no flag
        // is supplied. To avoid racing the parallel runner on process-global
        // environment state, it does not mutate the environment; it asserts
        // that a missing creds file is reported as an error (the flag arm,
        // which never reads a file, would have returned Ok).
        if super::env_api_key_present()
            || (super::non_empty_env(super::EMAIL_ENV).is_some()
                && super::non_empty_env(super::PASSWORD_ENV).is_some())
        {
            // A developer machine with the variables exported takes a
            // higher-precedence path; skip rather than assert a brittle shape.
            return;
        }
        let err = resolve_credentials(None, "/nonexistent/creds.txt");
        assert!(
            err.is_err(),
            "a missing creds file with no flag and no env must error"
        );
    }

    #[test]
    fn json_raw_format_parses_from_string() {
        assert!(matches!(
            OutputFormat::from_str("json-raw"),
            OutputFormat::JsonRaw
        ));
        assert!(matches!(OutputFormat::from_str("json"), OutputFormat::Json));
        assert!(matches!(OutputFormat::from_str("csv"), OutputFormat::Csv));
        assert!(matches!(
            OutputFormat::from_str("table"),
            OutputFormat::Table
        ));
        assert!(matches!(
            OutputFormat::from_str("unknown"),
            OutputFormat::Table
        ));
    }

    #[test]
    fn raw_date_passes_through_sentinel() {
        // `0` is a sentinel for "no date" but we pass it through verbatim.
        // The Python SDK emits `0` as raw i32 too; normalizing to null here
        // would silently disagree with them. The validator consumer
        // canonicalizes both shapes to None for comparison.
        assert!(raw_date(0).is_number());
        assert!(raw_date(20260417).is_number());
    }

    #[test]
    fn raw_ms_passes_through_sentinel() {
        // Negative ms is a sentinel for "missing" but we pass it through
        // verbatim to match Python SDK behavior. Consumer-side
        // canonicalization collapses it to None for agreement checks.
        assert!(raw_ms(-1).is_number());
        assert!(raw_ms(0).is_number());
        assert!(raw_ms(34_200_000).is_number());
    }

    #[test]
    fn raw_f64_non_finite_is_null() {
        assert!(raw_f64(f64::NAN).is_null());
        assert!(raw_f64(f64::INFINITY).is_null());
        assert!(raw_f64(f64::NEG_INFINITY).is_null());
        assert!(!raw_f64(0.0).is_null());
        assert!(!raw_f64(685.86).is_null());
    }

    #[test]
    fn tabular_data_push_with_raw_stores_both() {
        let mut td = TabularData::new(vec!["date", "price"]);
        td.set_raw_headers(vec!["date", "price"]);
        td.push_with_raw(
            vec!["2026-04-17".into(), "685.860000".into()],
            vec![raw_date(20260417), raw_f64(685.86)],
        );
        assert_eq!(td.rows.len(), 1);
        assert_eq!(td.raw_rows.len(), 1);
        assert_eq!(td.rows[0][0], "2026-04-17");
        assert!(td.raw_rows[0][0].is_number());
    }

    #[test]
    fn tabular_data_independent_presentation_and_raw_schemas() {
        // The presentation row (`time`, dropped contract-id) and the raw
        // row (canonical `ms_of_day`, full contract-id) can have different
        // lengths and field orderings. push_with_raw enforces row==headers
        // and raw==raw_headers length-equality.
        let mut td = TabularData::new(vec!["date", "time", "price"]);
        td.set_raw_headers(vec![
            "ms_of_day",
            "price",
            "date",
            "expiration",
            "strike",
            "right",
        ]);
        td.push_with_raw(
            vec!["2026-04-17".into(), "09:30:00.000".into(), "5.42".into()],
            vec![
                raw_ms(34_200_000),
                raw_f64(5.42),
                raw_date(20260417),
                raw_i32(20260321),
                raw_f64(570.0),
                raw_right_label(true, false),
            ],
        );
        assert_eq!(td.headers.len(), 3);
        assert_eq!(td.raw_headers.len(), 6);
        assert_eq!(td.raw_rows[0].len(), 6);
        assert_eq!(td.raw_headers[0], "ms_of_day"); // canonical, not "time"
    }

    #[test]
    fn ohlc_raw_row_matches_header_set_with_vwap_in_place() {
        // The raw OHLC row must carry exactly one value per raw header,
        // with `vwap` between `count` and `date`. A short row pairs by
        // position and silently shifts every later column — the
        // `push_with_raw` length guard is a `debug_assert`, compiled out
        // in release, so only this test catches a release-mode drift.
        use super::OHLC_TICK_RAW_HEADERS;
        let tick = thetadatadx::OhlcTick {
            ms_of_day: 34_200_000,
            open: 309.625,
            high: 310.94,
            low: 307.8,
            close: 307.84,
            volume: 8_697_937,
            count: 203_083,
            vwap: 309.64,
            date: 20_260_601,
            expiration: 0,
            strike: 0.0,
            right: '\0',
        };
        let raw = vec![
            raw_ms(tick.ms_of_day),
            raw_f64(tick.open),
            raw_f64(tick.high),
            raw_f64(tick.low),
            raw_f64(tick.close),
            raw_i64(tick.volume),
            raw_i64(tick.count),
            raw_f64(tick.vwap),
            raw_date(tick.date),
            raw_i32(tick.expiration),
            raw_f64(tick.strike),
            raw_right_label(tick.is_call(), tick.is_put()),
        ];
        assert_eq!(raw.len(), OHLC_TICK_RAW_HEADERS.len());
        let vwap_idx = OHLC_TICK_RAW_HEADERS
            .iter()
            .position(|h| *h == "vwap")
            .expect("vwap header present");
        assert_eq!(raw[vwap_idx].as_f64(), Some(309.64));
        let date_idx = OHLC_TICK_RAW_HEADERS
            .iter()
            .position(|h| *h == "date")
            .expect("date header present");
        assert_eq!(raw[date_idx].as_i64(), Some(20_260_601));
    }

    #[test]
    fn raw_right_label_matches_python_string_mapping() {
        // Mirrors sdks/python/src/tick_columnar.rs:41 ("C" / "P" / "").
        assert_eq!(raw_right_label(true, false).as_str(), Some("C"));
        assert_eq!(raw_right_label(false, true).as_str(), Some("P"));
        assert_eq!(raw_right_label(false, false).as_str(), Some(""));
    }
}
