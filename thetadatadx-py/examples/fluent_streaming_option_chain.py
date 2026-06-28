"""Fluent option-chain streaming example.

Builds a small chain of option contracts and bulk-subscribes via
`subscribe_many(...)`. Demonstrates how the typed `Subscription` value
returned by `Contract.option(...).quote()` mixes homogeneously with
`SecType.OPTION.full_open_interest()` in the same iterable — no
manual string formatting, no per-contract flag dance.
"""

from __future__ import annotations

import time

from thetadatadx import (
    Config,
    Contract,
    Credentials,
    Quote,
    SecType,
    Client,
    Trade,
)


def on_event(event):
    match event:
        case Quote(bid=b, ask=a, contract=c):
            right = "C" if c.right == "C" else "P"
            print(
                f"[{c.symbol} {c.expiration} {right} {c.strike}] "
                f"bid={b:.2f} ask={a:.2f}"
            )
        case Trade(price=px, size=sz, contract=c):
            print(f"[{c.symbol}] TRADE {px:.2f} x {sz}")
        case _:
            pass


def main() -> None:
    creds = Credentials.from_file("creds.txt")
    client = Client(creds, Config.production())

    # Build a strike chain around 550, both wings, 20-Jun-2026.
    expiration = "20260620"
    strikes = ["540", "545", "550", "555", "560"]
    chain = [
        Contract.option("SPY", expiration=expiration, strike=k, right="C")
        for k in strikes
    ] + [
        Contract.option("SPY", expiration=expiration, strike=k, right="P")
        for k in strikes
    ]

    # Quote subscriptions for every contract in the chain, plus the
    # universe-wide option open-interest stream.
    subs = [c.quote() for c in chain] + [SecType.OPTION.full_open_interest()]

    with client.streaming(on_event) as session:
        session.subscribe_many(subs)
        time.sleep(60)


if __name__ == "__main__":
    main()
