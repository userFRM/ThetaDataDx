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

import type { ThetaDataDxClient, FpssEvent, ContractRef } from './index';

export * from './index';

/** `Contract` aliases `ContractRef`. napi-rs exposes the fluent
 * contract type under `ContractRef` because the `Contract` symbol is
 * already taken by the FPSS event-payload data class. The public
 * surface documented in the quickstart and reference is
 * `Contract.stock("AAPL")` / `Contract.option(...)`, so the alias
 * keeps the type-side and runtime-side names identical. */
export const Contract: typeof ContractRef;
export type Contract = ContractRef;

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
  }
}
