"""Fluent contract-first streaming example.

Demonstrates the primary documented surface for ThetaData Python:
typed `Contract` / `Subscription` values feeding the polymorphic
`client.stream.subscribe(...)` and `client.stream.subscribe_many(...)`
paths (reached here through the `client.streaming(...)` session).

Run with valid `creds.txt` (line 1 = email, line 2 = password) in the
working directory:

    python thetadatadx-py/examples/fluent_streaming_quote.py

The callback fires on the event-dispatch consumer thread under the
GIL; keep it fast.
"""

from __future__ import annotations

import time

from thetadatadx import (
    Config,
    Contract,
    Credentials,
    Ohlcvc,
    Quote,
    SecType,
    Client,
    Trade,
)


def on_event(event):
    """Match-case dispatch on typed streaming event classes."""
    match event:
        case Trade(price=px, size=sz, contract=c):
            print(f"[{c.symbol}] TRADE {px:.2f} x {sz}")
        case Quote(bid=b, ask=a, contract=c):
            print(f"[{c.symbol}] QUOTE bid={b:.2f} ask={a:.2f}")
        # The full-trade stream sends a quote and an OHLC bar before each
        # trade, so the same callback also receives Ohlcvc bars.
        case Ohlcvc(open=o, high=h, low=lo, close=cl, contract=c):
            print(f"[{c.symbol}] BAR o={o:.2f} h={h:.2f} l={lo:.2f} c={cl:.2f}")
        case _:
            pass


def main() -> None:
    creds = Credentials.from_file("creds.txt")
    config = Config.production()
    client = Client(creds, config)

    # Fluent contract-first construction. Every subscription is a
    # typed `Subscription` value — no string flags, no kwarg
    # gymnastics.
    stock = Contract.stock("AAPL")
    option = Contract.option(
        "SPY", expiration="20260620", strike="550", right="C"
    )

    with client.streaming(on_event) as session:
        # One subscription at a time:
        session.subscribe(stock.quote())
        session.subscribe(stock.trade())

        # Or many at once:
        session.subscribe_many(
            [
                option.quote(),
                option.trade(),
                option.open_interest(),
            ]
        )

        # Full-stream — every option trade across the universe.
        session.subscribe(SecType.OPTION.full_trades())

        # Park the main thread while events flow.
        time.sleep(60)
    # `__exit__` calls stop_streaming() + await_drain(5_000) so the
    # consumer thread is guaranteed to have stopped firing the
    # callback before control returns here.


if __name__ == "__main__":
    main()
