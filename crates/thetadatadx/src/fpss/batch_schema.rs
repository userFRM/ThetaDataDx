//! Fixed columnar schema for the streaming Arrow `RecordBatch` reader.
//!
//! The per-event callback delivers heterogeneous [`StreamData`] variants
//! (quote, trade, open-interest, OHLCVC, market-value) one at a time. The
//! pull-based [`super::batch_reader::RecordBatchStream`] instead delivers
//! the same market-data events in columnar batches, and the columnar
//! contract requires a single schema that is FIXED for the lifetime of the
//! subscription and identical across every batch so a downstream consumer
//! can concatenate batches without a schema reconciliation pass.
//!
//! A live subscription interleaves event variants, so the only lossless,
//! concat-safe layout is a unified record that can carry any data variant:
//! a leading `event_type` discriminator, the contract identity columns, the
//! three fields common to every variant (`ms_of_day`, `date`,
//! `received_at_ns`), and the union of every per-variant payload column.
//! Columns that do not apply to a given event are null for that row. This is
//! the same shape an institutional live feed uses for a multiplexed columnar
//! channel: one record layout, a type tag, nullable payload columns.
//!
//! This module is the single source of truth for that layout. The schema is
//! built once here; every binding consumes it through the Arrow C Data
//! Interface (Python / C++) or Arrow IPC (TypeScript), so the column order,
//! names, and dtypes are defined in exactly one place and cannot drift.
//!
//! Control / lifecycle events (login, reconnect, market open/close, …) are
//! not market data and carry no columnar payload; they are not delivered on
//! the batch channel. A consumer that needs lifecycle visibility uses the
//! per-event callback, which is unchanged and remains the sibling delivery
//! mode.

use std::sync::Arc;

use arrow_array::builder::{
    Float64Builder, Int32Builder, Int64Builder, StringBuilder, UInt64Builder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{ArrowError, DataType, Field, Schema};

use super::events::{StreamData, StreamEvent};
use crate::tdbe::types::enums::Right;

/// Discriminator value written into the `event_type` column for each
/// market-data variant. Stable string tags so a consumer can filter a
/// batch by `event_type` without reaching for an enum mapping.
pub mod event_type {
    /// `event_type` tag for a [`super::super::StreamData::Quote`] row.
    pub const QUOTE: &str = "quote";
    /// `event_type` tag for a [`super::super::StreamData::Trade`] row.
    pub const TRADE: &str = "trade";
    /// `event_type` tag for a [`super::super::StreamData::OpenInterest`] row.
    pub const OPEN_INTEREST: &str = "open_interest";
    /// `event_type` tag for a [`super::super::StreamData::Ohlcvc`] row.
    pub const OHLCVC: &str = "ohlcvc";
    /// `event_type` tag for a [`super::super::StreamData::MarketValue`] row.
    pub const MARKET_VALUE: &str = "market_value";
}

/// Build the fixed Arrow schema shared by every batch the streaming reader
/// emits.
///
/// The column order here is the canonical order; [`StreamBatchBuilder`]
/// assembles its column arrays in exactly this order, and the FFI / binding
/// layers never reorder. Keeping the order in one function means a new
/// column is added in one place and every binding picks it up through the
/// exported schema.
#[must_use]
pub fn stream_batch_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        // ── discriminator + contract identity ──
        Field::new("event_type", DataType::Utf8, false),
        Field::new("symbol", DataType::Utf8, false),
        Field::new("sec_type", DataType::Int32, false),
        Field::new("expiration", DataType::Int32, true),
        Field::new("strike", DataType::Float64, true),
        Field::new("right", DataType::Utf8, true),
        // ── common to every data variant ──
        Field::new("ms_of_day", DataType::Int32, false),
        Field::new("date", DataType::Int32, false),
        Field::new("received_at_ns", DataType::UInt64, false),
        // ── quote payload ──
        Field::new("bid", DataType::Float64, true),
        Field::new("bid_size", DataType::Int32, true),
        Field::new("bid_exchange", DataType::Int32, true),
        Field::new("bid_condition", DataType::Int32, true),
        Field::new("ask", DataType::Float64, true),
        Field::new("ask_size", DataType::Int32, true),
        Field::new("ask_exchange", DataType::Int32, true),
        Field::new("ask_condition", DataType::Int32, true),
        // ── trade payload ──
        Field::new("price", DataType::Float64, true),
        Field::new("size", DataType::Int32, true),
        Field::new("exchange", DataType::Int32, true),
        Field::new("sequence", DataType::Int32, true),
        Field::new("condition", DataType::Int32, true),
        Field::new("ext_condition1", DataType::Int32, true),
        Field::new("ext_condition2", DataType::Int32, true),
        Field::new("ext_condition3", DataType::Int32, true),
        Field::new("ext_condition4", DataType::Int32, true),
        Field::new("condition_flags", DataType::Int32, true),
        Field::new("price_flags", DataType::Int32, true),
        Field::new("volume_type", DataType::Int32, true),
        Field::new("records_back", DataType::Int32, true),
        // ── open-interest payload ──
        Field::new("open_interest", DataType::Int32, true),
        // ── OHLCVC payload ──
        Field::new("open", DataType::Float64, true),
        Field::new("high", DataType::Float64, true),
        Field::new("low", DataType::Float64, true),
        Field::new("close", DataType::Float64, true),
        Field::new("volume", DataType::Int64, true),
        Field::new("count", DataType::Int64, true),
        // ── market-value payload ──
        Field::new("market_bid", DataType::Float64, true),
        Field::new("market_ask", DataType::Float64, true),
        Field::new("market_price", DataType::Float64, true),
    ]))
}

/// Column-oriented accumulator that turns a run of [`StreamData`] events
/// into a single [`RecordBatch`] under [`stream_batch_schema`].
///
/// One builder is reused across the lifetime of a [`RecordBatchStream`]:
/// [`Self::append`] pushes one row per data event, [`Self::len`] reports the
/// rows buffered so far so the reader can decide when a batch is full, and
/// [`Self::finish`] drains the builders into a `RecordBatch` and leaves the
/// accumulator empty for the next batch. Every batch therefore carries the
/// identical schema instance, which is what makes the output concat-safe.
///
/// Builders preallocate to `capacity` so the steady-state append path does
/// no reallocation until a batch is flushed.
///
/// [`RecordBatchStream`]: super::batch_reader::RecordBatchStream
pub struct StreamBatchBuilder {
    schema: Arc<Schema>,
    rows: usize,

    event_type: StringBuilder,
    symbol: StringBuilder,
    sec_type: Int32Builder,
    expiration: Int32Builder,
    strike: Float64Builder,
    right: StringBuilder,

    ms_of_day: Int32Builder,
    date: Int32Builder,
    received_at_ns: UInt64Builder,

    bid: Float64Builder,
    bid_size: Int32Builder,
    bid_exchange: Int32Builder,
    bid_condition: Int32Builder,
    ask: Float64Builder,
    ask_size: Int32Builder,
    ask_exchange: Int32Builder,
    ask_condition: Int32Builder,

    price: Float64Builder,
    size: Int32Builder,
    exchange: Int32Builder,
    sequence: Int32Builder,
    condition: Int32Builder,
    ext_condition1: Int32Builder,
    ext_condition2: Int32Builder,
    ext_condition3: Int32Builder,
    ext_condition4: Int32Builder,
    condition_flags: Int32Builder,
    price_flags: Int32Builder,
    volume_type: Int32Builder,
    records_back: Int32Builder,

    open_interest: Int32Builder,

    open: Float64Builder,
    high: Float64Builder,
    low: Float64Builder,
    close: Float64Builder,
    volume: Int64Builder,
    count: Int64Builder,

    market_bid: Float64Builder,
    market_ask: Float64Builder,
    market_price: Float64Builder,
}

impl StreamBatchBuilder {
    /// Create an empty builder with every column preallocated to
    /// `capacity` rows.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        // String builders take both an item-count and a byte-count hint;
        // symbols are short roots (a handful of bytes), the discriminator
        // and right tags shorter still, so the byte hints are deliberately
        // small multiples of `capacity`.
        Self {
            schema: stream_batch_schema(),
            rows: 0,
            event_type: StringBuilder::with_capacity(capacity, capacity * 8),
            symbol: StringBuilder::with_capacity(capacity, capacity * 8),
            sec_type: Int32Builder::with_capacity(capacity),
            expiration: Int32Builder::with_capacity(capacity),
            strike: Float64Builder::with_capacity(capacity),
            right: StringBuilder::with_capacity(capacity, capacity),
            ms_of_day: Int32Builder::with_capacity(capacity),
            date: Int32Builder::with_capacity(capacity),
            received_at_ns: UInt64Builder::with_capacity(capacity),
            bid: Float64Builder::with_capacity(capacity),
            bid_size: Int32Builder::with_capacity(capacity),
            bid_exchange: Int32Builder::with_capacity(capacity),
            bid_condition: Int32Builder::with_capacity(capacity),
            ask: Float64Builder::with_capacity(capacity),
            ask_size: Int32Builder::with_capacity(capacity),
            ask_exchange: Int32Builder::with_capacity(capacity),
            ask_condition: Int32Builder::with_capacity(capacity),
            price: Float64Builder::with_capacity(capacity),
            size: Int32Builder::with_capacity(capacity),
            exchange: Int32Builder::with_capacity(capacity),
            sequence: Int32Builder::with_capacity(capacity),
            condition: Int32Builder::with_capacity(capacity),
            ext_condition1: Int32Builder::with_capacity(capacity),
            ext_condition2: Int32Builder::with_capacity(capacity),
            ext_condition3: Int32Builder::with_capacity(capacity),
            ext_condition4: Int32Builder::with_capacity(capacity),
            condition_flags: Int32Builder::with_capacity(capacity),
            price_flags: Int32Builder::with_capacity(capacity),
            volume_type: Int32Builder::with_capacity(capacity),
            records_back: Int32Builder::with_capacity(capacity),
            open_interest: Int32Builder::with_capacity(capacity),
            open: Float64Builder::with_capacity(capacity),
            high: Float64Builder::with_capacity(capacity),
            low: Float64Builder::with_capacity(capacity),
            close: Float64Builder::with_capacity(capacity),
            volume: Int64Builder::with_capacity(capacity),
            count: Int64Builder::with_capacity(capacity),
            market_bid: Float64Builder::with_capacity(capacity),
            market_ask: Float64Builder::with_capacity(capacity),
            market_price: Float64Builder::with_capacity(capacity),
        }
    }

    /// The fixed schema every batch this builder produces will carry.
    #[must_use]
    pub fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }

    /// Rows buffered since the last [`Self::finish`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows
    }

    /// Whether any rows are buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Append one event if it is a market-data variant, returning `true`
    /// when a row was written.
    ///
    /// Control events carry no columnar payload and are skipped (returning
    /// `false`) — they never reach the batch channel, so in practice the
    /// reader only ever hands data events here, but accepting a
    /// [`StreamEvent`] keeps the call site on the consumer side simple and
    /// makes the data/control split explicit in one place.
    pub fn append(&mut self, event: &StreamEvent) -> bool {
        let StreamEvent::Data(data) = event else {
            return false;
        };
        self.append_data(data);
        true
    }

    /// Append one market-data event as a row, filling the discriminator,
    /// contract identity, and common columns, the payload columns for the
    /// matching variant, and a null for every payload column that does not
    /// apply to this variant.
    pub fn append_data(&mut self, data: &StreamData) {
        match data {
            StreamData::Quote {
                contract,
                ms_of_day,
                bid_size,
                bid_exchange,
                bid,
                bid_condition,
                ask_size,
                ask_exchange,
                ask,
                ask_condition,
                date,
                received_at_ns,
            } => {
                self.push_header(
                    event_type::QUOTE,
                    contract,
                    *ms_of_day,
                    *date,
                    *received_at_ns,
                );
                // quote payload
                self.bid.append_value(*bid);
                self.bid_size.append_value(*bid_size);
                self.bid_exchange.append_value(*bid_exchange);
                self.bid_condition.append_value(*bid_condition);
                self.ask.append_value(*ask);
                self.ask_size.append_value(*ask_size);
                self.ask_exchange.append_value(*ask_exchange);
                self.ask_condition.append_value(*ask_condition);
                // non-quote payloads null
                self.null_trade();
                self.null_open_interest();
                self.null_ohlcvc();
                self.null_market_value();
            }
            StreamData::Trade {
                contract,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                received_at_ns,
            } => {
                self.push_header(
                    event_type::TRADE,
                    contract,
                    *ms_of_day,
                    *date,
                    *received_at_ns,
                );
                self.null_quote();
                // trade payload
                self.price.append_value(*price);
                self.size.append_value(*size);
                self.exchange.append_value(*exchange);
                self.sequence.append_value(*sequence);
                self.condition.append_value(*condition);
                self.ext_condition1.append_value(*ext_condition1);
                self.ext_condition2.append_value(*ext_condition2);
                self.ext_condition3.append_value(*ext_condition3);
                self.ext_condition4.append_value(*ext_condition4);
                self.condition_flags.append_value(*condition_flags);
                self.price_flags.append_value(*price_flags);
                self.volume_type.append_value(*volume_type);
                self.records_back.append_value(*records_back);
                self.null_open_interest();
                self.null_ohlcvc();
                self.null_market_value();
            }
            StreamData::OpenInterest {
                contract,
                ms_of_day,
                open_interest,
                date,
                received_at_ns,
            } => {
                self.push_header(
                    event_type::OPEN_INTEREST,
                    contract,
                    *ms_of_day,
                    *date,
                    *received_at_ns,
                );
                self.null_quote();
                self.null_trade();
                self.open_interest.append_value(*open_interest);
                self.null_ohlcvc();
                self.null_market_value();
            }
            StreamData::Ohlcvc {
                contract,
                ms_of_day,
                open,
                high,
                low,
                close,
                volume,
                count,
                date,
                received_at_ns,
            } => {
                self.push_header(
                    event_type::OHLCVC,
                    contract,
                    *ms_of_day,
                    *date,
                    *received_at_ns,
                );
                self.null_quote();
                self.null_trade();
                self.null_open_interest();
                self.open.append_value(*open);
                self.high.append_value(*high);
                self.low.append_value(*low);
                self.close.append_value(*close);
                self.volume.append_value(*volume);
                self.count.append_value(*count);
                self.null_market_value();
            }
            StreamData::MarketValue {
                contract,
                ms_of_day,
                market_bid,
                market_ask,
                market_price,
                date,
                received_at_ns,
            } => {
                self.push_header(
                    event_type::MARKET_VALUE,
                    contract,
                    *ms_of_day,
                    *date,
                    *received_at_ns,
                );
                self.null_quote();
                self.null_trade();
                self.null_open_interest();
                self.null_ohlcvc();
                self.market_bid.append_value(*market_bid);
                self.market_ask.append_value(*market_ask);
                self.market_price.append_value(*market_price);
            }
        }
        self.rows += 1;
    }

    /// Drain the buffered rows into a [`RecordBatch`] and reset the
    /// accumulator so the next batch starts empty.
    ///
    /// Returns `Ok(None)` when no rows are buffered, so a caller can call
    /// this unconditionally on a linger timeout without emitting an empty
    /// batch.
    ///
    /// # Errors
    ///
    /// Returns an [`ArrowError`] if the assembled column arrays cannot form
    /// a `RecordBatch` against the schema (a column-length mismatch, which
    /// the append discipline above makes unreachable in practice).
    pub fn finish(&mut self) -> Result<Option<RecordBatch>, ArrowError> {
        if self.rows == 0 {
            return Ok(None);
        }
        let columns: Vec<ArrayRef> = vec![
            Arc::new(self.event_type.finish()) as ArrayRef,
            Arc::new(self.symbol.finish()) as ArrayRef,
            Arc::new(self.sec_type.finish()) as ArrayRef,
            Arc::new(self.expiration.finish()) as ArrayRef,
            Arc::new(self.strike.finish()) as ArrayRef,
            Arc::new(self.right.finish()) as ArrayRef,
            Arc::new(self.ms_of_day.finish()) as ArrayRef,
            Arc::new(self.date.finish()) as ArrayRef,
            Arc::new(self.received_at_ns.finish()) as ArrayRef,
            Arc::new(self.bid.finish()) as ArrayRef,
            Arc::new(self.bid_size.finish()) as ArrayRef,
            Arc::new(self.bid_exchange.finish()) as ArrayRef,
            Arc::new(self.bid_condition.finish()) as ArrayRef,
            Arc::new(self.ask.finish()) as ArrayRef,
            Arc::new(self.ask_size.finish()) as ArrayRef,
            Arc::new(self.ask_exchange.finish()) as ArrayRef,
            Arc::new(self.ask_condition.finish()) as ArrayRef,
            Arc::new(self.price.finish()) as ArrayRef,
            Arc::new(self.size.finish()) as ArrayRef,
            Arc::new(self.exchange.finish()) as ArrayRef,
            Arc::new(self.sequence.finish()) as ArrayRef,
            Arc::new(self.condition.finish()) as ArrayRef,
            Arc::new(self.ext_condition1.finish()) as ArrayRef,
            Arc::new(self.ext_condition2.finish()) as ArrayRef,
            Arc::new(self.ext_condition3.finish()) as ArrayRef,
            Arc::new(self.ext_condition4.finish()) as ArrayRef,
            Arc::new(self.condition_flags.finish()) as ArrayRef,
            Arc::new(self.price_flags.finish()) as ArrayRef,
            Arc::new(self.volume_type.finish()) as ArrayRef,
            Arc::new(self.records_back.finish()) as ArrayRef,
            Arc::new(self.open_interest.finish()) as ArrayRef,
            Arc::new(self.open.finish()) as ArrayRef,
            Arc::new(self.high.finish()) as ArrayRef,
            Arc::new(self.low.finish()) as ArrayRef,
            Arc::new(self.close.finish()) as ArrayRef,
            Arc::new(self.volume.finish()) as ArrayRef,
            Arc::new(self.count.finish()) as ArrayRef,
            Arc::new(self.market_bid.finish()) as ArrayRef,
            Arc::new(self.market_ask.finish()) as ArrayRef,
            Arc::new(self.market_price.finish()) as ArrayRef,
        ];
        self.rows = 0;
        let batch = RecordBatch::try_new(Arc::clone(&self.schema), columns)?;
        Ok(Some(batch))
    }

    /// Write the discriminator, contract identity, and the three columns
    /// common to every data variant.
    fn push_header(
        &mut self,
        tag: &str,
        contract: &crate::fpss::protocol::Contract,
        ms_of_day: i32,
        date: i32,
        received_at_ns: u64,
    ) {
        self.event_type.append_value(tag);
        self.symbol.append_value(&*contract.symbol);
        self.sec_type.append_value(contract.sec_type as i32);
        // Option columns: expiration / strike / right carry a value only
        // for option contracts, null otherwise. Strike is reported in
        // dollars to match the historical tick Arrow schema.
        match contract.expiration {
            Some(exp) => self.expiration.append_value(exp),
            None => self.expiration.append_null(),
        }
        match contract.strike_thousandths {
            Some(strike_thousandths) => self
                .strike
                .append_value(f64::from(strike_thousandths) / 1000.0),
            None => self.strike.append_null(),
        }
        match contract.right() {
            Some(Right::Call) => self.right.append_value("C"),
            Some(Right::Put) => self.right.append_value("P"),
            // Event-carried contracts never report `Right::Both`; treat any
            // non-call/put right as absent rather than guessing a tag.
            Some(Right::Both) | None => self.right.append_null(),
        }
        self.ms_of_day.append_value(ms_of_day);
        self.date.append_value(date);
        self.received_at_ns.append_value(received_at_ns);
    }

    fn null_quote(&mut self) {
        self.bid.append_null();
        self.bid_size.append_null();
        self.bid_exchange.append_null();
        self.bid_condition.append_null();
        self.ask.append_null();
        self.ask_size.append_null();
        self.ask_exchange.append_null();
        self.ask_condition.append_null();
    }

    fn null_trade(&mut self) {
        self.price.append_null();
        self.size.append_null();
        self.exchange.append_null();
        self.sequence.append_null();
        self.condition.append_null();
        self.ext_condition1.append_null();
        self.ext_condition2.append_null();
        self.ext_condition3.append_null();
        self.ext_condition4.append_null();
        self.condition_flags.append_null();
        self.price_flags.append_null();
        self.volume_type.append_null();
        self.records_back.append_null();
    }

    fn null_open_interest(&mut self) {
        self.open_interest.append_null();
    }

    fn null_ohlcvc(&mut self) {
        self.open.append_null();
        self.high.append_null();
        self.low.append_null();
        self.close.append_null();
        self.volume.append_null();
        self.count.append_null();
    }

    fn null_market_value(&mut self) {
        self.market_bid.append_null();
        self.market_ask.append_null();
        self.market_price.append_null();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpss::protocol::Contract;
    use arrow_array::{Array, Float64Array, Int32Array, StringArray};

    fn trade(contract: &std::sync::Arc<Contract>) -> StreamEvent {
        StreamEvent::Data(StreamData::Trade {
            contract: std::sync::Arc::clone(contract),
            ms_of_day: 100,
            sequence: 7,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 3,
            price: 150.25,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20240315,
            received_at_ns: 42,
        })
    }

    fn quote(contract: &std::sync::Arc<Contract>) -> StreamEvent {
        StreamEvent::Data(StreamData::Quote {
            contract: std::sync::Arc::clone(contract),
            ms_of_day: 200,
            bid_size: 10,
            bid_exchange: 1,
            bid: 150.00,
            bid_condition: 0,
            ask_size: 12,
            ask_exchange: 2,
            ask: 150.50,
            ask_condition: 0,
            date: 20240315,
            received_at_ns: 43,
        })
    }

    /// The discriminator plus contract-identity and common columns are
    /// non-nullable; the per-variant payload columns are nullable. The fixed
    /// layout must match this contract so every binding sees the same shape.
    #[test]
    fn schema_nullability_matches_contract() {
        let schema = stream_batch_schema();
        for f in [
            "event_type",
            "symbol",
            "sec_type",
            "ms_of_day",
            "date",
            "received_at_ns",
        ] {
            assert!(
                !schema.field_with_name(f).unwrap().is_nullable(),
                "{f} must be non-nullable"
            );
        }
        for f in [
            "expiration",
            "strike",
            "right",
            "bid",
            "price",
            "open_interest",
            "open",
            "market_bid",
        ] {
            assert!(
                schema.field_with_name(f).unwrap().is_nullable(),
                "{f} must be nullable"
            );
        }
    }

    /// A trade row fills the trade payload columns and nulls the quote
    /// columns; the discriminator carries the trade tag.
    #[test]
    fn trade_row_fills_trade_columns_and_nulls_others() {
        let contract = std::sync::Arc::new(Contract::stock("SPY"));
        let mut b = StreamBatchBuilder::with_capacity(8);
        assert!(b.append(&trade(&contract)));
        let batch = b.finish().unwrap().expect("one row");
        assert_eq!(batch.num_rows(), 1);

        let event_type = batch
            .column_by_name("event_type")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(event_type.value(0), event_type::TRADE);

        let price = batch
            .column_by_name("price")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!(!price.is_null(0), "trade row carries a price");

        let bid = batch
            .column_by_name("bid")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!(bid.is_null(0), "trade row nulls the quote bid");

        // Stock contract: option-identity columns are null.
        let exp = batch
            .column_by_name("expiration")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert!(exp.is_null(0), "stock contract nulls expiration");
    }

    /// Mixed variants in one batch share the schema; each row tags its own
    /// variant and fills only its payload columns.
    #[test]
    fn mixed_variants_share_one_batch() {
        let contract = std::sync::Arc::new(Contract::stock("SPY"));
        let mut b = StreamBatchBuilder::with_capacity(8);
        b.append(&trade(&contract));
        b.append(&quote(&contract));
        let batch = b.finish().unwrap().expect("two rows");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.schema(), stream_batch_schema());

        let event_type = batch
            .column_by_name("event_type")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(event_type.value(0), event_type::TRADE);
        assert_eq!(event_type.value(1), event_type::QUOTE);

        let bid = batch
            .column_by_name("bid")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!(bid.is_null(0), "trade row nulls bid");
        assert!(!bid.is_null(1), "quote row carries bid");
    }

    /// An option contract surfaces expiration / strike / right; strike is
    /// reported in dollars.
    #[test]
    fn option_contract_fills_identity_columns() {
        let leg = crate::fpss::protocol::OptionLeg {
            expiration: "20240920",
            strike: "150", // $150.00
            right: "C",
        };
        let contract = std::sync::Arc::new(Contract::option("SPY", leg).expect("valid option"));
        let mut b = StreamBatchBuilder::with_capacity(8);
        b.append(&StreamEvent::Data(StreamData::Trade {
            contract: std::sync::Arc::clone(&contract),
            ms_of_day: 1,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 1,
            exchange: 0,
            price: 1.0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20240315,
            received_at_ns: 0,
        }));
        let batch = b.finish().unwrap().expect("one row");

        let strike = batch
            .column_by_name("strike")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((strike.value(0) - 150.0).abs() < 1e-9, "strike in dollars");

        let right = batch
            .column_by_name("right")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(right.value(0), "C");
    }

    /// `finish` on an empty builder yields no batch (so a linger flush on a
    /// quiet stream never emits an empty batch) and resets `len`.
    #[test]
    fn empty_finish_yields_none() {
        let mut b = StreamBatchBuilder::with_capacity(8);
        assert!(b.is_empty());
        assert!(b.finish().unwrap().is_none());
        let contract = std::sync::Arc::new(Contract::stock("SPY"));
        b.append(&trade(&contract));
        assert_eq!(b.len(), 1);
        let _ = b.finish().unwrap();
        assert_eq!(b.len(), 0, "finish resets the row count");
    }

    /// Control events carry no columnar payload and are not appended.
    #[test]
    fn control_events_are_skipped() {
        let mut b = StreamBatchBuilder::with_capacity(8);
        let appended = b.append(&StreamEvent::Control(
            crate::fpss::StreamControl::MarketOpen,
        ));
        assert!(!appended, "control events are not columnar rows");
        assert!(b.is_empty());
    }
}
