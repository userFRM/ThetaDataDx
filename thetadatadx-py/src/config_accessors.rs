//! The `Config` properties that read or write one configuration value,
//! expanded from the core config table.
//!
//! The entries become one `#[pymethods]` block, in table order: a property
//! takes the documentation of whichever of its getter and setter comes first.
//! Documentation lines go through `concat!` so PyO3 keeps each line exactly as
//! the table writes it; a default is spelled as Python spells it, and an enum
//! lists the lowercase names it accepts.

use pyo3::prelude::*;
use thetadatadx::config;

use super::Config;

/// Collects every `__config!` entry into the methods of one block.
macro_rules! py_config_accessors {
    ({ $($done:tt)* }) => {
        thetadatadx::__paste! {
            #[pymethods]
            impl Config {
                $($done)*
            }
        }
    };

    // An enum takes and returns its lowercase name.
    ({ $($done:tt)* } set $sym:ident ($p:ident: i32) $target:tt {
        param: $bp:ident,
        default: $default:ident,
        values: $enum:ident { $($code:literal $variant:ident $vdoc:literal),* $(,)? },
        errors: { null: $null:ident, ffi: $ffi:literal, py: $err:literal, ts: $ts:literal $(,)? },
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { {
            $($done)*
            $(#[doc = concat!($d)])*
            #[doc = "Default `\"" [<$default:lower>] "\"`."]
            #[doc = ""]
            #[doc = "Accepts, case-insensitively:"]
            #[doc = ""]
            $(#[doc = "- `\"" [<$variant:lower>] "\"`: " $vdoc])*
            #[setter]
            fn [<$sym:replace("thetadatadx_config_", "")>](&self, $bp: &str) -> PyResult<()> {
                let parsed = config::$enum::parse($bp)
                    .ok_or_else(|| crate::errors::invalid_parameter_err(format!($err, $bp = $bp)))?;
                let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                thetadatadx::__config_write!(guard $target = parsed);
                Ok(())
            }
        } $($rest)* }
    };
    ({ $($done:tt)* } get $sym:ident ($p:ident: i32) $target:tt {
        default: $default:ident,
        values: $enum:ident $values:tt,
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { {
            $($done)*
            $(#[doc = concat!($d)])*
            #[doc = "Default `\"" [<$default:lower>] "\"`."]
            #[getter]
            fn [<$sym:replace("thetadatadx_config_", "")>](&self) -> &'static str {
                let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                thetadatadx::__config_read!(guard $target).as_str()
            }
        } $($rest)* }
    };

    ({ $($done:tt)* } set $sym:ident ($p:ident: Option<$t:ident>) $target:tt {
        param: $bp:ident,
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { { $($done)* } @setter $sym ($bp: Option<$t>) $target [
            $(#[doc = concat!($d)])*
            #[doc = ""]
            #[doc = "`None` unsets the value."]
        ] $($rest)* }
    };
    ({ $($done:tt)* } set $sym:ident ($p:ident: $t:ident) $target:tt {
        param: $bp:ident,
        $(default: $default:tt $($gloss:literal)?,)?
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { { $($done)* } @setter $sym ($bp: $t) $target [
            $(#[doc = concat!($d)])*
            $(#[doc = concat!("Default `", py_config_accessors!(@py $default), "`", $(" (", $gloss, ")",)? ".")])?
        ] $($rest)* }
    };
    ({ $($done:tt)* } @setter $sym:ident ($bp:ident: $($ty:tt)+) $target:tt [$($doc:tt)*] $($rest:tt)*) => {
        py_config_accessors! { {
            $($done)*
            $($doc)*
            #[setter]
            fn [<$sym:replace("thetadatadx_config_", "")>](&self, $bp: $($ty)+) {
                let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                thetadatadx::__config_write!(guard $target = $bp);
            }
        } $($rest)* }
    };
    ({ $($done:tt)* } get $sym:ident ($p:ident: String) $target:tt {
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { {
            $($done)*
            $(#[doc = concat!($d)])*
            #[getter]
            fn [<$sym:replace("thetadatadx_config_", "")>](&self) -> String {
                let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                thetadatadx::__config_read!(guard $target).to_string()
            }
        } $($rest)* }
    };
    ({ $($done:tt)* } get $sym:ident ($p:ident: $($ty:tt)+) $target:tt {
        $(default: $default:tt $($gloss:literal)?,)?
        doc: [$($d:literal),* $(,)?] $(,)?
    } $($rest:tt)*) => {
        py_config_accessors! { {
            $($done)*
            $(#[doc = concat!($d)])*
            $(#[doc = concat!("Default `", py_config_accessors!(@py $default), "`", $(" (", $gloss, ")",)? ".")])?
            #[getter]
            fn [<$sym:replace("thetadatadx_config_", "")>](&self) -> $($ty)+ {
                let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                thetadatadx::__config_read!(guard $target)
            }
        } $($rest)* }
    };

    // A default as Python spells it.
    (@py true) => { "True" };
    (@py false) => { "False" };
    (@py $v:tt) => { stringify!($v) };
}

thetadatadx::__config! { py_config_accessors { {} } }
