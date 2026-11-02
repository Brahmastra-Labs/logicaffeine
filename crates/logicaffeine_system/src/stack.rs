//! The stack a compiled program's `Main` runs on.
//!
//! A LOGOS program recurses as deep as its data: a tree of a million nodes is built and walked by recursion. A
//! compiled program's `Main` therefore runs on a stack of [`MAIN_STACK_BYTES`] mapped for it at start, on the thread
//! that started it. Switching stacks rather than starting a thread keeps the process single-threaded, and that is
//! worth having: once a second thread exists, even one only waiting for another to finish, the C library takes its
//! locked, atomic path through every `malloc` and `free`, which doubled the time of an allocation-heavy program.
//!
//! Recursion that runs past the end of the stack reaches an inaccessible guard page. The program then reports
//! [`logicaffeine_data::failure::CALL_DEPTH_EXCEEDED`], the sentence every tier reports for it, after writing out what
//! it had shown, and ends with exit status 1. A native target whose stack the switch does not support runs `Main` on a
//! thread of its own instead, and WebAssembly, which has one thread and a stack its module declares, runs it in place.

/// The room a compiled program's `Main` has for its calls.
pub const MAIN_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Run `main`, a compiled program's `Main`, on a stack of [`MAIN_STACK_BYTES`].
///
/// A failure no boundary catches ends the process in the program's failure hook, so nothing unwinds out of `main`.
/// Generic, so `Main` is compiled into the call that switches to its stack, as it was into a thread's start.
#[inline]
pub fn run_main<F: FnOnce() + Send + 'static>(main: F) {
    switch::run_main(main);
}

/// `main` on a thread of its own with a stack of [`MAIN_STACK_BYTES`]; a failure that reaches the thread's end ends
/// the program with exit status 1.
#[allow(dead_code)]
fn on_a_thread<F: FnOnce() + Send + 'static>(main: F) {
    let outcome = std::thread::Builder::new()
        .stack_size(MAIN_STACK_BYTES)
        .spawn(main)
        .expect("the program's thread starts")
        .join();
    if outcome.is_err() {
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod switch {
    psm::psm_stack_manipulation! {
        yes {
            #[inline]
            pub(super) fn run_main<F: FnOnce() + Send + 'static>(main: F) {
                let Some(stack) = super::mapped::Stack::map(super::MAIN_STACK_BYTES) else {
                    return super::on_a_thread(main);
                };
                super::overflow::watch(stack.guards());
                /// Ends the process if a panic ever unwinds to the switch: unwinding across it is undefined.
                struct NoUnwind;
                impl Drop for NoUnwind {
                    fn drop(&mut self) {
                        std::process::abort();
                    }
                }
                // SAFETY: `stack` maps `size` readable, writable bytes from `base`, between two inaccessible
                // pages, and outlives the call; `NoUnwind` keeps an unwind from crossing the switch.
                unsafe {
                    psm::on_stack(stack.base(), stack.size(), || {
                        let no_unwind = NoUnwind;
                        main();
                        std::mem::forget(no_unwind);
                    });
                }
            }
        }
        no {
            pub(super) fn run_main<F: FnOnce() + Send + 'static>(main: F) {
                super::on_a_thread(main);
            }
        }
    }
}

#[cfg(all(not(unix), not(target_arch = "wasm32")))]
mod switch {
    pub(super) fn run_main<F: FnOnce() + Send + 'static>(main: F) {
        super::on_a_thread(main);
    }
}

#[cfg(target_arch = "wasm32")]
mod switch {
    pub(super) fn run_main<F: FnOnce() + Send + 'static>(main: F) {
        main();
    }
}

#[cfg(unix)]
mod mapped {
    /// An anonymous mapping of `size` readable, writable bytes between two inaccessible guard pages, whichever way
    /// the target's stack grows.
    pub(super) struct Stack {
        mapping: *mut u8,
        total: usize,
        page: usize,
        size: usize,
    }

    impl Stack {
        /// The mapping, or `None` when the system will not map it.
        pub(super) fn map(size: usize) -> Option<Stack> {
            // SAFETY: `sysconf` reads a constant of the system.
            let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            let page = usize::try_from(page).ok().filter(|p| p.is_power_of_two())?;
            let size = size.checked_add(page - 1)? & !(page - 1);
            let total = size.checked_add(2 * page)?;
            #[cfg(target_os = "linux")]
            let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK;
            #[cfg(not(target_os = "linux"))]
            let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
            // SAFETY: a fresh anonymous mapping, inaccessible until the stack between the guards is opened below.
            let mapping = unsafe { libc::mmap(std::ptr::null_mut(), total, libc::PROT_NONE, flags, -1, 0) };
            if mapping == libc::MAP_FAILED {
                return None;
            }
            let stack = Stack {
                mapping: mapping.cast(),
                total,
                page,
                size,
            };
            // SAFETY: the range lies inside the mapping just made.
            let opened = unsafe {
                libc::mprotect(
                    stack.mapping.add(page).cast(),
                    size,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            (opened == 0).then_some(stack)
        }

        /// The lowest address of the usable stack.
        pub(super) fn base(&self) -> *mut u8 {
            // SAFETY: one page into the mapping.
            unsafe { self.mapping.add(self.page) }
        }

        /// The usable stack's length.
        pub(super) fn size(&self) -> usize {
            self.size
        }

        /// The two guard pages, as `[start, end)` address ranges.
        pub(super) fn guards(&self) -> [(usize, usize); 2] {
            let start = self.mapping as usize;
            [
                (start, start + self.page),
                (start + self.page + self.size, start + self.total),
            ]
        }
    }

    impl Drop for Stack {
        fn drop(&mut self) {
            // SAFETY: the mapping this value made, unmapped once.
            unsafe { libc::munmap(self.mapping.cast(), self.total) };
        }
    }
}

/// The report a run past the end of the stack makes. A signal handler, run on an alternate stack, recognizes a fault
/// in a guard page of the stack [`run_main`] mapped; any other fault goes to the handler that was there before.
#[cfg(unix)]
mod overflow {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Once;

    static GUARDS: [(AtomicUsize, AtomicUsize); 2] = [
        (AtomicUsize::new(0), AtomicUsize::new(0)),
        (AtomicUsize::new(0), AtomicUsize::new(0)),
    ];
    static mut PREVIOUS: [std::mem::MaybeUninit<libc::sigaction>; 2] =
        [std::mem::MaybeUninit::uninit(), std::mem::MaybeUninit::uninit()];
    const SIGNALS: [libc::c_int; 2] = [libc::SIGSEGV, libc::SIGBUS];
    /// The alternate stack the handler runs on: room for writing out what the program had shown.
    const SIGNAL_STACK_BYTES: usize = 64 * 1024;

    /// Report a fault inside `guards` as a stack overflow, from now on.
    pub(super) fn watch(guards: [(usize, usize); 2]) {
        for (slot, (start, end)) in GUARDS.iter().zip(guards) {
            slot.0.store(start, Ordering::SeqCst);
            slot.1.store(end, Ordering::SeqCst);
        }
        ensure_signal_stack();
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            for (i, signal) in SIGNALS.into_iter().enumerate() {
                // SAFETY: `PREVIOUS` is written here, once, before the handler that reads it is installed.
                unsafe {
                    let mut action: libc::sigaction = std::mem::zeroed();
                    action.sa_sigaction = on_fault as *const () as usize;
                    action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                    libc::sigemptyset(&mut action.sa_mask);
                    let previous = std::ptr::addr_of_mut!(PREVIOUS[i]).cast::<libc::sigaction>();
                    libc::sigaction(signal, &action, previous);
                }
            }
        });
    }

    /// An alternate signal stack on this thread of at least [`SIGNAL_STACK_BYTES`]: the handler cannot run on the
    /// stack it reports full.
    fn ensure_signal_stack() {
        // SAFETY: reads this thread's alternate stack, then installs a fresh mapping in its place when it is missing
        // or small; the mapping lives as long as the process.
        unsafe {
            let mut current: libc::stack_t = std::mem::zeroed();
            if libc::sigaltstack(std::ptr::null(), &mut current) != 0 {
                return;
            }
            if current.ss_flags & libc::SS_DISABLE == 0 && current.ss_size >= SIGNAL_STACK_BYTES {
                return;
            }
            let Some(stack) = super::mapped::Stack::map(SIGNAL_STACK_BYTES) else {
                return;
            };
            let installed = libc::stack_t {
                ss_sp: stack.base().cast(),
                ss_flags: 0,
                ss_size: stack.size(),
            };
            if libc::sigaltstack(&installed, std::ptr::null_mut()) == 0 {
                std::mem::forget(stack);
            }
        }
    }

    fn in_a_guard(address: usize) -> bool {
        GUARDS.iter().any(|(start, end)| {
            let (start, end) = (start.load(Ordering::Relaxed), end.load(Ordering::Relaxed));
            start <= address && address < end
        })
    }

    extern "C" fn on_fault(signal: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
        // SAFETY: the kernel hands a valid `siginfo_t` to an `SA_SIGINFO` handler.
        let address = unsafe { (*info).si_addr() } as usize;
        if in_a_guard(address) {
            crate::io::flush_stdout_if_free();
            let report = logicaffeine_data::failure::CALL_DEPTH_EXCEEDED;
            // SAFETY: `write` and `_exit` are async-signal-safe.
            unsafe {
                libc::write(2, report.as_ptr().cast(), report.len());
                libc::write(2, b"\n".as_ptr().cast(), 1);
                libc::_exit(1);
            }
        }
        let i = SIGNALS.iter().position(|s| *s == signal).unwrap_or(0);
        // SAFETY: `PREVIOUS[i]` was written before this handler was installed.
        unsafe {
            let previous = &*std::ptr::addr_of!(PREVIOUS[i]).cast::<libc::sigaction>();
            if previous.sa_flags & libc::SA_SIGINFO != 0 && previous.sa_sigaction > libc::SIG_IGN {
                let handler: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(previous.sa_sigaction);
                handler(signal, info, context);
            } else {
                // Put the previous disposition back and return: the faulting instruction runs again under it.
                libc::sigaction(signal, previous, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(all(test, unix))]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/stack/tests.rs"]
mod tests;
