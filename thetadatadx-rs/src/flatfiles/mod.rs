//! Flat files: the whole universe of one dataset for one date.
//!
//! Each flat file is a market-data endpoint on [`MarketDataClient`], such as
//! `option_flat_file_trade_quote`, returning typed rows, with `.stream(handler)`
//! to take them chunk by chunk. Each also has a `<endpoint>_to_path` companion,
//! such as `option_flat_file_trade_quote_to_path`, that writes the file to disk
//! as the terminal writes it, in any [`FlatFileFormat`]: under the column names
//! the service sends, each cell as the service sent it. The file streams to disk
//! chunk by chunk, so memory stays flat whatever its size (`json_legacy` aside,
//! which holds the file until its last row, as the terminal does). It is written
//! under a temporary name beside the requested path and renamed onto it once
//! complete, so a failed or cancelled download never leaves a partial file under
//! that name. A path without an extension gets the format's.

mod format;
mod render;

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

pub use format::FlatFileFormat;
#[cfg(not(feature = "__internal"))]
use render::FileRenderer;
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use render::{FileRenderer, HTML_END, HTML_START, escape_html};

use crate::error::Error;
use crate::mdds::MarketDataClient;
use crate::util::random_id::random_id_hex;

/// Removes `path` on drop unless [`disarm`](Self::disarm) has been called.
///
/// Reaps the partial file on every exit path (a `?` early return, an error,
/// or a dropped, cancelled future) where a single post-await cleanup line
/// would be skipped on cancellation. Cleanup is best-effort and ignores a
/// missing file; the unlink is one synchronous syscall, so `Drop` can do it.
struct ScratchGuard {
    path: PathBuf,
    armed: bool,
}

impl ScratchGuard {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            armed: true,
        }
    }

    /// Cancel removal: the file has been renamed onto its final name.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A file being written: the renderer, the open file, and the first failure
/// to write it.
struct PartialFile {
    renderer: FileRenderer,
    out: BufWriter<File>,
    failure: Option<Error>,
}

impl MarketDataClient {
    /// Write the flat file `endpoint` (such as `option_flat_file_eod`) for
    /// `date` to `output_path` in `format`, and return the path written.
    /// Every `<endpoint>_to_path` method is this call with its endpoint named.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed date or an endpoint that is not a flat
    /// file, on network, authentication or decode failure, and when the file
    /// cannot be written.
    pub(crate) async fn write_flat_file(
        &self,
        endpoint: &str,
        date: &str,
        output_path: &Path,
        format: FlatFileFormat,
    ) -> Result<PathBuf, Error> {
        crate::mdds::validate::validate_date(date, "date")?;
        let path = format.ensure_extension(output_path);
        // The per-call id keeps two calls writing the same path from
        // truncating each other's partial file.
        let partial = {
            let mut name = path.as_os_str().to_owned();
            name.push(format!(".{}.tmp", random_id_hex()));
            PathBuf::from(name)
        };
        let mut guard = ScratchGuard::new(&partial);
        let file = tokio::sync::Mutex::new(PartialFile {
            renderer: FileRenderer::new(format),
            out: BufWriter::with_capacity(1 << 20, File::create(&partial).await?),
            failure: None,
        });
        self.flat_file_chunks(endpoint, date, |headers, rows| {
            let file = &file;
            async move {
                let mut file = file.lock().await;
                let written = match file.renderer.chunk(&headers, &rows) {
                    Ok(text) => file
                        .out
                        .write_all(text.as_bytes())
                        .await
                        .map_err(Error::from),
                    Err(error) => Err(error),
                };
                match written {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(error) => {
                        file.failure = Some(error);
                        ControlFlow::Break(())
                    }
                }
            }
        })
        .await?;
        let PartialFile {
            renderer,
            mut out,
            failure,
        } = file.into_inner();
        if let Some(error) = failure {
            return Err(error);
        }
        for piece in renderer.finish() {
            out.write_all(piece.as_bytes()).await?;
        }
        out.flush().await?;
        drop(out);
        // Windows `rename` (unlike Unix) fails when the destination exists,
        // so remove a prior file first there; the Unix path stays an atomic
        // replace.
        // shortcut: on Windows the publish is remove-then-rename, not atomic; use ReplaceFileW if a concurrent-writer race is ever observed.
        #[cfg(windows)]
        let _ = tokio::fs::remove_file(&path).await;
        tokio::fs::rename(&partial, &path).await?;
        guard.disarm();
        Ok(path)
    }
}

/// The by-name flat-file calls of the workspace's server, MCP server and
/// response recorder. A default build keeps them crate-private, so the wire
/// rows they pass to a handler stay off the public API.
#[cfg(feature = "__internal")]
impl MarketDataClient {
    /// Write the flat file `endpoint` for `date` to `output_path` in `format`,
    /// and return the path written.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed date or an endpoint that is not a flat
    /// file, on network, authentication or decode failure, and when the file
    /// cannot be written.
    #[doc(hidden)]
    pub async fn flat_file_to_path(
        &self,
        endpoint: &str,
        date: &str,
        output_path: &Path,
        format: FlatFileFormat,
    ) -> Result<PathBuf, Error> {
        self.write_flat_file(endpoint, date, output_path, format)
            .await
    }

    /// Stream the flat file `endpoint` for `date` to `handler` as the service
    /// sends it: each chunk's column headers and its cells, unconverted. A
    /// `Break` from `handler` ends the download.
    ///
    /// # Errors
    ///
    /// Returns an error for an endpoint that is not a flat file, and on
    /// network, authentication or decode failure.
    #[doc(hidden)]
    pub async fn stream_flat_file<F, Fut>(
        &self,
        endpoint: &str,
        date: &str,
        handler: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<crate::proto::DataValueList>) -> Fut,
        Fut: std::future::Future<Output = ControlFlow<()>>,
    {
        self.flat_file_chunks(endpoint, date, handler).await
    }
}
