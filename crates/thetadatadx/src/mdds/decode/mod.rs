//! `ResponseData` → `DataTable` → tick decoders.
//!
//! Split of the original `crates/thetadatadx/src/decode.rs` god-file:
//!
//! | Submodule    | Concern                                                       |
//! |--------------|---------------------------------------------------------------|
//! | [`error`]    | [`DecodeError`] enum + `observed_name` diagnostic helper      |
//! | [`headers`]  | `HEADER_ALIASES` v3 ↔ schema map + `find_header` lookup       |
//! | [`transport`]| `decompress_response` / `decode_data_table` zstd path         |
//! | [`extract`]  | `extract_{number,text,price}_column` column projections       |
//! | [`cell`]     | Per-cell strict decoders (`row_*`) + generated parser surface |
//! | [`dual_type_columns`] | Hand-written parsers for columns that arrive as either `Number` or `Text` on the v3 wire (`parse_option_contracts_v3`, …) |
//!
//! Public API surface is preserved at `thetadatadx::decode::*` via the
//! crate-root re-export of this module. Eastern-time / DST primitives
//! previously living here have moved to [`tdbe::time`] and are reused by
//! the FPSS latency path.

pub mod cell;
pub mod dual_type_columns;
pub mod error;
pub mod extract;
pub mod headers;
pub mod transport;

pub use dual_type_columns::{
    parse_calendar_days_v3, parse_option_contracts_v3, CALENDAR_STATUS_EARLY_CLOSE,
    CALENDAR_STATUS_FULL_CLOSE, CALENDAR_STATUS_OPEN, CALENDAR_STATUS_UNKNOWN,
    CALENDAR_STATUS_WEEKEND,
};
pub use error::DecodeError;
pub use extract::{extract_number_column, extract_price_column, extract_text_column};
pub use transport::{
    decode_data_table, decode_data_table_with_max, decompress_response,
    decompress_response_with_max,
};

// Re-export the macro-generated parser functions (`parse_trade_ticks`,
// `parse_eod_ticks`, etc.) at this module's top level so external consumers
// (sdks/python, benches) can keep using `thetadatadx::decode::parse_*`.
pub use cell::*;

// `observed_name` is `pub(crate)` and intentionally not part of the public
// surface; it stays accessible as `crate::decode::observed_name` via this
// re-export so the generated parser code (emitted by `build.rs` from the
// templates in `build_support/ticks/templates/parser/`) still resolves it.
pub(crate) use error::observed_name;

#[cfg(test)]
mod tests;
