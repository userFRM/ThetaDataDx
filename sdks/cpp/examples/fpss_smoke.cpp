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

#include "thetadatadx.hpp"

namespace {

constexpr const char* kSymbol = "AAPL";
constexpr const char* kOptionSymbol = "SPY";
constexpr const char* kExpiration = "20260417";
constexpr const char* kStrike = "550";
constexpr const char* kRight = "C";

constexpr auto kCollectFor = std::chrono::seconds(5);
constexpr int kMaxEventsPrinted = 25;

// Replaced the flat `THETADATADX_STREAM_CONTROL` discriminant with
// one kind per typed `StreamControl::*` variant. Returning a friendly
// short name keeps the smoke test's stdout compact.
const char* event_kind_name(thetadatadx::StreamEventKind kind) {
    switch (kind) {
        case THETADATADX_STREAM_QUOTE: return "quote";
        case THETADATADX_STREAM_TRADE: return "trade";
        case THETADATADX_STREAM_OPEN_INTEREST: return "open_interest";
        case THETADATADX_STREAM_OHLCVC: return "ohlcvc";
        case THETADATADX_STREAM_CONNECTED: return "connected";
        case THETADATADX_STREAM_CONTRACT_ASSIGNED: return "contract_assigned";
        case THETADATADX_STREAM_DISCONNECTED: return "disconnected";
        case THETADATADX_STREAM_PARSE_ERROR: return "parse_error";
        case THETADATADX_STREAM_LOGIN_SUCCESS: return "login_success";
        case THETADATADX_STREAM_MARKET_CLOSE: return "market_close";
        case THETADATADX_STREAM_MARKET_OPEN: return "market_open";
        case THETADATADX_STREAM_PING: return "ping";
        case THETADATADX_STREAM_RECONNECTED: return "reconnected";
        case THETADATADX_STREAM_RECONNECTED_SERVER: return "reconnected_server";
        case THETADATADX_STREAM_RECONNECTING: return "reconnecting";
        case THETADATADX_STREAM_REQ_RESPONSE: return "req_response";
        case THETADATADX_STREAM_RESTART: return "restart";
        case THETADATADX_STREAM_SERVER_ERROR: return "server_error";
        case THETADATADX_STREAM_UNKNOWN_CONTROL: return "unknown_control";
        case THETADATADX_STREAM_UNKNOWN_FRAME: return "unknown_frame";
    }
    return "unknown";
}

// Returns true when the kind is one of the typed control variants
// (everything except the four data variants).
bool is_control_kind(thetadatadx::StreamEventKind kind) {
    switch (kind) {
        case THETADATADX_STREAM_QUOTE:
        case THETADATADX_STREAM_TRADE:
        case THETADATADX_STREAM_OPEN_INTEREST:
        case THETADATADX_STREAM_OHLCVC:
            return false;
        default:
            return true;
    }
}

} // namespace

int main(int argc, char** argv) {
    const std::string creds_path = (argc > 1) ? argv[1] : "creds.txt";
    try {
        auto creds = thetadatadx::Credentials::from_file(creds_path);
        auto config = thetadatadx::Config::production();

        thetadatadx::StreamingClient fpss(creds, config);

        std::atomic<int> total_events{0};
        std::atomic<int> data_events{0};
        std::mutex print_mtx;

        fpss.set_callback([&](const thetadatadx::StreamEvent& event) {
            const int seq = total_events.fetch_add(1, std::memory_order_relaxed);
            if (!is_control_kind(event.kind)) {
                data_events.fetch_add(1, std::memory_order_relaxed);
            }
            if (seq >= kMaxEventsPrinted) return;
            std::lock_guard<std::mutex> guard(print_mtx);
            std::cout << "[" << seq << "] kind=" << event_kind_name(event.kind);
            switch (event.kind) {
                case THETADATADX_STREAM_QUOTE:
                    std::cout << " symbol="
                              << (event.quote.contract.symbol ? event.quote.contract.symbol : "")
                              << " bid=" << event.quote.bid
                              << " ask=" << event.quote.ask;
                    break;
                case THETADATADX_STREAM_TRADE:
                    std::cout << " symbol="
                              << (event.trade.contract.symbol ? event.trade.contract.symbol : "")
                              << " price=" << event.trade.price
                              << " size=" << event.trade.size;
                    break;
                case THETADATADX_STREAM_OPEN_INTEREST:
                    std::cout << " symbol="
                              << (event.open_interest.contract.symbol
                                      ? event.open_interest.contract.symbol
                                      : "")
                              << " open_interest=" << event.open_interest.open_interest;
                    break;
                case THETADATADX_STREAM_OHLCVC:
                    std::cout << " symbol="
                              << (event.ohlcvc.contract.symbol ? event.ohlcvc.contract.symbol : "")
                              << " close=" << event.ohlcvc.close;
                    break;
                case THETADATADX_STREAM_LOGIN_SUCCESS:
                    if (event.login_success.permissions) {
                        std::cout << " permissions=" << event.login_success.permissions;
                    }
                    break;
                case THETADATADX_STREAM_CONTRACT_ASSIGNED:
                    std::cout << " id=" << event.contract_assigned.id;
                    if (event.contract_assigned.contract.symbol) {
                        std::cout << " symbol=" << event.contract_assigned.contract.symbol;
                    }
                    break;
                case THETADATADX_STREAM_REQ_RESPONSE:
                    std::cout << " req_id=" << event.req_response.req_id
                              << " result=" << event.req_response.result;
                    break;
                case THETADATADX_STREAM_DISCONNECTED:
                    std::cout << " reason=" << event.disconnected.reason;
                    break;
                case THETADATADX_STREAM_RECONNECTING:
                    std::cout << " reason=" << event.reconnecting.reason
                              << " attempt=" << event.reconnecting.attempt
                              << " delay_ms=" << event.reconnecting.delay_ms;
                    break;
                case THETADATADX_STREAM_SERVER_ERROR:
                    if (event.server_error.message) {
                        std::cout << " message=" << event.server_error.message;
                    }
                    break;
                case THETADATADX_STREAM_PARSE_ERROR:
                    if (event.parse_error.message) {
                        std::cout << " message=" << event.parse_error.message;
                    }
                    break;
                case THETADATADX_STREAM_UNKNOWN_FRAME:
                    std::cout << " code=" << static_cast<int>(event.unknown_frame.code)
                              << " len=" << event.unknown_frame.payload_len;
                    break;
                case THETADATADX_STREAM_PING:
                    std::cout << " len=" << event.ping.payload_len;
                    break;
                case THETADATADX_STREAM_MARKET_OPEN:
                case THETADATADX_STREAM_MARKET_CLOSE:
                case THETADATADX_STREAM_CONNECTED:
                case THETADATADX_STREAM_RECONNECTED:
                case THETADATADX_STREAM_RECONNECTED_SERVER:
                case THETADATADX_STREAM_RESTART:
                case THETADATADX_STREAM_UNKNOWN_CONTROL:
                    // Payload-less variants — discriminator alone carries
                    // the meaning.
                    break;
            }
            std::cout << std::endl;
        });

        // Fluent contract-first subscriptions.
        auto stock = thetadatadx::Contract::stock(kSymbol);
        auto option = thetadatadx::Contract::option(
            kOptionSymbol, {.expiration = kExpiration, .strike = kStrike, .right = kRight});
        fpss.subscribe(stock.quote());
        fpss.subscribe(stock.trade());
        fpss.subscribe(option.quote());

        std::this_thread::sleep_for(kCollectFor);

        const int total = total_events.load(std::memory_order_relaxed);
        const int data = data_events.load(std::memory_order_relaxed);
        const uint64_t dropped = fpss.dropped_event_count();

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
