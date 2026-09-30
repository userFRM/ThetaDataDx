// Pull-based Arrow RecordBatch reader (`client.stream().batches(..)`).
//
// Offline: no live server. The end-to-end batching / linger / backpressure
// behaviour is proven offline in the Rust core's `fpss::batch_reader` tests.
// This C++ test pins the binding-side type contract that does NOT need a
// connection: `thetadatadx::RecordBatchStream` is a concrete
// `arrow::RecordBatchReader` (the type the design mandates), with the
// `ReadNext` / `schema` / `dropped` / `close` surface.
//
// Built only when `-DTHETADATADX_CPP_ARROW=ON` (which links arrow-cpp). The
// CMake guard keeps this file out of the default build, matching the
// header-side `#ifdef THETADATADX_CPP_ARROW` gate on the reader itself.

#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include <arrow/record_batch.h>

#include "thetadatadx.hpp"

TEST_CASE("RecordBatchStream is an arrow::RecordBatchReader", "[streaming][arrow][offline]") {
    // The design mandates the C++ reader subclass `arrow::RecordBatchReader`.
    static_assert(
        std::is_base_of<arrow::RecordBatchReader, thetadatadx::RecordBatchStream>::value,
        "thetadatadx::RecordBatchStream must be an arrow::RecordBatchReader");
    SUCCEED("type contract holds at compile time");
}
