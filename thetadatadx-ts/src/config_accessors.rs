//! The `Config` setters and getters that read or write one configuration
//! value, expanded from the core config table.
//!
//! `#[napi]` reads argument and return types as written and needs a string
//! literal for `js_name`, so each entry first gets its names pasted and its
//! types resolved, and only then becomes a method. A `u64` or `usize` crosses
//! as a `bigint`; a `u32` or `u16` arrives as a JavaScript number and is
//! refused unless it is a whole number in range, never wrapped.

use crate::Config;
use crate::config_class::bigint_to_u64;
use thetadatadx::config;

macro_rules! ts_config_accessors {
    ({} $($dir:ident $sym:ident $sig:tt $target:tt $body:tt)*) => {
        $(ts_config_accessors! { @names $dir $sym $sig $target $body })*
    };

    // The Rust method, the JavaScript name, and for a setter the camelCase
    // value name its validation errors report.
    (@names set $sym:ident $($k:tt)*) => {
        thetadatadx::__paste! {
            ts_config_accessors! { @set
                [<$sym:replace("thetadatadx_config_", "")>]
                [<$sym:replace("thetadatadx_config_", ""):lower_camel>]
                [<$sym:replace("thetadatadx_config_set_", ""):lower_camel>]
                $($k)* }
        }
    };
    (@names get $sym:ident $($k:tt)*) => {
        thetadatadx::__paste! {
            ts_config_accessors! { @get
                [<$sym:replace("thetadatadx_config_get_", "")>]
                [<$sym:replace("thetadatadx_config_get_", ""):lower_camel>]
                $($k)* }
        }
    };

    // An enum takes and returns its lowercase name.
    (@set $f:ident $js:ident $camel:ident ($p:ident: i32) $target:tt {
        param: $bp:ident,
        values: $enum:ident $values:tt,
        errors: { null: $null:ident, ffi: $ffi:literal, py: $py:literal, ts: $err:literal $(,)? },
        doc: $doc:tt,
        cpp_doc: $cpp:tt,
        py_doc: $pyd:tt,
        ts_doc: [$($d:tt),* $(,)?] $(,)?
    }) => {
        ts_config_accessors! { @setter $f $js [String] [
            let parsed = config::$enum::parse(&$bp)
                .ok_or_else(|| crate::invalid_parameter_err(format!($err, $bp = $bp)))?;
        ] parsed $bp $target [$($d)*] }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: Option<u32>) $target:tt {
        param: $bp:ident, $($body:tt)*
    }) => {
        ts_config_accessors! { @set_body $f $js [Option<f64>] [
            let resolved = match $bp {
                Some(v) => Some(crate::validate_u32_arg(stringify!($js), v)?),
                None => None,
            };
        ] resolved $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: String) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [String] [] $bp $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: bool) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [bool] [] $bp $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: u64) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [napi::bindgen_prelude::BigInt] [
            let value = bigint_to_u64(stringify!($js), &$bp)?;
        ] value $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: usize) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [napi::bindgen_prelude::BigInt] [
            let value = bigint_to_u64(stringify!($js), &$bp)?;
            let value = usize::try_from(value)
                .map_err(|_| napi::Error::from_reason("value exceeds usize on this platform"))?;
        ] value $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: u32) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [f64] [
            let value = crate::validate_u32_arg(stringify!($camel), $bp)?;
        ] value $bp $target $($body)* }
    };
    (@set $f:ident $js:ident $camel:ident ($p:ident: u16) $target:tt { param: $bp:ident, $($body:tt)* }) => {
        ts_config_accessors! { @set_body $f $js [f64] [
            let value = crate::validate_u32_arg(stringify!($camel), $bp)?;
            let value = u16::try_from(value).map_err(|_| {
                crate::invalid_parameter_err(format!(
                    concat!(stringify!($js), ": port must be in the u16 range 0..=65535; got {}"),
                    value
                ))
            })?;
        ] value $bp $target $($body)* }
    };
    (@set_body $f:ident $js:ident $arg:tt $decode:tt $value:ident $bp:ident $target:tt
        doc: $doc:tt, cpp_doc: $cpp:tt, py_doc: $pyd:tt, ts_doc: [$($d:tt),* $(,)?] $(,)?
    ) => {
        ts_config_accessors! { @setter $f $js $arg $decode $value $bp $target [$($d)*] }
    };
    // The value is decoded and validated before the lock is taken.
    (@setter $f:ident $js:ident [$($arg:tt)*] [$($decode:tt)*] $value:ident $bp:ident $target:tt [$($d:tt)*]) => {
        thetadatadx::__paste! {
            #[napi]
            impl Config {
                $(#[doc = $d])*
                #[napi(js_name = $js "")]
                pub fn $f(&self, $bp: $($arg)*) -> napi::Result<()> {
                    $($decode)*
                    let mut guard = self
                        .inner
                        .lock()
                        .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
                    thetadatadx::__config_write!(guard $target = $value);
                    Ok(())
                }
            }
        }
    };

    (@get $f:ident $js:ident ($p:ident: i32) $target:tt {
        values: $enum:ident $values:tt, $($body:tt)*
    }) => {
        ts_config_accessors! { @get_body $f $js [&'static str] [as_str] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: Option<u32>) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [Option<u32>] [] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: String) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [String] [to_string] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: bool) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [bool] [] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: u32) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [u32] [] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: u16) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [u32] [u32::from] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: u64) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [napi::bindgen_prelude::BigInt] [napi::bindgen_prelude::BigInt::from] $target $($body)* }
    };
    (@get $f:ident $js:ident ($p:ident: usize) $target:tt { $($body:tt)* }) => {
        ts_config_accessors! { @get_body $f $js [napi::bindgen_prelude::BigInt] [usize_to_bigint] $target $($body)* }
    };
    (@get_body $f:ident $js:ident [$($ret:tt)*] $convert:tt $target:tt
        doc: $doc:tt, cpp_doc: $cpp:tt, py_doc: $pyd:tt, ts_doc: [$($d:tt),* $(,)?] $(,)?
    ) => {
        thetadatadx::__paste! {
            #[napi]
            impl Config {
                $(#[doc = $d])*
                #[napi(getter, js_name = $js "")]
                pub fn $f(&self) -> napi::Result<$($ret)*> {
                    let guard = self
                        .inner
                        .lock()
                        .map_err(|_| napi::Error::from_reason("Config mutex poisoned"))?;
                    Ok(ts_config_accessors!(@convert $convert thetadatadx::__config_read!(guard $target)))
                }
            }
        }
    };
    (@convert [] $v:expr) => {
        $v
    };
    (@convert [as_str] $v:expr) => {
        $v.as_str()
    };
    (@convert [to_string] $v:expr) => {
        $v.to_string()
    };
    (@convert [usize_to_bigint] $v:expr) => {
        napi::bindgen_prelude::BigInt::from($v as u64)
    };
    (@convert [$($f:tt)+] $v:expr) => {
        $($f)+($v)
    };
}

thetadatadx::__config! { ts_config_accessors { {} } }
