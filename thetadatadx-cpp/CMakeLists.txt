cmake_minimum_required(VERSION 3.16)
# `VERSION` tracks the C++ binding alongside the Rust Cargo manifests
# (`Cargo.toml`, `thetadatadx-rs/Cargo.toml`, `tools/*/Cargo.toml`) and the
# TypeScript `package.json` files; all bindings carry the same version.
project(thetadatadx-cpp VERSION 0.2.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Path to the Rust FFI library (built with: cargo build --release -p thetadatadx-ffi)
set(THETADATADX_FFI_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../target/release" CACHE PATH
    "Directory containing the platform-specific thetadatadx FFI artifacts")

# ── thetadatadx C++ library ──

add_library(thetadatadx_cpp STATIC
    src/thetadatadx.cpp
)

target_include_directories(thetadatadx_cpp PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

# Link against the Rust FFI library using an imported target so the correct
# artifact is selected per platform.
add_library(thetadatadx_ffi SHARED IMPORTED GLOBAL)

if(WIN32)
    set(THETADATADX_FFI_IMPLIB "${THETADATADX_FFI_DIR}/thetadatadx_ffi.dll.lib")
    set(THETADATADX_FFI_RUNTIME "${THETADATADX_FFI_DIR}/thetadatadx_ffi.dll")
    if(NOT EXISTS "${THETADATADX_FFI_IMPLIB}")
        message(FATAL_ERROR "thetadatadx FFI import library not found: ${THETADATADX_FFI_IMPLIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_IMPLIB "${THETADATADX_FFI_IMPLIB}"
        IMPORTED_LOCATION "${THETADATADX_FFI_RUNTIME}"
    )
elseif(APPLE)
    set(THETADATADX_FFI_LIB "${THETADATADX_FFI_DIR}/libthetadatadx_ffi.dylib")
    if(NOT EXISTS "${THETADATADX_FFI_LIB}")
        message(FATAL_ERROR "thetadatadx FFI shared library not found: ${THETADATADX_FFI_LIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_LOCATION "${THETADATADX_FFI_LIB}"
    )
else()
    set(THETADATADX_FFI_LIB "${THETADATADX_FFI_DIR}/libthetadatadx_ffi.so")
    if(NOT EXISTS "${THETADATADX_FFI_LIB}")
        message(FATAL_ERROR "thetadatadx FFI shared library not found: ${THETADATADX_FFI_LIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_LOCATION "${THETADATADX_FFI_LIB}"
    )
endif()

target_link_libraries(thetadatadx_cpp PUBLIC thetadatadx_ffi)

if(UNIX AND NOT APPLE)
    target_link_libraries(thetadatadx_cpp PUBLIC pthread dl m)
endif()

# ── Optional: native arrow-cpp RecordBatch reader ──
#
# The pull-based columnar streaming reader (`client.stream().batches(..)`
# returning an `arrow::RecordBatchReader`) requires arrow-cpp. It is gated so
# the SDK still builds for users who do not link arrow-cpp; the rest of the
# surface (including the per-tick `*_to_arrow_ipc` byte-buffer terminals) is
# unaffected. Enable with `-DTHETADATADX_CPP_ARROW=ON` and ensure arrow-cpp is
# discoverable by `find_package(Arrow)`.
option(THETADATADX_CPP_ARROW "Enable the native arrow-cpp RecordBatch streaming reader" OFF)
if(THETADATADX_CPP_ARROW)
    find_package(Arrow REQUIRED)
    target_compile_definitions(thetadatadx_cpp PUBLIC THETADATADX_CPP_ARROW)
    if(TARGET Arrow::arrow_shared)
        target_link_libraries(thetadatadx_cpp PUBLIC Arrow::arrow_shared)
    else()
        target_link_libraries(thetadatadx_cpp PUBLIC Arrow::arrow_static)
    endif()
endif()

# ── Example ──

add_executable(thetadatadx_example examples/main.cpp)
target_link_libraries(thetadatadx_example PRIVATE thetadatadx_cpp)

# ── Validation and smoke executables ──

add_executable(thetadatadx_validate examples/validate.cpp)
target_link_libraries(thetadatadx_validate PRIVATE thetadatadx_cpp)

add_executable(thetadatadx_streaming_smoke examples/streaming_smoke.cpp)
target_link_libraries(thetadatadx_streaming_smoke PRIVATE thetadatadx_cpp)

add_executable(thetadatadx_market_data_stream_smoke examples/market_data_stream_smoke.cpp)
target_link_libraries(thetadatadx_market_data_stream_smoke PRIVATE thetadatadx_cpp)

add_executable(thetadatadx_flatfiles_example examples/flatfiles_trade_quotes_to_arrow.cpp)
target_link_libraries(thetadatadx_flatfiles_example PRIVATE thetadatadx_cpp)

# ── Tests (Catch2-based) ──
#
# Opt-in via `-DTHETADATADX_CPP_BUILD_TESTS=ON`. The default is OFF so
# a pristine `cmake -S thetadatadx-cpp` stays a no-network build; the test
# subdirectory pulls Catch2 via FetchContent which needs Git + a
# reachable github.com. CI flips the flag on; downstream consumers
# building against the SDK do not pay the dependency cost.
option(THETADATADX_CPP_BUILD_TESTS "Build the C++ SDK test suite" OFF)
if(THETADATADX_CPP_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()
