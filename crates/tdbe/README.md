# tdbe -- ThetaData Binary Encoding

Pure data-format crate for ThetaData market data. Zero networking dependencies.

`tdbe` owns all data types and codecs with zero networking dependencies.
The `thetadatadx` client crate depends on `tdbe` for type definitions.

## What it contains

| Module | Contents |
|--------|----------|
| `types` | `Price`, `SecType`, `DataType`, `StreamMsgType`, and the generated tick structs (`TradeTick`, `QuoteTick`, `EodTick`, `OhlcTick`, ...) |
| `codec` | FIT nibble decoder and FIE string encoder used by FPSS tick compression |
| `greeks` | Full Black-Scholes calculator: 22 Greeks + IV bisection solver |
| `flags` | Bit flags and condition codes for market data records |
| `latency` | Wire-to-application latency computation (`latency_ns`) |
| `error` | Encoding-layer error types |

## Quick start

```toml
[dependencies]
tdbe = "0.1"
```

```rust
use tdbe::{Price, TradeTick, EodTick};
use tdbe::greeks;

// Fixed-point price encoding
let p = Price::new(15025, 8); // 150.25
assert_eq!(p.to_f64(), 150.25);

// Compute all 22 Greeks offline. Returns `Result<GreeksResult, Error>`
// — `Error::Config` for an unrecognised `right`.
let result = greeks::all_greeks(
    450.0,        // spot
    455.0,        // strike
    0.05,         // risk-free rate
    0.015,        // dividend yield
    30.0 / 365.0, // time to expiry (years)
    8.50,         // option market price
    "C",          // right ("C"/"P" or "call"/"put", case-insensitive)
)?;
println!("IV: {:.4}, Delta: {:.4}", result.iv, result.delta);
```

## Relationship to `thetadatadx`

`thetadatadx` depends on `tdbe` for all data types and codecs, then adds
networking (gRPC historical via MDDS, real-time streaming via FPSS), authentication,
and the unified `ThetaDataDx` client. If you only need types and offline Greeks,
depend on `tdbe` alone.

## License

Apache-2.0 -- see [LICENSE](../../LICENSE).
