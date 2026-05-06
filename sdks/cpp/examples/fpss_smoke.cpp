// fpss_smoke.cpp -- C++ FPSS smoke test.
//
// The poll-based `tdx::FpssClient::next_event` API was removed in
// issue #482 (PR B) along with the underlying `tdx_fpss_next_event`
// C ABI symbol. This example will be rewritten to drive the new
// callback API (`tdx_fpss_set_callback` / `tdx_fpss_set_inline_callback`)
// when the C++ wrapper migration ships in PR E.
//
// Compiling this file in its current form is intentional only after
// the C++ wrapper migration lands; until then it is a static breakage
// signal so downstream consumers do not silently miss the API change.

#error "fpss_smoke.cpp depends on the removed `next_event` poll API. Re-enable when the C++ wrapper migrates to the callback C ABI in PR E (refs #482)."
