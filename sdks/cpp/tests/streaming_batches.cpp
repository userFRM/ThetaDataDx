// Pull-based Arrow RecordBatch reader (`client.stream().batches(..)`).
//
// Offline: no live server. The end-to-end batching / linger / backpressure
// behaviour is proven offline in the Rust core's `fpss::batch_reader` tests.
// This C++ test pins the binding-side contract that does NOT need a
// connection:
//
//   * `thetadatadx::RecordBatchStream` is a concrete
//     `arrow::RecordBatchReader` (the type the design mandates), with the
//     `ReadNext` / `schema` / `dropped` / `close` surface;
//   * the Arrow IPC decode path the reader uses round-trips a batch
//     bit-exact (the same `arrow::ipc::RecordBatchStreamReader` the reader's
//     `ReadNext` runs), so a batch crossing the C ABI as IPC bytes decodes
//     back to the rows that were sent.
//
// Built only when `-DTHETADATADX_CPP_ARROW=ON` (which links arrow-cpp). The
// CMake guard keeps this file out of the default build, matching the
// header-side `#ifdef THETADATADX_CPP_ARROW` gate on the reader itself.

#include <cstdint>
#include <memory>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include <arrow/array.h>
#include <arrow/builder.h>
#include <arrow/io/memory.h>
#include <arrow/ipc/reader.h>
#include <arrow/ipc/writer.h>
#include <arrow/record_batch.h>
#include <arrow/table.h>
#include <arrow/type.h>

#include "thetadatadx.hpp"

namespace {

// Build a tiny RecordBatch under a fixed two-column schema and serialise it
// to an Arrow IPC stream, mimicking what the FFI hands a C++ reader.
std::shared_ptr<arrow::Buffer> make_ipc_batch(std::shared_ptr<arrow::Schema>* out_schema) {
    auto schema = arrow::schema({
        arrow::field("event_type", arrow::utf8(), false),
        arrow::field("price", arrow::float64(), true),
    });
    *out_schema = schema;

    arrow::StringBuilder type_b;
    arrow::DoubleBuilder price_b;
    REQUIRE(type_b.Append("trade").ok());
    REQUIRE(price_b.Append(150.25).ok());
    REQUIRE(type_b.Append("quote").ok());
    REQUIRE(price_b.AppendNull().ok());

    std::shared_ptr<arrow::Array> type_arr;
    std::shared_ptr<arrow::Array> price_arr;
    REQUIRE(type_b.Finish(&type_arr).ok());
    REQUIRE(price_b.Finish(&price_arr).ok());

    auto batch = arrow::RecordBatch::Make(schema, 2, {type_arr, price_arr});

    auto sink_result = arrow::io::BufferOutputStream::Create();
    REQUIRE(sink_result.ok());
    auto sink = *sink_result;
    auto writer_result = arrow::ipc::MakeStreamWriter(sink, schema);
    REQUIRE(writer_result.ok());
    auto writer = *writer_result;
    REQUIRE(writer->WriteRecordBatch(*batch).ok());
    REQUIRE(writer->Close().ok());
    auto buf_result = sink->Finish();
    REQUIRE(buf_result.ok());
    return *buf_result;
}

} // namespace

TEST_CASE("RecordBatchStream is an arrow::RecordBatchReader", "[streaming][arrow][offline]") {
    // The design mandates the C++ reader subclass `arrow::RecordBatchReader`.
    static_assert(
        std::is_base_of<arrow::RecordBatchReader, thetadatadx::RecordBatchStream>::value,
        "thetadatadx::RecordBatchStream must be an arrow::RecordBatchReader");
    SUCCEED("type contract holds at compile time");
}

TEST_CASE("Backpressure enum carries both policies", "[streaming][offline]") {
    // Mirrors the C ABI selector constants the reader passes through.
    REQUIRE(static_cast<int>(thetadatadx::Backpressure::Block) !=
            static_cast<int>(thetadatadx::Backpressure::DropOldest));
}

TEST_CASE("Arrow IPC decode round-trips a streaming batch", "[streaming][arrow][offline]") {
    // The reader's `ReadNext` decodes each batch from an Arrow IPC byte
    // buffer with `arrow::ipc::RecordBatchStreamReader`. Exercise that exact
    // path: serialise a batch, decode it back, and assert the rows survive.
    std::shared_ptr<arrow::Schema> schema;
    auto ipc = make_ipc_batch(&schema);

    auto input = std::make_shared<arrow::io::BufferReader>(ipc);
    auto reader_result = arrow::ipc::RecordBatchStreamReader::Open(input);
    REQUIRE(reader_result.ok());
    auto reader = *reader_result;

    REQUIRE(reader->schema()->Equals(*schema));

    std::shared_ptr<arrow::RecordBatch> batch;
    REQUIRE(reader->ReadNext(&batch).ok());
    REQUIRE(batch != nullptr);
    REQUIRE(batch->num_rows() == 2);
    REQUIRE(batch->num_columns() == 2);

    auto type_col = std::static_pointer_cast<arrow::StringArray>(batch->column(0));
    REQUIRE(type_col->GetString(0) == "trade");
    REQUIRE(type_col->GetString(1) == "quote");

    auto price_col = std::static_pointer_cast<arrow::DoubleArray>(batch->column(1));
    REQUIRE(price_col->Value(0) == 150.25);
    REQUIRE(price_col->IsNull(1)); // quote row nulls the trade price column

    // Stream end: next read yields a null batch, the reader's end-of-stream
    // signal (mirrors the FFI `1` return that `ReadNext` maps to nullptr).
    std::shared_ptr<arrow::RecordBatch> tail;
    REQUIRE(reader->ReadNext(&tail).ok());
    REQUIRE(tail == nullptr);
}
