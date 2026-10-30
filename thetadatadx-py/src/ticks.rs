//! The tick row classes, their `<Tick>List` wrappers and iterators, and the
//! converters the market-data methods return through, expanded from the core
//! tick table (`thetadatadx::__ticks!`). `StringList` and the pyarrow handoff
//! are written by hand below.
//!
//! A row class holds a nullable column as `Optional[int]` and the contract
//! fields of a wildcard response as `Optional`, where the core row keeps a
//! value beside a presence flag because its layout is `repr(C)`.

use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use pyo3::prelude::*;
use thetadatadx::columns::ColumnPresence;
use thetadatadx::frames_api::TicksArrowExt;

use crate::{pyarrow_table_to_pandas, pyarrow_table_to_polars};

/// Expands every table entry into its classes and registers them all.
macro_rules! py_ticks {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        $(thetadatadx::__tick! { [$(#[$attr])* $name $body] => py_tick {} })*

        pub(crate) fn register_tick_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
            // By name, so `thetadatadx.__all__` lists the tick classes alphabetically.
            let mut ticks = thetadatadx::__paste! {[$(
                (stringify!($name), (|m: &Bound<'_, PyModule>| {
                    m.add_class::<$name>()?;
                    m.add_class::<[<$name List>]>()?;
                    m.add_class::<[<$name ListIter>]>()
                }) as fn(&Bound<'_, PyModule>) -> PyResult<()>),
            )*]};
            ticks.sort_unstable_by_key(|&(name, _)| name);
            for (_, add) in ticks {
                add(m)?;
            }
            m.add_class::<StringList>()?;
            m.add_class::<StringListIter>()?;
            Ok(())
        }
    };
}

/// One tick type: the row class, `<Tick>List`, its iterator and the
/// converters.
macro_rules! py_tick {
    ($name:ident $kind:ident contract $trio:tt doc $doc:tt summary $summary:tt more $more:tt
        required $required:tt columns [$($col:tt)*] timestamps $timestamps:tt flags $flags:tt) => {
        py_tick! { @cols [$name $kind $trio $summary $more $timestamps $flags] [] [$($col)*] }
    };

    // Resolve each column to its Python attribute: name, type, keyword
    // default and conversion. `lambda` is a Python keyword, so its attribute
    // takes the PEP 8 trailing underscore; Arrow columns keep the name.
    (@cols $st:tt $done:tt [{ lambda $w:ident $ty:ident $has:tt $d:tt } $($rest:tt)*]) => {
        py_tick! { @col $st $done [$($rest)*] lambda lambda_ $ty $has }
    };
    (@cols $st:tt $done:tt [{ $f:ident $w:ident $ty:ident $has:tt $d:tt } $($rest:tt)*]) => {
        py_tick! { @col $st $done [$($rest)*] $f $f $ty $has }
    };
    (@cols $st:tt $done:tt []) => {
        py_tick! { @emit $st $done }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident $pf:ident eod_num $has:tt) => {
        py_tick! { @col $st $done $rest $f $pf i32 $has }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident $pf:ident eod_date $has:tt) => {
        py_tick! { @col $st $done $rest $f $pf i32 $has }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident $pf:ident eod_num64 $has:tt) => {
        py_tick! { @col $st $done $rest $f $pf i64 $has }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident $pf:ident price $has:tt) => {
        py_tick! { @col $st $done $rest $f $pf f64 $has }
    };
    (@col $st:tt $done:tt $rest:tt $f:ident $pf:ident eod_price $has:tt) => {
        py_tick! { @col $st $done $rest $f $pf f64 $has }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident i32 []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [i32] [= 0i32] copy [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident i64 []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [i64] [= 0i64] copy [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident f64 []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [f64] [= 0.0f64] copy [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident String []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [String] [= String::new()] string [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident right []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [String] [= String::new()] right [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident calendar_status []) => {
        py_tick! { @cols $st [$($done)* { $f $pf [String] [] status [] }] $rest }
    };
    (@col $st:tt [$($done:tt)*] $rest:tt $f:ident $pf:ident $ty:ident [$has:ident]) => {
        py_tick! { @cols $st [$($done)* { $f $pf [Option<$ty>] [= None] optional [$has] }] $rest }
    };

    (@emit [$name:ident $kind:ident [$($exp:ident $strike:ident $right:ident)?] [$($summary:tt)*] [$($more:tt)*]
        [$({ $acc:ident $date:ident $ms:ident })*] [$({ $flag:ident $field:ident $lo:tt $hi:tt $fdoc:tt })*]]
        [$({ $f:ident $pf:ident [$($pyty:tt)*] [$($default:tt)*] $conv:ident [$($has:ident)?] })*]) => {
        thetadatadx::__paste! {
            #[doc = concat!($($summary),*)]
            $(#[doc = concat!($more)])*
            #[must_use]
            #[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
            #[derive(Clone)]
            pub(crate) struct $name {
                $(#[pyo3(get)] pub $pf: $($pyty)*,)*
                $(
                    #[pyo3(get)] pub $exp: Option<i32>,
                    #[pyo3(get)] pub $strike: Option<f64>,
                    #[pyo3(get)] pub $right: Option<String>,
                )?
            }

            #[pymethods]
            impl $name {
                #[new]
                #[pyo3(signature = (*, $($pf $($default)*),* $(, $exp = None, $strike = None, $right = None)?))]
                fn new($($pf: $($pyty)*,)* $($exp: Option<i32>, $strike: Option<f64>, $right: Option<String>)?) -> Self {
                    Self { $($pf,)* $($exp, $strike, $right)? }
                }

                fn __repr__(&self) -> String {
                    py_tick!(@repr self $name [x x x x x x] [] [$({ $pf $conv })*])
                }

                $(
                    #[doc = concat!("Unix epoch milliseconds (UTC, DST-aware) combining `", stringify!($date), "` with")]
                    #[doc = concat!("`", stringify!($ms), "` (Eastern-Time milliseconds-of-day). `None` when `", stringify!($date), "`")]
                    #[doc = concat!("is absent (`0`). The raw integer fields stay primary; this is a")]
                    #[doc = concat!("convenience at the epoch boundary.")]
                    #[getter]
                    fn $acc(&self) -> Option<i64> {
                        thetadatadx::time::date_ms_to_epoch_ms(self.$date, self.$ms)
                    }
                )*

                $(
                    #[doc = $fdoc]
                    #[getter]
                    fn $flag(&self) -> bool {
                        self.$field.is_some_and(|cell| ($lo..=$hi).contains(&cell))
                    }
                )*
            }

            impl From<&thetadatadx::$name> for $name {
                fn from(t: &thetadatadx::$name) -> Self {
                    Self {
                        $($pf: py_tick!(@from $conv t $f [$($has)?]),)*
                        $(
                            $exp: t.has_contract_id().then_some(t.$exp),
                            $strike: t.has_contract_id().then_some(t.$strike),
                            $right: if t.$right == '\0' { None } else { Some(t.$right.to_string()) },
                        )?
                    }
                }
            }

            #[doc = concat!("Typed list of `", stringify!($name), "` returned by every market-data endpoint")]
            #[doc = concat!("whose response is `", stringify!($name), "`. Wraps the decoder-owned `Vec<", stringify!($name), ">`")]
            #[doc = concat!("and exposes the Python list protocol (`len(ticks)`, `ticks[i]`,")]
            #[doc = concat!("`for t in ticks: ...`, `bool(ticks)`) plus the chained terminals")]
            #[doc = concat!("`.to_list()`, `.to_arrow()`, `.to_pandas()`, `.to_polars()`.")]
            #[must_use]
            #[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
            pub(crate) struct [<$name List>] {
                inner: Vec<thetadatadx::$name>,
                columns: ColumnPresence,
            }

            impl [<$name List>] {
                pub(crate) fn new(inner: Vec<thetadatadx::$name>, columns: ColumnPresence) -> Self {
                    Self { inner, columns }
                }
            }

            #[pymethods]
            impl [<$name List>] {
                #[new]
                #[pyo3(signature = (ticks = Vec::new()))]
                fn py_new(ticks: Vec<pyo3::PyRef<'_, $name>>) -> PyResult<Self> {
                    let mut inner = Vec::with_capacity(ticks.len());
                    for t in &ticks {
                        inner.push(thetadatadx::$name {
                            $(
                                $f: py_tick!(@to $conv t $f $pf),
                                $($has: t.$pf.is_some(),)?
                            )*
                            $(
                                $exp: t.$exp.unwrap_or(0),
                                $strike: t.$strike.unwrap_or(0.0),
                                $right: match t.$right.as_deref() {
                                    Some("C") => 'C',
                                    Some("P") => 'P',
                                    None | Some("") => '\0',
                                    Some(other) => return Err(pyo3::exceptions::PyValueError::new_err(format!(
                                        concat!(stringify!($right), " must be \"C\" or \"P\", got {:?}"), other
                                    ))),
                                },
                            )?
                        });
                    }
                    let columns = <thetadatadx::$name as thetadatadx::columns::WireColumns>::all_columns();
                    Ok(Self { inner, columns })
                }

                fn __len__(&self) -> usize {
                    self.inner.len()
                }

                fn __bool__(&self) -> bool {
                    !self.inner.is_empty()
                }

                fn __repr__(&self) -> String {
                    format!(concat!(stringify!($name), "List({} rows)"), self.inner.len())
                }

                fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
                    if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
                        let indices = slice.indices(self.inner.len() as isize)?;
                        let mut rows = Vec::new();
                        let src_syms = self.columns.symbols();
                        let mut syms: Vec<Box<str>> = Vec::new();
                        let mut i = indices.start;
                        // Walk exactly `slicelength` positions with the SAME index for
                        // rows and symbols. Comparing against `stop` and stepping by
                        // `indices.step` overflows `i` for a huge step (e.g.
                        // `lst[k::sys.maxsize]`); the count is exact for either
                        // direction and `wrapping_add` keeps the unused final
                        // increment from panicking.
                        for _ in 0..indices.slicelength {
                            rows.push(py_tick!(@row $kind self.inner[i as usize]));
                            if let Some(s) = src_syms { syms.push(s[i as usize].clone()); }
                            i = i.wrapping_add(indices.step);
                        }
                        let mut columns = self.columns.clone();
                        if src_syms.is_some() { columns = columns.with_symbols(syms); }
                        return Ok(Py::new(py, [<$name List>]::new(rows, columns))?.into_any());
                    }
                    let idx: isize = key.extract()?;
                    let len = self.inner.len() as isize;
                    let resolved = if idx < 0 { idx + len } else { idx };
                    if resolved < 0 || resolved >= len {
                        return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                            concat!(stringify!($name), "List index {} out of range (len={})"), idx, self.inner.len()
                        )));
                    }
                    Ok(Py::new(py, $name::from(&self.inner[resolved as usize]))?.into_any())
                }

                fn __iter__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<[<$name ListIter>]>> {
                    Py::new(py, [<$name ListIter>] { inner: slf.inner.clone(), cursor: 0 })
                }

                /// Return a plain Python `list` of typed tick pyclass instances.
                /// Explicit escape hatch for callers that want a mutable list or
                /// need to pass the data into an API that only accepts `list`.
                fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                    let list = pyo3::types::PyList::empty(py);
                    for t in &self.inner {
                        list.append(Py::new(py, $name::from(t))?)?;
                    }
                    Ok(list.into_any().unbind())
                }

                /// Return a `pyarrow.Table` built from the decoded rows. The rows are
                /// copied into Arrow columns once, and those columns cross into
                /// pyarrow without a further copy through the Arrow C Data
                /// Interface. Carries only the columns the response's wire sent.
                fn to_arrow(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                    ticks_to_arrow_table(py, &self.inner, &self.columns)
                }

                /// Return a `pandas.DataFrame` via `pyarrow.Table.to_pandas()`.
                /// Requires pandas + pyarrow: `pip install thetadatadx-py[pandas]`.
                fn to_pandas(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                    let table = ticks_to_arrow_table(py, &self.inner, &self.columns)?;
                    pyarrow_table_to_pandas(py, table)
                }

                /// Return a `polars.DataFrame` via `polars.from_arrow`.
                /// Requires polars + pyarrow: `pip install thetadatadx-py[polars]`.
                fn to_polars(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                    let table = ticks_to_arrow_table(py, &self.inner, &self.columns)?;
                    pyarrow_table_to_polars(py, table)
                }
            }

            #[doc = concat!("Companion iterator for [`", stringify!($name), "List`]. Yields `", stringify!($name), "` pyclass")]
            #[doc = concat!("instances until the wrapped `Vec` is exhausted, then raises")]
            #[doc = concat!("`StopIteration`. One allocation at construction (Vec clone); each")]
            #[doc = concat!("`__next__` only copies one tick out of the inner buffer.")]
            #[doc = concat!("")]
            #[doc = concat!("`unsendable` keeps the iterator on the thread that created it, so")]
            #[doc = concat!("under free-threaded Python (PEP 703) another thread that reaches it")]
            #[doc = concat!("gets an error at the binding boundary instead of contending for the")]
            #[doc = concat!("cursor.")]
            #[pyclass(module = "thetadatadx", unsendable, skip_from_py_object)]
            pub(crate) struct [<$name ListIter>] {
                inner: Vec<thetadatadx::$name>,
                cursor: usize,
            }

            #[pymethods]
            impl [<$name ListIter>] {
                fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
                    slf
                }

                fn __next__(&mut self) -> Option<$name> {
                    if self.cursor >= self.inner.len() {
                        return None;
                    }
                    let t = &self.inner[self.cursor];
                    self.cursor += 1;
                    Some($name::from(t))
                }
            }

            pub(crate) fn [<$name:snake s_to_pyclass_list>](
                py: Python<'_>,
                ticks: thetadatadx::Ticks<thetadatadx::$name>,
            ) -> PyResult<Py<[<$name List>]>> {
                let columns = ticks.columns().clone();
                Py::new(py, [<$name List>]::new(ticks.into_vec(), columns))
            }
        }
    };

    // `__repr__` shows the first six columns, a string between quotes and an
    // absent value as `None`.
    (@repr $s:tt $name:ident $count:tt [$($piece:tt)*] []) => {
        py_tick!(@repr_format $s $name [$($piece)*])
    };
    (@repr $s:tt $name:ident [] [$($piece:tt)*] $rest:tt) => {
        py_tick!(@repr_format $s $name [$($piece)*])
    };
    (@repr $s:tt $name:ident [$c:tt $($cs:tt)*] [$($piece:tt)*] [{ $pf:ident copy } $($rest:tt)*]) => {
        py_tick!(@repr $s $name [$($cs)*] [$($piece)* [$pf "{}" ($s.$pf)]] [$($rest)*])
    };
    (@repr $s:tt $name:ident [$c:tt $($cs:tt)*] [$($piece:tt)*] [{ $pf:ident optional } $($rest:tt)*]) => {
        py_tick!(@repr $s $name [$($cs)*] [$($piece)* [$pf "{}" ($s.$pf.map_or_else(|| "None".to_string(), |v| v.to_string()))]] [$($rest)*])
    };
    (@repr $s:tt $name:ident [$c:tt $($cs:tt)*] [$($piece:tt)*] [{ $pf:ident $conv:ident } $($rest:tt)*]) => {
        py_tick!(@repr $s $name [$($cs)*] [$($piece)* [$pf "{:?}" ($s.$pf)]] [$($rest)*])
    };
    (@repr_format $s:tt $name:ident [[$pf0:ident $fmt0:tt $arg0:tt] $([$pf:ident $fmt:tt $arg:tt])*]) => {
        format!(
            concat!(stringify!($name), "(", stringify!($pf0), "=", $fmt0 $(, ", ", stringify!($pf), "=", $fmt)*, ")"),
            $arg0 $(, $arg)*
        )
    };

    // Core row to row class.
    (@from copy $t:tt $f:ident []) => { $t.$f };
    (@from optional $t:tt $f:ident [$has:ident]) => { if $t.$has { Some($t.$f) } else { None } };
    (@from string $t:tt $f:ident []) => { $t.$f.clone() };
    (@from right $t:tt $f:ident []) => { if $t.$f == '\0' { String::new() } else { $t.$f.to_string() } };
    (@from status $t:tt $f:ident []) => { $t.$f.as_str().to_string() };

    // Row class to core row, rejecting text the core type cannot hold.
    (@to copy $t:tt $f:ident $pf:ident) => { $t.$pf };
    (@to optional $t:tt $f:ident $pf:ident) => { $t.$pf.unwrap_or_default() };
    (@to string $t:tt $f:ident $pf:ident) => { $t.$pf.clone() };
    (@to right $t:tt $f:ident $pf:ident) => {
        match $t.$pf.as_str() {
            "C" => 'C',
            "P" => 'P',
            "" => '\0',
            other => return Err(pyo3::exceptions::PyValueError::new_err(format!(
                concat!(stringify!($f), " must be \"C\" or \"P\", got {:?}"), other
            ))),
        }
    };
    (@to status $t:tt $f:ident $pf:ident) => {
        match thetadatadx::CalendarStatus::from_wire_text(&$t.$pf) {
            Some(status) => status,
            None => return Err(pyo3::exceptions::PyValueError::new_err(format!(
                concat!(stringify!($f), " must be one of open, early_close, full_close, weekend; got {:?}"), $t.$pf
            ))),
        }
    };

    (@row pod $row:expr) => { $row };
    (@row owned $row:expr) => { $row.clone() };
}

thetadatadx::__ticks! { py_ticks {} }

/// Snapshot-endpoint fast path: a plain `list` of row objects without the
/// `<Tick>List` wrapper, for results nobody chains `.to_polars()` on.
pub(crate) fn ticks_to_pylist<T, P>(
    py: Python<'_>,
    ticks: thetadatadx::Ticks<T>,
) -> PyResult<Py<pyo3::types::PyList>>
where
    P: for<'a> From<&'a T> + pyo3::PyClass + Into<pyo3::PyClassInitializer<P>>,
{
    let list = pyo3::types::PyList::empty(py);
    for t in &ticks {
        list.append(Py::new(py, P::from(t))?)?;
    }
    Ok(list.unbind())
}

/// The rows as a `pyarrow.Table` carrying the columns in `present`, built
/// with the GIL released.
fn ticks_to_arrow_table<T: Sync>(
    py: Python<'_>,
    ticks: &[T],
    present: &ColumnPresence,
) -> PyResult<Py<PyAny>>
where
    [T]: TicksArrowExt,
{
    let batch = py
        .detach(|| ticks.to_arrow_projected(present))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    record_batch_to_pyarrow_table(py, batch)
}

/// Zero-copy `RecordBatch` -> `pyarrow.Table` over the Arrow C Stream
/// Interface. One batch in, one Table out; pyarrow aliases the underlying
/// buffers in place and owns them once the import returns.
///
/// The Rust side wraps the batch in a `RecordBatchIterator` and exports it as
/// an `FFI_ArrowArrayStream`. Buffer ownership transfers through the C Data
/// Interface, so no column data is copied at the boundary.
///
/// Preferred path: the stable Arrow PyCapsule Interface (public since
/// pyarrow 14). The stream is wrapped in an `arrow_array_stream` capsule and
/// imported via `RecordBatchReader._import_from_c_capsule`. Fallback path (for
/// pyarrow < 14, which lacks the capsule entry point): the older address-based
/// `_import_from_c`. Both consumers MOVE the stream out and null its release
/// callback, so the `FFI_ArrowArrayStream` `Drop` (run by the capsule
/// destructor or on stack unwind) is a no-op on the moved-out handle.
pub(crate) fn record_batch_to_pyarrow_table(
    py: Python<'_>,
    batch: RecordBatch,
) -> PyResult<Py<PyAny>> {
    use arrow::ffi_stream::FFI_ArrowArrayStream;
    use arrow::record_batch::RecordBatchIterator;

    let schema = batch.schema();
    let reader = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
    let stream = FFI_ArrowArrayStream::new(Box::new(reader));

    let pyarrow = py.import("pyarrow")?;
    let record_batch_reader = pyarrow.getattr("RecordBatchReader")?;

    let reader_obj = if record_batch_reader.hasattr("_import_from_c_capsule")? {
        // Capsule owns the stream; `_import_from_c_capsule` moves it out and
        // nulls the release, so the capsule destructor's `Drop` is a no-op.
        let capsule = pyo3::types::PyCapsule::new_with_value(py, stream, c"arrow_array_stream")?;
        record_batch_reader.call_method1("_import_from_c_capsule", (capsule,))?
    } else {
        // Stack-owned stream handed to pyarrow by address on the legacy path.
        let mut stream = stream;
        let stream_addr = std::ptr::addr_of_mut!(stream) as usize;
        record_batch_reader.call_method1("_import_from_c", (stream_addr,))?
    };
    let table = reader_obj.call_method0("read_all")?;
    Ok(table.unbind())
}

// ─────────────── String list wrapper ───────────────
//
// Shared by every endpoint whose public return is `Vec<String>`.

/// Typed list of strings returned by list endpoints
/// (`stock_list_symbols`, `option_list_expirations`, ...).
/// Wraps `Vec<String>` and exposes the Python list protocol plus
/// the chainable terminals (`to_list`, `to_arrow`, `to_pandas`,
/// `to_polars`). The `column_name` captured at construction drives
/// the DataFrame column name produced by the Arrow terminals, so
/// `stock_list_symbols().to_polars()` lands with a typed `symbol`
/// column rather than a numeric positional index.
#[must_use]
#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
pub(crate) struct StringList {
    inner: Vec<String>,
    column_name: String,
}

impl StringList {
    pub(crate) fn new(inner: Vec<String>, column_name: &str) -> Self {
        Self {
            inner,
            column_name: column_name.to_string(),
        }
    }
}

#[pymethods]
impl StringList {
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __repr__(&self) -> String {
        format!(
            "StringList({} rows, column={:?})",
            self.inner.len(),
            self.column_name
        )
    }

    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<pyo3::types::PySlice>() {
            let indices = slice.indices(self.inner.len() as isize)?;
            let mut rows = Vec::new();
            let mut i = indices.start;
            // Walk exactly `slicelength` positions. Comparing against
            // `stop` and stepping by `indices.step` overflows `i` for a
            // huge step (e.g. `lst[k::sys.maxsize]`); the count is exact
            // for either direction and `wrapping_add` keeps the unused
            // final increment from panicking.
            for _ in 0..indices.slicelength {
                rows.push(self.inner[i as usize].clone());
                i = i.wrapping_add(indices.step);
            }
            return Ok(Py::new(py, StringList::new(rows, &self.column_name))?.into_any());
        }
        let idx: isize = key.extract()?;
        let len = self.inner.len() as isize;
        let resolved = if idx < 0 { idx + len } else { idx };
        if resolved < 0 || resolved >= len {
            return Err(pyo3::exceptions::PyIndexError::new_err(format!(
                "StringList index {} out of range (len={})",
                idx,
                self.inner.len()
            )));
        }
        Ok(
            pyo3::types::PyString::new(py, &self.inner[resolved as usize])
                .into_any()
                .unbind(),
        )
    }

    fn __iter__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<StringListIter>> {
        Py::new(
            py,
            StringListIter {
                inner: slf.inner.clone(),
                cursor: 0,
            },
        )
    }

    /// Return the wrapped values as a plain Python `list[str]`.
    fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = pyo3::types::PyList::empty(py);
        for s in &self.inner {
            list.append(s.as_str())?;
        }
        Ok(list.into_any().unbind())
    }

    /// Return a `pyarrow.Table` with a single `column_name`-named string column.
    fn to_arrow(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        string_list_slice_to_arrow_table(py, &self.inner, &self.column_name)
    }

    /// Return a `pandas.DataFrame` with a single `column_name` column.
    fn to_pandas(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let table = string_list_slice_to_arrow_table(py, &self.inner, &self.column_name)?;
        pyarrow_table_to_pandas(py, table)
    }

    /// Return a `polars.DataFrame` with a single `column_name` column.
    fn to_polars(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let table = string_list_slice_to_arrow_table(py, &self.inner, &self.column_name)?;
        pyarrow_table_to_polars(py, table)
    }
}

/// Companion iterator for [`StringList`]. Yields owned `String`s
/// until the wrapped `Vec` is exhausted. One allocation per call to
/// `__next__` (clone of the inner entry); the underlying Vec is
/// cloned once at construction so mutating it mid-iteration is
/// a non-issue.
///
/// `unsendable` keeps the iterator on the thread that created it, so
/// under free-threaded Python (PEP 703) another thread that reaches it
/// gets an error at the binding boundary instead of contending for the
/// cursor.
#[pyclass(module = "thetadatadx", unsendable, skip_from_py_object)]
pub(crate) struct StringListIter {
    inner: Vec<String>,
    cursor: usize,
}

#[pymethods]
impl StringListIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<String> {
        if self.cursor >= self.inner.len() {
            return None;
        }
        let s = self.inner[self.cursor].clone();
        self.cursor += 1;
        Some(s)
    }
}

pub(crate) fn strings_to_string_list(
    py: Python<'_>,
    values: Vec<String>,
    column_name: &str,
) -> PyResult<Py<StringList>> {
    Py::new(py, StringList::new(values, column_name))
}

fn string_list_slice_to_arrow_table(
    py: Python<'_>,
    values: &[String],
    column_name: &str,
) -> PyResult<Py<PyAny>> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        column_name,
        DataType::Utf8,
        false,
    )]));
    let column: ArrayRef = Arc::new(StringArray::from_iter_values(
        values.iter().map(String::as_str),
    ));
    let batch = RecordBatch::try_new(schema, vec![column])
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
    record_batch_to_pyarrow_table(py, batch)
}
