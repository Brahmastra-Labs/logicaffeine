//! Where a failure ends: the program, or the boundary that catches it.
//!
//! A generated program reports a runtime failure in its panic hook and ends with exit status 1, as every tier does.
//! A failure inside a served request, or inside a task the replay scheduler polls, ends there instead: the server
//! answers `500` and keeps serving, the scheduler fails the task and runs its peers. Every place the runtime catches a
//! panic does it through [`caught`], and the hook asks [`is_caught`] before it ends the process.

use std::cell::Cell;
use std::panic::{catch_unwind, UnwindSafe};

thread_local! {
    static CATCHING: Cell<u32> = const { Cell::new(0) };
    static SPECULATING: Cell<u32> = const { Cell::new(0) };
}

/// Run `f`, the word form of an exact region: what it returns, or `None` where it stopped at a value past the word
/// (`logicaffeine_data::ops::raise_overflow`), for the caller to run the region again exactly. Any other failure
/// is not caught here: it ends where it would have.
pub fn speculate<R>(f: impl FnOnce() -> R) -> Option<R> {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            SPECULATING.with(|n| n.set(n.get() - 1));
        }
    }
    let _ = logicaffeine_data::ops::take_overflow();
    SPECULATING.with(|n| n.set(n.get() + 1));
    let leave = Leave;
    let ran = catch_unwind(std::panic::AssertUnwindSafe(f));
    drop(leave);
    match ran {
        Ok(r) => Some(r),
        Err(_) if logicaffeine_data::ops::take_overflow() => None,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Whether a value past the word raised now ends at an enclosing [`speculate`], silently.
fn speculation_catches() -> bool {
    cfg!(panic = "unwind") && SPECULATING.with(|n| n.get() > 0) && logicaffeine_data::ops::overflow_raised()
}

/// Run `f`, catching a panic it raises; while it runs, [`is_caught`] answers true on this thread.
pub fn caught<R>(f: impl FnOnce() -> R + UnwindSafe) -> std::thread::Result<R> {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            CATCHING.with(|n| n.set(n.get() - 1));
        }
    }
    CATCHING.with(|n| n.set(n.get() + 1));
    let _leave = Leave;
    catch_unwind(f)
}

/// Whether a panic raised on this thread now ends at an enclosing [`caught`] rather than ending the program. Never
/// in a build that aborts on panic, where no boundary can catch one.
pub fn is_caught() -> bool {
    cfg!(panic = "unwind") && CATCHING.with(|n| n.get() > 0)
}

/// Report a runtime failure the way every tier does: the output shown so far flushed, then the reason alone on
/// standard error. With `ends_the_program`, a failure no boundary catches ([`is_caught`]) then ends the program with
/// exit status 1; without it the panic unwinds on, for a replay scheduler that fails the task and runs its peers.
/// A generated program installs it first thing in `main`.
pub fn install_hook(ends_the_program: bool) {
    std::panic::set_hook(Box::new(move |info| {
        if speculation_catches() {
            return;
        }
        crate::io::flush_stdout();
        let reason: String = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "LOGOS runtime error".to_string());
        eprintln!("{reason}");
        if ends_the_program && !is_caught() {
            std::process::exit(1);
        }
    }));
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/failure/tests.rs"]
mod tests;
