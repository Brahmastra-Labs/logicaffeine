//! `LOGOS_PERF_MAP` — name the JIT's machine code for `perf`.
//!
//! Linux `perf` symbolizes a JIT's anonymous executable memory through `/tmp/perf-<pid>.map`: one
//! `START SIZE name` line per code region, addresses and sizes in hex. When the env var is set (and
//! non-empty, non-`"0"`), the VM appends a line each time native code is installed for a function or
//! a loop region, named after the LOGOS source (`logos::fib [jit]`, `logos::main::loop@42 [jit]`), so a
//! profile of `largo run --interpret` reads in the program's own terms instead of `[unknown]`.
//!
//! `LOGOS_JIT_CODE=<dir>` keeps the machine code itself: each extent's bytes go to `<dir>/<n>.bin`, listed in
//! `<dir>/index.tsv` as `n START SIZE name` (hex START and SIZE, the map's names), for a disassembler.

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

/// Tri-state cache of the env check (0 = unread, 1 = on, 2 = off).
static ENABLED: AtomicU8 = AtomicU8::new(0);

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Whether `LOGOS_PERF_MAP` requests the map (set, non-empty, not `"0"`).
pub fn enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let on = std::env::var("LOGOS_PERF_MAP")
                .map(|v| !v.is_empty() && v != "0")
                .unwrap_or(false);
            ENABLED.store(if on { 1 } else { 2 }, Ordering::Relaxed);
            on
        }
    }
}

/// The map file `perf` reads for this process.
pub fn map_path() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/perf-{}.map", std::process::id()))
}

/// The symbol a natively compiled function is named under.
pub fn function_symbol(name: &str) -> String {
    format!(
        "logos::{} [jit]",
        if name.is_empty() { "<anonymous>" } else { name }
    )
}

/// The symbol a natively compiled loop region is named under: its enclosing function and the pc of
/// its loop head.
pub fn region_symbol(enclosing: &str, head_pc: usize) -> String {
    format!(
        "logos::{}::loop@{head_pc} [jit]",
        if enclosing.is_empty() {
            "main"
        } else {
            enclosing
        }
    )
}

/// One map line. Pure — the unit under test.
pub fn format_line(start: usize, len: usize, symbol: &str) -> String {
    format!("{start:x} {len:x} {symbol}\n")
}

/// Append one line per code extent iff the map is enabled. A write failure disables nothing and
/// never reaches the program: the map is observability only.
pub fn record(extents: &[(usize, usize)], symbol: &str) {
    if !enabled() || extents.is_empty() {
        return;
    }
    let Ok(mut guard) = FILE.lock() else { return };
    if guard.is_none() {
        *guard = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(map_path())
            .ok();
    }
    if let Some(file) = guard.as_mut() {
        let text: String = extents
            .iter()
            .map(|&(start, len)| format_line(start, len, symbol))
            .collect();
        let _ = file.write_all(text.as_bytes());
        let _ = file.flush();
    }
}

static CODE_DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

static CODE_INDEX: Mutex<usize> = Mutex::new(0);

/// The directory `LOGOS_JIT_CODE` names, if any.
fn code_dir() -> Option<&'static std::path::Path> {
    CODE_DIR
        .get_or_init(|| {
            std::env::var_os("LOGOS_JIT_CODE")
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
        })
        .as_deref()
}

/// Whether a caller has anything to record: the perf map or the code dump is on.
pub fn observed() -> bool {
    enabled() || code_dir().is_some()
}

/// Record installed native code under `symbol`: one perf-map line per extent, and its bytes when `LOGOS_JIT_CODE`
/// names a directory. Each extent must be sealed code the native tier owns, readable for its whole length.
pub fn record_code(extents: &[(usize, usize)], symbol: &str) {
    record(extents, symbol);
    let Some(dir) = code_dir() else { return };
    let Ok(mut next) = CODE_INDEX.lock() else {
        return;
    };
    if *next == 0 && std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(mut index) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("index.tsv"))
    else {
        return;
    };
    for &(start, len) in extents {
        // SAFETY: the caller hands over extents of installed, sealed native code (read + execute), which the native
        // tier keeps mapped for the life of the process.
        let bytes = unsafe { std::slice::from_raw_parts(start as *const u8, len) };
        if std::fs::write(dir.join(format!("{}.bin", *next)), bytes).is_ok() {
            let _ =
                index.write_all(format!("{}\t{start:x}\t{len:x}\t{symbol}\n", *next).as_bytes());
        }
        *next += 1;
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/perf_map/tests.rs"]
mod tests;
