cmake_minimum_required(VERSION 3.16)
project(thetadatadx-cpp VERSION 6.0.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Path to the Rust FFI library (built with: cargo build --release -p thetadatadx-ffi)
set(THETADX_FFI_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/release" CACHE PATH
    "Directory containing the platform-specific thetadatadx FFI artifacts")

# ── thetadatadx C++ library ──

add_library(thetadatadx_cpp STATIC
    src/thetadx.cpp
)

target_include_directories(thetadatadx_cpp PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

# Link against the Rust FFI library using an imported target so the correct
# artifact is selected per platform.
add_library(thetadatadx_ffi SHARED IMPORTED GLOBAL)

if(WIN32)
    set(THETADX_FFI_IMPLIB "${THETADX_FFI_DIR}/thetadatadx_ffi.dll.lib")
    set(THETADX_FFI_RUNTIME "${THETADX_FFI_DIR}/thetadatadx_ffi.dll")
    if(NOT EXISTS "${THETADX_FFI_IMPLIB}")
        message(FATAL_ERROR "thetadatadx FFI import library not found: ${THETADX_FFI_IMPLIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_IMPLIB "${THETADX_FFI_IMPLIB}"
        IMPORTED_LOCATION "${THETADX_FFI_RUNTIME}"
    )
elseif(APPLE)
    set(THETADX_FFI_LIB "${THETADX_FFI_DIR}/libthetadatadx_ffi.dylib")
    if(NOT EXISTS "${THETADX_FFI_LIB}")
        message(FATAL_ERROR "thetadatadx FFI shared library not found: ${THETADX_FFI_LIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_LOCATION "${THETADX_FFI_LIB}"
    )
else()
    set(THETADX_FFI_LIB "${THETADX_FFI_DIR}/libthetadatadx_ffi.so")
    if(NOT EXISTS "${THETADX_FFI_LIB}")
        message(FATAL_ERROR "thetadatadx FFI shared library not found: ${THETADX_FFI_LIB}")
    endif()
    set_target_properties(thetadatadx_ffi PROPERTIES
        IMPORTED_LOCATION "${THETADX_FFI_LIB}"
    )
endif()

target_link_libraries(thetadatadx_cpp PUBLIC thetadatadx_ffi)

if(UNIX AND NOT APPLE)
    target_link_libraries(thetadatadx_cpp PUBLIC pthread dl m)
endif()

# ── Example ──

add_executable(thetadatadx_example examples/main.cpp)
target_link_libraries(thetadatadx_example PRIVATE thetadatadx_cpp)

# ── Validation and smoke executables ──

add_executable(thetadatadx_validate examples/validate.cpp)
target_link_libraries(thetadatadx_validate PRIVATE thetadatadx_cpp)

add_executable(thetadatadx_fpss_smoke examples/fpss_smoke.cpp)
target_link_libraries(thetadatadx_fpss_smoke PRIVATE thetadatadx_cpp)
