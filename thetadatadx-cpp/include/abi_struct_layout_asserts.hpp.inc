// Layout guards for the hand-written `#[repr(C)]` ABI structs that are
// not emitted from a schema (so no generated assert covers them). The
// generated tick and FPSS event structs are pinned in
// `tick_layout_asserts.hpp.inc` and `fpss_layout_asserts.hpp.inc`; the
// structs below are declared by hand on both sides, so these
// hand-written asserts are the only thing that fails the build if a
// field width, padding gap, or member order drifts from the Rust
// definitions in `thetadatadx-ffi/src` (`types.rs`, `streaming.rs`, `flatfiles.rs`).
//
// Sizes and offsets are the LP64 layout (x86_64 / aarch64 Linux, macOS)
// of the matching `#[repr(C)]` structs.

// ── ThetaDataDxOptionContract (thetadatadx-ffi/src/types.rs) ──
// symbol (const char*) @0, expiration (int32_t) @8, 4-byte pad,
// strike (double) @16, right (uint32_t) @24, 4-byte tail pad → 32 bytes.
static_assert(offsetof(ThetaDataDxOptionContract, symbol) == 0,
              "ThetaDataDxOptionContract::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, expiration) == 8,
              "ThetaDataDxOptionContract::expiration offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, strike) == 16,
              "ThetaDataDxOptionContract::strike offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, right) == 24,
              "ThetaDataDxOptionContract::right offset drifted from Rust");
static_assert(sizeof(ThetaDataDxOptionContract) == 32,
              "ThetaDataDxOptionContract total size drifted from Rust");

// ── ThetaDataDxOptionContractArray (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxOptionContractArray, data) == 0,
              "ThetaDataDxOptionContractArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContractArray, len) == 8,
              "ThetaDataDxOptionContractArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxOptionContractArray) == 16,
              "ThetaDataDxOptionContractArray total size drifted from Rust");

// ── ThetaDataDxStringArray (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxStringArray, data) == 0,
              "ThetaDataDxStringArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxStringArray, len) == 8,
              "ThetaDataDxStringArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxStringArray) == 16,
              "ThetaDataDxStringArray total size drifted from Rust");

// ── ThetaDataDxColumnPresence (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxColumnPresence, names) == 0,
              "ThetaDataDxColumnPresence::names offset drifted from Rust");
static_assert(offsetof(ThetaDataDxColumnPresence, len) == 8,
              "ThetaDataDxColumnPresence::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxColumnPresence) == 16,
              "ThetaDataDxColumnPresence total size drifted from Rust");

// ── ThetaDataDxSubscription (thetadatadx-ffi/src/streaming.rs) ──
static_assert(offsetof(ThetaDataDxSubscription, kind) == 0,
              "ThetaDataDxSubscription::kind offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscription, contract) == 8,
              "ThetaDataDxSubscription::contract offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscription) == 16,
              "ThetaDataDxSubscription total size drifted from Rust");

// ── ThetaDataDxSubscriptionArray (thetadatadx-ffi/src/streaming.rs) ──
static_assert(offsetof(ThetaDataDxSubscriptionArray, data) == 0,
              "ThetaDataDxSubscriptionArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionArray, len) == 8,
              "ThetaDataDxSubscriptionArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscriptionArray) == 16,
              "ThetaDataDxSubscriptionArray total size drifted from Rust");

// ── ThetaDataDxSubscriptionRequest (thetadatadx-ffi/src/streaming.rs) ──
// scope (int32_t) @0, kind (int32_t) @4, then six pointer fields packed
// 8 bytes apart with no interior padding → 48 bytes.
static_assert(offsetof(ThetaDataDxSubscriptionRequest, scope) == 0,
              "ThetaDataDxSubscriptionRequest::scope offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, kind) == 4,
              "ThetaDataDxSubscriptionRequest::kind offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, symbol) == 8,
              "ThetaDataDxSubscriptionRequest::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, expiration) == 16,
              "ThetaDataDxSubscriptionRequest::expiration offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, strike) == 24,
              "ThetaDataDxSubscriptionRequest::strike offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, right) == 32,
              "ThetaDataDxSubscriptionRequest::right offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, sec_type) == 40,
              "ThetaDataDxSubscriptionRequest::sec_type offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscriptionRequest) == 48,
              "ThetaDataDxSubscriptionRequest total size drifted from Rust");

// ── ThetaDataDxArrowBytes (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxArrowBytes, data) == 0,
              "ThetaDataDxArrowBytes::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxArrowBytes, len) == 8,
              "ThetaDataDxArrowBytes::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxArrowBytes) == 16,
              "ThetaDataDxArrowBytes total size drifted from Rust");

// ── ThetaDataDxFlatFileBytes (thetadatadx-ffi/src/flatfiles.rs) ──
// Layout-identical to ThetaDataDxArrowBytes by contract.
static_assert(offsetof(ThetaDataDxFlatFileBytes, data) == 0,
              "ThetaDataDxFlatFileBytes::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxFlatFileBytes, len) == 8,
              "ThetaDataDxFlatFileBytes::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxFlatFileBytes) == 16,
              "ThetaDataDxFlatFileBytes total size drifted from Rust");
