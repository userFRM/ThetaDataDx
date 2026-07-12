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
///
/// Only compiled under `__internal` — called by workspace bindings only.
#[cfg(feature = "__internal")]
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
/// Unlike the strict single-cell `row_*` decoders, this extractor
/// stringifies `Number` and `Price` cells rather than rejecting them.
/// That tolerance is load-bearing for the served `list_*` endpoints:
/// `list_dates` / `list_expirations` carry `YYYYMMDD` values as
/// `Number` cells and `list_strikes` carries the strike as a `Number`
/// or `Price` cell, yet every binding presents these lists as
/// `Vec<String>`. Rejecting numeric cells here would break those endpoints on every
/// call, so the coercion is intentional rather than a swallowed drift.
///
/// To keep an unexpected numeric cell observable (a text column that
/// the server starts publishing as numeric is still worth knowing about
/// without failing the request), each coercion emits a `tracing::trace!`
/// naming the requested header and the observed wire variant. `trace`
/// rather than `warn` because numeric cells are the *expected* shape on
/// the numeric list endpoints, so a higher level would be pure noise.
///
/// `Price` cells with `price_type` outside
/// `0..=crate::tdbe::types::price::MAX_PRICE_TYPE` yield `None` for that row
/// and emit a rate-unlimited `tracing::warn!`.
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
                    proto::data_value::DataType::Number(n) => {
                        tracing::trace!(
                            target: "thetadatadx::mdds::decode::extract",
                            requested = header,
                            column_kind = "Text",
                            observed = "Number",
                            "coercing Number cell to string in text column",
                        );
                        Some(n.to_string())
                    }
                    proto::data_value::DataType::Price(p) => {
                        match crate::tdbe::types::price::Price::with_value_and_type(p.value, p.r#type) {
                            Ok(price) => {
                                tracing::trace!(
                                    target: "thetadatadx::mdds::decode::extract",
                                    requested = header,
                                    column_kind = "Text",
                                    observed = "Price",
                                    "coercing Price cell to string in text column",
                                );
                                Some(format!("{}", price.to_f64()))
                            }
                            Err(err) => {
                                tracing::warn!(
                                    target: "thetadatadx::mdds::decode::extract",
                                    requested = header,
                                    column_kind = "Text",
                                    price_value = p.value,
                                    price_type = p.r#type,
                                    error = %err,
                                    "dropping Price cell with out-of-range price_type from text column",
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
/// `Price` cells with `price_type` outside
/// `0..=crate::tdbe::types::price::MAX_PRICE_TYPE` yield `None` for that row
/// and emit a `tracing::warn!`.
///
/// Only compiled under `__internal` — called by workspace bindings only.
#[cfg(feature = "__internal")]
#[must_use]
pub fn extract_price_column(
    table: &proto::DataTable,
    header: &str,
) -> Vec<Option<crate::tdbe::types::price::Price>> {
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
                        match crate::tdbe::types::price::Price::with_value_and_type(
                            p.value, p.r#type,
                        ) {
                            Ok(price) => Some(price),
                            Err(err) => {
                                tracing::warn!(
                                    target: "thetadatadx::mdds::decode::extract",
                                    requested = header,
                                    column_kind = "Price",
                                    price_value = p.value,
                                    price_type = p.r#type,
                                    error = %err,
                                    "dropping Price cell with out-of-range price_type",
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

/// The response's `symbol` (root) shape, read once from the wire's
/// `symbol`/`root` column and carried onto the projected frame's leading
/// `symbol` column. The flat POD ticks hold no per-row `String`, so the
/// symbol rides on the response's [`crate::columns::ColumnPresence`] instead
/// of a tick field.
#[derive(Debug, PartialEq, Eq)]
pub enum ResponseSymbol {
    /// The wire carried no `symbol`/`root` header — stock history responses,
    /// which gain no `symbol` column.
    Absent,
    /// The wire's `symbol` column is constant across every row (broadcast as
    /// one value): the option + index single-underlying endpoints, and a
    /// single-symbol stock snapshot. A header-present-but-rowless response is
    /// `Constant("")` so the column set stays keyed on the header.
    Constant(Box<str>),
    /// The wire's `symbol` column varies row-to-row — a multi-symbol snapshot
    /// (`stock_snapshot_quote(["AAPL","MSFT"])`). One value per row, so each
    /// row is attributable to its underlying rather than mislabelled with row
    /// 0's value. A non-`Text` symbol cell yields the empty string for that
    /// row (matching the per-column projection's null handling).
    PerRow(Vec<Box<str>>),
}

/// Classify the response's `symbol` (root) column: absent, constant across
/// all rows (broadcast), or per-row-varying (a multi-symbol snapshot). The
/// decode seam calls this once per response and attaches the result to the
/// [`crate::columns::ColumnPresence`] so the projected builders emit the
/// leading `symbol` column terminal-exact — one value broadcast, one value
/// per row, or none.
pub fn response_symbol(table: &proto::DataTable) -> ResponseSymbol {
    let header_refs: Vec<&str> = table.headers.iter().map(String::as_str).collect();
    let Some(col_idx) = find_header(&header_refs, "root") else {
        return ResponseSymbol::Absent;
    };
    if table.data_table.is_empty() {
        return ResponseSymbol::Constant("".into());
    }
    // Borrow the cell text; a local fn (not a closure) so the returned `&str`
    // borrows from the `row` argument.
    fn cell(row: &proto::DataValueList, col_idx: usize) -> Option<&str> {
        match row.values.get(col_idx).and_then(|dv| dv.data_type.as_ref()) {
            Some(proto::data_value::DataType::Text(s)) => Some(s.as_str()),
            _ => None,
        }
    }
    let first = cell(&table.data_table[0], col_idx);
    if table
        .data_table
        .iter()
        .all(|row| cell(row, col_idx) == first)
    {
        return match first {
            Some(first) => {
                // Constant: box the single value once, not per row — a ~1M-row
                // response would otherwise pay ~1M heap allocations.
                ResponseSymbol::Constant(first.into())
            }
            None => ResponseSymbol::Absent,
        };
    }
    // Varying: one value per row so each row is attributable. A non-`Text`
    // cell yields `""` for that row (the per-column projection nulls it too).
    ResponseSymbol::PerRow(
        table
            .data_table
            .iter()
            .map(|row| cell(row, col_idx).unwrap_or("").into())
            .collect(),
    )
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

    /// A `Number` cell in a text-requested column is coerced to its
    /// decimal string, not dropped to `None`. This tolerance is
    /// load-bearing for the numeric list endpoints (`list_dates`,
    /// `list_expirations`, `list_strikes`) which publish their values
    /// as `Number` cells yet return `Vec<String>`. The coercion path is
    /// observable via `tracing::trace!`, so the drift is logged rather
    /// than silently swallowed.
    #[test]
    fn extract_text_column_coerces_number_cell_to_string() {
        let table = proto::DataTable {
            headers: vec!["date".to_string()],
            data_table: vec![
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(20260413)),
                    }],
                },
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(20260414)),
                    }],
                },
            ],
        };
        let column = extract_text_column(&table, "date");
        assert_eq!(
            column,
            vec![Some("20260413".to_string()), Some("20260414".to_string())],
            "numeric list-endpoint cells must coerce to their decimal \
             string, not drop to None"
        );
    }

    /// `response_symbol` reads the response's constant root from the first
    /// row, resolving the schema-side `root` against the wire's `symbol`
    /// header via the shared alias table.
    #[test]
    fn response_symbol_reads_root_via_symbol_alias() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string(), "price".to_string()],
            data_table: vec![
                proto::DataValueList {
                    values: vec![
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Text("SPY".into())),
                        },
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Number(1)),
                        },
                    ],
                },
                proto::DataValueList {
                    values: vec![
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Text("SPY".into())),
                        },
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Number(2)),
                        },
                    ],
                },
            ],
        };
        assert_eq!(
            response_symbol(&table),
            ResponseSymbol::Constant("SPY".into())
        );
    }

    /// A stock history response carries no `symbol`/`root` header, so
    /// `response_symbol` is `Absent` — the projected frame gains no `symbol`
    /// column.
    #[test]
    fn response_symbol_absent_header_is_none() {
        let table = proto::DataTable {
            headers: vec!["ms_of_day".to_string(), "price".to_string()],
            data_table: vec![proto::DataValueList {
                values: vec![
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(1)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(2)),
                    },
                ],
            }],
        };
        assert_eq!(response_symbol(&table), ResponseSymbol::Absent);
    }

    /// A multi-symbol snapshot response carries a per-row-varying `symbol`, so
    /// `response_symbol` returns `PerRow` with one value per row — each row is
    /// attributable to its underlying rather than mislabelled with row 0's
    /// symbol.
    #[test]
    fn response_symbol_varying_column_is_per_row() {
        let row = |sym: &str| proto::DataValueList {
            values: vec![proto::DataValue {
                data_type: Some(proto::data_value::DataType::Text(sym.into())),
            }],
        };
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: vec![row("AAPL"), row("MSFT"), row("SPY")],
        };
        assert_eq!(
            response_symbol(&table),
            ResponseSymbol::PerRow(vec!["AAPL".into(), "MSFT".into(), "SPY".into()]),
            "a per-row-varying symbol must carry one value per row",
        );
    }

    #[test]
    fn response_symbol_null_first_cell_keeps_later_per_row_values() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: vec![
                proto::DataValueList {
                    values: vec![proto::DataValue { data_type: None }],
                },
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Text("MSFT".into())),
                    }],
                },
                proto::DataValueList {
                    values: vec![proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Text("SPY".into())),
                    }],
                },
            ],
        };
        assert_eq!(
            response_symbol(&table),
            ResponseSymbol::PerRow(vec!["".into(), "MSFT".into(), "SPY".into()])
        );
    }

    /// A non-`Text` cell in a single-row symbol column yields `Absent` (the
    /// lone value is unreadable, so there is nothing to broadcast) rather than
    /// broadcasting an empty string.
    #[test]
    fn response_symbol_non_text_cell_is_none() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: vec![proto::DataValueList {
                values: vec![proto::DataValue {
                    data_type: Some(proto::data_value::DataType::Number(0)),
                }],
            }],
        };
        assert_eq!(response_symbol(&table), ResponseSymbol::Absent);
    }

    /// Header present but no data rows -> `Constant("")`: the column set stays
    /// keyed on the header (matching per-column projection), broadcast over
    /// zero rows.
    #[test]
    fn response_symbol_header_present_no_rows_is_empty() {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string()],
            data_table: Vec::new(),
        };
        assert_eq!(response_symbol(&table), ResponseSymbol::Constant("".into()));
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
}

#[cfg(test)]
#[cfg(feature = "__internal")]
mod internal_tests {
    use super::*;

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

    #[test]
    fn extract_price_column_drops_price_type_20() {
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
        let table = price_table("price", &[(100, 19)]);
        let col = extract_price_column(&table, "price");
        assert_eq!(col.len(), 1);
        let p = col[0].expect("price_type=19 must round-trip");
        assert_eq!(p.value(), 100);
        assert_eq!(p.price_type(), 19);
    }

    #[test]
    fn extract_price_column_keeps_typical_price_type_10() {
        let table = price_table("price", &[(12345, 10)]);
        let col = extract_price_column(&table, "price");
        let p = col[0].expect("in-range price must round-trip");
        assert_eq!(p.value(), 12345);
        assert_eq!(p.price_type(), 10);
    }

    #[test]
    fn extract_text_column_drops_price_with_out_of_range_price_type() {
        let table = price_table("price", &[(100, 20)]);
        let col = extract_text_column(&table, "price");
        assert_eq!(col, vec![None]);
    }

    #[test]
    fn extract_text_column_keeps_price_at_boundary() {
        let table = price_table("price", &[(100, 19)]);
        let col = extract_text_column(&table, "price");
        assert_eq!(col.len(), 1);
        assert!(col[0].is_some());
    }
}
