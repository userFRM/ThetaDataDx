package main

import (
	"fmt"
	"log"
	"os"
	"time"

	thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

const (
	fpssSymbol           = "AAPL"
	fpssOptionSymbol     = "SPY"
	fpssOptionExpiration = "20260417"
	fpssOptionStrike     = "550"
	fpssOptionRight      = "C"
)

func requireDataEvent(client *thetadatadx.FpssClient, timeout time.Duration) (int32, string, error) {
	deadline := time.Now().Add(timeout)
	lastKind := "none"
	for time.Now().Before(deadline) {
		event, err := client.NextEvent(500)
		if err != nil {
			return 0, lastKind, err
		}
		if event == nil {
			continue
		}
		switch event.Kind {
		case thetadatadx.FpssQuoteEvent:
			lastKind = "quote"
			if event.Quote != nil {
				return event.Quote.ContractID, lastKind, nil
			}
		case thetadatadx.FpssTradeEvent:
			lastKind = "trade"
			if event.Trade != nil {
				return event.Trade.ContractID, lastKind, nil
			}
		case thetadatadx.FpssOpenInterestEvent:
			lastKind = "open_interest"
			if event.OpenInterest != nil {
				return event.OpenInterest.ContractID, lastKind, nil
			}
		case thetadatadx.FpssOhlcvcEvent:
			lastKind = "ohlcvc"
			if event.Ohlcvc != nil {
				return event.Ohlcvc.ContractID, lastKind, nil
			}
		case thetadatadx.FpssControlEvent:
			lastKind = "control"
		case thetadatadx.FpssRawDataEvent:
			lastKind = "raw_data"
		}
	}
	return 0, lastKind, fmt.Errorf("timed out waiting for FPSS data event (last kind=%s)", lastKind)
}

func subscriptionsSnapshot(client *thetadatadx.FpssClient) (map[string]struct{}, error) {
	subs, err := client.ActiveSubscriptions()
	if err != nil {
		return nil, err
	}
	out := make(map[string]struct{}, len(subs))
	for _, sub := range subs {
		out[sub.Kind+"|"+sub.Contract] = struct{}{}
	}
	return out, nil
}

func sameSubscriptions(a, b map[string]struct{}) bool {
	if len(a) != len(b) {
		return false
	}
	for key := range a {
		if _, ok := b[key]; !ok {
			return false
		}
	}
	return true
}

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

	cfg := thetadatadx.DevConfig()
	cfg.SetReconnectPolicy(1)
	cfg.SetDeriveOhlcvc(false)
	defer cfg.Close()

	fpss, err := thetadatadx.NewFpssClient(creds, cfg)
	if err != nil {
		log.Fatalf("connect: %v", err)
	}
	defer fpss.Close()
	defer fpss.Shutdown()

	if _, err := fpss.SubscribeQuotes(fpssSymbol); err != nil {
		log.Fatalf("subscribe quotes: %v", err)
	}
	if _, err := fpss.SubscribeTrades(fpssSymbol); err != nil {
		log.Fatalf("subscribe trades: %v", err)
	}
	if _, err := fpss.SubscribeOptionQuotes(
		fpssOptionSymbol, fpssOptionExpiration, fpssOptionStrike, fpssOptionRight,
	); err != nil {
		log.Fatalf("subscribe option quotes: %v", err)
	}

	expected, err := subscriptionsSnapshot(fpss)
	if err != nil {
		log.Fatalf("active subscriptions: %v", err)
	}
	if len(expected) < 3 {
		log.Fatalf("expected at least 3 active subscriptions, got %v", expected)
	}

	contractID, kind, err := requireDataEvent(fpss, 20*time.Second)
	if err != nil {
		log.Fatal(err)
	}
	if contractID != 0 {
		contract, err := fpss.ContractLookup(int(contractID))
		if err != nil {
			log.Fatalf("contract lookup after %s event: %v", kind, err)
		}
		if contract == "" {
			log.Fatalf("contract lookup after %s event returned empty string", kind)
		}
	}

	contractMap, err := fpss.ContractMap()
	if err != nil {
		log.Fatalf("contract map: %v", err)
	}
	if len(contractMap) == 0 {
		log.Fatal("contract map returned no entries after first data event")
	}

	if err := fpss.Reconnect(); err != nil {
		log.Fatalf("reconnect: %v", err)
	}

	after, err := subscriptionsSnapshot(fpss)
	if err != nil {
		log.Fatalf("active subscriptions after reconnect: %v", err)
	}
	if !sameSubscriptions(expected, after) {
		log.Fatalf("subscriptions drifted across reconnect: expected %v got %v", expected, after)
	}

	contractID, kind, err = requireDataEvent(fpss, 20*time.Second)
	if err != nil {
		log.Fatal(err)
	}
	if contractID != 0 {
		contract, err := fpss.ContractLookup(int(contractID))
		if err != nil {
			log.Fatalf("contract lookup after reconnect %s event: %v", kind, err)
		}
		if contract == "" {
			log.Fatalf("contract lookup after reconnect %s event returned empty string", kind)
		}
	}

	fmt.Printf(
		"go fpss smoke: ok (symbol=%s, option=%s %s %s %s)\n",
		fpssSymbol,
		fpssOptionSymbol,
		fpssOptionExpiration,
		fpssOptionStrike,
		fpssOptionRight,
	)
}
