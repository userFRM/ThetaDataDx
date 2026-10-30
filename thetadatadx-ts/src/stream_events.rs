//! The streaming event objects, expanded from the core event table.
//!
//! The streaming callback converts each core event into a `BufferedEvent`,
//! which crosses the channel to the Node thread; `buffered_event_to_typed`
//! turns it into the flat `StreamEvent` a JavaScript callback receives:
//! a `kind` tag plus one optional typed payload per event.

use super::*;

/// Streaming contract identifier. Surfaced on every decoded streaming data
/// event as `event.quote.contract` / `event.trade.contract` / etc.
/// `secType` is the symbolic uppercase name (`"STOCK"` / `"OPTION"` /
/// `"INDEX"` / `"RATE"`); `right` is `"C"` / `"P"` / `null`;
/// `strike` is the option strike in dollars — the same unit historical
/// rows carry under the same name, so streaming contracts join against
/// historical data directly. `expiration` is a `YYYYMMDD` integer.
#[must_use]
#[napi(object)]
#[derive(Clone)]
pub struct Contract {
    pub symbol: String,
    pub sec_type: String,
    pub expiration: Option<i32>,
    pub right: Option<String>,
    pub strike: Option<f64>,
    pub strike_thousandths: Option<i32>,
}

impl Contract {
    fn from_core(c: &fpss::protocol::Contract) -> Self {
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

/// Expands `__events!` into one `#[napi(object)]` payload per event,
/// `BufferedEvent`, `StreamEvent` and the conversions between them.
macro_rules! ts_events {
    ({} $( $(#[$attr:ident])* $kind:ident $name:ident {
        doc: $doc:literal,
        $(from: $v:ident $({ $($b:ident),* })?,)?
        columns: [$($c:ident : $t:ident $(= $e:tt)? $d:literal),* $(,)?],
    } )*) => {
        $( ts_events! { @object $name $doc [$($attr)*] [] [$({ $c $t })*] } )*

        /// A core event, owned, on its way to the Node thread.
        #[derive(Clone, Debug)]
        pub(crate) enum BufferedEvent {
            $( $name { $($c: ts_events!(@rust $t),)* }, )*
        }

        ts_events! { @arms [] $({ $kind $name [$($v $({ $($b),* })?)?] [$({ $c $t [$($e)?] })*] })* }

        ts_events! { @sort [] [] [$($name)*] $({ $kind $name })* }

        pub(crate) fn buffered_event_to_typed(event: BufferedEvent) -> StreamEvent {
            thetadatadx::__paste! {
                match event {
                    $( BufferedEvent::$name { $($c),* } => StreamEvent {
                        kind: stringify!([<$name:snake>]),
                        [<$name:snake>]: Some(ts_events!(@typed $name [$($attr)*] [$({ $c $t })*])),
                        ..StreamEvent::default()
                    }, )*
                }
            }
        }
    };

    // The payload object, with each column's type resolved before napi reads
    // it: 64-bit integers cross as `bigint`, a byte as a 32-bit number.
    (@object $n:ident $doc:literal $attr:tt [$($fld:tt)*] [{ $c:ident i64 } $($rest:tt)*]) => {
        ts_events! { @object $n $doc $attr [$($fld)* pub $c: BigInt,] [$($rest)*] }
    };
    (@object $n:ident $doc:literal $attr:tt [$($fld:tt)*] [{ $c:ident u64 } $($rest:tt)*]) => {
        ts_events! { @object $n $doc $attr [$($fld)* pub $c: BigInt,] [$($rest)*] }
    };
    (@object $n:ident $doc:literal $attr:tt [$($fld:tt)*] [{ $c:ident u8 } $($rest:tt)*]) => {
        ts_events! { @object $n $doc $attr [$($fld)* pub $c: u32,] [$($rest)*] }
    };
    (@object $n:ident $doc:literal $attr:tt [$($fld:tt)*] [{ $c:ident Bytes } $($rest:tt)*]) => {
        ts_events! { @object $n $doc $attr [$($fld)* pub $c: Vec<u8>,] [$($rest)*] }
    };
    (@object $n:ident $doc:literal $attr:tt [$($fld:tt)*] [{ $c:ident $t:ident } $($rest:tt)*]) => {
        ts_events! { @object $n $doc $attr [$($fld)* pub $c: $t,] [$($rest)*] }
    };
    // A disconnect event also carries its reason and cause names.
    (@object $n:ident $doc:literal [disconnect] [$($fld:tt)*] []) => {
        ts_events! { @object $n $doc [] [$($fld)*
            // Plain literals: napi copies a forwarded doc comment into
            // `index.d.ts` with its raw-string quoting.
            #[doc = " Resolved disconnect-reason name (e.g. `\"TooManyRequests\"`,"]
            #[doc = " `\"InvalidCredentials\"`, `\"Unspecified\"` for unknown codes)."]
            #[doc = " Derived from the wire-level `reason` integer."]
            pub reason_name: String,
            #[doc = " Resolved disconnect-cause name: `\"ServerSent\"`,"]
            #[doc = " `\"ClosedByServer\"`, `\"ReadFailed\"`, `\"WriteFailed\"` or"]
            #[doc = " `\"ReadTimeout\"`. Derived from the `cause` integer."]
            pub cause_name: String,
        ] [] }
    };
    (@object $n:ident $doc:literal [] [$($fld:tt)*] []) => {
        #[doc = $doc]
        #[must_use]
        #[napi(object)]
        #[derive(Clone)]
        pub struct $n {
            $($fld)*
        }
    };

    // `fpss_event_to_buffered`: a data event binds the core fields of its
    // column names, a control event what its `from` names. An event without
    // a core variant is the fallback for one this build does not know.
    (@arms [$($arm:tt)*] { data $name:ident [] [$({ $c:ident $t:ident $e:tt })*] } $($rest:tt)*) => {
        ts_events! { @arms [$($arm)* {
            (fpss::StreamEvent::Data(fpss::StreamData::$name { $($c,)* .. })) $name [$({ $c $t $e })*]
        }] $($rest)* }
    };
    (@arms $arms:tt { control $name:ident [] $cols:tt } $($rest:tt)*) => {
        ts_events! { @arms $arms $($rest)* }
    };
    (@arms [$($arm:tt)*] { control $name:ident [$($from:tt)+] $cols:tt } $($rest:tt)*) => {
        ts_events! { @arms [$($arm)* { (fpss::StreamEvent::Control(fpss::StreamControl::$($from)+)) $name $cols }] $($rest)* }
    };
    (@arms [$({ ($($pat:tt)*) $name:ident [$({ $c:ident $t:ident $e:tt })*] })*]) => {
        pub(crate) fn fpss_event_to_buffered(event: &fpss::StreamEvent) -> BufferedEvent {
            match event {
                $( $($pat)* => BufferedEvent::$name { $($c: ts_events!(@value $t $c $e),)* }, )*
                _ => BufferedEvent::UnknownControl {},
            }
        }
    };

    // `StreamEvent`: the data payloads first, then the control payloads; the
    // `kind` union lists every tag in table order.
    (@sort [$($d:ident)*] $k:tt $all:tt { data $name:ident } $($rest:tt)*) => {
        ts_events! { @sort [$($d)* $name] $k $all $($rest)* }
    };
    (@sort $d:tt [$($k:ident)*] $all:tt { control $name:ident } $($rest:tt)*) => {
        ts_events! { @sort $d [$($k)* $name] $all $($rest)* }
    };
    (@sort [$($d:ident)*] [$($k:ident)*] [$($a:ident)*]) => {
        thetadatadx::__paste! {
            ts_events! { @stream_event [$([<$d:snake>] $d)*] [$([<$k:snake>] $k)*] [$([<$a:snake>])*] }
        }
    };
    (@stream_event [$($ds:ident $d:ident)*] [$($ks:ident $k:ident)*] [$a0:ident $($a:ident)*]) => {
        thetadatadx::__paste! {
            /// A single streaming event surfaced to JS/TS.
            ///
            /// `kind` is the discriminator; switch on it and read the matching
            /// payload field. The shape is stable and every payload is typed, so
            /// consumers never fall back to untyped `any`.
            #[must_use]
            #[napi(object, object_from_js = false)]
            #[derive(Clone, Default)]
            pub struct StreamEvent {
                /// Discriminator matching one of the typed payload fields below.
                /// Narrowed to a literal union in TS so `switch (event.kind)`
                /// correctly narrows the optional payload fields.
                #[napi(ts_type = "'" $a0 "'" $(" | '" $a "'")*)]
                pub kind: &'static str,
                $(pub $ds: Option<$d>,)*
                $(pub $ks: Option<$k>,)*
            }
        }
    };

    // A column's owned value: a data column reads the bound core field, a
    // control column computes it from what `from` binds.
    (@value Contract $c:ident []) => { (**$c).clone() };
    (@value Contract $c:ident [($($e:tt)*)]) => { (**$($e)*).clone() };
    (@value $t:ident $c:ident []) => { *$c };
    (@value $t:ident $c:ident [($($e:tt)*)]) => { $($e)* };

    (@rust Contract) => { fpss::protocol::Contract };
    (@rust Bytes) => { Vec<u8> };
    (@rust $t:ident) => { $t };

    // The payload object from the buffered columns.
    (@typed $name:ident [] [$({ $c:ident $t:ident })*]) => {
        $name { $($c: ts_events!(@js $t $c),)* }
    };
    (@typed $name:ident [disconnect] [$({ $c:ident $t:ident })*]) => {{
        let payload = $name {
            $($c: ts_events!(@js $t $c),)*
            reason_name: String::new(),
            cause_name: String::new(),
        };
        $name {
            reason_name: thetadatadx::RemoveReason::from_code(payload.reason as i16)
                .as_str()
                .to_string(),
            cause_name: thetadatadx::fpss::DisconnectCause::name_for_code(payload.cause).to_string(),
            ..payload
        }
    }};
    (@js i64 $c:ident) => { BigInt::from($c) };
    (@js u64 $c:ident) => { BigInt::from($c) };
    (@js u8 $c:ident) => { u32::from($c) };
    (@js Contract $c:ident) => { Contract::from_core(&$c) };
    (@js $t:ident $c:ident) => { $c };
}

thetadatadx::__events! { ts_events {{}} }
