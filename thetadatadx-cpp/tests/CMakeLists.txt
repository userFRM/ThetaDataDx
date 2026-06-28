# Catch2 v3-based test harness for the ThetaDataDx C++ SDK.
#
# Two test buckets:
#
#   * Offline (always built + run by `ctest`): exercise the type-level
#     surface — null-safe getters, move semantics, RAII teardown paths,
#     compile-time symbol bindings. No network, no credentials.
#
#   * Live (gated on `THETADATADX_LIVE_CREDS`): exercise the full FPSS /
#     historical round-trip against the production server. CI exports
#     the secret only on the protected branch; PR builds skip the live
#     subset and pass on offline alone.
#
# Catch2 is pulled in via FetchContent so contributors don't need a
# system-wide install. Pinned to v3.5.4 (the most recent LTS at the
# time of writing) to keep CI hermetic.

include(FetchContent)
FetchContent_Declare(
    catch2
    GIT_REPOSITORY https://github.com/catchorg/Catch2.git
    GIT_TAG v3.5.4
    GIT_SHALLOW TRUE
)
FetchContent_MakeAvailable(catch2)

set(THETADATADX_CPP_TEST_SOURCES
    lifecycle.cpp
    history.cpp
    # Async historical query surface — the `<endpoint>_async` companions
    # returning std::future, cross-binding parity with the Python
    # `<endpoint>_async` / TypeScript Promise data methods.
    history_async.cpp
    fpss.cpp
    thetadatadx_client.cpp
    error_taxonomy.cpp
    # MDDS pool-sizing setters on thetadatadx::Config.
    config_pool_sizing.cpp
    # ReconnectConfig setters on thetadatadx::Config — cross-binding parity
    # with Python / TypeScript / FFI.
    config_reconnect.cpp
    # MDDS two-stage decode pipeline setters on thetadatadx::Config
    # (Phase 3 of 3, follow-up to PR #587 / #588).
    # FlatFilesConfig setters on thetadatadx::Config — cross-binding parity
    # with Python / TypeScript / FFI (issue #594).
    flatfiles_config.cpp
    # InterestRateTick is a 2-field shape (date/rate); guards against a
    # regression to a fictitious 3-field ms_of_day layout.
    interest_rate.cpp
    # Gate 3 - C++ doctest harness. Compiles + runs
    # `// @example` blocks extracted from the public headers.
    doctest_examples.cpp
    # String rendering on the fluent value types (operator<< / str) plus
    # the streaming-contract strike accessor — cross-binding parity with
    # the Python __repr__ / TypeScript toString surfaces.
    fluent_rendering.cpp
    # Arrow-IPC terminal on the history result vectors — cross-binding
    # parity with the Python `<TickName>List.to_arrow()` columnar exit.
    arrow_ipc.cpp
)

# The pull-based Arrow RecordBatch reader test needs the native arrow-cpp
# reader, so it is built only when that optional feature is on. It mirrors
# the header-side `#ifdef THETADATADX_CPP_ARROW` gate on the reader itself.
if(THETADATADX_CPP_ARROW)
    list(APPEND THETADATADX_CPP_TEST_SOURCES streaming_batches.cpp)
endif()

add_executable(thetadatadx_cpp_tests ${THETADATADX_CPP_TEST_SOURCES})
target_link_libraries(thetadatadx_cpp_tests
    PRIVATE
        thetadatadx_cpp
        Catch2::Catch2WithMain
)
target_include_directories(thetadatadx_cpp_tests
    PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/../include
)

# CTest registration. Live tests carry the `[live]` Catch2 tag and
# `SKIP()` at runtime unless `THETADATADX_LIVE_CREDS=/path/to/creds.txt`
# is in the environment. Catch2 returns exit code 4 for a skipped
# test case; `SKIP_RETURN_CODE` tells CTest to interpret that as a
# skip rather than a failure so the offline subset reports clean.
include(CTest)
include(${catch2_SOURCE_DIR}/extras/Catch.cmake)
catch_discover_tests(
    thetadatadx_cpp_tests
    PROPERTIES SKIP_RETURN_CODE 4
)
