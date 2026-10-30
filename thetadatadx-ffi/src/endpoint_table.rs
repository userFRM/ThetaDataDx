// The market-data C entry points and their options struct, expanded from the
// core endpoint table. Every endpoint keeps its
// `thetadatadx_<endpoint>_with_options` symbol, and a chunked endpoint whose
// rows are flat `#[repr(C)]` ticks also keeps its
// `thetadatadx_<endpoint>_stream` symbol.

/// Expands `__request_options!` into `ThetaDataDxEndpointRequestOptions` and
/// `apply_endpoint_request_options`.
macro_rules! ffi_request_options {
    ({} params { all [$({ $p:ident $a:ident opt $t:ident $v:tt $d:literal })*] $($rest:tt)* }) => {
        ffi_request_options! { @fields [$({ $p $t $d })*] [] [$({ $p $t })*] }
    };
    // A bool, int or float field is applied only when its `has_` flag is set;
    // a string pointer is applied when it is non-null.
    (@fields [{ $p:ident Bool $d:literal } $($more:tt)*] [$($out:tt)*] $apply:tt) => {
        ffi_request_options! { @flagged i32 [{ $p $d } $($more)*] [$($out)*] $apply }
    };
    (@fields [{ $p:ident Int $d:literal } $($more:tt)*] [$($out:tt)*] $apply:tt) => {
        ffi_request_options! { @flagged i32 [{ $p $d } $($more)*] [$($out)*] $apply }
    };
    (@fields [{ $p:ident Float $d:literal } $($more:tt)*] [$($out:tt)*] $apply:tt) => {
        ffi_request_options! { @flagged f64 [{ $p $d } $($more)*] [$($out)*] $apply }
    };
    (@fields [{ $p:ident $t:ident $d:literal } $($more:tt)*] [$($out:tt)*] $apply:tt) => {
        ffi_request_options! { @fields [$($more)*] [$($out)* #[doc = $d] pub $p: *const c_char,] $apply }
    };
    (@flagged $ty:ident [{ $p:ident $d:literal } $($more:tt)*] [$($out:tt)*] $apply:tt) => {
        ffi_request_options! { @fields [$($more)*] [$($out)*
            #[doc = $d]
            pub $p: $ty,
            #[doc = concat!("Presence flag for `", stringify!($p), "`; set to `1` to apply the value.")]
            pub [<has_ $p>]: i32,
        ] $apply }
    };
    (@fields [] [$($out:tt)*] [$({ $p:ident $t:ident })*]) => {
        thetadatadx::__paste! {
            /// Optional builder parameters for registry-driven endpoint requests over FFI.
            ///
            /// Fields use simple C-friendly sentinels:
            /// - integer/boolean/float fields: check the companion `has_*` flag
            /// - string pointers: null means unset
            ///
            /// `timeout_ms` is the cross-cutting per-call deadline. Set with the
            /// companion `has_timeout_ms = 1`; on expiry the call returns an error and the
            /// underlying gRPC stream is cancelled, leaving the client handle reusable.
            #[repr(C)]
            pub struct ThetaDataDxEndpointRequestOptions {
                $($out)*
                /// Per-call deadline in milliseconds; gated by `has_timeout_ms`.
                pub timeout_ms: u64,
                /// Presence flag for `timeout_ms`; set to `1` to apply the deadline.
                pub has_timeout_ms: i32,
            }

            fn apply_endpoint_request_options(
                args: &mut thetadatadx::EndpointArgs,
                options: *const ThetaDataDxEndpointRequestOptions,
            ) -> Result<(), String> {
                if options.is_null() {
                    return Ok(());
                }

                // SAFETY: options is non-null (checked above) and the caller is required
                // to keep the ThetaDataDxEndpointRequestOptions alive for the duration of this call.
                let options = unsafe { &*options };
                $(ffi_request_options!(@apply $t $p [<has_ $p>] args options);)*
                if options.has_timeout_ms != 0 {
                    args.set_timeout_ms(options.timeout_ms);
                }
                Ok(())
            }
        }
    };
    (@apply Bool $p:ident $has:ident $args:ident $options:ident) => {
        if $options.$has != 0 {
            insert_bool_arg($args, stringify!($p), $options.$p)?;
        }
    };
    (@apply Int $p:ident $has:ident $args:ident $options:ident) => {
        if $options.$has != 0 {
            insert_int_arg($args, stringify!($p), $options.$p);
        }
    };
    (@apply Float $p:ident $has:ident $args:ident $options:ident) => {
        if $options.$has != 0 {
            insert_float_arg($args, stringify!($p), $options.$p);
        }
    };
    (@apply $t:ident $p:ident $has:ident $args:ident $options:ident) => {
        insert_optional_str_arg($args, stringify!($p), $options.$p)?;
    };
}

thetadatadx::__request_options! { ffi_request_options {{}} }

macro_rules! ffi_endpoint {
    ({} [$($attr:ident)*] $shape:ident $name:ident $ret:ident $(($col:tt))? [$desc:literal $($doc:literal)*] params {
        all $all:tt req [$($req:tt)*] opt [$({ $op:ident $($o:tt)* })*] $($rest:tt)*
    }) => {
        // Fails to compile when the endpoint gains an optional parameter the
        // options struct does not carry.
        const _: () = { $(let _ = core::mem::offset_of!(ThetaDataDxEndpointRequestOptions, $op);)* };
        ffi_endpoint! { @req [$($req)*] [] { $shape $name $ret $desc } }
    };
    // Required C arguments: a symbol list arrives as a pointer and a length.
    // The length parameter is pasted once and carried, so the signature and
    // the body name the same binding.
    (@req [{ $p:ident $a:ident req Symbols $v:tt $d:literal } $($more:tt)*] [$($out:tt)*] $k:tt) => {
        thetadatadx::__paste! {
            ffi_endpoint! { @req [$($more)*] [$($out)* { list $p $a [*const *const c_char] [[<$a _len>]] }] $k }
        }
    };
    (@req [{ $p:ident $a:ident req $t:ident $v:tt $d:literal } $($more:tt)*] [$($out:tt)*] $k:tt) => {
        ffi_endpoint! { @req [$($more)*] [$($out)* { one $p $a [*const c_char] [] }] $k }
    };
    (@req [] $args:tt { $($k:tt)* }) => {
        ffi_endpoint! { @emit $args $($k)* }
    };
    // The options bag is applied first, so a positional argument always wins
    // over a bag field of the same name.
    (@args [$({ $kind:ident $p:ident $a:ident $ty:tt [$($len:ident)?] })*] $args:ident $options:ident $empty:ident) => {
        let mut $args = thetadatadx::EndpointArgs::new();
        if let Err(message) = apply_endpoint_request_options(&mut $args, $options) {
            crate::error::set_error_with_code(&message, crate::error::THETADATADX_ERR_INVALID_PARAMETER);
            return $empty;
        }
        $(ffi_endpoint!(@insert $kind $p $a [$($len)?] $args $empty);)*
    };
    (@insert list $p:ident $a:ident [$len:ident] $args:ident $empty:ident) => {
        let $a = require_symbol_array!($a, $len, $empty);
        $args.insert(
            stringify!($p).to_string(),
            thetadatadx::EndpointArgValue::Str($a.join(",")),
        );
    };
    (@insert one $p:ident $a:ident [] $args:ident $empty:ident) => {
        let $a = require_cstr!($a, $empty);
        $args.insert(
            stringify!($p).to_string(),
            thetadatadx::EndpointArgValue::Str($a.to_string()),
        );
    };
    (@unexpected $name:ident $other:ident $empty:ident) => {{
        set_error(&format!(
            concat!("internal error: unexpected endpoint output for ", stringify!($name), ": {:?}"),
            $other
        ));
        $empty
    }};

    // A list returns strings.
    (@emit [$({ $kind:ident $p:ident $a:ident [$($ty:tt)*] [$($len:ident)?] })*] list $name:ident $ret:ident $desc:literal) => {
        thetadatadx::__paste! {
            #[doc = $desc]
            #[doc = ""]
            #[doc = "Accepts optional builder parameters."]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<thetadatadx_ $name _with_options>](
                client: *const ThetaDataDxMarketDataClient,
                $($a: $($ty)*, $($len: usize,)?)*
                options: *const ThetaDataDxEndpointRequestOptions,
            ) -> ThetaDataDxStringArray {
                ffi_boundary!(ThetaDataDxStringArray::EMPTY, {
                    let empty = ThetaDataDxStringArray::EMPTY;
                    let client = require_client!(client, empty);

                    ffi_endpoint!(@args [$({ $kind $p $a [$($ty)*] [$($len)?] })*] args options empty);

                    match invoke_endpoint_blocking(&client.inner, stringify!($name), &args) {
                        Ok(thetadatadx::EndpointOutput::StringList(values)) => match ThetaDataDxStringArray::from_vec(values) {
                            Ok(arr) => arr,
                            Err(e) => {
                                set_error(&format!("interior NUL in server string: {e}"));
                                empty
                            }
                        },
                        Ok(other) => ffi_endpoint!(@unexpected $name other empty),
                        Err(error) => {
                            set_error_from(&thetadatadx::Error::from(error));
                            empty
                        }
                    }
                })
            }
        }
    };

    // Ticks, and the chunk stream of a chunked endpoint.
    (@emit $args:tt $shape:ident $name:ident $item:ident $desc:literal) => {
        ffi_endpoint! { @buffered $args $name $item $desc }
        ffi_endpoint! { @stream $shape $item $args $name $desc }
    };
    (@buffered [$({ $kind:ident $p:ident $a:ident [$($ty:tt)*] [$($len:ident)?] })*] $name:ident $item:ident $desc:literal) => {
        thetadatadx::__paste! {
            #[doc = $desc]
            #[doc = ""]
            #[doc = "Accepts optional builder parameters."]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<thetadatadx_ $name _with_options>](
                client: *const ThetaDataDxMarketDataClient,
                $($a: $($ty)*, $($len: usize,)?)*
                options: *const ThetaDataDxEndpointRequestOptions,
                out_presence: *mut ThetaDataDxColumnPresence,
            ) -> [<ThetaDataDx $item Array>] {
                ffi_boundary!([<ThetaDataDx $item Array>]::EMPTY, {
                    let empty = [<ThetaDataDx $item Array>]::EMPTY;
                    if !out_presence.is_null() {
                        // SAFETY: caller-supplied writable slot (checked non-null).
                        unsafe { *out_presence = ThetaDataDxColumnPresence::EMPTY };
                    }
                    let client = require_client!(client, empty);

                    ffi_endpoint!(@args [$({ $kind $p $a [$($ty)*] [$($len)?] })*] args options empty);

                    match invoke_endpoint_blocking(&client.inner, stringify!($name), &args) {
                        Ok(thetadatadx::EndpointOutput::[<$item s>](values)) => {
                            let columns = values.columns().clone();
                            match [<ThetaDataDx $item Array>]::from_vec(values.into_vec()) {
                                Ok(arr) => {
                                    if !out_presence.is_null() {
                                        // SAFETY: caller-supplied writable slot (checked non-null).
                                        unsafe { *out_presence = ThetaDataDxColumnPresence::from_presence(&columns) };
                                    }
                                    arr
                                }
                                Err(e) => {
                                    set_error(&format!("interior NUL in server string: {e}"));
                                    empty
                                }
                            }
                        }
                        Ok(other) => ffi_endpoint!(@unexpected $name other empty),
                        Err(error) => {
                            set_error_from(&thetadatadx::Error::from(error));
                            empty
                        }
                    }
                })
            }
        }
    };
    (@stream buffered $($rest:tt)*) => {};
    // A contract row owns its symbol string, so it has no flat chunk layout.
    (@stream chunked OptionContract $($rest:tt)*) => {};
    (@stream chunked $item:ident [$({ $kind:ident $p:ident $a:ident [$($ty:tt)*] [$($len:ident)?] })*] $name:ident $desc:literal) => {
        thetadatadx::__paste! {
            #[doc = $desc]
            #[doc = ""]
            #[doc = "Stream the result chunk-by-chunk through `callback` without"]
            #[doc = "materialising the full response. `callback(rows, len, columns, ctx)`"]
            #[doc = "is invoked once per server chunk with a pointer to a contiguous run"]
            #[doc = "of the endpoint's tick struct (the same layout the buffered"]
            #[doc = "`_with_options` array returns) and the chunk's column presence,"]
            #[doc = "which carries the root each row belongs to when the response has one;"]
            #[doc = "the chunk is freed before the next is fetched, so peak memory tracks a"]
            #[doc = "single chunk. Accepts the same optional builder parameters as the"]
            #[doc = "buffered form. A non-zero return from `callback` stops the stream."]
            #[doc = "Returns 0 once the stream ends or is stopped, -1 on error (inspect"]
            #[doc = "`thetadatadx_last_error`)."]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<thetadatadx_ $name _stream>](
                client: *const ThetaDataDxMarketDataClient,
                $($a: $($ty)*, $($len: usize,)?)*
                callback: Option<ThetaDataDxTickChunkCallback>,
                ctx: *mut c_void,
                options: *const ThetaDataDxEndpointRequestOptions,
            ) -> i32 {
                ffi_boundary!(-1, {
                    let empty = -1;
                    let client = require_client!(client, empty);

                    ffi_endpoint!(@args [$({ $kind $p $a [$($ty)*] [$($len)?] })*] args options empty);

                    let Some(callback) = callback else {
                        set_error("callback function pointer is null");
                        return empty;
                    };
                    let sink = TickChunkSink { callback, ctx };
                    match invoke_endpoint_stream_blocking(&client.inner, stringify!($name), &args, sink) {
                        Ok(()) => 0,
                        Err(error) => {
                            set_error_from(&thetadatadx::Error::from(error));
                            empty
                        }
                    }
                })
            }
        }
    };
}

macro_rules! ffi_market_data {
    ($( $(#[$attr:ident])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident,
        rest: $rest:literal,
        returns: $ret:ident $(($col:literal))?,
        params: [$($key:tt)*],
        doc: [$($doc:literal),* $(,)?],
    } )*) => {
        $( thetadatadx::__resolve! { [$($key)*] => ffi_endpoint { {} [$($attr)*] $shape $name $ret $(($col))? [$($doc)*] } } )*
    };
}

thetadatadx::__endpoints! { ffi_market_data {} }
