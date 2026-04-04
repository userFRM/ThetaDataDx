# Reverse-Engineering Guide

This crate was built by decompiling ThetaData's Java terminal. This document covers how to repeat the process when ThetaData releases a new version.

## Source Terminal Version

| Field | Value |
|-------|-------|
| JAR version | `202603181` (2026-03-18, revision 1) |
| Size | 58.5 MB |
| Git commit | `85346bb` (branch: main, repo: `github.com-td:AXIOMXLLC/ThetaData.git`) |
| Build author | William Speirs |
| Build date | 2025-07-09 |
| Proto packages | `Endpoints` (shared types), `BetaEndpoints` (v3 MDDS service) |

## 1. Download the Latest Terminal

The terminal JARs are served by the Nexus API bootstrap endpoint. No authentication is required.

```bash
# List all available versions (returns a JSON array of version strings)
curl -s https://nexus-api.thetadata.us/bootstrap/jars | python3 -m json.tool

# Grab the latest version string
VERSION=$(curl -s https://nexus-api.thetadata.us/bootstrap/jars \
    | python3 -c "import sys,json; print(json.load(sys.stdin)[-1])")
echo "Latest version: $VERSION"

# Download the JAR (redirects to CDN)
curl -L -o terminal.jar "https://nexus-api.thetadata.us/bootstrap/jars/$VERSION"
```

The CDN endpoint is `https://td-terminals.nyc3.cdn.digitaloceanspaces.com/{version}.jar` - the bootstrap URL redirects there.

## 2. Decompile with CFR

[CFR](https://www.benf.org/other/cfr/) is the recommended decompiler. You need JDK 21+ installed.

```bash
# Download CFR if you don't have it
curl -L -o cfr-0.152.jar "https://github.com/leibnitz27/cfr/releases/download/0.152/cfr-0.152.jar"

# Decompile only ThetaData packages (skip third-party deps)
java -jar cfr-0.152.jar terminal.jar \
    --outputdir decompiled/ \
    --jarfilter "net.thetadata.*"

# The interesting packages:
# decompiled/net/thetadata/fpssclient/   - FPSS streaming protocol
# decompiled/net/thetadata/fie/          - FIT/FIE codecs
# decompiled/net/thetadata/auth/         - Nexus authentication
# decompiled/net/thetadata/providers/    - MDDS gRPC channel setup
# decompiled/net/thetadata/config/       - Configuration management
# decompiled/net/thetadata/generated/    - Protobuf generated classes
```

## 3. Extract Proto Definitions

The protobuf `.proto` files are not shipped as text in the JAR. They are embedded as compiled `FileDescriptorProto` byte arrays inside the generated Java classes. Use runtime reflection to extract them.

### DumpV3Proto.java

Create `DumpV3Proto.java` (stored in `theta-terminal-re/`):

```java
import com.google.protobuf.DescriptorProtos;
import com.google.protobuf.Descriptors;

public class DumpV3Proto {
    public static void main(String[] args) throws Exception {
        // The v3 service descriptor is in the BetaEndpoints generated class
        Class<?> cls = Class.forName("net.thetadata.generated.v3grpc.Endpoints");
        java.lang.reflect.Method method = cls.getMethod("getDescriptor");
        Descriptors.FileDescriptor fd = (Descriptors.FileDescriptor) method.invoke(null);

        // Convert to proto text
        DescriptorProtos.FileDescriptorProto fdp = fd.toProto();
        System.out.println(fdp);
    }
}
```

### Running the extraction

```bash
# Extract all classes from the JAR into a working directory
mkdir -p classes && cd classes && unzip ../terminal.jar > /dev/null && cd ..

# Compile the dumper against the JAR's classpath
javac -cp terminal.jar DumpV3Proto.java -d dump/

# Run it - outputs the proto definition to stdout
java -cp "dump/:classes/" DumpV3Proto > v3_endpoints.proto

# For the shared types proto (endpoints.proto), modify the class to:
#   Class.forName("net.thetadata.generated.Endpoints")
# and run again
java -cp "dump/:classes/" DumpV3Proto > endpoints.proto
```

The extracted `.proto` files go into `proto/` in the crate root. Run `cargo build` to regenerate the Rust bindings via `tonic-prost-build` (configured in `build.rs`).

## 4. Key Java Classes and Rust Module Mapping

| Java Class | What It Contains | Rust Module |
|------------|-----------------|-------------|
| `net.thetadata.fpssclient.FPSSClient` | FPSS connection lifecycle, auth handshake, subscriptions, reconnection state machine | `fpss/mod.rs` |
| `net.thetadata.fpssclient.PacketStream` | Wire framing (1-byte len + 1-byte code + payload), request ID generation | `fpss/framing.rs` |
| `net.thetadata.fpssclient.Contract` | Contract binary serialization (stock/option/index wire format) | `fpss/protocol.rs` |
| `net.thetadata.fie.FITReader` | FIT nibble decoder (4-bit variable-length integer compression) | `crates/tdbe/src/codec/fit.rs` |
| `net.thetadata.FIE` | FIE string encoder (nibble packing for request building) | `crates/tdbe/src/codec/fie.rs` |
| `net.thetadata.auth.UserAuthenticator` | Nexus HTTP auth endpoint, terminal key constant | `auth/nexus.rs` |
| `net.thetadata.providers.ChannelProvider` | MDDS gRPC channel construction (host, port, TLS, keepalive) | `direct.rs` |
| `net.thetadata.providers.MddsConnectionManager` | MDDS v3 gRPC path (single endpoint over TLS) | `config.rs` |
| `net.thetadata.providers.FpssConnectionManager` | FPSS multi-host round-robin failover | `fpss/connection.rs` |
| `net.thetadata.generated.v3grpc.Endpoints` | v3 proto file descriptor (BetaThetaTerminal service) | `proto/v3_endpoints.proto` |
| `net.thetadata.generated.Endpoints` | Shared proto file descriptor (ResponseData, DataTable, Price) | `proto/endpoints.proto` |
| `net.thetadata.config.ConfigurationManager` | Config keys (hosts, ports, timeouts) from `config_0.properties` | `config.rs` |
| `net.thetadata.StreamMsgType` | FPSS message type enum (byte codes 0-53) | `crates/tdbe/src/types/enums.rs` |

## 5. Hardcoded Constants

All constants extracted from the decompiled source:

### Authentication

| Constant | Value | Source Java Class |
|----------|-------|-------------------|
| Nexus auth URL | `https://nexus-api.thetadata.us/identity/terminal/auth_user` | `UserAuthenticator.CLOUD_AUTH_URL` |
| Terminal API key | `cf58ada4-4175-11f0-860f-1e2e95c79e64` | `UserAuthenticator.TERMINAL_KEY` |
| Terminal key header | `TD-TERMINAL-KEY` | `UserAuthenticator.authenticateViaCloud()` |

### MDDS (Historical gRPC)

| Constant | Value | Source |
|----------|-------|--------|
| gRPC host | `mdds-01.thetadata.us` | `MddsConnectionManager` (v3 code path) |
| gRPC port | `443` | `MddsConnectionManager` |
| TLS | `true` (standard gRPC-over-TLS) | `ChannelProvider` |
| gRPC service | `BetaEndpoints.BetaThetaTerminal` | `v3_endpoints.proto` |
| RPC count | 60 methods (all server-streaming) — thetadatadx wraps all 60 as 61 `ThetaDataDx` methods via `parsed_endpoint!` macro | `v3_endpoints.proto` |

### FPSS (Real-time TCP)

| Constant | Value | Source |
|----------|-------|--------|
| NJ-A host:port | `nj-a.thetadata.us:20000`, `:20001` | `config_0.properties` `FPSS_NJ_HOSTS` |
| NJ-B host:port | `nj-b.thetadata.us:20000`, `:20001` | `config_0.properties` `FPSS_NJ_HOSTS` |
| Ping interval | 100ms | `FPSSClient.startPinging()` |
| Reconnect delay (normal) | 2,000ms | `FPSSClient.RECONNECT_DELAY_MS` |
| Reconnect delay (rate limited) | 130,000ms (130s) | `FPSSClient.handleInvoluntaryDisconnect()` |
| Connect timeout | 2,000ms | `FPSSClient.connect()` socket timeout |
| Read timeout | 10,000ms | `FPSSClient.connect()` SO_TIMEOUT |
| TCP_NODELAY | `true` | `FPSSClient.connect()` |

### FIT/FIE Codec

| Constant | Value | Source |
|----------|-------|--------|
| SPACING | 5 (ROW_SEP jumps to field index 5) | `FITReader.readChanges()` |
| DATE marker | `0xCE` (-50 as signed byte) | `FIE.DATE` |
| FIELD_SEPARATOR nibble | `0xB` | `FITReader` |
| ROW_SEPARATOR nibble | `0xC` | `FITReader` |
| END nibble | `0xD` | `FITReader` |
| NEGATIVE nibble | `0xE` | `FITReader` |

### Bootstrap / CDN

| Constant | Value | Source |
|----------|-------|--------|
| Bootstrap JAR list URL | `https://nexus-api.thetadata.us/bootstrap/jars` | `default_config.toml` |
| Bootstrap JAR download | `https://nexus-api.thetadata.us/bootstrap/jars/{version}` | `default_config.toml` |
| CDN direct URL | `https://td-terminals.nyc3.cdn.digitaloceanspaces.com/{version}.jar` | HTTP redirect target |

## 6. What to Check on Terminal Updates

When ThetaData releases a new terminal version, here is the checklist:

### Authentication changes

1. Check `UserAuthenticator.java` for changes to `CLOUD_AUTH_URL` or `TERMINAL_KEY`
2. Check the auth request/response JSON format
3. Update `auth/nexus.rs` if anything changed

### gRPC / MDDS changes

1. Re-extract protos using Steps 1-3 above
2. Diff `v3_endpoints.proto` and `endpoints.proto` against the previous versions
3. Look for new RPC methods - add a `parsed_endpoint!` invocation in `direct.rs`
4. Look for changed request/response types - update `decode.rs` parsers
5. Check `ChannelProvider.java` for host/port changes - update `config.rs`

### FPSS changes

1. Check `StreamMsgType.java` for new or changed message codes - update `crates/tdbe/src/types/enums.rs` and `fpss/framing.rs`
2. Check `FPSSClient.java` for changes to the connection/auth/reconnection state machine - update `fpss/mod.rs`
3. Check `PacketStream.java` for framing changes - update `fpss/framing.rs`
4. Check `Contract.java` for wire format changes - update `fpss/protocol.rs`
5. Check `FPSSClient.SERVERS` for new/changed server addresses - update `fpss/protocol.rs` `SERVERS` const

### Codec changes

1. Check `FITReader.java` for changes to nibble encoding or the SPACING constant - update `crates/tdbe/src/codec/fit.rs`
2. Check `FIE.java` for changes to the nibble alphabet - update `crates/tdbe/src/codec/fie.rs`
3. Run the existing test suite - FIT test vectors will catch encoding changes

### Configuration changes

1. Check `config_0.properties` embedded in the JAR for new hosts, ports, or timeouts
2. Check `ConfigurationManager.java` for new config keys
3. Update `config.rs` with any new defaults

## 7. Useful Decompilation Commands

```bash
# Search for a specific string in decompiled source
grep -r "session_uuid" decompiled/net/thetadata/

# Find all hardcoded URLs
grep -rn "https://" decompiled/net/thetadata/

# Find all constants in auth code
grep -rn "static final" decompiled/net/thetadata/auth/

# Find FPSS message code definitions
grep -rn "StreamMsgType" decompiled/net/thetadata/

# List all gRPC method names
grep -rn "getMethod" decompiled/net/thetadata/generated/v3grpc/

# Find config property keys
grep -rn "getProperty\|config_0" decompiled/net/thetadata/config/
```
