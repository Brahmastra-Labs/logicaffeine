//! Checking a large export across cores. The whole environment is built once, serially, admitting
//! its leaf declarations (theorems, opaques, axioms — never unfolded by a later check) unchecked;
//! then those leaves are re-checked in forked children, each over the environment shared
//! copy-on-write. Every leaf reads only the frozen environment its predecessors built, so any order
//! of re-checking is a sequential replay of the serial check, and the verdict is identical. Peak
//! memory stays near the serial run's, because the operating system counts the shared pages once.
//!
//! Only the forked workers run across cores; the single-core path ([`super::arena::run`] with
//! `THREADS` unset or 1) is byte-for-byte the serial check, so it defends the single-core and
//! start-up categories. Linux, x86-64 only — every other target checks serially.

use super::arena::STACK_BYTES;
use super::export::{check_export_build_only, Observe};
use super::tc::{Engine, Kernel};
use std::io::{BufReader, Read};
use std::path::PathBuf;

/// A file at or above this size is checked across cores; below it the fork is not worth its cost
/// and the serial path runs (which also keeps the start-up and small-file categories serial).
pub const MIN_BYTES: u64 = 32 << 20;

/// Each forked worker runs its session at this budget (in values), well below the single-core
/// default: the session's peak is floored by the heaviest declaration rather than the budget, so a
/// smaller budget here trims the concurrent sessions' memory at little instruction cost and keeps
/// the parallel run within the arena's memory cap. `LOGOS_SESSION_VALUES` overrides it.
pub const CHILD_SESSION_VALUES: usize = 1 << 20;

/// Declarations a worker claims per turn. Larger amortizes the shared-cursor atomic; smaller
/// balances the load more finely. `LOGOS_PARALLEL_CHUNK` overrides it.
const CHUNK: usize = 64;

/// The worker count from `THREADS` (the Arena harness sets it to the lane's core count), else 1.
pub fn threads() -> usize {
    std::env::var("THREADS").ok().and_then(|s| s.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(1)
}

/// The size gate, in bytes. `LOGOS_PARALLEL_MIN_BYTES` overrides it (a test forces the path on a
/// small fixture with 0).
pub fn min_bytes() -> u64 {
    std::env::var("LOGOS_PARALLEL_MIN_BYTES").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(MIN_BYTES)
}

/// The session budget a forked worker uses: the explicit `LOGOS_SESSION_VALUES`, else the default.
fn child_session_values() -> usize {
    std::env::var("LOGOS_SESSION_VALUES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(CHILD_SESSION_VALUES)
}

fn parallel_chunk() -> usize {
    std::env::var("LOGOS_PARALLEL_CHUNK").ok().and_then(|s| s.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(CHUNK)
}

const SYS_FORK: usize = 57;
const SYS_WAIT4: usize = 61;
const SYS_EXIT: usize = 60;
const SYS_MMAP: usize = 9;

unsafe fn syscall4(n: usize, a: usize, b: usize, c: usize, d: usize) -> isize {
    let r: isize;
    std::arch::asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b,
        in("rdx") c, in("r10") d, lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

unsafe fn syscall6(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> isize {
    let r: isize;
    std::arch::asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b,
        in("rdx") c, in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _,
        options(nostack));
    r
}

/// A counter in a page shared across the forked workers (`MAP_SHARED`): each `claim` returns the
/// next chunk's start by an atomic fetch-add, so workers draw work until the leaves run out and no
/// core idles on a heavy tail. `None` if the page could not be mapped.
struct Cursor {
    at: *const std::sync::atomic::AtomicU32,
}

impl Cursor {
    fn new() -> Option<Cursor> {
        // mmap(NULL, 4096, PROT_READ|PROT_WRITE, MAP_SHARED|MAP_ANONYMOUS, -1, 0)
        let p = unsafe { syscall6(SYS_MMAP, 0, 4096, 0x3, 0x21, usize::MAX, 0) };
        if p < 0 {
            return None;
        }
        let at = p as *const std::sync::atomic::AtomicU32;
        unsafe { (*at).store(0, std::sync::atomic::Ordering::Relaxed) };
        Some(Cursor { at })
    }

    fn claim(&self, chunk: usize) -> usize {
        unsafe { &*self.at }.fetch_add(chunk as u32, std::sync::atomic::Ordering::Relaxed) as usize
    }
}

unsafe fn fork() -> isize {
    syscall4(SYS_FORK, 0, 0, 0, 0)
}

unsafe fn wait4(pid: i32) -> i32 {
    let mut status: i32 = 0;
    syscall4(SYS_WAIT4, pid as usize, &mut status as *mut i32 as usize, 0, 0);
    status
}

unsafe fn exit_now(code: i32) -> ! {
    syscall4(SYS_EXIT, code as usize, 0, 0, 0);
    loop {}
}

/// Build the environment at `path` (standard input for `None` or `-`) and re-check its leaves across
/// `threads` forked workers, then end the process with the Arena protocol's exit code (0 accepted,
/// 1 rejected, 2 declined). The build runs on a thread whose stack is sized for deep recursion, so
/// the workers inherit that stack.
pub fn run(path: Option<PathBuf>, engine: Engine, threads: usize) -> ! {
    let worker = std::thread::Builder::new().name("logos-kernel".into()).stack_size(STACK_BYTES).spawn(move || {
        let mut bytes = None;
        let input: Box<dyn Read> = match &path {
            Some(p) if p.as_os_str() != "-" => match std::fs::File::open(p) {
                Ok(f) => {
                    bytes = f.metadata().ok().map(|m| m.len());
                    Box::new(f)
                }
                Err(e) => {
                    eprintln!("declined: cannot open {}: {e}", p.display());
                    std::process::exit(2);
                }
            },
            _ => Box::new(std::io::stdin()),
        };
        let mut k = Kernel::new();
        k.engine = engine;
        let (built, deferred, stats) =
            check_export_build_only(&mut k, BufReader::with_capacity(1 << 20, input), bytes, Observe::default());
        if let Err(e) = built {
            eprintln!("{e}");
            std::process::exit(if e.is_reject() { 1 } else { 2 });
        }
        let n = deferred.len();
        let t = threads.clamp(1, n.max(1));
        let child_budget = child_session_values();
        let chunk = parallel_chunk();
        // Drop the environment-building session before forking. It holds the values and caches of the
        // in-place checks at the wide single-core budget; left in place, every worker inherits it
        // copy-on-write and privatizes its pages as it re-checks, multiplying it across the workers.
        // A worker builds its own session from empty — only the frozen environment is shared.
        k.drop_session();
        // A cursor in shared memory hands the workers chunks of leaves to re-check; every leaf reads
        // only the frozen environment its predecessors built, so any claim order is a sequential
        // replay of the serial check.
        let Some(cursor) = Cursor::new() else {
            eprintln!("declined: cannot map the shared work cursor");
            std::process::exit(2);
        };
        // Fork t workers; each claims chunks over the copy-on-write environment until the leaves run
        // out, then exits with its verdict. A worker uses the raw exit syscall, running no
        // destructors (it shares the parent's locks by copy only).
        let mut pids: Vec<i32> = Vec::with_capacity(t);
        for _ in 0..t {
            let pid = unsafe { fork() };
            if pid == 0 {
                k.set_session_budget(child_budget);
                loop {
                    let lo = cursor.claim(chunk);
                    if lo >= n {
                        break;
                    }
                    let hi = (lo + chunk).min(n);
                    for d in &deferred[lo..hi] {
                        if let Err(e) = k.recheck_decl(d) {
                            eprintln!("{e}");
                            unsafe { exit_now(if e.is_reject() { 1 } else { 2 }) };
                        }
                    }
                }
                unsafe { exit_now(0) };
            } else if pid < 0 {
                eprintln!("declined: cannot fork a worker");
                std::process::exit(2);
            }
            pids.push(pid as i32);
        }
        // The worst verdict wins: a rejection beats an acceptance, a decline beats both. A worker
        // killed by a signal (out of memory, a crash) is a decline, never a rejection.
        let mut worst = 0i32;
        for pid in pids {
            let status = unsafe { wait4(pid) };
            let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { 2 };
            worst = worst.max(code);
        }
        if worst == 0 {
            eprintln!(
                "accepted: {} declarations, {} records; native primitives: {}",
                stats.declarations,
                stats.records,
                stats.validated_primitives.join(" ")
            );
        }
        std::process::exit(worst)
    });
    match worker {
        Ok(worker) => {
            let _ = worker.join();
            eprintln!("declined: the checking thread ended without a verdict");
        }
        Err(e) => eprintln!("declined: cannot start the checking thread: {e}"),
    }
    std::process::exit(2)
}
