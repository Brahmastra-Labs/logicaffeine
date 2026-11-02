//! `logos-kernel`: the LOGOS kernel core as a Lean Kernel Arena checker.
//!
//! Reads a `lean4export` NDJSON file (path argument, or standard input) and exits with the Arena
//! protocol: 0 accepted, 1 rejected, 2 declined. The reason for a rejection or a decline is
//! written to standard error.
//!
//! `--conv nbe` (with the `nbe` feature) decides definitional equality with the closure-based
//! engine instead of the substitution engine; `--check sem` checks declaration types and values
//! with the value engine (evaluation into closures, types as values). `--parse-only` reads and interns the export without
//! checking any declaration (a measurement of the reader; it never reports acceptance).
//! `--decl-costs <file>` writes one tab-separated line per declaration — index, user-space
//! instructions the kernel spent on it, kind, admitted, name — read from the CPU's instruction
//! counter (Linux, x86-64; needs perf access and a pin to one core type: `taskset -c 2`).
//! `--trust-before <index>` admits the declarations before that index unchecked (primitives and
//! quotients excepted), to measure declarations deep in an export; such a run always declines.

use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_in, Verdict};
use std::io::{BufReader, Read};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod decl_costs {
    use logicaffeine_kernel::dag::export::DeclMeter;
    use std::io::Write;

    /// A per-thread counter of user-space instructions retired (`perf_event_open`).
    struct Counter {
        fd: i32,
    }

    unsafe fn syscall5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> isize {
        let r: isize;
        std::arch::asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c,
            in("r10") d, in("r8") e, lateout("rcx") _, lateout("r11") _, options(nostack));
        r
    }

    const SYS_READ: usize = 0;
    const SYS_PERF_EVENT_OPEN: usize = 298;
    const PERF_TYPE_HARDWARE: u32 = 0;
    const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
    const EXCLUDE_KERNEL: u64 = 1 << 5;
    const EXCLUDE_HV: u64 = 1 << 6;
    const PERF_FLAG_FD_CLOEXEC: usize = 8;

    impl Counter {
        /// Counts the calling thread. On a hybrid CPU the event is bound to the performance-core
        /// PMU, so the thread must run on a performance core for the count to be whole.
        fn open() -> Result<Counter, String> {
            let pmu = std::fs::read_to_string("/sys/bus/event_source/devices/cpu_core/type")
                .ok()
                .and_then(|t| t.trim().parse::<u64>().ok());
            let mut attr = [0u64; 8];
            attr[0] = PERF_TYPE_HARDWARE as u64 | (64u64 << 32);
            attr[1] = PERF_COUNT_HW_INSTRUCTIONS | pmu.map_or(0, |t| t << 32);
            attr[5] = EXCLUDE_KERNEL | EXCLUDE_HV;
            let fd = unsafe {
                syscall5(
                    SYS_PERF_EVENT_OPEN,
                    attr.as_ptr() as usize,
                    0,
                    usize::MAX,
                    usize::MAX,
                    PERF_FLAG_FD_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "perf_event_open failed (errno {}); run with perf access",
                    -fd
                ));
            }
            Ok(Counter { fd: fd as i32 })
        }

        fn read(&self) -> u64 {
            let mut v = 0u64;
            let n = unsafe {
                syscall5(
                    SYS_READ,
                    self.fd as usize,
                    &mut v as *mut u64 as usize,
                    8,
                    0,
                    0,
                )
            };
            assert_eq!(n, 8, "reading the instruction counter");
            v
        }
    }

    pub struct CostLog {
        counter: Counter,
        out: std::io::BufWriter<std::fs::File>,
        index: u64,
        start: u64,
    }

    impl CostLog {
        pub fn create(path: &str) -> Result<CostLog, String> {
            let counter = Counter::open()?;
            let file =
                std::fs::File::create(path).map_err(|e| format!("cannot create {path}: {e}"))?;
            let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
            writeln!(out, "index\tinstructions\tkind\tadmitted\tname")
                .map_err(|e| e.to_string())?;
            Ok(CostLog {
                counter,
                out,
                index: 0,
                start: 0,
            })
        }
    }

    impl DeclMeter for CostLog {
        fn begin(&mut self) {
            self.start = self.counter.read();
        }

        fn end(&mut self, name: &str, kind: &'static str, admitted: bool) {
            let spent = self.counter.read() - self.start;
            writeln!(
                self.out,
                "{}\t{}\t{}\t{}\t{}",
                self.index, spent, kind, admitted as u8, name
            )
            .expect("writing the declaration costs");
            self.index += 1;
        }
    }
}

/// Deeply nested terms recurse deeply; the checker runs on a thread whose stack is sized for it.
const STACK_BYTES: usize = 8 << 30;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut nbe = false;
    if let Some(i) = args.iter().position(|a| a == "--conv") {
        let engine = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..(i + 2).min(args.len()));
        match engine.as_str() {
            "nbe" => nbe = true,
            "classic" => {}
            other => {
                eprintln!(
                    "declined: unknown conversion engine '{other}' (expected nbe or classic)"
                );
                std::process::exit(2);
            }
        }
    }
    let mut engine = Engine::Classic;
    if let Some(i) = args.iter().position(|a| a == "--check") {
        let name = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..(i + 2).min(args.len()));
        engine = match name.as_str() {
            "sem" => Engine::Sem,
            "classic" => Engine::Classic,
            other => {
                eprintln!("declined: unknown checking engine '{other}' (expected sem or classic)");
                std::process::exit(2);
            }
        };
    }
    let parse_only = match args.iter().position(|a| a == "--parse-only") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let report = match args.iter().position(|a| a == "--stats") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let decl_costs = match args.iter().position(|a| a == "--decl-costs") {
        Some(i) => {
            let Some(out) = args.get(i + 1).cloned() else {
                eprintln!("declined: --decl-costs needs an output file");
                std::process::exit(2);
            };
            args.drain(i..i + 2);
            Some(out)
        }
        None => None,
    };
    let trust_before = match args.iter().position(|a| a == "--trust-before") {
        Some(i) => {
            let Some(n) = args.get(i + 1).and_then(|n| n.parse::<u64>().ok()) else {
                eprintln!("declined: --trust-before needs a declaration index");
                std::process::exit(2);
            };
            args.drain(i..i + 2);
            n
        }
        None => 0,
    };
    let path = args.into_iter().next();
    let worker = std::thread::Builder::new()
        .name("logos-kernel".into())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let mut bytes = None;
            let input: Box<dyn Read> = match &path {
                Some(p) if p != "-" => match std::fs::File::open(p) {
                    Ok(f) => {
                        bytes = f.metadata().ok().map(|m| m.len());
                        Box::new(f)
                    }
                    Err(e) => {
                        eprintln!("declined: cannot open {p}: {e}");
                        return 2;
                    }
                },
                _ => Box::new(std::io::stdin()),
            };
            if parse_only {
                let mut k = logicaffeine_kernel::dag::Kernel::new();
                let mut r = logicaffeine_kernel::dag::export::Reader::new(&mut k);
                r.parse_only = true;
                if let Some(b) = bytes {
                    r.size_hint(b);
                }
                return match r.run(BufReader::with_capacity(1 << 20, input)) {
                    Ok(()) => {
                        eprintln!(
                            "parsed: {} records, {} expressions",
                            r.stats.records,
                            r.k.s.expr_count()
                        );
                        2
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        2
                    }
                };
            }
            let meter: Option<Box<dyn logicaffeine_kernel::dag::export::DeclMeter>> =
                match &decl_costs {
                    None => None,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    Some(out) => match decl_costs::CostLog::create(out) {
                        Ok(log) => Some(Box::new(log)),
                        Err(e) => {
                            eprintln!("declined: {e}");
                            return 2;
                        }
                    },
                    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
                    Some(_) => {
                        eprintln!("declined: --decl-costs needs Linux on x86-64");
                        return 2;
                    }
                };
            let configure = |k: &mut logicaffeine_kernel::dag::Kernel| {
                k.engine = engine;
                #[cfg(feature = "nbe")]
                {
                    k.use_nbe = nbe;
                }
                #[cfg(not(feature = "nbe"))]
                {
                    if nbe {
                        eprintln!(
                            "declined: this build has no NbE engine (build with --features nbe)"
                        );
                        std::process::exit(2);
                    }
                }
            };
            let mut k = logicaffeine_kernel::dag::Kernel::new();
            configure(&mut k);
            let observe = logicaffeine_kernel::dag::export::Observe {
                meter,
                trust_before,
            };
            let (verdict, stats) = check_export_in(
                &mut k,
                BufReader::with_capacity(1 << 20, input),
                bytes,
                observe,
            );
            match &verdict {
                Verdict::Accept => {
                    eprintln!(
                        "accepted: {} declarations, {} records; native primitives: {}",
                        stats.declarations,
                        stats.records,
                        stats.validated_primitives.join(" ")
                    )
                }
                Verdict::Reject(e) | Verdict::Decline(e) => eprintln!("{e}"),
            }
            if report {
                for (what, n) in &stats.memory {
                    eprintln!("  {what}: {n}");
                }
            }
            // The verdict is out: end the process here, so that the kernel's memory goes back to
            // the system whole instead of being freed piece by piece.
            std::process::exit(verdict.exit_code())
        })
        .expect("spawn checker thread");
    let code = worker.join().unwrap_or(3);
    std::process::exit(code);
}
