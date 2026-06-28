/**
 * thetadatadx C++ RAII wrapper.
 *
 * Wraps the C FFI handles in RAII classes with unique_ptr-based ownership.
 * All data methods return typed C++ vectors directly from #[repr(C)] struct arrays.
 * No JSON parsing required — the tick structs are layout-compatible with Rust.
 */

#include "thetadatadx.hpp"

#include <utility>

namespace thetadatadx {

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
//  Historical server-stream endpoints (generated)
// ═══════════════════════════════════════════════════════════════

#include "historical_stream.cpp.inc"

// ═══════════════════════════════════════════════════════════════
//  FPSS (streaming) — typed #[repr(C)] events (generated)
// ═══════════════════════════════════════════════════════════════

#include "fpss.cpp.inc"

// Lifecycle: intentionally hand-written (C++ destructor semantics with unique_ptr).
//
// Member destruction order (REVERSE declaration order): `handle_` first,
// `callback_` second. The `handle_` deleter calls `thetadatadx_streaming_free` which
// performs an internal 5 s drain barrier — see the ordering invariant
// comment above the member declarations in the header.
//
// The body raises the shutdown signal early so the consumer thread starts
// quiescing before the deleter polls the drain flag. On drain timeout (rare,
// a wedged user callback) the consumer may still be firing through
// `callback_`'s storage, so we MUST NOT let the storage drop synchronously.
// We mirror `StreamingClient::operator=`'s rescue path exactly: hand BOTH the
// retired handle and callback storage to a reclaimer that polls the same
// drain barrier and releases them (handle first, then storage) only once the
// consumer is confirmed quiesced, with the bounded `kReclaimQuiescenceCap` so
// a wedged callback cannot leak — never a guessed wall-clock window.
StreamingClient::~StreamingClient() {
    if (handle_) {
        thetadatadx_streaming_shutdown(handle_.get());
        int drained = thetadatadx_streaming_await_drain(handle_.get(), 5000);
        if (drained == 0) {
            const ThetaDataDxStreamHandle* raw = handle_.get();
            detail::reclaim_after_drain(
                [raw]() {
                    return thetadatadx_streaming_await_drain(
                               raw,
                               static_cast<uint64_t>(
                                   detail::kReclaimPollStep.count())) == 1;
                },
                [retired_handle = std::move(handle_),
                 retired_cb = std::move(callback_)]() mutable {
                    // Free the handle first so its internal drain barrier
                    // still observes a live `ctx`, then drop the storage.
                    // Mirrors the handle-before-callback member invariant.
                    retired_handle.reset();
                    retired_cb.reset();
                });
            // `handle_` and `callback_` are now empty; the reclaimer owns
            // the retired session.
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Standalone utilities (generated)
// ═══════════════════════════════════════════════════════════════

#include "utilities.cpp.inc"

} // namespace thetadatadx
