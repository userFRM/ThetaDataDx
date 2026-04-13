/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadx.hpp"

#include <stdexcept>
#include <sstream>

namespace tdx {

namespace detail {

// Borrow `const char*` views into a stable vector<string> for C FFI calls.
static std::vector<const char*> string_ptrs(const std::vector<std::string>& items) {
    std::vector<const char*> ptrs;
    ptrs.reserve(items.size());
    for (const auto& item : items) {
        ptrs.push_back(item.c_str());
    }
    return ptrs;
}

} // namespace detail

// ── Credentials / Config / Client lifecycle (generated) ──

#include "lifecycle.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  Historical endpoints (generated)
// ═══════════════════════════════════════════════════════════════

#include "historical.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  FPSS (streaming) — typed #[repr(C)] events (generated)
// ═══════════════════════════════════════════════════════════════

#include "fpss.cpp.inc"

// Lifecycle: intentionally hand-written (C++ destructor semantics with unique_ptr).
FpssClient::~FpssClient() {
    if (handle_) {
        tdx_fpss_shutdown(handle_.get());
    }
}

// ═══════════════════════════════════════════════════════════════
//  Standalone utilities (generated)
// ═══════════════════════════════════════════════════════════════

#include "utilities.cpp.inc"

} // namespace tdx
