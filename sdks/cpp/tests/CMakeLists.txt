# Catch2 v3-based test harness for the ThetaDataDx C++ SDK.
#
# Two test buckets:
#
#   * Offline (always built + run by `ctest`): exercise the type-level
#     surface — null-safe getters, move semantics, RAII teardown paths,
#     compile-time symbol bindings. No network, no credentials.
#
#   * Live (gated on `THETADX_LIVE_CREDS`): exercise the full FPSS /
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

set(THETADX_CPP_TEST_SOURCES
    lifecycle.cpp
    history.cpp
    fpss.cpp
    unified_client.cpp
    streaming_iter.cpp
    error_taxonomy.cpp
    # REST-fallback policy + Config::withRestFallback wiring tests
    # (issue #571 mitigation, cross-binding parity with Python/TS/FFI).
    fallback.cpp
    # Gate 3 (issue #546) - C++ doctest harness. Compiles + runs
    # `// @example` blocks extracted from the public headers.
    doctest_examples.cpp
)

add_executable(thetadatadx_cpp_tests ${THETADX_CPP_TEST_SOURCES})
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
# `SKIP()` at runtime unless `THETADX_LIVE_CREDS=/path/to/creds.txt`
# is in the environment. Catch2 returns exit code 4 for a skipped
# test case; `SKIP_RETURN_CODE` tells CTest to interpret that as a
# skip rather than a failure so the offline subset reports clean.
include(CTest)
include(${catch2_SOURCE_DIR}/extras/Catch.cmake)
catch_discover_tests(
    thetadatadx_cpp_tests
    PROPERTIES SKIP_RETURN_CODE 4
)
