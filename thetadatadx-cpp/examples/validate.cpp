// Live parameter-mode matrix validator for the C++ SDK. Runs every cell in
// validate_cells.inc, which `cargo run -p xtask --bin generate_sdk_surfaces`
// renders from the endpoint table before a live run (the file is not
// committed), against production; the server is the ground truth for what the
// account can access. Cells whose documented min_tier exceeds the live
// account tier come back as a SubscriptionError and are classified
// SKIP: tier-permission. Real configuration bugs surface as FAIL.
//
// Every cell is classified by the exception's type, never by message text.
//
// Each cell carries a per-call deadline (60 s, or 180 s for a chain-wide
// cell, whose payload legitimately takes longer than a minute). The SDK
// cancels the in-flight gRPC stream on expiry and throws a
// DeadlineExceededError; the client stays usable for the next cell.

#include <chrono>
#include <cstdlib>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <sys/stat.h>
#include <utility>
#include <vector>
#include "thetadatadx.hpp"

namespace {

struct CellRecord {
    std::string endpoint;
    std::string mode;
    // The one-sentence cell description; surfaces in
    // scripts/ci/check_agreement.py disagreement output.
    std::string rationale;
    std::string status;
    int row_count = 0;
    long long duration_ms = 0;
    std::string detail;
};

std::string json_escape(const std::string& in) {
    std::string out;
    out.reserve(in.size() + 2);
    for (char c : in) {
        switch (c) {
            case '\\': out += "\\\\"; break;
            case '"':  out += "\\\""; break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            case '\t': out += "\\t";  break;
            default:
                if (static_cast<unsigned char>(c) < 0x20) {
                    char buf[8];
                    std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                    out += buf;
                } else {
                    out += c;
                }
        }
    }
    return out;
}

} // namespace

int main(int argc, char** argv) {
    const std::string creds_path = argc > 1 ? argv[1] : "creds.txt";
    int pass = 0;
    int skip = 0;
    int fail = 0;
    std::vector<CellRecord> records;

    try {
        auto creds = thetadatadx::Credentials::from_file(creds_path);
        auto config = thetadatadx::Config::production();
        auto client = thetadatadx::MarketDataClient::connect(creds, config);

        auto cell = [&](const char* endpoint, const char* mode, const char* declared_min_tier, const char* rationale, auto&& call) {
            const std::string label = std::string(endpoint) + "::" + mode;
            CellRecord rec{endpoint, mode, rationale, std::string{}, 0, 0, std::string{}};
            const auto t0 = std::chrono::steady_clock::now();
            try {
                auto value = call();
                rec.row_count = static_cast<int>(value.size());
                std::cout << "  " << std::left << std::setw(60) << label << " PASS" << std::endl;
                ++pass;
                rec.status = "PASS";
            } catch (const thetadatadx::DeadlineExceededError&) {
                // SDK cancelled the in-flight gRPC stream on deadline elapse;
                // the next cell runs normally on the same MarketDataClient handle.
                const long long elapsed_s = std::chrono::duration_cast<std::chrono::seconds>(std::chrono::steady_clock::now() - t0).count();
                std::cout << "  " << std::left << std::setw(60) << label << " FAIL  timeout after " << elapsed_s << "s" << std::endl;
                ++fail;
                rec.status = "FAIL";
                rec.detail = "timeout after " + std::to_string(elapsed_s) + "s";
            } catch (const thetadatadx::SubscriptionError&) {
                std::cout << "  " << std::left << std::setw(60) << label << " SKIP: tier-permission (declared min_tier=" << declared_min_tier << ")" << std::endl;
                ++skip;
                rec.status = "SKIP";
                rec.detail = "tier-permission";
            } catch (const thetadatadx::NotFoundError&) {
                // The endpoint answered; the account simply has no row for
                // this contract and date. That is a served response, not a
                // failure to reach the surface under test.
                std::cout << "  " << std::left << std::setw(60) << label << " PASS  (no data)" << std::endl;
                ++pass;
                rec.status = "PASS";
                rec.detail = "no data";
            } catch (const std::exception& e) {
                std::cout << "  " << std::left << std::setw(60) << label << " FAIL  " << e.what() << std::endl;
                ++fail;
                rec.status = "FAIL";
                // Runtime error messages can contain embedded newlines;
                // escape them so the agreement-table row stays on one
                // line (see scripts/ci/check_agreement.py).
                std::string d = e.what();
                for (size_t pos = 0; (pos = d.find('\n', pos)) != std::string::npos; ) { d.replace(pos, 1, "\\n"); pos += 2; }
                for (size_t pos = 0; (pos = d.find('\r', pos)) != std::string::npos; ) { d.replace(pos, 1, "\\r"); pos += 2; }
                if (d.size() > 200) { d = d.substr(0, 200); }
                rec.detail = std::move(d);
            }
            rec.duration_ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count();
            records.push_back(rec);
        };

        #include "validate_cells.inc"
    } catch (const std::exception& e) {
        std::cerr << "validator bootstrap failure: " << e.what() << std::endl;
        return 1;
    }

    std::cout << "\nC++: " << pass << " PASS, " << skip << " SKIP, " << fail << " FAIL" << std::endl;
    std::cout << "COUNTS:" << pass << ":" << skip << ":" << fail << std::endl;
    try {
        const char* artifact_dir = "artifacts";
        mkdir(artifact_dir, 0755);
        std::ofstream artifact("artifacts/validator_cpp.json");
        artifact << "{\n  \"lang\": \"cpp\",\n  \"records\": [\n";
        for (size_t i = 0; i < records.size(); ++i) {
            const auto& r = records[i];
            artifact << "    {\"endpoint\": \"" << json_escape(r.endpoint) << "\", ";
            artifact << "\"mode\": \"" << json_escape(r.mode) << "\", ";
            artifact << "\"rationale\": \"" << json_escape(r.rationale) << "\", ";
            artifact << "\"status\": \"" << json_escape(r.status) << "\", ";
            artifact << "\"row_count\": " << r.row_count << ", ";
            artifact << "\"duration_ms\": " << r.duration_ms << ", ";
            artifact << "\"detail\": \"" << json_escape(r.detail) << "\"}";
            if (i + 1 < records.size()) { artifact << ","; }
            artifact << "\n";
        }
        artifact << "  ]\n}\n";
        artifact.close();
        std::cout << "artifact: artifacts/validator_cpp.json" << std::endl;
    } catch (const std::exception& e) {
        std::cerr << "artifact write failure: " << e.what() << std::endl;
    }
    std::cout.flush();
    // An empty result is a failure to measure, not a pass. Every cell can land
    // in the SKIP branch -- a lapsed entitlement, a vendor-side outage refusing
    // every request -- and the exit code, derived from `fail` alone, would
    // still be 0. The live workflow reads only that code, and the release
    // validator sums these counts into its failure total, where zero passes is
    // indistinguishable from zero failures. Mirrors the same guard in the
    // Python validator (scripts/ci/check_python.py).
    if (pass == 0) {
        std::cerr << "\nC++: nothing passed (" << skip << " SKIP, " << fail
                  << " FAIL of " << records.size()
                  << " cells). An all-skip run measures nothing and is not a pass."
                  << std::endl;
        std::cerr.flush();
        return 1;
    }
    std::cerr.flush();
    return fail > 0 ? 1 : 0;
}
