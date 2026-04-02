#include <iostream>
#include <iomanip>
#include "thetadx.hpp"

int main() {
    try {
        // Load credentials from creds.txt (line 1 = email, line 2 = password)
        auto creds = tdx::Credentials::from_file("creds.txt");
        auto config = tdx::Config::production();
        auto client = tdx::Client::connect(creds, config);

        // Fetch end-of-day data (raw #[repr(C)] structs — prices are raw integers)
        auto eod = client.stock_history_eod("AAPL", "20240101", "20240301");
        std::cout << "Got " << eod.size() << " EOD ticks for AAPL" << std::endl;
        for (auto& tick : eod) {
            std::cout << "  " << tick.date
                      << ": O=" << std::fixed << std::setprecision(2)
                      << tdx::price_to_f64(tick.open, tick.price_type)
                      << " H=" << tdx::price_to_f64(tick.high, tick.price_type)
                      << " L=" << tdx::price_to_f64(tick.low, tick.price_type)
                      << " C=" << tdx::price_to_f64(tick.close, tick.price_type)
                      << " V=" << tick.volume
                      << std::endl;
        }

        // Greeks calculator (no server connection needed)
        auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
        std::cout << "\nGreeks:"
                  << " IV=" << std::setprecision(4) << g.iv
                  << " Delta=" << g.delta
                  << " Gamma=" << std::setprecision(6) << g.gamma
                  << " Theta=" << std::setprecision(4) << g.theta
                  << std::endl;

        // Implied volatility
        auto [iv, err] = tdx::implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, true);
        std::cout << "IV=" << std::setprecision(6) << iv
                  << " (error=" << std::scientific << err << ")" << std::endl;

    } catch (const std::exception& e) {
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
