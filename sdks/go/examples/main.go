package main

import (
	"fmt"
	"log"

	thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
	// Load credentials from creds.txt (line 1 = email, line 2 = password)
	creds, err := thetadatadx.CredentialsFromFile("creds.txt")
	if err != nil {
		log.Fatalf("Failed to load credentials: %v", err)
	}
	defer creds.Close()

	// Connect to ThetaData production servers
	config := thetadatadx.ProductionConfig()
	defer config.Close()

	client, err := thetadatadx.Connect(creds, config)
	if err != nil {
		log.Fatalf("Failed to connect: %v", err)
	}
	defer client.Close()

	// Fetch end-of-day data
	eod, err := client.StockHistoryEOD("AAPL", "20240101", "20240301")
	if err != nil {
		log.Fatalf("Failed to fetch EOD: %v", err)
	}
	fmt.Printf("Got %d EOD ticks for AAPL\n", len(eod))
	for _, tick := range eod {
		fmt.Printf("  %d: O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
			tick.Date, tick.Open, tick.High, tick.Low, tick.Close, tick.Volume)
	}

	// Greeks calculator (no server connection needed)
	greeks, err := thetadatadx.AllGreeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
	if err != nil {
		log.Fatalf("Failed to compute greeks: %v", err)
	}
	fmt.Printf("\nGreeks: IV=%.4f Delta=%.4f Gamma=%.6f Theta=%.4f\n",
		greeks.IV, greeks.Delta, greeks.Gamma, greeks.Theta)

	// Implied volatility
	iv, ivErr, err := thetadatadx.ImpliedVolatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
	if err != nil {
		log.Fatalf("Failed to compute IV: %v", err)
	}
	fmt.Printf("IV=%.6f (error=%.2e)\n", iv, ivErr)
}
