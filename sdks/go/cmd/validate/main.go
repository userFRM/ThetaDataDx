// Release validation for the Go SDK.
//
// Calls all non-stream endpoints to prove the CGo FFI bridge works end-to-end.
// Run via:
//
//	cd sdks/go && LD_LIBRARY_PATH=../../target/release go run ./cmd/validate
//
// On Windows, build the GNU-targeted FFI first and make that directory available
// to CGo and the runtime loader:
//
//	rustup target add x86_64-pc-windows-gnu
//	cargo build --release --target x86_64-pc-windows-gnu -p thetadatadx-ffi
//	cd sdks/go && set CGO_LDFLAGS=-L..\\..\\target\\x86_64-pc-windows-gnu\\release && go run ./cmd/validate
//
// Expects creds.txt at the repository root (two lines: email, password).

package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"

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

	cfg := thetadatadx.ProductionConfig()

	client, err := thetadatadx.Connect(c, cfg)
	if err != nil {
		log.Fatalf("connect: %v", err)
	}

	pass, skip, fail, records := thetadatadx.ValidateAllEndpoints(client)
	fmt.Printf("\nGo: %d PASS, %d SKIP, %d FAIL\n", pass, skip, fail)
	fmt.Printf("COUNTS:%d:%d:%d\n", pass, skip, fail)

	// Write JSON artifact for the cross-language agreement check.
	// Path is relative to this binary's CWD, which is sdks/go when invoked
	// via `go run`. Repo root is two levels up.
	artifactPath := filepath.Join("..", "..", "artifacts", "validator_go.json")
	if err := os.MkdirAll(filepath.Dir(artifactPath), 0o755); err != nil {
		log.Printf("warning: mkdir artifacts: %v", err)
	}
	artifact := map[string]interface{}{
		"lang":    "go",
		"records": records,
	}
	data, err := json.MarshalIndent(artifact, "", "  ")
	if err != nil {
		log.Printf("warning: marshal artifact: %v", err)
	} else if err := os.WriteFile(artifactPath, data, 0o644); err != nil {
		log.Printf("warning: write artifact: %v", err)
	} else {
		fmt.Printf("artifact: %s\n", artifactPath)
	}

	// W3: every timeout is now cancelled by the SDK before returning, so there
	// are no leaked goroutines holding the client handle. Close unconditionally.
	client.Close()
	cfg.Close()
	c.Close()
	if fail > 0 {
		os.Exit(1)
	}
	os.Exit(0)
}
