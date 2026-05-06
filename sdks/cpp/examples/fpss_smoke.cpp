// fpss_smoke.cpp -- C++ FPSS smoke test driven by the callback C ABI.
//
// Subscribes to a stock and an option contract, registers a queued
// callback, prints every event for a few seconds, then exits cleanly.

#include <atomic>
#include <chrono>
#include <iostream>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>

#include "thetadx.hpp"

namespace {

constexpr const char* kSymbol = "AAPL";
constexpr const char* kOptionSymbol = "SPY";
constexpr const char* kExpiration = "20260417";
constexpr const char* kStrike = "550";
constexpr const char* kRight = "C";

constexpr auto kCollectFor = std::chrono::seconds(5);
constexpr int kMaxEventsPrinted = 25;

const char* event_kind_name(tdx::FpssEventKind kind) {
    switch (kind) {
        case TDX_FPSS_QUOTE: return "quote";
        case TDX_FPSS_TRADE: return "trade";
        case TDX_FPSS_OPEN_INTEREST: return "open_interest";
        case TDX_FPSS_OHLCVC: return "ohlcvc";
        case TDX_FPSS_CONTROL: return "control";
        case TDX_FPSS_RAW_DATA: return "raw_data";
    }
    return "unknown";
}

} // namespace

int main(int argc, char** argv) {
    const std::string creds_path = (argc > 1) ? argv[1] : "creds.txt";
    try {
        auto creds = tdx::Credentials::from_file(creds_path);
        auto config = tdx::Config::production();

        tdx::FpssClient fpss(creds, config);

        std::atomic<int> total_events{0};
        std::atomic<int> data_events{0};
        std::mutex print_mtx;

        fpss.set_callback([&](const tdx::FpssEvent& event) {
            const int seq = total_events.fetch_add(1, std::memory_order_relaxed);
            if (event.kind != TDX_FPSS_CONTROL && event.kind != TDX_FPSS_RAW_DATA) {
                data_events.fetch_add(1, std::memory_order_relaxed);
            }
            if (seq >= kMaxEventsPrinted) return;
            std::lock_guard<std::mutex> guard(print_mtx);
            std::cout << "[" << seq << "] kind=" << event_kind_name(event.kind);
            switch (event.kind) {
                case TDX_FPSS_QUOTE:
                    std::cout << " contract_id=" << event.quote.contract_id
                              << " bid=" << event.quote.bid
                              << " ask=" << event.quote.ask;
                    break;
                case TDX_FPSS_TRADE:
                    std::cout << " contract_id=" << event.trade.contract_id
                              << " price=" << event.trade.price
                              << " size=" << event.trade.size;
                    break;
                case TDX_FPSS_OPEN_INTEREST:
                    std::cout << " contract_id=" << event.open_interest.contract_id
                              << " open_interest=" << event.open_interest.open_interest;
                    break;
                case TDX_FPSS_OHLCVC:
                    std::cout << " contract_id=" << event.ohlcvc.contract_id
                              << " close=" << event.ohlcvc.close;
                    break;
                case TDX_FPSS_CONTROL:
                    std::cout << " control_kind=" << event.control.kind;
                    if (event.control.detail) std::cout << " detail=" << event.control.detail;
                    break;
                case TDX_FPSS_RAW_DATA:
                    std::cout << " code=" << static_cast<int>(event.raw_data.code)
                              << " len=" << event.raw_data.payload_len;
                    break;
            }
            std::cout << std::endl;
        });

        if (fpss.subscribe_quotes(kSymbol) < 0) {
            throw std::runtime_error("subscribe_quotes failed");
        }
        if (fpss.subscribe_trades(kSymbol) < 0) {
            throw std::runtime_error("subscribe_trades failed");
        }
        if (fpss.subscribe_option_quotes(kOptionSymbol, kExpiration, kStrike, kRight) < 0) {
            throw std::runtime_error("subscribe_option_quotes failed");
        }

        std::this_thread::sleep_for(kCollectFor);

        const int total = total_events.load(std::memory_order_relaxed);
        const int data = data_events.load(std::memory_order_relaxed);
        const uint64_t dropped = fpss.dropped_events();

        std::cout << "summary: total=" << total
                  << " data=" << data
                  << " dropped=" << dropped << std::endl;

        fpss.shutdown();
        return data > 0 ? 0 : 1;
    } catch (const std::exception& e) {
        std::cerr << "fpss_smoke error: " << e.what() << std::endl;
        return 2;
    }
}
