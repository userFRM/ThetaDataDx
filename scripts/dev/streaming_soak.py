#!/usr/bin/env python3
"""Exercise streaming reconnect + subscription restoration against the replay servers."""

from __future__ import annotations

import argparse
import queue
import threading
import time


def _drain_data_kind(events: "queue.Queue", *, timeout_secs: float) -> tuple[str, str]:
    deadline = time.monotonic() + timeout_secs
    last_kind = "none"
    while time.monotonic() < deadline:
        try:
            event = events.get(timeout=0.5)
        except queue.Empty:
            continue
        last_kind = event.kind
        if last_kind in {"quote", "trade", "open_interest", "ohlcvc"}:
            return event.contract.symbol, last_kind
    raise RuntimeError(f"timed out waiting for data event (last kind={last_kind})")


def _subscriptions_snapshot(stream) -> set[tuple[str, str]]:
    subs = stream.active_subscriptions()
    return {(entry["kind"], entry["contract"]) for entry in subs}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("creds", help="Path to creds.txt")
    parser.add_argument("--symbol", default="AAPL", help="Replay symbol to subscribe to")
    parser.add_argument(
        "--duration-secs",
        type=int,
        default=120,
        help="Total soak duration in seconds",
    )
    parser.add_argument(
        "--reconnects",
        type=int,
        default=3,
        help="Number of explicit reconnect cycles to require",
    )
    args = parser.parse_args()

    from thetadatadx import Config, Contract, Credentials, Client  # type: ignore

    cfg = Config.dev()
    cfg.reconnect_policy = "manual"
    cfg.derive_ohlcvc = False
    client = Client(Credentials.from_file(args.creds), cfg)

    events: "queue.Queue" = queue.Queue(maxsize=8192)
    stop_consuming = threading.Event()

    def on_event(event):
        if stop_consuming.is_set():
            return
        try:
            events.put_nowait(event)
        except queue.Full:
            pass

    stream = client.stream
    stream.start_streaming(on_event)

    reconnect_count = 0
    data_events = 0
    reconnect_deadline = time.monotonic() + args.duration_secs
    expected_subs: set[tuple[str, str]] = set()

    try:
        stream.subscribe(Contract.stock(args.symbol).quote())
        stream.subscribe(Contract.stock(args.symbol).trade())
        expected_subs = _subscriptions_snapshot(stream)
        if len(expected_subs) < 2:
            raise RuntimeError(f"expected 2 active subscriptions, got {expected_subs!r}")

        symbol, _ = _drain_data_kind(events, timeout_secs=20.0)
        data_events += 1
        if not symbol:
            raise RuntimeError("first data event carried an empty contract.symbol")

        interval = max(5.0, args.duration_secs / max(args.reconnects + 1, 1))
        next_reconnect = time.monotonic() + interval

        while time.monotonic() < reconnect_deadline:
            if reconnect_count < args.reconnects and time.monotonic() >= next_reconnect:
                stream.reconnect()
                reconnect_count += 1
                after = _subscriptions_snapshot(stream)
                if after != expected_subs:
                    raise RuntimeError(
                        f"subscriptions drifted across reconnect: expected {expected_subs!r}, got {after!r}"
                    )
                symbol, _ = _drain_data_kind(events, timeout_secs=20.0)
                data_events += 1
                if not symbol:
                    raise RuntimeError(
                        f"data event after reconnect {reconnect_count} carried an empty contract.symbol"
                    )
                next_reconnect += interval
                continue

            try:
                event = events.get(timeout=0.5)
            except queue.Empty:
                continue
            if event.kind in {"quote", "trade", "open_interest", "ohlcvc"}:
                data_events += 1

        if reconnect_count < args.reconnects:
            raise RuntimeError(
                f"completed only {reconnect_count} reconnects within soak window; expected {args.reconnects}"
            )
        if data_events <= reconnect_count:
            raise RuntimeError(
                f"observed insufficient data events after reconnects: {data_events} events, {reconnect_count} reconnects"
            )
    finally:
        stop_consuming.set()
        stream.stop_streaming()
        stream.await_drain(5_000)

    print(
        f"streaming soak: ok ({reconnect_count} reconnects, {data_events} data events, symbol={args.symbol})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
