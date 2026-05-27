//! Column-extraction helpers (Number / Text / Price) over a `DataTable`.
//!
//! These helpers return `Vec<Option<T>>` keyed by the column header. They
//! drive the macro-generated list endpoints in `crate::macros` and the
//! Polars / Arrow column projections.
//!
//! Header lookup is alias-aware: each helper routes through the private
//! `super::headers::find_header` helper so v3 MDDS column renames (e.g.
//! `symbol` → `root`, `timestamp` → `ms_of_day`) resolve through the
//! shared `HEADER_ALIASES` table instead of returning a silent empty
//! `Vec` when the server-side rename lands.

use crate::proto;

use super::headers::find_header;

/// Resolve a column index honouring the shared `HEADER_ALIASES`
/// table, then warn-and-return-empty when the column is missing from a
/// non-empty `DataTable`. The macro-generated parsers surface the
/// stricter [`crate::mdds::decode::DecodeError::MissingRequiredHeader`]
/// when the same column is declared `required`; these direct extractors
/// keep the existing `Vec<Option<T>>` shape so call sites that chain
/// `.into_iter().flatten().collect()` stay unchanged.
///
/// An empty `DataTable` (no rows) is a legitimate "no data today"
/// outcome and never warns.
fn resolve_column(
    table: &proto::DataTable,
    header: &str,
    column_kind: &'static str,
) -> Option<usize> {
    // Resolve through the alias-aware helper. `find_header` borrows
    // `&[&str]`; build the borrowed view on the stack so we honour
    // the shared alias table without cloning the header strings.
    let header_refs: Vec<&str> = table.headers.iter().map(String::as_str).collect();
    if let Some(idx) = find_header(&header_refs, header) {
        return Some(idx);
    }
    if !table.data_table.is_empty() {
        tracing::warn!(
            target: "thetadatadx::mdds::decode::extract",
            requested = header,
            column_kind,
            available = ?table.headers,
            rows = table.data_table.len(),
            "DataTable missing requested column (no alias match); returning empty Vec",
        );
    }
    None
}

/// Extract a column of i64 values from a `DataTable` by header name.
///
/// Honours the shared `HEADER_ALIASES` table so v3-renamed columns
/// resolve to the schema-side name. Returns an empty `Vec` when no
/// column matches (with a `warn` emitted on non-empty tables).
#[must_use]
pub fn extract_number_column(table: &proto::DataTable, header: &str) -> Vec<Option<i64>> {
    let Some(col_idx) = resolve_column(table, header, "Number") else {
        return vec![];
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Number(n) => Some(*n),
                    _ => None,
                })
        })
        .collect()
}

/// Extract a column of string values from a `DataTable` by header name.
///
/// Honours the shared `HEADER_ALIASES` table so v3-renamed columns
/// resolve to the schema-side name. Returns an empty `Vec` when no
/// column matches (with a `warn` emitted on non-empty tables).
///
/// `Price` cells route through the strict
/// [`tdbe::types::price::Price::with_value_and_type`] constructor so a
/// `price_type` outside the documented `0..=19` range yields `None` for
/// that row (and emits a `tracing::warn!`) instead of silently snapping
/// to `19` and producing a fabricated magnitude in the rendered text.
#[must_use]
pub fn extract_text_column(table: &proto::DataTable, header: &str) -> Vec<Option<String>> {
    let Some(col_idx) = resolve_column(table, header, "Text") else {
        return vec![];
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Text(s) => Some(s.clone()),
                    proto::data_value::DataType::Number(n) => Some(n.to_string()),
                    proto::data_value::DataType::Price(p) => {
                        match tdbe::types::price::Price::with_value_and_type(p.value, p.r#type) {
                            Ok(price) => Some(format!("{}", price.to_f64())),
                            Err(err) => {
                                tracing::warn!(
                                    target: "thetadatadx::mdds::decode::extract",
                                    requested = header,
                                    column_kind = "Text",
                                    price_value = p.value,
                                    price_type = p.r#type,
                                    error = %err,
                                    "dropping Price cell with out-of-range price_type from text \
                                     column; downstream sees None instead of a silently clamped \
                                     magnitude",
                                );
                                None
                            }
                        }
                    }
                    _ => None,
                })
        })
        .collect()
}

/// Extract a column of Price values from a `DataTable` by header name.
///
/// Honours the shared `HEADER_ALIASES` table so v3-renamed columns
/// resolve to the schema-side name. Returns an empty `Vec` when no
/// column matches (with a `warn` emitted on non-empty tables).
///
/// `Price` cells route through the strict
/// [`tdbe::types::price::Price::with_value_and_type`] constructor so a
/// `price_type` outside the documented `0..=19` range yields `None` for
/// that row (with a `tracing::warn!` emitted) rather than silently
/// snapping to `19` and surfacing a fabricated downstream magnitude.
/// `None` is the public schema-drift signal for this column shape; the
/// `Vec<Option<tdbe::Price>>` signature is preserved so the upstream
/// `.into_iter().flatten().collect()` chains keep working.
#[must_use]
pub fn extract_price_column(table: &proto::DataTable, header: &str) -> Vec<Option<tdbe::Price>> {
    let Some(col_idx) = resolve_column(table, header, "Price") else {
        return vec![];
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Price(p) => {
                        match tdbe::types::price::Price::with_value_and_type(p.value, p.r#type) {
                            Ok(price) => Some(price),
                            Err(err) => {
                                tracing::warn!(
                                    target: "thetadatadx::mdds::decode::extract",
                                    requested = header,
                                    column_kind = "Price",
                                    price_value = p.value,
                                    price_type = p.r#type,
                                    error = %err,
                                    "dropping Price cell with out-of-range price_type; downstream \
                                     sees None instead of a silently clamped magnitude",
                                );
                                None
                            }
                        }
                    }
                    _ => None,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `extract_text_column` must resolve the schema-side `root`
    /// against an upstream-renamed `symbol` column via
    /// the shared `HEADER_ALIASES` table. Without the
    /// alias-aware path this returned a silent empty Vec on every
    /// v3 list-symbols response.
    #[test]
    fn extract_text_column_resolves_via_header_alias() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: vec![
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Text("AAPL".into())),
                    }],
                },
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Text("MSFT".into())),
                    }],
                },
            ],
        };
        // Schema-side name is `root`; alias entry routes to `symbol`.
        let column = extract_text_column(&table, "root");
        assert_eq!(
            column,
            vec![Some("AAPL".to_string()), Some("MSFT".to_string())],
            "alias-aware lookup must return the column even when only \
             the server-side name is present"
        );
    }

    /// Empty `DataTable` returns empty Vec without alias resolution —
    /// no rows means no warning either.
    #[test]
    fn extract_text_column_empty_table_returns_empty_vec() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: Vec::new(),
        };
        assert_eq!(
            extract_text_column(&table, "missing"),
            Vec::<Option<String>>::new()
        );
    }

    /// Number / Price extractors also honour the alias table — a
    /// regression that only fixed one of the three would slip through.
    #[test]
    fn extract_number_column_resolves_via_header_alias() {
        let table = proto::DataTable {
            headers: vec!["timestamp".to_string()],
            data_table: vec![proto::DataValueList {
                values: vec![proto::DataValue {
                    data_type: Some(proto::data_value::DataType::Number(123)),
                }],
            }],
        };
        // Schema-side `ms_of_day` aliases to `timestamp`.
        let column = extract_number_column(&table, "ms_of_day");
        assert_eq!(column, vec![Some(123)]);
    }

    // ─── Round-2 hardening: column extractors reject out-of-range price_type ───
    //
    // Round-1 closed the silent-clamp hole at the per-cell decode boundary
    // (`row_price_f64` / `row_number_i64`) but `extract_price_column` and
    // `extract_text_column` still went through `tdbe::Price::new`, which
    // saturates `price_type` to `19` instead of erroring. A drifted upstream
    // payload with `price_type = 20` therefore produced a fabricated
    // magnitude downstream (the value scaled by `10^9` instead of being
    // dropped). The column-bulk extractors now route through
    // `Price::with_value_and_type` and yield `None` on schema drift while
    // emitting a `tracing::warn!` for operator visibility.

    /// Build a `DataTable` with one column of `Price` cells.
    fn price_table(header: &str, cells: &[(i32, i32)]) -> proto::DataTable {
        proto::DataTable {
            headers: vec![header.to_string()],
            data_table: cells
                .iter()
                .map(|(v, t)| proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Price(proto::Price {
                            value: *v,
                            r#type: *t,
                        })),
                    }],
                })
                .collect(),
        }
    }

    #[test]
    fn extract_price_column_drops_price_type_20() {
        // The flagged drift case: a `price_type` one above MAX_PRICE_TYPE
        // (= 19) used to saturate silently and produce a 10x-larger
        // downstream magnitude. The strict constructor now drops the row
        // and `tracing::warn!` documents the schema drift.
        let table = price_table("price", &[(100, 20)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_price_column_drops_price_type_21() {
        let table = price_table("price", &[(100, 21)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_price_column_drops_price_type_i32_max() {
        let table = price_table("price", &[(100, i32::MAX)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_price_column_drops_negative_price_type() {
        let table = price_table("price", &[(100, -1)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_price_column_keeps_price_type_19_at_boundary() {
        // MAX_PRICE_TYPE itself must still produce `Some(Price)` — the
        // boundary is inclusive.
        let table = price_table("price", &[(100, 19)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col.len(), 1);
        let p = col[0].expect("price_type=19 is in range and must round-trip");
        assert_eq!(p.value(), 100);
        assert_eq!(p.price_type(), 19);
    }

    #[test]
    fn extract_price_column_keeps_typical_price_type_10() {
        // Sanity: typical vendor `price_type = 10` (no scaling) still
        // round-trips after the strict-constructor switch.
        let table = price_table("price", &[(12345, 10)]);
        let col = extract_price_column(&table, "price");
        let p = col[0].expect("in-range price must round-trip");
        assert_eq!(p.value(), 12345);
        assert_eq!(p.price_type(), 10);
    }

    #[test]
    fn extract_text_column_drops_price_with_out_of_range_price_type() {
        // `extract_text_column` stringifies `Price` cells through the
        // same constructor; the round-1 fix missed this path too.
        let table = price_table("price", &[(100, 20)]);
        let col = extract_text_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_text_column_keeps_price_at_boundary() {
        let table = price_table("price", &[(100, 19)]);
        let col = extract_text_column(&table, "price");
        assert_eq!(col.len(), 1);
        assert!(col[0].is_some(), "price_type=19 must still render to text");
    }
}
