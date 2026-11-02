//! Containing a failure the Studio did not expect.
//!
//! The Studio runs the kernel, the English parser and the prover on the page's own thread, so a
//! bug in any of them — a panic, or the engine running out of stack on a deeply nested term —
//! would take the whole page down with it. The web build aborts on panic, and an abort in wasm is
//! a trap: it throws straight out of the module and leaves the page frozen mid-action.
//!
//! [`contain`] runs work behind a boundary that turns such a failure into a [`Fault`] and leaves
//! the page running. In the browser the boundary is a JavaScript `try`/`catch` around a call back
//! into the module: a trap unwinds only the frames inside it. What the trap leaves behind is the
//! module's shadow-stack pointer, still set to the depth where the work died, so the boundary puts
//! it back before any Rust runs again (`build.rs` exports `__stack_pointer` for this). A panic's
//! own message is taken on its way out by a panic hook, because the trap that ends it carries
//! none. Natively a panic unwinds, and the boundary is `catch_unwind`.
//!
//! What a trap cannot give back: the heap the abandoned frames owned stays allocated, and a
//! `RefCell` they held borrowed stays borrowed — a later use of it faults again, and is contained
//! again. And once a panic has been contained, `std::thread::panicking()` answers `true` for the
//! rest of the page's life, because the abort that ended the panic never let std count it done.

use std::cell::{Cell, RefCell};
use std::panic::PanicHookInfo;
use std::sync::Once;

/// Why contained work stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The work panicked.
    Panic,
    /// The work ran out of stack — the engine's own, or the module's shadow stack.
    StackExhausted,
    /// The engine stopped the work for a reason no panic explains.
    Trap,
}

/// A failure [`contain`] caught.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// Why the work stopped.
    pub cause: Cause,
    /// What went wrong, in the words of the panic or of the engine.
    pub message: String,
    /// Where a panic was raised, as `file:line:column`.
    pub location: Option<String>,
}

impl Fault {
    /// The work ran out of stack.
    pub fn stack_exhausted() -> Self {
        Fault { cause: Cause::StackExhausted, message: "ran out of stack space".to_string(), location: None }
    }

    fn panicked(info: &PanicHookInfo<'_>) -> Self {
        Fault {
            cause: Cause::Panic,
            message: payload_text(info.payload()),
            location: info.location().map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column())),
        }
    }

    fn escaped(escape: Escape) -> Self {
        match escape.cause {
            Cause::StackExhausted => Fault::stack_exhausted(),
            Cause::Panic => Fault { cause: Cause::Panic, message: escape.message, location: None },
            Cause::Trap => Fault {
                cause: Cause::Trap,
                message: format!("the engine stopped it ({})", escape.message),
                location: None,
            },
        }
    }
}

/// A Studio action that stopped on a fault: which action, and what stopped it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionFault {
    /// The action as its button names it: `Execute`, `Compile`, `Auto`.
    pub action: &'static str,
    /// What stopped it.
    pub fault: Fault,
}

impl ActionFault {
    /// One line for the toolbar: the action, and the first line of what stopped it.
    pub fn headline(&self) -> String {
        let first = self.fault.message.lines().next().unwrap_or_default();
        format!("{} failed: {first}", self.action)
    }

    /// Everything known about the fault, for the note's tooltip.
    pub fn detail(&self) -> String {
        let mut detail = format!("{} failed unexpectedly: {}", self.action, self.fault.message);
        if let Some(location) = &self.fault.location {
            detail.push_str(&format!("\nat {location}"));
        }
        detail.push_str("\nThe Studio is still running, and the editor holds your text as it was.");
        detail
    }
}

/// Run `work` behind the fault boundary: its value, or the [`Fault`] that stopped it.
pub fn contain<T>(work: impl FnOnce() -> T) -> Result<T, Fault> {
    install_hook();
    let mut work = Some(work);
    let mut value = None;
    let entered = DEPTH.with(|depth| depth.replace(depth.get() + 1));
    let outcome = boundary::run(&mut || value = work.take().map(|work| work()));
    DEPTH.with(|depth| depth.set(entered));
    let caught = CAUGHT.with(|caught| caught.borrow_mut().take());
    match outcome {
        Ok(()) => Ok(value.expect("the boundary returned without running the work")),
        Err(escape) => {
            let fault = caught.unwrap_or_else(|| Fault::escaped(escape));
            boundary::report(&fault);
            Err(fault)
        }
    }
}

/// What the boundary saw leave the work: the thrown message, and what it says the cause was.
struct Escape {
    message: String,
    cause: Cause,
}

thread_local! {
    /// How many boundaries the running code is inside.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
    /// The last panic raised inside a boundary, taken by the hook on its way out.
    static CAUGHT: RefCell<Option<Fault>> = const { RefCell::new(None) };
}

/// Install, once, the hook that takes a contained panic's message. A panic outside every
/// boundary goes on to the hook that was installed before, as it always did.
fn install_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if DEPTH.with(Cell::get) == 0 {
                previous(info);
                return;
            }
            let fault = Fault::panicked(info);
            CAUGHT.with(|caught| {
                if let Ok(mut slot) = caught.try_borrow_mut() {
                    *slot = Some(fault);
                }
            });
        }));
    });
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return text.to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "a panic that carried no message".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
mod boundary {
    use super::{payload_text, Cause, Escape, Fault};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    pub(super) fn run(work: &mut impl FnMut()) -> Result<(), Escape> {
        catch_unwind(AssertUnwindSafe(work))
            .map_err(|payload| Escape { message: payload_text(&*payload), cause: Cause::Panic })
    }

    pub(super) fn report(_: &Fault) {}
}

#[cfg(target_arch = "wasm32")]
mod boundary {
    use super::{Cause, Escape, Fault};
    use wasm_bindgen::closure::ScopedClosure;
    use wasm_bindgen::prelude::*;

    // A trap in `work` throws out of every wasm frame it crossed; the `catch` ends it here, in
    // JavaScript, before control returns to the module. The trap left the shadow-stack pointer
    // where the dead frames had it — below zero when they overran the stack — so it is put back
    // to where it stood when the boundary was entered.
    #[wasm_bindgen(inline_js = r#"
export function contain_call(stackPointer, work) {
    const entered = stackPointer ? stackPointer.value : 0;
    try {
        work();
        return undefined;
    } catch (thrown) {
        let exhausted = thrown instanceof RangeError
            || (typeof InternalError === 'function' && thrown instanceof InternalError);
        if (stackPointer) {
            exhausted = exhausted || stackPointer.value < 0;
            stackPointer.value = entered;
        }
        const message = thrown && typeof thrown.message === 'string' ? thrown.message : String(thrown);
        return [message, exhausted];
    }
}

// Logging a contained fault must never become a fault of its own: some hosts (a test runner, a
// worker) have no console to write to.
export function contain_report(text) {
    const console = globalThis.console;
    if (console && typeof console.warn === 'function') {
        console.warn(text);
    }
}
"#)]
    extern "C" {
        fn contain_call(stack_pointer: &JsValue, work: &ScopedClosure<dyn FnMut()>) -> JsValue;

        fn contain_report(text: &str);
    }

    thread_local! {
        /// The module's exported `__stack_pointer` global (see `build.rs`).
        static STACK_POINTER: JsValue =
            js_sys::Reflect::get(&wasm_bindgen::exports(), &JsValue::from_str("__stack_pointer"))
                .unwrap_or(JsValue::UNDEFINED);
    }

    pub(super) fn run(work: &mut impl FnMut()) -> Result<(), Escape> {
        let stack_pointer = STACK_POINTER.with(JsValue::clone);
        // Lent to JavaScript for this one call; the build aborts on panic, so the closure needs
        // no unwind guard of its own — the boundary is the `catch` above.
        let work = ScopedClosure::<dyn FnMut()>::borrow_mut_aborting(work);
        let thrown = contain_call(&stack_pointer, &work);
        if thrown.is_undefined() {
            return Ok(());
        }
        let thrown = js_sys::Array::from(&thrown);
        let exhausted = thrown.get(1).as_bool().unwrap_or(false);
        Err(Escape {
            message: thrown.get(0).as_string().unwrap_or_default(),
            cause: if exhausted { Cause::StackExhausted } else { Cause::Trap },
        })
    }

    /// The console keeps what the panic hook no longer passes on.
    pub(super) fn report(fault: &Fault) {
        match &fault.location {
            Some(location) => contain_report(&format!("contained a fault: {} (at {location})", fault.message)),
            None => contain_report(&format!("contained a fault: {}", fault.message)),
        }
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/fault/tests.rs"]
mod tests;
