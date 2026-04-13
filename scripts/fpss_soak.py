#!/usr/bin/env python3
"""Exercise FPSS reconnect + subscription restoration against the replay servers."""

from __future__ import annotations

import argparse
import time


def _require_data_event(client, *, timeout_secs: float) -> tuple[int | None, str]:
    deadline = time.monotonic() + timeout_secs
    last_kind = "none"
    while time.monotonic() < deadline:
        event = client.next_event(timeout_ms=500)
        if event is None:
            continue
        kind = event.get("kind", "unknown")
        last_kind = str(kind)
        if kind in {"quote", "trade", "open_interest", "ohlcvc"}:
            return event.get("contract_id"), str(kind)
    raise RuntimeError(f"timed out waiting for data event (last kind={last_kind})")


def _subscriptions_snapshot(client) -> set[tuple[str, str]]:
    subs = client.active_subscriptions()
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

    from thetadatadx import Config, Credentials, ThetaDataDx  # type: ignore

    cfg = Config.dev()
    cfg.reconnect_policy = "manual"
    cfg.derive_ohlcvc = False
    client = ThetaDataDx(Credentials.from_file(args.creds), cfg)

    reconnect_count = 0
    data_events = 0
    reconnect_deadline = time.monotonic() + args.duration_secs
    expected_subs: set[tuple[str, str]] = set()

    try:
        client.start_streaming()
        client.subscribe_quotes(args.symbol)
        client.subscribe_trades(args.symbol)
        expected_subs = _subscriptions_snapshot(client)
        if len(expected_subs) < 2:
            raise RuntimeError(f"expected 2 active subscriptions, got {expected_subs!r}")

        contract_id, kind = _require_data_event(client, timeout_secs=20.0)
        data_events += 1
        if contract_id is not None:
            contract = client.contract_lookup(contract_id)
            if not contract:
                raise RuntimeError(f"contract_lookup({contract_id}) returned nothing after {kind}")

        interval = max(5.0, args.duration_secs / max(args.reconnects + 1, 1))
        next_reconnect = time.monotonic() + interval

        while time.monotonic() < reconnect_deadline:
            if reconnect_count < args.reconnects and time.monotonic() >= next_reconnect:
                client.reconnect()
                reconnect_count += 1
                after = _subscriptions_snapshot(client)
                if after != expected_subs:
                    raise RuntimeError(
                        f"subscriptions drifted across reconnect: expected {expected_subs!r}, got {after!r}"
                    )
                contract_id, _ = _require_data_event(client, timeout_secs=20.0)
                data_events += 1
                if contract_id is not None and not client.contract_lookup(contract_id):
                    raise RuntimeError(
                        f"contract_lookup({contract_id}) returned nothing after reconnect {reconnect_count}"
                    )
                next_reconnect += interval
                continue

            event = client.next_event(timeout_ms=500)
            if event is None:
                continue
            if event.get("kind") in {"quote", "trade", "open_interest", "ohlcvc"}:
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
        client.shutdown()

    print(
        f"fpss soak: ok ({reconnect_count} reconnects, {data_events} data events, symbol={args.symbol})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
