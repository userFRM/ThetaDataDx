//! The streaming event classes and the conversion from the core event,
//! expanded from the core event table.
//!
//! Each event is a frozen `#[pyclass]`. `fpss_event_to_typed` builds one
//! from a borrowed core event in a single pass: owned strings and byte
//! payloads are copied once, and the contract is stored inline (a refcount
//! bump on its symbol) so the nested `ContractRef` is built only when a
//! consumer reads `event.contract`.

use super::*;

/// Streaming contract identifier. Surfaced on every decoded streaming data
/// event as `event.contract`. Reads `symbol`, `sec_type` (symbolic
/// string, e.g. `"STOCK"` / `"OPTION"`), `expiration` (`YYYYMMDD`),
/// `right` (`"C"` / `"P"` / `None`), and `strike` (option strike in
/// dollars) directly. `strike` carries dollars on every surface —
/// user code reads the same notation it writes when calling
/// `Contract.option(symbol, expiration=..., strike="5400", right="C")`,
/// and values join directly against historical-row `strike` columns.
///
/// Distinct from the fluent `Contract` builder (in `fluent.rs`): this
/// type is the read-only event payload; the fluent builder is what
/// users instantiate via `Contract.stock("AAPL")` / `Contract.option(...)`.
#[must_use]
#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct ContractRef {
    #[pyo3(get)]
    pub symbol: String,
    #[pyo3(get)]
    pub sec_type: String,
    #[pyo3(get)]
    pub expiration: Option<i32>,
    #[pyo3(get)]
    pub right: Option<String>,
    #[pyo3(get)]
    pub strike: Option<f64>,
    #[pyo3(get)]
    pub strike_thousandths: Option<i32>,
}

#[pymethods]
impl ContractRef {
    fn __repr__(&self) -> String {
        format!(
            "ContractRef(symbol={:?}, sec_type={:?}, expiration={:?}, right={:?}, strike={:?}, strike_thousandths={:?})",
            self.symbol,
            self.sec_type,
            self.expiration,
            self.right,
            self.strike,
            self.strike_thousandths
        )
    }
}

impl ContractRef {
    /// Build from the core contract carried by each data event. `sec_type`
    /// is the symbolic uppercase name (`"STOCK"` / `"OPTION"` / `"INDEX"` /
    /// `"RATE"` / `"UNKNOWN"`). `strike` is dollars; `strike_thousandths` is
    /// the exact wire integer (a `$550.00` strike is `550000`).
    pub(crate) fn from_core(c: &fpss::protocol::Contract) -> Self {
        Self {
            symbol: c.symbol.to_string(),
            sec_type: c.sec_type.as_str().to_string(),
            expiration: c.expiration,
            right: c.right().map(|r| r.as_char().to_string()),
            strike: c.strike_dollars(),
            strike_thousandths: c.strike_thousandths,
        }
    }
}

/// Expands `__events!` into one `#[pyclass]` per event,
/// `fpss_event_to_typed` and `register_fpss_event_classes`.
macro_rules! py_events {
    ({} $( $(#[$attr:ident])* $kind:ident $name:ident {
        doc: $doc:literal,
        $(from: $v:ident $({ $($b:ident),* })?,)?
        columns: [$($c:ident : $t:ident $(= $e:tt)? $d:literal),* $(,)?],
    } )*) => {
        $( py_events! { @class $name $doc [$($attr)*] [$({ $c $t })*] } )*

        py_events! { @arms [] $({ $kind $name [$($v $({ $($b),* })?)?] [$({ $c $t [$($e)?] })*] })* }

        pub(crate) fn register_fpss_event_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
            m.add_class::<ContractRef>()?;
            $( m.add_class::<$name>()?; )*
            Ok(())
        }
    };

    // A disconnect event also resolves its reason and cause to their names.
    (@class $name:ident $doc:literal [disconnect] $cols:tt) => {
        py_events! { @fields $name $doc [] [] [
            /// Resolved disconnect-reason name (e.g. `"TooManyRequests"`,
            /// `"InvalidCredentials"`, `"Unspecified"` for unknown codes).
            /// Derived from the wire-level `reason` integer.
            #[getter]
            fn reason_name(&self) -> &'static str {
                thetadatadx::RemoveReason::from_code(self.reason as i16).as_str()
            }

            /// Resolved disconnect-cause name: `"ServerSent"`,
            /// `"ClosedByServer"`, `"ReadFailed"`, `"WriteFailed"` or
            /// `"ReadTimeout"`. Derived from the `cause` integer.
            #[getter]
            fn cause_name(&self) -> &'static str {
                thetadatadx::fpss::DisconnectCause::name_for_code(self.cause)
            }
        ] [] [x x x x x x] $cols }
    };
    (@class $name:ident $doc:literal [] $cols:tt) => {
        py_events! { @fields $name $doc [] [] [] [] [x x x x x x] $cols }
    };

    // Fields, the contract getter and the `__repr__` columns. The contract
    // is stored inline and read through a getter; `__repr__` shows at most
    // six columns, leaving out the contract, byte payloads and the receive
    // timestamp.
    (@fields $n:ident $doc:literal [$($fld:tt)*] [$($get:tt)*] $x:tt $r:tt $budget:tt
        [{ $c:ident Contract } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* pub $c: fpss::protocol::Contract,] [$($get)*
            /// Streaming contract identity (`symbol`, `sec_type`,
            /// `expiration`, `right`, `strike`). Built on access from the
            /// event's inline contract.
            #[getter]
            fn contract(&self, py: Python<'_>) -> PyResult<Py<ContractRef>> {
                Py::new(py, ContractRef::from_core(&self.$c))
            }
        ] $x $r $budget [$($rest)*] }
    };
    (@fields $n:ident $doc:literal [$($fld:tt)*] $get:tt $x:tt $r:tt $budget:tt
        [{ $c:ident Bytes } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* #[pyo3(get)] pub $c: Vec<u8>,] $get $x $r $budget [$($rest)*] }
    };
    (@fields $n:ident $doc:literal [$($fld:tt)*] $get:tt $x:tt $r:tt $budget:tt
        [{ received_at_ns $t:ident } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* #[pyo3(get)] pub received_at_ns: $t,] $get $x $r $budget [$($rest)*] }
    };
    (@fields $n:ident $doc:literal [$($fld:tt)*] $get:tt $x:tt [$($r:tt)*] [$b0:tt $($b:tt)*]
        [{ $c:ident String } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* #[pyo3(get)] pub $c: String,] $get $x [$($r)* { $c "{:?}" }] [$($b)*] [$($rest)*] }
    };
    (@fields $n:ident $doc:literal [$($fld:tt)*] $get:tt $x:tt [$($r:tt)*] [$b0:tt $($b:tt)*]
        [{ $c:ident $t:ident } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* #[pyo3(get)] pub $c: $t,] $get $x [$($r)* { $c "{}" }] [$($b)*] [$($rest)*] }
    };
    (@fields $n:ident $doc:literal [$($fld:tt)*] $get:tt $x:tt $r:tt []
        [{ $c:ident $t:ident } $($rest:tt)*]) => {
        py_events! { @fields $n $doc [$($fld)* #[pyo3(get)] pub $c: $t,] $get $x $r [] [$($rest)*] }
    };
    (@fields $n:ident $doc:literal $fld:tt $get:tt $x:tt [] $budget:tt []) => {
        py_events! { @emit $n $doc $fld $get $x [
            fn __repr__(&self) -> String {
                String::from(concat!(stringify!($n), "()"))
            }
        ] }
    };
    (@fields $n:ident $doc:literal $fld:tt $get:tt $x:tt
        [{ $c0:ident $f0:literal } $({ $c:ident $f:literal })*] $budget:tt []) => {
        py_events! { @emit $n $doc $fld $get $x [
            fn __repr__(&self) -> String {
                format!(
                    concat!(stringify!($n), "(", stringify!($c0), "=", $f0, $(", ", stringify!($c), "=", $f,)* ")"),
                    self.$c0 $(, self.$c)*
                )
            }
        ] }
    };
    (@emit $n:ident $doc:literal [$($fld:tt)*] [$($get:tt)*] [$($x:tt)*] [$($repr:tt)*]) => {
        thetadatadx::__paste! {
            #[doc = concat!($doc)]
            #[must_use]
            #[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
            pub(crate) struct $n {
                $($fld)*
            }

            #[pymethods]
            impl $n {
                $($repr)*

                $($get)*

                #[getter]
                fn kind(&self) -> &'static str {
                    stringify!([<$n:snake>])
                }

                $($x)*
            }
        }
    };

    // The conversion arms: a data event binds the core fields of its column
    // names; a control event binds what its `from` names. An event without a
    // core variant is the fallback for one this build does not know.
    (@arms [$($arm:tt)*] { data $name:ident [] [$({ $c:ident $t:ident $e:tt })*] } $($rest:tt)*) => {
        py_events! { @arms [$($arm)* {
            (fpss::StreamEvent::Data(fpss::StreamData::$name { $($c,)* .. })) $name [$({ $c $t $e })*]
        }] $($rest)* }
    };
    (@arms $arms:tt { control $name:ident [] $cols:tt } $($rest:tt)*) => {
        py_events! { @arms $arms $($rest)* }
    };
    (@arms [$($arm:tt)*] { control $name:ident [$($from:tt)+] $cols:tt } $($rest:tt)*) => {
        py_events! { @arms [$($arm)* { (fpss::StreamEvent::Control(fpss::StreamControl::$($from)+)) $name $cols }] $($rest)* }
    };
    (@arms [$({ ($($pat:tt)*) $name:ident [$({ $c:ident $t:ident $e:tt })*] })*]) => {
        /// Convert a borrowed streaming event into its typed `#[pyclass]`.
        pub(crate) fn fpss_event_to_typed(
            py: Python<'_>,
            event: &fpss::StreamEvent,
        ) -> PyResult<Py<PyAny>> {
            match event {
                $( $($pat)* => Py::new(py, $name { $($c: py_events!(@value $t $c $e),)* }).map(Py::into_any), )*
                _ => Py::new(py, UnknownControl {}).map(Py::into_any),
            }
        }
    };

    // A column's value: a data column reads the bound core field, a control
    // column computes it from what `from` binds.
    (@value Contract $c:ident []) => { (**$c).clone() };
    (@value Contract $c:ident [($($e:tt)*)]) => { (**$($e)*).clone() };
    (@value $t:ident $c:ident []) => { *$c };
    (@value $t:ident $c:ident [($($e:tt)*)]) => { $($e)* };
}

thetadatadx::__events! { py_events {{}} }
