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

// Replaced the flat `TDX_FPSS_CONTROL` discriminant with
// one kind per typed `FpssControl::*` variant. Returning a friendly
// short name keeps the smoke test's stdout compact.
const char* event_kind_name(tdx::FpssEventKind kind) {
    switch (kind) {
        case TDX_FPSS_QUOTE: return "quote";
        case TDX_FPSS_TRADE: return "trade";
        case TDX_FPSS_OPEN_INTEREST: return "open_interest";
        case TDX_FPSS_OHLCVC: return "ohlcvc";
        case TDX_FPSS_CONNECTED: return "connected";
        case TDX_FPSS_CONTRACT_ASSIGNED: return "contract_assigned";
        case TDX_FPSS_DISCONNECTED: return "disconnected";
        case TDX_FPSS_PARSE_ERROR: return "parse_error";
        case TDX_FPSS_LOGIN_SUCCESS: return "login_success";
        case TDX_FPSS_MARKET_CLOSE: return "market_close";
        case TDX_FPSS_MARKET_OPEN: return "market_open";
        case TDX_FPSS_PING: return "ping";
        case TDX_FPSS_RECONNECTED: return "reconnected";
        case TDX_FPSS_RECONNECTED_SERVER: return "reconnected_server";
        case TDX_FPSS_RECONNECTING: return "reconnecting";
        case TDX_FPSS_REQ_RESPONSE: return "req_response";
        case TDX_FPSS_RESTART: return "restart";
        case TDX_FPSS_SERVER_ERROR: return "server_error";
        case TDX_FPSS_UNKNOWN_CONTROL: return "unknown_control";
        case TDX_FPSS_UNKNOWN_FRAME: return "unknown_frame";
    }
    return "unknown";
}

// Returns true when the kind is one of the typed control variants
// (everything except the four data variants).
bool is_control_kind(tdx::FpssEventKind kind) {
    switch (kind) {
        case TDX_FPSS_QUOTE:
        case TDX_FPSS_TRADE:
        case TDX_FPSS_OPEN_INTEREST:
        case TDX_FPSS_OHLCVC:
            return false;
        default:
            return true;
    }
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
            if (!is_control_kind(event.kind)) {
                data_events.fetch_add(1, std::memory_order_relaxed);
            }
            if (seq >= kMaxEventsPrinted) return;
            std::lock_guard<std::mutex> guard(print_mtx);
            std::cout << "[" << seq << "] kind=" << event_kind_name(event.kind);
            switch (event.kind) {
                case TDX_FPSS_QUOTE:
                    std::cout << " symbol="
                              << (event.quote.contract.symbol ? event.quote.contract.symbol : "")
                              << " bid=" << event.quote.bid
                              << " ask=" << event.quote.ask;
                    break;
                case TDX_FPSS_TRADE:
                    std::cout << " symbol="
                              << (event.trade.contract.symbol ? event.trade.contract.symbol : "")
                              << " price=" << event.trade.price
                              << " size=" << event.trade.size;
                    break;
                case TDX_FPSS_OPEN_INTEREST:
                    std::cout << " symbol="
                              << (event.open_interest.contract.symbol
                                      ? event.open_interest.contract.symbol
                                      : "")
                              << " open_interest=" << event.open_interest.open_interest;
                    break;
                case TDX_FPSS_OHLCVC:
                    std::cout << " symbol="
                              << (event.ohlcvc.contract.symbol ? event.ohlcvc.contract.symbol : "")
                              << " close=" << event.ohlcvc.close;
                    break;
                case TDX_FPSS_LOGIN_SUCCESS:
                    if (event.login_success.permissions) {
                        std::cout << " permissions=" << event.login_success.permissions;
                    }
                    break;
                case TDX_FPSS_CONTRACT_ASSIGNED:
                    std::cout << " id=" << event.contract_assigned.id;
                    if (event.contract_assigned.contract.symbol) {
                        std::cout << " symbol=" << event.contract_assigned.contract.symbol;
                    }
                    break;
                case TDX_FPSS_REQ_RESPONSE:
                    std::cout << " req_id=" << event.req_response.req_id
                              << " result=" << event.req_response.result;
                    break;
                case TDX_FPSS_DISCONNECTED:
                    std::cout << " reason=" << event.disconnected.reason;
                    break;
                case TDX_FPSS_RECONNECTING:
                    std::cout << " reason=" << event.reconnecting.reason
                              << " attempt=" << event.reconnecting.attempt
                              << " delay_ms=" << event.reconnecting.delay_ms;
                    break;
                case TDX_FPSS_SERVER_ERROR:
                    if (event.server_error.message) {
                        std::cout << " message=" << event.server_error.message;
                    }
                    break;
                case TDX_FPSS_PARSE_ERROR:
                    if (event.parse_error.message) {
                        std::cout << " message=" << event.parse_error.message;
                    }
                    break;
                case TDX_FPSS_UNKNOWN_FRAME:
                    std::cout << " code=" << static_cast<int>(event.unknown_frame.code)
                              << " len=" << event.unknown_frame.payload_len;
                    break;
                case TDX_FPSS_PING:
                    std::cout << " len=" << event.ping.payload_len;
                    break;
                case TDX_FPSS_MARKET_OPEN:
                case TDX_FPSS_MARKET_CLOSE:
                case TDX_FPSS_CONNECTED:
                case TDX_FPSS_RECONNECTED:
                case TDX_FPSS_RECONNECTED_SERVER:
                case TDX_FPSS_RESTART:
                case TDX_FPSS_UNKNOWN_CONTROL:
                    // Payload-less variants — discriminator alone carries
                    // the meaning.
                    break;
            }
            std::cout << std::endl;
        });

        // Fluent contract-first subscriptions.
        auto stock = tdx::Contract::stock(kSymbol);
        auto option = tdx::Contract::option(kOptionSymbol, kExpiration, kStrike, kRight);
        fpss.subscribe(stock.quote());
        fpss.subscribe(stock.trade());
        fpss.subscribe(option.quote());

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
