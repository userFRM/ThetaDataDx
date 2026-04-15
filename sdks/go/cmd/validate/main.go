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

	cfg := thetadatadx.ProductionConfig()

	client, err := thetadatadx.Connect(c, cfg)
	if err != nil {
		log.Fatalf("connect: %v", err)
	}

	pass, skip, fail, hadTimeout := thetadatadx.ValidateAllEndpoints(client)
	fmt.Printf("\nGo: %d PASS, %d SKIP, %d FAIL\n", pass, skip, fail)
	fmt.Printf("COUNTS:%d:%d:%d\n", pass, skip, fail)

	// When any cell timed out, a background goroutine is still running a CGo
	// call holding a pointer to the client handle. Closing the handle now
	// would race with that goroutine (use-after-free). Skip the deferred
	// Close and let the OS reclaim memory + descriptors via os.Exit. When no
	// timeout fired, run Close explicitly so this binary matches normal
	// library-user hygiene. See issue #290.
	if !hadTimeout {
		client.Close()
		cfg.Close()
		c.Close()
	}
	if fail > 0 {
		os.Exit(1)
	}
	os.Exit(0)
}
