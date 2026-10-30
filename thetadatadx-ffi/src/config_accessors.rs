// The `thetadatadx_config_{set,get}_*` entry points, expanded from the core
// config table. A scalar setter ignores a null handle; every other accessor
// returns `-1` (or null) and sets the last error on a null pointer.

/// Expands `__config!` into one C entry point per accessor.
macro_rules! ffi_config_accessors {
    ({} $($dir:ident $sym:ident $sig:tt $target:tt $body:tt)*) => {
        $(ffi_config_accessors! { @entry $dir $sym $sig $target $body })*
    };

    // An enum code: an unknown code is refused as an invalid parameter.
    (@entry set $sym:ident ($p:ident: i32) $target:tt {
        param: $bp:ident,
        values: $enum:ident { $($code:literal $variant:ident),* },
        errors: { null: $null:ident, ffi: $expected:literal, py: $py:literal, ts: $ts:literal $(,)? },
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(config: *mut ThetaDataDxConfig, $p: i32) -> i32 {
            ffi_boundary!(-1, {
                if config.is_null() {
                    crate::error::set_error_with_code(
                        concat!(stringify!($sym), ": config handle is null"),
                        crate::error::$null,
                    );
                    return -1;
                }
                let value = match $p {
                    $($code => thetadatadx::$enum::$variant,)*
                    other => {
                        crate::error::set_error_with_code(
                            &format!(concat!(stringify!($sym), ": invalid ", stringify!($p), " {}; ", $expected), other),
                            crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                        );
                        return -1;
                    }
                };
                // SAFETY: config is a non-null pointer returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
                let config = unsafe { &mut *config };
                thetadatadx::__config_write!(config.inner $target = value);
                0
            })
        }
    };
    (@entry get $sym:ident ($p:ident: i32) $target:tt {
        values: $enum:ident { $($code:literal $variant:ident),* },
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(config: *const ThetaDataDxConfig, $p: *mut i32) -> i32 {
            ffi_boundary!(-1, {
                if config.is_null() || $p.is_null() {
                    set_error("config or out-parameter pointer is null");
                    return -1;
                }
                // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
                let config = unsafe { &*config };
                let value = match thetadatadx::__config_read!(config.inner $target) {
                    $(thetadatadx::$enum::$variant => $code,)*
                    // The enum is non-exhaustive: a later variant reads as the last code.
                    _ => ffi_config_accessors!(@last $($code)*),
                };
                // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
                unsafe {
                    *$p = value;
                }
                0
            })
        }
    };
    (@last $code:literal) => {
        $code
    };
    (@last $code:literal $($more:literal)+) => {
        ffi_config_accessors!(@last $($more)+)
    };

    // An optional value crosses as a `has_value` flag beside the value.
    (@entry set $sym:ident ($p:ident: Option<$t:ident>) $target:tt {
        param: $bp:ident,
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(config: *mut ThetaDataDxConfig, has_value: bool, $p: $t) -> i32 {
            ffi_boundary!(-1, {
                if config.is_null() {
                    set_error("config handle is null");
                    return -1;
                }
                // SAFETY: config is a non-null pointer returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
                let config = unsafe { &mut *config };
                thetadatadx::__config_write!(config.inner $target = if has_value { Some($p) } else { None });
                0
            })
        }
    };
    (@entry get $sym:ident ($p:ident: Option<$t:ident>) $target:tt {
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(
            config: *const ThetaDataDxConfig,
            out_has_value: *mut bool,
            $p: *mut $t,
        ) -> i32 {
            ffi_boundary!(-1, {
                if config.is_null() || out_has_value.is_null() || $p.is_null() {
                    set_error("config or out-parameter pointer is null");
                    return -1;
                }
                // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
                let config = unsafe { &*config };
                let (has_value, value) = match thetadatadx::__config_read!(config.inner $target) {
                    Some(v) => (true, v),
                    None => (false, 0),
                };
                // SAFETY: both out pointers checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
                unsafe {
                    *out_has_value = has_value;
                    *$p = value;
                }
                0
            })
        }
    };

    // A string crosses as a NUL-terminated UTF-8 C string; the getter's is
    // heap-owned and released with `thetadatadx_string_free`. Diagnostics name
    // the field, not the C parameter.
    (@entry set $sym:ident ($p:ident: String) $target:tt {
        param: $bp:ident,
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        thetadatadx::__paste! {
            $(#[doc = $doc])*
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $sym(config: *mut ThetaDataDxConfig, $p: *const c_char) -> i32 {
                ffi_boundary!(-1, {
                    if config.is_null() {
                        set_error("config handle is null");
                        return -1;
                    }
                    // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
                    let $p = match unsafe { cstr_to_str($p) } {
                        Ok(Some(s)) => s,
                        Ok(None) => {
                            set_error(concat!(stringify!([<$sym:replace("thetadatadx_config_set_", "")>]), " is null"));
                            return -1;
                        }
                        Err(e) => {
                            set_error(&format!(
                                concat!(stringify!([<$sym:replace("thetadatadx_config_set_", "")>]), " is not valid UTF-8: {}"),
                                e
                            ));
                            return -1;
                        }
                    };
                    // SAFETY: config is a non-null pointer returned by thetadatadx_config_* and not yet freed.
                    let config = unsafe { &mut *config };
                    thetadatadx::__config_write!(config.inner $target = $p.to_string());
                    0
                })
            }
        }
    };
    (@entry get $sym:ident ($p:ident: String) $target:tt {
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        thetadatadx::__paste! {
            $(#[doc = $doc])*
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $sym(config: *const ThetaDataDxConfig) -> *mut c_char {
                ffi_boundary!(ptr::null_mut(), {
                    if config.is_null() {
                        set_error("config handle is null");
                        return ptr::null_mut();
                    }
                    // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
                    let config = unsafe { &*config };
                    match std::ffi::CString::new(thetadatadx::__config_read!(config.inner $target).to_string()) {
                        Ok(c) => c.into_raw(),
                        Err(e) => {
                            set_error(&format!(
                                concat!(stringify!([<$sym:replace("thetadatadx_config_get_", "")>]), " contains an interior NUL: {}"),
                                e
                            ));
                            ptr::null_mut()
                        }
                    }
                })
            }
        }
    };

    // A number or a flag.
    (@entry set $sym:ident ($p:ident: $t:ident) $target:tt {
        param: $bp:ident,
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(config: *mut ThetaDataDxConfig, $p: $t) {
            ffi_boundary!((), {
                let config = require_config_mut!(config);
                thetadatadx::__config_write!(config.inner $target = $p);
            })
        }
    };
    (@entry get $sym:ident ($p:ident: $t:ident) $target:tt {
        doc: [$($doc:literal),* $(,)?],
        $($docs:tt)*
    }) => {
        $(#[doc = $doc])*
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $sym(config: *const ThetaDataDxConfig, $p: *mut $t) -> i32 {
            ffi_boundary!(-1, {
                if config.is_null() || $p.is_null() {
                    set_error("config or out-parameter pointer is null");
                    return -1;
                }
                // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.
                let config = unsafe { &*config };
                let value = thetadatadx::__config_read!(config.inner $target);
                // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.
                unsafe {
                    *$p = value;
                }
                0
            })
        }
    };
}

thetadatadx::__config! { ffi_config_accessors { {} } }
