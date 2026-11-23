// Every C++ name accessor against the core's name for each code.
//
// The cases are rendered by `generate_sdk_surfaces` from the coded fields
// table and the streaming event table, so a code added to either reaches this
// test without an edit here, and a coded field no accessor below covers fails
// it.

#include <catch2/catch_test_macros.hpp>

#include <cstdint>
#include <optional>
#include <string_view>

#include "thetadatadx.hpp"

namespace {

struct CodeName {
    std::string_view field;
    int32_t code;
    std::string_view name;
};

constexpr CodeName kCodeNames[] = {
#include "code_names.inc"
};

std::optional<std::string_view> cpp_name(std::string_view field, int32_t code) {
    using namespace thetadatadx;
    if (field == "sec_type") return sec_type_name(code);
    if (field == "result") return result_name(code);
    if (field == "reason") return reason_name(code);
    if (field == "reconnect_reason") return reconnect_reason_name(code);
    if (field == "cause") return cause_name(code);
    if (field == "implied_vol_source") return implied_vol_source_name(code);
    if (field == "calendar_status") return calendar_status_name(code);
    if (field == "wait_mode") return wait_mode_name(code);
    if (field == "bulk_fetch") return bulk_fetch_name(code);
    if (field == "reconnect_jitter") return reconnect_jitter_name(code);
    if (field == "reconnect_policy") return reconnect_policy_name(code);
    if (field == "kind") return kind_name(code);
    return std::nullopt;
}

// The fields the C ABI also names with a function of its own.
std::optional<std::string_view> c_name(std::string_view field, int32_t code) {
    if (field == "calendar_status") return thetadatadx_calendar_status_name(code);
    if (field == "reconnect_reason") return thetadatadx_reconnect_reason_name(code);
    return std::nullopt;
}

} // namespace

TEST_CASE("every C++ name accessor gives each code the core's name", "[codes][offline]") {
    for (const auto& c : kCodeNames) {
        INFO(c.field << " code " << c.code);
        const auto name = cpp_name(c.field, c.code);
        REQUIRE(name.has_value());
        CHECK(*name == c.name);
        if (const auto from_c = c_name(c.field, c.code)) {
            CHECK(*from_c == c.name);
        }
    }
}
