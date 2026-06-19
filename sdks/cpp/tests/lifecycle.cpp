// C++ SDK lifecycle smoke tests.
//
// Confirms the connect / disconnect path against the production
// server (live-only) and the type-level surface that does not need
// credentials (offline). The live half is gated on
// `THETADATADX_LIVE_CREDS` pointing at a `creds.txt` file with the
// account email on line 1 and the password on line 2.

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <string>
#include <unistd.h>

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

namespace {

std::string env_or_empty(const char* key) {
    const char* raw = std::getenv(key);
    return raw == nullptr ? std::string() : std::string(raw);
}

// Build a unique temp path for the current process without `tmpnam`
// (whose use trips a linker warning). The PID + a per-call counter keeps
// parallel test runners from colliding.
std::string unique_dotenv_path() {
    static int counter = 0;
    return "/tmp/thetadatadx-cpp-dotenv-" + std::to_string(::getpid()) + "-"
           + std::to_string(counter++) + ".env";
}

} // namespace

TEST_CASE("Config::production builds without network access", "[lifecycle][offline]") {
    auto config = thetadatadx::Config::production();
    REQUIRE(config.get() != nullptr);
}

TEST_CASE("Credentials::from_api_key builds a handle without network access",
          "[lifecycle][offline]") {
    auto creds = thetadatadx::Credentials::from_api_key("super-secret-key");
    REQUIRE(creds.get() != nullptr);
}

TEST_CASE("Credentials::from_api_key_with_email builds a handle without network access",
          "[lifecycle][offline]") {
    auto creds = thetadatadx::Credentials::from_api_key_with_email("user@example.com",
                                                                   "super-secret-key");
    REQUIRE(creds.get() != nullptr);
}

TEST_CASE("Credentials::from_env_or_file sources from THETADATA_API_KEY",
          "[lifecycle][offline]") {
    setenv("THETADATA_API_KEY", "env-sourced-key", 1);
    auto creds = thetadatadx::Credentials::from_env_or_file("/nonexistent/creds.txt");
    REQUIRE(creds.get() != nullptr);
    unsetenv("THETADATA_API_KEY");
}

TEST_CASE("Credentials::from_env_or_file falls back to the file when the env is unset",
          "[lifecycle][offline]") {
    unsetenv("THETADATA_API_KEY");
    // No fallback file exists, so the file path must surface an error
    // (a throw) rather than silently building a handle.
    REQUIRE_THROWS(thetadatadx::Credentials::from_env_or_file("/nonexistent/creds.txt"));
}

TEST_CASE("Credentials::from_dotenv reads THETADATA_API_KEY from a .env file",
          "[lifecycle][offline]") {
    const std::string path = unique_dotenv_path();
    {
        std::ofstream out(path);
        out << "# comment\nTHETADATA_API_KEY=\"td_example_key\"\n";
    }
    auto creds = thetadatadx::Credentials::from_dotenv(path);
    REQUIRE(creds.get() != nullptr);
    std::remove(path.c_str());
}

TEST_CASE("Credentials::from_dotenv throws when the file defines no recognized keys",
          "[lifecycle][offline]") {
    const std::string path = unique_dotenv_path();
    {
        std::ofstream out(path);
        out << "OTHER=value\n";
    }
    REQUIRE_THROWS(thetadatadx::Credentials::from_dotenv(path));
    std::remove(path.c_str());
}

TEST_CASE("Config::from_dotenv selects the staging environment from a .env file",
          "[lifecycle][offline]") {
    const std::string path = unique_dotenv_path();
    {
        std::ofstream out(path);
        out << "# select staging\nTHETADATA_MDDS_TYPE=STAGE\n";
    }
    auto config = thetadatadx::Config::from_dotenv(path);
    REQUIRE(config.get() != nullptr);
    // A staging `.env` resolves to the staging historical host, distinct
    // from the production host a prod `.env` (or no selector) yields.
    REQUIRE(config.get_historical_host() == "mdds-stage.thetadata.us");
    std::remove(path.c_str());
}

TEST_CASE("Config::from_dotenv with only an API key yields the production environment",
          "[lifecycle][offline]") {
    const std::string path = unique_dotenv_path();
    {
        std::ofstream out(path);
        out << "THETADATA_API_KEY=td_example_key\n";
    }
    auto config = thetadatadx::Config::from_dotenv(path);
    REQUIRE(config.get() != nullptr);
    // No cluster selector in the file: the prod default stays in force and
    // differs from the staging host a `STAGE` selector would produce.
    REQUIRE(config.get_historical_host() == "mdds-01.thetadata.us");
    std::remove(path.c_str());
}

TEST_CASE("Config setters do not throw on a fresh config handle", "[lifecycle][offline]") {
    auto config = thetadatadx::Config::production();
    REQUIRE_NOTHROW(config.set_reconnect_policy(0));
    REQUIRE_NOTHROW(config.set_reconnect_max_attempts(5));
    REQUIRE_NOTHROW(config.set_reconnect_max_rate_limited_attempts(50));
    REQUIRE_NOTHROW(config.set_reconnect_stable_window_secs(120));
    REQUIRE_NOTHROW(config.set_flush_mode(0));
    REQUIRE_NOTHROW(config.set_derive_ohlcvc(true));
}

TEST_CASE("Config flush_mode / derive_ohlcvc getters round-trip", "[lifecycle][offline]") {
    // The readback getters mirror the Python `Config.flush_mode` /
    // `.derive_ohlcvc` and TypeScript `flushMode` / `deriveOhlcvc`
    // surfaces, so a value set through the C++ wrapper reads back
    // through the same wrapper.
    auto config = thetadatadx::Config::production();

    config.set_flush_mode(1);
    REQUIRE(config.get_flush_mode() == 1);
    config.set_flush_mode(0);
    REQUIRE(config.get_flush_mode() == 0);

    config.set_derive_ohlcvc(false);
    REQUIRE(config.get_derive_ohlcvc() == false);
    config.set_derive_ohlcvc(true);
    REQUIRE(config.get_derive_ohlcvc() == true);
}

TEST_CASE("Config wait_strategy / tuning / consumer_cpu round-trip", "[lifecycle][offline]") {
    // Mirrors the Python `Config.wait_strategy` / `.consumer_cpu` and
    // TypeScript `waitStrategy` / `consumerCpu` surfaces: a value set
    // through the C++ wrapper reads back through the same wrapper.
    auto config = thetadatadx::Config::production();

    // Default preset is LowLatency (preserves the historical behaviour).
    REQUIRE(config.get_wait_strategy() == THETADATADX_WAIT_LOW_LATENCY);

    for (int mode : {THETADATADX_WAIT_LOW_LATENCY, THETADATADX_WAIT_BALANCED,
                     THETADATADX_WAIT_EFFICIENT, THETADATADX_WAIT_BUSY_SPIN}) {
        config.set_wait_strategy(mode);
        REQUIRE(config.get_wait_strategy() == mode);
    }

    config.set_wait_spin_iters(16);
    REQUIRE(config.get_wait_spin_iters() == 16);
    config.set_wait_yield_iters(2);
    REQUIRE(config.get_wait_yield_iters() == 2);
    config.set_wait_park_us(200);
    REQUIRE(config.get_wait_park_us() == 200);

    // Default consumer cpu is unpinned (the negative sentinel).
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
    config.set_consumer_cpu(3);
    REQUIRE(config.get_consumer_cpu() == 3);
    config.set_consumer_cpu(THETADATADX_CONSUMER_CPU_UNPINNED);
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
}

TEST_CASE("HistoricalClient::connect succeeds against the production server", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();
    REQUIRE_NOTHROW(thetadatadx::HistoricalClient::connect(creds, config));
}
