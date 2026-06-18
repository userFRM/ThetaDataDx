//! Docs-site generator: one reference page per registry endpoint, the
//! streaming reference pages, the subscriptions matrix, the sidebar
//! manifests, and the `llms.txt` machine index.
//!
//! Reachable only from the `generate_docs_site` binary (gated on the
//! `__internal` feature because the sample-output renderer decodes the
//! checked-in capture fixtures through `thetadatadx::wire` +
//! `thetadatadx::decode`).
//!
//! Layout of emitted files (all under `docs-site/`):
//!   * `docs/reference/<rest-path>.md` — one page per non-stream endpoint,
//!     six-block anatomy: title + tier badges, language tab carousel
//!     (signature + runnable sample per language, HTTP as the fifth tab),
//!     description, parameters table, response schema, sample output.
//!   * `docs/streaming/<sec>/<kind>.md` — one page per stream type with
//!     the same language tab carousel for subscribe / callback code.
//!   * `docs/articles/subscriptions.md` — tier × endpoint capability
//!     matrix derived from the pinned upstream OpenAPI snapshot.
//!   * `docs/.vitepress/generated/*.json` — sidebar manifests imported
//!     by `config.ts` so navigation can never drift from the registry.
//!   * `docs/public/llms.txt` — one-line-per-page machine index.

mod articles;
mod lang;
mod llms;
mod page;
mod response;
mod samples;
mod streaming;

use std::path::Path;

use super::super::upstream_openapi::UpstreamOpenApi;
use super::helpers::is_streaming_endpoint;
use super::model::GeneratedEndpoint;
use super::parser::load_endpoint_specs;

/// One generated docs-site file: its path relative to the repo root and
/// its full rendered contents.
pub(super) struct DocFile {
    pub(super) relative_path: String,
    pub(super) contents: String,
}

/// Subscription tier for an endpoint, resolved from the pinned upstream
/// OpenAPI snapshot by `operationId`. Two endpoints have no upstream
/// counterpart and resolve through the explicit table below.
fn endpoint_tier(endpoint: &GeneratedEndpoint, upstream: &UpstreamOpenApi) -> String {
    // Endpoints absent from the upstream spec:
    //   * `stock_history_ohlc_range` is the ranged form of the upstream
    //     `stock_history_ohlc` endpoint — same data, same tier.
    //   * `interest_rate_history_eod` is not documented upstream; rate
    //     data is available on every tier.
    let lookup_name = match endpoint.name.as_str() {
        "stock_history_ohlc_range" => "stock_history_ohlc",
        "interest_rate_history_eod" => return "free".to_string(),
        other => other,
    };
    upstream
        .endpoint(lookup_name)
        .map(|e| e.min_subscription.clone())
        .unwrap_or_else(|| {
            panic!(
                "endpoint {} has no x-min-subscription in scripts/ci/data/upstream_openapi.yaml \
                 and no fallback in endpoint_tier()",
                endpoint.name
            )
        })
}

/// Docs path (relative to `docs-site/docs/`, no extension) for an
/// endpoint page. Derived from the REST path so the page tree mirrors
/// the HTTP surface one-for-one: `/v3/option/history/trade_greeks/all`
/// → `reference/option/history/trade-greeks/all`.
fn endpoint_page_path(endpoint: &GeneratedEndpoint) -> String {
    let rest = endpoint._rest_path.trim_start_matches("/v3/");
    let kebab = rest.replace('_', "-");
    format!("reference/{kebab}")
}

fn render_all(repo_root: &Path) -> Result<Vec<DocFile>, Box<dyn std::error::Error>> {
    let parsed = load_endpoint_specs()?;
    let tick_schema = super::super::ticks::schema::load_schema()?;
    let upstream = UpstreamOpenApi::load();

    let endpoints: Vec<&GeneratedEndpoint> = parsed
        .endpoints
        .iter()
        .filter(|e| !is_streaming_endpoint(e))
        .collect();

    let mut files = Vec::new();
    let mut page_index: Vec<page::PageMeta> = Vec::new();

    for endpoint in &endpoints {
        let tier = endpoint_tier(endpoint, upstream);
        let path = endpoint_page_path(endpoint);
        let (contents, meta) = page::render_endpoint_page(endpoint, &tier, &path, &tick_schema)?;
        files.push(DocFile {
            relative_path: format!("docs-site/docs/{path}.md"),
            contents,
        });
        page_index.push(meta);
    }

    files.push(DocFile {
        relative_path: "docs-site/docs/.vitepress/generated/reference-sidebar.json".into(),
        contents: page::render_reference_sidebar(&page_index),
    });

    let stream_pages = streaming::render_stream_pages()?;
    files.push(DocFile {
        relative_path: "docs-site/docs/.vitepress/generated/streaming-sidebar.json".into(),
        contents: streaming::render_streaming_sidebar(&stream_pages),
    });
    for (path, contents) in stream_pages {
        files.push(DocFile {
            relative_path: format!("docs-site/docs/{path}.md"),
            contents,
        });
    }

    files.push(DocFile {
        relative_path: "docs-site/docs/articles/subscriptions.md".into(),
        contents: articles::render_subscriptions_page(&endpoints, upstream),
    });

    // llms.txt is composed from the generated set above plus the
    // hand-written pages on disk, so it must be rendered last.
    files.push(DocFile {
        relative_path: "docs-site/docs/public/llms.txt".into(),
        contents: llms::render_llms_txt(repo_root, &files)?,
    });

    // Normalize: no trailing whitespace, exactly one trailing newline.
    for file in &mut files {
        let mut cleaned: String = file
            .contents
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        cleaned.push('\n');
        file.contents = cleaned;
    }

    Ok(files)
}

/// Renders every docs-site file and writes it to disk under `repo_root`,
/// creating parent directories as needed.
pub fn write_docs_site_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_all(repo_root)? {
        let path = repo_root.join(&file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    Ok(())
}

/// Renders every docs-site file and verifies the on-disk copy matches,
/// returning an error naming the first missing or stale page.
pub fn check_docs_site_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_all(repo_root)? {
        let path = repo_root.join(&file.relative_path);
        let actual = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "generated docs page '{}' is missing ({e}); run \
                 `cargo run -p thetadatadx --features config-file,__internal --bin generate_docs_site` to refresh",
                file.relative_path
            )
        })?;
        let actual_normalized = actual.replace("\r\n", "\n");
        if actual_normalized != file.contents {
            return Err(format!(
                "generated docs page '{}' is stale; run \
                 `cargo run -p thetadatadx --features config-file,__internal --bin generate_docs_site` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}
