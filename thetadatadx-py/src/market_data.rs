//! The `MarketDataView` endpoint methods, their builder classes and the
//! offline decode hook, expanded from the core endpoint table.

// A `#[pymethods]` signature is the Python call signature: every keyword
// argument is its own parameter, so an endpoint method takes one per query
// parameter.
#![expect(clippy::too_many_arguments)]

use super::*;

/// Python argument type of a parameter.
macro_rules! py_arg {
    (Symbols) => {
        PySymbols
    };
    (Date) => {
        PyDateArg
    };
    (Expiration) => {
        PyDateArg
    };
    (Time) => {
        PyTimeArg
    };
    (Int) => {
        i32
    };
    (Float) => {
        f64
    };
    (Bool) => {
        bool
    };
    ($t:ident) => {
        PyStringArg
    };
}

/// Stored builder type of an optional parameter.
macro_rules! py_stored {
    (Int) => { Option<i32> };
    (Float) => { Option<f64> };
    (Bool) => { Option<bool> };
    ($t:ident) => { Option<String> };
}

/// Owned value a builder stores for an extracted argument.
macro_rules! py_owned {
    (Symbols $v:ident) => {
        $v.into_vec()
    };
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
        $v.into_string()
    };
}

/// Engine setter argument from an extracted value.
macro_rules! py_set {
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

/// Engine setter argument from a reference to a stored value.
macro_rules! py_set_ref {
    (Int $v:ident) => {
        *$v
    };
    (Float $v:ident) => {
        *$v
    };
    (Bool $v:ident) => {
        *$v
    };
    ($t:ident $v:ident) => {
        $v.as_str()
    };
}

/// Copy of a stored optional field.
macro_rules! py_field {
    (Int $f:expr) => {
        $f
    };
    (Float $f:expr) => {
        $f
    };
    (Bool $f:expr) => {
        $f
    };
    ($t:ident $f:expr) => {
        $f.clone()
    };
}

/// Borrowed engine arguments for a multi-symbol argument; nothing otherwise.
macro_rules! py_refs {
    (Symbols $a:ident $refs:ident) => {
        let $refs: Vec<&str> = $a.iter().map(|s| s.as_str()).collect();
    };
    ($t:ident $a:ident $refs:ident) => {};
}

/// Engine call argument from an extracted argument.
macro_rules! py_call_arg {
    (Symbols $a:ident $refs:ident) => {
        &$refs
    };
    ($t:ident $a:ident $refs:ident) => {
        $a.as_str()
    };
}

/// Engine call argument from a stored builder field.
macro_rules! py_call_field {
    (Symbols $a:ident $refs:ident) => {
        &$refs
    };
    ($t:ident $a:ident $refs:ident) => {
        &$a
    };
}

/// The sentence a `#[bands]` endpoint's stream docs add on fan-out.
macro_rules! py_fan_out {
    (bands) => {
        "Under `bulk_fetch = \"auto\"` a large history pull may fan out across concurrent sub-requests: every chunk is still delivered exactly once, but chunks from different sub-requests interleave in arrival order rather than the single-stream order (`bulk_fetch = \"off\"` restores it). "
    };
    () => { "" };
}

/// One endpoint: its `MarketDataView` methods and its builder class.
macro_rules! py_endpoint {
    // Keyword defaults: the sync method shows booleans and numbers.
    (@sig [{ $p:ident $a:ident opt Bool [$v:tt] $d:tt } $($more:tt)*] [$($s:tt)*] $k:tt) => {
        py_endpoint! { @sig [$($more)*] [$($s)* $a = $v,] $k }
    };
    (@sig [{ $p:ident $a:ident opt Int [$v:tt] $d:tt } $($more:tt)*] [$($s:tt)*] $k:tt) => {
        py_endpoint! { @sig [$($more)*] [$($s)* $a = $v,] $k }
    };
    (@sig [{ $p:ident $a:ident opt Float [$v:tt] $d:tt } $($more:tt)*] [$($s:tt)*] $k:tt) => {
        py_endpoint! { @sig [$($more)*] [$($s)* $a = $v,] $k }
    };
    (@sig [{ $p:ident $a:ident opt $t:ident $v:tt $d:tt } $($more:tt)*] [$($s:tt)*] $k:tt) => {
        py_endpoint! { @sig [$($more)*] [$($s)* $a = None,] $k }
    };
    (@sig [] $s:tt { $($k:tt)* }) => {
        py_endpoint! { @emit $s $($k)* }
    };

    // The async docstring keeps one more blank line before its suffix when
    // the composed doc ends in a newline: a vendor docstring or defaults.
    ([$($attr:ident)*] $shape:ident $name:ident $ret:ident $(($col:tt))? [$desc:tt] params {
        all $all:tt req $req:tt opt [$($opt:tt)*] defaults [] dates $dates:tt
    }) => {
        py_endpoint! { @sig [$($opt)*] [] { [] [$($attr)*] $shape $name $ret $(($col))? [$desc] params {
            all $all req $req opt [$($opt)*] defaults [] dates $dates
        } } }
    };
    ([$($attr:ident)*] $shape:ident $name:ident $ret:ident $(($col:tt))? [$($doc:tt)*] params {
        all $all:tt req $req:tt opt [$($opt:tt)*] $($rest:tt)*
    }) => {
        py_endpoint! { @sig [$($opt)*] [] { [""] [$($attr)*] $shape $name $ret $(($col))? [$($doc)*] params {
            all $all req $req opt [$($opt)*] $($rest)*
        } } }
    };

    // Buffered endpoints without a multi-symbol argument return a plain list.
    (@emit $s:tt $tail:tt $attrs:tt buffered $name:ident $item:ident $docs:tt params {
        all $all:tt req [{ $sp:ident $sa:ident req Symbols $($sx:tt)* } $($req:tt)*] $($rest:tt)*
    }) => {
        py_endpoint! { @view [to_pyclass_list] $s $tail $attrs buffered $name $item $docs params {
            all $all req [{ $sp $sa req Symbols $($sx)* } $($req)*] $($rest)*
        } }
    };
    (@emit $s:tt $tail:tt $attrs:tt buffered $($rest:tt)*) => {
        py_endpoint! { @view [vec_to_pylist] $s $tail $attrs buffered $($rest)* }
    };
    (@emit $s:tt $tail:tt $attrs:tt $($rest:tt)*) => {
        py_endpoint! { @view [to_pyclass_list] $s $tail $attrs $($rest)* }
    };

    // List endpoints.
    (@view $conv:tt [$($sig:tt)*] [$($tail:tt)?] [$($attr:ident)*] list $name:ident StringList($col:tt) [$($doc:tt)*] params {
        all $all:tt
        req [$({ $p:ident $a:ident req $t:ident [] $pd:tt })*]
        opt [$({ $op:ident $oa:ident opt $ot:ident [$($ov:tt)?] $od:tt })*]
        defaults [$({ $dp0:ident $dv0:tt $dq0:tt } $({ $dp:ident $dv:tt $dq:tt })*)?]
        dates $dates:tt
    }) => {
        thetadatadx::__paste! {
            #[pymethods]
            impl MarketDataView {
                $(#[doc = concat!($doc)])*
                $(
                    #[doc = ""]
                    #[doc = "Defaults (upstream):"]
                    #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                    $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
                )?
                #[pyo3(signature = ($($a,)* *, $($sig)* timeout_ms=None))]
                fn $name(
                    &self,
                    py: Python<'_>,
                    $($a: py_arg!($t),)*
                    $($oa: Option<py_arg!($ot)>,)*
                    timeout_ms: Option<u64>,
                ) -> PyResult<Py<StringList>> {
                    let values: Vec<String> = run_blocking(py, async move {
                        py_endpoint!(@list_call self.client, $name, [$($a)*], [$($op $oa $ot)*], timeout_ms)
                    })?;
                    strings_to_string_list(py, values, $col)
                }

                $(#[doc = concat!($doc)])*
                $(
                    #[doc = ""]
                    #[doc = "Defaults (upstream):"]
                    #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                    $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
                )?
                #[doc = ""]
                $(#[doc = $tail])?
                #[doc = "Async companion: returns an awaitable (`asyncio.Future`)."]
                #[doc = "Shares the same shared tokio runtime as the sync variant; no"]
                #[doc = "second runtime is created per call."]
                #[pyo3(signature = ($($a,)* *, $($oa=None,)* timeout_ms=None))]
                fn [<$name _async>]<'py>(
                    &self,
                    py: Python<'py>,
                    $($a: py_arg!($t),)*
                    $($oa: Option<py_arg!($ot)>,)*
                    timeout_ms: Option<u64>,
                ) -> PyResult<Bound<'py, PyAny>> {
                    let client = self.client.clone();
                    spawn_awaitable(py, async move {
                        py_endpoint!(@list_call client, $name, [$($a)*], [$($op $oa $ot)*], timeout_ms)
                    }, |py, values| strings_to_string_list(py, values, $col).map(|p| p.into_any()))
                }

                #[doc = concat!("Create a fluent-builder for `", stringify!($name), "`. Chain setters then call `.list()`")]
                #[doc = "(or `.list_async()`) to execute the request; the returned typed list"]
                #[doc = "wrapper exposes the chainable terminals `.to_list()` / `.to_arrow()`"]
                #[doc = "/ `.to_pandas()` / `.to_polars()`."]
                #[doc = ""]
                #[doc = concat!("See `", stringify!($name), "()` for the sync signature; `", stringify!($name), "_async()` for the awaitable companion.")]
                #[pyo3(signature = ($($a),*))]
                fn [<$name _builder>](&self, $($a: py_arg!($t)),*) -> [<$name:camel Builder>] {
                    [<$name:camel Builder>] {
                        client: self.client.clone(),
                        $($a: py_owned!($t $a),)*
                        timeout_ms: None,
                    }
                }
            }

            $(#[doc = concat!($doc)])*
            $(
                #[doc = ""]
                #[doc = "Defaults (upstream):"]
                #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
            )?
            #[pyclass(module = "thetadatadx")]
            pub struct [<$name:camel Builder>] {
                client: std::sync::Arc<thetadatadx::Client>,
                $($a: String,)*
                timeout_ms: Option<u64>,
            }

            #[pymethods]
            impl [<$name:camel Builder>] {
                $(
                    #[doc = concat!("Replace the required `", stringify!($p), "` on the pending request.")]
                    fn $a<'py>(mut slf: PyRefMut<'py, Self>, value: py_arg!($t)) -> PyRefMut<'py, Self> {
                        slf.$a = value.into_string();
                        slf
                    }
                )*

                /// Apply a per-request deadline in milliseconds.
                fn timeout_ms<'py>(mut slf: PyRefMut<'py, Self>, value: u64) -> PyRefMut<'py, Self> {
                    slf.timeout_ms = Some(value);
                    slf
                }

                /// Execute the request and return a typed `StringList` wrapper.
                ///
                /// Chain `.to_polars()` / `.to_pandas()` / `.to_arrow()` / `.to_list()`
                /// on the result to convert to the downstream representation.
                fn list(&self, py: Python<'_>) -> PyResult<Py<StringList>> {
                    let client = self.client.clone();
                    let timeout_ms = self.timeout_ms;
                    $(let $a = self.$a.clone();)*
                    let values: Vec<String> = run_blocking(py, async move {
                        py_endpoint!(@list_field_call client, $name, [$($a)*], [$($oa)*], timeout_ms)
                    })?;
                    strings_to_string_list(py, values, $col)
                }

                /// Async companion to `list()`; awaitable yields the `StringList` wrapper.
                fn list_async<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                    let client = self.client.clone();
                    $(let $a = self.$a.clone();)*
                    let timeout_ms = self.timeout_ms;
                    spawn_awaitable(py, async move {
                        py_endpoint!(@list_field_call client, $name, [$($a)*], [$($oa)*], timeout_ms)
                    }, |py, values| strings_to_string_list(py, values, $col).map(|p| p.into_any()))
                }
            }
        }
    };

    // A list call without optionals has a `_with_deadline` twin; with
    // optionals it is a builder.
    (@list_call $client:expr, $name:ident, [$($a:ident)*], [], $timeout:ident) => {
        thetadatadx::__paste! {
            match $timeout {
                None => $client.market_data().$name($($a.as_str()),*).await,
                Some(ms) => $client.market_data().[<$name _with_deadline>]($($a.as_str(),)* std::time::Duration::from_millis(ms)).await,
            }
        }
    };
    (@list_call $client:expr, $name:ident, [$($a:ident)*], [$($op:ident $oa:ident $ot:ident)+], $timeout:ident) => {{
        let mut request = $client.market_data().$name($($a.as_str()),*);
        $(
            if let Some(value) = $oa {
                request = request.$op(py_set!($ot value));
            }
        )+
        if let Some(ms) = $timeout {
            request = request.with_deadline(std::time::Duration::from_millis(ms));
        }
        request.await
    }};
    (@list_field_call $client:ident, $name:ident, [$($a:ident)*], [], $timeout:ident) => {
        thetadatadx::__paste! {
            match $timeout {
                None => $client.market_data().$name($(&$a),*).await,
                Some(ms) => $client.market_data().[<$name _with_deadline>]($(&$a,)* std::time::Duration::from_millis(ms)).await,
            }
        }
    };
    (@list_field_call $client:ident, $name:ident, [$($a:ident)*], [$($oa:ident)+], $timeout:ident) => {
        match $timeout {
            None => $client.market_data().$name($(&$a),*).await,
            Some(ms) => $client.market_data().$name($(&$a),*).with_deadline(std::time::Duration::from_millis(ms)).await,
        }
    };

    // Tick endpoints.
    (@view [$conv:ident] [$($sig:tt)*] [$($tail:tt)?] [$($attr:ident)*] $shape:ident $name:ident $item:ident [$($doc:tt)*] params {
        all $all:tt
        req [$({ $p:ident $a:ident req $t:ident [] $pd:tt })*]
        opt [$({ $op:ident $oa:ident opt $ot:ident [$($ov:tt)?] $od:tt })*]
        defaults [$({ $dp0:ident $dv0:tt $dq0:tt } $({ $dp:ident $dv:tt $dq:tt })*)?]
        dates $dates:tt
    }) => {
        thetadatadx::__paste! {
            #[pymethods]
            impl MarketDataView {
                $(#[doc = concat!($doc)])*
                $(
                    #[doc = ""]
                    #[doc = "Defaults (upstream):"]
                    #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                    $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
                )?
                #[pyo3(signature = ($($a,)* *, $($sig)* timeout_ms=None))]
                fn $name(
                    &self,
                    py: Python<'_>,
                    $($a: py_arg!($t),)*
                    $($oa: Option<py_arg!($ot)>,)*
                    timeout_ms: Option<u64>,
                ) -> PyResult<Py<py_endpoint!(@ret $conv $item)>> {
                    $(py_refs!($t $a refs);)*
                    let mut request = self.client.market_data().$name($(py_call_arg!($t $a refs)),*);
                    $(
                        if let Some(value) = $oa {
                            request = request.$op(py_set!($ot value));
                        }
                    )*
                    if let Some(ms) = timeout_ms {
                        request = request.with_deadline(std::time::Duration::from_millis(ms));
                    }
                    let ticks = run_blocking(py, async move { request.await })?;
                    py_endpoint!(@convert $conv $item)(py, ticks)
                }

                $(#[doc = concat!($doc)])*
                $(
                    #[doc = ""]
                    #[doc = "Defaults (upstream):"]
                    #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                    $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
                )?
                #[doc = ""]
                $(#[doc = $tail])?
                #[doc = "Async companion: returns an awaitable (`asyncio.Future`)."]
                #[doc = "Shares the same shared tokio runtime as the sync variant; no"]
                #[doc = "second runtime is created per call."]
                #[pyo3(signature = ($($a,)* *, $($oa=None,)* timeout_ms=None))]
                fn [<$name _async>]<'py>(
                    &self,
                    py: Python<'py>,
                    $($a: py_arg!($t),)*
                    $($oa: Option<py_arg!($ot)>,)*
                    timeout_ms: Option<u64>,
                ) -> PyResult<Bound<'py, PyAny>> {
                    let client = self.client.clone();
                    spawn_awaitable(py, async move {
                        $(py_refs!($t $a refs);)*
                        let mut request = client.market_data().$name($(py_call_arg!($t $a refs)),*);
                        $(
                            if let Some(value) = $oa {
                                request = request.$op(py_set!($ot value));
                            }
                        )*
                        if let Some(ms) = timeout_ms {
                            request = request.with_deadline(std::time::Duration::from_millis(ms));
                        }
                        request.await
                    }, |py, ticks| py_endpoint!(@convert $conv $item)(py, ticks).map(|p| p.into_any()))
                }

                #[doc = concat!("Create a fluent-builder for `", stringify!($name), "`. Chain setters then call `.list()`")]
                #[doc = "(or `.list_async()`) to execute the request; the returned typed list"]
                #[doc = "wrapper exposes the chainable terminals `.to_list()` / `.to_arrow()`"]
                #[doc = "/ `.to_pandas()` / `.to_polars()`."]
                #[doc = ""]
                #[doc = concat!("See `", stringify!($name), "()` for the sync signature; `", stringify!($name), "_async()` for the awaitable companion.")]
                #[pyo3(signature = ($($a),*))]
                fn [<$name _builder>](&self, $($a: py_arg!($t)),*) -> [<$name:camel Builder>] {
                    [<$name:camel Builder>] {
                        client: self.client.clone(),
                        $($a: py_owned!($t $a),)*
                        $($oa: None,)*
                        timeout_ms: None,
                    }
                }
            }

            $(#[doc = concat!($doc)])*
            $(
                #[doc = ""]
                #[doc = "Defaults (upstream):"]
                #[doc = concat!("- `", stringify!($dp0), "`: `", $dq0, $dv0, $dq0, "`")]
                $(#[doc = concat!("- `", stringify!($dp), "`: `", $dq, $dv, $dq, "`")])*
            )?
            #[pyclass(module = "thetadatadx")]
            pub struct [<$name:camel Builder>] {
                client: std::sync::Arc<thetadatadx::Client>,
                $($a: py_endpoint!(@stored_req $t),)*
                $($oa: py_stored!($ot),)*
                timeout_ms: Option<u64>,
            }

            #[pymethods]
            impl [<$name:camel Builder>] {
                $(
                    #[doc = concat!("Set `", stringify!($op), "` on the pending request.")]
                    fn $oa<'py>(mut slf: PyRefMut<'py, Self>, value: py_arg!($ot)) -> PyRefMut<'py, Self> {
                        slf.$oa = Some(py_owned!($ot value));
                        slf
                    }
                )*

                $(
                    #[doc = concat!("Replace the required `", stringify!($p), "` on the pending request.")]
                    fn $a<'py>(mut slf: PyRefMut<'py, Self>, value: py_arg!($t)) -> PyRefMut<'py, Self> {
                        slf.$a = py_owned!($t value);
                        slf
                    }
                )*

                /// Apply a per-request deadline in milliseconds.
                fn timeout_ms<'py>(mut slf: PyRefMut<'py, Self>, value: u64) -> PyRefMut<'py, Self> {
                    slf.timeout_ms = Some(value);
                    slf
                }

                /// Execute the request and return a typed list wrapper.
                ///
                /// Chain `.to_polars()` / `.to_pandas()` / `.to_arrow()` / `.to_list()`
                /// on the result to convert to the downstream representation.
                fn list(&self, py: Python<'_>) -> PyResult<Py<[<$item List>]>> {
                    let client = self.client.clone();
                    $(let $a = self.$a.clone();)*
                    $(let $oa = py_field!($ot self.$oa);)*
                    let timeout_ms = self.timeout_ms;
                    let ticks = run_blocking(py, async move {
                        $(py_refs!($t $a refs);)*
                        let mut request = client.market_data().$name($(py_call_field!($t $a refs)),*);
                        $(
                            if let Some(value) = &$oa {
                                request = request.$op(py_set_ref!($ot value));
                            }
                        )*
                        if let Some(ms) = timeout_ms {
                            request = request.with_deadline(std::time::Duration::from_millis(ms));
                        }
                        request.await
                    })?;
                    [<$item:snake s_to_pyclass_list>](py, ticks)
                }

                /// Async companion to `list()`; awaitable yields the typed list wrapper.
                fn list_async<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                    let client = self.client.clone();
                    $(let $a = self.$a.clone();)*
                    $(let $oa = py_field!($ot self.$oa);)*
                    let timeout_ms = self.timeout_ms;
                    spawn_awaitable(py, async move {
                        $(py_refs!($t $a refs);)*
                        let mut request = client.market_data().$name($(py_call_field!($t $a refs)),*);
                        $(
                            if let Some(value) = &$oa {
                                request = request.$op(py_set_ref!($ot value));
                            }
                        )*
                        if let Some(ms) = timeout_ms {
                            request = request.with_deadline(std::time::Duration::from_millis(ms));
                        }
                        request.await
                    }, |py, ticks| [<$item:snake s_to_pyclass_list>](py, ticks).map(|p| p.into_any()))
                }

            }
        }
        py_endpoint!(@stream $shape [$($attr)*] $name $item [$($a $t)*] [$($op $oa $ot)*]);
    };

    // Chunk streaming on the builder of a chunked endpoint.
    (@stream buffered $($rest:tt)*) => {};
    (@stream chunked [$($attr:ident)*] $name:ident $item:ident [$($a:ident $t:ident)*] [$($op:ident $oa:ident $ot:ident)*]) => {
        thetadatadx::__paste! {
            #[pymethods]
            impl [<$name:camel Builder>] {
                #[doc = concat!("Stream chunks of `", stringify!($name), "` rows into `handler` without materialising the full response in memory. `handler(chunk: ", stringify!([<$item List>]), ") -> None` is called once per gRPC chunk with the chunk's rows in the typed list a whole result returns, which carries the root of each row: when the response carries roots, as a flat file or option history does, its `to_arrow()`, `to_pandas()` and `to_polars()` lead with a `symbol` column naming the root each row belongs to. The chunk is freed before the stream's next chunk is fetched. ", py_fan_out!($($attr)*), "An exception raised by `handler` stops the stream, so no further chunk is fetched, and is raised from this call.")]
                fn stream(&self, py: Python<'_>, handler: Py<PyAny>) -> PyResult<()> {
                    let client = self.client.clone();
                    $(let $a = self.$a.clone();)*
                    $(let $oa = py_field!($ot self.$oa);)*
                    let timeout_ms = self.timeout_ms;
                    let handler_arc = std::sync::Arc::new(handler);
                    let handler_for_closure = std::sync::Arc::clone(&handler_arc);
                    // The chunk closure is `FnMut`, so the callback's error is
                    // parked in a `Mutex` (which `run_blocking` also needs `Send`).
                    let callback_error: std::sync::Arc<std::sync::Mutex<Option<PyErr>>> =
                        std::sync::Arc::new(std::sync::Mutex::new(None));
                    let cb_err_for_closure = std::sync::Arc::clone(&callback_error);
                    let stream_result = run_blocking(py, async move {
                        $(py_refs!($t $a refs);)*
                        let mut request = client.market_data().$name($(py_call_field!($t $a refs)),*);
                        $(
                            if let Some(value) = &$oa {
                                request = request.$op(py_set_ref!($ot value));
                            }
                        )*
                        if let Some(ms) = timeout_ms {
                            request = request.with_deadline(std::time::Duration::from_millis(ms));
                        }
                        request.stream_ticks(|chunk| {
                            Python::attach(|py| {
                                let delivered = [<$item:snake s_to_pyclass_list>](py, chunk).and_then(|py_list| {
                                    let _reentry_guard = crate::DeliveryHandlerGuard::enter();
                                    handler_for_closure.call1(py, (py_list,)).map(drop)
                                });
                                match delivered {
                                    Ok(()) => std::ops::ControlFlow::Continue(()),
                                    Err(e) => {
                                        // The first error ends the stream.
                                        *cb_err_for_closure.lock().unwrap() = Some(e);
                                        std::ops::ControlFlow::Break(())
                                    }
                                }
                            })
                        }).await
                    });
                    if let Some(py_err) = callback_error.lock().unwrap().take() {
                        return Err(py_err);
                    }
                    stream_result?;
                    Ok(())
                }

                #[doc = concat!("Async companion to `stream()`: the awaitable yields `None` when the streamed response of `", stringify!($name), "` rows finishes. `handler(chunk: ", stringify!([<$item List>]), ") -> None` receives each gRPC chunk as `stream()` does, rows and roots, one call at a time, on a worker thread; a stream's next chunk is not fetched until its handler call returns. ", py_fan_out!($($attr)*), "Cancelling the awaitable drops the in-flight gRPC stream (RST_STREAM on the underlying h2 stream) and fetches no further chunks. A handler already running when cancellation lands runs to completion (Python is not interruptible mid-call), so one final `handler` call may finish after the awaitable is cancelled or its deadline expires.")]
                fn stream_async<'py>(&self, py: Python<'py>, handler: Py<PyAny>) -> PyResult<Bound<'py, PyAny>> {
                    let client = self.client.clone();
                    $(let $a = self.$a.clone();)*
                    $(let $oa = py_field!($ot self.$oa);)*
                    let timeout_ms = self.timeout_ms;
                    let handler_arc = std::sync::Arc::new(handler);
                    let handler_for_closure = std::sync::Arc::clone(&handler_arc);
                    let callback_error: std::sync::Arc<std::sync::Mutex<Option<PyErr>>> =
                        std::sync::Arc::new(std::sync::Mutex::new(None));
                    let cb_err_for_closure = std::sync::Arc::clone(&callback_error);
                    let cb_err_for_convert = std::sync::Arc::clone(&callback_error);
                    spawn_awaitable(py, async move {
                        $(py_refs!($t $a refs);)*
                        let mut request = client.market_data().$name($(py_call_field!($t $a refs)),*);
                        $(
                            if let Some(value) = &$oa {
                                request = request.$op(py_set_ref!($ot value));
                            }
                        )*
                        if let Some(ms) = timeout_ms {
                            request = request.with_deadline(std::time::Duration::from_millis(ms));
                        }
                        Ok::<_, thetadatadx::Error>(request.stream_ticks_async(|chunk| {
                            let handler_for_task = std::sync::Arc::clone(&handler_for_closure);
                            let cb_err_for_task = std::sync::Arc::clone(&cb_err_for_closure);
                            let cb_err_for_join = std::sync::Arc::clone(&cb_err_for_closure);
                            async move {
                                // The GIL is taken only inside `spawn_blocking`, never
                                // while this task awaits the join.
                                let join = tokio::task::spawn_blocking(move || {
                                    Python::attach(|py| {
                                        let py_list = match [<$item:snake s_to_pyclass_list>](py, chunk) {
                                            Ok(list) => list,
                                            Err(e) => {
                                                *cb_err_for_task.lock().unwrap() = Some(e);
                                                return;
                                            }
                                        };
                                        let _reentry_guard = crate::DeliveryHandlerGuard::enter();
                                        if let Err(e) = handler_for_task.call1(py, (py_list,)) {
                                            *cb_err_for_task.lock().unwrap() = Some(e);
                                        }
                                    })
                                }).await;
                                if let Err(join_err) = join {
                                    crate::async_runtime::capture_join_error(&cb_err_for_join, join_err);
                                }
                                if cb_err_for_join.lock().unwrap().is_some() {
                                    std::ops::ControlFlow::Break(())
                                } else {
                                    std::ops::ControlFlow::Continue(())
                                }
                            }
                        }).await)
                    }, move |py, stream_result| {
                        if let Some(py_err) = cb_err_for_convert.lock().unwrap().take() {
                            return Err(py_err);
                        }
                        stream_result.map_err(to_py_err)?;
                        Ok::<_, PyErr>(py.None())
                    })
                }
            }
        }
    };
    (@ret to_pyclass_list $item:ident) => { thetadatadx::__paste!([<$item List>]) };
    (@ret vec_to_pylist $item:ident) => { pyo3::types::PyList };
    (@convert to_pyclass_list $item:ident) => { thetadatadx::__paste!([<$item:snake s_to_pyclass_list>]) };
    (@convert vec_to_pylist $item:ident) => { ticks_to_pylist::<_, $item> };
    (@stored_req Symbols) => { Vec<String> };
    (@stored_req $t:ident) => { String };
}

/// Expands the endpoint table into the endpoint methods, the builder classes
/// and their registration.
macro_rules! py_market_data {
    ($( $(#[$attr:ident])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident,
        rest: $rest:tt,
        returns: $ret:ident $(($col:tt))?,
        params: [$($key:tt)*],
        doc: [$($doc:tt),* $(,)?],
    } )*) => {
        $( thetadatadx::__resolve! { [$($key)*] => py_endpoint { [$($attr)*] $shape $name $ret $(($col))? [$($doc)*] } } )*

        thetadatadx::__paste! {
            pub(crate) fn register_market_data_builders(m: &Bound<'_, PyModule>) -> PyResult<()> {
                $(m.add_class::<[<$name:camel Builder>]>()?;)*
                Ok(())
            }
        }
    };
}

thetadatadx::__endpoints! { py_market_data {} }

// Offline-decode hook used by the external parity bench harness: feed a
// recorded `Vec<&[u8]>` of `proto::ResponseData` chunks (one per on-the-wire
// gRPC frame) and receive the same typed pyclass list the corresponding
// endpoint would have returned. Separates decode cost from end-to-end wall
// time so benchmarks can attribute CPU to the parts of the stack they
// actually exercise.

/// Merge a list of prost-encoded `proto::ResponseData` frames into the same
/// single `DataTable` the live gRPC collector would produce.
///
/// Mirrors `MarketDataClient::collect_stream`: each chunk is prost-decoded, then
/// the zstd payload is decompressed and the rows are concatenated. Headers
/// are taken from the first chunk that carries them (matches collect_stream
/// semantics).
fn decode_chunks_into_table(chunks: &[&[u8]]) -> PyResult<thetadatadx::wire::DataTable> {
    let mut headers: Vec<String> = Vec::new();
    let mut rows: Vec<thetadatadx::wire::DataValueList> = Vec::new();
    for (idx, chunk) in chunks.iter().enumerate() {
        let mut response = <thetadatadx::wire::ResponseData as prost::Message>::decode(*chunk)
            .map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "decode_response_bytes: chunk {idx} is not a valid proto::ResponseData: {e}"
                ))
            })?;
        // Offline test harness path: uses the default ceiling
        // (`DEFAULT_MAX_MESSAGE_SIZE`) which is the earlier channel-side
        // default. Test fixtures embed pre-captured `ResponseData`
        // bytes that were already validated against the live channel's
        // `max_message_size` at capture time. The ceiling exists to
        // defend against a hostile peer, not against re-decoding
        // captured frames.
        let table = thetadatadx::decode::decode_data_table(&mut response).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "decode_response_bytes: chunk {idx} decode failed: {e}"
            ))
        })?;
        if headers.is_empty() {
            headers = table.headers;
        } else if !table.headers.is_empty() && table.headers != headers {
            // Same mid-stream invariant the live `collect_stream` enforces: a
            // chunk that re-declares headers must match the first chunk's, else
            // the concatenated rows would be projected against the wrong columns.
            return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "decode_response_bytes: chunk {idx} header drift: first [{first}] != chunk [{chunk}]",
                first = headers.join(","),
                chunk = table.headers.join(","),
            )));
        }
        rows.extend(table.data_table);
    }
    Ok(thetadatadx::wire::DataTable {
        headers,
        data_table: rows,
    })
}

/// Expands `decode_response_bytes` over every tick endpoint. A list endpoint
/// has no typed decoder, so the sort pass drops it before the one `match` is
/// emitted.
macro_rules! py_decode {
    (@sort [$($arms:tt)*] $(#[$attr:ident])* list $name:ident { $($body:tt)* } $($rest:tt)*) => {
        py_decode! { @sort [$($arms)*] $($rest)* }
    };
    (@sort [$($arms:tt)*] $(#[$attr:ident])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident, rest: $r:tt, returns: $item:ident, $($body:tt)*
    } $($rest:tt)*) => {
        py_decode! { @sort [$($arms)* { $name $item }] $($rest)* }
    };
    (@sort [$({ $name:ident $item:ident })*]) => {
        /// Decode recorded gRPC response bytes into the same typed pyclass list
        /// the matching `Client` endpoint would have returned.
        ///
        /// `endpoint` is the name of a market-data endpoint that returns typed rows,
        /// which is every endpoint except those returning a list of strings.
        /// `chunks` is the recorded byte list; one prost-encoded `proto::ResponseData`
        /// per entry, same frame boundaries tonic hands out of `Streaming::next()`.
        ///
        /// Unknown `endpoint` values return [`PyValueError`]; the caller is
        /// expected to stage the right fixture for the right endpoint.
        #[pyfunction]
        #[pyo3(signature = (endpoint, chunks))]
        pub(crate) fn decode_response_bytes(py: Python<'_>, endpoint: &str, chunks: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
            let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();
            match endpoint {
                $(stringify!($name) => {
                    let table = decode_chunks_into_table(&refs)?;
                    let rows = thetadatadx::__parse_fn!($item)(&table).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
                    let header_refs: Vec<&str> = table.headers.iter().map(String::as_str).collect();
                    let columns = <tick::$item as thetadatadx::WireColumns>::present_columns(&header_refs);
                    // Carry the response's `symbol` (root) as the leading column, as the
                    // live collector does: a constant broadcast, a per-row column, or neither.
                    let columns = if columns.contains("symbol") {
                        columns
                    } else {
                        match thetadatadx::decode::response_symbol(&table) {
                            thetadatadx::decode::ResponseSymbol::Constant(symbol) => columns.with_symbol(symbol),
                            thetadatadx::decode::ResponseSymbol::PerRow(symbols) => columns.with_symbols(symbols),
                            thetadatadx::decode::ResponseSymbol::Absent => columns,
                        }
                    };
                    let ticks = thetadatadx::Ticks::new(rows, columns);
                    Ok(thetadatadx::__paste!([<$item:snake s_to_pyclass_list>])(py, ticks)?.into_any())
                })*
                other => Err(PyValueError::new_err(format!(
                    "decode_response_bytes: unknown endpoint `{other}`; expected a market-data endpoint that returns typed rows, not a list of strings"
                ))),
            }
        }
    };
}

thetadatadx::__endpoints! { py_decode { @sort [] } }
