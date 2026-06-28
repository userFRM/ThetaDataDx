// Minimal end-to-end example for the C++ SDK.
//
// Connects an `HistoricalClient` from a `creds.txt` file, pulls end-of-day
// history for one symbol, and runs the offline Greeks / implied-volatility
// calculators that need no server connection.

#include <iostream>
#include <iomanip>
#include "thetadatadx.hpp"

int main() {
    try {
        // Load credentials from creds.txt (line 1 = email, line 2 = password)
        auto creds = thetadatadx::Credentials::from_file("creds.txt");
        auto config = thetadatadx::Config::production();
        auto client = thetadatadx::HistoricalClient::connect(creds, config);

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

        // Greeks calculator (no server connection needed)
        auto g = thetadatadx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
        std::cout << "\nGreeks:"
                  << " IV=" << std::setprecision(4) << g.iv
                  << " Delta=" << g.delta
                  << " Gamma=" << std::setprecision(6) << g.gamma
                  << " Theta=" << std::setprecision(4) << g.theta
                  << std::endl;

        // Implied volatility
        auto [iv, err] = thetadatadx::implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
        std::cout << "IV=" << std::setprecision(6) << iv
                  << " (error=" << std::scientific << err << ")" << std::endl;

    } catch (const std::exception& e) {
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
