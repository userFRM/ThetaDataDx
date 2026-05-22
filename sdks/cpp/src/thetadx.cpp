/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadx.hpp"

#include <chrono>
#include <stdexcept>
#include <sstream>
#include <thread>
#include <utility>

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
//
// Member destruction order (REVERSE declaration order): `handle_` first,
// `callback_` second. The `handle_` deleter calls `tdx_fpss_free` which
// performs an internal 5 s drain barrier — see the ordering invariant
// comment above the member declarations in the header.
//
// The body raises the shutdown signal early so the consumer thread starts
// quiescing before the deleter polls the drain flag. If the drain barrier
// inside `tdx_fpss_free` times out, the FFI logs a `tracing::error!` and
// proceeds with destruction. We mirror the move-assign rescue path here:
// detach `callback_` storage to a helper thread for an extra 30 s grace
// window so user code never observes a synchronous UAF on destructor exit.
FpssClient::~FpssClient() {
    if (handle_) {
        tdx_fpss_shutdown(handle_.get());
        int drained = tdx_fpss_await_drain(handle_.get(), 5000);
        if (drained == 0) {
            // Drain timed out: the consumer may still be firing through
            // `callback_`'s storage. Detach the storage onto a helper
            // thread for a 30 s grace window so destruction happens off
            // the destructor path, then let `handle_`'s deleter run
            // (its own drain barrier will observe `drained == 1` since
            // we already polled it to timeout).
            std::thread([cb = std::move(callback_)]() mutable {
                std::this_thread::sleep_for(std::chrono::seconds(30));
                // `cb` destructs here, off the destructor path.
            }).detach();
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Standalone utilities (generated)
// ═══════════════════════════════════════════════════════════════

#include "utilities.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  REST fallback policy + _with_fallback shims (issue #571)
// ═══════════════════════════════════════════════════════════════

#include "fallback.cpp.inc"

} // namespace tdx
