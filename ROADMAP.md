# Roadmap

This roadmap shows where ThetaDataDx is headed. It is intentionally undated: items move between horizons as priorities shift, and nothing here is a commitment. Each item links to a tracking issue where you can follow progress or weigh in.

## Now

The SDK is published under per-language package names starting at `0.1.0` (`thetadatadx-rs`, `thetadatadx-py`, `thetadatadx-ts`, plus the in-repo `thetadatadx-cpp`). The four SDKs, the bundled server, the MCP server, and the documentation share one Rust engine.

## Next

- **Native Go SDK** ([#1019](https://github.com/userFRM/ThetaDataDx/issues/1019)). A first-class Go SDK written in pure Go, not a cgo wrapper. It will be a native peer of the other SDKs, held to the same machine-enforced cross-SDK parity guarantee, so Go keeps everything that makes it Go: static-binary cross-compilation, the goroutine scheduler under streaming load, and a toolchain-free `go get`.
- **Self-updating server** ([#957](https://github.com/userFRM/ThetaDataDx/issues/957)). Push a tagged release and running servers pick it up, verify its signature, and swap themselves in, with staged rollout and a force-upgrade floor. Operators get urgent fixes without a manual re-download.
- **Live-state tools in the MCP** ([#1027](https://github.com/userFRM/ThetaDataDx/issues/1027)). The MCP gains a background subscription and pull-query tools so a model can watch a contract and read its live state on demand (`latest`, a bounded `window`, OHLCV bars) instead of consuming the raw feed. Raw data only; the derived signals stay in the analytics layer.

## Recently shipped

One identical surface across the Rust, Python, TypeScript, and C++ SDKs with machine-enforced cross-binding parity, a REST and WebSocket server, an MCP server, and a generated OpenAPI reference. See the [CHANGELOG](CHANGELOG.md) for the full history.

---

Have a request, or a different priority? [Open an issue](https://github.com/userFRM/ThetaDataDx/issues/new), or bring it up on the [ThetaData Discord](https://discord.thetadata.us/).
