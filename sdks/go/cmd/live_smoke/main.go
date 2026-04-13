package main

import (
	"fmt"
	"log"
	"os"

	thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

const (
	histSymbol   = "AAPL"
	histStart    = "20260401"
	histEnd      = "20260402"
	atTime       = "09:30"
	atTimeLegacy = "34200000"
)

func main() {
	credsPath := "../../creds.txt"
	if len(os.Args) > 1 {
		credsPath = os.Args[1]
	}

	creds, err := thetadatadx.CredentialsFromFile(credsPath)
	if err != nil {
		log.Fatalf("creds: %v", err)
	}
	defer creds.Close()

	cfg := thetadatadx.ProductionConfig()
	defer cfg.Close()

	client, err := thetadatadx.Connect(creds, cfg)
	if err != nil {
		log.Fatalf("connect: %v", err)
	}
	defer client.Close()

	days, err := client.CalendarOpenToday()
	if err != nil {
		log.Fatalf("calendar_open_today: %v", err)
	}
	if len(days) == 0 {
		log.Fatal("calendar_open_today returned no rows")
	}

	eod, err := client.StockHistoryEOD(histSymbol, histStart, histEnd)
	if err != nil {
		log.Fatalf("stock_history_eod: %v", err)
	}
	if len(eod) == 0 {
		log.Fatal("stock_history_eod returned no rows")
	}

	atRows, err := client.StockAtTimeTrade(histSymbol, histStart, histEnd, atTime)
	if err != nil {
		log.Fatalf("stock_at_time_trade formatted time: %v", err)
	}
	if len(atRows) == 0 {
		log.Fatal("stock_at_time_trade formatted time returned no rows")
	}

	legacyRows, err := client.StockAtTimeTrade(histSymbol, histStart, histEnd, atTimeLegacy)
	if err != nil {
		log.Fatalf("stock_at_time_trade legacy ms: %v", err)
	}
	if len(legacyRows) == 0 {
		log.Fatal("stock_at_time_trade legacy ms returned no rows")
	}

	fmt.Println("go live smoke: ok")
}
