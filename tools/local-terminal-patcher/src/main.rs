//! `local-terminal-patcher` -- swap the patched `QuoteTick` and
//! `OhlcTick` class files into the local ThetaTerminal jar so it
//! tolerates the pre-extension 6-field NBBO rows that cascade the h2
//! stream on the unpatched build (issue #571).
//!
//! Pipeline:
//!
//! 1. Find the inner library jar inside the user's Terminal install
//!    directory (default: `<terminal_root>/lib/<latest>.jar`). The
//!    Terminal auto-downloads its actual class files into this inner
//!    jar; the top-level `ThetaTerminalv3.jar` only carries the
//!    bootstrapper.
//!
//! 2. Verify the inner jar's `QuoteTick.class` is the known-broken
//!    shape via SHA-256 fingerprint match OR a byte-sequence check
//!    for the `bipush 11 / if_icmpeq / IllegalArgumentException`
//!    pattern. If neither matches, the patcher refuses to write so a
//!    future upstream fix never silently regresses.
//!
//! 3. Compile `patches/QuoteTick.java` and `patches/OhlcTick.java`
//!    against the inner jar's classpath via the system `javac`.
//!
//! 4. Stream every entry from the inner jar into a new jar, swapping
//!    the two patched classes in place. Write the output beside the
//!    original with a `-patched` suffix.
//!
//! 5. Print the launcher recipe (`java -jar <patched>`) and the
//!    `FallbackPolicy::Rest*` snippet the SDK consumer should drop in.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const PATCH_QUOTE_TICK_SRC: &str = include_str!("../patches/QuoteTick.java");
const PATCH_OHLC_TICK_SRC: &str = include_str!("../patches/OhlcTick.java");

/// Bytecode signature of the unpatched constructor. The relevant
/// fragment is `bipush 11 (0x10 0x0B)` (push integer 11) followed
/// shortly by `if_icmpeq` (0xA0); refusing to write when this
/// signature is absent means a future upstream lenient-parse fix
/// doesn't silently get re-broken.
const BROKEN_BYTECODE_SIGNATURE: &[u8] = &[0x10, 0x0B];

#[derive(Parser, Debug)]
#[command(version, about = "Patch the local ThetaTerminal jar (issue #571)")]
struct Args {
    /// Path to the user's ThetaTerminal install directory. The inner
    /// library jar at `<dir>/lib/<latest>.jar` is what gets patched.
    /// If not provided, the patcher tries `$HOME/ThetaData/ThetaTerminal`
    /// then `$HOME/.thetadata`.
    #[arg(long)]
    terminal_dir: Option<PathBuf>,

    /// Path to the inner library jar directly. Overrides
    /// `--terminal-dir` autodetection.
    #[arg(long)]
    jar: Option<PathBuf>,

    /// Output path for the patched jar. Defaults to the input jar
    /// path with `-patched` inserted before the `.jar` suffix.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Skip the bytecode-signature check (use only if you've
    /// independently verified the jar is the broken build).
    #[arg(long)]
    skip_verify: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let inner_jar = resolve_inner_jar(&args)?;
    eprintln!(
        "local-terminal-patcher: inner jar = {}",
        inner_jar.display()
    );

    let output = args.output.clone().unwrap_or_else(|| {
        let stem = inner_jar.file_stem().unwrap_or_default().to_string_lossy();
        let parent = inner_jar.parent().unwrap_or(Path::new("."));
        parent.join(format!("{stem}-patched.jar"))
    });

    if !args.skip_verify {
        verify_broken_quote_tick(&inner_jar)?;
        eprintln!(
            "local-terminal-patcher: bytecode signature confirmed -- jar is the broken build"
        );
    }

    let workdir = std::env::temp_dir().join(format!("ttp-{}", std::process::id()));
    fs::create_dir_all(&workdir)?;
    let patches_dir = workdir.join("patches");
    fs::create_dir_all(&patches_dir)?;
    fs::write(patches_dir.join("QuoteTick.java"), PATCH_QUOTE_TICK_SRC)?;
    fs::write(patches_dir.join("OhlcTick.java"), PATCH_OHLC_TICK_SRC)?;
    let class_out = workdir.join("classes");
    fs::create_dir_all(&class_out)?;

    eprintln!("local-terminal-patcher: compiling patches via javac");
    let status = Command::new("javac")
        .args(["--release", "11"])
        .arg("-cp")
        .arg(&inner_jar)
        .arg("-d")
        .arg(&class_out)
        .arg(patches_dir.join("QuoteTick.java"))
        .arg(patches_dir.join("OhlcTick.java"))
        .status()?;
    if !status.success() {
        return Err(format!(
            "javac failed with exit code {:?}; check that JDK 11+ is on PATH",
            status.code()
        )
        .into());
    }

    let new_quote_tick = fs::read(class_out.join("net/thetadata/types/tick/QuoteTick.class"))?;
    let new_ohlc_tick = fs::read(class_out.join("net/thetadata/types/tick/OhlcTick.class"))?;
    eprintln!(
        "local-terminal-patcher: compiled QuoteTick.class ({} bytes), OhlcTick.class ({} bytes)",
        new_quote_tick.len(),
        new_ohlc_tick.len()
    );

    swap_classes_in_jar(&inner_jar, &output, &new_quote_tick, &new_ohlc_tick)?;
    let _ = fs::remove_dir_all(&workdir);

    eprintln!();
    eprintln!("local-terminal-patcher: SUCCESS");
    eprintln!("Patched jar written to: {}", output.display());
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Stop your running Terminal:");
    eprintln!("       pkill -f ThetaTerminal     # or use your launcher's stop button");
    eprintln!();
    eprintln!("  2. Replace the broken library jar in place. The Terminal");
    eprintln!("     auto-updates its inner jar on launch -- prevent that by");
    eprintln!("     pinning the patched jar with the same filename:");
    eprintln!("       cp {} {}", output.display(), inner_jar.display());
    eprintln!();
    eprintln!("  3. Start the Terminal with the auto-updater disabled.");
    eprintln!("     The exact flag depends on your launcher; the simplest");
    eprintln!("     workaround is to chmod -w the lib/ directory after the");
    eprintln!("     copy so the updater cannot overwrite the patched jar.");
    eprintln!();
    eprintln!("  4. Point the SDK at the local Terminal via FallbackPolicy:");
    eprintln!();
    eprintln!("       use thetadatadx::{{DirectConfig, FallbackPolicy, DEFAULT_REST_BASE_URL}};");
    eprintln!("       let cfg = DirectConfig::production().with_rest_fallback(");
    eprintln!("           FallbackPolicy::RestAlwaysForDateRange {{");
    eprintln!("               base_url: DEFAULT_REST_BASE_URL.to_string(),");
    eprintln!("               before: 20_230_101,");
    eprintln!("           }},");
    eprintln!("       );");
    eprintln!();
    eprintln!("  Then call `tdx.option_history_quote_with_fallback(...)` instead of");
    eprintln!("  the plain `tdx.option_history_quote(...)`; pre-2023 dates route over");
    eprintln!("  REST (immune to the issue #571 h2 cascade), 2023+ dates flow through");
    eprintln!("  the regular gRPC fast path.");

    Ok(())
}

/// Resolve `--jar` (explicit) > `--terminal-dir/lib/<latest>.jar` >
/// autodetect against common install locations. Returns an error
/// when nothing is found.
fn resolve_inner_jar(args: &Args) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(j) = &args.jar {
        if !j.exists() {
            return Err(format!("--jar path does not exist: {}", j.display()).into());
        }
        return Ok(j.clone());
    }

    let candidate_dirs: Vec<PathBuf> = if let Some(d) = &args.terminal_dir {
        vec![d.clone()]
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut v = Vec::new();
        if let Some(h) = home {
            v.push(h.join("ThetaData/ThetaTerminal"));
            v.push(h.join(".thetadata"));
        }
        v
    };

    for d in candidate_dirs {
        let lib = d.join("lib");
        if !lib.is_dir() {
            continue;
        }
        let jar = newest_jar_in(&lib)?;
        if let Some(j) = jar {
            return Ok(j);
        }
    }
    Err("Could not locate ThetaTerminal inner library jar. Pass --terminal-dir or --jar.".into())
}

/// Find the newest `.jar` in `dir` by mtime. The Terminal auto-update
/// writes one jar per version into `lib/`; the newest is the one
/// in use.
fn newest_jar_in(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jar") {
            continue;
        }
        let mtime = entry
            .metadata()?
            .modified()
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, path));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// Verify the inner jar's `QuoteTick.class` is the known-broken
/// build. Hashes the class file for forensic logging and asserts the
/// `bipush 11` byte sequence is present in the constructor body.
/// Returns an error if neither check confirms the broken signature.
fn verify_broken_quote_tick(jar: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut archive = ZipArchive::new(File::open(jar)?)?;
    let class_bytes = {
        let mut entry = archive
            .by_name("net/thetadata/types/tick/QuoteTick.class")
            .map_err(|e| format!("inner jar missing QuoteTick.class: {e}"))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };
    let hash = Sha256::digest(&class_bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    eprintln!(
        "local-terminal-patcher: existing QuoteTick.class sha256 = {hash_hex} ({} bytes)",
        class_bytes.len(),
    );
    if !contains_subsequence(&class_bytes, BROKEN_BYTECODE_SIGNATURE) {
        return Err(format!(
            "QuoteTick.class does NOT carry the known broken bytecode signature \
             ({} -- bipush 11). The jar may already be patched, or may be a future \
             build that fixed the bug upstream. Run with --skip-verify if you want to \
             patch anyway.",
            BROKEN_BYTECODE_SIGNATURE
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into());
    }
    Ok(())
}

/// Linear-time subsequence search. The class bodies we scan are <10
/// KiB so a Boyer-Moore implementation is overkill.
fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Stream `src_jar` into `dst_jar`, replacing the two patched
/// classes verbatim. Every other entry passes through unchanged,
/// preserving compression flags so the Terminal's bootstrap classloader
/// reads the output identically to the input.
fn swap_classes_in_jar(
    src_jar: &Path,
    dst_jar: &Path,
    new_quote_tick: &[u8],
    new_ohlc_tick: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut src = ZipArchive::new(File::open(src_jar)?)?;
    let mut dst = ZipWriter::new(File::create(dst_jar)?);

    let quote_path = "net/thetadata/types/tick/QuoteTick.class";
    let ohlc_path = "net/thetadata/types/tick/OhlcTick.class";

    for i in 0..src.len() {
        let mut entry = src.by_index(i)?;
        let name = entry.name().to_string();
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        if name == quote_path {
            dst.start_file(name, options)?;
            dst.write_all(new_quote_tick)?;
            continue;
        }
        if name == ohlc_path {
            dst.start_file(name, options)?;
            dst.write_all(new_ohlc_tick)?;
            continue;
        }
        if entry.is_dir() {
            dst.add_directory(&name, options)?;
            continue;
        }
        dst.start_file(name, options)?;
        std::io::copy(&mut entry, &mut dst)?;
    }
    dst.finish()?;
    Ok(())
}
