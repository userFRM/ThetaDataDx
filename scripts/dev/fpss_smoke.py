#!/usr/bin/env python3
"""Cross-platform FPSS smoke checks for the Python SDK."""

from __future__ import annotations

import argparse
import queue
import sys
import threading
import time


def _subscriptions_snapshot(client) -> set[tuple[str, str]]:
    return {(entry["kind"], entry["contract"]) for entry in client.stream.active_subscriptions()}


def _drain_data_kind(events: "queue.Queue", *, timeout_secs: float) -> tuple[str, str]:
    """Block on the queue until a data event surfaces. Returns
    `(symbol, kind)` so the caller can sanity-check the typed
    contract carried on every data event without a side-table."""
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
    raise RuntimeError(f"timed out waiting for FPSS data event (last kind={last_kind})")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("creds", help="Path to creds.txt")
    parser.add_argument("--symbol", default="AAPL", help="Stock symbol for live replay checks")
    parser.add_argument("--option-symbol", default="SPY", help="Option root for subscription API smoke")
    parser.add_argument("--expiration", default="20260417", help="Option expiration YYYYMMDD")
    parser.add_argument("--strike", default="550", help="Option strike (dollars)")
    parser.add_argument("--right", default="C", help="Option right (`C` or `P`)")
    args = parser.parse_args()

    from thetadatadx import Config, Contract, Credentials, Client  # type: ignore

    cfg = Config.dev()
    cfg.reconnect_policy = "manual"
    cfg.derive_ohlcvc = False
    client = Client(Credentials.from_file(args.creds), cfg)

    # Push-callback delivery — fan events into a thread-safe queue so
    # the main thread can drive the smoke assertions synchronously
    # while the event-dispatch consumer thread keeps producing.
    events: "queue.Queue" = queue.Queue(maxsize=4096)
    stop_consuming = threading.Event()

    def on_event(event):
        if stop_consuming.is_set():
            return
        try:
            events.put_nowait(event)
        except queue.Full:
            pass

    client.stream.start_streaming(on_event)

    try:
        client.stream.subscribe(Contract.stock(args.symbol).quote())
        client.stream.subscribe(Contract.stock(args.symbol).trade())
        client.stream.subscribe(
            Contract.option(
                args.option_symbol,
                expiration=args.expiration,
                strike=args.strike,
                right=args.right,
            ).quote()
        )

        expected_subs = _subscriptions_snapshot(client)
        if len(expected_subs) < 3:
            raise RuntimeError(f"expected at least 3 active subscriptions, got {expected_subs!r}")

        symbol, first_kind = _drain_data_kind(events, timeout_secs=20.0)
        if not symbol:
            raise RuntimeError(
                f"first {first_kind} event carried an empty contract.symbol — "
                "data variants must surface the resolved typed Contract"
            )

        client.stream.reconnect()
        after = _subscriptions_snapshot(client)
        if after != expected_subs:
            raise RuntimeError(
                f"subscriptions drifted across reconnect: expected {expected_subs!r}, got {after!r}"
            )

        symbol, second_kind = _drain_data_kind(events, timeout_secs=20.0)
        if not symbol:
            raise RuntimeError(
                f"reconnect {second_kind} event carried an empty contract.symbol"
            )
    finally:
        stop_consuming.set()
        client.stream.stop_streaming()
        client.stream.await_drain(5_000)

    print(
        "python fpss smoke: ok "
        f"(symbol={args.symbol}, option={args.option_symbol} {args.expiration} {args.strike} {args.right})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
