// Minimal end-to-end example for the C++ SDK.
//
// Connects an `MarketDataClient` from a `creds.txt` file and pulls end-of-day
// history for one symbol.

#include <iostream>
#include <iomanip>
#include "thetadatadx.hpp"

int main() {
    try {
        // Load credentials from creds.txt (line 1 = email, line 2 = password)
        auto creds = thetadatadx::Credentials::from_file("creds.txt");
        auto config = thetadatadx::Config::production();
        auto client = thetadatadx::MarketDataClient::connect(creds, config);

        // Fetch end-of-day data -- prices are already decoded to f64
        auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
        std::cout << "Got " << eod.size() << " EOD ticks for AAPL" << std::endl;
        for (auto& tick : eod) {
            std::cout << "  " << tick.date
                      << ": O=" << std::fixed << std::setprecision(2)
                      << tick.open
                      << " H=" << tick.high
                      << " L=" << tick.low
                      << " C=" << tick.close
                      << " V=" << tick.volume
                      << std::endl;
        }

    } catch (const std::exception& e) {
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
