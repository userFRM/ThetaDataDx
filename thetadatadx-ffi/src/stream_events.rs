// The `#[repr(C)]` streaming event types and `fpss_event_to_ffi`, expanded
// from the core event table. Included from `streaming.rs`, after
// `FfiBufferedEvent`, which owns the memory behind every borrowed pointer an
// event carries.

/// FPSS `Contract` shared across every data event.
///
/// `symbol` is a NUL-terminated C string; may be null when the SDK has not
/// yet resolved the server-assigned contract_id to a `ContractAssigned`
/// frame. Optional option fields (`expiration`, `right`, `strike`) use a
/// tagged-present bool because `#[repr(C)]` cannot express `Option<T>`
/// directly. `right` is the ASCII byte `b'C'` / `b'P'` (`0` when
/// `has_right` is false) and `strike` is the option strike in dollars —
/// the same notation the public option builder takes.
#[repr(C)]
pub struct ThetaDataDxContract {
    /// Ticker symbol (e.g. "AAPL"). Null until ContractAssigned arrives.
    pub symbol: *const c_char,
    /// Security type code — matches `thetadatadx::SecType`.
    pub sec_type: i32,
    /// Whether `expiration` is meaningful (options only).
    pub has_expiration: bool,
    /// Option expiration date as `YYYYMMDD` (0 when `has_expiration` is false).
    pub expiration: i32,
    /// Whether `right` is meaningful (options only).
    pub has_right: bool,
    /// Option side as ASCII: `b'C'` / `b'P'` (`0` when `has_right` is false).
    pub right: c_char,
    /// Whether `strike` is meaningful (options only).
    pub has_strike: bool,
    /// Option strike price in dollars (0.0 when `has_strike` is false).
    pub strike: f64,
    /// Option strike in thousandths of a dollar (a `$550.00` strike is
    /// `550000`; `0` when `has_strike` is false). The exact integer the
    /// wire carries — read this for an exact key, `strike` for dollars.
    pub strike_thousandths: i32,
}

/// The C contract and the C string that backs its `symbol`.
fn ffi_contract(contract: &thetadatadx::fpss::protocol::Contract) -> (ThetaDataDxContract, Option<CString>) {
    let symbol = (!contract.symbol.is_empty()).then(|| cstring_for_ffi(&*contract.symbol));
    let ffi = ThetaDataDxContract {
        symbol: symbol.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        sec_type: contract.sec_type as i32,
        has_expiration: contract.expiration.is_some(),
        expiration: contract.expiration.unwrap_or(0),
        has_right: contract.is_call.is_some(),
        right: contract.right().map_or(0, |r| r.as_char() as c_char),
        has_strike: contract.strike_thousandths.is_some(),
        strike: contract.strike_dollars().unwrap_or(0.0),
        strike_thousandths: contract.strike_thousandths.unwrap_or(0),
    };
    (ffi, symbol)
}

/// A C string of `value`, with any interior NUL replaced by `?`. A borrowed
/// `&str` is copied once into a buffer sized for the NUL; an owned `String`
/// is moved in.
fn cstring_for_ffi(value: impl Into<Vec<u8>>) -> CString {
    CString::new(value).unwrap_or_else(|nul| {
        let mut bytes = nul.into_vec();
        for b in bytes.iter_mut().filter(|b| **b == 0) {
            *b = b'?';
        }
        CString::new(bytes).expect("embedded NULs were replaced")
    })
}

/// The lines of `fpss_layout_asserts.hpp.inc` that pin a struct's size and
/// field offsets, computed from the Rust layout.
#[cfg(test)]
macro_rules! c_layout {
    ($ty:ty; $($f:ident)*) => {
        vec![
            format!("sizeof({}) == {},", stringify!($ty), std::mem::size_of::<$ty>()),
            $(format!("offsetof({}, {}) == {},", stringify!($ty), stringify!($f), std::mem::offset_of!($ty, $f)),)*
        ]
    };
}

#[cfg(test)]
impl ThetaDataDxContract {
    fn c_layout() -> Vec<String> {
        c_layout!(ThetaDataDxContract; symbol sec_type has_expiration expiration has_right right has_strike strike strike_thousandths)
    }
}

/// Expands `__events!` into one `#[repr(C)]` struct per event, the kind tag,
/// the tagged `ThetaDataDxStreamEvent` and `fpss_event_to_ffi`.
macro_rules! ffi_events {
    ({} $( $(#[$attr:ident])* $kind:ident $name:ident {
        doc: $doc:literal,
        $(from: $v:ident $({ $($b:ident),* })?,)?
        columns: [$($c:ident : $t:ident $(= $e:tt)? $d:literal),* $(,)?],
    } )*) => {
        $( ffi_events! { @struct $name $doc [] [] [$({ $c $t $d })*] } )*

        ffi_events! { @arms [] $({ $kind $name [$($v $({ $($b),* })?)?] [$({ $c $t [$($e)?] })*] })* }

        ffi_events! { @sort [] [] [$($name)*] $({ $kind $name })* }
    };

    // One event struct. A byte payload is a pointer and a length; an event
    // without columns carries one padding byte, because an empty C struct
    // has size 1 on MSVC and 0 elsewhere.
    (@struct $n:ident $doc:literal [$($fld:tt)*] [$($pf:ident)*] [{ $c:ident Contract $d:literal } $($rest:tt)*]) => {
        ffi_events! { @struct $n $doc [$($fld)* #[doc = $d] pub $c: ThetaDataDxContract,] [$($pf)* $c] [$($rest)*] }
    };
    (@struct $n:ident $doc:literal [$($fld:tt)*] [$($pf:ident)*] [{ $c:ident String $d:literal } $($rest:tt)*]) => {
        ffi_events! { @struct $n $doc [$($fld)* #[doc = $d] pub $c: *const c_char,] [$($pf)* $c] [$($rest)*] }
    };
    (@struct $n:ident $doc:literal [$($fld:tt)*] [$($pf:ident)*] [{ $c:ident Bytes $d:literal } $($rest:tt)*]) => {
        thetadatadx::__paste! {
            ffi_events! { @struct $n $doc [$($fld)*
                #[doc = concat!($d, " Pointer to the first byte; null when empty.")]
                pub $c: *const u8,
                #[doc = concat!("Length in bytes of the `", stringify!($c), "` buffer.")]
                pub [<$c _len>]: usize,
            ] [$($pf)* $c [<$c _len>]] [$($rest)*] }
        }
    };
    (@struct $n:ident $doc:literal [$($fld:tt)*] [$($pf:ident)*] [{ $c:ident $t:ident $d:literal } $($rest:tt)*]) => {
        ffi_events! { @struct $n $doc [$($fld)* #[doc = $d] pub $c: $t,] [$($pf)* $c] [$($rest)*] }
    };
    (@struct $n:ident $doc:literal [] [] []) => {
        ffi_events! { @struct $n $doc [
            /// Placeholder so the struct has size 1 on every C compiler.
            /// Empty `#[repr(C)]` structs are size 0 on Rust but size 1 on
            /// MSVC; the byte keeps the layout consistent across both.
            pub _padding: u8,
        ] [_padding] [] }
    };
    (@struct $n:ident $doc:literal [$($fld:tt)+] [$($pf:ident)+] []) => {
        thetadatadx::__paste! {
            #[doc = $doc]
            #[repr(C)]
            pub struct [<ThetaDataDxStream $n>] {
                $($fld)*
            }

            #[cfg(test)]
            impl [<ThetaDataDxStream $n>] {
                fn c_layout() -> Vec<String> {
                    c_layout!([<ThetaDataDxStream $n>]; $($pf)*)
                }
            }
        }
    };

    // `fpss_event_to_ffi`: a data event binds the core fields of its column
    // names, a control event what its `from` names. An event without a core
    // variant is the fallback for one this build does not know.
    (@arms [$($arm:tt)*] { data $name:ident [] [$({ $c:ident $t:ident $e:tt })*] } $($rest:tt)*) => {
        ffi_events! { @arms [$($arm)* {
            (StreamEvent::Data(StreamData::$name { $($c,)* .. })) $name [$({ $c $t $e })*]
        }] $($rest)* }
    };
    (@arms $arms:tt { control $name:ident [] $cols:tt } $($rest:tt)*) => {
        ffi_events! { @arms $arms $($rest)* }
    };
    (@arms [$($arm:tt)*] { control $name:ident [$($from:tt)+] $cols:tt } $($rest:tt)*) => {
        ffi_events! { @arms [$($arm)* { (StreamEvent::Control(StreamControl::$($from)+)) $name $cols }] $($rest)* }
    };
    (@arms [$({ ($($pat:tt)*) $name:ident $cols:tt })*]) => {
        pub(crate) fn fpss_event_to_ffi(event: &thetadatadx::fpss::StreamEvent) -> FfiBufferedEvent {
            use thetadatadx::fpss::{StreamControl, StreamData, StreamEvent};

            match event {
                $( $($pat)* => ffi_events!(@convert $name [] [] [] $cols), )*
                _ => UNKNOWN_EVENT,
            }
        }
    };

    // One converted event: each column's value, and for a contract, string
    // or byte payload the owner of the memory its pointer borrows.
    (@convert $n:ident [$($l:tt)*] [$($f:tt)*] [$($s:tt)*] [{ $c:ident Contract $e:tt } $($rest:tt)*]) => {
        ffi_events!(@convert $n [$($l)* let $c = ffi_contract(ffi_events!(@ref $c $e));]
            [$($f)* $c: $c.0,] [$($s)* _contract_symbol: $c.1,] [$($rest)*])
    };
    (@convert $n:ident [$($l:tt)*] [$($f:tt)*] [$($s:tt)*] [{ $c:ident String $e:tt } $($rest:tt)*]) => {
        ffi_events!(@convert $n [$($l)* let $c = cstring_for_ffi(ffi_events!(@value $c $e));]
            [$($f)* $c: $c.as_ptr(),] [$($s)* _string: Some($c),] [$($rest)*])
    };
    (@convert $n:ident [$($l:tt)*] [$($f:tt)*] [$($s:tt)*] [{ $c:ident Bytes $e:tt } $($rest:tt)*]) => {
        thetadatadx::__paste! {
            ffi_events!(@convert $n [$($l)* let $c: Vec<u8> = ffi_events!(@value $c $e);]
                [$($f)* $c: if $c.is_empty() { ptr::null() } else { $c.as_ptr() }, [<$c _len>]: $c.len(),]
                [$($s)* _payload_bytes: Some($c),] [$($rest)*])
        }
    };
    (@convert $n:ident $l:tt [$($f:tt)*] $s:tt [{ $c:ident $t:ident $e:tt } $($rest:tt)*]) => {
        ffi_events!(@convert $n $l [$($f)* $c: ffi_events!(@value $c $e),] $s [$($rest)*])
    };
    (@convert $n:ident [] [] [] []) => {
        ffi_events!(@convert $n [] [_padding: 0,] [] [])
    };
    (@convert $n:ident [$($l:tt)*] [$($f:tt)+] [$($s:tt)*] []) => {
        thetadatadx::__paste! {{
            $($l)*
            FfiBufferedEvent {
                event: ThetaDataDxStreamEvent {
                    kind: ThetaDataDxStreamEventKind::$n,
                    [<$n:snake>]: [<ThetaDataDxStream $n>] { $($f)* },
                    ..ZERO_STREAM_EVENT
                },
                $($s)*
                ..UNKNOWN_EVENT
            }
        }}
    };
    // A data column reads the bound core field, a control column computes its
    // value from what `from` binds; a contract is read by reference.
    (@value $c:ident []) => { *$c };
    (@value $c:ident [($($e:tt)*)]) => { $($e)* };
    (@ref $c:ident []) => { $c };
    (@ref $c:ident [($($e:tt)*)]) => { $($e)* };

    // The kind tag and the tagged event. Tags number the events in table
    // order; the tagged event embeds the data payloads first, then the
    // control payloads.
    (@sort [$($d:ident)*] $k:tt $all:tt { data $name:ident } $($rest:tt)*) => {
        ffi_events! { @sort [$($d)* $name] $k $all $($rest)* }
    };
    (@sort $d:tt [$($k:ident)*] $all:tt { control $name:ident } $($rest:tt)*) => {
        ffi_events! { @sort $d [$($k)* $name] $all $($rest)* }
    };
    (@sort [$($d:ident)*] [$($k:ident)*] [$($a:ident)*]) => {
        thetadatadx::__paste! {
            ffi_events! { @tagged [$($d [<$d:snake>])* $($k [<$k:snake>])*] [$($a [<$a:snake>])*] }
        }
    };
    (@tagged [$($p:ident $ps:ident)*] [$($a:ident $as:ident)*]) => {
        thetadatadx::__paste! {
            /// FPSS event kind tag. Check this to determine which field of
            /// `ThetaDataDxStreamEvent` is valid. One discriminant per data variant
            /// (Quote / Trade / OpenInterest / Ohlcvc) and one per control
            /// variant (LoginSuccess / ContractAssigned / ...). Values are stable
            /// for the lifetime of the current C ABI but may renumber on a future
            /// major bump.
            #[repr(C)]
            pub enum ThetaDataDxStreamEventKind {
                $(
                    #[doc = "`" $a "` event; the `" $as "` field carries its payload."]
                    $a,
                )*
            }

            /// Tagged FPSS event for FFI. Check `kind` then read the corresponding
            /// field. Only the field matching `kind` contains valid data; this is
            /// a flat struct (not a C union) for simplicity and safety. The
            /// per-variant control payloads (LoginSuccess, ContractAssigned, ...)
            /// mirror the Rust `StreamControl::*` enum one-for-one; consumers
            /// dispatch via `kind` and read the matching `event.<variant>` field.
            #[repr(C)]
            pub struct ThetaDataDxStreamEvent {
                /// Discriminant selecting which payload field below is valid.
                pub kind: ThetaDataDxStreamEventKind,
                $(
                    #[doc = "`" $p "` payload; valid when `kind` is `" $p "`."]
                    pub $ps: [<ThetaDataDxStream $p>],
                )*
            }

            // SAFETY: every field of the tagged event is an integer, a float,
            // a bool, a raw pointer or the kind tag, whose first value is 0,
            // so the all-zero bit pattern is a valid value of each.
            const ZERO_STREAM_EVENT: ThetaDataDxStreamEvent = unsafe { std::mem::zeroed() };

            /// The event delivered for one this build does not know; it owns no memory.
            const UNKNOWN_EVENT: FfiBufferedEvent = FfiBufferedEvent {
                event: ThetaDataDxStreamEvent {
                    kind: ThetaDataDxStreamEventKind::UnknownControl,
                    ..ZERO_STREAM_EVENT
                },
                _contract_symbol: None,
                _string: None,
                _payload_bytes: None,
            };

            #[cfg(test)]
            mod c_layout_tests {
                use super::*;

                /// The Rust structs and the C header must agree byte for byte:
                /// every size and offset the C++ layout asserts pin is the Rust one.
                #[test]
                fn event_structs_match_the_c_layout_asserts() {
                    let asserts = include_str!("../../thetadatadx-cpp/include/fpss_layout_asserts.hpp.inc");
                    let event = c_layout!(ThetaDataDxStreamEvent; kind $($ps)*);
                    let lines = [
                        ThetaDataDxContract::c_layout(),
                        $( [<ThetaDataDxStream $a>]::c_layout(), )*
                        event,
                    ];
                    for line in lines.concat() {
                        assert!(
                            asserts.contains(&format!("static_assert({line}")),
                            "the C++ layout asserts do not pin `{line}`"
                        );
                    }
                }
            }
        }
    };
}

thetadatadx::__events! { ffi_events {{}} }
