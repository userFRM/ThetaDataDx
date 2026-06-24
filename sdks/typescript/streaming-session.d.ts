/**
 * TypeScript surface for the `await using` streaming wrapper.
 *
 * `await using session = await client.streaming(callback)` (TC39 explicit
 * resource management) registers the callback via `startStreaming`
 * and pairs `stopStreaming()` + `awaitDrain(5000)` on dispose, mirroring
 * the C++ RAII destructor in `sdks/cpp/src/thetadatadx.cpp`.
 *
 * The runtime wrapper proxies every attribute access to the underlying
 * `Client` (resolving the `client.stream` `StreamView` surface first),
 * so the type surface here extends `Client` and `StreamView` -- adding a
 * `subscribeX` method to either napi binding flows through to the
 * session type with no drift.
 */

/* eslint-disable */

import type { Client, StreamView, StreamEvent, ContractRef } from './index';

export * from './index';

/** `Contract` aliases `ContractRef`. napi-rs exposes the fluent
 * contract type under `ContractRef` because the `Contract` symbol is
 * already taken by the streaming event-payload data class. The public
 * surface documented in the quickstart and reference is
 * `Contract.stock("AAPL")` / `Contract.option(...)`, so the alias
 * keeps the type-side and runtime-side names identical. */
export const Contract: typeof ContractRef;
export type Contract = ContractRef;

// ── Typed error hierarchy ─────────────────────────────────────────────
//
// Every `thetadatadx::Error` surfaced through the napi boundary is
// re-cast on the JS side as one of the leaves below. The canonical leaf
// set (`NotFoundError`, `DeadlineExceededError`, `UnavailableError`,
// `InvalidParameterError`, ...) is identical to the Python, C++, and C
// ABI leaf sets, so a `catch` clause ports across bindings by class name
// — port a Python `except thetadatadx.SubscriptionError` clause to TS by
// writing `catch (e) { if (e instanceof thetadatadx.SubscriptionError) { ... } }`.
// Python additionally ships two back-compat aliases
// (`NoDataFoundError` / `TimeoutError`) that have no equivalent here.

/** Base class for every typed error this binding throws. */
export class ThetaDataError extends Error {}
/** Authentication failed against ThetaData Nexus. */
export class AuthenticationError extends ThetaDataError {}
/** Supplied credentials were rejected. */
export class InvalidCredentialsError extends AuthenticationError {}
/** Tier / plan does not cover the request (gRPC `PermissionDenied`). */
export class SubscriptionError extends ThetaDataError {}
/** Rate limit / quota exhausted (gRPC `ResourceExhausted`, HTTP 429). */
export class RateLimitError extends ThetaDataError {
  /**
   * Server-supplied minimum back-off in seconds, parsed from the
   * upstream `google.rpc.RetryInfo` hint, or `null` when none was
   * supplied. Always present so callers can read it unconditionally.
   */
  retryAfter: number | null;
}
/** A client-side parameter was rejected by input validation. */
export class InvalidParameterError extends ThetaDataError {}
/** Empty result / unknown contract (gRPC `NotFound`). */
export class NotFoundError extends ThetaDataError {}
/** Per-request deadline elapsed (gRPC `DeadlineExceeded`). */
export class DeadlineExceededError extends ThetaDataError {}
/** Upstream unavailable (gRPC `Unavailable`, often retryable). */
export class UnavailableError extends ThetaDataError {}
/** Transport-layer failure (TCP / TLS / IO) other than `Unavailable`. */
export class NetworkError extends ThetaDataError {}
/** Decoder schema mismatch — usually a server proto bump. */
export class SchemaMismatchError extends ThetaDataError {}
/** Streaming protocol / state-machine failure. */
export class StreamError extends ThetaDataError {}
/** Configuration fault (config-file I/O, TOML parse). */
export class ConfigError extends ThetaDataError {}

/** Callback signature mirrored from the napi-generated
 * `startStreaming(callback)` declaration in `index.d.ts`. */
export type StreamEventCallback = (event: StreamEvent) => void;

/**
 * Context object returned by `client.streaming(callback)`. Implements
 * `Symbol.asyncDispose` so `await using session = ...` blocks pair
 * `startStreaming` (on the awaited factory call) with
 * `stopStreaming() + awaitDrain(5000)` on scope exit. The drain
 * barrier guarantees the consumer thread has finished firing the
 * registered callback before the JS closure can be released.
 *
 * The runtime forwarding is `Proxy`-based and resolves names against
 * `client.stream` (the `StreamView` streaming surface) first, then the
 * `Client` itself. The type surface mirrors that by extending both
 * `Client` and `StreamView`, so `session.subscribe(...)`,
 * `session.reconnect()`, and `session.activeSubscriptions()` type-check
 * alongside every `Client` method with zero hand-listed mirror.
 */
export interface StreamingSession extends Client, StreamView {
  /**
   * Invoked by `await using session = ...` on scope exit. Stops the
   * streaming connection and awaits the drain barrier so the consumer
   * thread is guaranteed to have finished firing the registered
   * callback before the JS closure can be released. Drain timeouts
   * emit `console.warn` rather than throwing.
   */
  [Symbol.asyncDispose](): Promise<void>;
}

export declare const StreamingSession: {
  new (client: Client): StreamingSession;
  prototype: StreamingSession;
};

declare module './index' {
  interface Client {
    /**
     * Open a context-managed streaming session.
     *
     * `await using session = await client.streaming(callback)` registers
     * `callback` via `startStreaming` and pairs `stopStreaming()` +
     * `awaitDrain(5000)` on scope exit, mirroring the C++ RAII
     * destructor in `sdks/cpp/src/thetadatadx.cpp`. If the drain barrier
     * times out, `console.warn` fires but the `using` scope exits
     * normally so any error from the body is not masked.
     */
    streaming(callback: StreamEventCallback): Promise<StreamingSession>;
  }

  interface StreamView {
    /**
     * Open a pull-based columnar reader over the live stream — a sibling
     * to the per-event `startStreaming(callback)`.
     *
     * The same subscriptions feed it, but market-data events arrive as
     * apache-arrow `RecordBatch` values under a fixed schema, consumed
     * with `for await (const batch of reader)`. The reader closes
     * (unsubscribe + tear down) on `close()` or `Symbol.asyncDispose`
     * (`await using reader = await client.stream.batches()`). Subscribe on
     * this same surface first, then open the reader.
     *
     * The runtime returns the JS {@link RecordBatchStream} wrapper around
     * the native handle; this override replaces the napi-generated
     * `Promise<RecordBatchStreamHandle>` return type.
     */
    batches(options?: BatchesOptions): Promise<RecordBatchStream>;
  }
}

/** Optional tuning for {@link StreamView.batches}. */
export interface BatchesOptions {
  /** Rows per batch. Default 65536. A batch also flushes on {@link lingerMs}. */
  batchSize?: number;
  /**
   * Milliseconds a partial batch waits before flushing, so a quiet stream
   * still delivers. Default 50.
   */
  lingerMs?: number;
  /**
   * Backpressure when the reader falls behind: `"block"` (default,
   * lossless — applies backpressure to the wire) or `"dropOldest"`
   * (bounded buffer; drops the oldest batch and counts it in
   * {@link RecordBatchStream.dropped}).
   */
  backpressure?: 'block' | 'dropOldest';
  /** Bounded-buffer depth in batches for `"dropOldest"`. Default 4. */
  capacity?: number;
}

/**
 * Pull-based columnar reader returned by `client.stream.batches(...)`.
 *
 * `AsyncIterable` of apache-arrow `RecordBatch` values under a fixed
 * schema, and a TC39 async-disposable: `await using reader = ...` closes
 * it on scope exit, or call {@link close}. Yields are concat-safe — every
 * batch carries the identical {@link schema}.
 */
export interface RecordBatchStream extends AsyncIterable<import('apache-arrow').RecordBatch> {
  /** The fixed Arrow schema every yielded batch carries. */
  readonly schema: import('apache-arrow').Schema;
  /** Batches dropped so far under `"dropOldest"`; `0` under `"block"`. */
  readonly dropped: number;
  /** Close the reader: unsubscribe and tear the streaming session down. Idempotent. */
  close(): void;
  [Symbol.asyncDispose](): Promise<void>;
}
