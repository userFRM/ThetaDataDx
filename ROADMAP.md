# Roadmap

This roadmap shows where ThetaDataDx is headed. It is intentionally undated: items move between horizons as priorities shift, and nothing here is a commitment. Each item links to a tracking issue where you can follow progress or weigh in.

## Now

Stabilizing v13 toward a stable `13.0.0` release. The release-candidate series is hardening the four SDKs, the bundled server, the MCP server, and the documentation ahead of the first stable cut.

## Next

- **Native Go SDK** ([#1019](https://github.com/userFRM/ThetaDataDx/issues/1019)). A first-class Go SDK written in pure Go, not a cgo wrapper. It will be a native peer of the other SDKs, held to the same machine-enforced cross-SDK parity guarantee, so Go keeps everything that makes it Go: static-binary cross-compilation, the goroutine scheduler under streaming load, and a toolchain-free `go get`.
- **Self-updating server** ([#957](https://github.com/userFRM/ThetaDataDx/issues/957)). Push a tagged release and running servers pick it up, verify its signature, and swap themselves in, with staged rollout and a force-upgrade floor. Operators get urgent fixes without a manual re-download.

## Recently shipped

One identical surface across the Rust, Python, TypeScript, and C++ SDKs with machine-enforced cross-binding parity, a REST and WebSocket server, an MCP server, and a generated OpenAPI reference. See the [CHANGELOG](CHANGELOG.md) for the full history.

---

Have a request, or a different priority? [Open an issue](https://github.com/userFRM/ThetaDataDx/issues/new), or bring it up on the [ThetaData Discord](https://discord.thetadata.us/).
