//! The Lean Kernel Arena's protocol over this core, run the one way every surface runs it: the Arena entry (the
//! `logos_kernel` example's run line), `largo lean check` and this crate's `logos-kernel` binary. One lean4export
//! stream, from a path or standard input, checked by an engine; the process exits 0 when it is accepted, 1 when
//! rejected and 2 when declined, with the reason, or the accepted counts, on standard error.

use super::export::{check_export_in, Observe};
use super::tc::{Engine, Kernel};
use super::Verdict;
use std::io::{BufReader, Read};
use std::path::PathBuf;

/// Deeply nested terms recurse deeply: the check runs on a thread whose stack is sized for them.
pub const STACK_BYTES: usize = 8 << 30;

/// The protocol's exit code for an input that could not be checked at all (it is a decline).
pub const DECLINED: i32 = 2;

/// Check the export at `path` (standard input for `None` or `-`) with `engine` and end the process with the
/// protocol's exit code. The process ends on the checking thread once the verdict is written, so the kernel's memory
/// goes back to the system whole instead of being freed piece by piece.
pub fn run(path: Option<PathBuf>, engine: Engine) -> ! {
    // A large export under THREADS > 1 is checked across cores (forked workers over the shared
    // environment); everything else — THREADS unset or 1, a small export, a non-Linux target —
    // falls through to the serial check below, byte-for-byte today's single-core path.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let threads = super::parallel::threads();
        if threads > 1 {
            let bytes = path
                .as_deref()
                .filter(|p| p.as_os_str() != "-")
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);
            if bytes >= super::parallel::min_bytes() {
                super::parallel::run(path, engine, threads);
            }
        }
    }
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
                    std::process::exit(DECLINED);
                }
            },
            _ => Box::new(std::io::stdin()),
        };
        let mut k = Kernel::new();
        k.engine = engine;
        let (verdict, stats) = check_export_in(&mut k, BufReader::with_capacity(1 << 20, input), bytes, Observe::default());
        match &verdict {
            Verdict::Accept => eprintln!(
                "accepted: {} declarations, {} records; native primitives: {}",
                stats.declarations,
                stats.records,
                stats.validated_primitives.join(" ")
            ),
            Verdict::Reject(e) | Verdict::Decline(e) => eprintln!("{e}"),
        }
        std::process::exit(verdict.exit_code())
    });
    match worker {
        Ok(worker) => {
            let _ = worker.join();
            eprintln!("declined: the checking thread ended without a verdict");
        }
        Err(e) => eprintln!("declined: cannot start the checking thread: {e}"),
    }
    std::process::exit(DECLINED)
}

/// [`run`]'s verdict for a caller that goes on: the whole `export`, held in memory, checked by `engine` on a thread of
/// [`STACK_BYTES`], and the kernel torn down before the verdict returns. The Lean import's independent recheck asks the
/// core this way.
pub fn verdict(export: &str, engine: Engine) -> Verdict {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new().name("logos-kernel".into()).stack_size(STACK_BYTES).spawn_scoped(
            scope,
            || {
                let mut k = Kernel::new();
                k.engine = engine;
                check_export_in(&mut k, export.as_bytes(), Some(export.len() as u64), Observe::default()).0
            },
        );
        match worker.map(|w| w.join()) {
            Ok(Ok(verdict)) => verdict,
            Ok(Err(_)) => Verdict::Decline(super::KernelFailure::resource("the core's checking thread panicked".to_string())),
            Err(e) => Verdict::Decline(super::KernelFailure::resource(format!("cannot start the core's checking thread: {e}"))),
        }
    })
}
