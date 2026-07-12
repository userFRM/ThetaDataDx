//! Response-schema rendering for the endpoint reference pages.
//!
//! The field tables are driven by `tick_schema.toml` (the same registry
//! that generates every binding's tick structs), including the
//! per-column `doc` sentences added for the docs site.

use super::super::super::ticks::schema::{Schema, TickTypeDef};
use super::super::model::GeneratedEndpoint;

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
        "bool" => "bool",
        other => panic!("unmapped tick column type {other}"),
    }
}

fn tick_def<'a>(schema: &'a Schema, collection: &str) -> &'a TickTypeDef {
    let type_name = schema_type_name(collection);
    schema
        .types
        .get(&type_name)
        .unwrap_or_else(|| panic!("tick type {type_name} not found in tick_schema.toml"))
}

/// Render the response-schema section for an endpoint.
pub(super) fn render_response_section(
    endpoint: &GeneratedEndpoint,
    schema: &Schema,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut out = String::new();
    out.push_str("## Response\n\n");

    if endpoint.return_type == "StringList" {
        let column = endpoint
            .list_column
            .as_deref()
            .expect("list endpoint must declare list_column");
        out.push_str(&format!(
            "A list of strings — one `{column}` value per row.\n\n"
        ));
        return Ok(out);
    }

    let type_name = schema_type_name(&endpoint.return_type);
    let def = tick_def(schema, &endpoint.return_type);

    out.push_str(&format!("Rows of `{type_name}`:\n\n"));
    out.push_str("| Field | Type | Description |\n|---|---|---|\n");
    for col in &def.columns {
        let doc = col.doc.as_deref().unwrap_or_else(|| {
            panic!(
                "tick_schema.toml column {type_name}.{} is missing its doc sentence",
                col.name
            )
        });
        out.push_str(&format!(
            "| `{}` | {} | {} |\n",
            col.field,
            docs_field_type(&col.r#type),
            doc
        ));
    }
    if def.contract_id && endpoint.category == "option" {
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
