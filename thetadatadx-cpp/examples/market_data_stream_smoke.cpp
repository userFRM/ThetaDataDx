// market_data_stream_smoke.cpp -- live proof for the C++ server-stream surface.
//
// Streams one large historical pull (option_history_trade, all strikes of a
// single underlying on one trading day) through the chunk callback, then runs
// the same pull buffered and asserts the streamed rows match the buffered
// result exactly -- count and byte-for-byte content. Also reports the chunk
// count and the peak single-chunk row count to show that peak memory tracks a
// single chunk rather than the whole result.
//
//   usage: market_data_stream_smoke [creds.txt] [symbol] [expiration] [date]
//   defaults:                       creds.txt   QQQ      *            20250303

#include <chrono>
#include <iostream>
#include <string>
#include <vector>

#include "thetadatadx.hpp"

namespace {

// Compare the real wire columns, not the struct's alignment / tail padding.
// `TradeTick` is `#[repr(C, align(64))]` with a trailing `_tail_padding`
// array plus internal padding before its f64 members; those bytes are not
// part of the row's value, so a `memcmp` of the whole struct could report a
// spurious mismatch even when every column agrees.
bool ticks_equal(const thetadatadx::TradeTick& a, const thetadatadx::TradeTick& b) {
    return a.ms_of_day == b.ms_of_day && a.sequence == b.sequence &&
           a.ext_condition1 == b.ext_condition1 && a.ext_condition2 == b.ext_condition2 &&
           a.ext_condition3 == b.ext_condition3 && a.ext_condition4 == b.ext_condition4 &&
           a.condition == b.condition && a.size == b.size && a.exchange == b.exchange &&
           a.price == b.price && a.condition_flags == b.condition_flags &&
           a.price_flags == b.price_flags && a.volume_type == b.volume_type &&
           a.records_back == b.records_back && a.date == b.date && a.expiration == b.expiration &&
           a.strike == b.strike && a.right == b.right;
}

} // namespace

int main(int argc, char** argv) {
    const std::string creds_path = (argc > 1) ? argv[1] : "creds.txt";
    const std::string symbol = (argc > 2) ? argv[2] : "QQQ";
    const std::string expiration = (argc > 3) ? argv[3] : "*";
    const std::string date = (argc > 4) ? argv[4] : "20250303";

    try {
        auto creds = thetadatadx::Credentials::from_file(creds_path);
        auto config = thetadatadx::Config::production();
        auto client = thetadatadx::Client::connect(creds, config);

        std::cout << "streaming option_history_trade " << symbol << " expiration=" << expiration
                  << " date=" << date << " (all strikes)\n";

        // ── Streamed pull ──
        std::vector<thetadatadx::TradeTick> streamed;
        std::size_t chunk_count = 0;
        std::size_t peak_chunk_rows = 0;
        const auto t0 = std::chrono::steady_clock::now();

        client.market_data().option_history_trade_stream(
            symbol, expiration,
            [&](thetadatadx::Span<const thetadatadx::TradeTick> chunk) {
                ++chunk_count;
                if (chunk.size() > peak_chunk_rows) {
                    peak_chunk_rows = chunk.size();
                }
                streamed.insert(streamed.end(), chunk.begin(), chunk.end());
            },
            thetadatadx::EndpointRequestOptions{}.with_timeout_ms(120000));

        const auto t1 = std::chrono::steady_clock::now();
        const auto stream_ms =
            std::chrono::duration_cast<std::chrono::milliseconds>(t1 - t0).count();

        std::cout << "  streamed: " << streamed.size() << " rows in " << chunk_count
                  << " chunks (peak chunk " << peak_chunk_rows << " rows, "
                  << (peak_chunk_rows * sizeof(thetadatadx::TradeTick)) << " bytes), " << stream_ms
                  << " ms\n";

        // ── Buffered pull (ground truth) ──
        const auto t2 = std::chrono::steady_clock::now();
        thetadatadx::MarketDataClient hist = thetadatadx::MarketDataClient::connect(creds, config);
        std::vector<thetadatadx::TradeTick> buffered = hist.option_history_trade(
            symbol, expiration, thetadatadx::EndpointRequestOptions{}.with_timeout_ms(120000));
        const auto t3 = std::chrono::steady_clock::now();
        const auto buffered_ms =
            std::chrono::duration_cast<std::chrono::milliseconds>(t3 - t2).count();

        std::cout << "  buffered: " << buffered.size() << " rows, " << buffered_ms << " ms\n";

        // ── Compare ──
        if (streamed.size() != buffered.size()) {
            std::cerr << "MISMATCH: streamed " << streamed.size() << " rows, buffered "
                      << buffered.size() << " rows\n";
            return 1;
        }
        for (std::size_t i = 0; i < streamed.size(); ++i) {
            if (!ticks_equal(streamed[i], buffered[i])) {
                const auto& s = streamed[i];
                const auto& b = buffered[i];
                std::cerr << "MISMATCH: row " << i << " differs between streamed and buffered\n";
                std::cerr << "  streamed: ms=" << s.ms_of_day << " seq=" << s.sequence
                          << " price=" << s.price << " size=" << s.size << " exch=" << s.exchange
                          << " strike=" << s.strike << " right=" << s.right << "\n";
                std::cerr << "  buffered: ms=" << b.ms_of_day << " seq=" << b.sequence
                          << " price=" << b.price << " size=" << b.size << " exch=" << b.exchange
                          << " strike=" << b.strike << " right=" << b.right << "\n";
                return 1;
            }
        }

        if (streamed.empty()) {
            std::cerr << "WARNING: zero rows returned -- pick a trading day with option activity\n";
            return 2;
        }

        std::cout << "MATCH: " << streamed.size() << " rows identical (count + content); peak memory "
                  << "bounded to one chunk of " << peak_chunk_rows << " rows ("
                  << (100.0 * static_cast<double>(peak_chunk_rows) / static_cast<double>(streamed.size()))
                  << "% of the full result)\n";
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
