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
}

TEST_CASE("FluentSecType renders its symbolic name", "[fluent][offline]") {
    std::ostringstream os;
    os << tdx::SecType::option();
    REQUIRE(os.str() == "OPTION");
    REQUIRE(tdx::str(tdx::SecType::stock()) == "STOCK");
}

TEST_CASE("FluentSubscription renders scope, kind, and contract", "[fluent][offline]") {
    const auto per_contract = tdx::Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"}).trade();
    REQUIRE(tdx::str(per_contract) == "Subscription(Trade, SPY OPTION 20260620 C 550)");

    const auto stock_quote = tdx::Contract::stock("AAPL").quote();
    REQUIRE(tdx::str(stock_quote) == "Subscription(Quote, AAPL STOCK)");

    const auto full = tdx::SecType::option().full_open_interest();
    REQUIRE(tdx::str(full) == "Subscription(full OpenInterest, OPTION)");
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
