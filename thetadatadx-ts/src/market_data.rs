//! The market-data endpoint methods of `MarketDataView` and
//! `MarketDataClient`, with their options objects and `<Tick>WithColumns`
//! results, expanded from the core endpoint table.
//!
//! `#[napi]` reads argument and field types as written, without expanding a
//! macro inside them, and renders a doc line only when it is one string
//! literal. So every type is resolved to plain tokens before an item is
//! built, and `__paste!` joins the pieces of a doc line into one literal. Per
//! endpoint, in table order, each class gets the method, then its `Stream`
//! companion when the endpoint is chunked, then its `WithColumns` variant
//! when it returns ticks, then its `ToPath` method when it is a flat file:
//! the member order of the published declarations.

use std::ops::ControlFlow;
use std::time::Duration;

use super::*;

/// Normalizes a required argument in place before the request is built.
macro_rules! ts_req {
    (Symbols $a:ident) => {
        let $a = normalize_symbols($a);
    };
    (Date $a:ident) => {
        let $a = normalize_date($a);
    };
    (Expiration $a:ident) => {
        let $a = normalize_date($a);
    };
    (Time $a:ident) => {
        let $a = normalize_time($a);
    };
    ($t:ident $a:ident) => {};
}

/// Reads an optional argument from the options object. An integer arrives
/// as a JavaScript number and is refused, under its camelCase key, unless it
/// is a non-negative `i32`.
macro_rules! ts_opt {
    (Int $a:ident $o:ident) => {
        thetadatadx::__paste!(validate_optional_nonneg_i32(stringify!([<$a:lower_camel>]), $o.$a)?)
    };
    (Date $a:ident $o:ident) => {
        normalize_optional_date($o.$a)
    };
    (Time $a:ident $o:ident) => {
        normalize_optional_time($o.$a)
    };
    ($t:ident $a:ident $o:ident) => {
        $o.$a
    };
}

/// The builder setter's argument for an optional value.
macro_rules! ts_set {
    (Int $v:ident) => {
        $v
    };
    (Float $v:ident) => {
        $v
    };
    (Bool $v:ident) => {
        $v
    };
    ($t:ident $v:ident) => {
        $v.as_str()
    };
}

/// One endpoint in one context: `options` emits its options object, a class
/// name its methods on that class.
macro_rules! ts_endpoint {
    // The names. The method keeps the acronyms the declarations publish
    // (`stockHistoryEOD`).
    ({ $ctx:ident } $attrs:tt $shape:ident $name:ident $ret:ident $($rest:tt)*) => {
        thetadatadx::__paste! {
            ts_endpoint! { @types $ctx $attrs $shape $name $ret
                [<$name:lower_camel:replace("Eod", "EOD"):replace("Ohlc", "OHLC")>]
                [<$name:camel Options>]
                [<$ret:lower_camel>]
                $($rest)* }
        }
    };

    // The method documentation, then each argument's binding type, before
    // any napi item sees them.
    (@types $ctx:ident $attrs:tt $shape:ident $name:ident $ret:ident $js:ident $opts:ident $tjs:ident
        [$($doc:tt)*] params {
            all $all:tt req [$($req:tt)*] opt $opt:tt
            defaults [$({ $dp0:ident $dv0:tt $dq0:tt } $({ $dp:ident $dv:tt $dq:tt })*)?]
            dates $dates:tt
        }
    ) => {
        ts_endpoint! { @req [$($req)*] [] $opt { $ctx $attrs $shape $name $ret $js $opts $tjs [
            $(#[doc = $doc])*
            $(
                #[doc = ""]
                #[doc = "Defaults (upstream):"]
                #[doc = "- `" $dp0 "`: `" $dq0 $dv0 $dq0 "`"]
                $(#[doc = "- `" $dp "`: `" $dq $dv $dq "`"])*
            )?
        ] } }
    };
    (@req [{ $p:ident $a:ident req Symbols $v:tt $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @req [$($more)*] [$($out)* { $a Symbols [Either<String, Vec<String>>] }] $($k)* }
    };
    (@req [{ $p:ident $a:ident req $t:ident $v:tt $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @req [$($more)*] [$($out)* { $a $t [String] }] $($k)* }
    };
    (@req [] $req:tt [$({ $p:ident $a:ident opt $t:ident $v:tt $d:tt })*] $k:tt) => {
        ts_endpoint! { @opt [$({ $a $t $d })*] [] $req $k }
    };
    // An integer is read as a JavaScript number and validated, never
    // narrowed by a wrapping conversion.
    (@opt [{ $a:ident Int $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @opt [$($more)*] [$($out)* { $a Int $d [Option<f64>] }] $($k)* }
    };
    (@opt [{ $a:ident Float $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @opt [$($more)*] [$($out)* { $a Float $d [Option<f64>] }] $($k)* }
    };
    (@opt [{ $a:ident Bool $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @opt [$($more)*] [$($out)* { $a Bool $d [Option<bool>] }] $($k)* }
    };
    (@opt [{ $a:ident $t:ident $d:tt } $($more:tt)*] [$($out:tt)*] $($k:tt)*) => {
        ts_endpoint! { @opt [$($more)*] [$($out)* { $a $t $d [Option<String>] }] $($k)* }
    };
    (@opt [] $opt:tt $req:tt { $ctx:ident $attrs:tt $shape:ident $($k:tt)* }) => {
        ts_endpoint! { @emit $ctx $shape $attrs $req $opt $($k)* }
    };

    // The options object: the camelCase optional parameters and `timeoutMs`.
    (@emit options $shape:ident $attrs:tt $req:tt [$({ $a:ident $t:ident $d:tt [$($ty:tt)*] })*]
        $name:ident $ret:ident $js:ident $opts:ident $($k:tt)*
    ) => {
        thetadatadx::__paste! {
            #[doc = "Optional parameters for the `" $js "` method. Keys are"]
            #[doc = "the camelCase parameter names; absent keys behave exactly like an"]
            #[doc = "omitted parameter. `timeoutMs` bounds the whole call: on expiry the"]
            #[doc = "returned Promise rejects and the underlying request is cancelled."]
            #[napi(object)]
            #[derive(Default)]
            pub struct $opts {
                $(
                    #[doc = $d]
                    pub $a: $($ty)*,
                )*
                /// Per-call deadline as a non-negative whole number of milliseconds;
                /// on expiry the returned Promise rejects and the underlying request
                /// is cancelled. A non-finite, negative, or fractional value is
                /// rejected with `InvalidParameterError` rather than coerced.
                pub timeout_ms: Option<f64>,
            }
        }
    };

    // A list endpoint: the strings of its one column.
    (@emit $class:ident list $attrs:tt [$({ $a:ident $t:ident [$($ty:tt)*] })*]
        [$({ $oa:ident $ot:ident $od:tt $oty:tt })*]
        $name:ident $ret:ident $js:ident $opts:ident $tjs:ident [$($doc:tt)*]
    ) => {
        thetadatadx::__paste! {
            #[napi]
            impl $class {
                $($doc)*
                #[napi(js_name = $js "")]
                pub async fn $name(
                    &self,
                    $($a: $($ty)*,)*
                    options: Option<$opts>,
                ) -> napi::Result<Vec<String>> {
                    ts_endpoint!(@prelude self options timeout_ms client [$($a $t)*] [$($oa $ot)*]);
                    spawn_endpoint_task(async move {
                        ts_endpoint!(@request client $name [$($a $t)*] [$($oa $ot)*] timeout_ms).await
                    })
                    .await
                }
            }
        }
    };

    // A tick endpoint: the method, the chunk stream when the endpoint is
    // chunked, and the variant that also returns the columns the wire carried.
    (@emit $class:ident $shape:ident $attrs:tt [$({ $a:ident $t:ident [$($ty:tt)*] })*]
        [$({ $oa:ident $ot:ident $od:tt $oty:tt })*]
        $name:ident $item:ident $js:ident $opts:ident $tjs:ident [$($doc:tt)*]
    ) => {
        thetadatadx::__paste! {
            #[napi]
            impl $class {
                $($doc)*
                #[napi(js_name = $js "")]
                pub async fn $name(
                    &self,
                    $($a: $($ty)*,)*
                    options: Option<$opts>,
                ) -> napi::Result<Vec<$item>> {
                    let ticks = ts_endpoint!(@fetch self options $name [$($a $t)*] [$($oa $ot)*]);
                    Ok([<$item:snake s_to_class_vec>](&ticks))
                }
            }
        }
        ts_endpoint! { @stream $shape $attrs $class $name $item $js $opts [$({ $a $t [$($ty)*] })*] [$($oa $ot)*] }
        thetadatadx::__paste! {
            #[napi]
            impl $class {
                #[doc = "Run the `" $js "` query and return the rows together with the columns the response's wire carried, so a projected Arrow-IPC frame is drivable from a live call. Same parameters and result rows as the `" $js "` method; the returned object adds `presentColumns` (the schema columns the wire sent, in schema order), `symbol` (the root every row shares, set for option and index history and single-symbol snapshot responses), and `symbols` (the per-row roots when the rows span several, as in a multi-symbol snapshot or a flat file). Feed them to `" $tjs "ToArrowIpcProjected` for a terminal-exact columnar export that omits the columns the wire omitted and attributes each row to its symbol."]
                #[napi(js_name = $js "WithColumns")]
                pub async fn [<$name _with_columns>](
                    &self,
                    $($a: $($ty)*,)*
                    options: Option<$opts>,
                ) -> napi::Result<[<$item WithColumns>]> {
                    let ticks = ts_endpoint!(@fetch self options $name [$($a $t)*] [$($oa $ot)*]);
                    Ok([<$item WithColumns>] {
                        rows: [<$item:snake s_to_class_vec>](&ticks),
                        present_columns: ticks.columns().present_names().map(String::from).collect(),
                        symbol: ticks.columns().symbol().map(String::from),
                        symbols: ticks.columns().symbols().map(|s| s.iter().map(|v| v.to_string()).collect()),
                    })
                }
            }
        }
    };

    // A chunked endpoint's stream. One whose pulls fan out into bands says
    // that its chunks then interleave.
    (@stream buffered $($rest:tt)*) => {};
    (@stream chunked [bands] $($rest:tt)*) => {
        ts_endpoint! { @stream_method "Under `bulkFetch = \"auto\"` a large history pull may fan out across concurrent sub-requests: every chunk is still delivered exactly once, but chunks from different sub-requests interleave in arrival order rather than the single-stream order (set `bulkFetch = \"off\"` to restore it). " $($rest)* }
    };
    (@stream chunked [] $($rest:tt)*) => {
        ts_endpoint! { @stream_method "" $($rest)* }
    };
    (@stream_method $fan:tt $class:ident $name:ident $item:ident $js:ident $opts:ident
        [$({ $a:ident $t:ident [$($ty:tt)*] })*] [$($oa:ident $ot:ident)*]
    ) => {
        thetadatadx::__paste! {
            #[napi]
            impl $class {
                #[doc = "Stream `" $name "` rows into `callback` without materialising the full response in memory. `callback(rows: " $item "[], columns: ColumnPresence) => void` is invoked once per server chunk with the chunk's rows and the columns its wire carried: when the rows span several roots, `columns.symbols` holds the root of each row; when they share one, as in option and index history, `columns.symbol` holds it. A flat file's chunks come either way, so read whichever is set; a response that carries no root, such as stock history, sets neither. The chunk is freed before that stream's next chunk is fetched, so peak memory tracks a single chunk rather than the whole result. " $fan "This is the memory-bounded companion to the `" $js "` method; prefer it for multi-day or full-universe pulls. The returned Promise resolves when the stream drains and rejects (typed like the buffered method) on a wire or decode error. `options` carries the same optional builder parameters and `timeoutMs` as the buffered method; the `callback` is the trailing argument."]
                #[napi(js_name = $js "Stream")]
                pub async fn [<$name _stream>](
                    &self,
                    $($a: $($ty)*,)*
                    options: Option<$opts>,
                    #[napi(ts_arg_type = "(rows: Array<" $item ">, columns: ColumnPresence) => void")] callback: ChunkCallback<$item>,
                ) -> napi::Result<()> {
                    ts_endpoint!(@prelude self options timeout_ms client [$($a $t)*] [$($oa $ot)*]);
                    let callback = Arc::new(callback);
                    let callback_error = Arc::new(Mutex::new(None::<napi::Error>));
                    spawn_napi_task(async move {
                        let request = ts_endpoint!(@request client $name [$($a $t)*] [$($oa $ot)*] timeout_ms);
                        let callback_error_for_stream = Arc::clone(&callback_error);
                        // A throw from the callback ends the stream and
                        // rejects the Promise with that error.
                        let stream_result = request
                            .stream_ticks_async(move |chunk| {
                                let callback = Arc::clone(&callback);
                                let callback_error = Arc::clone(&callback_error_for_stream);
                                let rows = [<$item:snake s_to_class_vec>](chunk.as_slice());
                                let columns = ColumnPresence::from(chunk.columns());
                                async move {
                                    match callback.call_async_catch((rows, columns).into()).await {
                                        Ok(_) => ControlFlow::Continue(()),
                                        Err(err) => {
                                            *callback_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(err);
                                            ControlFlow::Break(())
                                        }
                                    }
                                }
                            })
                            .await;
                        if let Some(err) = callback_error.lock().unwrap_or_else(|e| e.into_inner()).take() {
                            Err(err)
                        } else {
                            stream_result.map_err(to_napi_err)
                        }
                    })
                    .await
                }
            }
        }
    };

    // The setup every method shares: the options object, the deadline, the
    // client handle, then each argument normalized or validated in parameter
    // order. The bindings it makes are named by the caller.
    (@prelude $self:ident $options:ident $timeout:ident $client:ident [$($a:ident $t:ident)*] [$($oa:ident $ot:ident)*]) => {
        let $options = $options.unwrap_or_default();
        let $timeout = match $options.timeout_ms {
            Some(ms) => Some(validate_timeout_ms(ms)?),
            None => None,
        };
        let $client = $self.client_handle()?;
        $(ts_req!($t $a);)*
        $(let $oa = ts_opt!($ot $oa $options);)*
    };
    // The decoded ticks of one buffered call.
    (@fetch $self:ident $options:ident $name:ident $req:tt $opt:tt) => {{
        ts_endpoint!(@prelude $self $options timeout_ms client $req $opt);
        spawn_endpoint_task(async move { ts_endpoint!(@request client $name $req $opt timeout_ms).await }).await?
    }};
    // The builder with every option set and the deadline applied.
    (@request $client:ident $name:ident [$($a:ident $t:ident)*] [$($oa:ident $ot:ident)*] $timeout:ident) => {{
        $(ts_endpoint!(@refs $t $a);)*
        let mut request = $client.market_data().$name($(&$a),*);
        $(
            if let Some(value) = $oa {
                request = request.$oa(ts_set!($ot value));
            }
        )*
        if let Some(ms) = $timeout {
            request = request.with_deadline(Duration::from_millis(ms));
        }
        request
    }};
    (@refs Symbols $a:ident) => {
        let $a: Vec<&str> = $a.iter().map(|s| s.as_str()).collect();
    };
    (@refs $t:ident $a:ident) => {};
}

/// A flat file's `<method>ToPath` method on a class, which writes the file
/// to disk.
macro_rules! ts_to_path {
    (options $sub:ident $name:ident) => {};
    ($class:ident flat_file $name:ident) => {
        thetadatadx::__paste! {
            ts_to_path! { @emit $class $name [<$name:lower_camel:replace("Eod", "EOD"):replace("Ohlc", "OHLC")>] }
        }
    };
    ($class:ident $sub:ident $name:ident) => {};
    (@emit $class:ident $name:ident $js:ident) => {
        thetadatadx::__paste! {
            #[napi]
            impl $class {
                #[doc = "Write the `" $js "` flat file for `date` to `path` as the terminal writes it, and resolve to the path written. `format` is `\"csv\"` (the default), `\"json\"`, `\"json_legacy\"`, `\"ndjson\"` (or `\"jsonl\"`) or `\"html\"`. The file streams to disk chunk by chunk and takes its name only once complete; a path without an extension gets the format's."]
                #[napi(js_name = $js "ToPath")]
                pub async fn [<$name _to_path>](
                    &self,
                    date: String,
                    path: String,
                    format: Option<String>,
                ) -> napi::Result<String> {
                    let format = format
                        .as_deref()
                        .map_or(Ok(thetadatadx::flatfiles::FlatFileFormat::Csv), str::parse)
                        .map_err(invalid_parameter_err)?;
                    let client = self.client_handle()?;
                    ts_req!(Date date);
                    let written = spawn_endpoint_task(async move {
                        client.market_data().[<$name _to_path>](&date, &path, format).await
                    })
                    .await?;
                    Ok(written.to_string_lossy().into_owned())
                }
            }
        }
    };
}

/// Expands every endpoint of the table in one context.
macro_rules! ts_market_data {
    ($ctx:ident $( $(#[$attr:ident])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident,
        rest: $rest:tt,
        returns: $ret:ident $(($col:tt))?,
        params: [$($key:tt)*],
        doc: [$($doc:tt),* $(,)?],
    } )*) => {
        $(
            thetadatadx::__resolve! { [$($key)*] => ts_endpoint { { $ctx } [$($attr)*] $shape $name $ret [$($doc)*] } }
            ts_to_path! { $ctx $sub $name }
        )*
    };
}

/// The result of every `<method>WithColumns` variant, one per tick type.
macro_rules! ts_with_columns {
    (@struct $tick:ident $name:ident $js:ident) => {
        thetadatadx::__paste! {
            #[doc = "A decoded history result paired with the columns the response's wire"]
            #[doc = "actually carried. Returned by the `<method>WithColumns` variant so a"]
            #[doc = "live caller can serialize a projected Arrow-IPC frame (only the wire's"]
            #[doc = "columns, matching the terminal's columnar output) without hand-supplying"]
            #[doc = "the header list: pass `presentColumns` and `symbol` to `" $js "ToArrowIpcProjected`."]
            #[napi(object)]
            pub struct $name {
                /// The decoded rows, identical to the `Array<Tick>` the buffered
                /// method returns.
                pub rows: Vec<$tick>,
                /// The schema-column names the response's wire carried, in schema
                /// order; the projection set for a terminal-exact Arrow frame.
                pub present_columns: Vec<String>,
                /// The response's constant `symbol` (root) when the wire carried one
                /// constant across every row (option, index, and single-symbol snapshot
                /// responses), else `undefined`.
                pub symbol: Option<String>,
                /// The response's per-row `symbol` values when the wire carried a
                /// `symbol` column that varies across rows (a multi-symbol snapshot or a
                /// flat file), one
                /// value per row so each row is attributable to its underlying, else
                /// `undefined`. Pass straight to `<tick>ToArrowIpcProjected`'s `symbols`.
                pub symbols: Option<Vec<String>>,
            }
        }
    };
    ($($(#[$attr:ident])* $tick:ident $body:tt)*) => {
        $( thetadatadx::__paste! { ts_with_columns! { @struct $tick [<$tick WithColumns>] [<$tick:lower_camel>] } } )*
    };
}

/// The columns a streamed chunk's wire carried and the root each row belongs
/// to: the second argument of every `<method>Stream` callback. Pass
/// `presentColumns`, `symbol` and `symbols` to `<tick>ToArrowIpcProjected` for
/// a terminal-exact columnar export of the chunk.
#[napi(object)]
pub struct ColumnPresence {
    /// The schema-column names the chunk's wire carried, in schema order.
    pub present_columns: Vec<String>,
    /// The root every row of the chunk shares, as in option and index
    /// history or a flat-file chunk inside one root, else `undefined`.
    pub symbol: Option<String>,
    /// One root per row when the chunk's rows span several, else
    /// `undefined`.
    pub symbols: Option<Vec<String>>,
}

impl From<&thetadatadx::columns::ColumnPresence> for ColumnPresence {
    fn from(columns: &thetadatadx::columns::ColumnPresence) -> Self {
        Self {
            present_columns: columns.present_names().map(String::from).collect(),
            symbol: columns.symbol().map(String::from),
            symbols: columns
                .symbols()
                .map(|symbols| symbols.iter().map(|symbol| symbol.to_string()).collect()),
        }
    }
}

/// The `callback` every `<method>Stream` method calls once per chunk, with the
/// chunk's rows and its `ColumnPresence`. `CalleeHandled = false` with
/// `call_async_catch` returns a throw from the callback as `Err` instead of
/// routing it to `uncaughtException`.
type ChunkCallback<Tick> = napi::threadsafe_function::ThreadsafeFunction<
    napi::bindgen_prelude::FnArgs<(Vec<Tick>, ColumnPresence)>,
    (),
    napi::bindgen_prelude::FnArgs<(Vec<Tick>, ColumnPresence)>,
    napi::Status,
    false,
>;

thetadatadx::__ticks! { ts_with_columns {} }
thetadatadx::__endpoints! { ts_market_data { options } }
thetadatadx::__endpoints! { ts_market_data { MarketDataView } }
thetadatadx::__endpoints! { ts_market_data { MarketDataClient } }
