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

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/failure/tests.rs"]
mod tests;
