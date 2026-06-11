//! `llms.txt` — terse machine-readable index of every docs page.
//!
//! One line per page: `path — description`. Composed from the
//! in-memory generated set plus the hand-written pages on disk, so the
//! `--check` mode also catches a stale index after article edits.

use std::fmt::Write as _;
use std::path::Path;

use super::DocFile;

/// Pull `title` and `description` out of a page's frontmatter. Pages
/// without frontmatter (the changelog mirror is byte-identical to
/// `CHANGELOG.md`, the migration ledgers are append-only) fall back to
/// their first `#` heading as the title.
fn frontmatter(contents: &str) -> (String, String) {
    let mut title = String::new();
    let mut description = String::new();
    let mut in_frontmatter = false;
    for line in contents.lines() {
        if line.trim() == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if let Some(value) = line.strip_prefix("title:") {
            title = value.trim().trim_matches('"').to_string();
        } else if let Some(value) = line.strip_prefix("description:") {
            description = value.trim().trim_matches('"').to_string();
        }
    }
    if title.is_empty() {
        if let Some(heading) = contents
            .lines()
            .find_map(|line| line.strip_prefix("# ").map(str::trim))
        {
            title = heading.to_string();
        }
    }
    (title, description)
}

/// Site URL for a page path relative to `docs-site/docs/`.
fn page_url(rel: &str) -> String {
    let no_ext = rel.trim_end_matches(".md");
    if let Some(stripped) = no_ext.strip_suffix("/index") {
        format!("/{stripped}/")
    } else if no_ext == "index" {
        "/".to_string()
    } else {
        format!("/{no_ext}")
    }
}

fn walk_markdown(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == ".vitepress" || name == "node_modules" || name == "public" {
                continue;
            }
            walk_markdown(&path, out)?;
        } else if name.ends_with(".md") {
            out.push(path);
        }
    }
    Ok(())
}

pub(super) fn render_llms_txt(
    repo_root: &Path,
    generated: &[DocFile],
) -> Result<String, Box<dyn std::error::Error>> {
    let docs_root = repo_root.join("docs-site/docs");

    // (url, title, description), generated pages taking precedence over
    // whatever is on disk at the same path.
    let mut entries: Vec<(String, String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for file in generated {
        let Some(rel) = file.relative_path.strip_prefix("docs-site/docs/") else {
            continue;
        };
        if !rel.ends_with(".md") {
            continue;
        }
        let (title, description) = frontmatter(&file.contents);
        let url = page_url(rel);
        seen.insert(url.clone());
        entries.push((url, title, description));
    }

    let mut disk_pages = Vec::new();
    walk_markdown(&docs_root, &mut disk_pages)?;
    disk_pages.sort();
    for path in disk_pages {
        let rel = path
            .strip_prefix(&docs_root)
            .expect("walked path under docs root")
            .to_string_lossy()
            .replace('\\', "/");
        let url = page_url(&rel);
        if seen.contains(&url) {
            continue;
        }
        // The changelog mirror is a 300 KB release ledger — index the page,
        // read the title from a fixed label rather than frontmatter.
        let contents = std::fs::read_to_string(&path)?;
        let (title, description) = frontmatter(&contents);
        entries.push((url, title, description));
    }

    entries.sort();

    let mut out = String::from(
        "# ThetaDataDx documentation\n\
         # SDK for ThetaData market data: Rust, Python, TypeScript, C++, plus a local HTTP/WebSocket server and an MCP server.\n\
         # One line per page: path — summary. All paths are relative to the docs site root.\n\n",
    );
    for (url, title, description) in entries {
        let label = match (title.is_empty(), description.is_empty()) {
            (false, false) => format!("{title}: {description}"),
            (false, true) => title,
            (true, false) => description,
            (true, true) => String::from("(untitled)"),
        };
        let _ = writeln!(out, "{url} — {label}");
    }
    Ok(out)
}
