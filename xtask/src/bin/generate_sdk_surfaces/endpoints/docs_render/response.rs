//! Response-schema rendering for the endpoint reference pages.
//!
//! The field tables are driven by the tick table (the same table every
//! binding's tick types expand from), including the per-column description
//! sentences.

use super::super::super::ticks::table::tick_for_collection;
use super::super::table::GeneratedEndpoint;

/// Singular tick-type name for a wire-collection plural
/// (`"QuoteTicks"` → `"QuoteTick"`, `"CalendarDays"` → `"CalendarDay"`).
pub(super) fn schema_type_name(collection: &str) -> String {
    collection
        .strip_suffix('s')
        .unwrap_or_else(|| panic!("collection {collection} does not end in 's'"))
        .to_string()
}

/// Representative fields printed by the runnable samples, per
/// wire-collection. Two to four fields that identify the row at a
/// glance; the full field set is documented in the schema table.
pub(super) fn display_fields(collection: &str) -> &'static [&'static str] {
    match collection {
        "TradeTicks" => &["date", "ms_of_day", "price", "size"],
        "QuoteTicks" => &["date", "ms_of_day", "bid", "ask"],
        "OhlcTicks" => &["date", "open", "high", "low", "close"],
        "EodTicks" => &["date", "open", "close", "volume"],
        "TradeQuoteTicks" => &["ms_of_day", "price", "bid", "ask"],
        "OpenInterestTicks" => &["date", "open_interest"],
        "IntervalQuoteTicks" => &["interval_ms_of_day", "bid", "ask"],
        "MarketValueTicks" => &["date", "market_price"],
        "IvTicks" => &["date", "implied_volatility", "iv_error"],
        "GreeksAllTicks" => &["date", "delta", "gamma", "theta", "implied_volatility"],
        "GreeksEodTicks" => &["date", "close", "delta", "implied_volatility"],
        "GreeksFirstOrderTicks" => &["date", "delta", "theta", "vega"],
        "GreeksSecondOrderTicks" => &["date", "gamma", "vanna", "charm"],
        "GreeksThirdOrderTicks" => &["date", "speed", "zomma", "color"],
        "TradeGreeksAllTicks" => &["ms_of_day", "price", "delta", "implied_volatility"],
        "TradeGreeksFirstOrderTicks" => &["ms_of_day", "price", "delta", "theta"],
        "TradeGreeksSecondOrderTicks" => &["ms_of_day", "price", "gamma", "vanna"],
        "TradeGreeksThirdOrderTicks" => &["ms_of_day", "price", "speed", "zomma"],
        "TradeGreeksImpliedVolatilityTicks" => &["ms_of_day", "price", "implied_volatility"],
        "PriceTicks" => &["date", "ms_of_day", "price"],
        "IndexPriceAtTimeTicks" => &["date", "ms_of_day", "price"],
        "CalendarDays" => &["date", "open_time", "close_time", "status"],
        "InterestRateTicks" => &["date", "rate"],
        "OptionContracts" => &["symbol", "expiration", "strike", "right"],
        other => panic!("no display fields declared for collection {other}"),
    }
}

/// Field type as rendered on the docs schema table: the cross-language
/// scalar shape, not any one binding's name.
fn docs_field_type(column_type: &str) -> &'static str {
    match column_type {
        "i32" | "eod_num" | "eod_date" => "i32",
        "i64" | "eod_num64" => "i64",
        "f64" | "price" | "eod_price" => "f64",
        // Logical columns document their cross-language shape: the
        // option right is a one-character string ("C" / "P"), the
        // calendar day type is the vendor vocabulary string.
        "String" | "right" | "calendar_status" => "string",
        other => panic!("unmapped tick column type {other}"),
    }
}

/// Render the response-schema section for an endpoint.
pub(super) fn render_response_section(
    endpoint: &GeneratedEndpoint,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut out = String::new();
    out.push_str("## Response\n\n");

    if endpoint.return_type == "StringList" {
        let column = endpoint
            .list_column
            .expect("list endpoint must declare list_column");
        out.push_str(&format!(
            "A list of strings, one `{column}` value per row.\n\n"
        ));
        return Ok(out);
    }

    let tick = tick_for_collection(endpoint.return_type);

    out.push_str(&format!("Rows of `{}`:\n\n", tick.name));
    out.push_str("| Field | Type | Description |\n|---|---|---|\n");
    for col in tick.columns {
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            col.field,
            docs_field_type(col.ty),
            col.doc
        ));
    }
    if tick.contract_id && endpoint.category == "option" && endpoint.subcategory == "flat_file" {
        out.push_str(
            "\nEvery row carries `expiration` (YYYYMMDD), `strike` (dollars) and `right` \
             (\"C\" / \"P\"); with the row's root, which the whole result carries per row, \
             they identify the contract.\n",
        );
    } else if tick.contract_id && endpoint.category == "option" {
        out.push_str(
            "\nWildcard requests additionally populate `expiration` (YYYYMMDD), `strike` \
             (dollars), and `right` (\"C\" / \"P\") on every row to identify the contract; \
             on single-contract requests these are absent (None / null / undefined; the Rust \
             and C rows carry the documented `0` / `0.0` / `'\\0'` fills).\n",
        );
    }
    out.push('\n');
    Ok(out)
}
