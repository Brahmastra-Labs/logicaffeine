//! ★ LOCK — reading a file's bytes can FAIL rather than abort the process. ★
//!
//! `read` (file.lg) answers `Result of Text and Text`: a missing file is a value the program
//! inspects. `readBytes` — the byte-exact path, the one that serves a `.wasm` or a `.png`
//! without a lossy text round trip — had no such sibling. It TRAPS, which on a compiled binary
//! is a `panic!` (`codegen/expr.rs`) and on the interpreter a runtime error.
//!
//! ★ SO THE STDLIB HAD TO WORK AROUND ITS OWN BUILTIN, and wrote down that it was doing so.
//! `static.lg::fileResponse` probes with the recoverable `read` first and only then calls
//! `readBytes`, with a comment explaining why. That costs two real problems on the hot path of
//! every asset a LOGOS site serves:
//!
//!   1. **A RACE.** The file can be removed between the probe and the read — and then the byte
//!      reader aborts the whole server, not the request. A static file server that dies when a
//!      deploy replaces a file underneath it is not a static file server.
//!   2. **TWICE THE WORK.** Every asset is read twice, and the first read fully decodes it as
//!      lossy UTF-8 — a `String` allocation the size of the file — purely to obtain an `Ok`.
//!      For a multi-megabyte `.wasm` that is the largest allocation the request makes, and it
//!      is thrown away.
//!
//! The trapping `readBytes` stays: a program reading its own data wants a trap, exactly as
//! `parseInt` does. What was missing is the total form for a program reading a stranger's URL.

#![allow(unused_imports)]
mod common;
use common::assert_all_tiers;

/// A file that exists reads back byte for byte.
#[test]
fn a_present_file_answers_ok_with_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload.bin");
    // Bytes that are NOT valid UTF-8, which is the whole point of a byte reader.
    std::fs::write(&path, [0u8, 159, 146, 150, 255]).unwrap();
    let p = path.display().to_string();
    let src = format!(
        "## Main\n\
         Inspect tryReadBytes(\"{p}\"):\n\
         \x20   When Ok (b):\n\
         \x20       Show length of b.\n\
         \x20       Show item 2 of b.\n\
         \x20       Show item 5 of b.\n\
         \x20   When Err (why):\n\
         \x20       Show \"failed\".\n"
    );
    assert_all_tiers(&src, "5\n159\n255");
}

/// A file that is not there is an ANSWER, not an abort.
#[test]
fn a_missing_file_answers_err_instead_of_aborting() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("no-such-file.bin").display().to_string();
    let src = format!(
        "## Main\n\
         Inspect tryReadBytes(\"{p}\"):\n\
         \x20   When Ok (b):\n\
         \x20       Show \"read {{length of b}} bytes\".\n\
         \x20   When Err (why):\n\
         \x20       Show \"missing\".\n"
    );
    assert_all_tiers(&src, "missing");
}

/// An empty file is `Ok` with nothing in it — distinct from a missing one.
#[test]
fn an_empty_file_is_ok_and_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.bin");
    std::fs::write(&path, b"").unwrap();
    let p = path.display().to_string();
    let src = format!(
        "## Main\n\
         Inspect tryReadBytes(\"{p}\"):\n\
         \x20   When Ok (b):\n\
         \x20       Show length of b.\n\
         \x20   When Err (why):\n\
         \x20       Show \"missing\".\n"
    );
    assert_all_tiers(&src, "0");
}

/// The trapping reader is unchanged — a program reading its own data still wants the trap.
#[test]
fn the_trapping_reader_still_traps() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("absent.bin").display().to_string();
    let src = format!("## Main\nShow length of readBytes(\"{p}\").\n");
    let out = logicaffeine_compile::compile::tw_outcome(&src);
    assert!(
        out.error.is_some_and(|e| e.contains("readBytes")),
        "`readBytes` keeps its trap, and names itself when it fires"
    );
}
