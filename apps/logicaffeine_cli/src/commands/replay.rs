//! `largo replay` — re-execute a `.replay` failure artifact and report whether
//! the recorded failure still reproduces byte-identically.
//!
//! The bundle is self-contained (embedded source + seed + schedule trace), so
//! this command works anywhere — no project required. Verdicts:
//!
//! - **reproduced** — the failure fires with byte-identical message and cursor.
//! - **no failure** — the program no longer fails: the recorded bug is fixed
//!   (the green state of a committed regression bundle).
//! - **diverged** — the re-run no longer matches the recording (edited bundle,
//!   schedule drift); with `--verify`, divergence is a hard error.

use std::fs;
use std::path::PathBuf;

use logicaffeine_compile::replay::{ReplayBundle, Verify};

use crate::ui::CliError;

pub(crate) fn cmd_replay(file: PathBuf, verify: bool) -> Result<(), Box<dyn std::error::Error>> {
    let json = fs::read_to_string(&file)
        .map_err(|e| CliError::new(format!("cannot read {}: {e}", file.display())))?;
    let bundle = ReplayBundle::from_json(&json).map_err(CliError::new)?;

    println!(
        "replaying {} (seed {}, {} recorded decisions)",
        bundle.source.entry,
        bundle.schedule.seed,
        bundle.schedule.trace.len()
    );
    println!(
        "recorded failure: {:?} — {}",
        bundle.failure.kind, bundle.failure.message
    );

    match logicaffeine_compile::replay::verify(&bundle) {
        Verify::Reproduced => {
            println!("verdict: reproduced byte-identically");
            Ok(())
        }
        Verify::NoFailure => {
            println!("verdict: the recorded failure no longer fires (fixed)");
            Ok(())
        }
        Verify::Diverged(reason) => {
            println!("verdict: DIVERGED — {reason}");
            if verify {
                Err(CliError::with_hint(
                    format!("replay diverged from the recording: {reason}"),
                    "the program or bundle changed since recording — re-capture the failure",
                )
                .into())
            } else {
                Ok(())
            }
        }
    }
}

/// Write a captured failure bundle under `target/replays/`, returning its path.
/// Called from failure exits (`largo run --interpret` today; the test runner
/// from Wave 3). Best-effort: an unwritable artifact never masks the real error.
pub(crate) fn write_artifact(
    project_root: &std::path::Path,
    bundle: &ReplayBundle,
) -> Option<PathBuf> {
    let dir = project_root.join("target").join("replays");
    fs::create_dir_all(&dir).ok()?;
    let stem = std::path::Path::new(&bundle.source.entry)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("run");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{stem}-{stamp}.replay"));
    fs::write(&path, bundle.to_json()).ok()?;
    Some(path)
}
