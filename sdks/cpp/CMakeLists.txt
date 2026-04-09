cmake_minimum_required(VERSION 3.16)
project(thetadatadx-cpp VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Path to the Rust FFI library (built with: cargo build --release -p thetadatadx-ffi)
set(THETADX_FFI_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../target/release" CACHE PATH
    "Directory containing libthetadatadx_ffi.so/.a")

# ── thetadatadx C++ library ──

add_library(thetadatadx_cpp STATIC
    src/thetadx.cpp
)

target_include_directories(thetadatadx_cpp PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
)

# Link against the Rust FFI library
find_library(THETADX_FFI_LIB
    NAMES thetadatadx_ffi
    PATHS ${THETADX_FFI_DIR}
    NO_DEFAULT_PATH
    REQUIRED
)

target_link_libraries(thetadatadx_cpp PUBLIC
    ${THETADX_FFI_LIB}
    pthread
    dl
    m
)

# ── Example ──

add_executable(thetadatadx_example examples/main.cpp)
target_link_libraries(thetadatadx_example PRIVATE thetadatadx_cpp)
