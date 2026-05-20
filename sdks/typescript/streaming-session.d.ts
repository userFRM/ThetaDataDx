// TypeScript surface for the `await using` streaming wrapper.
//
// `await using session = await tdx.streaming(callback)` (TC39 explicit
// resource management) registers the callback via `startStreaming`
// and pairs `stopStreaming()` + `awaitDrain(5000)` on dispose, mirroring
// the C++ RAII destructor in `sdks/cpp/src/thetadx.cpp`.
//
// SSOT: the runtime wrapper proxies every attribute access to the
// underlying `ThetaDataDxClient`, so the type surface here just extends
// `ThetaDataDxClient` -- adding a `subscribeX` method to the napi binding
// flows through to the session type with no drift.

/* eslint-disable */

import type { ThetaDataDxClient, FpssEvent, ContractRef, EventIterator } from './index';

export * from './index';

/** `Contract` aliases `ContractRef`. napi-rs exposes the fluent
 * contract type under `ContractRef` because the `Contract` symbol is
 * already taken by the FPSS event-payload data class. The public
 * surface documented in the quickstart and reference is
 * `Contract.stock("AAPL")` / `Contract.option(...)`, so the alias
 * keeps the type-side and runtime-side names identical. */
export const Contract: typeof ContractRef;
export type Contract = ContractRef;

// ── Typed error hierarchy ─────────────────────────────────────────────
//
// Every `thetadatadx::Error` surfaced through the napi boundary is
// re-cast on the JS side as one of the leaves below. The hierarchy
// mirrors the Python `to_py_err` leaf set one-for-one so the
// cross-binding error contract stays uniform — port a Python
// `except thetadatadx.SubscriptionError` clause to TS by writing
// `catch (e) { if (e instanceof tdx.SubscriptionError) { ... } }`.

export class ThetaDataError extends Error {}
export class AuthenticationError extends ThetaDataError {}
export class InvalidCredentialsError extends AuthenticationError {}
export class SubscriptionError extends ThetaDataError {}
export class RateLimitError extends ThetaDataError {}
export class NotFoundError extends ThetaDataError {}
export class DeadlineExceededError extends ThetaDataError {}
export class UnavailableError extends ThetaDataError {}
export class NetworkError extends ThetaDataError {}
export class SchemaMismatchError extends ThetaDataError {}
export class StreamError extends ThetaDataError {}

/** Callback signature mirrored from the napi-generated
 * `startStreaming(callback)` declaration in `index.d.ts`. */
export type FpssEventCallback = (event: FpssEvent) => void;

/**
 * Context object returned by `tdx.streaming(callback)`. Implements
 * `Symbol.asyncDispose` so `await using session = ...` blocks pair
 * `startStreaming` (on the awaited factory call) with
 * `stopStreaming() + awaitDrain(5000)` on scope exit. The drain
 * barrier guarantees the consumer thread has finished firing the
 * registered callback before the JS closure can be released.
 *
 * The runtime forwarding is `Proxy`-based, so the type surface here
 * extends `ThetaDataDxClient` directly -- every method on the underlying
 * client is reachable on the session with zero hand-listed mirror.
 */
export interface StreamingSession extends ThetaDataDxClient {
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
  new (tdx: ThetaDataDxClient): StreamingSession;
  prototype: StreamingSession;
};

/**
 * Pull-iter context-managed streaming session returned by
 * `tdx.streamingIter()`. Drives the FPSS pull-iter delivery path:
 * `for await (const event of session) { ... }` drains the per-client
 * bounded queue, and the `[Symbol.asyncDispose]` hook pairs
 * `close()` + `stopStreaming()` + `awaitDrain(5000)` on scope exit.
 *
 * The runtime forwarding is `Proxy`-based: every method on the
 * underlying `EventIterator` (e.g. `tryNext`, `close`) AND every
 * method on the parent `ThetaDataDxClient` (e.g. `subscribe`,
 * `activeSubscriptions`) is reachable on the session.
 */
export interface StreamingIterSession extends EventIterator {
  [Symbol.asyncIterator](): AsyncIterableIterator<FpssEvent>;
  [Symbol.asyncDispose](): Promise<void>;
}

export declare const StreamingIterSession: {
  new (
    tdx: ThetaDataDxClient,
    iter: EventIterator,
  ): StreamingIterSession;
  prototype: StreamingIterSession;
};

declare module './index' {
  interface ThetaDataDxClient {
    /**
     * Open a context-managed streaming session.
     *
     * `await using session = await tdx.streaming(callback)` registers
     * `callback` via `startStreaming` and pairs `stopStreaming()` +
     * `awaitDrain(5000)` on scope exit, mirroring the C++ RAII
     * destructor in `sdks/cpp/src/thetadx.cpp`. If the drain barrier
     * times out, `console.warn` fires but the `using` scope exits
     * normally so any error from the body is not masked.
     */
    streaming(callback: FpssEventCallback): Promise<StreamingSession>;

    /**
     * Open a context-managed pull-iter streaming session.
     *
     * `await using session = await tdx.streamingIter()` opens the FPSS
     * connection in pull-iter mode and pairs `stopStreaming()` +
     * `awaitDrain(5000)` on scope exit. Drain timeouts emit
     * `console.warn`. Iterate inside the body with
     * `for await (const event of session)` — the async iterator
     * yields typed `FpssEvent` values and terminates cleanly on
     * upstream shutdown.
     *
     * Mutually exclusive with `streaming(callback)` on the same
     * client; switch by stopping the active session first.
     */
    streamingIter(): Promise<StreamingIterSession>;
  }
}
