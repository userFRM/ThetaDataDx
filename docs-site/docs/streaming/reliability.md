---
title: Reconnection & Monitoring
description: Automatic reconnect policy, flush mode, and the counters that tell you a stream is healthy.
---

# Reconnection & Monitoring

## Automatic reconnection

A dropped streaming connection recovers by itself: the client reconnects with exponential backoff and jitter, re-subscribes everything you had installed (paced in bursts so a recovering server is not flooded), and reports progress through `reconnecting` / `reconnected` [events](/streaming/events). Rate-limited drops honor the server's cooldown in full; a session that runs cleanly for the stable window earns its full retry budget back.

If recovery stops — budget exhausted, a permanent disconnect reason, or a `manual` policy — a terminal `reconnects_exhausted` event fires with the reason and attempt count:

```python
def on_event(event):
    if event.kind == "reconnects_exhausted":
        page_operator(f"stream gave up: {event.reason_name} after {event.attempts} attempts")
```

A clean `stop_streaming()` emits no terminal event; only an unrecoverable session does.

### Policy knobs

All on the [configuration object](/articles/configuration), prefixed `reconnect_`:

| Knob | Default | Meaning |
|---|---|---|
| `reconnect_policy` | `"auto"` | `"auto"` recovers automatically; `"manual"` never reconnects (you call `reconnect()`); `"custom"` uses your callback. |
| `reconnect_wait_ms` / `reconnect_wait_max_ms` | 250 / 30000 | Exponential backoff ladder: initial delay and cap. |
| `reconnect_jitter` | `"full"` | Jitter mode: `"full"`, `"equal"`, `"decorrelated"`, `"none"`. |
| `reconnect_max_attempts` | 30 | Attempt budget for transient drops. |
| `reconnect_max_elapsed_secs` | 300 | Wall-clock cap on one recovery sequence; `0` disables. |
| `reconnect_wait_rate_limited_ms` / `reconnect_max_rate_limited_attempts` | 130000 / 100 | Floor and budget for rate-limited drops. |
| `reconnect_wait_server_restart_ms` / `reconnect_max_server_restart_attempts` | 5000 / 60 | Cadence and budget for server-restart drops. |
| `reconnect_stable_window_secs` | 60 | Clean runtime after which budgets reset. |
| `reconnect_replay_burst_size` / `reconnect_replay_pace_ms` | 50 / 5 | Re-subscription pacing after a reconnect. |

A custom policy is a callback receiving `(reason, attempt)` and returning the delay in milliseconds — or nothing to give up:

```python
cfg = Config.production()
cfg.reconnect_callback = lambda reason, attempt: min(1_000 * attempt, 60_000)
```

Permanent failures (for example rejected credentials) never reach the callback — no policy can turn them into a retry loop.

Caller-driven recovery is always available: `reconnect()` re-opens the session and restores the saved subscription set on demand.

## Flush mode

`flush_mode` trades write-path latency against syscall volume:

| Mode | Behavior |
|---|---|
| `"batched"` (default) | Outbound frames flush on the heartbeat cadence — the throughput-friendly default. |
| `"immediate"` | Every frame flushes as written — lowest latency. |

```python
cfg = Config.production()
cfg.flush_mode = "immediate"
```

## Monitoring a live stream

Incoming events are buffered between the connection and your callback. If your callback can't keep up, the buffer fills and the newest events are **counted and dropped** rather than stalling the feed — so these counters are the health dashboard:

| Accessor | Tells you |
|---|---|
| `ring_occupancy()` / `ring_capacity()` | Buffered-event count against the fixed buffer size. Occupancy trending toward capacity predicts drops; sample it freely, it never blocks the feed. |
| `dropped_event_count()` | Total events dropped since the session started. Nonzero means your callback is too slow — do less work per event or hand off to a queue. |
| `millis_since_last_event()` | Milliseconds since the last inbound frame of any kind. Steady growth during market hours is the earliest sign of a dead connection. |
| `last_event_received_at_unix_nanos()` | Timestamp of the most recent inbound frame. |
| `last_connected_addr()` | The live server `host:port`, following the session across reconnects. |
| `panic_count()` | Callback exceptions caught and isolated (your callback errors never kill the session — fix them, they cost events). |

Every accessor exists on all four bindings under the language's naming convention (`ringOccupancy()` in TypeScript, `ring_occupancy()` elsewhere). The buffer capacity is configurable via `streaming_ring_size`; keep the callback fast and capacity rarely matters.
