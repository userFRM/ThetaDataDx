//! FIT/FIE codec for `ThetaData`'s FPSS streaming protocol.
//!
//! Mirrors the Java terminal implementation for byte-for-byte wire parity:
//! - `FITReader.java` — nibble-oriented variable-length integer decoder (FIT)
//! - `FIE.java` — string-to-nibble encoder for request building (FIE)
//!
//! FIT (Field-Indexed Tick) is a nibble-oriented (4-bit) compression format
//! used to encode integer fields in FPSS tick data. Each byte packs two nibbles
//! (high bits 7-4, low bits 3-0) that represent either decimal digits (0-9) or
//! control codes (field separator, row separator, end marker, negative sign).
//!
//! The stream carries delta-compressed rows: the first row contains absolute
//! values, and subsequent rows contain deltas that are added to the previous
//! row's values.

pub mod fie;
pub mod fit;

pub use fie::string_to_fie_line;
pub use fit::decode_fit_buffer_bulk;
pub use fit::FitReader;
pub use fit::FitRows;
