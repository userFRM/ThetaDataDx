//! FIT delta-decompression state per `(msg_type, contract_id)`.
//!
//! FPSS ticks arrive FIT-encoded: the first tick for a given stream is
//! absolute, subsequent ticks are deltas against the previous absolute row.
//! This module owns the running absolute state plus the reusable scratch
//! buffer used by [`DeltaState::decode_tick`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::tdbe::codec::fit::{apply_deltas, FitReader};

/// Number of FIT fields per tick type (excluding the `contract_id` which is the
/// first FIT field). The FIT decoder returns `n_fields` total, where field [0]
/// is the `contract_id` and fields [1..] are the tick data.
pub(super) const QUOTE_FIELDS: usize = 11;
/// The FPSS stream trade is the 8-field layout. (The 16-field "extended"
/// trade is an MDDS gRPC shape on a different protocol and never reaches
/// this stream decoder.)
pub(super) const TRADE_FIELDS: usize = 8;
pub(super) const OI_FIELDS: usize = 3;
pub(super) const OHLCVC_FIELDS: usize = 9;

/// Largest data-field count any FPSS stream tick shape declares (the quote,
/// at 11 fields). Tick data is stored in stack arrays of this size so the
/// decode hot path is fully heap-free and the `prev` map can carry the
/// previous absolute row inline rather than behind a `Vec`.
pub(super) const MAX_DATA_FIELDS: usize = {
    // `Ord::max` is not const-stable; fold over the tick widths so this stays
    // the true maximum if any field count changes.
    let mut m = QUOTE_FIELDS;
    if TRADE_FIELDS > m {
        m = TRADE_FIELDS;
    }
    if OHLCVC_FIELDS > m {
        m = OHLCVC_FIELDS;
    }
    if OI_FIELDS > m {
        m = OI_FIELDS;
    }
    m
};

/// One row of absolute tick data. Sized to the widest tick shape so every
/// caller can pass the same stack buffer regardless of message type. Slots
/// past the active field count are zero-filled and ignored.
pub(super) type TickFields = [i32; MAX_DATA_FIELDS];

/// Per-contract, per-message-type delta decompression state.
///
/// FIT uses delta compression: the first tick for a contract is absolute,
/// subsequent ticks carry only the difference from the previous tick.
/// We maintain the last absolute values per `(msg_type, contract_id)`.
#[doc(hidden)]
pub struct DeltaState {
    /// Key: `(StreamMsgType as u8, contract_id)`, Value: last absolute
    /// tick data. Stored inline (no `Vec`) so the per-tick `apply_deltas`
    /// step costs one stack copy + one in-place add loop with zero
    /// allocations on the hot path.
    prev: HashMap<(u8, i32), TickFields>,
    /// Reusable scratch buffer for FIT decoding, avoiding per-tick allocation.
    /// Resized (never shrunk) to fit the largest tick type seen.
    alloc_buf: Vec<i32>,
    /// Set after `decode_tick` to indicate the last row was a DATE marker.
    /// Callers use this to distinguish normal DATE skips from corrupt payloads.
    pub(super) last_was_date: bool,
    /// Actual data field count from the first absolute tick for each
    /// `(msg_type, contract_id)`. The FPSS stream trade is the 8-field
    /// layout; the 16-field "extended" trade is an MDDS gRPC shape on a
    /// different protocol and is never seen here.
    ///
    /// The width is fixed at the first absolute row and not revised by later
    /// rows. This is correct because the trade-format width is a server-wide
    /// constant for the session, not a per-row property: every row a given
    /// peer emits carries the same field count. Subsequent rows for the same
    /// `(msg_type, contract_id)` are FIT delta rows whose changed-field count
    /// is smaller than the full width, so they cannot be used to re-derive it.
    /// The decode buffer is always the full [`MAX_DATA_FIELDS`] width with
    /// unused slots zero-filled, so a stale width can only under-report which
    /// trailing fields a caller reads, never read out of bounds.
    field_counts: HashMap<(u8, i32), usize>,
    /// Timestamp of last STOP (market close) signal. Used to suppress
    /// "unknown `contract_id`" warnings for 5 seconds after STOP;
    /// stale ticks are expected during teardown.
    pub(super) last_stop: Option<Instant>,
}

/// Duration after a STOP signal during which "unknown `contract_id`" warnings
/// are suppressed (stale ticks are expected during market-close teardown).
const STOP_SUPPRESS_DURATION: Duration = Duration::from_secs(5);

impl DeltaState {
    #[doc(hidden)]
    pub fn new() -> Self {
        // Pre-allocate the FIT scratch buffer for the widest tick shape
        // (`MAX_DATA_FIELDS` data fields + 1 contract_id). It resizes at
        // runtime if needed, but the initial capacity is the real maximum.
        Self {
            prev: HashMap::new(),
            alloc_buf: vec![0i32; MAX_DATA_FIELDS + 1],
            last_was_date: false,
            field_counts: HashMap::new(),
            last_stop: None,
        }
    }

    /// Clear all accumulated delta state.
    ///
    /// Called on START/STOP (market open/close) signals to reset delta
    /// decompression: the contract-id read starts fresh after a session
    /// boundary.
    ///
    /// Note: `last_stop` is intentionally NOT cleared here because STOP
    /// itself calls `clear()`, and the timestamp must survive to suppress
    /// stale-tick warnings for 5 seconds after the STOP signal.
    pub fn clear(&mut self) {
        self.prev.clear();
        self.last_was_date = false;
        self.field_counts.clear();
    }

    /// Whether we are within the post-STOP suppression window.
    pub(super) fn is_in_stop_suppression_window(&self) -> bool {
        self.last_stop
            .is_some_and(|t| t.elapsed() < STOP_SUPPRESS_DURATION)
    }

    /// Decode FIT payload and apply delta decompression.
    ///
    /// The ENTIRE payload is FIT-encoded. The first FIT field (alloc[0]) is the
    /// `contract_id`. Tick data fields start at alloc[1..].
    ///
    /// `out` is a caller-owned stack buffer that receives the absolute tick
    /// values for fields `[0..data_field_count]`. The caller reads the
    /// individual fields directly from `out` to construct the public event
    /// — no `Vec<i32>` is allocated on the decode path.
    ///
    /// Wire sequence: the FIT reader opens at offset 0 and decodes the row's
    /// changes into `alloc`, where `alloc[0]` is the `contract_id` used to
    /// resolve the contract and `alloc[1..]` are the tick fields.
    ///
    /// Returns `Some((contract_id, data_field_count))` on success, or `None`
    /// when the payload is empty or the FIT row is a DATE marker. Sets
    /// `self.last_was_date` so callers can distinguish DATE markers from
    /// corrupt payloads.
    pub(super) fn decode_tick(
        &mut self,
        msg_code: u8,
        payload: &[u8],
        expected_fields: usize,
        out: &mut TickFields,
    ) -> Option<(i32, usize)> {
        self.last_was_date = false;

        if payload.is_empty() {
            return None;
        }

        // Reuse the FIT scratch buffer: resize if needed (retains
        // capacity), then zero-fill the portion we need.
        let total_fields = expected_fields + 1;
        if self.alloc_buf.len() < total_fields {
            self.alloc_buf.resize(total_fields, 0);
        }
        self.alloc_buf[..total_fields].fill(0);

        let mut reader = FitReader::new(payload);
        let n = reader.read_changes(&mut self.alloc_buf[..total_fields]);

        if reader.is_date {
            // DATE marker row -- skip (no user-visible data).
            self.last_was_date = true;
            return None;
        }

        // Reject a truncated row: the FIT reader hit end-of-buffer before the
        // terminating `END` nibble, so it flushed a partial integer and the
        // remaining slots stayed zero-filled. Emitting that as a tick would
        // surface silent zero/garbage fields, so treat it as a decode failure.
        // Callers map the `None` to `Unparseable`, which the io-loop already
        // handles without panicking.
        if !reader.row_complete {
            return None;
        }

        if n == 0 {
            return None;
        }

        // First FIT field is the contract_id.
        let contract_id = self.alloc_buf[0];

        // Copy tick data (alloc[1..]) into the caller-owned stack buffer.
        // The slot count is bounded by MAX_DATA_FIELDS at the type level —
        // expected_fields is a build-time constant for every supported
        // tick shape (QUOTE_FIELDS, TRADE_FIELDS, OI_FIELDS, OHLCVC_FIELDS).
        debug_assert!(expected_fields <= MAX_DATA_FIELDS);

        // Delta decompression applies only to the tick portion (excluding
        // contract_id): the first field (contract_id) is skipped, and deltas
        // from `firstData[1..]` accumulate onto `data[0..]`:
        //   for i in 1..len { data[i - 1] = firstData[i] + data[i - 1]; }
        let tick_n = n.saturating_sub(1);

        let key = (msg_code, contract_id);
        let is_absolute = !self.prev.contains_key(&key);

        // An absolute row defines the cached field width and seeds the delta
        // baseline. It must not declare MORE fields than the tick shape holds:
        // a row wider than `expected_fields` would have its trailing fields
        // silently dropped (the scratch buffer is `total_fields` wide) and the
        // cached width would over-report, so reject it as a decode failure.
        // A truncated row is already rejected above via `row_complete`. A
        // complete row with fewer fields is a legitimately narrower wire
        // layout (e.g. the simple-format trade); the per-tick consumer
        // validates that the narrower width is one it knows how to read.
        if is_absolute && tick_n > expected_fields {
            return None;
        }

        out.fill(0);
        out[..expected_fields].copy_from_slice(&self.alloc_buf[1..total_fields]);

        if let Some(prev) = self.prev.get(&key) {
            // Delta row: accumulate onto previous absolute values
            // in-place into the caller's buffer.
            apply_deltas(
                &mut out[..expected_fields],
                &prev[..expected_fields],
                tick_n,
            );
        } else {
            // First absolute tick for this `(msg_type, contract_id)`: record
            // the actual field count. The maps grow with the live universe
            // and reset at every START/STOP/RESTART/RECONNECTED session
            // boundary (see `clear`), matching the terminal, which imposes
            // no per-session contract cap.
            self.field_counts.insert(key, tick_n);
        }

        // Store the resolved absolute row into `prev` for the next delta.
        // `[i32; MAX_DATA_FIELDS]` is `Copy`, so this is a memcpy into the
        // existing slot (or a one-time node allocation on first insert);
        // no `Vec::clone` per tick.
        self.prev.insert(key, *out);

        let data_fields = *self.field_counts.get(&key).unwrap_or(&expected_fields);
        Some((contract_id, data_fields))
    }

    /// Distinct-row counts of the two per-session maps, exposed for
    /// state-retention tests.
    #[cfg(test)]
    pub(super) fn state_sizes(&self) -> (usize, usize) {
        (self.prev.len(), self.field_counts.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIELD_SEP: u8 = 0xB;
    const END: u8 = 0xD;

    /// Encode a single complete FIT row of non-negative `i32` values:
    /// digit runs joined by `FIELD_SEP` and terminated by `END`. The first
    /// value is the `contract_id`, the rest are tick fields.
    fn encode_row(values: &[i32]) -> Vec<u8> {
        let mut nibbles: Vec<u8> = Vec::new();
        for (i, &v) in values.iter().enumerate() {
            if i > 0 {
                nibbles.push(FIELD_SEP);
            }
            for ch in v.to_string().bytes() {
                nibbles.push(ch - b'0');
            }
        }
        nibbles.push(END);
        let mut out = Vec::with_capacity(nibbles.len() / 2 + 1);
        let mut i = 0;
        while i < nibbles.len() {
            let high = nibbles[i];
            let low = if i + 1 < nibbles.len() {
                nibbles[i + 1]
            } else {
                0
            };
            out.push((high << 4) | (low & 0x0F));
            i += 2;
        }
        out
    }

    // A representative non-trade message code; the value only keys the maps.
    const QUOTE_CODE: u8 = 2;

    #[test]
    fn complete_quote_row_decodes_to_correct_tick() {
        let mut state = DeltaState::new();
        // contract_id = 7, then 11 quote data fields 100..110.
        let mut values = vec![7];
        values.extend(100..=110);
        assert_eq!(values.len(), QUOTE_FIELDS + 1);
        let payload = encode_row(&values);

        let mut out: TickFields = [0; MAX_DATA_FIELDS];
        let decoded = state.decode_tick(QUOTE_CODE, &payload, QUOTE_FIELDS, &mut out);
        let (contract_id, n_data) = decoded.expect("complete row decodes");
        assert_eq!(contract_id, 7);
        assert_eq!(n_data, QUOTE_FIELDS);
        for (i, v) in (100..=110).enumerate() {
            assert_eq!(out[i], v, "field {i} mismatch");
        }
    }

    #[test]
    fn truncated_quote_row_missing_end_is_rejected() {
        let mut state = DeltaState::new();
        let mut values = vec![7];
        values.extend(100..=110);
        let mut payload = encode_row(&values);
        // Drop the trailing byte(s) so the END nibble is gone: the row now
        // ends mid-buffer with no terminator.
        payload.pop();
        payload.pop();

        let mut out: TickFields = [0; MAX_DATA_FIELDS];
        let decoded = state.decode_tick(QUOTE_CODE, &payload, QUOTE_FIELDS, &mut out);
        assert!(
            decoded.is_none(),
            "a truncated row must decode to None, not a zero-filled tick"
        );
        assert!(
            !state.last_was_date,
            "rejection is a decode failure, not a DATE skip"
        );
        // The reject path must not have cached any width/baseline.
        assert_eq!(state.state_sizes().0, 0, "no prev baseline cached");
    }

    #[test]
    fn truncated_first_row_does_not_poison_width_for_later_delta() {
        let mut state = DeltaState::new();
        let cid = 7;

        // A truncated first row (no END) must be rejected and leave NO cached
        // baseline or width.
        let mut full = vec![cid];
        full.extend(100..=110);
        let mut truncated = encode_row(&full);
        truncated.pop();
        truncated.pop();
        let mut out: TickFields = [0; MAX_DATA_FIELDS];
        assert!(state
            .decode_tick(QUOTE_CODE, &truncated, QUOTE_FIELDS, &mut out)
            .is_none());
        assert_eq!(state.state_sizes().0, 0, "truncated row poisoned the cache");

        // A subsequent COMPLETE absolute row for the same contract decodes as
        // a clean absolute tick with the full width, proving the earlier
        // truncated row left no stale state behind.
        let good = encode_row(&full);
        let decoded = state.decode_tick(QUOTE_CODE, &good, QUOTE_FIELDS, &mut out);
        let (contract_id, n_data) = decoded.expect("complete row decodes");
        assert_eq!(contract_id, cid);
        assert_eq!(n_data, QUOTE_FIELDS);
        for (i, v) in (100..=110).enumerate() {
            assert_eq!(out[i], v, "field {i} mismatch after clean re-seed");
        }
    }

    #[test]
    fn complete_then_delta_row_accumulates() {
        // Confirms the happy path is unchanged: a complete absolute row seeds
        // the baseline and a following delta row accumulates onto it.
        let mut state = DeltaState::new();
        let cid = 7;
        let mut abs = vec![cid];
        abs.extend(100..=110);
        let abs_payload = encode_row(&abs);
        let mut out: TickFields = [0; MAX_DATA_FIELDS];
        state
            .decode_tick(QUOTE_CODE, &abs_payload, QUOTE_FIELDS, &mut out)
            .expect("absolute row decodes");

        // Delta row: contract_id then a single +5 change to field 0.
        let delta_payload = encode_row(&[cid, 5]);
        let decoded = state.decode_tick(QUOTE_CODE, &delta_payload, QUOTE_FIELDS, &mut out);
        let (contract_id, _n) = decoded.expect("delta row decodes");
        assert_eq!(contract_id, cid);
        assert_eq!(out[0], 105, "delta accumulated onto prior absolute value");
        assert_eq!(out[1], 101, "unchanged field carried forward from baseline");
    }
}
