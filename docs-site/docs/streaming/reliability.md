---
title: Reconnection & Monitoring
description: Automatic reconnect policy and the counters that tell you a stream is healthy.
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
| `reconnect_jitter` | `"full"` | Jitter mode: `"full"`, `"none"`. |
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

## Monitoring a live stream

Incoming events are buffered between the connection and your callback. If your callback can't keep up, the buffer fills and the newest events are **counted and dropped** rather than stalling the feed — so these counters are the health dashboard:

| Accessor | Tells you |
|---|---|
| `ring_occupancy()` / `ring_capacity()` | Buffered-event count against the fixed buffer size. Occupancy trending toward capacity predicts drops; sample it freely, it never blocks the feed. |
| `dropped_event_count()` | Total events dropped since the session started. Nonzero means your callback is too slow — do less work per event or hand off to a queue. |
| `millis_since_last_event()` | Milliseconds since the last inbound frame of any kind. Steady growth during market hours is the earliest sign of a dead connection. |
| `last_event_received_at_unix_nanos()` | Timestamp of the most recent inbound frame. |
| `last_connected_addr()` | The live server `host:port`, following the session across reconnects. |
| `panic_count()` | Callback panics or binding-contained exceptions counted by the delivery boundary. In TypeScript, JavaScript exceptions follow Node's normal exception handling; fix callback failures, they cost events. |

Every accessor exists on all four bindings under the language's naming convention (`ringOccupancy()` in TypeScript, `ring_occupancy()` elsewhere). The buffer capacity is configurable via `streaming_ring_size`; keep the callback fast and capacity rarely matters.

## Idle CPU and the consumer wait mode

The streaming consumer runs a thread that waits for the next event. By default it busy-spins for the lowest possible latency, which holds **~100% of one core the whole time the stream is connected**, including overnight, weekends, and holidays when nothing is trading. A long-running or 24/7 consumer therefore burns a full core continuously. `wait_mode` trades a little latency for that core back:

| `wait_mode` | Idle CPU | Latency | Use it when |
|---|---:|---|---|
| `spin` (default) | ~100% | lowest | a dedicated core and latency is everything |
| `busyspin` | ~100% | lowest, least jitter | a pinned core and you want the last sliver of jitter reduction |
| `park` | ~0-1% | fixed, `park_interval_us` | you want a predictable low-CPU sleep between polls |
| `backoff` | ~0-1% | full speed while events flow | a 24/7 consumer that should stay fast in-hours and idle cheap |

`spin` and `busyspin` both hold ~100% of a core and differ only in jitter, so neither saves CPU. Only `park` and `backoff` lower it, by sleeping between polls. `backoff` is the hands-free choice: it stays at full spinning responsiveness while events are arriving and only drops to sleeping after a short idle lull, so you keep low latency during market hours and pay about ~1% of a core on a quiet weekend, with no manual switching.

`park_interval_us` sets the sleep length for `park` and `backoff`, in microseconds (default `1000` = 1 ms, range `[50, 1000000]`). It bounds the extra latency a parked event can wait. The OS timer floors at ~50 us, so a request shorter than that just hits kernel slack. A 100 us park measures about a few percent of a core with ~150 us wake latency, so you can park close to spin-latency for almost no CPU.

```python
cfg = Config.production()
cfg.wait_mode = "backoff"       # low latency when active, low CPU when idle
cfg.park_interval_us = 1000     # idle sleep length in microseconds; default 1 ms
```

See [Configuration](/articles/configuration) for how each binding exposes these.
