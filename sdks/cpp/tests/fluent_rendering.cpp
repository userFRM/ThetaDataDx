// Offline tests for the fluent value-type string rendering and the
// streaming-contract strike accessor.
//
//   * `operator<<` / `thetadatadx::str(...)` on FluentContract / FluentSubscription
//     / FluentSecType give C++ the string rendering Python exposes through
//     `__repr__` / `__str__` and TypeScript through `toString()`.
//   * `thetadatadx_contract_strike_dollars` folds the `has_strike` presence flag
//     of a streaming `ThetaDataDxContract` into a single accessor, mirroring the
//     C++ `thetadatadx::strike(...)` helper and the Python / TypeScript
//     `contract.strike` surface.

#include <sstream>
#include <string>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.hpp"

namespace {

std::string render(const thetadatadx::FluentContract& c) {
    std::ostringstream os;
    os << c;
    return os.str();
}

} // namespace

TEST_CASE("FluentContract renders symbol and option identity", "[fluent][offline]") {
    const auto stock = thetadatadx::Contract::stock("AAPL");
    REQUIRE(render(stock) == "AAPL STOCK");
    REQUIRE(thetadatadx::str(stock) == "AAPL STOCK");

    const auto option = thetadatadx::Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"});
    REQUIRE(render(option) == "SPY OPTION 20260620 C 550");
    REQUIRE(thetadatadx::str(option) == "SPY OPTION 20260620 C 550");

    // Fractional strikes keep the needed decimals — same rendering as the
    // Rust `Display` / Python `__str__` / TypeScript `toString` surface.
    const auto fractional = thetadatadx::Contract::option("SPY", {.expiration = "20260620", .strike = "552.5", .right = "P"});
    REQUIRE(render(fractional) == "SPY OPTION 20260620 P 552.5");

    // An index contract renders "INDEX", not "STOCK" — it carries its
    // own security type rather than borrowing the stock-shape default.
    const auto index = thetadatadx::Contract::index("VIX");
    REQUIRE(render(index) == "VIX INDEX");
    REQUIRE(thetadatadx::str(index) == "VIX INDEX");
    REQUIRE(index.sec_type() == "INDEX");
    // The index identity also carries into a per-contract subscription.
    REQUIRE(thetadatadx::str(index.trade()) == "Subscription(Trade, VIX INDEX)");
}

TEST_CASE("FluentSecType renders its symbolic name", "[fluent][offline]") {
    std::ostringstream os;
    os << thetadatadx::SecType::option();
    REQUIRE(os.str() == "OPTION");
    REQUIRE(thetadatadx::str(thetadatadx::SecType::stock()) == "STOCK");
    REQUIRE(thetadatadx::str(thetadatadx::SecType::index()) == "INDEX");
    // `rate()` matches the Python `SecType.RATE` / TypeScript
    // `SecType.rate()` constructor.
    REQUIRE(thetadatadx::str(thetadatadx::SecType::rate()) == "RATE");
}

TEST_CASE("FluentSubscription renders scope, kind, and contract", "[fluent][offline]") {
    const auto per_contract = thetadatadx::Contract::option("SPY", {.expiration = "20260620", .strike = "550", .right = "C"}).trade();
    REQUIRE(thetadatadx::str(per_contract) == "Subscription(Trade, SPY OPTION 20260620 C 550)");

    const auto stock_quote = thetadatadx::Contract::stock("AAPL").quote();
    REQUIRE(thetadatadx::str(stock_quote) == "Subscription(Quote, AAPL STOCK)");

    const auto market_value = thetadatadx::Contract::stock("AAPL").market_value();
    REQUIRE(thetadatadx::str(market_value) == "Subscription(MarketValue, AAPL STOCK)");

    const auto full = thetadatadx::SecType::option().full_open_interest();
    REQUIRE(thetadatadx::str(full) == "Subscription(full OpenInterest, OPTION)");
}

TEST_CASE("FluentSubscription::kind_string is snake_case", "[fluent][offline]") {
    // The snake_case kind label matches the Python / TypeScript
    // `Subscription.kind` accessor and the C ABI active-subscription
    // `kind` field — distinct from the PascalCase `operator<<` rendering.
    REQUIRE(thetadatadx::Contract::stock("AAPL").quote().kind_string() == "quote");
    REQUIRE(thetadatadx::Contract::stock("AAPL").trade().kind_string() == "trade");
    REQUIRE(thetadatadx::Contract::stock("AAPL").open_interest().kind_string() == "open_interest");
    REQUIRE(thetadatadx::Contract::stock("AAPL").market_value().kind_string() == "market_value");

    // Full-stream kinds carry the `full_` prefix, so a full-stream
    // open-interest never reads the same as a per-contract one.
    REQUIRE(thetadatadx::SecType::option().full_trades().kind_string() == "full_trades");
    REQUIRE(thetadatadx::SecType::option().full_open_interest().kind_string() == "full_open_interest");
    REQUIRE(thetadatadx::Contract::stock("AAPL").open_interest().kind_string()
            != thetadatadx::SecType::option().full_open_interest().kind_string());
}

TEST_CASE("thetadatadx_contract_strike_dollars folds the presence flag", "[fluent][offline]") {
    ThetaDataDxContract option{};
    option.sec_type = 1; // OPTION
    option.has_strike = true;
    option.strike = 550.0;
    double dollars = 0.0;
    REQUIRE(thetadatadx_contract_strike_dollars(&option, &dollars));
    REQUIRE(dollars == 550.0);

    // The C++ `thetadatadx::strike(...)` accessor agrees with the C function.
    const auto via_cpp = thetadatadx::strike(option);
    REQUIRE(via_cpp.has_value());
    REQUIRE(*via_cpp == 550.0);

    ThetaDataDxContract stock{};
    stock.sec_type = 0; // STOCK
    stock.has_strike = false;
    double untouched = -1.0;
    REQUIRE_FALSE(thetadatadx_contract_strike_dollars(&stock, &untouched));
    REQUIRE(untouched == -1.0); // left untouched on absence
    REQUIRE_FALSE(thetadatadx::strike(stock).has_value());

    // Null guards.
    REQUIRE_FALSE(thetadatadx_contract_strike_dollars(nullptr, &dollars));
    REQUIRE_FALSE(thetadatadx_contract_strike_dollars(&option, nullptr));
}
