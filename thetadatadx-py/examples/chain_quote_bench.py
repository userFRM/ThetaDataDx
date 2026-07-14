"""Full-chain option quote streaming benchmark.

Pulls an entire option chain's quote history over the streaming market-data
endpoint and reports TTFB, throughput, and an approximate in-memory decoded
volume so the effect of the h2 flow-control window sizes can be measured
against the live backend.

Ported from `thetadatadx-rs/examples/chain_quote_bench.rs` -- see that file
for the semantics this mirrors.

Usage:
    python chain_quote_bench.py [symbol] [expiration] [date] [interval]

Args (all optional):
  symbol      option root (default SPXW)
  expiration  contract expiration YYYYMMDD (default 20260710)
  date        history date YYYYMMDD (default = expiration, i.e. a 0DTE pull)
  interval    tick | 1s | 1m | ... (default tick)

The h2 windows are set on the config before connect via
STREAM_WINDOW_SIZE_KB / CONNECTION_WINDOW_SIZE_KB below -- edit those
constants and rerun to benchmark different values (both are clamped into
[64, 2_097_151] KB by `validate`, which `MarketDataClient.__init__` runs).
This binding has no post-connect (validated) config read-back, so the
constants themselves are printed as the "effective" values every run.

Credentials are loaded from $CREDS (default ./creds.txt).
"""

from __future__ import annotations

import os
import sys
import time

from thetadatadx import Config, Credentials, MarketDataClient

USAGE = """\
usage: chain_quote_bench.py [symbol] [expiration] [date] [interval]
       defaults: SPXW 20260710 <expiration> tick
       date defaults to <expiration> (a 0DTE full-chain pull)
       h2 windows come from the STREAM_WINDOW_SIZE_KB / CONNECTION_WINDOW_SIZE_KB
       constants in this file; edit and rerun to test different values
       credentials from $CREDS (default ./creds.txt)"""

# h2 flow-control windows applied to Config's `market_data_stream_window_size_kb`
# / `market_data_connection_window_size_kb` properties before connect. Edit
# and rerun to benchmark different values; `validate` clamps both into
# [64, 2_097_151] KB at connect.
STREAM_WINDOW_SIZE_KB = 8_192
# STREAM_WINDOW_SIZE_KB = 64
CONNECTION_WINDOW_SIZE_KB = 32_768
# CONNECTION_WINDOW_SIZE_KB = 64

MIB = 1024.0 * 1024.0

# Decoded in-memory size of a single parsed quote row. The `.stream()`
# callback only exposes a list of rows, so decoded volume is approximated as
# `rows * QUOTE_TICK_SIZE` -- an in-memory figure, not wire bytes. Mirrors
# `std::mem::size_of::<QuoteTick>()` in the Rust example (measured 128 bytes:
# `#[repr(C, align(64))]`, 13 fields) so figures are comparable across
# bindings.
QUOTE_TICK_SIZE = 128


def arg_or(args: list[str], idx: int, default: str) -> str:
    return args[idx] if idx < len(args) else default


def human_bytes(n: int) -> str:
    f = float(n)
    gib = MIB * 1024.0
    if f >= gib:
        return f"{f / gib:.2f} GiB"
    return f"{f / MIB:.2f} MiB"


def main() -> None:
    args = sys.argv[1:]
    if any(a in ("-h", "--help") for a in args):
        print(USAGE)
        return
    if len(args) > 4:
        print(USAGE, file=sys.stderr)
        sys.exit(2)

    symbol = arg_or(args, 0, "SPXW")
    expiration = arg_or(args, 1, "20260710")
    date = arg_or(args, 2, expiration)
    interval = arg_or(args, 3, "tick")

    creds_path = os.environ.get("CREDS", "creds.txt")
    try:
        creds = Credentials.from_file(creds_path)
    except Exception as e:  # noqa: BLE001 -- mirrors the Rust example's blanket `Err(e)`
        print(f"creds load failed ({creds_path}): {e}", file=sys.stderr)
        sys.exit(1)

    # production() supplies the defaults; the benchmark constants override
    # the h2 window knobs before connect, which clamps the applied values
    # into [64, 2_097_151] KB via validate.
    config = Config.production()
    config.market_data_stream_window_size_kb = STREAM_WINDOW_SIZE_KB
    config.market_data_connection_window_size_kb = CONNECTION_WINDOW_SIZE_KB

    connect_start = time.monotonic()
    try:
        client = MarketDataClient(creds, config)
    except Exception as e:  # noqa: BLE001
        print(f"connect failed: {e}", file=sys.stderr)
        sys.exit(1)
    connect_elapsed = time.monotonic() - connect_start

    # Effective h2 window sizes, so every run is self-documenting. No
    # post-connect (validated) config read-back exists on this binding, so
    # the constants are printed as-is; validate clamps both into
    # [64, 2_097_151] KB at connect.
    stream_window_size_kb = config.market_data_stream_window_size_kb
    connection_window_size_kb = config.market_data_connection_window_size_kb
    print(
        f"[bench] effective h2 windows: stream={stream_window_size_kb} KB, "
        f"connection={connection_window_size_kb} KB",
        file=sys.stderr,
    )
    print(
        f"[bench] streaming option_history_quote {symbol} exp={expiration} date={date} "
        f"interval={interval} strike=* right=both (no deadline)",
        file=sys.stderr,
    )

    rows = 0
    chunks = 0
    ttfb: float | None = None
    last_log = time.monotonic()

    # Dispatch clock: started immediately before dispatching the stream call
    # so TTFB excludes connect/auth and measures backend-to-first-chunk latency.
    dispatch = time.monotonic()

    def on_chunk(chunk: list) -> None:
        nonlocal rows, chunks, ttfb, last_log
        if ttfb is None:
            ttfb = time.monotonic() - dispatch
        rows += len(chunk)
        chunks += 1
        # Lightweight liveness so a 10-minute pull is not silent.
        now = time.monotonic()
        if now - last_log >= 10.0:
            last_log = now
            secs = max(now - dispatch, sys.float_info.epsilon)
            approx_mib = (rows * QUOTE_TICK_SIZE) / MIB
            print(
                f"[bench] +{secs:6.0f}s rows={rows} chunks={chunks} "
                f"~{approx_mib:.2f} MiB in-mem ({approx_mib / secs:.2f} MiB/s)",
                file=sys.stderr,
            )

    # A full-day 0DTE pull can run 6-15 minutes; the config default
    # request_timeout_secs (300 s) would kill it, so opt out of any deadline
    # with timeout_ms=0 (normalized to "no deadline" by effective_deadline).
    try:
        (
            client.option_history_quote_builder(symbol, expiration)
            .strike("*")
            .right("both")
            .date(date)
            .interval(interval)
            .timeout_ms(0)
            .stream(on_chunk)
        )
    except Exception as e:  # noqa: BLE001
        total = time.monotonic() - dispatch
        print(f"stream failed after {total:.1f}s: {e}", file=sys.stderr)
        sys.exit(1)

    total = time.monotonic() - dispatch
    secs = max(total, sys.float_info.epsilon)
    ttfb_secs = ttfb if ttfb is not None else 0.0
    # Approximate decoded VOLUME (in-memory, not wire): the `.stream()`
    # callback exposes only parsed rows, so multiply the row count by the
    # decoded row size. This is a lower bound on RSS (ignores per-row heap)
    # and is unrelated to the compressed bytes that crossed the h2 window.
    approx_decoded = rows * QUOTE_TICK_SIZE

    # Greppable key=value block on stdout; progress/logs stay on stderr.
    print(f"symbol={symbol}")
    print(f"expiration={expiration}")
    print(f"date={date}")
    print(f"interval={interval}")
    print(f"stream_window_size_kb={stream_window_size_kb}")
    print(f"connection_window_size_kb={connection_window_size_kb}")
    print(f"connect_auth_secs={connect_elapsed:.3f}")
    print(f"ttfb_secs={ttfb_secs:.3f}")
    print(f"total_secs={secs:.3f}")
    print(f"rows={rows}")
    print(f"chunks={chunks}")
    print(f"rows_per_sec={rows / secs:.1f}")
    print(f"quote_tick_size_bytes={QUOTE_TICK_SIZE}")
    print(f"approx_decoded_bytes={approx_decoded}")
    print(f"approx_decoded={human_bytes(approx_decoded)}")
    print(f"approx_decoded_bytes_per_sec={approx_decoded / secs:.0f}")
    print(f"approx_rate_mib_per_sec={approx_decoded / secs / MIB:.2f}")
    print(
        f"# approx_decoded* is in-memory volume = rows x QUOTE_TICK_SIZE "
        f"({QUOTE_TICK_SIZE} B); not wire bytes"
    )


if __name__ == "__main__":
    main()
