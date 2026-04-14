#!/usr/bin/env python3
"""Cross-platform FPSS smoke checks for the Python SDK."""

from __future__ import annotations

import argparse
import sys
import time


def _require_data_event(client, *, timeout_secs: float) -> tuple[int | None, str]:
    deadline = time.monotonic() + timeout_secs
    last_kind = "none"
    while time.monotonic() < deadline:
        event = client.next_event(timeout_ms=500)
        if event is None:
            continue
        kind = str(event.get("kind", "unknown"))
        last_kind = kind
        if kind in {"quote", "trade", "open_interest", "ohlcvc"}:
            return event.get("contract_id"), kind
    raise RuntimeError(f"timed out waiting for FPSS data event (last kind={last_kind})")


def _require_data_event_with_retry(
    client, *, timeout_secs: float, attempts: int = 3
) -> tuple[int | None, str]:
    last_error: RuntimeError | None = None
    for attempt in range(1, attempts + 1):
        try:
            return _require_data_event(client, timeout_secs=timeout_secs)
        except RuntimeError as exc:
            last_error = exc
            if attempt == attempts:
                break
            print(
                f"fpss smoke retry {attempt}/{attempts - 1}: {exc}",
                file=sys.stderr,
            )
            client.reconnect()
            time.sleep(1.0)
    assert last_error is not None
    raise last_error


def _subscriptions_snapshot(client) -> set[tuple[str, str]]:
    return {(entry["kind"], entry["contract"]) for entry in client.active_subscriptions()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("creds", help="Path to creds.txt")
    parser.add_argument("--symbol", default="AAPL", help="Stock symbol for live replay checks")
    parser.add_argument("--option-symbol", default="SPY", help="Option root for subscription API smoke")
    parser.add_argument("--expiration", default="20260417", help="Option expiration YYYYMMDD")
    parser.add_argument("--strike", default="550", help="Option strike")
    parser.add_argument("--right", default="C", help="Option right")
    args = parser.parse_args()

    from thetadatadx import Config, Credentials, ThetaDataDx  # type: ignore

    cfg = Config.dev()
    cfg.reconnect_policy = "manual"
    cfg.derive_ohlcvc = False
    client = ThetaDataDx(Credentials.from_file(args.creds), cfg)

    try:
        client.start_streaming()
        client.subscribe_quotes(args.symbol)
        client.subscribe_trades(args.symbol)
        client.subscribe_option_quotes(
            args.option_symbol, args.expiration, args.strike, args.right
        )

        expected_subs = _subscriptions_snapshot(client)
        if len(expected_subs) < 3:
            raise RuntimeError(f"expected at least 3 active subscriptions, got {expected_subs!r}")

        contract_id, first_kind = _require_data_event_with_retry(
            client, timeout_secs=20.0
        )
        if contract_id is not None and not client.contract_lookup(contract_id):
            raise RuntimeError(
                f"contract_lookup({contract_id}) returned nothing after first {first_kind} event"
            )

        contract_map = client.contract_map()
        if not contract_map:
            raise RuntimeError("contract_map() returned no entries after first data event")

        client.reconnect()
        after = _subscriptions_snapshot(client)
        if after != expected_subs:
            raise RuntimeError(
                f"subscriptions drifted across reconnect: expected {expected_subs!r}, got {after!r}"
            )

        contract_id, second_kind = _require_data_event_with_retry(
            client, timeout_secs=20.0
        )
        if contract_id is not None and not client.contract_lookup(contract_id):
            raise RuntimeError(
                f"contract_lookup({contract_id}) returned nothing after reconnect {second_kind} event"
            )
    finally:
        client.shutdown()

    print(
        "python fpss smoke: ok "
        f"(symbol={args.symbol}, option={args.option_symbol} {args.expiration} {args.strike} {args.right})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
