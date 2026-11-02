//! `logos-dual`: the LOGOS kernel core with two independently written checkers that must agree,
//! as a Lean Kernel Arena checker.
//!
//! Every declaration is checked twice, by two complete implementations of the judgment: the
//! substitution engine (Lean's algorithm over the hash-consed DAG, crossing binders by
//! substitution) and the value engine (the same judgment by evaluation into closures, types as
//! values). A disagreement is never resolved towards acceptance: the run declines and reports
//! both verdicts.
//!
//! Protocol as `logos-kernel`: a `lean4export` file (path argument, or standard input); exit 0
//! accepted, 1 rejected, 2 declined; the reason on standard error.

use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_sized, Verdict};
use std::io::{BufReader, Read};

const STACK_BYTES: usize = 8 << 30;

fn main() {
    let path = std::env::args().nth(1);
    let worker = std::thread::Builder::new()
        .name("logos-dual".into())
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
            let (verdict, stats) =
                check_export_sized(BufReader::with_capacity(1 << 20, input), bytes, |k| {
                    k.engine = Engine::Dual
                });
            match &verdict {
                Verdict::Accept => eprintln!(
                    "accepted by both engines: {} declarations, {} records",
                    stats.declarations, stats.records
                ),
                Verdict::Reject(e) | Verdict::Decline(e) => eprintln!("{e}"),
            }
            verdict.exit_code()
        })
        .expect("spawn checker thread");
    let code = worker.join().unwrap_or(3);
    std::process::exit(code);
}
