//! Decode-fed projected Arrow IPC terminal (the C++ `<tick>_to_arrow_ipc_projected`
//! path) over the C ABI.
//!
//! The all-present `thetadatadx_<tick>_ticks_to_arrow_ipc` terminal serialises
//! every column a hand-built row vector could carry. The decode-fed pair added
//! here mirrors Python's `<TickName>List.to_arrow()`: build a response's
//! wire-column presence from its header names
//! (`thetadatadx_<tick>_present_columns`), then serialise ONLY those columns
//! (`thetadatadx_<tick>_ticks_to_arrow_ipc_projected`). These are the exact C
//! symbols the generated C++ `tick_arrow_ipc.hpp.inc` wrappers bind, so this
//! test is the offline proof the C++/FFI projected export projects.
//!
//! A `stock_history_trade` response omits the four trade-flag columns
//! (`condition_flags` / `price_flags` / `volume_type` / `records_back`) and the
//! contract-identity trio (`expiration` / `strike` / `right`); the projected
//! stream must omit them too, while the all-present terminal keeps them.

use std::ffi::{c_char, CString};

use arrow_ipc::reader::StreamReader;

use thetadatadx_ffi::{
    thetadatadx_arrow_bytes_free, thetadatadx_column_presence_free,
    thetadatadx_trade_ticks_present_columns, thetadatadx_trade_ticks_to_arrow_ipc,
    thetadatadx_trade_ticks_to_arrow_ipc_projected, ThetaDataDxArrowBytes,
    ThetaDataDxColumnPresence,
};

/// The columns a `stock_history_trade` wire response carries: the ten trade
/// execution columns plus `date`. No flag columns, no contract-id trio — the
/// equity endpoint omits both (see `thetadatadx-rs/src/columns.rs`).
const STOCK_TRADE_HEADERS: &[&str] = &[
    "ms_of_day",
    "sequence",
    "ext_condition1",
    "ext_condition2",
    "ext_condition3",
    "ext_condition4",
    "condition",
    "size",
    "exchange",
    "price",
    "date",
];

fn sample_rows() -> Vec<thetadatadx::TradeTick> {
    // Two rows; the flag / contract-id fields carry non-seed values so a
    // projection bug (emitting them) would surface as real columns, not
    // all-zero ones a reader might overlook.
    vec![
        thetadatadx::TradeTick {
            ms_of_day: 34_200_000,
            sequence: 1,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 5,
            price: 12.5,
            condition_flags: 7,
            price_flags: 3,
            volume_type: 1,
            records_back: 2,
            date: 20_260_115,
            expiration: 20_260_117,
            strike: 100.0,
            right: 'C',
        },
        thetadatadx::TradeTick {
            ms_of_day: 34_200_100,
            sequence: 2,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 200,
            exchange: 5,
            price: 12.75,
            condition_flags: 7,
            price_flags: 3,
            volume_type: 1,
            records_back: 1,
            date: 20_260_115,
            expiration: 20_260_117,
            strike: 100.0,
            right: 'C',
        },
    ]
}

/// Decode an Arrow IPC byte buffer to its column names, in schema order.
fn ipc_columns(bytes: &ThetaDataDxArrowBytes) -> Vec<String> {
    assert!(
        !bytes.data.is_null(),
        "terminal returned the error sentinel"
    );
    // SAFETY: `data` / `len` describe the buffer the terminal leaked; valid
    // until we free it below.
    let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) };
    let reader = StreamReader::try_new(std::io::Cursor::new(slice), None)
        .expect("terminal must emit a valid Arrow IPC stream");
    reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect()
}

/// Decode an Arrow IPC byte buffer to its (column names, row count).
fn ipc_columns_and_rows(bytes: &ThetaDataDxArrowBytes) -> (Vec<String>, usize) {
    assert!(
        !bytes.data.is_null(),
        "terminal returned the error sentinel"
    );
    // SAFETY: `data` / `len` describe the buffer the terminal leaked.
    let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) };
    let mut reader = StreamReader::try_new(std::io::Cursor::new(slice), None)
        .expect("terminal must emit a valid Arrow IPC stream");
    let cols: Vec<String> = reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    // Sum row counts across batches (a zero-column stream still carries them).
    let rows = reader
        .by_ref()
        .map(|b| b.expect("valid batch").num_rows())
        .sum();
    (cols, rows)
}

/// Decode the first column of an Arrow IPC buffer as UTF-8 string values.
fn ipc_first_column_strings(bytes: &ThetaDataDxArrowBytes) -> Vec<String> {
    use arrow_array::{cast::AsArray, Array};
    assert!(
        !bytes.data.is_null(),
        "terminal returned the error sentinel"
    );
    // SAFETY: `data` / `len` describe the buffer the terminal leaked.
    let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) };
    let mut reader = StreamReader::try_new(std::io::Cursor::new(slice), None)
        .expect("terminal must emit a valid Arrow IPC stream");
    let batch = reader
        .next()
        .expect("stream carries a batch")
        .expect("valid batch");
    let col = batch.column(0).as_string::<i32>();
    (0..col.len()).map(|i| col.value(i).to_string()).collect()
}

/// The C++ `present_columns` wrapper hands the C ABI a `const char* const*`;
/// build that from Rust `&str` headers for the FFI call.
fn presence_from_headers(headers: &[&str]) -> ThetaDataDxColumnPresence {
    let owned: Vec<CString> = headers.iter().map(|h| CString::new(*h).unwrap()).collect();
    let ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    // SAFETY: `ptrs` points to `owned.len()` valid C strings, live for the call.
    unsafe { thetadatadx_trade_ticks_present_columns(ptrs.as_ptr(), ptrs.len()) }
}

#[test]
fn decode_fed_projected_export_omits_flags_and_contract_id() {
    let rows = sample_rows();
    let presence = presence_from_headers(STOCK_TRADE_HEADERS);

    // The C ABI takes the carrier by value — a bitwise copy — and does NOT
    // free it (the caller keeps ownership, exactly as the C++ RAII wrapper
    // passes `presence.raw()` and frees the original in its destructor).
    // Model that copy explicitly so the original is still ours to free.
    let carrier_copy = ThetaDataDxColumnPresence {
        names: presence.names,
        len: presence.len,
        symbols: presence.symbols,
        symbols_len: presence.symbols_len,
    };
    // SAFETY: `rows` is a live slice; `carrier_copy` aliases the still-owned
    // `presence` names, valid for the call. The terminal only reads it.
    let bytes = unsafe {
        thetadatadx_trade_ticks_to_arrow_ipc_projected(
            rows.as_ptr(),
            rows.len(),
            carrier_copy,
            std::ptr::null(),
        )
    };
    let cols = ipc_columns(&bytes);
    // SAFETY: `bytes` came from the terminal; freed exactly once.
    unsafe { thetadatadx_arrow_bytes_free(bytes) };
    // SAFETY: free the no-symbol/no-flags presence carrier once; the projected serializer borrowed (copied) it, so the original is still ours to free.
    unsafe { thetadatadx_column_presence_free(presence) };

    // The projected frame is exactly the wire's columns, in schema order.
    assert_eq!(
        cols,
        vec![
            "ms_of_day",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "date",
        ],
        "projected export must carry only the stock-trade wire columns"
    );
    for absent in [
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ] {
        assert!(
            !cols.contains(&absent.to_string()),
            "projected export leaked the wire-absent column `{absent}`"
        );
    }
}

#[test]
fn projected_export_broadcasts_symbol_as_leading_column() {
    // An option-trade response carries a constant `symbol` (root). Passing it
    // to the projected terminal must prepend a `symbol` Utf8 column, first in
    // schema order, valued on every row.
    let rows = sample_rows();
    let option_headers: &[&str] = &[
        "symbol",
        "expiration",
        "strike",
        "right",
        "ms_of_day",
        "sequence",
        "condition",
        "size",
        "exchange",
        "price",
    ];
    let presence = presence_from_headers(option_headers);
    let carrier_copy = ThetaDataDxColumnPresence {
        names: presence.names,
        len: presence.len,
        symbols: presence.symbols,
        symbols_len: presence.symbols_len,
    };
    let symbol = CString::new("SPY").unwrap();
    // SAFETY: `rows` is a live slice; `carrier_copy` aliases the still-owned
    // `presence`; `symbol` is a live C string for the call.
    let bytes = unsafe {
        thetadatadx_trade_ticks_to_arrow_ipc_projected(
            rows.as_ptr(),
            rows.len(),
            carrier_copy,
            symbol.as_ptr(),
        )
    };
    let cols = ipc_columns(&bytes);
    // SAFETY: `bytes` came from the terminal; freed exactly once.
    unsafe { thetadatadx_arrow_bytes_free(bytes) };
    // SAFETY: free the presence carrier that held the broadcast `symbol` once; the serializer copied it, so this frees the original, not the copy.
    unsafe { thetadatadx_column_presence_free(presence) };

    assert_eq!(
        cols.first().map(String::as_str),
        Some("symbol"),
        "symbol must be the leading projected column; got {cols:?}"
    );
    assert!(
        cols.contains(&"expiration".to_string()),
        "option projection keeps the contract-id trio; got {cols:?}"
    );
}

#[test]
fn projected_export_emits_per_row_symbols_in_row_order() {
    // A multi-symbol snapshot carries one `symbol` per row on the presence
    // carrier (not the constant broadcast). The projected terminal must emit a
    // leading `symbol` column valued row-for-row, in the carrier's order.
    let rows = sample_rows();
    let presence = presence_from_headers(STOCK_TRADE_HEADERS);
    // One C string per row, in row order — the shape the decode seam leaks via
    // `from_presence`. Kept owned here so the terminal only borrows them.
    let syms_owned: Vec<CString> = ["AAPL", "MSFT"]
        .iter()
        .map(|s| CString::new(*s).unwrap())
        .collect();
    let syms_ptrs: Vec<*const c_char> = syms_owned.iter().map(|c| c.as_ptr()).collect();
    let carrier = ThetaDataDxColumnPresence {
        names: presence.names,
        len: presence.len,
        symbols: syms_ptrs.as_ptr(),
        symbols_len: syms_ptrs.len(),
    };
    // SAFETY: `rows` is a live slice; `carrier` aliases the still-owned
    // `presence` names and the live `syms_ptrs`. The terminal only reads it.
    let bytes = unsafe {
        thetadatadx_trade_ticks_to_arrow_ipc_projected(
            rows.as_ptr(),
            rows.len(),
            carrier,
            std::ptr::null(),
        )
    };
    let cols = ipc_columns(&bytes);
    let syms = ipc_first_column_strings(&bytes);
    // SAFETY: `bytes` came from the terminal; freed exactly once.
    unsafe { thetadatadx_arrow_bytes_free(bytes) };
    // SAFETY: free the `presence` (its `names` array; `symbols` was null on it —
    // the per-row array is `syms_owned`, dropped by Rust). Freed exactly once.
    unsafe { thetadatadx_column_presence_free(presence) };

    assert_eq!(
        cols.first().map(String::as_str),
        Some("symbol"),
        "per-row symbols must be the leading projected column; got {cols:?}"
    );
    assert_eq!(
        syms,
        vec!["AAPL", "MSFT"],
        "per-row symbol column must follow row order",
    );
}

#[test]
fn hand_built_all_present_terminal_keeps_every_column() {
    // The same rows through the ORIGINAL all-present terminal — a hand-built
    // vector never touched a wire, so every column stays present. This is the
    // path users of the free function keep, unchanged.
    let rows = sample_rows();
    // SAFETY: `rows` is a live slice for the call.
    let bytes = unsafe { thetadatadx_trade_ticks_to_arrow_ipc(rows.as_ptr(), rows.len()) };
    let cols = ipc_columns(&bytes);
    // SAFETY: freed exactly once.
    unsafe { thetadatadx_arrow_bytes_free(bytes) };

    for present in [
        "ms_of_day",
        "price",
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ] {
        assert!(
            cols.contains(&present.to_string()),
            "all-present terminal dropped `{present}`"
        );
    }
}

#[test]
fn presence_carrier_reports_the_wire_columns() {
    // The presence producer names the public schema fields the wire carried,
    // in schema order — the same set the projected frame uses.
    let presence = presence_from_headers(STOCK_TRADE_HEADERS);
    assert_eq!(presence.len, STOCK_TRADE_HEADERS.len());
    // SAFETY: `names` / `len` describe the carrier the producer leaked.
    let names: Vec<String> = unsafe {
        std::slice::from_raw_parts(presence.names, presence.len)
            .iter()
            .map(|&p| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
            .collect()
    };
    // SAFETY: free exactly once.
    unsafe { thetadatadx_column_presence_free(presence) };
    assert_eq!(names, STOCK_TRADE_HEADERS);
}

#[test]
fn empty_presence_projects_to_zero_columns_with_row_count() {
    // A response whose wire headers resolve to zero schema columns yields an
    // empty `ColumnPresence`. The projected export must still succeed — a
    // 0-column stream carrying the row count — not error. Arrow's plain
    // `RecordBatch::try_new` cannot infer a row count from zero columns, so
    // the builder must pin it explicitly.
    let rows = sample_rows();
    // Header names that match no schema column -> empty presence.
    let presence = presence_from_headers(&["not_a_column", "also_missing"]);
    assert_eq!(
        presence.len, 0,
        "no header should resolve to a schema column"
    );
    let carrier_copy = ThetaDataDxColumnPresence {
        names: presence.names,
        len: presence.len,
        symbols: presence.symbols,
        symbols_len: presence.symbols_len,
    };
    // SAFETY: `rows` is a live slice; `carrier_copy` aliases the still-owned
    // empty carrier. The terminal only reads it.
    let bytes = unsafe {
        thetadatadx_trade_ticks_to_arrow_ipc_projected(
            rows.as_ptr(),
            rows.len(),
            carrier_copy,
            std::ptr::null(),
        )
    };
    let (cols, num_rows) = ipc_columns_and_rows(&bytes);
    // SAFETY: `bytes` came from the terminal; freed exactly once.
    unsafe { thetadatadx_arrow_bytes_free(bytes) };
    // SAFETY: free the empty-presence carrier once; the serializer copied it, so the original is freed here exactly once.
    unsafe { thetadatadx_column_presence_free(presence) };

    assert!(
        cols.is_empty(),
        "empty presence must project to zero columns"
    );
    assert_eq!(
        num_rows,
        rows.len(),
        "zero-column projected batch must still carry its row count"
    );
}
