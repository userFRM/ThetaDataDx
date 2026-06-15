// FLATFILES dynamic-schema decode -> Arrow IPC bytes.
//
// The flat-file surface returns whole-universe data for a single
// (sec_type, req_type, date) tuple. Decoded shape is determined at
// runtime by the request type, so the C++ wrapper exposes Arrow IPC
// bytes — pair with arrow-cpp on the consumer side to materialise an
// `arrow::Table`.

#include <fstream>
#include <iostream>

#include "thetadx.hpp"

int main() {
    try {
        auto creds = thetadatadx::Credentials::from_file("creds.txt");
        auto config = thetadatadx::Config::production();
        auto unified = thetadatadx::Client::connect(creds, config);

        // Whole-universe option trade-quotes for one trading day.
        auto rows = unified.flat_files().option_trade_quote("20260428");
        std::cout << "option_trade_quote rows: " << rows.size() << std::endl;

        // Arrow IPC stream bytes -- feed into arrow::ipc::RecordBatchStreamReader
        // to materialise a typed RecordBatch with the dynamic schema.
        auto ipc = rows.to_arrow_ipc();
        std::ofstream out("/tmp/option-trade-quote.arrow", std::ios::binary);
        out.write(reinterpret_cast<const char*>(ipc.data()),
                  static_cast<std::streamsize>(ipc.size()));
        std::cout << "wrote " << ipc.size() << " bytes of Arrow IPC to "
                  << "/tmp/option-trade-quote.arrow" << std::endl;

        // Same path, dispatched dynamically.
        auto oi = unified.flat_files().request("OPTION", "OPEN_INTEREST", "20260428");
        std::cout << "open_interest rows: " << oi.size() << std::endl;

        // Drop raw vendor CSV bytes to disk without materialising rows.
        unified.flat_files().to_path("OPTION", "TRADE_QUOTE", "20260428",
                                     "/tmp/option-trade-quote", "csv");
        std::cout << "raw vendor CSV at /tmp/option-trade-quote.csv" << std::endl;
    } catch (const std::exception& e) {
        std::cerr << "error: " << e.what() << std::endl;
        return 1;
    }
    return 0;
}
