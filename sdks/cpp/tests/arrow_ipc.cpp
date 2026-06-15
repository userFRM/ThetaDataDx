// Arrow-IPC terminal on the history result vectors (M.2).
//
// Mirrors the FlatFiles `FlatFileRowList::to_arrow_ipc()` exit for the
// typed history rows: `thetadatadx::<tick>_to_arrow_ipc(std::vector<Tick>)`
// serialises the rows to an Arrow IPC stream so a C++ caller can hand the
// bytes to arrow-cpp — the same columnar exit Python exposes via
// `<TickName>List.to_arrow()`. Offline: builds tick vectors in-process and
// checks the serialiser returns a well-formed, non-empty IPC stream
// (schema header present) for both populated and empty inputs, without
// needing an arrow-cpp reader linked into the test.

#include <cstdint>
#include <vector>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

// Arrow IPC streams open with the 0xFFFFFFFF continuation marker followed
// by the metadata length. Checking the marker keeps the assertion free of
// an arrow-cpp dependency while still proving real IPC framing was emitted.
bool looks_like_arrow_ipc_stream(const std::vector<uint8_t>& bytes) {
    if (bytes.size() < 8) {
        return false;
    }
    return bytes[0] == 0xFF && bytes[1] == 0xFF && bytes[2] == 0xFF && bytes[3] == 0xFF;
}

} // namespace

TEST_CASE("eod_ticks_to_arrow_ipc serialises a populated vector", "[arrow][offline]") {
    std::vector<thetadatadx::EodTick> rows;
    TdxEodTick a{};
    a.open = 1.0;
    a.high = 2.0;
    a.low = 0.5;
    a.close = 1.5;
    a.volume = 1000;
    a.date = 20260115;
    rows.push_back(a);
    TdxEodTick b{};
    b.open = 1.5;
    b.close = 1.7;
    b.volume = 2000;
    b.date = 20260116;
    rows.push_back(b);

    const auto ipc = thetadatadx::eod_ticks_to_arrow_ipc(rows);
    REQUIRE(looks_like_arrow_ipc_stream(ipc));
}

TEST_CASE("an empty history vector still yields a valid schema-only stream",
          "[arrow][offline]") {
    const std::vector<thetadatadx::TradeTick> empty;
    const auto ipc = thetadatadx::trade_ticks_to_arrow_ipc(empty);
    // A zero-row result is a valid Arrow stream carrying the schema, not an
    // error — the terminal must not throw on it.
    REQUIRE(looks_like_arrow_ipc_stream(ipc));
}

TEST_CASE("the columnar terminal is present for several history tick types",
          "[arrow][offline]") {
    // A representative spread across the tick families confirms the
    // generator emitted the terminal for each, not just EOD.
    REQUIRE(looks_like_arrow_ipc_stream(thetadatadx::ohlc_ticks_to_arrow_ipc({})));
    REQUIRE(looks_like_arrow_ipc_stream(thetadatadx::quote_ticks_to_arrow_ipc({})));
    REQUIRE(looks_like_arrow_ipc_stream(thetadatadx::greeks_all_ticks_to_arrow_ipc({})));
    REQUIRE(looks_like_arrow_ipc_stream(thetadatadx::interest_rate_ticks_to_arrow_ipc({})));
    REQUIRE(looks_like_arrow_ipc_stream(thetadatadx::calendar_days_to_arrow_ipc({})));
}
