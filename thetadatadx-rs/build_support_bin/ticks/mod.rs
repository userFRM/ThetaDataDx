//! Checked-in tick projections emitted by `generate_sdk_surfaces`.
//!
//! The C++ layout asserts, flag accessors and Arrow IPC helpers, rendered from
//! the tick table. The Rust, Python and TypeScript tick types expand from the
//! table directly.

mod cpp;
mod idents;
pub(super) mod table;

use std::path::Path;

/// One rendered SDK surface: the repo-relative output path paired with the generated file contents.
pub(super) struct GeneratedSourceFile {
    pub(super) relative_path: &'static str,
    pub(super) contents: String,
}

/// Renders every SDK surface from the table and writes each to its output path under `repo_root`, creating parent directories as needed.
pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files() {
        let path = repo_root.join(file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    Ok(())
}

/// Re-renders every SDK surface and compares it against the checked-in file under `repo_root`, returning an error naming the first stale surface.
pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files() {
        let path = repo_root.join(file.relative_path);
        let actual = std::fs::read_to_string(&path)?;
        if actual.replace("\r\n", "\n") != file.contents {
            return Err(format!(
                "generated SDK surface '{}' is stale; run `cargo run -p thetadatadx-rs --bin generate_sdk_surfaces` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files() -> Vec<GeneratedSourceFile> {
    vec![
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/tick_layout_asserts.hpp.inc",
            contents: cpp::render_cpp_tick_layout_asserts(),
        },
        GeneratedSourceFile {
            // C++ free-function flag-word accessors (`thetadatadx::is_cancelled`,
            // ...). Included from `thetadatadx.hpp`; mirrors the Python
            // computed properties and TypeScript precomputed fields from
            // the same table flags.
            relative_path: "thetadatadx-cpp/include/tick_flag_accessors.hpp.inc",
            contents: cpp::render_cpp_tick_flag_accessors(),
        },
        GeneratedSourceFile {
            // C++ Arrow-IPC terminals on the history result vectors
            // (`thetadatadx::eod_ticks_to_arrow_ipc(...)`). Included from
            // `thetadatadx.hpp`; mirrors `FlatFileRowList::to_arrow_ipc()` and
            // the Python `<TickName>List.to_arrow()` columnar exit.
            relative_path: "thetadatadx-cpp/include/tick_arrow_ipc.hpp.inc",
            contents: cpp::render_cpp_tick_arrow_ipc(),
        },
    ]
}
