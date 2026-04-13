// Release validation for the Go SDK.
//
// Calls all non-stream endpoints to prove the CGo FFI bridge works end-to-end.
// Run via:
//
//	cd sdks/go && LD_LIBRARY_PATH=../../target/release go run ./cmd/validate
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

	pass, skip, fail := thetadatadx.ValidateAllEndpoints(client)
	fmt.Printf("\nGo: %d PASS, %d SKIP, %d FAIL\n", pass, skip, fail)
	fmt.Printf("COUNTS:%d:%d:%d\n", pass, skip, fail)
	if fail > 0 {
		os.Exit(1)
	}
}
