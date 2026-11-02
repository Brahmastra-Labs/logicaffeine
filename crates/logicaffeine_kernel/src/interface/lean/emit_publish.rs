//! Publishing an emission's outcome: all or nothing.
//!
//! A document is published only after it has been verified again from its bytes, and only by
//! renaming a fully written, synced staging file over its final name — a reader sees the old
//! file or the new one, never a prefix. A FAILED run publishes no document and leaves no earlier
//! success under its name: the stale `<stem>.lg` is removed and the failure is recorded beside it,
//! separately, in `<stem>.failure.txt`.

use super::{verify, EmittedDocument};
use crate::interface::lean::{Result, Verdict};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The published document's extension.
pub const DOCUMENT_EXTENSION: &str = "lg";
/// The extension of a failed run's diagnostic record.
pub const FAILURE_EXTENSION: &str = "failure.txt";

/// What a publication left in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Published {
    /// The verified document, at this path.
    Document(PathBuf),
    /// No document: the run failed, and its diagnostic is at this path.
    Failure(PathBuf),
}

/// Publish `outcome` under `dir/<stem>.lg`, or its failure under `dir/<stem>.failure.txt`.
pub fn publish(dir: &Path, stem: &str, outcome: &Result<EmittedDocument>) -> io::Result<Published> {
    publish_through(dir, stem, outcome, &mut |sink, bytes| sink.write_all(bytes))
}

/// [`publish`], writing every file's bytes through `write` — the seam a test uses to make a write
/// fail half way.
pub fn publish_through(
    dir: &Path,
    stem: &str,
    outcome: &Result<EmittedDocument>,
    write: &mut dyn FnMut(&mut dyn Write, &[u8]) -> io::Result<()>,
) -> io::Result<Published> {
    let plain = !stem.is_empty()
        && stem != "."
        && stem != ".."
        && !stem.contains(['/', '\\', '\0'])
        && !stem.starts_with('.');
    if !plain {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{stem:?} is not a plain file stem"),
        ));
    }
    let document = dir.join(format!("{stem}.{DOCUMENT_EXTENSION}"));
    let failure = dir.join(format!("{stem}.{FAILURE_EXTENSION}"));

    let verified: std::result::Result<&EmittedDocument, String> = match outcome {
        Ok(doc) => verify(doc)
            .map(|_| doc)
            .map_err(|fault| format!("unsupported_feature: {fault}")),
        Err(verdict) => Err(describe(verdict)),
    };
    match verified {
        Ok(doc) => match write_atomically(dir, &document, doc.source.as_bytes(), write) {
            Ok(()) => {
                remove_if_present(&failure)?;
                Ok(Published::Document(document))
            }
            Err(error) => {
                remove_if_present(&document)?;
                Err(error)
            }
        },
        Err(diagnostic) => {
            remove_if_present(&document)?;
            write_atomically(dir, &failure, diagnostic.as_bytes(), write)?;
            Ok(Published::Failure(failure))
        }
    }
}

fn describe(verdict: &Verdict) -> String {
    let (outcome, reason) = match verdict {
        Verdict::Reject(r) => ("reject", r),
        Verdict::Decline(r) => ("decline", r),
    };
    format!("{outcome} {}: {}\n", reason.code(), reason.message())
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Write `bytes` to a staging file beside `path`, sync it, and rename it over `path`. On any
/// failure the staging file is removed and `path` is untouched by this call.
fn write_atomically(
    dir: &Path,
    path: &Path,
    bytes: &[u8],
    write: &mut dyn FnMut(&mut dyn Write, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document");
    let staging = dir.join(format!(
        ".{name}.staging-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        write(&mut file, bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&staging, path)?;
        if let Ok(handle) = std::fs::File::open(dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    result
}
