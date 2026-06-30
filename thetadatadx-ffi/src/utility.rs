//! Standalone utilities: condition / exchange / sequence lookups,
//! calendar-status helper, and the feature-gated panic-test entry points.
//!
//! Split verbatim from `lib.rs`; the exported C ABI is unchanged.

use std::os::raw::c_char;

use crate::streaming::ThetaDataDxContract;

// ═══════════════════════════════════════════════════════════════════════
//  Condition / exchange / sequence helper accessors
//
//  Cross-language utility parity. The lookup tables live in
//  `thetadatadx::{conditions, exchange, sequences}`. The C ABI wraps them as
//  string-returning entry points (returning `'static` UTF-8 NUL-terminated
//  C strings — the underlying tables are `&'static str`, so the caller
//  MUST NOT free the returned pointer) plus a couple of `bool`-returning
//  predicates and integer accessors for trade-sequence math.
// ═══════════════════════════════════════════════════════════════════════

// every `thetadatadx_condition_*` / `thetadatadx_exchange_*` / `thetadatadx_quote_condition_*`
// entry point wraps its body in `ffi_boundary!` so a panic in the
// condition / exchange lookup tables (debug-build invariant trips,
// etc.) or in `static_cstr` (cache mutex contention, allocator OOM) cannot abort
// the host process. The wrappers return the documented "unknown"
// sentinel:
//   - `*const c_char` returners → `std::ptr::null()` (caller already
//     handles NUL on unknown codes — every binding's lookup contract
//     is "NULL or unknown sentinel = no data").
//   - `bool` returners → `false` (the predicate convention is "false
//     by default; only true when the table positively asserts").

/// Look up the human-readable trade condition name for `code`.
///
/// Returns a NUL-terminated `'static` UTF-8 C string. The pointer is
/// owned by the library and MUST NOT be freed. Returns the literal
/// `"UNKNOWN"` for codes outside the table; returns `NULL` if the
/// boundary catches a panic — surfaced through `thetadatadx_last_error()`.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::condition_name(code))
    })
}

/// Look up the human-readable trade condition description for `code`.
///
/// Returns a NUL-terminated `'static` UTF-8 C string (empty string for
/// unknown codes). The pointer is owned by the library and MUST NOT be
/// freed. Returns `NULL` if the boundary catches a panic.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_description(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::condition_description(code))
    })
}

/// True if the trade condition code represents a cancellation.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_is_cancel(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_cancel(code) })
}

/// True if the trade condition code updates the volume bar.
#[no_mangle]
pub extern "C" fn thetadatadx_condition_updates_volume(code: i32) -> bool {
    ffi_boundary!(false, {
        thetadatadx::utils::conditions::updates_volume(code)
    })
}

/// Look up the human-readable quote condition name for `code`.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::quote_condition_name(code))
    })
}

/// Look up the human-readable quote condition description for `code`.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_description(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::conditions::quote_condition_description(
            code,
        ))
    })
}

/// True if the quote condition is firm (binding).
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_is_firm(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_firm(code) })
}

/// True if the quote condition indicates a trading halt.
#[no_mangle]
pub extern "C" fn thetadatadx_quote_condition_is_halted(code: i32) -> bool {
    ffi_boundary!(false, { thetadatadx::utils::conditions::is_halted(code) })
}

/// Look up the human-readable exchange name for a numeric code.
#[no_mangle]
pub extern "C" fn thetadatadx_exchange_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::exchange::exchange_name(code))
    })
}

/// Look up the MIC-like symbol for a numeric exchange code.
#[no_mangle]
pub extern "C" fn thetadatadx_exchange_symbol(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(thetadatadx::utils::exchange::exchange_symbol(code))
    })
}

/// Convert a signed wire-encoded trade-sequence value to its unsigned
/// monotonic form. Mirrors `thetadatadx::utils::sequences::signed_to_unsigned`.
///
/// `signed` must lie in the i32 wire range
/// (`-2_147_483_648 ..= 2_147_483_647`): the upstream terminal encodes
/// trade sequences as i32, so a value outside that domain is not a wire
/// sequence and would otherwise be silently reinterpreted into a
/// look-correct-but-wrong id. Writes the converted value to `out` and
/// returns `0` on success; returns `-1` and sets `thetadatadx_last_error` /
/// `thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER` when `signed` is
/// outside the wire range or `out` is null, matching the typed class the
/// Python / TypeScript bindings raise for the same input.
///
/// # Safety
/// `out` must be a valid, non-null pointer to a `uint64_t` the caller
/// keeps alive for the call duration.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_sequence_signed_to_unsigned(
    signed: i64,
    out: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_sequence_signed_to_unsigned: out pointer is null",
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        if !(thetadatadx::utils::sequences::SEQUENCE_MIN
            ..=thetadatadx::utils::sequences::SEQUENCE_MAX)
            .contains(&signed)
        {
            crate::error::set_error_with_code(
                &format!(
                    "thetadatadx_sequence_signed_to_unsigned: {signed} is outside the i32 wire range (-2_147_483_648 ..= 2_147_483_647)"
                ),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        // SAFETY: `out` is non-null per the guard above and the FFI
        // contract pins the storage for the call duration.
        unsafe {
            *out = thetadatadx::utils::sequences::signed_to_unsigned(signed);
        }
        0
    })
}

/// Convert an unsigned monotonic trade-sequence value back to its
/// signed wire encoding. Mirrors `thetadatadx::utils::sequences::unsigned_to_signed`.
///
/// `unsigned` must lie in the unsigned wire range (`0 ..= 2^32 - 1`):
/// the monotonic sequence id is never wider than one i32 cycle, so a
/// value above that domain is not a wire sequence and would otherwise be
/// silently reinterpreted. Writes the converted value to `out` and
/// returns `0` on success; returns `-1` and sets `thetadatadx_last_error` /
/// `thetadatadx_last_error_code = THETADATADX_ERR_INVALID_PARAMETER` when `unsigned` is
/// above the wire range or `out` is null, matching the typed class the
/// Python / TypeScript bindings raise for the same input.
///
/// # Safety
/// `out` must be a valid, non-null pointer to an `int64_t` the caller
/// keeps alive for the call duration.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_sequence_unsigned_to_signed(
    unsigned: u64,
    out: *mut i64,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            crate::error::set_error_with_code(
                "thetadatadx_sequence_unsigned_to_signed: out pointer is null",
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        if unsigned > u64::from(u32::MAX) {
            crate::error::set_error_with_code(
                &format!(
                    "thetadatadx_sequence_unsigned_to_signed: {unsigned} is above the unsigned wire range (0 ..= 2^32 - 1)"
                ),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            return -1;
        }
        // SAFETY: `out` is non-null per the guard above and the FFI
        // contract pins the storage for the call duration.
        unsafe {
            *out = thetadatadx::utils::sequences::unsigned_to_signed(unsigned);
        }
        0
    })
}

/// Look up the vendor vocabulary text for a `ThetaDataDxCalendarDay.status`
/// code (`0` -> `"open"`, `1` -> `"early_close"`, `2` -> `"full_close"`,
/// `3` -> `"weekend"`).
///
/// Returns a NUL-terminated `'static` UTF-8 C string. The pointer is
/// owned by the library and MUST NOT be freed. Returns the literal
/// `"UNKNOWN"` for codes outside the table; returns `NULL` if the
/// boundary catches a panic — surfaced through `thetadatadx_last_error()`.
#[no_mangle]
pub extern "C" fn thetadatadx_calendar_status_name(code: i32) -> *const c_char {
    ffi_boundary!(std::ptr::null(), {
        static_cstr(
            thetadatadx::CalendarStatus::from_code(code)
                .map_or("UNKNOWN", thetadatadx::CalendarStatus::as_str),
        )
    })
}

/// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day into
/// Unix epoch milliseconds (UTC, DST-aware). Mirrors the
/// `*_timestamp_ms()` accessors the Rust / Python row surfaces expose,
/// usable with any `(date, *_ms_of_day)` pair on the tick structs.
///
/// Returns `-1` when `date` is not a valid Gregorian `YYYYMMDD`
/// (including the `0` absent fill) or `ms_of_day` is outside
/// `0..86_400_000` — `-1` is unreachable for real market data (it
/// denotes 1969-12-31T23:59:59.999Z).
#[no_mangle]
pub extern "C" fn thetadatadx_timestamp_ms(date: i32, ms_of_day: i32) -> i64 {
    ffi_boundary!(-1, {
        thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day).unwrap_or(-1)
    })
}

/// Read the option strike of a streaming `ThetaDataDxContract` in dollars, folding
/// the `has_strike` presence flag into the return value. Mirrors the C++
/// `thetadatadx::strike(const ThetaDataDxContract&)` accessor and the Python / TypeScript
/// `contract.strike` surface, which return an absent value for non-option
/// contracts rather than a bare `0.0` the caller must special-case.
///
/// `contract.strike` already carries dollars, so this only surfaces the
/// presence flag a plain field read would otherwise drop: writes the
/// dollar value to `out_dollars` and returns `true` when the contract is
/// an option, leaves `out_dollars` untouched and returns `false`
/// otherwise (non-option, or a null contract / output pointer).
///
/// # Safety
/// `contract` must be a valid `ThetaDataDxContract` pointer (e.g. the
/// `event.<variant>.contract` field of a `ThetaDataDxStreamEvent`). `out_dollars`
/// must be a valid, writable `double` pointer.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_contract_strike_dollars(
    contract: *const ThetaDataDxContract,
    out_dollars: *mut f64,
) -> bool {
    ffi_boundary!(false, {
        if contract.is_null() || out_dollars.is_null() {
            return false;
        }
        // SAFETY: both pointers null-checked above; the caller pins the
        // contract and output storage for the call duration.
        let contract = unsafe { &*contract };
        if !contract.has_strike {
            return false;
        }
        // SAFETY: out_dollars null-checked above; the caller pins the
        // output storage for the call duration.
        unsafe {
            *out_dollars = contract.strike;
        }
        true
    })
}

/// Convert a `&'static str` from the lookup tables into a stable
/// `*const c_char` for FFI. The lookup tables are compile-time arrays
/// of NUL-free `&'static str`; we register one `CString` per distinct
/// string in a process-lifetime `OnceLock<Mutex<HashMap<...>>>` and
/// return the cached pointer so the C side can hold it indefinitely.
///
/// poison-tolerant via `PoisonError::into_inner` rather than
/// `.expect(...)`. A panic in a previous holder of this cache mutex
/// (e.g. an OOM during `Box::leak`) leaves the map structurally
/// valid — we have no transient half-mutated state because every
/// insertion is `guard.insert(k, v)` after a successful allocation,
/// not a partial update. Recovering the inner map rather than
/// panicking again keeps every `thetadatadx_condition_*` / `thetadatadx_exchange_*`
/// /`thetadatadx_quote_condition_*` lookup non-aborting, matching the
/// `ffi_boundary!` contract on every other FFI entry point.
fn static_cstr(s: &'static str) -> *const c_char {
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::sync::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<HashMap<&'static str, &'static CString>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    let cstr_ref: &'static CString = match guard.get(&s) {
        Some(existing) => existing,
        None => {
            // Tables are compile-time `&'static str` literals known to
            // be NUL-free; CString::new only fails on interior NULs.
            let owned =
                CString::new(s).expect("lookup-table strings must not contain interior NULs");
            // Leak so the pointer is `'static` for the caller. There is
            // a finite, small number of distinct entries (≤ a few hundred
            // across all tables), so the leak is bounded.
            let leaked: &'static CString = Box::leak(Box::new(owned));
            guard.insert(s, leaked);
            leaked
        }
    };
    cstr_ref.as_ptr()
}

// ═══════════════════════════════════════════════════════════════════════
//  Test-only panic entry points (feature `testing-panic-boundary`)
//
//  These exist purely so the integration test at
//  `thetadatadx-ffi/tests/panic_boundary.rs` can prove that panics inside an
//  `extern "C"` body:
//    1. do NOT abort the process (the test binary would crash),
//    2. return the declared default (`-1` here, matching the
//       existing `i32` status-code convention),
//    3. make the panic payload retrievable via `thetadatadx_last_error()`.
//
//  The symbols are only compiled in when the feature is enabled, so
//  the shared library shipped to C++ / Python consumers never
//  carries a "panic-on-demand" entry point.
// ═══════════════════════════════════════════════════════════════════════

/// Deliberately panic with a `&'static str` payload. Returns -1 via the
/// boundary's default handler. The panic message becomes part of the
/// `thetadatadx_last_error()` string so the caller can verify the downcast path
/// that handles `&'static str` payloads works end to end.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn thetadatadx_test_panic_str() -> i32 {
    ffi_boundary!(-1, {
        panic!("intentional test panic via &'static str");
    })
}

/// Deliberately panic with a heap-allocated `String` payload. Returns -1
/// via the boundary's default handler. Separate from the `&'static str`
/// variant so the test suite can exercise both `downcast_ref::<&'static
/// str>` and `downcast_ref::<String>` branches of the macro.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn thetadatadx_test_panic_string() -> i32 {
    ffi_boundary!(-1, {
        panic!("{}", String::from("intentional test panic via String"));
    })
}

#[cfg(test)]
mod sequence_tests {
    //! Wire-range validation for the trade-sequence converters. Out-of
    //! -wire-range inputs must be rejected with the typed
    //! invalid-parameter class rather than silently reinterpreted — the
    //! cross-binding contract the Python `ValueError` / TypeScript
    //! `InvalidParameterError` already honour.

    #[test]
    fn signed_to_unsigned_round_trips_in_wire_range() {
        for signed in [i64::from(i32::MIN), -1, 0, 1, i64::from(i32::MAX)] {
            let mut out: u64 = 0;
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(signed, &mut out) };
            assert_eq!(rc, 0, "in-range input {signed} accepted");
            assert_eq!(
                out,
                thetadatadx::utils::sequences::signed_to_unsigned(signed)
            );
        }
    }

    #[test]
    fn signed_to_unsigned_rejects_out_of_wire_range() {
        for signed in [i64::from(i32::MAX) + 1, i64::from(i32::MIN) - 1] {
            let mut out: u64 = 0;
            crate::error::thetadatadx_clear_error();
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(signed, &mut out) };
            assert_eq!(rc, -1, "out-of-range input {signed} rejected");
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER
            );
        }
    }

    #[test]
    fn unsigned_to_signed_round_trips_in_wire_range() {
        for unsigned in [0u64, 1, u64::from(i32::MAX as u32), u64::from(u32::MAX)] {
            let mut out: i64 = 0;
            // SAFETY: `out` points at a live stack slot for the call.
            let rc = unsafe { super::thetadatadx_sequence_unsigned_to_signed(unsigned, &mut out) };
            assert_eq!(rc, 0, "in-range input {unsigned} accepted");
            assert_eq!(
                out,
                thetadatadx::utils::sequences::unsigned_to_signed(unsigned)
            );
        }
    }

    #[test]
    fn unsigned_to_signed_rejects_above_wire_range() {
        // 2^32 is the first value past the unsigned wire range.
        let mut out: i64 = 0;
        crate::error::thetadatadx_clear_error();
        // SAFETY: `out` points at a live stack slot for the call.
        let rc = unsafe {
            super::thetadatadx_sequence_unsigned_to_signed(u64::from(u32::MAX) + 1, &mut out)
        };
        assert_eq!(rc, -1, "2^32 rejected as above the wire range");
        assert_eq!(
            crate::error::thetadatadx_last_error_code(),
            crate::error::THETADATADX_ERR_INVALID_PARAMETER
        );
    }

    #[test]
    fn null_out_pointer_rejected() {
        crate::error::thetadatadx_clear_error();
        // SAFETY: deliberately passing null to exercise the guard.
        let rc = unsafe { super::thetadatadx_sequence_signed_to_unsigned(0, std::ptr::null_mut()) };
        assert_eq!(rc, -1);
        assert_eq!(
            crate::error::thetadatadx_last_error_code(),
            crate::error::THETADATADX_ERR_INVALID_PARAMETER
        );
    }
}
