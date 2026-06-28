// Gate 3 - C++ side: doctest gate.
//
// Compiles every `// @example` block extracted from the public C++
// headers (`thetadatadx.hpp` + `.inc` includes). The C++ harness piggybacks
// on the existing Catch2 test binary - `cpp-tests` runs us at the same
// time as every other offline test.
//
// Today the headers carry zero `// @example` blocks (the C++ surface
// was originally hand-written from the C ABI; examples live in
// `thetadatadx-cpp/examples/` as standalone .cpp files). The gate is wired
// in CI so adding an example block to a header automatically starts
// exercising it here.
//
// When you add a header example block, prefer this pattern:
//
//     /// @example
//     ///     auto creds = thetadatadx::Credentials("user@example.com", "pw");
//     ///     auto cfg   = thetadatadx::Config::production();
//     ///     thetadatadx::Client client(creds, cfg);
//
// Then translate the body into a TEST_CASE below. The build step
// will fail at compile time if the example references a stale symbol.

#include <catch2/catch_test_macros.hpp>

#include "thetadatadx.hpp"

TEST_CASE("C++ doctest harness compiles against the public header",
          "[doctest][offline]") {
    // Smoke: the harness binary links against `thetadatadx.hpp` and every
    // hand-translated example below sees the full public surface.
    // Adding new `// @example` blocks to headers requires translating
    // them into TEST_CASEs here - the gate is "every example block in
    // a header has a matching test in this file".
    SUCCEED("doctest harness reachable; no header-extracted examples yet");
}
