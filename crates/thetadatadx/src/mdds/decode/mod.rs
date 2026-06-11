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

// `parse_calendar_days_v3` and `parse_option_contracts_v3` are used by the
// generated MDDS endpoint macros (`mdds_parsed_endpoints_generated.rs`) — keep
// them always-compiled. The `CALENDAR_STATUS_*` constants are used only by
// workspace bindings (`sdks/python`); gate those on `__internal`.
pub use dual_type_columns::{parse_calendar_days_v3, parse_option_contracts_v3};
#[cfg(feature = "__internal")]
pub use dual_type_columns::{
    CALENDAR_STATUS_EARLY_CLOSE, CALENDAR_STATUS_FULL_CLOSE, CALENDAR_STATUS_OPEN,
    CALENDAR_STATUS_UNKNOWN, CALENDAR_STATUS_WEEKEND,
};
pub use error::DecodeError;
// `extract_number_column` and `extract_price_column` are used by workspace
// bindings only; gate them under `__internal`. `extract_text_column` may be
// used in the `cell` generated parsers — keep it always-available.
pub use extract::extract_text_column;
#[cfg(feature = "__internal")]
pub use extract::{extract_number_column, extract_price_column};
// `decode_data_table` and `decompress_response` (non-`_with_max` variants)
// plus `decompress_response_with_max` are only used by workspace
// bindings and tests; gate them under `__internal`.
// `decode_data_table_with_max` is the production per-chunk decode used
// by `mdds/stream.rs` — keep it always-available.
pub use transport::decode_data_table_with_max;
#[cfg(feature = "__internal")]
pub use transport::{decode_data_table, decompress_response, decompress_response_with_max};

// Re-export the generated parser functions at this module's top level.
// `cell.rs` handles the split: `__internal` builds get `pub use cell::*`
// (all generated parsers); default builds get the explicit subset that the
// generated MDDS endpoint macros call directly.
pub use cell::*;

// `observed_name` is `pub(crate)` and intentionally not part of the public
// surface; it stays accessible as `crate::decode::observed_name` via this
// re-export so the generated parser code (emitted by `build.rs` from the
// templates in `build_support/ticks/templates/parser/`) still resolves it.
pub(crate) use error::observed_name;

#[cfg(test)]
mod tests;
