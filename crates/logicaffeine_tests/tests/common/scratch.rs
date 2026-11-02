//! A test process's scratch directory, `<temp>/<prefix>_<pid>`: made empty at its first use in the process and
//! removed when the process exits, whether its tests passed or failed. A process killed by a signal or a timeout runs
//! no exit handler, so the first use of a prefix in a process also removes that prefix's directories whose process
//! no longer runs.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The scratch directories this process made, removed at its exit.
static MADE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// This process's scratch directory under `prefix`. The first use of a prefix removes whatever a dead process of the
/// same pid left under the name, and every directory of the prefix whose process is gone.
pub fn dir(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir();
    let d = root.join(format!("{prefix}_{}", std::process::id()));
    {
        let mut made = MADE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !made.contains(&d) {
            for stale in orphaned(&root, prefix) {
                let _ = std::fs::remove_dir_all(stale);
            }
            let _ = std::fs::remove_dir_all(&d);
            if made.is_empty() {
                // SAFETY: `remove_made` is a plain `extern "C"` function that never unwinds.
                assert_eq!(unsafe { libc::atexit(remove_made) }, 0, "the scratch directory's exit handler was refused");
            }
            made.push(d.clone());
        }
    }
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `root`'s directories named `<prefix>_<pid>`, the pid in decimal digits alone, whose process no longer runs.
pub fn orphaned(root: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let digits = name.to_str()?.strip_prefix(prefix)?.strip_prefix('_')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let pid = i32::try_from(digits.parse::<u64>().ok()?).ok()?;
            (e.file_type().ok()?.is_dir() && !running(pid)).then(|| e.path())
        })
        .collect()
}

/// Whether process `pid` exists: signal 0 asks without delivering anything, and a process of another user refuses
/// the asking rather than not existing.
pub fn running(pid: i32) -> bool {
    if pid <= 0 {
        return true;
    }
    // SAFETY: signal 0 to one positive pid delivers nothing.
    let asked = unsafe { libc::kill(pid, 0) };
    asked == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The exit handler: removes every scratch directory this process made. A thread still holding the list at exit
/// leaves the directories to the next process's sweep instead of deadlocking the exit.
extern "C" fn remove_made() {
    let made = match MADE.try_lock() {
        Ok(made) => made,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    for d in made.iter() {
        let _ = std::fs::remove_dir_all(d);
    }
}
