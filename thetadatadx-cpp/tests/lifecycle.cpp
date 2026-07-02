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

TEST_CASE("Credentials::from_env sources strictly from THETADATA_API_KEY",
          "[lifecycle][offline]") {
    setenv("THETADATA_API_KEY", "env-sourced-key", 1);
    auto creds = thetadatadx::Credentials::from_env();
    REQUIRE(creds.get() != nullptr);
    unsetenv("THETADATA_API_KEY");
}

TEST_CASE("Credentials::from_env throws when THETADATA_API_KEY is unset",
          "[lifecycle][offline]") {
    // Strict: an unset value is an error, with NO creds.txt fallback.
    unsetenv("THETADATA_API_KEY");
    REQUIRE_THROWS(thetadatadx::Credentials::from_env());
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
        out << "# select staging\nTHETADATA_HISTORICAL_TYPE=STAGE\n";
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
}

TEST_CASE("Config flush_mode getter round-trips", "[lifecycle][offline]") {
    // The readback getter mirrors the Python `Config.flush_mode` and
    // TypeScript `flushMode` surfaces, so a value set through the C++
    // wrapper reads back through the same wrapper.
    auto config = thetadatadx::Config::production();

    config.set_flush_mode(1);
    REQUIRE(config.get_flush_mode() == 1);
    config.set_flush_mode(0);
    REQUIRE(config.get_flush_mode() == 0);
}

TEST_CASE("Config consumer_cpu round-trip", "[lifecycle][offline]") {
    // Mirrors the Python `Config.consumer_cpu` and TypeScript
    // `consumerCpu` surfaces: a value set through the C++ wrapper reads
    // back through the same wrapper.
    auto config = thetadatadx::Config::production();

    // Default consumer cpu is unpinned (the negative sentinel).
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
    config.set_consumer_cpu(3);
    REQUIRE(config.get_consumer_cpu() == 3);
    config.set_consumer_cpu(THETADATADX_CONSUMER_CPU_UNPINNED);
    REQUIRE(config.get_consumer_cpu() == THETADATADX_CONSUMER_CPU_UNPINNED);
}

// Compile-time surface pin (issue #1069): the base clients carry a
// deterministic `void close()` teardown. Taking the member-function pointers
// fails to compile if `close` is dropped or its signature drifts, so the
// cross-binding lifecycle surface is enforced without a live connection.
TEST_CASE("base clients expose a deterministic close()", "[lifecycle][offline]") {
    void (thetadatadx::Client::*unified_close)() = &thetadatadx::Client::close;
    void (thetadatadx::HistoricalClient::*historical_close)() =
        &thetadatadx::HistoricalClient::close;
    REQUIRE(unified_close != nullptr);
    REQUIRE(historical_close != nullptr);
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

TEST_CASE("close() is idempotent and safe before destruction", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();

    // Unified client: close explicitly, twice; the second is a no-op, and the
    // subsequent destructor frees nothing (handle already released).
    auto client = thetadatadx::Client::connect(creds, config);
    REQUIRE_NOTHROW(client.close());
    REQUIRE_NOTHROW(client.close());

    // Historical-only client: same idempotent-close contract, no streaming to
    // drain.
    auto historical = thetadatadx::HistoricalClient::connect(creds, config);
    REQUIRE_NOTHROW(historical.close());
    REQUIRE_NOTHROW(historical.close());
}

TEST_CASE("close() releases the handle deterministically", "[lifecycle][live]") {
    const auto creds_path = env_or_empty("THETADATADX_LIVE_CREDS");
    if (creds_path.empty()) {
        SKIP("THETADATADX_LIVE_CREDS not set");
    }
    auto creds = thetadatadx::Credentials::from_file(creds_path);
    auto config = thetadatadx::Config::production();

    // Cross-binding parity anchor (the Python / TypeScript bindings match this):
    // close() RELEASES the client handle, so the client is unusable afterward.
    // The public raw-handle accessor going null is the deterministic-release
    // proof — the historical gRPC channel pool is freed at close, not at some
    // later GC. `HistoricalClient` releases through the same `handle_.reset()`
    // path (its handle accessor is private, so the idempotent-close case above
    // is its observable pin).
    auto client = thetadatadx::Client::connect(creds, config);
    REQUIRE(client.get() != nullptr);
    client.close();
    REQUIRE(client.get() == nullptr);
    // Idempotent after release: the accessor stays null, no double-free.
    client.close();
    REQUIRE(client.get() == nullptr);
}
