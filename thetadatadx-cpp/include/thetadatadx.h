/**
 * thetadatadx C FFI header.
 *
 * This header declares the C interface to the thetadatadx SDK.
 * Used by both the C++ wrapper and any other C-compatible language.
 *
 * Memory model:
 * - Opaque handles (ThetaDataDxCredentials*, ThetaDataDxMarketDataClient*, ThetaDataDxConfig*) are heap-allocated
 *   by the library and MUST be freed with the corresponding thetadatadx_*_free function.
 * - Tick data is returned as fixed-layout struct arrays. Each array type has a
 *   corresponding thetadatadx_*_array_free function that MUST be called.
 * - String arrays (ThetaDataDxStringArray) must be freed with thetadatadx_string_array_free.
 * - Functions that can fail return empty arrays (data=NULL, len=0) and set a
 *   thread-local error string retrievable via thetadatadx_last_error().
 */

#ifndef THETADATADX_H
#define THETADATADX_H

#include <stdint.h>
#include <stddef.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif

#if defined(_MSC_VER)
#define THETADATADX_ALIGN64_BEGIN __declspec(align(64))
#define THETADATADX_ALIGN64_END
#else
#define THETADATADX_ALIGN64_BEGIN
#define THETADATADX_ALIGN64_END __attribute__((aligned(64)))
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handle types ── */
typedef struct ThetaDataDxCredentials ThetaDataDxCredentials;
typedef struct ThetaDataDxMarketDataClient ThetaDataDxMarketDataClient;
typedef struct ThetaDataDxConfig ThetaDataDxConfig;
typedef struct ThetaDataDxStreamHandle ThetaDataDxStreamHandle;
typedef struct ThetaDataDxClient ThetaDataDxClient;

/* Generated request-options bridge shared with the FFI surface. */
#include "endpoint_request_options.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Fixed-layout tick types                                               */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All tick structs are 64-byte aligned and carry explicit tail padding as
 * part of the ABI contract, so C/C++ array stepping stays byte-for-byte
 * compatible with the wire layout. Price fields are 64-bit doubles.
 * A `bool has_<field>` is false when the response carried no value for
 * <field>; zero is a valid code there, so the value alone cannot tell. */

/* Calendar day-type codes carried by ThetaDataDxCalendarDay.status — the
 * vendor's own vocabulary. Resolve the text form with
 * thetadatadx_calendar_status_name(). */
#define THETADATADX_CALENDAR_STATUS_OPEN 0
#define THETADATADX_CALENDAR_STATUS_EARLY_CLOSE 1
#define THETADATADX_CALENDAR_STATUS_FULL_CLOSE 2
#define THETADATADX_CALENDAR_STATUS_WEEKEND 3

/* Heap-owned byte buffer (Arrow IPC stream) returned by the per-tick
 * thetadatadx_*_to_arrow_ipc terminals. Caller MUST free with thetadatadx_arrow_bytes_free. */
typedef struct ThetaDataDxArrowBytes {
    const uint8_t* data;
    size_t len;
} ThetaDataDxArrowBytes;

/* Heap-owned set of present schema-column names plus optional symbol
 * attribution (the decode's ColumnPresence crossing the C boundary). Built by
 * a thetadatadx_*_present_columns terminal and consumed by the matching
 * thetadatadx_*_to_arrow_ipc_projected terminal. Caller MUST free with
 * thetadatadx_column_presence_free, except the carrier a stream chunk
 * callback receives, which the library owns and frees itself.
 *
 * `symbol` carries a constant root value for responses whose wire has one
 * symbol across every row.
 *
 * `symbols` carries the per-row `symbol` (root) values of a response whose
 * rows span several roots, such as a multi-symbol snapshot or a flat file:
 * one NUL-terminated C string per decoded row, so the projected serialiser
 * emits a leading per-row `symbol` column attributing each row to its
 * underlying. It is NULL for every other response and takes precedence over
 * the constant `symbol` field. */
typedef struct ThetaDataDxColumnPresence {
    const char* const* names;
    size_t len;
    const char* symbol;
    const char* const* symbols;
    size_t symbols_len;
} ThetaDataDxColumnPresence;

/* Generated from the tick table: each tick struct, its array type, its free
 * function and its Arrow IPC entry points. */
#include "ticks.h.inc"

/** Free a byte buffer returned by any thetadatadx_*_to_arrow_ipc terminal.
 *  @param bytes Buffer from a thetadatadx_*_to_arrow_ipc call; a (data=NULL, len=0)
 *               buffer is a no-op. Call exactly once. */
void thetadatadx_arrow_bytes_free(ThetaDataDxArrowBytes bytes);

/** Free a ThetaDataDxColumnPresence returned by any thetadatadx_*_present_columns
 *  terminal, including its names and symbol attribution.
 *  @param presence Carrier from a thetadatadx_*_present_columns call; a
 *                  (names=NULL, len=0) carrier is a no-op. Call exactly once. */
void thetadatadx_column_presence_free(ThetaDataDxColumnPresence presence);

/* ── OptionContract (has heap-allocated symbol string) ── */

typedef struct {
    const char* symbol;     /* heap-allocated, freed with thetadatadx_option_contract_array_free */
    int32_t expiration;     /* YYYYMMDD */
    /* 4 bytes padding before the double field */
    double strike;          /* dollars */
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put. Cast to char for display. */
    uint32_t right;
} ThetaDataDxOptionContract;

typedef struct { const ThetaDataDxOptionContract* data; size_t len; } ThetaDataDxOptionContractArray;

/* ── String array (for list endpoints) ── */

typedef struct {
    const char* const* data;  /* array of NUL-terminated C strings */
    size_t len;
} ThetaDataDxStringArray;

/* ── Subscription types (active_subscriptions) ── */

typedef struct {
    const char* kind;      /* snake_case: per-contract "quote"/"trade"/"open_interest"/"market_value", full-stream "full_trades"/"full_open_interest" */
    const char* contract;  /* "SPY" or "SPY 20260417 550 C" */
} ThetaDataDxSubscription;

typedef struct {
    const ThetaDataDxSubscription* data;
    size_t len;
} ThetaDataDxSubscriptionArray;

/** Free an option-contract or string array returned by its data call.
 *  @param arr The array returned by the data call; a NULL/empty
 *             (data=NULL, len=0) array is a no-op.
 *  @note Call exactly once per returned array. */
void thetadatadx_option_contract_array_free(ThetaDataDxOptionContractArray arr);
void thetadatadx_string_array_free(ThetaDataDxStringArray arr);
/** Free a subscription array returned by an active-subscriptions query.
 *  @param arr Array from thetadatadx_*_active_subscriptions; no-op when NULL.
 *             Call exactly once. */
void thetadatadx_subscription_array_free(ThetaDataDxSubscriptionArray* arr);

/* ── Streaming Arrow RecordBatch reader (pull-based) ── */

/** Opaque handle to a live pull-based Arrow RecordBatch reader. Created by
 *  thetadatadx_client_batches_open, drained by
 *  thetadatadx_record_batch_stream_next_ipc, closed by
 *  thetadatadx_record_batch_stream_close, freed by
 *  thetadatadx_record_batch_stream_free.
 *
 *  thetadatadx_record_batch_stream_close is safe to call from another thread
 *  while a thetadatadx_record_batch_stream_next_ipc pull is in flight: the
 *  pull is woken and returns end of stream. thetadatadx_record_batch_stream_free
 *  takes ownership of the opaque handle and must be serialized with all other
 *  entry points for the same handle. */
typedef struct ThetaDataDxRecordBatchStream ThetaDataDxRecordBatchStream;

/** Backpressure: block — the reader backpressures with no queue-side drops, though a sustained reader stall can still overflow the upstream event ring. */
#define THETADATADX_BACKPRESSURE_BLOCK 0
/** Backpressure: bounded buffer, drop the oldest batch on overflow (counted
 *  by thetadatadx_record_batch_stream_dropped). */
#define THETADATADX_BACKPRESSURE_DROP_OLDEST 1

/** Open a pull-based Arrow RecordBatch reader over the unified client's
 *  stream — a sibling to thetadatadx_client_set_callback. Open first — this
 *  starts the streaming session — then subscribe on the same surface.
 *  Opening a reader drops the callback registration a stopped callback
 *  session saved, so thetadatadx_client_reconnect cannot revive it over the
 *  reader.
 *  @param handle Client from thetadatadx_client_connect.
 *  @param batch_size Rows per batch (0 clamped to 1).
 *  @param linger_ms Partial-batch flush deadline in ms (quiet-stream flush).
 *  @param backpressure THETADATADX_BACKPRESSURE_BLOCK or _DROP_OLDEST.
 *  @param capacity Bounded-buffer depth in batches for drop-oldest (ignored
 *                  for block; may be 0).
 *  @return Reader handle, or NULL with thetadatadx_last_error() set on
 *          failure. Free with thetadatadx_record_batch_stream_free. */
ThetaDataDxRecordBatchStream* thetadatadx_client_batches_open(const ThetaDataDxClient* handle,
                                                              size_t batch_size,
                                                              uint64_t linger_ms,
                                                              int32_t backpressure,
                                                              size_t capacity);

/** Block for the next batch and serialise it as an Arrow IPC stream into
 *  *out.
 *  @param stream Reader from thetadatadx_client_batches_open.
 *  @param out Out-param, always initialised: holds the batch IPC bytes on a 0
 *             return (free with thetadatadx_arrow_bytes_free), or empty
 *             (data=NULL, len=0) otherwise.
 *  @return 0 = a batch was produced; 1 = clean end of stream; -1 = error
 *          (thetadatadx_last_error() set). */
int32_t thetadatadx_record_batch_stream_next_ipc(const ThetaDataDxRecordBatchStream* stream,
                                                 ThetaDataDxArrowBytes* out);

/** Serialise the reader's fixed schema as a schema-only Arrow IPC stream into
 *  *out, so a reader can report its schema before the first batch.
 *  @return 0 on success (out holds schema IPC bytes, free with
 *          thetadatadx_arrow_bytes_free); -1 on error (out empty,
 *          thetadatadx_last_error() set). */
int32_t thetadatadx_record_batch_stream_schema_ipc(const ThetaDataDxRecordBatchStream* stream,
                                                   ThetaDataDxArrowBytes* out);

/** Number of batches dropped so far under the drop-oldest policy. Always 0
 *  under block. */
uint64_t thetadatadx_record_batch_stream_dropped(const ThetaDataDxRecordBatchStream* stream);

/** Stop the reader, tear down the streaming session, WITHOUT freeing the handle.
 *  Safe to call from another thread while a pull is in flight: it wakes the
 *  pull (which returns 1, clean end of stream) and shuts the session down.
 *  Idempotent. The handle stays valid and must still be released with
 *  thetadatadx_record_batch_stream_free. A NULL handle is a no-op. */
void thetadatadx_record_batch_stream_close(const ThetaDataDxRecordBatchStream* stream);

/** Release the reader handle. Signals close first, then drops this handle's
 *  reference. This call takes ownership of the opaque handle and must be
 *  serialized with next_ipc, schema_ipc, dropped, and close for the same
 *  handle. To tear down from another thread, call close to wake a parked pull,
 *  wait for in-flight entry points to return, then free. A NULL handle is a
 *  no-op. After this call the handle is invalid. */
void thetadatadx_record_batch_stream_free(ThetaDataDxRecordBatchStream* stream);

/* ── Error ── */

/** Retrieve the last error message for the current thread.
 *  @return The error string, or NULL when no error is set. The pointer is
 *          valid only until the next FFI call on the same thread; do NOT
 *          free it.
 *  @note Thread-local: each thread observes only its own last error. */
const char* thetadatadx_last_error(void);

/** Clear the thread-local error string.
 *  @note Higher-level wrappers should call this before issuing an FFI call
 *        so they can distinguish "the call set a new error" from "the
 *        previous call left a stale error in the slot" when an empty value
 *        (e.g. zero rows) is also a valid success outcome. */
void thetadatadx_clear_error(void);

/** Typed discriminant of the last FFI error on the current thread. Higher-
 *  level bindings (the C++ exception hierarchy below, the typed error
 *  subclasses in the TypeScript SDK) dispatch on this to pick the right
 *  exception / error subclass without substring-matching the formatted
 *  error string. The string from thetadatadx_last_error() carries the diagnostic.
 *  @return One of the THETADATADX_ERR_* discriminants below; THETADATADX_ERR_NONE when no
 *          error is set or after thetadatadx_clear_error(). */
int32_t thetadatadx_last_error_code(void);

/** Server-supplied rate-limit back-off of the last FFI error on the current
 *  thread, in milliseconds. Set only for a rate-limit error whose upstream
 *  status attached the hint. The C++ RateLimitError::retry_after() surfaces
 *  this as a typed value.
 *  @return The back-off in milliseconds, or THETADATADX_RETRY_AFTER_NONE when
 *          the error carries no retry hint (every non-rate-limit error reads
 *          this sentinel). */
int64_t thetadatadx_last_error_retry_after_ms(void);

/* Sentinel returned by `thetadatadx_last_error_retry_after_ms()` when the last
 * error carries no rate-limit back-off hint. */
#define THETADATADX_RETRY_AFTER_NONE (-1)

/* Error-code discriminants returned by `thetadatadx_last_error_code()`. */
#define THETADATADX_ERR_NONE 0
#define THETADATADX_ERR_OTHER 1
#define THETADATADX_ERR_AUTHENTICATION 2
#define THETADATADX_ERR_INVALID_CREDENTIALS 3
#define THETADATADX_ERR_SUBSCRIPTION 4
#define THETADATADX_ERR_RATE_LIMIT 5
#define THETADATADX_ERR_NOT_FOUND 6
#define THETADATADX_ERR_DEADLINE_EXCEEDED 7
#define THETADATADX_ERR_UNAVAILABLE 8
#define THETADATADX_ERR_NETWORK 9
#define THETADATADX_ERR_SCHEMA_MISMATCH 10
#define THETADATADX_ERR_STREAM 11
#define THETADATADX_ERR_CONFIG 12
#define THETADATADX_ERR_INVALID_PARAMETER 13

/* ── Credentials ── */

/** Create a credentials handle from an email and password.
 *  @param email Account email; must be non-NULL.
 *  @param password Account password; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_email(const char* email, const char* password);

/** Create a credentials handle that authenticates with an API key.
 *  @param api_key API key; must be non-NULL. Trimmed and held as secret
 *         material on the handle.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_api_key(const char* api_key);

/** Create a credentials handle that authenticates with an API key paired
 *  with an account email.
 *  @param email Account email; must be non-NULL. An empty email is dropped.
 *  @param api_key API key; must be non-NULL. Trimmed and held as secret
 *         material on the handle.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_api_key_with_email(const char* email, const char* api_key);

/** Create a credentials handle by reading a file (line 1 = email,
 *  line 2 = password).
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_file(const char* path);

/** Source a credentials handle strictly from the THETADATA_API_KEY
 *  environment variable. Strict: an unset or whitespace-only value is an
 *  error rather than a silent fallback, and there is no creds.txt file
 *  fallback. Use thetadatadx_credentials_from_env_or_file when a file
 *  fallback is wanted instead.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_env(void);

/** Source a credentials handle from the environment, falling back to a file.
 *  When THETADATA_API_KEY is set and non-empty an API key is used; otherwise
 *  the two-line file (line 1 = email, line 2 = password) at path is read.
 *  @param path Filesystem path to the fallback credentials file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_env_or_file(const char* path);

/** Source a credentials handle from a .env-format file.
 *  The file uses the common .env grammar (one KEY=VALUE per line, optional
 *  export prefix, # comment lines, optional matching quotes). When
 *  THETADATA_API_KEY is present and non-empty an API key is used; otherwise a
 *  complete THETADATA_EMAIL + THETADATA_PASSWORD pair builds email + password
 *  credentials.
 *  @param path Filesystem path to the .env file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_dotenv(const char* path);

/** Release a credentials handle.
 *  @param creds Handle from thetadatadx_credentials_from_email /
 *               thetadatadx_credentials_from_file; no-op when NULL. Call exactly once. */
void thetadatadx_credentials_free(ThetaDataDxCredentials* creds);

/* ── Config ── */

/** Create a production config (ThetaData NJ datacenter).
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_production(void);

/** Create a dev streaming config (port 20200, infinite historical replay).
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_dev(void);

/** Create a market-data-staging config (market-data staging cluster + auth marker;
 *  streaming stays on production). Testing, unstable.
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_stage(void);

/** Select the market-data environment on a config handle in place:
 *  kind 0 = production, kind 1 = staging. The market-data and streaming
 *  channels are selected independently, so this leaves the streaming
 *  channel untouched.
 *  @param config Config handle to mutate.
 *  @param kind 0 for PROD, 1 for STAGE.
 *  @return 0 on success, or -1 on error (config is null, or kind is
 *          outside {0, 1}); check thetadatadx_last_error(). */
int32_t thetadatadx_config_with_market_data_environment(ThetaDataDxConfig* config, int32_t kind);

/** Select the streaming environment on a config handle in place:
 *  kind 0 = production, kind 1 = dev, kind 2 = staging. The streaming and
 *  market-data channels are selected independently, so this leaves the
 *  market-data channel and the auth marker untouched.
 *  @param config Config handle to mutate.
 *  @param kind 0 for PROD, 1 for DEV, 2 for STAGE.
 *  @return 0 on success, or -1 on error (config is null, or kind is
 *          outside {0, 1, 2}); check thetadatadx_last_error(). */
int32_t thetadatadx_config_with_streaming_environment(ThetaDataDxConfig* config, int32_t kind);

/** Source a config from a .env-format file. Starts from the production
 *  configuration and applies the cluster keys carried by the file:
 *  THETADATA_MARKET_DATA_TYPE (PROD / STAGE, case-insensitive) selects the
 *  environment, and the optional THETADATA_MARKET_DATA_HOST /
 *  THETADATA_STREAMING_HOST keys override the hosts (an explicit host wins
 *  over the environment default). Reads the same file format and keys as
 *  thetadatadx_credentials_from_dotenv, so one .env can carry both
 *  THETADATA_API_KEY and THETADATA_MARKET_DATA_TYPE.
 *  @param path Path to the .env file.
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free, or NULL on error
 *          (check thetadatadx_last_error()). */
ThetaDataDxConfig* thetadatadx_config_from_dotenv(const char* path);

/** Release a config handle.
 *  @param config Handle from a config factory; no-op when NULL.
 *                Call exactly once. */
void thetadatadx_config_free(ThetaDataDxConfig* config);

/* Generated from the config table: every accessor that reads or writes one
 * value. The accessors below it carry their own logic. */
#include "config_accessors.h.inc"

/**
 * Set the streaming reconnect policy on a config handle.
 *   policy=0: Auto (default) -- auto-reconnect with split per-class attempt
 *             budgets. Generic transient failures (TimedOut, ServerRestarting,
 *             Unspecified) use the budget set by
 *             `thetadatadx_config_set_reconnect_max_attempts`; the rate-limited
 *             (`TooManyRequests`) class uses
 *             `thetadatadx_config_set_reconnect_max_rate_limited_attempts`. Counters
 *             reset after a continuous data-flow window configured via
 *             `thetadatadx_config_set_reconnect_stable_window_secs`.
 *   policy=1: Manual -- no auto-reconnect.
 * @param config Config handle to mutate.
 * @param policy Reconnect policy selector (0 = Auto, 1 = Manual).
 * @return 0 on success, -1 on an invalid policy (outside {0, 1}) or null
 *         config. A rejected policy sets thetadatadx_last_error_code to
 *         THETADATADX_ERR_INVALID_PARAMETER so an unknown value is rejected with the
 *         same typed class the Python / TypeScript bindings raise, never
 *         silently coerced to Auto.
 */
int32_t thetadatadx_config_set_reconnect_policy(ThetaDataDxConfig* config, int policy);


/**
 * Read the configured reconnect policy selector.
 * @param config Config handle to read.
 * @param out_policy Receives 0 (Auto), 1 (Manual), or 2 (Custom) on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_policy(const ThetaDataDxConfig* config, int32_t* out_policy);

/**
 * Reconnect-decision callback for thetadatadx_config_set_reconnect_callback.
 * Invoked on an SDK streaming thread after each retriable involuntary
 * disconnect.
 * @param reason The disconnect-reason discriminant.
 * @param attempt The 1-based consecutive-reconnect counter.
 * @param user_data The opaque pointer registered alongside the callback.
 * @return The reconnect delay in milliseconds, or any negative value to
 *         stop reconnecting.
 * @note This callback must not unwind across the C ABI. A C++ throw or a C
 *       longjmp that escapes the callback into the calling frame is undefined
 *       behavior, the same as for any C library. The library wraps each
 *       invocation to contain a fault on its own side of the boundary, but
 *       that does not contain an exception thrown out of your callback. Catch
 *       and handle every exception before returning a decision.
 */
typedef int64_t (*ThetaDataDxReconnectCallback)(int32_t reason, uint32_t attempt, void* user_data);

/**
 * Install a custom reconnect policy driven by a C callback. Permanent
 * disconnect reasons never reach the callback.
 * @param config Config handle to mutate.
 * @param cb The reconnect-decision callback; NULL restores the default Auto
 *           policy.
 * @param user_data Opaque pointer passed back to cb unchanged.
 * @return 0 on success, -1 if config is null.
 * @note cb runs on an SDK streaming thread: cb and user_data must be safe
 *       to use from another thread for as long as any client built from
 *       this config is alive.
 */
int32_t thetadatadx_config_set_reconnect_callback(ThetaDataDxConfig* config, ThetaDataDxReconnectCallback cb,
                                          void* user_data);

/**
 * Set the streaming event ring buffer size (slots). Must be a power of two
 * from 64 to 2^24; invalid values are rejected at the setter
 * (thetadatadx_last_error). Default 131_072.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Ring buffer size in slots (power of two, 64 to 2^24).
 */
void thetadatadx_config_set_streaming_ring_size(ThetaDataDxConfig* config, size_t n);

/**
 * Set the async worker-thread count for embedded runtimes, using the
 * widened (has_value, n) shape so an unset value is distinct from any
 * explicit count across the Python / TypeScript / C++ bindings.
 *
 * The async worker pool is process-global: it is built once, from the
 * config of the first client connected in the process. This setting is
 * therefore honoured when the first client in the process is created;
 * clients connected later share the already-built pool, so setting it on
 * a subsequent config has no effect.
 * @param config Config handle to mutate.
 * @param has_value false leaves the count unset (auto-sized) and ignores n;
 *                  true sets an explicit count. An explicit 0 is preserved
 *                  across the boundary; the connection clamps it to 1.
 * @param n The explicit worker-thread count, honoured only when has_value
 *          is true.
 * @return 0 on success, -1 if config is NULL.
 */
int32_t thetadatadx_config_set_worker_threads(ThetaDataDxConfig* config, bool has_value, size_t n);

/**
 * Read the current async worker-thread count, using the same widened
 * (has_value, n) shape.
 * @param config Config handle to read.
 * @param out_has_value Receives false when the count is unset (auto-sized),
 *                      true when an explicit count is set.
 * @param out_n Receives the count (0 when unset, the explicit count otherwise).
 * @return 0 on success, -1 if any pointer is null.
 */
int32_t thetadatadx_config_get_worker_threads(const ThetaDataDxConfig* config, bool* out_has_value, size_t* out_n);

/**
 * Read the market-data environment carried by the config: "PROD"
 * for the production cluster or "STAGE" for staging. The market-data and
 * streaming environments are selected independently; the production /
 * stage / dev presets (and the THETADATA_MARKET_DATA_TYPE dotenv key) set the
 * market-data channel, and this is the readback of that selection.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST free with
 *         thetadatadx_string_free, or NULL if config is null.
 */
char* thetadatadx_config_get_market_data_environment(const ThetaDataDxConfig* config);

/**
 * Read the streaming environment carried by the config: "PROD" for
 * the production cluster, "STAGE" for staging or "DEV" for the dev
 * cluster. The streaming and market-data environments are selected
 * independently; the production / stage / dev presets (and the
 * THETADATA_STREAMING_TYPE dotenv key) set the streaming channel, and this
 * is the readback of that selection.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST free with
 *         thetadatadx_string_free, or NULL if config is null.
 */
char* thetadatadx_config_get_streaming_environment(const ThetaDataDxConfig* config);

/* Sentinel for thetadatadx_config_set_consumer_cpu /
 * _get_consumer_cpu: a negative core id means "unpinned" (the default,
 * OS scheduler). */
#define THETADATADX_CONSUMER_CPU_UNPINNED (-1)

/**
 * Pin the streaming consumer thread to a CPU core.
 *   core >= 0: pin the tick-consumer thread to that core (deterministic,
 *              low-jitter delivery; out-of-range/offline core is a no-op).
 *   core <  0: unpinned (THETADATADX_CONSUMER_CPU_UNPINNED, the default).
 * @param config Config handle to mutate.
 * @param core Core id, or a negative value for unpinned.
 * @return 0 on success, -1 (THETADATADX_ERR_CONFIG) on a null handle.
 */
int32_t thetadatadx_config_set_consumer_cpu(ThetaDataDxConfig* config, int64_t core);

/**
 * Read the streaming consumer-thread CPU pin.
 * @param config Config handle to read.
 * @param out_core Receives the pinned core, or
 *        THETADATADX_CONSUMER_CPU_UNPINNED (-1) when unpinned.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_consumer_cpu(const ThetaDataDxConfig* config, int64_t* out_core);

/* ── MarketDataClient ── */

/** Connect a market-data client to ThetaData servers.
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A connected client the caller must release with
 *          thetadatadx_market_data_free, or NULL on connection/auth failure
 *          (check thetadatadx_last_error()). */
ThetaDataDxMarketDataClient* thetadatadx_market_data_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect a market-data client, reading credentials from a file
 *  (line 1 = email, line 2 = password). One-call equivalent of
 *  thetadatadx_credentials_from_file + thetadatadx_market_data_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A connected client the caller must release with
 *          thetadatadx_market_data_free, or NULL on argument or connection/auth
 *          failure (check thetadatadx_last_error()). */
ThetaDataDxMarketDataClient* thetadatadx_market_data_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Release a market-data client handle.
 *  @param client Handle from a thetadatadx_market_data_connect* call; no-op when
 *                NULL. Call exactly once. */
void thetadatadx_market_data_free(ThetaDataDxMarketDataClient* client);

/* ── String free ── */

/** Free a string returned by any thetadatadx_* function.
 *  @param s Heap-owned string from a thetadatadx_* call; no-op when NULL. Call
 *           exactly once. */
void thetadatadx_string_free(char* s);

/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/** User callback signature for the thetadatadx_<endpoint>_stream server-stream entry
 *  points. Invoked once per decoded chunk drained from a market-data result.
 *
 *  `rows` points at the first element of a contiguous run of `len` tick
 *  structs -- the SAME layout the matching thetadatadx_<endpoint>_with_options array
 *  returns (e.g. a thetadatadx_option_history_trade_stream chunk is `len` x
 *  ThetaDataDxTradeTick). Cast `rows` to that tick pointer type before indexing. The
 *  pointer is valid only for the duration of the call -- copy any rows the
 *  caller wants to outlive the callback. An empty result drains as zero
 *  invocations (a null `rows` with `len == 0` is never delivered).
 *
 *  `columns` is the chunk's column presence, never NULL: the columns the
 *  chunk's wire carried and the root each row belongs to. When the chunk's
 *  rows span several roots, `symbols` holds one root per row
 *  (`symbols_len == len`); when every row shares one, as in option and index
 *  history and a flat-file chunk inside one root, `symbol` holds it, so a
 *  flat-file reader checks both. A response that carries no root, such as
 *  stock history, sets neither. The library owns it: it is valid only for
 *  the duration of the call and must not be freed by the callback.
 *
 *  `ctx` is the opaque pointer registered alongside the callback; it is passed
 *  back unchanged on every invocation.
 *
 *  Return 0 to receive the next chunk, or any other value to stop the stream:
 *  the download is cancelled, no further chunk is delivered, and the `_stream`
 *  call returns 0.
 *
 *  The callback runs SERIALLY, but not necessarily on the thread that called
 *  the `_stream` entry point: the library may invoke it from an internal
 *  runtime worker while the calling thread blocks in the drain. Do not assume
 *  the calling thread's thread-local / UI / COM affinity is available inside
 *  the callback; marshal to the owning thread if you need it.
 *
 *  This callback must not unwind across the C ABI. A C++ throw or a C longjmp
 *  that escapes the callback into the calling frame is undefined behavior, the
 *  same as for any C library. The library wraps each invocation to contain a
 *  fault on its own side of the boundary, but that does not contain an
 *  exception thrown out of your callback. Catch and handle every exception
 *  before returning. */
typedef int32_t (*ThetaDataDxTickChunkCallback)(const void* rows, size_t len, const ThetaDataDxColumnPresence* columns, void* ctx);

/* Generated server-stream endpoint declarations. */
#include "market_data_stream.h.inc"

/** Each flat file's thetadatadx_<endpoint>_to_path writes the file for `date`
 *  to `path` as the terminal writes it. `format` is "csv" (also when NULL),
 *  "json", "json_legacy", "ndjson" (or "jsonl") or "html"; any other name
 *  fails with THETADATADX_ERR_INVALID_PARAMETER. The file streams to disk
 *  chunk by chunk and takes its name only once complete; a path without an
 *  extension gets the format's. Returns 0 on success, -1 on error (inspect
 *  thetadatadx_last_error()). */
#include "market_data_to_path.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Cross-language utility helpers — conditions / exchange / sequences   */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All `thetadatadx_*_name` / `thetadatadx_*_description` / `thetadatadx_exchange_*` returns are
 * NUL-terminated UTF-8 C strings owned by the library — DO NOT FREE. The
 * pointer remains valid for the lifetime of the process. Unknown codes
 * return either "UNKNOWN" (name lookup) or "" (description lookup), never
 * NULL. */

/** Trade condition name lookup.
 *  @param code The trade condition code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_condition_name(int32_t code);

/** Trade condition description lookup.
 *  @param code The trade condition code.
 *  @return A process-lifetime string; "" for unrecognised codes. Never
 *          NULL; DO NOT FREE. */
const char* thetadatadx_condition_description(int32_t code);

/** Whether a trade condition code represents a cancellation.
 *  @param code The trade condition code.
 *  @return true if the code represents a cancellation, false otherwise. */
bool thetadatadx_condition_is_cancel(int32_t code);

/** Whether a trade condition code updates the volume bar.
 *  @param code The trade condition code.
 *  @return true if the code updates the volume bar, false otherwise. */
bool thetadatadx_condition_updates_volume(int32_t code);

/** Quote condition name lookup.
 *  @param code The quote condition code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_quote_condition_name(int32_t code);

/** Quote condition description lookup.
 *  @param code The quote condition code.
 *  @return A process-lifetime string; "" for unrecognised codes. Never
 *          NULL; DO NOT FREE. */
const char* thetadatadx_quote_condition_description(int32_t code);

/** Whether a quote condition is firm (binding).
 *  @param code The quote condition code.
 *  @return true if the quote condition is firm, false otherwise. */
bool thetadatadx_quote_condition_is_firm(int32_t code);

/** Whether a quote condition indicates a trading halt.
 *  @param code The quote condition code.
 *  @return true if the quote condition indicates a trading halt, false
 *          otherwise. */
bool thetadatadx_quote_condition_is_halted(int32_t code);

/** Exchange name lookup (e.g. 3 -> "NewYorkStockExchange").
 *  @param code The exchange code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_exchange_name(int32_t code);

/** Exchange MIC-like symbol lookup (e.g. 3 -> "NYSE").
 *  @param code The exchange code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_exchange_symbol(int32_t code);

/** Calendar day-type vocabulary lookup (0 -> "open", 1 -> "early_close",
 *  2 -> "full_close", 3 -> "weekend").
 *  @param code The calendar day-type code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_calendar_status_name(int32_t code);

/** Combine an Eastern-Time YYYYMMDD date and milliseconds-of-day into
 *  Unix epoch milliseconds (UTC, DST-aware). Usable with any
 *  (date, *_ms_of_day) pair on the tick structs.
 *  @param date An Eastern-Time YYYYMMDD date.
 *  @param ms_of_day Milliseconds since midnight Eastern.
 *  @return Unix epoch milliseconds, or -1 when date is not a valid YYYYMMDD
 *          (including the 0 absent fill) or ms_of_day is outside
 *          0..86,400,000. */
int64_t thetadatadx_timestamp_ms(int32_t date, int32_t ms_of_day);

/** Convert a signed wire-encoded trade-sequence value to its unsigned
 *  monotonic form.
 *  @param signed_value Wire value; must lie in the 32-bit signed wire range
 *                      (-2,147,483,648 ..= 2,147,483,647).
 *  @param out Receives the unsigned monotonic value on success.
 *  @return 0 on success; -1 with thetadatadx_last_error_code set to
 *          THETADATADX_ERR_INVALID_PARAMETER when signed_value is outside the wire
 *          range or out is null, so an out-of-range value is rejected rather
 *          than silently reinterpreted. */
int32_t thetadatadx_sequence_signed_to_unsigned(int64_t signed_value, uint64_t* out);

/** Convert an unsigned monotonic trade-sequence value back to its signed
 *  wire encoding.
 *  @param unsigned_value Monotonic value; must lie in the unsigned wire
 *                        range (0 ..= 2^32 - 1).
 *  @param out Receives the signed wire value on success.
 *  @return 0 on success; -1 with thetadatadx_last_error_code set to
 *          THETADATADX_ERR_INVALID_PARAMETER when unsigned_value is above the wire
 *          range or out is null. */
int32_t thetadatadx_sequence_unsigned_to_signed(uint64_t unsigned_value, int64_t* out);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Streaming — C-layout event types                                      */
/* ═══════════════════════════════════════════════════════════════════════ */

/* Streaming event structs are schema-driven. The include below pulls in
 * the typedefs generated at build time from the canonical wire schema — so
 * the C++ header can never drift from it again. See `thetadatadx.hpp` for
 * `static_assert(offsetof)` guards that fail the build at compile time if
 * the schema and the C++ consumer ever disagree.
 *
 * Each variant is a typed fixed-layout struct. Consumers dispatch via
 * `event->kind` and read the matching `event-><variant>` payload —
 * for example
 *
 *   if (event->kind == THETADATADX_STREAM_LOGIN_SUCCESS)
 *       printf("perms=%s\n", event->login_success.permissions);
 *   if (event->kind == THETADATADX_STREAM_DISCONNECTED)
 *       printf("reason=%d\n", event->disconnected.reason);
 *
 * `Disconnected`, `Reconnecting` and `ReconnectsExhausted` also carry
 * `cause`, saying where the disconnect came from: 0 the server sent a
 * disconnect message, 1 the server ended the stream without one, 2 a
 * read failed, 3 a write failed, 4 nothing arrived inside the client
 * read deadline. `cause_io_error_kind` names the I/O error kind behind
 * a read or write failure and is empty otherwise; `cause_timeout_ms`
 * is the read deadline that expired and is zero otherwise. C++ callers
 * resolve the code with thetadatadx::cause_name().
 *
 * Borrowed pointers (`Contract.symbol`, `LoginSuccess.permissions`,
 * `ServerError.message`, `ParseError.message`, `Ping.payload`,
 * `UnknownFrame.payload`, and `cause_io_error_kind` on `Disconnected`,
 * `Reconnecting` and `ReconnectsExhausted`) are valid only for the
 * duration of the user callback — copy out before returning. Copying
 * the payload struct by value copies these pointers, not what they
 * point at. Do NOT free. */
#include "fpss_event_structs.h.inc"

/** Read the option strike of a streaming ThetaDataDxContract in dollars, folding
 *  the has_strike presence flag into the return value. ThetaDataDxContract.strike
 *  already carries dollars; this surfaces the presence flag a plain field
 *  read would drop. Mirrors the C++ thetadatadx::strike(const ThetaDataDxContract&) accessor.
 *  @param contract The streaming contract to read.
 *  @param out_dollars Receives the strike in dollars when the contract is an
 *                     option; left untouched otherwise.
 *  @return true when the contract is an option and out_dollars was written;
 *          false for a non-option, null contract, or null output pointer. */
bool thetadatadx_contract_strike_dollars(const ThetaDataDxContract* contract, double* out_dollars);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Real-time streaming client                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Allocate a streaming-only handle. Opens nothing and issues no
 *  authentication request: the streaming connection is opened by
 *  thetadatadx_streaming_set_callback. Every thetadatadx_streaming_* call
 *  follows the contract of its thetadatadx_client_* counterpart.
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A streaming handle the caller must release with thetadatadx_streaming_free, or
 *          NULL on failure (check thetadatadx_last_error()). */
ThetaDataDxStreamHandle* thetadatadx_streaming_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect to the real-time streaming servers, reading credentials from a
 *  file (line 1 = email, line 2 = password). One-call equivalent of
 *  thetadatadx_credentials_from_file + thetadatadx_streaming_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A streaming handle the caller must release with thetadatadx_streaming_free, or
 *          NULL on failure (check thetadatadx_last_error()). */
ThetaDataDxStreamHandle* thetadatadx_streaming_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Polymorphic subscribe / unsubscribe — see ThetaDataDxSubscriptionRequest below. */

/** Report whether the streaming connection is currently open. Distinct
 *  from thetadatadx_streaming_is_authenticated: the connection can be open yet
 *  briefly unauthenticated mid-reconnect.
 *  @param h The streaming handle.
 *  @return 1 when streaming, 0 otherwise (including after shutdown). */
int thetadatadx_streaming_is_streaming(const ThetaDataDxStreamHandle* h);

/** Report whether the streaming session is authenticated.
 *  @param h The streaming handle.
 *  @return 1 when authenticated, 0 otherwise. */
int thetadatadx_streaming_is_authenticated(const ThetaDataDxStreamHandle* h);

/** Read the active subscriptions as a typed array.
 *  @param h The streaming handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free, or NULL (check
 *          thetadatadx_last_error()) when streaming is not active. */
ThetaDataDxSubscriptionArray* thetadatadx_streaming_active_subscriptions(const ThetaDataDxStreamHandle* h);

/** Read the active full-stream subscriptions as a typed array. Each
 *  entry's `contract` field carries the security-type discriminant
 *  (`"Stock"` / `"Option"` / `"Index"`); the `kind` field is the
 *  full-stream label (`"full_trades"` / `"full_open_interest"`).
 *  @param h The streaming handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free. */
ThetaDataDxSubscriptionArray* thetadatadx_streaming_active_full_subscriptions(const ThetaDataDxStreamHandle* h);

/** User callback signature for thetadatadx_*_set_callback.
 *  `event` is valid only for the duration of the call -- copy any fields the
 *  caller wants to outlive the callback. `ctx` is the opaque pointer the
 *  caller registered alongside the callback; it is passed back unchanged.
 *
 *  This callback must not unwind across the C ABI. A C++ throw or a C longjmp
 *  that escapes the callback into the calling frame is undefined behavior, the
 *  same as for any C library. The library wraps each invocation to contain a
 *  fault on its own side of the boundary, but that does not contain an
 *  exception thrown out of your callback. Catch and handle every exception
 *  before returning. (The C++ wrapper's set_callback handles this for you: its
 *  shim is noexcept and swallows any exception the std::function raises.) */
typedef void (*ThetaDataDxStreamCallback)(const ThetaDataDxStreamEvent* event, void* ctx);

/** Register a streaming callback and open the streaming connection.
 *  Same contract as thetadatadx_client_set_callback.
 *
 *  Events flow from the streaming reader through a bounded ring to a
 *  dedicated consumer thread, which invokes the callback inside an
 *  isolation boundary. The reader thread NEVER blocks on user code:
 *  on ring overflow events are dropped and counted (thetadatadx_streaming_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) thetadatadx_streaming_free() returns
 *  (which performs shutdown if needed and applies the drain barrier
 *  internally with a 5 s timeout), or (b) thetadatadx_streaming_shutdown() /
 *  thetadatadx_streaming_reconnect() returns AND thetadatadx_streaming_await_drain() has
 *  returned 1. The consumer thread accesses ctx on every event and on
 *  every thetadatadx_streaming_reconnect(), serially on a single thread. Freeing ctx
 *  without one of these barriers is undefined behavior. Do not call
 *  thetadatadx_streaming_shutdown() while a thetadatadx_streaming_reconnect()
 *  is running on another thread: a reconnect the shutdown lands in still binds
 *  a fresh session to the same ctx.
 *
 *  The consumer thread invokes `callback(event, ctx)` serially on
 *  a single thread. The user does NOT need internal locks for callback-
 *  private state.
 *
 *  ## Lifecycle contract (replacement after shutdown)
 *
 *  Calling this while streaming is active returns -1 with "streaming already
 *  started". After thetadatadx_streaming_shutdown() the handle accepts a fresh
 *  registration, which REPLACES the saved (callback, ctx) and starts a session
 *  with nothing subscribed: the shutdown dropped the subscriptions, so
 *  subscribe again.
 *
 *  @param h        The streaming handle.
 *  @param callback The callback invoked once per streaming event.
 *  @param ctx      Opaque user pointer passed to every callback invocation.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_set_callback(const ThetaDataDxStreamHandle* h, ThetaDataDxStreamCallback callback, void* ctx);

/** Reconnect the live streaming session under the registered callback,
 *  replaying its subscriptions. A retry after a failed reconnect replays the
 *  subscriptions the failed attempt saved. Before
 *  thetadatadx_streaming_set_callback, and after
 *  thetadatadx_streaming_shutdown (which drops the subscriptions), there is
 *  nothing to reconnect: resume with thetadatadx_streaming_set_callback and
 *  subscribe again.
 *  @param h The streaming handle.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()); -1
 *          with THETADATADX_ERR_NETWORK when no session is live and no
 *          failed reconnect is waiting to be retried; -1 with
 *          THETADATADX_ERR_STREAM if
 *          the session reconnected but some subscriptions failed to restore
 *          (the stream stays live and
 *          thetadatadx_streaming_active_subscriptions lists what was
 *          restored). */
int thetadatadx_streaming_reconnect(const ThetaDataDxStreamHandle* h);

/** Cumulative count of streaming events that could not be published into
 *  the bounded ring because the consumer fell behind and the ring was full.
 *  @param h The streaming handle.
 *  @return The dropped-event count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_streaming_dropped_events(const ThetaDataDxStreamHandle* h);

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the feed and the dispatcher. Rising occupancy that approaches
 *  thetadatadx_streaming_ring_capacity predicts drops before thetadatadx_streaming_dropped_events
 *  moves; sampling never blocks the feed and is safe from any thread.
 *  @param h The streaming handle.
 *  @return The current ring occupancy, or 0 if the handle is null or no
 *          session is live. */
uint64_t thetadatadx_streaming_ring_occupancy(const ThetaDataDxStreamHandle* h);

/** Configured capacity of the streaming event ring in slots (the
 *  streaming_ring_size setting, a power of two) — the fixed denominator for
 *  thetadatadx_streaming_ring_occupancy.
 *  @param h The streaming handle.
 *  @return The ring capacity in slots, or 0 if the handle is null or no
 *          session is live. */
uint64_t thetadatadx_streaming_ring_capacity(const ThetaDataDxStreamHandle* h);

/** Milliseconds since the most recent inbound streaming frame of any kind
 *  on this streaming handle.
 *  @param h The streaming handle.
 *  @param out_ms Receives the elapsed milliseconds on success.
 *  @return 0 on success with the value in *out_ms, 1 when no session is live
 *          or no frame has been received yet, -1 on a null pointer. */
int32_t thetadatadx_streaming_millis_since_last_event(const ThetaDataDxStreamHandle* h, uint64_t* out_ms);

/** UNIX-nanosecond receive timestamp of the most recent inbound streaming
 *  frame of any kind on this streaming handle.
 *  @param h The streaming handle.
 *  @return The receive timestamp in Unix nanoseconds, or 0 when the handle
 *          is null, no session is live, or no frame has been received yet. */
int64_t thetadatadx_streaming_last_event_received_at_unix_nanos(const ThetaDataDxStreamHandle* h);

/** Address (host:port) of the streaming server the current session is
 *  connected to, following the session across auto-reconnects.
 *  @param h The streaming handle.
 *  @return A heap-owned C string the caller must release with
 *          thetadatadx_string_free, or NULL when no session is live. */
char* thetadatadx_streaming_last_connected_addr(const ThetaDataDxStreamHandle* h);



/** Cumulative count of user-callback failures contained by the
 *  per-invocation isolation boundary, across every session this handle has
 *  run. If the callback aborts on a given event, the failure is contained,
 *  recorded here, and does not stop event delivery: the next event continues
 *  normally. Safe to call from any thread.
 *  @param h The streaming handle.
 *  @return The contained-failure count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_streaming_panic_count(const ThetaDataDxStreamHandle* h);

/** Stop streaming. Same contract as thetadatadx_client_stop_streaming:
 *  idempotent, and returns once the consumer thread has exited, except when
 *  called from inside the callback. The shutdown drops the subscriptions, so
 *  thetadatadx_streaming_reconnect after it returns -1 with
 *  THETADATADX_ERR_NETWORK; resume with thetadatadx_streaming_set_callback
 *  and subscribe again.
 *  @param h The streaming handle.
 *  @note Pair with thetadatadx_streaming_await_drain() (or use thetadatadx_streaming_free(), which
 *        applies the drain barrier internally) before freeing the callback
 *        ctx. */
void thetadatadx_streaming_shutdown(const ThetaDataDxStreamHandle* h);

/** Wait for the previously-superseded streaming session to quiesce.
 *  @param h The streaming handle.
 *  @param timeout_ms Maximum time to wait, in milliseconds.
 *  @return 1 once every previous thetadatadx_streaming_reconnect / thetadatadx_streaming_shutdown
 *          session has finished firing the registered callback, and also when
 *          no session was ever stopped on this handle (nothing to drain); 0
 *          on timeout or a NULL handle.
 *  @note Must be called from a thread other than the streaming consumer;
 *        calling it from inside the user callback would block the very work
 *        it waits on and always time out. */
int thetadatadx_streaming_await_drain(const ThetaDataDxStreamHandle* h, uint64_t timeout_ms);

/** Free the streaming handle. If streaming is still live, this stops it
 *  first. Returns only after the registered callback has finished firing
 *  (internal 5-second drain barrier). On drain timeout it logs an error and
 *  proceeds with destruction; in that diagnostic case the callback may still
 *  be firing, so user code must keep ctx valid past return. Under normal
 *  operation drain completes in low single-digit milliseconds, so ctx is
 *  safe to free immediately on return.
 *  @warning Do NOT call this from inside the registered callback. The drain
 *           barrier waits for the callback to return, so a self-call blocks for
 *           the full 5-second timeout, then destroys the handle and resumes the
 *           callback against freed state (use-after-free). Tear the handle down
 *           from another thread: signal the callback to stop and free after it
 *           returns, or free from the owning thread once streaming is done.
 *  @param h The streaming handle; no-op when NULL. Call exactly once. */
void thetadatadx_streaming_free(ThetaDataDxStreamHandle* h);

/* ======================================================================= */
/*  Unified client -- market-data + streaming through one handle            */
/* ======================================================================= */

/** Connect to ThetaData (market-data only -- real-time streaming is NOT started).
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A unified handle the caller must release with thetadatadx_client_free, or
 *          NULL on connection/auth failure (check thetadatadx_last_error()). */
ThetaDataDxClient* thetadatadx_client_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect a unified client, reading credentials from a file (line 1 = email,
 *  line 2 = password). One-call equivalent of thetadatadx_credentials_from_file +
 *  thetadatadx_client_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A unified handle the caller must release with thetadatadx_client_free, or
 *          NULL on argument or connection/auth failure (check
 *          thetadatadx_last_error()). */
ThetaDataDxClient* thetadatadx_client_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Register a streaming callback and start streaming on the unified client.
 *
 *  Events flow from the streaming reader through a bounded ring to a
 *  dedicated consumer thread, which invokes the callback inside an
 *  isolation boundary. Reader never blocks on user code; ring-overflow
 *  events are dropped (thetadatadx_client_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) thetadatadx_client_free()
 *  returns (which calls stop_streaming and applies the drain barrier
 *  internally with a 5 s timeout), (b) thetadatadx_client_stop_streaming() /
 *  thetadatadx_client_reconnect() returns AND thetadatadx_client_await_drain() has
 *  returned 1, or (c) a successful replacement thetadatadx_client_set_callback
 *  has returned AND thetadatadx_client_await_drain() has returned 1 for the
 *  prior session. The consumer thread accesses ctx on every event and
 *  reconnect, serially on a single thread. Freeing ctx without one of
 *  these barriers is undefined behavior. Do not call
 *  thetadatadx_client_stop_streaming() while a thetadatadx_client_reconnect()
 *  is running on another thread: a reconnect the stop lands in still binds a
 *  fresh session to the same ctx.
 *
 *  ## Lifecycle contract (REPLACEMENT after stop)
 *
 *  Stop+register is a normal user flow: after thetadatadx_client_stop_streaming
 *  another thetadatadx_client_set_callback REPLACES the saved (callback, ctx).
 *  thetadatadx_client_reconnect is built on top of this. Calling set_callback
 *  while streaming is already active returns -1 with "streaming already
 *  started".
 *
 *  @param handle   The unified handle.
 *  @param callback The callback invoked once per streaming event.
 *  @param ctx      Opaque user pointer passed to every callback invocation.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_set_callback(const ThetaDataDxClient* handle, ThetaDataDxStreamCallback callback, void* ctx);

/** Subscription request scope discriminator (ThetaDataDxSubscriptionRequest.scope). */
#define THETADATADX_SUB_SCOPE_CONTRACT 0
#define THETADATADX_SUB_SCOPE_FULL     1

/** Subscription kind discriminator (ThetaDataDxSubscriptionRequest.kind). */
#define THETADATADX_SUB_KIND_QUOTE         0
#define THETADATADX_SUB_KIND_TRADE         1
#define THETADATADX_SUB_KIND_OPEN_INTEREST 2
#define THETADATADX_SUB_KIND_MARKET_VALUE  3

/** Polymorphic subscribe / unsubscribe request payload.
 *
 * One payload type expresses every subscription shape across the C ABI.
 *
 * - Per-contract stock: scope=CONTRACT, symbol="AAPL", option fields NULL.
 * - Per-contract option: scope=CONTRACT, symbol="SPY", expiration / strike / right set.
 * - Full-stream: scope=FULL, sec_type="OPTION" (or "STOCK", "INDEX"), per-contract fields NULL.
 */
typedef struct {
    int32_t scope;            /* THETADATADX_SUB_SCOPE_CONTRACT or THETADATADX_SUB_SCOPE_FULL */
    int32_t kind;             /* THETADATADX_SUB_KIND_QUOTE / _TRADE / _OPEN_INTEREST / _MARKET_VALUE */
    const char* symbol;       /* per-contract only */
    const char* expiration;   /* per-contract option only */
    const char* strike;       /* per-contract option only */
    const char* right;        /* per-contract option only */
    const char* sec_type;     /* full-stream; or STOCK/INDEX for a per-contract underlier (NULL or empty defaults to stock) */
} ThetaDataDxSubscriptionRequest;

/** Polymorphic subscribe on the unified client.
 *  @param handle The unified handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_subscribe(const ThetaDataDxClient* handle, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the unified client.
 *  @param handle The unified handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_unsubscribe(const ThetaDataDxClient* handle, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic subscribe on the standalone streaming client.
 *  @param h The streaming handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_subscribe(const ThetaDataDxStreamHandle* h, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the standalone streaming client.
 *  @param h The streaming handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_unsubscribe(const ThetaDataDxStreamHandle* h, const ThetaDataDxSubscriptionRequest* request);

/** Reconnect unified streaming, re-subscribing all previous subscriptions.
 *  After thetadatadx_client_stop_streaming, which drops the subscriptions,
 *  there is nothing to reconnect: resume with thetadatadx_client_set_callback
 *  and subscribe again.
 *  @param handle The unified handle.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()); -1
 *          with THETADATADX_ERR_NETWORK when no session is live and no
 *          failed reconnect is waiting to be retried; -1
 *          with THETADATADX_ERR_STREAM if the session reconnected but some
 *          subscriptions failed to restore (the stream stays live and
 *          thetadatadx_client_active_subscriptions lists what was
 *          restored). */
int thetadatadx_client_reconnect(const ThetaDataDxClient* handle);

/** Report whether streaming is active on the unified client.
 *  @param handle The unified handle.
 *  @return 1 when streaming, 0 otherwise. */
int thetadatadx_client_is_streaming(const ThetaDataDxClient* handle);

/** Report whether the live streaming session is authenticated on the
 *  unified client. Distinct from thetadatadx_client_is_streaming: the session
 *  can be live yet briefly unauthenticated mid-reconnect.
 *  @param handle The unified handle.
 *  @return 1 when authenticated, 0 otherwise. */
int thetadatadx_client_is_authenticated(const ThetaDataDxClient* handle);

/** Read the active subscriptions as a typed array.
 *  @param handle The unified handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free. */
ThetaDataDxSubscriptionArray* thetadatadx_client_active_subscriptions(const ThetaDataDxClient* handle);

/** Read the active full-stream subscriptions as a typed array. Each entry's
 *  `contract` field carries the security-type discriminant
 *  ("Stock" / "Option" / "Index") the full-stream subscription is bound
 *  to; the `kind` field is the snake_case full-stream kind label
 *  ("full_trades" / "full_open_interest"), matching the Python /
 *  TypeScript `Subscription.kind` accessor.
 *  @param handle The unified handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free, or NULL on error. */
ThetaDataDxSubscriptionArray* thetadatadx_client_active_full_subscriptions(const ThetaDataDxClient* handle);

/** Borrow the market-data client from a unified handle.
 *  @param handle The unified handle.
 *  @return A borrowed market-data client pointer owned by the unified handle;
 *          do NOT free it. */
const ThetaDataDxMarketDataClient* thetadatadx_client_market_data(const ThetaDataDxClient* handle);

/** Stop streaming on the unified client. Market-data remains available.
 *  Returns once the consumer thread has exited, except when called from
 *  inside the callback. The stop drops the subscriptions; resume with
 *  thetadatadx_client_set_callback and subscribe again.
 *  @param handle The unified handle.
 *  @note Pair with thetadatadx_client_await_drain() (or use thetadatadx_client_free(),
 *        which applies the drain barrier internally) before freeing the
 *        callback ctx. */
void thetadatadx_client_stop_streaming(const ThetaDataDxClient* handle);

/** Wait for the previously-superseded streaming session to quiesce.
 *  @param handle The unified handle.
 *  @param timeout_ms Maximum time to wait, in milliseconds.
 *  @return 1 once the previous session has finished firing the registered
 *          callback, or when no stream has ever been started or stopped on
 *          this handle (an idle handle is already quiesced); 0 on timeout
 *          or a NULL handle.
 *  @note Must be called from a thread other than the streaming consumer. */
int thetadatadx_client_await_drain(const ThetaDataDxClient* handle, uint64_t timeout_ms);

/** Cumulative count of streaming events that could not be published into
 *  the bounded ring because the consumer fell behind and the ring was full.
 *  @param handle The unified handle.
 *  @return The dropped-event count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_dropped_events(const ThetaDataDxClient* handle);

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the feed and the dispatcher. Rising occupancy that approaches
 *  thetadatadx_client_ring_capacity predicts drops before thetadatadx_client_dropped_events
 *  moves; sampling never blocks the feed and is safe from any thread.
 *  @param handle The unified handle.
 *  @return The current ring occupancy, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_ring_occupancy(const ThetaDataDxClient* handle);

/** Configured capacity of the streaming event ring in slots (the
 *  streaming_ring_size setting, a power of two) — the fixed denominator for
 *  thetadatadx_client_ring_occupancy.
 *  @param handle The unified handle.
 *  @return The ring capacity in slots, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_ring_capacity(const ThetaDataDxClient* handle);

/** Milliseconds since the most recent inbound streaming frame of any kind
 *  on this unified handle.
 *  @param handle The unified handle.
 *  @param out_ms Receives the elapsed milliseconds on success.
 *  @return 0 on success with the value in *out_ms, 1 when streaming has not
 *          started or no frame has been received yet, -1 on a null pointer. */
int32_t thetadatadx_client_millis_since_last_event(const ThetaDataDxClient* handle, uint64_t* out_ms);

/** UNIX-nanosecond receive timestamp of the most recent inbound streaming
 *  frame of any kind on this unified handle.
 *  @param handle The unified handle.
 *  @return The receive timestamp in Unix nanoseconds, or 0 when the handle
 *          is null, streaming has not started, or no frame has been received
 *          yet. */
int64_t thetadatadx_client_last_event_received_at_unix_nanos(const ThetaDataDxClient* handle);

/** Address (host:port) of the streaming server the current session is
 *  connected to, following the session across auto-reconnects.
 *  @param handle The unified handle.
 *  @return A heap-owned C string the caller must release with
 *          thetadatadx_string_free, or NULL when streaming has not started. */
char* thetadatadx_client_last_connected_addr(const ThetaDataDxClient* handle);



/** Cumulative count of user-callback failures contained by the
 *  per-invocation isolation boundary, across every session this handle has
 *  run. If the callback aborts on a given event, the failure is contained,
 *  recorded here, and does not stop event delivery: the next event continues
 *  normally. Safe to call from any thread.
 *  @param handle The unified handle.
 *  @return The contained-failure count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_panic_count(const ThetaDataDxClient* handle);

/** Free a unified client handle. Calls thetadatadx_client_stop_streaming
 *  internally, then waits up to 5 seconds for the registered callback to
 *  finish firing before destroying the handle. On drain timeout it logs an
 *  error and proceeds with destruction; in that diagnostic case the callback
 *  may still be firing, so user code must keep ctx valid past return. Under
 *  normal operation drain completes in low single-digit milliseconds, so ctx
 *  is safe to free immediately on return.
 *  @warning Do NOT call this from inside the registered callback. The drain
 *           barrier waits for the callback to return, so a self-call blocks for
 *           the full 5-second timeout, then destroys the handle and resumes the
 *           callback against freed state (use-after-free). Tear the handle down
 *           from another thread: signal the callback to stop and free after it
 *           returns, or free from the owning thread once streaming is done.
 *  @param handle The unified handle; no-op when NULL. Call exactly once. */
void thetadatadx_client_free(ThetaDataDxClient* handle);

#ifdef __cplusplus
}
#endif

#endif /* THETADATADX_H */
