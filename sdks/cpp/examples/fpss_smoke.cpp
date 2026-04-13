#include <chrono>
#include <iostream>
#include <set>
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

std::set<std::string> subscriptions_snapshot(const tdx::FpssClient& fpss) {
    std::set<std::string> out;
    for (const auto& sub : fpss.active_subscriptions()) {
        out.insert(sub.kind + "|" + sub.contract);
    }
    return out;
}

std::pair<int32_t, std::string> require_data_event(tdx::FpssClient& fpss, std::chrono::seconds timeout) {
    const auto deadline = std::chrono::steady_clock::now() + timeout;
    std::string last_kind = "none";
    std::string last_control;
    while (std::chrono::steady_clock::now() < deadline) {
        auto event = fpss.next_event(500);
        if (!event) {
            continue;
        }
        switch (event->kind) {
            case TDX_FPSS_QUOTE:
                last_kind = "quote";
                return {event->quote.contract_id, last_kind};
            case TDX_FPSS_TRADE:
                last_kind = "trade";
                return {event->trade.contract_id, last_kind};
            case TDX_FPSS_OPEN_INTEREST:
                last_kind = "open_interest";
                return {event->open_interest.contract_id, last_kind};
            case TDX_FPSS_OHLCVC:
                last_kind = "ohlcvc";
                return {event->ohlcvc.contract_id, last_kind};
            case TDX_FPSS_CONTROL:
                last_kind = "control";
                last_control = "kind=" + std::to_string(event->control.kind);
                if (event->control.detail != nullptr) {
                    last_control += " detail=";
                    last_control += event->control.detail;
                }
                break;
            case TDX_FPSS_RAW_DATA:
                last_kind = "raw_data";
                break;
        }
    }
    std::string message = "timed out waiting for FPSS data event (last kind=" + last_kind;
    if (!last_control.empty()) {
        message += ", last control=" + last_control;
    }
    message += ")";
    throw std::runtime_error(message);
}

}  // namespace

int main(int argc, char** argv) {
    try {
        const std::string creds_path = argc > 1 ? argv[1] : "creds.txt";
        auto creds = tdx::Credentials::from_file(creds_path);
        auto config = tdx::Config::dev();
        config.set_reconnect_policy(1);
        config.set_derive_ohlcvc(false);

        tdx::FpssClient fpss(creds, config);
        if (fpss.subscribe_quotes(kSymbol) < 0) {
            throw std::runtime_error("subscribe_quotes failed");
        }
        if (fpss.subscribe_trades(kSymbol) < 0) {
            throw std::runtime_error("subscribe_trades failed");
        }
        if (fpss.subscribe_option_quotes(kOptionSymbol, kExpiration, kStrike, kRight) < 0) {
            throw std::runtime_error("subscribe_option_quotes failed");
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(250));

        const auto expected = subscriptions_snapshot(fpss);
        if (expected.size() < 3) {
            throw std::runtime_error("expected at least 3 active subscriptions");
        }

        auto [contract_id, first_kind] = require_data_event(fpss, std::chrono::seconds(60));
        if (contract_id != 0) {
            const auto contract = fpss.contract_lookup(contract_id);
            if (!contract || contract->empty()) {
                throw std::runtime_error("contract_lookup failed after first " + first_kind + " event");
            }
        }

        const auto contract_map = fpss.contract_map();
        if (contract_map.empty()) {
            throw std::runtime_error("contract_map returned no entries after first data event");
        }

        fpss.reconnect();

        const auto after = subscriptions_snapshot(fpss);
        if (after != expected) {
            throw std::runtime_error("subscriptions drifted across reconnect");
        }

        auto [contract_id_after, second_kind] = require_data_event(fpss, std::chrono::seconds(60));
        if (contract_id_after != 0) {
            const auto contract = fpss.contract_lookup(contract_id_after);
            if (!contract || contract->empty()) {
                throw std::runtime_error("contract_lookup failed after reconnect " + second_kind + " event");
            }
        }

        fpss.shutdown();
        std::cout << "cpp fpss smoke: ok (symbol=" << kSymbol << ", option=" << kOptionSymbol << " "
                  << kExpiration << " " << kStrike << " " << kRight << ")" << std::endl;
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "fpss smoke failed: " << e.what() << std::endl;
        return 1;
    }
}
