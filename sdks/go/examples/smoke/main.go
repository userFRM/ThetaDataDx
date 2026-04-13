// Release smoke test for the Go SDK.
//
// Calls one stock, one option, and one index endpoint to prove the CGo FFI
// bridge works end-to-end. Run via:
//
//	cd sdks/go && LD_LIBRARY_PATH=../../target/release go run ./examples/smoke
//
// Expects creds.txt at the repository root (two lines: email, password).

package main

import (
	"fmt"
	"log"
	"os"

	thetadatadx "github.com/userFRM/thetadatadx/sdks/go"
)

func main() {
	creds := "../../creds.txt"
	if len(os.Args) > 1 {
		creds = os.Args[1]
	}

	c, err := thetadatadx.CredentialsFromFile(creds)
	if err != nil {
		log.Fatalf("creds: %v", err)
	}
	defer c.Close()

	cfg := thetadatadx.ProductionConfig()
	defer cfg.Close()

	client, err := thetadatadx.Connect(c, cfg)
	if err != nil {
		log.Fatalf("connect: %v", err)
	}
	defer client.Close()

	pass := 0
	fail := 0

	// 1. Stock: list symbols
	syms, err := client.StockListSymbols()
	if err != nil {
		fmt.Printf("  %-40s FAIL  %v\n", "stock_list_symbols", err)
		fail++
	} else {
		fmt.Printf("  %-40s PASS  %d symbols\n", "stock_list_symbols", len(syms))
		pass++
	}

	// 2. Option: list expirations
	exps, err := client.OptionListExpirations("SPY")
	if err != nil {
		fmt.Printf("  %-40s FAIL  %v\n", "option_list_expirations", err)
		fail++
	} else {
		fmt.Printf("  %-40s PASS  %d expirations\n", "option_list_expirations", len(exps))
		pass++
	}

	// 3. Index: list symbols
	idx, err := client.IndexListSymbols()
	if err != nil {
		fmt.Printf("  %-40s FAIL  %v\n", "index_list_symbols", err)
		fail++
	} else {
		fmt.Printf("  %-40s PASS  %d symbols\n", "index_list_symbols", len(idx))
		pass++
	}

	fmt.Printf("\nGo SDK: %d PASS, %d FAIL\n", pass, fail)
	if fail > 0 {
		os.Exit(1)
	}
}
