//! DataFrame extension traits — `.to_polars()` / `.to_arrow()` on
//! `&[Tick]`.
//!
//! Feature-gated: `polars`, `arrow`, or `frames` (both). The per-type
//! impls expand here from the tick table (`crate::__ticks!`), and the
//! Python binding builds its Arrow tables through the same impls, so
//! Python and Rust sides return columns in the same order.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "polars")]
//! # fn doc() {
//! use thetadatadx::frames::TicksPolarsExt;
//! use thetadatadx::EodTick;
//! let ticks: Vec<EodTick> = Vec::new();
//! let _df = ticks.as_slice().to_polars().expect("empty frame is valid");
//! # }
//! ```

/// Convert a slice of tick rows into a [`polars::prelude::DataFrame`].
///
/// Feature-gated on the `polars` Cargo feature. Implemented for every
/// tick type the crate exports (`thetadatadx::TradeTick`, `thetadatadx::QuoteTick`, ...).
#[cfg(feature = "polars")]
#[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
pub trait TicksPolarsExt {
    /// Materialise `self` as a polars `DataFrame`. Column order and
    /// names match the Arrow schema produced by `TicksArrowExt::to_arrow`
    /// (the `arrow`-gated sibling) and the Python `.to_polars()` terminal on
    /// the matching `<TickName>List` wrapper.
    ///
    /// Emits the full schema — every column the tick type defines. This is
    /// the hand-built-slice path: a slice a caller assembled itself never
    /// touched the wire, so every column is "present". For a slice decoded
    /// from a gRPC response, project to the wire's exact columns with
    /// [`Self::to_polars_projected`].
    ///
    /// # Errors
    ///
    /// Returns a [`polars::prelude::PolarsError`] when polars rejects the
    /// assembled columns (e.g. a length mismatch while building the frame).
    fn to_polars(&self) -> polars::prelude::PolarsResult<polars::prelude::DataFrame>;

    /// Materialise `self` as a polars `DataFrame` carrying only the columns
    /// present on the decoded response's wire (`present`), in schema order.
    ///
    /// Terminal-exact: a `stock_history_trade` response omits the
    /// contract-identity trio and the four trade-flag columns, so the frame
    /// omits them too — no always-null / always-zero superset columns.
    /// `present` comes from
    /// [`crate::columns::WireColumns::present_columns`] at the decode seam.
    ///
    /// # Errors
    ///
    /// Returns a [`polars::prelude::PolarsError`] when polars rejects the
    /// assembled columns.
    fn to_polars_projected(
        &self,
        present: &crate::columns::ColumnPresence,
    ) -> polars::prelude::PolarsResult<polars::prelude::DataFrame>;
}

/// Convert a slice of tick rows into an [`arrow_array::RecordBatch`].
///
/// Feature-gated on the `arrow` Cargo feature. Implemented for every
/// tick type the crate exports (`thetadatadx::TradeTick`, `thetadatadx::QuoteTick`, ...).
#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub trait TicksArrowExt {
    /// Materialise `self` as an Arrow `RecordBatch`. The Python,
    /// TypeScript and C++ Arrow outputs are built through this trait, so
    /// their column order, names, and dtypes match it.
    ///
    /// Emits the full schema — every column the tick type defines. This is
    /// the hand-built-slice path: a slice a caller assembled itself never
    /// touched the wire, so every column is "present". For a slice decoded
    /// from a gRPC response, project to the wire's exact columns with
    /// [`Self::to_arrow_projected`].
    ///
    /// # Errors
    ///
    /// Returns an [`arrow_schema::ArrowError`] when the column arrays
    /// cannot be assembled into a `RecordBatch` against the schema.
    fn to_arrow(
        &self,
    ) -> ::core::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    /// Materialise `self` as an Arrow `RecordBatch` carrying only the
    /// columns present on the decoded response's wire (`present`), in
    /// schema order.
    ///
    /// Terminal-exact: an equity/index response omits the contract-identity
    /// trio, and no gRPC trade response carries the four flag columns, so
    /// the batch omits them rather than emitting always-null / always-zero
    /// superset columns. `present` comes from
    /// [`crate::columns::WireColumns::present_columns`] at the decode seam.
    ///
    /// # Errors
    ///
    /// Returns an [`arrow_schema::ArrowError`] when the column arrays
    /// cannot be assembled into a `RecordBatch` against the schema.
    fn to_arrow_projected(
        &self,
        present: &crate::columns::ColumnPresence,
    ) -> ::core::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;
}

// Per-tick-type impls, expanded from the tick table. Each impl block carries
// its own feature gate, so enabling only one of `polars` / `arrow` compiles
// the matching impls without pulling the other dependency.

#[cfg(feature = "arrow")]
use arrow_array::{ArrayRef, Float64Array, Int32Array, Int64Array, StringArray};
#[cfg(feature = "arrow")]
use arrow_array::{RecordBatch, RecordBatchOptions};
#[cfg(feature = "arrow")]
use arrow_schema::{DataType, Field, Schema as ArrowSchema};
#[cfg(feature = "arrow")]
use std::sync::Arc;

#[cfg(feature = "polars")]
use polars::prelude::{DataFrame, NamedFrom, PlSmallStr, PolarsResult, Series};

macro_rules! tick_frames {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        $(crate::__tick! { [$(#[$attr])* $name $body] => tick_frame {} })*
    };
}

/// The Arrow and Polars builders of one tick type: one column per public
/// field in table order, a nullable column and the contract triple as
/// nulls where the row carries no value.
macro_rules! tick_frame {
    ($name:ident $kind:ident contract [$($exp:ident $strike:ident $right:ident)?] doc $doc:tt
        summary $summary:tt more $more:tt required $required:tt
        columns [$({ $f:ident $w:ident $ty:ident [$($has:ident)?] $d:tt })*]
        timestamps $timestamps:tt flags $flags:tt) => {
        crate::__paste! {
            #[cfg(feature = "arrow")]
            #[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
            impl crate::frames::TicksArrowExt for [crate::tdbe::types::tick::$name] {
                #[doc = "Builds an Arrow `RecordBatch` from a slice of `" $name "`, one column per public field."]
                fn to_arrow(&self) -> ::core::result::Result<RecordBatch, arrow_schema::ArrowError> {
                    let n = self.len();
                    $(let mut [<col_ $f>]: Vec<tick_frame!(@elem $ty [$($has)?])> = Vec::with_capacity(n);)*
                    $(
                        let mut [<col_ $exp>]: Vec<Option<i32>> = Vec::with_capacity(n);
                        let mut [<col_ $strike>]: Vec<Option<f64>> = Vec::with_capacity(n);
                        let mut [<col_ $right>]: Vec<Option<String>> = Vec::with_capacity(n);
                    )?
                    for t in self {
                        $([<col_ $f>].push(tick_frame!(@value t $f $ty [$($has)?]));)*
                        $(
                            [<col_ $exp>].push(t.has_contract_id().then_some(t.$exp));
                            [<col_ $strike>].push(t.has_contract_id().then_some(t.$strike));
                            [<col_ $right>].push(if t.$right == '\0' { None } else { Some(t.$right.to_string()) });
                        )?
                    }
                    let schema = Arc::new(ArrowSchema::new(vec![
                        $(Field::new(stringify!($f), tick_frame!(@datatype $ty), tick_frame!(@nullable [$($has)?])),)*
                        $(
                            Field::new(stringify!($exp), DataType::Int32, true),
                            Field::new(stringify!($strike), DataType::Float64, true),
                            Field::new(stringify!($right), DataType::Utf8, true),
                        )?
                    ]));
                    let columns: Vec<ArrayRef> = vec![
                        $(tick_frame!(@array $ty [<col_ $f>]),)*
                        $(
                            Arc::new(Int32Array::from([<col_ $exp>])) as ArrayRef,
                            Arc::new(Float64Array::from([<col_ $strike>])) as ArrayRef,
                            Arc::new(StringArray::from([<col_ $right>])) as ArrayRef,
                        )?
                    ];
                    RecordBatch::try_new(schema, columns)
                }

                #[doc = "Builds an Arrow `RecordBatch` from a slice of `" $name "`, one column per public field present on the wire."]
                fn to_arrow_projected(&self, present: &crate::columns::ColumnPresence) -> ::core::result::Result<RecordBatch, arrow_schema::ArrowError> {
                    let n = self.len();
                    $(let [<keep_ $f>] = present.contains(stringify!($f));)*
                    $(
                        let [<keep_ $exp>] = present.contains(stringify!($exp));
                        let [<keep_ $strike>] = present.contains(stringify!($strike));
                        let [<keep_ $right>] = present.contains(stringify!($right));
                    )?
                    $(let mut [<col_ $f>]: Vec<tick_frame!(@elem $ty [$($has)?])> = Vec::with_capacity(if [<keep_ $f>] { n } else { 0 });)*
                    $(
                        let mut [<col_ $exp>]: Vec<Option<i32>> = Vec::with_capacity(if [<keep_ $exp>] { n } else { 0 });
                        let mut [<col_ $strike>]: Vec<Option<f64>> = Vec::with_capacity(if [<keep_ $strike>] { n } else { 0 });
                        let mut [<col_ $right>]: Vec<Option<String>> = Vec::with_capacity(if [<keep_ $right>] { n } else { 0 });
                    )?
                    for t in self {
                        $(if [<keep_ $f>] { [<col_ $f>].push(tick_frame!(@value t $f $ty [$($has)?])); })*
                        $(
                            if [<keep_ $exp>] { [<col_ $exp>].push(t.has_contract_id().then_some(t.$exp)); }
                            if [<keep_ $strike>] { [<col_ $strike>].push(t.has_contract_id().then_some(t.$strike)); }
                            if [<keep_ $right>] { [<col_ $right>].push(if t.$right == '\0' { None } else { Some(t.$right.to_string()) }); }
                        )?
                    }
                    let mut fields: Vec<Field> = Vec::new();
                    let mut columns: Vec<ArrayRef> = Vec::new();
                    // The response's root leads, unless the row carries its own.
                    if ![$(stringify!($f)),*].contains(&"symbol") {
                        if let Some(syms) = present.symbols() {
                            fields.push(Field::new("symbol", DataType::Utf8, false));
                            columns.push(Arc::new(StringArray::from(syms.iter().map(Box::as_ref).collect::<Vec<&str>>())) as ArrayRef);
                        } else if let Some(sym) = present.symbol() {
                            fields.push(Field::new("symbol", DataType::Utf8, false));
                            columns.push(Arc::new(StringArray::from(vec![sym; n])) as ArrayRef);
                        }
                    }
                    $(if [<keep_ $f>] {
                        fields.push(Field::new(stringify!($f), tick_frame!(@datatype $ty), tick_frame!(@nullable [$($has)?])));
                        columns.push(tick_frame!(@array $ty [<col_ $f>]));
                    })*
                    $(
                        if [<keep_ $exp>] {
                            fields.push(Field::new(stringify!($exp), DataType::Int32, true));
                            columns.push(Arc::new(Int32Array::from([<col_ $exp>])) as ArrayRef);
                        }
                        if [<keep_ $strike>] {
                            fields.push(Field::new(stringify!($strike), DataType::Float64, true));
                            columns.push(Arc::new(Float64Array::from([<col_ $strike>])) as ArrayRef);
                        }
                        if [<keep_ $right>] {
                            fields.push(Field::new(stringify!($right), DataType::Utf8, true));
                            columns.push(Arc::new(StringArray::from([<col_ $right>])) as ArrayRef);
                        }
                    )?
                    RecordBatch::try_new_with_options(Arc::new(ArrowSchema::new(fields)), columns, &RecordBatchOptions::new().with_row_count(Some(n)))
                }
            }

            #[cfg(feature = "polars")]
            #[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
            impl crate::frames::TicksPolarsExt for [crate::tdbe::types::tick::$name] {
                #[doc = "Builds a Polars `DataFrame` from a slice of `" $name "`, one column per public field."]
                fn to_polars(&self) -> PolarsResult<DataFrame> {
                    let n = self.len();
                    $(let mut [<col_ $f>]: Vec<tick_frame!(@elem $ty [$($has)?])> = Vec::with_capacity(n);)*
                    $(
                        let mut [<col_ $exp>]: Vec<Option<i32>> = Vec::with_capacity(n);
                        let mut [<col_ $strike>]: Vec<Option<f64>> = Vec::with_capacity(n);
                        let mut [<col_ $right>]: Vec<Option<String>> = Vec::with_capacity(n);
                    )?
                    for t in self {
                        $([<col_ $f>].push(tick_frame!(@value t $f $ty [$($has)?]));)*
                        $(
                            [<col_ $exp>].push(t.has_contract_id().then_some(t.$exp));
                            [<col_ $strike>].push(t.has_contract_id().then_some(t.$strike));
                            [<col_ $right>].push(if t.$right == '\0' { None } else { Some(t.$right.to_string()) });
                        )?
                    }
                    DataFrame::new(n, vec![
                        $(Series::new(PlSmallStr::from_static(stringify!($f)), [<col_ $f>]).into(),)*
                        $(
                            Series::new(PlSmallStr::from_static(stringify!($exp)), [<col_ $exp>]).into(),
                            Series::new(PlSmallStr::from_static(stringify!($strike)), [<col_ $strike>]).into(),
                            Series::new(PlSmallStr::from_static(stringify!($right)), [<col_ $right>]).into(),
                        )?
                    ])
                }

                #[doc = "Builds a Polars `DataFrame` from a slice of `" $name "`, one column per public field present on the wire."]
                fn to_polars_projected(&self, present: &crate::columns::ColumnPresence) -> PolarsResult<DataFrame> {
                    let n = self.len();
                    $(let [<keep_ $f>] = present.contains(stringify!($f));)*
                    $(
                        let [<keep_ $exp>] = present.contains(stringify!($exp));
                        let [<keep_ $strike>] = present.contains(stringify!($strike));
                        let [<keep_ $right>] = present.contains(stringify!($right));
                    )?
                    $(let mut [<col_ $f>]: Vec<tick_frame!(@elem $ty [$($has)?])> = Vec::with_capacity(if [<keep_ $f>] { n } else { 0 });)*
                    $(
                        let mut [<col_ $exp>]: Vec<Option<i32>> = Vec::with_capacity(if [<keep_ $exp>] { n } else { 0 });
                        let mut [<col_ $strike>]: Vec<Option<f64>> = Vec::with_capacity(if [<keep_ $strike>] { n } else { 0 });
                        let mut [<col_ $right>]: Vec<Option<String>> = Vec::with_capacity(if [<keep_ $right>] { n } else { 0 });
                    )?
                    for t in self {
                        $(if [<keep_ $f>] { [<col_ $f>].push(tick_frame!(@value t $f $ty [$($has)?])); })*
                        $(
                            if [<keep_ $exp>] { [<col_ $exp>].push(t.has_contract_id().then_some(t.$exp)); }
                            if [<keep_ $strike>] { [<col_ $strike>].push(t.has_contract_id().then_some(t.$strike)); }
                            if [<keep_ $right>] { [<col_ $right>].push(if t.$right == '\0' { None } else { Some(t.$right.to_string()) }); }
                        )?
                    }
                    let mut series: Vec<polars::prelude::Column> = Vec::new();
                    // The response's root leads, unless the row carries its own.
                    if ![$(stringify!($f)),*].contains(&"symbol") {
                        if let Some(syms) = present.symbols() {
                            series.push(Series::new(PlSmallStr::from_static("symbol"), syms.iter().map(|s| s.to_string()).collect::<Vec<String>>()).into());
                        } else if let Some(sym) = present.symbol() {
                            series.push(Series::new(PlSmallStr::from_static("symbol"), vec![sym.to_string(); n]).into());
                        }
                    }
                    $(if [<keep_ $f>] {
                        series.push(Series::new(PlSmallStr::from_static(stringify!($f)), [<col_ $f>]).into());
                    })*
                    $(
                        if [<keep_ $exp>] {
                            series.push(Series::new(PlSmallStr::from_static(stringify!($exp)), [<col_ $exp>]).into());
                        }
                        if [<keep_ $strike>] {
                            series.push(Series::new(PlSmallStr::from_static(stringify!($strike)), [<col_ $strike>]).into());
                        }
                        if [<keep_ $right>] {
                            series.push(Series::new(PlSmallStr::from_static(stringify!($right)), [<col_ $right>]).into());
                        }
                    )?
                    DataFrame::new(n, series)
                }
            }
        }
    };
    // A nullable column holds `None` where the response carried no value.
    (@elem $ty:ident [$has:ident]) => { Option<tick_frame!(@elem $ty [])> };
    (@elem i64 []) => { i64 };
    (@elem eod_num64 []) => { i64 };
    (@elem f64 []) => { f64 };
    (@elem price []) => { f64 };
    (@elem eod_price []) => { f64 };
    (@elem String []) => { String };
    (@elem right []) => { String };
    (@elem calendar_status []) => { String };
    (@elem $ty:ident []) => { i32 };
    (@value $t:ident $f:ident $ty:ident [$has:ident]) => { $t.$has.then_some($t.$f) };
    (@value $t:ident $f:ident String []) => { $t.$f.clone() };
    (@value $t:ident $f:ident right []) => { if $t.$f == '\0' { String::new() } else { $t.$f.to_string() } };
    (@value $t:ident $f:ident calendar_status []) => { $t.$f.as_str().to_string() };
    (@value $t:ident $f:ident $ty:ident []) => { $t.$f };
    (@datatype i64) => { DataType::Int64 };
    (@datatype eod_num64) => { DataType::Int64 };
    (@datatype f64) => { DataType::Float64 };
    (@datatype price) => { DataType::Float64 };
    (@datatype eod_price) => { DataType::Float64 };
    (@datatype String) => { DataType::Utf8 };
    (@datatype right) => { DataType::Utf8 };
    (@datatype calendar_status) => { DataType::Utf8 };
    (@datatype $ty:ident) => { DataType::Int32 };
    (@nullable []) => { false };
    (@nullable [$has:ident]) => { true };
    (@array i64 $col:ident) => { Arc::new(Int64Array::from($col)) as ArrayRef };
    (@array eod_num64 $col:ident) => { Arc::new(Int64Array::from($col)) as ArrayRef };
    (@array f64 $col:ident) => { Arc::new(Float64Array::from($col)) as ArrayRef };
    (@array price $col:ident) => { Arc::new(Float64Array::from($col)) as ArrayRef };
    (@array eod_price $col:ident) => { Arc::new(Float64Array::from($col)) as ArrayRef };
    (@array String $col:ident) => { Arc::new(StringArray::from($col)) as ArrayRef };
    (@array right $col:ident) => { Arc::new(StringArray::from($col)) as ArrayRef };
    (@array calendar_status $col:ident) => { Arc::new(StringArray::from($col)) as ArrayRef };
    (@array $ty:ident $col:ident) => { Arc::new(Int32Array::from($col)) as ArrayRef };
}

crate::__ticks! { tick_frames {} }
