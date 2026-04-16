//! FIT delta-decompression state per `(msg_type, contract_id)`.
//!
//! FPSS ticks arrive FIT-encoded: the first tick for a given stream is
//! absolute, subsequent ticks are deltas against the previous absolute row.
//! This module owns the running absolute state plus the reusable scratch
//! buffer used by [`DeltaState::decode_tick`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tdbe::codec::fit::{apply_deltas, FitReader};

use super::accumulator::OhlcvcAccumulator;

/// Number of FIT fields per tick type (excluding the `contract_id` which is the
/// first FIT field). The FIT decoder returns `n_fields` total, where field [0]
/// is the `contract_id` and fields [1..] are the tick data.
pub(super) const QUOTE_FIELDS: usize = 11;
pub(super) const TRADE_FIELDS: usize = 16;
pub(super) const OI_FIELDS: usize = 3;
pub(super) const OHLCVC_FIELDS: usize = 9;

/// Per-contract, per-message-type delta decompression state.
///
/// FIT uses delta compression: the first tick for a contract is absolute,
/// subsequent ticks carry only the difference from the previous tick.
/// We maintain the last absolute values per `(msg_type, contract_id)`.
pub(super) struct DeltaState {
    /// Key: `(StreamMsgType as u8, contract_id)`, Value: last absolute field values.
    prev: HashMap<(u8, i32), Vec<i32>>,
    /// Per-contract OHLCVC accumulators.
    pub(super) ohlcvc: HashMap<i32, OhlcvcAccumulator>,
    /// Reusable scratch buffer for FIT decoding, avoiding per-tick allocation.
    /// Resized (never shrunk) to fit the largest tick type seen.
    alloc_buf: Vec<i32>,
    /// Set after `decode_tick` to indicate the last row was a DATE marker.
    /// Callers use this to distinguish normal DATE skips from corrupt payloads.
    pub(super) last_was_date: bool,
    /// Actual data field count from the first absolute tick for each
    /// `(msg_type, contract_id)`. The dev server sends 8-field trades (simple
    /// format) while production sends 16-field trades (extended format).
    field_counts: HashMap<(u8, i32), usize>,
    /// Timestamp of last STOP (market close) signal. Used to suppress
    /// "unknown `contract_id`" warnings for 5 seconds after STOP, matching
    /// Java terminal behavior where stale ticks are expected during teardown.
    pub(super) last_stop: Option<Instant>,
}

/// Duration after a STOP signal during which "unknown `contract_id`" warnings
/// are suppressed. The Java terminal silences these for 5 seconds.
const STOP_SUPPRESS_DURATION: Duration = Duration::from_secs(5);

impl DeltaState {
    pub(super) fn new() -> Self {
        // Pre-allocate for the largest tick type (Trade = 16 fields + 1 contract_id).
        Self {
            prev: HashMap::new(),
            ohlcvc: HashMap::new(),
            alloc_buf: vec![0i32; TRADE_FIELDS + 1],
            last_was_date: false,
            field_counts: HashMap::new(),
            last_stop: None,
        }
    }

    /// Clear all accumulated delta state.
    ///
    /// Called on START/STOP (market open/close) signals to reset delta
    /// decompression, matching Java's behavior where `Tick.readID()` starts
    /// fresh after a session boundary.
    ///
    /// Note: `last_stop` is intentionally NOT cleared here because STOP
    /// itself calls `clear()`, and the timestamp must survive to suppress
    /// stale-tick warnings for 5 seconds after the STOP signal.
    pub(super) fn clear(&mut self) {
        self.prev.clear();
        self.ohlcvc.clear();
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
    /// This matches the Java terminal's `FPSSClient` which calls:
    /// ```java
    /// fitReader.open(p.data(), 0, p.len());  // FIT starts at offset 0
    /// int size = fitReader.readChanges(alloc); // alloc[0] = contract_id
    /// Contract c = idToContract.get(alloc[0]); // first field IS the contract_id
    /// ```
    ///
    /// Returns `(contract_id, tick_fields, data_field_count)` or `None` if
    /// payload is too short or the FIT row is a DATE marker. Sets
    /// `self.last_was_date` so callers can distinguish DATE markers from
    /// corrupt payloads.
    pub(super) fn decode_tick(
        &mut self,
        msg_code: u8,
        payload: &[u8],
        expected_fields: usize,
    ) -> Option<(i32, Vec<i32>, usize)> {
        self.last_was_date = false;

        if payload.is_empty() {
            return None;
        }

        // Reuse the scratch buffer: resize if needed (retains capacity),
        // then zero-fill the portion we need.
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

        if n == 0 {
            return None;
        }

        // First FIT field is the contract_id.
        let contract_id = self.alloc_buf[0];

        // Tick data is alloc[1..]. Extract into its own vec.
        // This clone is unavoidable: we need to store a copy in the delta HashMap.
        let mut fields: Vec<i32> = self.alloc_buf[1..total_fields].to_vec();

        // Delta decompression applies only to the tick portion (excluding
        // contract_id), matching Java's `Tick.readID()`:
        //   for (int i = 1; i < len; ++i) {
        //       this.data[i - 1] = firstData[i] + this.data[i - 1];
        //   }
        // It skips firstData[0] (contract_id) and applies deltas from
        // firstData[1..] onto tick data[0..].
        let tick_n = n.saturating_sub(1);

        let key = (msg_code, contract_id);
        if let Some(prev) = self.prev.get(&key) {
            // Delta row: accumulate onto previous absolute values.
            apply_deltas(&mut fields, prev, tick_n);
        } else {
            // First absolute tick: record the actual field count.
            self.field_counts.insert(key, tick_n);
        }

        // Store as the new previous state (tick fields only, not contract_id).
        self.prev.insert(key, fields.clone());

        let data_fields = *self.field_counts.get(&key).unwrap_or(&expected_fields);
        Some((contract_id, fields, data_fields))
    }
}
