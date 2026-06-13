// Typed-error hierarchy tests for the C++ SDK.
//
// Before B4, every FFI error surfaced as a generic
// `std::runtime_error` carrying the formatted reason string —
// callers had to substring-match to distinguish auth failures from
// rate limits. B4 introduces a `ThetaDataError` base + a leaf class
// per `GrpcStatusKind` + `AuthErrorKind` discriminator that callers
// can `catch` on directly. The hierarchy mirrors the Python /
// TypeScript leaf set so the cross-binding contract stays uniform.

#include <chrono>
#include <stdexcept>
#include <string>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

#include "thetadx.h"
#include "thetadx.hpp"

TEST_CASE("ThetaDataError is the root of the SDK exception hierarchy",
          "[errors][offline]") {
    // The whole point of B4 is that callers can write
    // `catch (const tdx::SubscriptionError&)` to handle a tier /
    // permission error, and `catch (const tdx::ThetaDataError&)` to
    // catch everything from the SDK. The class hierarchy must root
    // on `ThetaDataError`, which itself must inherit from
    // `std::runtime_error` so callers writing
    // `catch (const std::exception&)` still observe the failure.
    STATIC_REQUIRE(std::is_base_of_v<std::runtime_error, tdx::ThetaDataError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::SubscriptionError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::RateLimitError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::AuthenticationError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::NotFoundError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::DeadlineExceededError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::UnavailableError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::NetworkError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::SchemaMismatchError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::InvalidParameterError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::StreamError>);
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::ConfigError>);
    // InvalidCredentialsError narrows AuthenticationError.
    STATIC_REQUIRE(std::is_base_of_v<tdx::AuthenticationError, tdx::InvalidCredentialsError>);
}

TEST_CASE("throw_for_code routes config discriminants to their leaf classes",
          "[errors][offline]") {
    // A rejected client parameter (`TDX_ERR_INVALID_PARAMETER`) must
    // surface as `InvalidParameterError`, distinguishable by catch type
    // from the environmental config fault (`TDX_ERR_CONFIG`) that
    // surfaces as `ConfigError`.
    try {
        tdx::detail::throw_for_code(TDX_ERR_INVALID_PARAMETER, "bad date");
        FAIL("throw_for_code must throw");
    } catch (const tdx::InvalidParameterError&) {
        // expected
    } catch (const tdx::ThetaDataError& e) {
        FAIL("expected InvalidParameterError, got generic ThetaDataError: " << e.what());
    }

    // The environmental config code routes to the dedicated
    // `ConfigError` leaf, not `InvalidParameterError` and not the root.
    try {
        tdx::detail::throw_for_code(TDX_ERR_CONFIG, "toml parse");
        FAIL("throw_for_code must throw");
    } catch (const tdx::InvalidParameterError& e) {
        FAIL("TDX_ERR_CONFIG must not surface as InvalidParameterError: " << e.what());
    } catch (const tdx::ConfigError&) {
        // expected — environmental config fault
    } catch (const tdx::ThetaDataError& e) {
        FAIL("expected ConfigError, got generic ThetaDataError: " << e.what());
    }
}

TEST_CASE("with_deadline rejects a negative deadline as InvalidParameterError",
          "[errors][offline]") {
    // A negative duration cannot be represented by the unsigned
    // `timeout_ms` field. `with_deadline` rejects it with
    // `InvalidParameterError` rather than coercing it: a
    // `static_cast<uint64_t>` of a negative count would wrap to a
    // multi-century deadline, the opposite of the caller's intent. This
    // matches the reject-don't-coerce contract the TypeScript binding
    // holds for the same out-of-domain `timeoutMs`, so a deadline a caller
    // could not have meant fails loudly across both surfaces.
    tdx::EndpointRequestOptions options;
    try {
        options.with_deadline(std::chrono::milliseconds(-5));
        FAIL("with_deadline must reject a negative deadline");
    } catch (const tdx::InvalidParameterError&) {
        // expected — distinguishable by catch type from a generic fault
    } catch (const tdx::ThetaDataError& e) {
        FAIL("expected InvalidParameterError, got generic ThetaDataError: " << e.what());
    }

    // A non-negative deadline is accepted and recorded as whole
    // milliseconds; the setter returns the bag by reference for chaining.
    tdx::EndpointRequestOptions ok;
    REQUIRE_NOTHROW(ok.with_deadline(std::chrono::milliseconds(5000)));
    REQUIRE(ok.timeout_ms.has_value());
    REQUIRE(ok.timeout_ms.value() == 5000u);

    // The boundary value zero is a valid (immediate) deadline, not a
    // rejected one.
    tdx::EndpointRequestOptions zero;
    REQUIRE_NOTHROW(zero.with_deadline(std::chrono::milliseconds(0)));
    REQUIRE(zero.timeout_ms.value() == 0u);
}

TEST_CASE("RateLimitError carries the server retry_after as a typed value",
          "[errors][offline]") {
    // A `RateLimitError` constructed with a back-off hint exposes it as
    // seconds; one constructed without a hint reports `std::nullopt`.
    tdx::RateLimitError with_hint("thetadatadx: 429", 1.5);
    REQUIRE(with_hint.retry_after().has_value());
    REQUIRE(with_hint.retry_after().value() == 1.5);

    tdx::RateLimitError without_hint("thetadatadx: 429", std::nullopt);
    REQUIRE_FALSE(without_hint.retry_after().has_value());

    // The legacy single-arg constructor still compiles and defaults the
    // hint to absent.
    tdx::RateLimitError legacy("thetadatadx: 429");
    REQUIRE_FALSE(legacy.retry_after().has_value());
}

TEST_CASE("classify_grpc_kind routes every canonical gRPC status to the right leaf",
          "[errors][offline]") {
    // Dispatch table test for `tdx::detail::throw_for_grpc_kind` —
    // the seam every generated FFI wrapper hits when
    // `tdx_get_last_error_code()` returns a typed discriminant. The
    // routing must match the Python leaf set one-for-one so a Python
    // user porting `except thetadatadx.SubscriptionError` to C++
    // gets `catch (const tdx::SubscriptionError&)` and the same
    // semantics.
    using K = tdx::GrpcStatusKind;

    auto throws_as = [](K kind, auto check) {
        try {
            tdx::detail::throw_for_grpc_kind(kind, "test");
            FAIL("throw_for_grpc_kind must throw");
        } catch (const tdx::ThetaDataError& e) {
            check(e);
        }
    };

    throws_as(K::PermissionDenied, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::SubscriptionError*>(&e) != nullptr);
    });
    throws_as(K::ResourceExhausted, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::RateLimitError*>(&e) != nullptr);
    });
    throws_as(K::Unauthenticated, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::AuthenticationError*>(&e) != nullptr);
    });
    throws_as(K::NotFound, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::NotFoundError*>(&e) != nullptr);
    });
    throws_as(K::DeadlineExceeded, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::DeadlineExceededError*>(&e) != nullptr);
    });
    throws_as(K::Unavailable, [](const tdx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const tdx::UnavailableError*>(&e) != nullptr);
    });
}

TEST_CASE("forced Unauthenticated from a real RPC surfaces as AuthenticationError",
          "[errors][live]") {
    // Live half: a bogus credential file forces the auth path to
    // return `Unauthenticated` from Nexus, which the C ABI surfaces
    // as a typed error. The C++ wrapper must catch on
    // `AuthenticationError`, not the generic `std::runtime_error`.
    //
    // Gated on `THETADX_LIVE_CREDS` because Nexus must actually be
    // reachable for the error path to fire — the offline harness
    // can't stand it up.
    const char* creds_path_raw = std::getenv("THETADX_LIVE_CREDS");
    if (creds_path_raw == nullptr) {
        SKIP("THETADX_LIVE_CREDS not set");
    }

    auto bogus = tdx::Credentials::from_email("not-a-real-user@example.invalid",
                                              "not-a-real-password");
    auto config = tdx::Config::production();
    try {
        (void)tdx::MddsClient::connect(bogus, config);
        FAIL("bogus credentials must surface an error");
    } catch (const tdx::AuthenticationError&) {
        // expected — auth failed before any data round-trip
    } catch (const tdx::ThetaDataError& e) {
        FAIL("expected AuthenticationError, got generic ThetaDataError: " << e.what());
    }
}

TEST_CASE("config enum setters reject an out-of-domain value with InvalidParameterError",
          "[errors][config][offline]") {
    // A bad enum int on a config setter is a rejected client parameter,
    // not an environmental config fault — every setter must surface
    // `InvalidParameterError` (narrowing `ThetaDataError`) so the C++
    // catch type matches the Python `ValueError` / TypeScript
    // `InvalidParameterError` for the same input. A valid value must not
    // throw.
    auto cfg = tdx::Config::production();

    REQUIRE_NOTHROW(cfg.set_flush_mode(1));
    REQUIRE_THROWS_AS(cfg.set_flush_mode(9), tdx::InvalidParameterError);
    REQUIRE_THROWS_AS(cfg.set_flush_mode(9), tdx::ThetaDataError);

    REQUIRE_NOTHROW(cfg.set_reconnect_jitter(2));
    REQUIRE_THROWS_AS(cfg.set_reconnect_jitter(9), tdx::InvalidParameterError);

    REQUIRE_NOTHROW(cfg.set_fpss_host_selection(1));
    REQUIRE_THROWS_AS(cfg.set_fpss_host_selection(5), tdx::InvalidParameterError);
}

TEST_CASE("sequence converters reject out-of-wire-range inputs with InvalidParameterError",
          "[errors][util][offline]") {
    // The wire domain is the i32 cycle. A representable integer outside
    // that domain is a rejected value, not a silent reinterpret — it
    // must throw `InvalidParameterError`, matching the Python
    // `ValueError` / TypeScript `InvalidParameterError`. In-range inputs
    // round-trip without throwing.
    REQUIRE_NOTHROW(tdx::util::sequence_signed_to_unsigned(0));
    REQUIRE(tdx::util::sequence_signed_to_unsigned(-1) ==
            tdx::util::sequence_signed_to_unsigned(-1));

    // i32::MAX + 1 and i32::MIN - 1 are outside the signed wire range.
    REQUIRE_THROWS_AS(tdx::util::sequence_signed_to_unsigned(2147483648LL),
                      tdx::InvalidParameterError);
    REQUIRE_THROWS_AS(tdx::util::sequence_signed_to_unsigned(-2147483649LL),
                      tdx::InvalidParameterError);

    // 2^32 is the first value past the unsigned wire range; the audit
    // repro that returned 0 before now rejects.
    REQUIRE_THROWS_AS(tdx::util::sequence_unsigned_to_signed(4294967296ULL),
                      tdx::InvalidParameterError);
    REQUIRE_THROWS_AS(tdx::util::sequence_unsigned_to_signed(4294967296ULL),
                      tdx::ThetaDataError);

    // The largest valid unsigned wire value still converts.
    REQUIRE_NOTHROW(tdx::util::sequence_unsigned_to_signed(4294967295ULL));
}
