// Typed-error hierarchy tests for the C++ SDK.
//
// The C++ surface exposes a `ThetaDataError` base plus a leaf class
// per `GrpcStatusKind` + `AuthErrorKind` discriminator, so callers
// `catch` a typed exception instead of substring-matching a generic
// `std::runtime_error` reason string. The hierarchy mirrors the
// Python / TypeScript leaf set so the cross-binding contract stays
// uniform.

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
    // `catch (const thetadatadx::SubscriptionError&)` to handle a tier /
    // permission error, and `catch (const thetadatadx::ThetaDataError&)` to
    // catch everything from the SDK. The class hierarchy must root
    // on `ThetaDataError`, which itself must inherit from
    // `std::runtime_error` so callers writing
    // `catch (const std::exception&)` still observe the failure.
    STATIC_REQUIRE(std::is_base_of_v<std::runtime_error, thetadatadx::ThetaDataError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::SubscriptionError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::RateLimitError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::AuthenticationError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::NotFoundError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::DeadlineExceededError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::UnavailableError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::NetworkError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::SchemaMismatchError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::InvalidParameterError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::StreamError>);
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::ThetaDataError, thetadatadx::ConfigError>);
    // InvalidCredentialsError narrows AuthenticationError.
    STATIC_REQUIRE(std::is_base_of_v<thetadatadx::AuthenticationError, thetadatadx::InvalidCredentialsError>);
}

TEST_CASE("throw_for_code routes config discriminants to their leaf classes",
          "[errors][offline]") {
    // A rejected client parameter (`TDX_ERR_INVALID_PARAMETER`) must
    // surface as `InvalidParameterError`, distinguishable by catch type
    // from the environmental config fault (`TDX_ERR_CONFIG`) that
    // surfaces as `ConfigError`.
    try {
        thetadatadx::detail::throw_for_code(TDX_ERR_INVALID_PARAMETER, "bad date");
        FAIL("throw_for_code must throw");
    } catch (const thetadatadx::InvalidParameterError&) {
        // expected
    } catch (const thetadatadx::ThetaDataError& e) {
        FAIL("expected InvalidParameterError, got generic ThetaDataError: " << e.what());
    }

    // The environmental config code routes to the dedicated
    // `ConfigError` leaf, not `InvalidParameterError` and not the root.
    try {
        thetadatadx::detail::throw_for_code(TDX_ERR_CONFIG, "toml parse");
        FAIL("throw_for_code must throw");
    } catch (const thetadatadx::InvalidParameterError& e) {
        FAIL("TDX_ERR_CONFIG must not surface as InvalidParameterError: " << e.what());
    } catch (const thetadatadx::ConfigError&) {
        // expected — environmental config fault
    } catch (const thetadatadx::ThetaDataError& e) {
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
    thetadatadx::EndpointRequestOptions options;
    try {
        options.with_deadline(std::chrono::milliseconds(-5));
        FAIL("with_deadline must reject a negative deadline");
    } catch (const thetadatadx::InvalidParameterError&) {
        // expected — distinguishable by catch type from a generic fault
    } catch (const thetadatadx::ThetaDataError& e) {
        FAIL("expected InvalidParameterError, got generic ThetaDataError: " << e.what());
    }

    // A non-negative deadline is accepted and recorded as whole
    // milliseconds; the setter returns the bag by reference for chaining.
    thetadatadx::EndpointRequestOptions ok;
    REQUIRE_NOTHROW(ok.with_deadline(std::chrono::milliseconds(5000)));
    REQUIRE(ok.timeout_ms.has_value());
    REQUIRE(ok.timeout_ms.value() == 5000u);

    // The boundary value zero is a valid (immediate) deadline, not a
    // rejected one.
    thetadatadx::EndpointRequestOptions zero;
    REQUIRE_NOTHROW(zero.with_deadline(std::chrono::milliseconds(0)));
    REQUIRE(zero.timeout_ms.value() == 0u);
}

TEST_CASE("RateLimitError carries the server retry_after as a typed value",
          "[errors][offline]") {
    // A `RateLimitError` constructed with a back-off hint exposes it as
    // seconds; one constructed without a hint reports `std::nullopt`.
    thetadatadx::RateLimitError with_hint("thetadatadx: 429", 1.5);
    REQUIRE(with_hint.retry_after().has_value());
    REQUIRE(with_hint.retry_after().value() == 1.5);

    thetadatadx::RateLimitError without_hint("thetadatadx: 429", std::nullopt);
    REQUIRE_FALSE(without_hint.retry_after().has_value());

    // The legacy single-arg constructor still compiles and defaults the
    // hint to absent.
    thetadatadx::RateLimitError legacy("thetadatadx: 429");
    REQUIRE_FALSE(legacy.retry_after().has_value());
}

TEST_CASE("classify_grpc_kind routes every canonical gRPC status to the right leaf",
          "[errors][offline]") {
    // Dispatch table test for `thetadatadx::detail::throw_for_grpc_kind` —
    // the seam every generated FFI wrapper hits when
    // `tdx_get_last_error_code()` returns a typed discriminant. The
    // routing must match the Python leaf set one-for-one so a Python
    // user porting `except thetadatadx.SubscriptionError` to C++
    // gets `catch (const thetadatadx::SubscriptionError&)` and the same
    // semantics.
    using K = thetadatadx::GrpcStatusKind;

    auto throws_as = [](K kind, auto check) {
        try {
            thetadatadx::detail::throw_for_grpc_kind(kind, "test");
            FAIL("throw_for_grpc_kind must throw");
        } catch (const thetadatadx::ThetaDataError& e) {
            check(e);
        }
    };

    throws_as(K::PermissionDenied, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::SubscriptionError*>(&e) != nullptr);
    });
    throws_as(K::ResourceExhausted, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::RateLimitError*>(&e) != nullptr);
    });
    throws_as(K::Unauthenticated, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::AuthenticationError*>(&e) != nullptr);
    });
    throws_as(K::NotFound, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::NotFoundError*>(&e) != nullptr);
    });
    throws_as(K::DeadlineExceeded, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::DeadlineExceededError*>(&e) != nullptr);
    });
    throws_as(K::Unavailable, [](const thetadatadx::ThetaDataError& e) {
        REQUIRE(dynamic_cast<const thetadatadx::UnavailableError*>(&e) != nullptr);
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

    auto bogus = thetadatadx::Credentials::from_email("not-a-real-user@example.invalid",
                                              "not-a-real-password");
    auto config = thetadatadx::Config::production();
    try {
        (void)thetadatadx::HistoricalClient::connect(bogus, config);
        FAIL("bogus credentials must surface an error");
    } catch (const thetadatadx::AuthenticationError&) {
        // expected — auth failed before any data round-trip
    } catch (const thetadatadx::ThetaDataError& e) {
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
    auto cfg = thetadatadx::Config::production();

    REQUIRE_NOTHROW(cfg.set_flush_mode(1));
    REQUIRE_THROWS_AS(cfg.set_flush_mode(9), thetadatadx::InvalidParameterError);
    REQUIRE_THROWS_AS(cfg.set_flush_mode(9), thetadatadx::ThetaDataError);

    REQUIRE_NOTHROW(cfg.set_reconnect_jitter(2));
    REQUIRE_THROWS_AS(cfg.set_reconnect_jitter(9), thetadatadx::InvalidParameterError);

    REQUIRE_NOTHROW(cfg.set_fpss_host_selection(1));
    REQUIRE_THROWS_AS(cfg.set_fpss_host_selection(5), thetadatadx::InvalidParameterError);
}

TEST_CASE("sequence converters reject out-of-wire-range inputs with InvalidParameterError",
          "[errors][util][offline]") {
    // The wire domain is the i32 cycle. A representable integer outside
    // that domain is a rejected value, not a silent reinterpret — it
    // must throw `InvalidParameterError`, matching the Python
    // `ValueError` / TypeScript `InvalidParameterError`. In-range inputs
    // round-trip without throwing.
    REQUIRE_NOTHROW(thetadatadx::util::sequence_signed_to_unsigned(0));
    REQUIRE(thetadatadx::util::sequence_signed_to_unsigned(-1) ==
            thetadatadx::util::sequence_signed_to_unsigned(-1));

    // i32::MAX + 1 and i32::MIN - 1 are outside the signed wire range.
    REQUIRE_THROWS_AS(thetadatadx::util::sequence_signed_to_unsigned(2147483648LL),
                      thetadatadx::InvalidParameterError);
    REQUIRE_THROWS_AS(thetadatadx::util::sequence_signed_to_unsigned(-2147483649LL),
                      thetadatadx::InvalidParameterError);

    // 2^32 is the first value past the unsigned wire range; the audit
    // repro that returned 0 before now rejects.
    REQUIRE_THROWS_AS(thetadatadx::util::sequence_unsigned_to_signed(4294967296ULL),
                      thetadatadx::InvalidParameterError);
    REQUIRE_THROWS_AS(thetadatadx::util::sequence_unsigned_to_signed(4294967296ULL),
                      thetadatadx::ThetaDataError);

    // The largest valid unsigned wire value still converts.
    REQUIRE_NOTHROW(thetadatadx::util::sequence_unsigned_to_signed(4294967295ULL));
}
