# Catch2 v3-based test harness for the ThetaDataDx C++ SDK.
#
# Two test buckets:
#
#   * Offline (always built + run by `ctest`): exercise the type-level
#     surface — null-safe getters, move semantics, RAII teardown paths,
#     compile-time symbol bindings. No network, no credentials.
#
#   * Live (gated on `THETADATADX_LIVE_CREDS`): exercise the full FPSS /
#     historical round-trip against the production server. No CI workflow
#     sets the variable, so CI runs the offline subset only; set
#     `THETADATADX_LIVE_CREDS=/path/to/creds.txt` to run the live subset
#     locally.
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
    # Async market-data query surface — the `<endpoint>_async` companions
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
    # InterestRateTick is a 2-field shape (date/rate); guards against a
    # regression to a fictitious 3-field ms_of_day layout.
    interest_rate.cpp
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

# The C++ column of the cross-binding manifest `parity.json`, rendered into a
# translation unit that takes the address of every entry the manifest lists
# for C++, so a missing one fails to compile and a missing C ABI export fails
# to link. It also holds the header's error codes to the manifest.
if(CMAKE_VERSION VERSION_LESS 3.19)
    message(FATAL_ERROR "the C++ tests read parity.json with string(JSON), which needs CMake 3.19")
endif()
set(THETADATADX_PARITY_JSON "${CMAKE_CURRENT_SOURCE_DIR}/../../parity.json")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${THETADATADX_PARITY_JSON}")
file(READ "${THETADATADX_PARITY_JSON}" parity)

# Sets `out` to the C++ spelling of `name` under `spec`, or to "" when C++ lacks it.
function(parity_cpp_name out name spec_type spec)
    set(result "")
    if(spec_type STREQUAL "OBJECT")
        string(JSON result ERROR_VARIABLE missing GET "${spec}" cpp)
        if(missing)
            set(result "")
        endif()
    elseif(spec STREQUAL "*" OR " ${spec} " MATCHES " cpp ")
        set(result "${name}")
    endif()
    set(${out} "${result}" PARENT_SCOPE)
endfunction()

set(parity_body "")
string(JSON class_count LENGTH "${parity}" classes)
math(EXPR last_class "${class_count} - 1")
foreach(i RANGE ${last_class})
    string(JSON class MEMBER "${parity}" classes ${i})
    string(JSON cpp_type ERROR_VARIABLE no_override TYPE "${parity}" classes ${class} cpp)
    if(cpp_type STREQUAL "NULL")
        continue()
    elseif(no_override)
        set(cpp_class "thetadatadx::${class}")
    else()
        string(JSON cpp_class GET "${parity}" classes ${class} cpp)
        set(cpp_class "thetadatadx::${cpp_class}")
    endif()
    string(JSON requires ERROR_VARIABLE unconditional GET "${parity}" classes ${class} cpp_requires)
    if(NOT unconditional)
        string(APPEND parity_body "#ifdef ${requires}\n")
    endif()
    string(APPEND parity_body "    static_cast<void>(sizeof(${cpp_class}));\n")
    string(JSON member_count LENGTH "${parity}" classes ${class} members)
    if(member_count GREATER 0)
        math(EXPR last_member "${member_count} - 1")
        foreach(j RANGE ${last_member})
            string(JSON member MEMBER "${parity}" classes ${class} members ${j})
            string(JSON spec_type TYPE "${parity}" classes ${class} members ${member})
            string(JSON spec GET "${parity}" classes ${class} members ${member})
            parity_cpp_name(cpp_member ${member} ${spec_type} "${spec}")
            if(cpp_member)
                set(member_requires "")
                if(spec_type STREQUAL "OBJECT")
                    string(JSON member_requires ERROR_VARIABLE no_requires GET "${spec}" cpp_requires)
                endif()
                if(member_requires)
                    string(APPEND parity_body "#ifdef ${member_requires}\n")
                endif()
                string(APPEND parity_body "    static_cast<void>(&${cpp_class}::${cpp_member});\n")
                if(member_requires)
                    string(APPEND parity_body "#endif\n")
                endif()
            endif()
        endforeach()
    endif()
    if(NOT unconditional)
        string(APPEND parity_body "#endif\n")
    endif()
endforeach()

string(JSON function_count LENGTH "${parity}" functions)
math(EXPR last_function "${function_count} - 1")
foreach(i RANGE ${last_function})
    string(JSON function MEMBER "${parity}" functions ${i})
    string(JSON spec_type TYPE "${parity}" functions ${function})
    string(JSON spec GET "${parity}" functions ${function})
    parity_cpp_name(cpp_function ${function} ${spec_type} "${spec}")
    if(cpp_function)
        string(APPEND parity_body "    static_cast<void>(&${cpp_function});\n")
    endif()
endforeach()

string(JSON c_count LENGTH "${parity}" c_functions)
math(EXPR last_c "${c_count} - 1")
foreach(i RANGE ${last_c})
    string(JSON c_function GET "${parity}" c_functions ${i})
    string(APPEND parity_body "    static_cast<void>(&${c_function});\n")
endforeach()

set(parity_asserts "")
string(JSON code_count LENGTH "${parity}" error_codes)
math(EXPR last_code "${code_count} - 1")
foreach(i RANGE ${last_code})
    string(JSON code MEMBER "${parity}" error_codes ${i})
    string(JSON value GET "${parity}" error_codes ${code})
    string(APPEND parity_asserts "static_assert(${code} == ${value}, \"${code} differs from parity.json\");\n")
endforeach()

file(CONFIGURE OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/parity_surface.cpp" CONTENT
"// Rendered from parity.json by thetadatadx-cpp/tests/CMakeLists.txt.
#include \"thetadatadx.h\"
#include \"thetadatadx.hpp\"

${parity_asserts}
void thetadatadx_parity_surface() {
    // The free functions live in `thetadatadx` or `thetadatadx::util`.
    using namespace thetadatadx;
    using namespace thetadatadx::util;
${parity_body}}
")
list(APPEND THETADATADX_CPP_TEST_SOURCES "${CMAKE_CURRENT_BINARY_DIR}/parity_surface.cpp")

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
