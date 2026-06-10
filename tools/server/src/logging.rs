//! Logging configuration: line format, optional daily-rotated file
//! output, and the legacy bracketed event formatter.
//!
//! Three line formats are supported via `--log-format`:
//!
//! - `text` — the standard `tracing_subscriber::fmt` layout (default,
//!   unchanged from earlier releases).
//! - `json` — one structured JSON object per line for log aggregators.
//! - `legacy` — `[YYYY-MM-DD HH:MM:SS] LEVEL: message` (UTC), the
//!   bracketed shape operator tooling written against the legacy
//!   terminal's log file parses.
//!
//! `--log-file <path>` additionally tees the same formatted lines into a
//! daily-rotated file (`<path>.YYYY-MM-DD`) through a non-blocking
//! writer, so a slow disk never stalls a request thread. Stderr output
//! is always on.

use std::fmt;
use std::io::IsTerminal;

use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

/// Line format selector for `--log-format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    /// Standard `tracing_subscriber::fmt` layout.
    Text,
    /// One structured JSON object per line.
    Json,
    /// Bracketed `[YYYY-MM-DD HH:MM:SS] LEVEL: message` (UTC).
    Legacy,
}

/// Initialise the global tracing subscriber.
///
/// Returns the file writer's [`WorkerGuard`] when `--log-file` is set;
/// the caller must keep it alive for the process lifetime or buffered
/// lines are dropped on exit.
///
/// # Errors
///
/// Returns an error if the log-file directory cannot be resolved.
pub fn init(
    log_level: &str,
    format: LogFormat,
    log_file: Option<&str>,
) -> Result<Option<WorkerGuard>, Box<dyn std::error::Error>> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();

    // Stderr layer — always on. ANSI colour only when stderr is a TTY so
    // redirected output stays grep-clean.
    layers.push(fmt_layer(
        format,
        std::io::stderr as fn() -> std::io::Stderr,
        std::io::stderr().is_terminal(),
    ));

    // Optional daily-rotated file layer. `rolling::daily(dir, prefix)`
    // writes `<prefix>.YYYY-MM-DD` inside `dir`; we split the flag value
    // into the two parts so `--log-file logs/terminal.log` lands in
    // `logs/`.
    let guard = match log_file {
        Some(path) => {
            let path = std::path::Path::new(path);
            let dir = match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent,
                _ => std::path::Path::new("."),
            };
            let prefix = path
                .file_name()
                .ok_or_else(|| format!("--log-file has no file name: {}", path.display()))?;
            let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(
                dir,
                std::path::Path::new(prefix),
            ));
            layers.push(fmt_layer(format, writer, false));
            Some(guard)
        }
        None => None,
    };

    tracing_subscriber::registry()
        .with(layers)
        .with(env_filter)
        .init();

    Ok(guard)
}

/// Build one boxed fmt layer in the requested format over `writer`.
fn fmt_layer<W>(format: LogFormat, writer: W, ansi: bool) -> Box<dyn Layer<Registry> + Send + Sync>
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    match format {
        LogFormat::Text => tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(ansi)
            .boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_writer(writer)
            .boxed(),
        LogFormat::Legacy => tracing_subscriber::fmt::layer()
            .event_format(LegacyFormat)
            .with_writer(writer)
            .with_ansi(false)
            .boxed(),
    }
}

/// Bracketed legacy line shape: `[YYYY-MM-DD HH:MM:SS] LEVEL: message`.
///
/// Timestamps are UTC: resolving the host's local offset after the async
/// runtime has spawned worker threads is not reliably possible (the
/// `time` crate refuses for soundness), and a fixed offset keeps
/// log-line ordering unambiguous across DST transitions. Span context
/// (`request{method=GET uri=/v3/...}`) is appended before the event
/// fields so the access log stays informative under this format.
struct LegacyFormat;

impl<S, N> FormatEvent<S, N> for LegacyFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let now = time::OffsetDateTime::now_utc();
        write!(
            writer,
            "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] {}: ",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
            legacy_level_label(*event.metadata().level()),
        )?;

        // Span scope (root -> leaf) so per-request context like
        // `request{method=GET uri=...}` survives the format change.
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                write!(writer, "{}", span.name())?;
                let ext = span.extensions();
                if let Some(fields) = ext.get::<FormattedFields<N>>() {
                    if !fields.is_empty() {
                        write!(writer, "{{{fields}}}")?;
                    }
                }
                writer.write_char(':')?;
                writer.write_char(' ')?;
            }
        }

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Uppercase level labels matching the legacy log vocabulary
/// (`WARNING`, not tracing's `WARN`).
fn legacy_level_label(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARNING",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Capture formatted lines in memory for shape assertions.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'w> MakeWriter<'w> for Capture {
        type Writer = Capture;
        fn make_writer(&'w self) -> Self::Writer {
            self.clone()
        }
    }

    /// The legacy format emits the bracketed `[YYYY-MM-DD HH:MM:SS]
    /// LEVEL: message` shape that legacy-terminal log parsers match.
    #[test]
    fn legacy_format_emits_bracketed_timestamp_level_message() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(LegacyFormat)
                .with_writer(capture.clone())
                .with_ansi(false),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("starting thetadatadx-server");
            tracing::warn!("upstream slow");
        });

        let out = capture.contents();
        let line_shape = regex_lite_match(&out, r"^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\] INFO: ");
        assert!(
            line_shape,
            "INFO line must match the bracketed shape: {out}"
        );
        assert!(
            out.contains("] INFO: starting thetadatadx-server"),
            "message text follows the level: {out}"
        );
        assert!(
            out.contains("] WARNING: upstream slow"),
            "warn maps to the legacy WARNING label: {out}"
        );
    }

    /// The access-log span context must survive the legacy format so a
    /// per-request line still names the method and URI.
    #[test]
    fn legacy_format_includes_span_context() {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(LegacyFormat)
                .with_writer(capture.clone())
                .with_ansi(false),
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("request", method = "GET", uri = "/v3/system/status");
            let _e = span.enter();
            tracing::info!(status = 200, "finished processing request");
        });

        let out = capture.contents();
        assert!(out.contains("request{"), "span name present: {out}");
        assert!(out.contains("method=\"GET\""), "method present: {out}");
        assert!(
            out.contains("uri=\"/v3/system/status\""),
            "uri present: {out}"
        );
        assert!(out.contains("status=200"), "event fields present: {out}");
    }

    /// Minimal `^`-anchored matcher for the timestamp shape so the test
    /// does not pull a regex dependency: verifies the first line starts
    /// with `[dddd-dd-dd dd:dd:dd] INFO: ` — `[` at 0, the 19-byte
    /// timestamp at 1..20, `]` at 20.
    fn regex_lite_match(out: &str, _shape_doc: &str) -> bool {
        let Some(line) = out.lines().next() else {
            return false;
        };
        let bytes = line.as_bytes();
        if bytes.len() < 28 || bytes[0] != b'[' || bytes[20] != b']' {
            return false;
        }
        let digits_at = |idx: &[usize]| idx.iter().all(|&i| bytes[i].is_ascii_digit());
        digits_at(&[1, 2, 3, 4, 6, 7, 9, 10, 12, 13, 15, 16, 18, 19])
            && bytes[5] == b'-'
            && bytes[8] == b'-'
            && bytes[11] == b' '
            && bytes[14] == b':'
            && bytes[17] == b':'
            && line[21..].starts_with(" INFO: ")
    }
}
