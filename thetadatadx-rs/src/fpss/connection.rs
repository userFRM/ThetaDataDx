//! TLS TCP connection to FPSS servers.
//!
//! # Transport
//!
//! TLS over TCP with:
//! - `TCP_NODELAY = true` (Nagle disabled for low latency)
//! - `SO_KEEPALIVE` armed with an aggressive probe schedule (default
//!   5 s idle / 2 s interval / 2 probes ≈ 9 s kernel-side half-open
//!   detection) so a peer that vanishes without a FIN/RST is detected
//!   by the transport long before the platform default of 2+ hours
//! - Connect timeout: 2 seconds (configurable)
//! - Read timeout: configurable (default 10 seconds)
//! - Host order: fault-domain-aware per-client shuffle by default (see
//!   [`order_hosts`]), `FixedOrder` escape hatch preserves declaration
//!   order
//!
//! # Implementation
//!
//! Uses `std::net::TcpStream` + `rustls::StreamOwned` for blocking TLS I/O.
//! No tokio, no async -- pure blocking I/O on `std::thread`.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Once};
use std::time::Duration;

use rand::seq::SliceRandom;
use rand::SeedableRng;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use crate::auth::Credentials;
use crate::backoff::JitterMode;
use crate::config::HostSelectionPolicy;
use crate::config::ReconnectPolicy;
use crate::config::StreamingFlushMode;

use super::pinning::PinnedVerifier;
#[cfg(test)]
use super::protocol::CONNECT_TIMEOUT_MS;

/// Type alias for the TLS-wrapped TCP stream (blocking).
pub type FpssStream = StreamOwned<ClientConnection, TcpStream>;

/// TCP keepalive schedule applied to every FPSS socket.
///
/// Mirrors the three `StreamingConfig` keepalive knobs; bundled so the
/// connect path takes one argument instead of three loose integers.
#[derive(Debug, Clone, Copy)]
pub struct TcpKeepaliveSpec {
    /// Idle time before the first probe.
    pub idle: Duration,
    /// Interval between unanswered probes.
    pub interval: Duration,
    /// Probe count before the kernel declares the peer dead. Applied
    /// only on platforms that expose the knob.
    pub retries: u32,
}

/// Parameter bundle for [`super::StreamingClient::connect_with_stream`].
///
/// Carries every connection-side knob plus the user callback. Bundled
/// into a struct so the call site stays linear instead of as a
/// positional list of a dozen heterogeneous arguments.
pub(crate) struct ConnectWithStreamArgs<'a> {
    pub creds: &'a Credentials,
    pub stream: FpssStream,
    pub server_addr: String,
    /// Declared FPSS host list. The initial connect applies
    /// [`order_hosts`] with `preferred = None`; reconnects may promote
    /// the last stable host while re-running the policy on the tail.
    pub hosts: &'a [(String, u16)],
    pub host_selection: HostSelectionPolicy,
    pub host_shuffle_seed: u64,
    pub ring_size: usize,
    pub flush_mode: StreamingFlushMode,
    /// Fixed low-latency event-ring consumer wait strategy
    /// ([`super::ring::AdaptiveWaitStrategy`]).
    pub wait_strategy: super::ring::AdaptiveWaitStrategy,
    /// Optional CPU core to pin the event-ring consumer thread to;
    /// `None` leaves it under the OS scheduler. Mirrors
    /// [`crate::config::StreamingConfig::consumer_cpu`].
    pub consumer_cpu: Option<usize>,
    pub policy: ReconnectPolicy,
    /// Initial reconnect delay (ms) for generic transient drops;
    /// doubles per attempt up to `wait_max_ms`. Mirrors
    /// [`crate::config::ReconnectConfig::wait_ms`].
    pub wait_ms: u64,
    /// Cap (ms) on the generic-transient reconnect ladder. Mirrors
    /// [`crate::config::ReconnectConfig::wait_max_ms`].
    pub wait_max_ms: u64,
    /// Reconnect floor (ms) for `TooManyRequests` drops. Mirrors
    /// [`crate::config::ReconnectConfig::wait_rate_limited_ms`].
    pub wait_rate_limited_ms: u64,
    /// Flat reconnect cadence (ms) for `ServerRestarting` drops.
    /// Mirrors [`crate::config::ReconnectConfig::wait_server_restart_ms`].
    pub wait_server_restart_ms: u64,
    /// Jitter strategy for every reconnect delay. Mirrors
    /// [`crate::config::ReconnectConfig::jitter`].
    pub jitter: JitterMode,
    /// Subscription-replay burst size after reconnect. Mirrors
    /// [`crate::config::ReconnectConfig::replay_burst_size`].
    pub replay_burst_size: u32,
    /// Pause (ms) between replay bursts. Mirrors
    /// [`crate::config::ReconnectConfig::replay_pace_ms`].
    pub replay_pace_ms: u64,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    /// Per-iteration blocking-read slice for the I/O loop. Mirrors
    /// [`crate::config::StreamingConfig::io_read_slice_ms`].
    pub io_read_slice: Duration,
    /// Keepalive schedule for reconnect-time socket construction.
    pub keepalive: TcpKeepaliveSpec,
    pub ping_interval: Duration,
}

/// Install the process-global rustls crypto provider exactly once.
///
/// Embedded consumers such as the Python SDK or FFI bindings do not always
/// have a top-level binary `main()` that installs this ahead of time.
fn ensure_rustls_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Apply the configured [`HostSelectionPolicy`] to the declared host
/// list, producing the per-client connect/failover order.
///
/// Under [`HostSelectionPolicy::FixedOrder`] the declared order is
/// preserved verbatim. Under [`HostSelectionPolicy::Shuffled`] (the
/// default) the hosts are grouped by hostname — each hostname is one
/// fault domain — the group order and the ports within each group are
/// shuffled with the supplied seed, and the result interleaves across
/// groups round-robin. Two properties follow:
///
/// * **Fleet spread** — clients with independent seeds distribute
///   their first connect uniformly across the fault domains instead of
///   all dialling the first declared host.
/// * **Cross-domain failover** — consecutive attempts alternate fault
///   domains, so the second attempt lands on a different physical
///   machine rather than a second port on the machine that just
///   failed.
///
/// The seed makes the order deterministic: tests and fleet-sharding
/// deployments pass a fixed seed, production defaults derive a fresh
/// per-client seed from process-local entropy.
pub(crate) fn order_hosts(
    hosts: &[(String, u16)],
    policy: HostSelectionPolicy,
    seed: u64,
    preferred: Option<usize>,
) -> Vec<(String, u16)> {
    let preferred = preferred.and_then(|idx| hosts.get(idx).map(|host| (idx, host.clone())));
    let preferred_idx = preferred.as_ref().map(|(idx, _)| *idx);
    let mut ordered = match policy {
        HostSelectionPolicy::FixedOrder => hosts
            .iter()
            .enumerate()
            .filter(|(idx, _)| preferred_idx != Some(*idx))
            .map(|(_, host)| host.clone())
            .collect(),
        // `HostSelectionPolicy` is non_exhaustive; route any future
        // variant added without an arm here to the safe default.
        _ => {
            // Group by hostname, preserving first-seen group order as
            // the pre-shuffle baseline.
            let mut groups: Vec<(String, Vec<u16>)> = Vec::new();
            for (idx, (host, port)) in hosts.iter().enumerate() {
                if preferred_idx == Some(idx) {
                    continue;
                }
                match groups.iter_mut().find(|(h, _)| h == host) {
                    Some((_, ports)) => ports.push(*port),
                    None => groups.push((host.clone(), vec![*port])),
                }
            }
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            groups.shuffle(&mut rng);
            for (_, ports) in &mut groups {
                ports.shuffle(&mut rng);
            }
            // Round-robin interleave across groups: one port per
            // group per round, so consecutive entries cross fault
            // domains whenever more than one domain exists.
            let mut ordered = Vec::with_capacity(hosts.len());
            let mut round = 0;
            loop {
                let mut emitted = false;
                for (host, ports) in &groups {
                    if let Some(port) = ports.get(round) {
                        ordered.push((host.clone(), *port));
                        emitted = true;
                    }
                }
                if !emitted {
                    break;
                }
                round += 1;
            }
            ordered
        }
    };
    if let Some((_, preferred_host)) = preferred {
        ordered.insert(0, preferred_host);
    }
    ordered
}

/// Establish a TLS connection to the first reachable FPSS server.
///
/// Tries each server in order. Returns the stream and connected server
/// address on success, or the last error if all fail. Checks `shutdown`
/// between host attempts so a Drop raised mid-reconnect aborts the dial
/// loop instead of blocking through every remaining host's dial + login.
///
/// # Connection sequence
///
/// 1. TCP connect with the configured timeout
/// 2. `TCP_NODELAY = true`
/// 3. `SO_KEEPALIVE` armed per `keepalive`
/// 4. Read timeout set to `read_timeout`
/// 5. Write timeout set to `write_timeout`
/// 6. TLS handshake pinned to the FPSS SPKI
///
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn connect_to_servers(
    servers: &[(&str, u16)],
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
    keepalive: TcpKeepaliveSpec,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<(FpssStream, String), crate::error::Error> {
    ensure_rustls_crypto_provider();
    let mut last_err = None;

    for &(host, port) in servers {
        // A Drop raised mid-reconnect must not be blocked for the full
        // dial + login of every remaining host. Check between attempts so a
        // shutting-down thread stops trying rather than dialling on.
        if shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(crate::error::Error::Stream {
                kind: crate::error::StreamErrorKind::Disconnected,
                message: "connection aborted: client shutting down".to_string(),
            });
        }
        let addr = format!("{host}:{port}");
        tracing::debug!(server = %addr, "attempting FPSS connection");

        match try_connect(
            host,
            port,
            connect_timeout,
            read_timeout,
            write_timeout,
            keepalive,
        ) {
            Ok(stream) => {
                tracing::info!(server = %addr, "FPSS connected");
                return Ok((stream, addr));
            }
            Err(e) => {
                tracing::warn!(server = %addr, error = %e, "FPSS connection failed");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| crate::error::Error::Stream {
        kind: crate::error::StreamErrorKind::ConnectionRefused,
        message: "no servers configured".to_string(),
    }))
}

/// Build a shared rustls `ClientConfig` pinned to the `ThetaData` FPSS
/// `SubjectPublicKeyInfo`.
///
/// `ThetaData`'s FPSS servers use TLS certificates that have been expired
/// since January 2024, so the stock `webpki` chain + validity check cannot
/// succeed. Instead of disabling verification entirely (which converts the
/// expiry problem into an open **MITM + credential harvest** hole -- the
/// very next frame after the handshake contains the user's email + password),
/// we pin on the leaf's SPKI. See [`super::pinning`] for the full rationale.
fn tls_client_config() -> Result<Arc<ClientConfig>, crate::error::Error> {
    // Build the config with an explicit ring provider so the handshake needs
    // no process-global default. ring is the sole provider in the dep graph.
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(PinnedVerifier::new())
            .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Arm `SO_KEEPALIVE` on the freshly-connected socket.
///
/// Best-effort: a kernel that rejects the schedule (or a platform
/// without per-socket retry control) degrades to whatever subset it
/// accepts, with a `warn` so operators can see the reduced transport
/// coverage. The application-level read timeout remains the primary
/// liveness check.
fn arm_keepalive(tcp: &TcpStream, spec: TcpKeepaliveSpec) {
    let sock = socket2::SockRef::from(tcp);
    let base = socket2::TcpKeepalive::new()
        .with_time(spec.idle)
        .with_interval(spec.interval);
    // Per-socket probe count is not exposed on every platform;
    // socket2 cfg-gates the setter to the platforms that support it.
    #[cfg(not(windows))]
    let ka = base.with_retries(spec.retries);
    #[cfg(windows)]
    let ka = {
        let _ = spec.retries;
        base
    };
    if let Err(e) = sock.set_tcp_keepalive(&ka) {
        tracing::warn!(
            error = %e,
            "failed to arm TCP keepalive on FPSS socket; \
             relying on application-level timeouts only"
        );
    }
}

/// Attempt a single blocking TLS connection to one server.
///
/// # Steps
///
/// 1. `TcpStream::connect_timeout`
/// 2. `set_nodelay(true)`
/// 3. `SO_KEEPALIVE` per the configured schedule
/// 4. `set_read_timeout`
/// 5. `set_write_timeout`
/// 6. Blocking TLS handshake via rustls `StreamOwned`
fn try_connect(
    host: &str,
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
    keepalive: TcpKeepaliveSpec,
) -> Result<FpssStream, crate::error::Error> {
    let addr = format!("{host}:{port}");

    // Resolve the hostname via the OS DNS resolver. This handles both IP
    // addresses and hostnames (e.g., "nj-a.thetadata.us:20000"). The
    // previous implementation used `SocketAddr::parse()` which only
    // accepts numeric IP addresses and would fail for DNS hostnames.
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ConnectionRefused,
            message: format!("DNS resolution failed for '{addr}': {e}"),
        })?
        .next()
        .ok_or_else(|| crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ConnectionRefused,
            message: format!("DNS resolution returned no addresses for '{addr}'"),
        })?;

    // TCP connect with timeout
    let tcp = TcpStream::connect_timeout(&sock_addr, connect_timeout)?;

    // TCP_NODELAY = true (socket.setTcpNoDelay(true)).
    tcp.set_nodelay(true)?;

    // SO_KEEPALIVE: kernel-side half-open detection in
    // ~(idle + interval * retries) seconds, versus the platform
    // default of 2+ hours.
    arm_keepalive(&tcp, keepalive);

    // Read timeout.
    tcp.set_read_timeout(Some(read_timeout))?;

    // Write timeout. The first write (CREDENTIALS) drives the lazy TLS
    // handshake, and steady-state ping/subscribe writes can otherwise
    // block indefinitely against a peer whose receive window has stalled
    // (alive enough to ACK at the kernel but not draining the socket).
    // `connect_timeout` only bounds the SYN/ACK, so an unbounded write
    // would wedge the I/O thread past that budget. The bound persists for
    // the life of the socket via `SO_SNDTIMEO`, so a write `TimedOut`
    // surfaces as a fatal I/O error and the caller reconnects, mirroring
    // the read-timeout liveness contract.
    tcp.set_write_timeout(Some(write_timeout))?;

    // TLS handshake (blocking) using rustls with webpki root certificates.
    let server_name =
        ServerName::try_from(host.to_owned()).map_err(|e| crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ConnectionRefused,
            message: format!("invalid TLS server name '{host}': {e}"),
        })?;

    let tls_conn = ClientConnection::new(tls_client_config()?, server_name).map_err(|e| {
        crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ConnectionRefused,
            message: format!("TLS setup for {addr} failed: {e}"),
        }
    })?;

    // StreamOwned performs the TLS handshake lazily on first read/write.
    // The first write_frame (CREDENTIALS) will drive the handshake to completion.
    let tls_stream = StreamOwned::new(tls_conn, tcp);

    Ok(tls_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keepalive() -> TcpKeepaliveSpec {
        TcpKeepaliveSpec {
            idle: Duration::from_secs(5),
            interval: Duration::from_secs(2),
            retries: 2,
        }
    }

    #[test]
    fn rustls_crypto_provider_install_is_idempotent() {
        ensure_rustls_crypto_provider();
        ensure_rustls_crypto_provider();
    }

    #[test]
    fn production_config_has_four_fpss_hosts() {
        let config = crate::config::DirectConfig::production();
        assert_eq!(config.streaming.hosts.len(), 4);
        assert_eq!(
            config.streaming.hosts[0],
            ("nj-a.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.streaming.hosts[1],
            ("nj-a.thetadata.us".to_string(), 20001)
        );
        assert_eq!(
            config.streaming.hosts[2],
            ("nj-b.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.streaming.hosts[3],
            ("nj-b.thetadata.us".to_string(), 20001)
        );
    }

    #[test]
    fn connect_timeout_matches_terminal() {
        // Parity reference: the JVM terminal connects with a 2000 ms deadline.
        // Used as the default seed for `StreamingConfig::connect_timeout_ms`; the
        // public knob now overrides this constant for callers who need to
        // dial in a different per-server connect deadline.
        assert_eq!(CONNECT_TIMEOUT_MS, 2_000);
    }

    /// `connect_to_servers` honours the caller-supplied connect timeout.
    ///
    /// We dial a non-routable RFC-5737 address. With a short
    /// `connect_timeout` the call must fail within that budget plus
    /// kernel scheduling slack; without timeout plumbing it would block
    /// for the kernel default (~75 s on Linux).
    ///
    /// This is the load-bearing assertion that `connect_timeout_ms`
    /// flows from `FpssConnectArgs` -> `connect_to_servers` ->
    /// `TcpStream::connect_timeout`.
    #[test]
    fn connect_to_servers_honors_configured_connect_timeout() {
        // 192.0.2.0/24 is the RFC-5737 TEST-NET-1 block — guaranteed
        // to be unroutable. A connect against any address in this
        // range must hit the configured deadline.
        let servers = [("192.0.2.1", 1)];
        let connect_timeout = Duration::from_millis(150);
        let read_timeout = Duration::from_millis(10_000);
        let start = std::time::Instant::now();
        let write_timeout = Duration::from_millis(10_000);
        let res = connect_to_servers(
            &servers,
            connect_timeout,
            read_timeout,
            write_timeout,
            test_keepalive(),
            &std::sync::atomic::AtomicBool::new(false),
        );
        let elapsed = start.elapsed();
        assert!(res.is_err(), "unroutable host must fail to connect");
        assert!(
            elapsed < Duration::from_millis(2_000),
            "connect_timeout = 150 ms but elapsed = {elapsed:?}; the knob is not wired"
        );
    }

    /// The keepalive schedule is applied to a real socket without
    /// error on this platform. Uses a loopback listener so the connect
    /// succeeds and the socket options are actually set.
    #[test]
    fn keepalive_arms_on_loopback_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let tcp = TcpStream::connect(addr).expect("connect loopback");
        arm_keepalive(&tcp, test_keepalive());
        let sock = socket2::SockRef::from(&tcp);
        assert!(
            sock.keepalive().expect("read SO_KEEPALIVE"),
            "SO_KEEPALIVE must be armed after arm_keepalive"
        );
    }

    /// Both the read and write timeouts round-trip onto a real socket.
    ///
    /// `try_connect` sets `SO_RCVTIMEO` and `SO_SNDTIMEO` before the TLS
    /// handshake. The handshake itself needs a live FPSS peer, so this
    /// asserts the load-bearing socket-option contract directly on a
    /// loopback socket: an unbounded write would let a stalled-receiver
    /// peer wedge the I/O thread past `connect_timeout`, so the write
    /// timeout must actually land on the kernel socket.
    #[test]
    fn read_and_write_timeouts_arm_on_loopback_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let tcp = TcpStream::connect(addr).expect("connect loopback");

        let read_timeout = Duration::from_millis(7_000);
        let write_timeout = Duration::from_millis(3_000);
        tcp.set_read_timeout(Some(read_timeout))
            .expect("set read timeout");
        tcp.set_write_timeout(Some(write_timeout))
            .expect("set write timeout");

        assert_eq!(
            tcp.read_timeout().expect("read SO_RCVTIMEO"),
            Some(read_timeout),
            "read timeout must round-trip onto the socket"
        );
        assert_eq!(
            tcp.write_timeout().expect("read SO_SNDTIMEO"),
            Some(write_timeout),
            "write timeout must round-trip onto the socket"
        );
    }

    fn production_hosts() -> Vec<(String, u16)> {
        vec![
            ("nj-a.thetadata.us".to_string(), 20000),
            ("nj-a.thetadata.us".to_string(), 20001),
            ("nj-b.thetadata.us".to_string(), 20000),
            ("nj-b.thetadata.us".to_string(), 20001),
        ]
    }

    #[test]
    fn order_hosts_fixed_order_preserves_declaration() {
        let hosts = production_hosts();
        let ordered = order_hosts(&hosts, HostSelectionPolicy::FixedOrder, 42, None);
        assert_eq!(ordered, hosts);
    }

    /// The shuffled order is deterministic for a given seed — the
    /// load-bearing property for fleet sharding and for this test
    /// suite itself.
    #[test]
    fn order_hosts_shuffled_is_deterministic_per_seed() {
        let hosts = production_hosts();
        let a = order_hosts(&hosts, HostSelectionPolicy::Shuffled, 7, None);
        let b = order_hosts(&hosts, HostSelectionPolicy::Shuffled, 7, None);
        assert_eq!(a, b, "same seed must produce the same order");
        // A different seed must be able to produce a different order;
        // with 2 groups x 2 ports there are 8 distinct outcomes, so
        // scanning a small seed range must find at least one divergence.
        let found_divergent =
            (0..32_u64).any(|s| order_hosts(&hosts, HostSelectionPolicy::Shuffled, s, None) != a);
        assert!(found_divergent, "shuffle must depend on the seed");
    }

    /// Consecutive entries must alternate fault domains (hostnames)
    /// whenever more than one domain exists — the property that makes
    /// the first failover land on a different physical machine.
    #[test]
    fn order_hosts_shuffled_interleaves_fault_domains() {
        let hosts = production_hosts();
        for seed in 0..64_u64 {
            let ordered = order_hosts(&hosts, HostSelectionPolicy::Shuffled, seed, None);
            assert_eq!(ordered.len(), 4, "no hosts may be lost");
            // Same multiset of entries.
            let mut sorted_in = hosts.clone();
            let mut sorted_out = ordered.clone();
            sorted_in.sort();
            sorted_out.sort();
            assert_eq!(sorted_in, sorted_out, "shuffle must be a permutation");
            // First two entries cross fault domains.
            assert_ne!(
                ordered[0].0, ordered[1].0,
                "seed {seed}: first failover must cross fault domains; got {ordered:?}"
            );
            // Full alternation for the 2x2 production shape.
            assert_ne!(ordered[2].0, ordered[3].0);
        }
    }

    /// First-host distribution: across seeds, both fault domains must
    /// appear in the first slot — the steady-state load-spreading
    /// property.
    #[test]
    fn order_hosts_shuffled_spreads_first_host_across_domains() {
        let hosts = production_hosts();
        let mut first_hosts = std::collections::HashSet::new();
        for seed in 0..64_u64 {
            let ordered = order_hosts(&hosts, HostSelectionPolicy::Shuffled, seed, None);
            first_hosts.insert(ordered[0].0.clone());
        }
        assert_eq!(
            first_hosts.len(),
            2,
            "both fault domains must appear as the first connect target across seeds"
        );
    }

    /// Degenerate shapes: a single host, and asymmetric port counts
    /// per domain, must survive the interleave without loss.
    #[test]
    fn order_hosts_handles_degenerate_shapes() {
        let single = vec![("nj-a.thetadata.us".to_string(), 20000)];
        assert_eq!(
            order_hosts(&single, HostSelectionPolicy::Shuffled, 1, None),
            single
        );

        let asymmetric = vec![
            ("a.example".to_string(), 1),
            ("a.example".to_string(), 2),
            ("a.example".to_string(), 3),
            ("b.example".to_string(), 9),
        ];
        let ordered = order_hosts(&asymmetric, HostSelectionPolicy::Shuffled, 5, None);
        assert_eq!(ordered.len(), 4);
        let mut sorted_in = asymmetric.clone();
        let mut sorted_out = ordered.clone();
        sorted_in.sort();
        sorted_out.sort();
        assert_eq!(sorted_in, sorted_out);
    }

    #[test]
    fn order_hosts_reconnect_pins_last_known_good_host_first() {
        let hosts = production_hosts();
        let ordered = order_hosts(&hosts, HostSelectionPolicy::FixedOrder, 42, Some(2));
        assert_eq!(ordered[0], hosts[2]);
    }

    #[test]
    fn order_hosts_reconnect_tail_still_follows_policy() {
        let hosts = production_hosts();
        let preferred = 2;
        let ordered = order_hosts(&hosts, HostSelectionPolicy::Shuffled, 17, Some(preferred));
        let tail_hosts: Vec<(String, u16)> = hosts
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != preferred)
            .map(|(_, host)| host.clone())
            .collect();
        let tail = order_hosts(&tail_hosts, HostSelectionPolicy::Shuffled, 17, None);
        assert_eq!(ordered[0], hosts[preferred]);
        assert_eq!(&ordered[1..], tail);
    }

    #[test]
    fn order_hosts_cold_connect_remains_pure_policy() {
        let hosts = production_hosts();
        let ordered = order_hosts(&hosts, HostSelectionPolicy::FixedOrder, 42, None);
        assert_eq!(ordered, hosts);
    }

    #[test]
    fn order_hosts_shuffled_tail_still_varies_after_pinning() {
        let hosts = production_hosts();
        let preferred = 2;
        let tails: std::collections::HashSet<Vec<(String, u16)>> = (0..32_u64)
            .map(|seed| {
                let ordered =
                    order_hosts(&hosts, HostSelectionPolicy::Shuffled, seed, Some(preferred));
                assert_eq!(ordered[0], hosts[preferred]);
                ordered[1..].to_vec()
            })
            .collect();
        assert!(
            tails.len() > 1,
            "pinning the first reconnect target must still leave a shuffled tail"
        );
    }
}
