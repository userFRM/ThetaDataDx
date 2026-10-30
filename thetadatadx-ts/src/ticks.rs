//! The tick row objects, their factories and their Arrow IPC functions,
//! expanded from the core tick table (`thetadatadx::__ticks!`).
//!
//! `#[napi]` reads field and argument types as written, without expanding a
//! macro inside them, and renders a doc line only when it is one string
//! literal. So every column type is resolved to plain tokens before the row
//! object is built, and `__paste!` joins the pieces of a doc line into one
//! literal.

use super::*;

/// Expands every table entry.
macro_rules! ts_ticks {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        $(thetadatadx::__tick! { [$(#[$attr])* $name $body] => ts_tick {} })*
    };
}

/// One tick type: the row object, its factory, the reverse conversion and
/// the Arrow IPC functions.
macro_rules! ts_tick {
    ($name:ident $kind:ident contract $trio:tt doc $doc:tt summary $summary:tt more $more:tt
        required $required:tt columns [$($col:tt)*] timestamps $timestamps:tt flags $flags:tt) => {
        ts_tick! { @cols [$name $trio $summary $timestamps $flags] [] [$($col)*] }
    };

    // Resolve each column to its JavaScript-facing type and conversion. A
    // nullable column becomes an optional number in place of the core's value
    // and presence flag. A 32-bit integer crosses as a `number`, not a napi
    // `i32`, so the reverse conversion can reject what `ToInt32` would wrap.
    (@cols $st:tt $done:tt [{ $f:ident $w:ident $ty:ident $has:tt $d:tt } $($rest:tt)*]) => {
        ts_tick! { @col $st $done [$($rest)*] $f $ty $has }
    };
    (@cols $st:tt $done:tt []) => {
        ts_tick! { @emit $st $done }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $ty:ident [$has:ident]) => {
        ts_tick! { @cols $st [$($done)* { $f [Option<f64>] optional [$has] }] $rest }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident eod_num []) => {
        ts_tick! { @col $st $done $rest $f i32 [] }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident eod_date []) => {
        ts_tick! { @col $st $done $rest $f i32 [] }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident eod_num64 []) => {
        ts_tick! { @col $st $done $rest $f i64 [] }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident price []) => {
        ts_tick! { @col $st $done $rest $f f64 [] }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident eod_price []) => {
        ts_tick! { @col $st $done $rest $f f64 [] }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident i32 []) => {
        ts_tick! { @cols $st [$($done)* { $f [f64] i32 [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident i64 []) => {
        ts_tick! { @cols $st [$($done)* { $f [BigInt] i64 [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident f64 []) => {
        ts_tick! { @cols $st [$($done)* { $f [f64] copy [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident String []) => {
        ts_tick! { @cols $st [$($done)* { $f [String] string [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident right []) => {
        ts_tick! { @cols $st [$($done)* { $f [String] right [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident calendar_status []) => {
        ts_tick! { @cols $st [$($done)* { $f [String] status [] }] $rest }
    };

    (@emit [$name:ident [$($exp:ident $strike:ident $right:ident)?] [$($summary:tt)*]
        [$({ $acc:ident $date:ident $ms:ident })*] [$({ $flag:ident $field:ident $lo:tt $hi:tt $fdoc:tt })*]]
        [$({ $f:ident [$($tsty:tt)*] $conv:ident [$($has:ident)?] })*]) => {
        thetadatadx::__paste! {
            #[doc = $($summary)*]
            #[must_use]
            #[napi(object)]
            #[derive(Clone)]
            pub struct $name {
                $(pub $f: $($tsty)*,)*
                // Contract identity on a wildcard response; `undefined` when
                // the response names a single contract.
                $(
                    pub $exp: Option<f64>,
                    pub $strike: Option<f64>,
                    pub $right: Option<String>,
                )?
                $(
                    #[doc = $fdoc]
                    pub $flag: bool,
                )*
                $(
                    #[doc = "Unix epoch milliseconds (UTC, DST-aware) combining `" $date "` with"]
                    #[doc = "`" $ms "` (Eastern-Time milliseconds-of-day). `undefined` when"]
                    #[doc = "`" $date "` is absent (`0`)."]
                    pub $acc: Option<BigInt>,
                )*
            }

            pub(crate) fn [<$name:snake s_to_class_vec>](ticks: &[thetadatadx::$name]) -> Vec<$name> {
                ticks
                    .iter()
                    .map(|t| $name {
                        $($f: ts_tick!(@to_js $conv t $f [$($has)?]),)*
                        $(
                            $exp: t.has_contract_id().then_some(f64::from(t.$exp)),
                            $strike: t.has_contract_id().then_some(t.$strike),
                            $right: if t.$right == '\0' { None } else { Some(t.$right.to_string()) },
                        )?
                        $($flag: ($lo..=$hi).contains(&t.$field),)*
                        $($acc: thetadatadx::time::date_ms_to_epoch_ms(t.$date, t.$ms).map(BigInt::from),)*
                    })
                    .collect()
            }

            /// The core rows the JavaScript objects describe, shared by the
            /// full and projected Arrow IPC functions.
            fn [<$name:snake _reconstruct_rows>](rows: Vec<$name>) -> napi::Result<Vec<thetadatadx::$name>> {
                rows.into_iter()
                    .map(|r| -> napi::Result<thetadatadx::$name> {
                        Ok(thetadatadx::$name {
                            $(
                                $f: ts_tick!(@from_js $conv r $f),
                                $($has: r.$f.is_some(),)?
                            )*
                            $(
                                $exp: match r.$exp { Some(v) => number_to_i32(stringify!($exp), v)?, None => 0 },
                                $strike: r.$strike.unwrap_or(0.0),
                                $right: match r.$right.as_deref() {
                                    Some("C") => 'C',
                                    Some("P") => 'P',
                                    None | Some("") => '\0',
                                    Some(other) => return Err(napi::Error::from_reason(format!(
                                        "[InvalidParameterError] right must be \"C\" or \"P\", got {other:?}"
                                    ))),
                                },
                            )?
                        })
                    })
                    .collect()
            }

            #[doc = "Serialise a hand-built `" $name "` row vector to a full-schema Arrow"]
            #[doc = "IPC stream (the `apache-arrow` wire form) carrying every column the"]
            #[doc = "tick type defines. For rows decoded from a history response, use"]
            #[doc = "`" [<$name:lower_camel>] "PresentColumns` + `" [<$name:lower_camel>] "ToArrowIpcProjected` for a terminal-exact"]
            #[doc = "frame carrying only the wire's columns, mirroring Python's"]
            #[doc = "projected `<TickName>List.to_arrow()`."]
            #[napi(js_name = [<$name:lower_camel>] "ToArrowIpc")]
            pub fn [<$name:snake _to_arrow_ipc>](rows: Vec<$name>) -> napi::Result<napi::bindgen_prelude::Buffer> {
                let owned = [<$name:snake _reconstruct_rows>](rows)?;
                let batch = thetadatadx::frames::TicksArrowExt::to_arrow(owned.as_slice())
                    .map_err(|e| napi::Error::from_reason(format!("arrow conversion failed: {e}")))?;
                arrow_batch_to_ipc_buffer(&batch)
            }

            #[doc = "Resolve a `" $name "` history response's wire header names to the schema"]
            #[doc = "columns it carried, in schema order. Feed the result to"]
            #[doc = "`" [<$name:lower_camel>] "ToArrowIpcProjected` for a terminal-exact columnar export. Mirrors the"]
            #[doc = "C ABI `thetadatadx_<tick>_present_columns` producer and Python's"]
            #[doc = "`<TickName>List.columns`."]
            #[napi(js_name = [<$name:lower_camel>] "PresentColumns")]
            pub fn [<$name:snake _present_columns>](headers: Vec<String>) -> Vec<String> {
                let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
                <thetadatadx::$name as thetadatadx::columns::WireColumns>::present_columns(&refs)
                    .present_names()
                    .map(String::from)
                    .collect()
            }

            #[doc = "Serialise a `" $name "` history result to a projected Arrow IPC stream"]
            #[doc = "carrying ONLY the columns named in `presentColumns` (build it with"]
            #[doc = "`" [<$name:lower_camel>] "PresentColumns` from the response headers), optionally broadcasting"]
            #[doc = "`symbol` as the leading column, or emitting a per-row `symbols` column"]
            #[doc = "for a multi-symbol snapshot or a flat file (one value per row; takes precedence over"]
            #[doc = "`symbol`). The decode-fed sibling of"]
            #[doc = "`" [<$name:lower_camel>] "ToArrowIpc`: same wire format, projected to the wire's exact column"]
            #[doc = "set, matching Python's projected `<TickName>List.to_arrow()` and the C"]
            #[doc = "ABI `thetadatadx_<tick>_to_arrow_ipc_projected`."]
            #[napi(js_name = [<$name:lower_camel>] "ToArrowIpcProjected")]
            pub fn [<$name:snake _to_arrow_ipc_projected>](
                rows: Vec<$name>,
                present_columns: Vec<String>,
                symbol: Option<String>,
                symbols: Option<Vec<String>>,
            ) -> napi::Result<napi::bindgen_prelude::Buffer> {
                let owned = [<$name:snake _reconstruct_rows>](rows)?;
                let mut columns = thetadatadx::columns::ColumnPresence::from_names(present_columns);
                // A tick that owns a per-row `symbol` column ignores the
                // broadcast, so the schema never carries it twice; per-row
                // `symbols` take precedence over the one `symbol`.
                if !columns.contains("symbol") {
                    if let Some(syms) = symbols {
                        columns = columns.with_symbols(syms);
                    } else if let Some(sym) = symbol {
                        columns = columns.with_symbol(sym);
                    }
                }
                let batch = thetadatadx::frames::TicksArrowExt::to_arrow_projected(owned.as_slice(), &columns)
                    .map_err(|e| napi::Error::from_reason(format!("arrow conversion failed: {e}")))?;
                arrow_batch_to_ipc_buffer(&batch)
            }
        }
    };

    // Core row to JavaScript object.
    (@to_js i32 $t:tt $f:ident []) => { f64::from($t.$f) };
    (@to_js i64 $t:tt $f:ident []) => { BigInt::from($t.$f) };
    (@to_js copy $t:tt $f:ident []) => { $t.$f };
    (@to_js string $t:tt $f:ident []) => { $t.$f.clone() };
    (@to_js right $t:tt $f:ident []) => { if $t.$f == '\0' { String::new() } else { $t.$f.to_string() } };
    (@to_js status $t:tt $f:ident []) => { $t.$f.as_str().to_string() };
    (@to_js optional $t:tt $f:ident [$has:ident]) => { if $t.$has { Some(f64::from($t.$f)) } else { None } };

    // JavaScript object to core row, rejecting a value the core type cannot
    // hold. An absent nullable value returns as the column's zero beside a
    // cleared presence flag.
    (@from_js i32 $r:tt $f:ident) => { number_to_i32(stringify!($f), $r.$f)? };
    (@from_js i64 $r:tt $f:ident) => { bigint_to_i64(stringify!($f), &$r.$f)? };
    (@from_js copy $r:tt $f:ident) => { $r.$f };
    (@from_js string $r:tt $f:ident) => { $r.$f };
    (@from_js right $r:tt $f:ident) => {
        match $r.$f.as_str() {
            "C" => 'C',
            "P" => 'P',
            "" => '\0',
            other => return Err(napi::Error::from_reason(format!(
                "[InvalidParameterError] right must be \"C\" or \"P\", got {other:?}"
            ))),
        }
    };
    (@from_js status $r:tt $f:ident) => {
        match thetadatadx::CalendarStatus::from_wire_text(&$r.$f) {
            Some(status) => status,
            None => return Err(napi::Error::from_reason(format!(
                "[InvalidParameterError] status must be one of open, early_close, full_close, weekend; got {:?}",
                $r.$f
            ))),
        }
    };
    (@from_js optional $r:tt $f:ident) => {
        match $r.$f { Some(v) => number_to_i32(stringify!($f), v)?, None => 0 }
    };
}

thetadatadx::__ticks! { ts_ticks {} }

/// Decode an `i64` from a JS `bigint` Arrow column, rejecting a magnitude that
/// does not fit `i64` rather than truncating its wrapped low bits.
fn bigint_to_i64(name: &str, v: &BigInt) -> napi::Result<i64> {
    let (value, lossless) = v.get_i64();
    if lossless {
        Ok(value)
    } else {
        Err(crate::invalid_parameter_err(format!(
            "{name}: BigInt magnitude must fit in i64"
        )))
    }
}

/// Decode an `i32` from a JS `number` Arrow column, rejecting a value that is
/// not a whole number within `i32` rather than wrapping or truncating it.
fn number_to_i32(name: &str, v: f64) -> napi::Result<i32> {
    if v.fract() == 0.0 && v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX) {
        Ok(v as i32)
    } else {
        Err(crate::invalid_parameter_err(format!(
            "{name}: must be a whole number within i32, got {v}"
        )))
    }
}

/// Serialise an Arrow `RecordBatch` to an Arrow IPC stream buffer (the
/// `apache-arrow` wire form) through a `StreamWriter`. Shared by the full and
/// projected `<tick>ToArrowIpc` terminals.
fn arrow_batch_to_ipc_buffer(
    batch: &arrow_array::RecordBatch,
) -> napi::Result<napi::bindgen_prelude::Buffer> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = arrow_ipc::writer::StreamWriter::try_new(
            std::io::Cursor::new(&mut buf),
            &batch.schema(),
        )
        .map_err(|e| napi::Error::from_reason(format!("arrow ipc writer init failed: {e}")))?;
        writer
            .write(batch)
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc write failed: {e}")))?;
        writer
            .finish()
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc finish failed: {e}")))?;
    }
    Ok(napi::bindgen_prelude::Buffer::from(buf))
}
