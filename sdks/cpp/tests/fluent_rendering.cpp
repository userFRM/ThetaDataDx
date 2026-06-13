// Offline tests for the fluent value-type string rendering and the
// streaming-contract strike accessor.
//
//   * `operator<<` / `tdx::str(...)` on FluentContract / FluentSubscription
//     / FluentSecType give C++ the string rendering Python exposes through
//     `__repr__` / `__str__` and TypeScript through `toString()`.
//   * `tdx_contract_strike_dollars` folds the `has_strike` presence flag
//     of a streaming `TdxContract` into a single accessor, mirroring the
//     C++ `tdx::strike(...)` helper and the Python / TypeScript
//     `contract.strike` surface.

#include <sstream>
#include <string>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string render(const tdx::FluentContract& c) {
    std::ostringstream os;
    os << c;
    return os.str();
}

} // namespace

TEST_CASE("FluentContract renders symbol and option identity", "[fluent][offline]") {
    const auto stock = tdx::Contract::stock("AAPL");
    REQUIRE(render(stock) == "AAPL STOCK");
    REQUIRE(tdx::str(stock) == "AAPL STOCK");

    const auto option = tdx::Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"});
    REQUIRE(render(option) == "SPY OPTION 20260620 C 550");
    REQUIRE(tdx::str(option) == "SPY OPTION 20260620 C 550");

    // An index contract renders "INDEX", not "STOCK" — it carries its
    // own security type rather than borrowing the stock-shape default.
    const auto index = tdx::Contract::index("VIX");
    REQUIRE(render(index) == "VIX INDEX");
    REQUIRE(tdx::str(index) == "VIX INDEX");
    REQUIRE(index.sec_type() == "INDEX");
    // The index identity also carries into a per-contract subscription.
    REQUIRE(tdx::str(index.trade()) == "Subscription(Trade, VIX INDEX)");
}

TEST_CASE("FluentSecType renders its symbolic name", "[fluent][offline]") {
    std::ostringstream os;
    os << tdx::SecType::option();
    REQUIRE(os.str() == "OPTION");
    REQUIRE(tdx::str(tdx::SecType::stock()) == "STOCK");
    REQUIRE(tdx::str(tdx::SecType::index()) == "INDEX");
    // `rate()` matches the Python `SecType.RATE` / TypeScript
    // `SecType.rate()` constructor.
    REQUIRE(tdx::str(tdx::SecType::rate()) == "RATE");
}

TEST_CASE("FluentSubscription renders scope, kind, and contract", "[fluent][offline]") {
    const auto per_contract = tdx::Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"}).trade();
    REQUIRE(tdx::str(per_contract) == "Subscription(Trade, SPY OPTION 20260620 C 550)");

    const auto stock_quote = tdx::Contract::stock("AAPL").quote();
    REQUIRE(tdx::str(stock_quote) == "Subscription(Quote, AAPL STOCK)");

    const auto market_value = tdx::Contract::stock("AAPL").market_value();
    REQUIRE(tdx::str(market_value) == "Subscription(MarketValue, AAPL STOCK)");

    const auto full = tdx::SecType::option().full_open_interest();
    REQUIRE(tdx::str(full) == "Subscription(full OpenInterest, OPTION)");
}

TEST_CASE("FluentSubscription::kind_string is snake_case", "[fluent][offline]") {
    // The snake_case kind label matches the Python / TypeScript
    // `Subscription.kind` accessor and the C ABI active-subscription
    // `kind` field — distinct from the PascalCase `operator<<` rendering.
    REQUIRE(tdx::Contract::stock("AAPL").quote().kind_string() == "quote");
    REQUIRE(tdx::Contract::stock("AAPL").trade().kind_string() == "trade");
    REQUIRE(tdx::Contract::stock("AAPL").open_interest().kind_string() == "open_interest");
    REQUIRE(tdx::Contract::stock("AAPL").market_value().kind_string() == "market_value");

    // Full-stream kinds carry the `full_` prefix, so a full-stream
    // open-interest never reads the same as a per-contract one.
    REQUIRE(tdx::SecType::option().full_trades().kind_string() == "full_trades");
    REQUIRE(tdx::SecType::option().full_open_interest().kind_string() == "full_open_interest");
    REQUIRE(tdx::Contract::stock("AAPL").open_interest().kind_string()
            != tdx::SecType::option().full_open_interest().kind_string());
}

TEST_CASE("tdx_contract_strike_dollars folds the presence flag", "[fluent][offline]") {
    TdxContract option{};
    option.sec_type = 1; // OPTION
    option.has_strike = true;
    option.strike = 550.0;
    double dollars = 0.0;
    REQUIRE(tdx_contract_strike_dollars(&option, &dollars));
    REQUIRE(dollars == 550.0);

    // The C++ `tdx::strike(...)` accessor agrees with the C function.
    const auto via_cpp = tdx::strike(option);
    REQUIRE(via_cpp.has_value());
    REQUIRE(*via_cpp == 550.0);

    TdxContract stock{};
    stock.sec_type = 0; // STOCK
    stock.has_strike = false;
    double untouched = -1.0;
    REQUIRE_FALSE(tdx_contract_strike_dollars(&stock, &untouched));
    REQUIRE(untouched == -1.0); // left untouched on absence
    REQUIRE_FALSE(tdx::strike(stock).has_value());

    // Null guards.
    REQUIRE_FALSE(tdx_contract_strike_dollars(nullptr, &dollars));
    REQUIRE_FALSE(tdx_contract_strike_dollars(&option, nullptr));
}
