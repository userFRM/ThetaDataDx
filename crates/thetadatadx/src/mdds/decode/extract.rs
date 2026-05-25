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
                        Some(format!("{}", tdbe::Price::new(p.value, p.r#type).to_f64()))
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
                        Some(tdbe::Price::new(p.value, p.r#type))
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
}
