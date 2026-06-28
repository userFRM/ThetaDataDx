"""Cross-language lookup and conversion helpers.

The ``thetadatadx.util`` submodule exposes the same finite lookup tables
and tick-field conversions the other language bindings share: trade and
quote condition vocabulary, exchange name / symbol lookups, the
calendar-day status vocabulary, the ``(date, ms-of-day)`` to epoch-
milliseconds conversion, and the signed / unsigned trade-sequence
re-encoding.

All functions are pure and side-effect free.

Example:
    >>> import thetadatadx.util as util
    >>> util.condition_name(0)
    'REGULAR'
    >>> util.exchange_symbol(3)
    'NYSE'
"""

from __future__ import annotations

from typing import Optional


def condition_name(code: int) -> str:
    """Return the trade-condition name for ``code`` (e.g. ``"REGULAR"``).

    Returns a placeholder name for codes outside the known table.
    """
    ...


def condition_description(code: int) -> str:
    """Return the human-readable trade-condition description for ``code``."""
    ...


def is_cancel(code: int) -> bool:
    """Return whether the trade-condition ``code`` marks a cancellation."""
    ...


def updates_volume(code: int) -> bool:
    """Return whether a trade with condition ``code`` updates daily volume."""
    ...


def quote_condition_name(code: int) -> str:
    """Return the quote-condition name for ``code``."""
    ...


def quote_condition_description(code: int) -> str:
    """Return the human-readable quote-condition description for ``code``."""
    ...


def is_firm(code: int) -> bool:
    """Return whether the quote-condition ``code`` is a firm quote."""
    ...


def is_halted(code: int) -> bool:
    """Return whether the quote-condition ``code`` marks a trading halt."""
    ...


def exchange_name(code: int) -> str:
    """Return the exchange name for ``code`` (e.g. ``"NewYorkStockExchange"``)."""
    ...


def exchange_symbol(code: int) -> str:
    """Return the short exchange symbol for ``code`` (e.g. ``"NYSE"``)."""
    ...


def calendar_status_name(code: int) -> str:
    """Return the calendar-day status text for ``code``.

    Maps ``0`` to ``"open"``, ``1`` to ``"early_close"``, ``2`` to
    ``"full_close"``, and ``3`` to ``"weekend"``; returns ``"UNKNOWN"``
    for codes outside the table.
    """
    ...


def timestamp_ms(date: int, ms_of_day: int) -> Optional[int]:
    """Combine an Eastern-Time ``YYYYMMDD`` ``date`` and ``ms_of_day``.

    Returns Unix epoch milliseconds (UTC, DST-aware), or ``None`` when
    ``date`` is ``0`` or either input is out of domain. Usable with any
    ``(date, *_ms_of_day)`` pair on the tick structs.
    """
    ...


def sequence_signed_to_unsigned(signed_value: int) -> int:
    """Convert a signed wire-encoded trade sequence to its unsigned form.

    ``signed_value`` must lie in the i32 wire range
    (``-2_147_483_648 ..= 2_147_483_647``); a value outside that domain
    is rejected with :class:`ValueError`.
    """
    ...


def sequence_unsigned_to_signed(unsigned_value: int) -> int:
    """Convert an unsigned monotonic trade sequence back to signed wire form.

    ``unsigned_value`` must lie in the unsigned wire range
    (``0 ..= 2**32 - 1``); a value above that domain is rejected with
    :class:`ValueError`.
    """
    ...
