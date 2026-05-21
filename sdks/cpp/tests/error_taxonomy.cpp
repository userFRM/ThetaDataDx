// Typed-error hierarchy tests for the C++ SDK.
//
// Before B4, every FFI error surfaced as a generic
// `std::runtime_error` carrying the formatted reason string —
// callers had to substring-match to distinguish auth failures from
// rate limits. B4 introduces a `ThetaDataError` base + a leaf class
// per `GrpcStatusKind` + `AuthErrorKind` discriminator that callers
// can `catch` on directly. The hierarchy mirrors the Python /
// TypeScript leaf set so the cross-binding contract stays uniform.

#include <stdexcept>
#include <string>
#include <type_traits>

#include <catch2/catch_test_macros.hpp>

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
    STATIC_REQUIRE(std::is_base_of_v<tdx::ThetaDataError, tdx::StreamError>);
    // InvalidCredentialsError narrows AuthenticationError.
    STATIC_REQUIRE(std::is_base_of_v<tdx::AuthenticationError, tdx::InvalidCredentialsError>);
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
        (void)tdx::Client::connect(bogus, config);
        FAIL("bogus credentials must surface an error");
    } catch (const tdx::AuthenticationError&) {
        // expected — auth failed before any data round-trip
    } catch (const tdx::ThetaDataError& e) {
        FAIL("expected AuthenticationError, got generic ThetaDataError: " << e.what());
    }
}
