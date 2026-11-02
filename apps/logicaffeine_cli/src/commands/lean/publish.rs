//! Publication and replay — competition rules R9 and R11.
//!
//! A package is written to a staging directory beside `--out`, replayed there by a NEW `largo`
//! process that shares nothing with this one, and renamed into place only when every stage passed.
//! A failed run never leaves a success artifact: its staging directory is removed, and an earlier
//! package under the same name is moved aside to `<name>.superseded-<manifest digest>` — kept, since
//! a failed run destroys nothing that was published under another result's identity, but no longer
//! under the name the failed run was asked to publish.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use logicaffeine_kernel::interface::lean::package::{self, PackageFiles, PACKAGE_SCHEMAS};
use logicaffeine_kernel::interface::lean::pipeline::{Diagnostic, LeanResult, PackageRef};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use logicaffeine_kernel::interface::lean::{DiagnosticCode, Outcome};
use serde_json::Value;

use super::process::{self, Failure};
use super::project::hex;

/// The deepest path a staged package may name.
const MAX_PACKAGE_DEPTH: usize = 16;

/// Whether `dir` holds a package: a `manifest.json` naming a package schema.
fn is_package(dir: &Path) -> bool {
    std::fs::read(dir.join("manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|manifest| {
            manifest["schema"]
                .as_str()
                .is_some_and(|schema| PACKAGE_SCHEMAS.contains(&schema))
        })
}

/// `--out` names where a package is published. Refuse, before any work, a target that is not
/// ours to replace: an existing file, or a directory that is neither empty nor a package.
pub(super) fn check_out(out: &Path) -> Result<(), String> {
    let Some(name) = out.file_name() else {
        return Err(format!("--out {} does not name a directory", out.display()));
    };
    if name.to_string_lossy().starts_with('.') {
        return Err(format!("--out {} must not be a hidden name", out.display()));
    }
    let parent = parent(out);
    if !parent.is_dir() {
        return Err(format!(
            "the parent directory of --out {} does not exist",
            out.display()
        ));
    }
    match std::fs::symlink_metadata(out) {
        Err(_) => Ok(()),
        Ok(meta) if meta.is_dir() => {
            let empty = std::fs::read_dir(out)
                .map(|mut rd| rd.next().is_none())
                .unwrap_or(false);
            if empty || is_package(out) {
                Ok(())
            } else {
                Err(format!(
                    "refusing to publish over {}: it exists and is not a LOGOS Lean package",
                    out.display()
                ))
            }
        }
        Ok(_) => Err(format!(
            "refusing to publish over {}: it exists and is not a directory",
            out.display()
        )),
    }
}

fn parent(out: &Path) -> PathBuf {
    match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

fn sibling(out: &Path, what: &str) -> PathBuf {
    let name = out
        .file_name()
        .expect("checked by check_out")
        .to_string_lossy();
    parent(out).join(format!(".{name}.{what}-{}", unique_suffix()))
}

/// Move an earlier package out from under `out`, keeping it under its own identity.
fn supersede(out: &Path) -> std::io::Result<()> {
    if !out.is_dir() || !is_package(out) {
        return Ok(());
    }
    let manifest = std::fs::read(out.join("manifest.json"))?;
    let digest = hex(&logicaffeine_compile::crypto_accel::sha256(&manifest));
    let name = out
        .file_name()
        .expect("checked by check_out")
        .to_string_lossy();
    let aside = parent(out).join(format!("{name}.superseded-{}", &digest[..16]));
    if aside.exists() {
        // The same manifest is already kept aside: nothing is lost by removing this copy.
        std::fs::remove_dir_all(out)
    } else {
        std::fs::rename(out, aside)
    }
}

/// Every path of a package is a plain relative path: no root, no `..`, no empty component.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    let plain = !path.is_empty()
        && candidate
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && candidate.components().count() <= MAX_PACKAGE_DEPTH;
    plain.then(|| candidate.to_path_buf())
}

/// Write `files` into a fresh staging directory beside `out`, synced to disk.
fn stage(out: &Path, files: &PackageFiles) -> Result<PathBuf, String> {
    let staging = sibling(out, "staging");
    let written = (|| -> Result<(), String> {
        std::fs::create_dir(&staging)
            .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
        for (path, bytes) in files {
            let relative = safe_relative(path)
                .ok_or_else(|| format!("the package names an unsafe path {path:?}"))?;
            let destination = staging.join(relative);
            if let Some(dir) = destination.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            let mut file = std::fs::File::create(&destination)
                .map_err(|e| format!("cannot write {path}: {e}"))?;
            std::io::Write::write_all(&mut file, bytes)
                .map_err(|e| format!("cannot write {path}: {e}"))?;
            file.sync_all()
                .map_err(|e| format!("cannot sync {path}: {e}"))?;
        }
        if let Ok(dir) = std::fs::File::open(&staging) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    match written {
        Ok(()) => Ok(staging),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Rename the staging directory to `out`, replacing an earlier package there.
fn promote(staging: &Path, out: &Path) -> std::io::Result<()> {
    let retired = if out.exists() {
        let retired = sibling(out, "retired");
        std::fs::rename(out, &retired)?;
        Some(retired)
    } else {
        None
    };
    match std::fs::rename(staging, out) {
        Ok(()) => {
            if let Some(retired) = retired {
                let _ = std::fs::remove_dir_all(retired);
            }
            if let Ok(dir) = std::fs::File::open(parent(out)) {
                let _ = dir.sync_all();
            }
            Ok(())
        }
        Err(e) => {
            if let Some(retired) = retired {
                let _ = std::fs::rename(retired, out);
            }
            Err(e)
        }
    }
}

/// Finish a run: stage, replay in a fresh process, and publish only a verified result. Every other
/// path leaves nothing staged and nothing under `out`.
pub(super) fn conclude(
    result: &mut LeanResult,
    files: Option<PackageFiles>,
    out: &Path,
    deadline: Instant,
) {
    if let Some(files) = files {
        publish(result, &files, out, deadline);
    }
    if result.outcome() != Outcome::Verified {
        if let Err(e) = supersede(out) {
            result.fail(
                "fresh_replay",
                Diagnostic::new(
                    "fresh_replay",
                    DiagnosticCode::Infrastructure,
                    format!(
                        "cannot move the earlier package at {} aside: {e}",
                        out.display()
                    ),
                ),
            );
        }
    }
}

fn publish(result: &mut LeanResult, files: &PackageFiles, out: &Path, deadline: Instant) {
    let infrastructure =
        |message: String| Diagnostic::new("fresh_replay", DiagnosticCode::Infrastructure, message);
    let Some(manifest) = files.get("manifest.json") else {
        result.fail(
            "fresh_replay",
            infrastructure("the assembled package has no manifest.json".into()),
        );
        return;
    };
    let manifest_sha256 = hex(&logicaffeine_compile::crypto_accel::sha256(manifest));
    let staging = match stage(out, files) {
        Ok(staging) => staging,
        Err(e) => {
            result.fail(
                "fresh_replay",
                infrastructure(format!("cannot stage the package: {e}")),
            );
            return;
        }
    };
    replay_fresh(result, &staging, deadline);
    if result.outcome() == Outcome::Verified {
        match promote(&staging, out) {
            Ok(()) => {
                result.package = Some(PackageRef {
                    path: out.display().to_string(),
                    manifest_sha256,
                });
                return;
            }
            Err(e) => result.fail(
                "fresh_replay",
                infrastructure(format!("cannot publish at {}: {e}", out.display())),
            ),
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
}

/// Replay the staged package in a new `largo` process: no importer state crosses the boundary.
fn replay_fresh(result: &mut LeanResult, staging: &Path, deadline: Instant) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            result.fail(
                "fresh_replay",
                Diagnostic::new(
                    "fresh_replay",
                    DiagnosticCode::Infrastructure,
                    format!("cannot locate largo: {e}"),
                ),
            );
            return;
        }
    };
    let mut command = Command::new(exe);
    command.args(["lean", "replay"]).arg(staging);
    let ran = match process::run(command, deadline, 64 << 20) {
        Ok(ran) => ran,
        Err(Failure::Deadline) => {
            result.fail(
                "fresh_replay",
                Diagnostic::new(
                    "fresh_replay",
                    DiagnosticCode::ResourceLimit,
                    "the deadline passed during the fresh-process replay",
                ),
            );
            return;
        }
        Err(Failure::Spawn(e)) => {
            result.fail(
                "fresh_replay",
                Diagnostic::new(
                    "fresh_replay",
                    DiagnosticCode::Infrastructure,
                    format!("cannot start the fresh-process replay: {e}"),
                ),
            );
            return;
        }
    };
    let receipt: Option<Value> = serde_json::from_slice(&ran.stdout).ok();
    let detail = receipt
        .as_ref()
        .and_then(|r| r["stages"].as_object().cloned())
        .map(|stages| {
            stages
                .iter()
                .filter(|(_, s)| s["status"] != "passed")
                .map(|(name, s)| {
                    format!(
                        "{name} {}: {}",
                        s["status"].as_str().unwrap_or("?"),
                        s["detail"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&ran.stderr).trim().to_string());
    match ran.status.code() {
        Some(0) => result.pass(
            "fresh_replay",
            "the published package replayed in a fresh process",
        ),
        Some(1) => result.fail(
            "fresh_replay",
            Diagnostic::new(
                "fresh_replay",
                DiagnosticCode::InternalInconsistency,
                format!("the fresh-process replay refused LOGOS's own package: {detail}"),
            ),
        ),
        Some(2) => result.mark(
            "fresh_replay",
            StageStatus::Unsupported,
            Diagnostic::new(
                "fresh_replay",
                DiagnosticCode::UnsupportedFeature,
                format!("the fresh-process replay cannot complete: {detail}"),
            ),
        ),
        Some(4) => result.fail(
            "fresh_replay",
            Diagnostic::new(
                "fresh_replay",
                DiagnosticCode::ResourceLimit,
                format!("the fresh-process replay exceeded its limits: {detail}"),
            ),
        ),
        other => result.fail(
            "fresh_replay",
            Diagnostic::new(
                "fresh_replay",
                DiagnosticCode::Infrastructure,
                format!("the fresh-process replay failed ({other:?}): {detail}"),
            ),
        ),
    }
}

/// `largo lean replay <dir>`: the `logos-lean-replay/1` receipt and the exit code — 0 replayed ·
/// 1 a stage failed · 2 a stage could not run · 5 the directory could not be read within the package
/// limits (nothing was replayed).
pub(super) fn replay_directory(dir: &Path) -> (Value, i32) {
    match package::read_directory(dir, &package::PackageLimits::default()) {
        Ok(files) => {
            let receipt = package::replay(&files);
            let manifest = files.get(package::MANIFEST).map(|m| package::sha256_hex(m));
            let code = if receipt.replayed() {
                0
            } else if receipt.first_failure().is_some() {
                1
            } else {
                2
            };
            (
                package::receipt_document(&receipt, manifest.as_deref()),
                code,
            )
        }
        Err(detail) => {
            let mut receipt = Receipt::unstarted();
            receipt.set_with_code(
                "fresh_replay",
                StageStatus::Failed,
                DiagnosticCode::Infrastructure.as_str(),
                format!("{}: {detail}", dir.display()),
            );
            (package::receipt_document(&receipt, None), 5)
        }
    }
}
