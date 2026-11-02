//! lgtime — measure the RUN-ONLY host execution time of a LOGOS `.lg` program on the register-bytecode
//! VM, so it can be compared fairly to the FPGA soft-core's on-chip cycle count (which also measures run
//! only — the program is pre-loaded, and the cycle counter counts execution, not parse/compile/stdlib).
//!
//!   cargo run --release -q -p logicaffeine-compile --example lgtime -- prog.lg [--tiers]
//!
//! Compiles ONCE through the chip path (the VM runs the SAME bytecode the soft-core executes), then prints
//! the MIN wall-time in nanoseconds over many runs — VM-on-host vs bytecode-on-silicon, the purest
//! one-instruction-stream comparison. (Full-pipeline timing was dominated by the ~300µs stdlib-prelude
//! load, dwarfing the small programs' actual run.)
//!
//! With `--tiers`, prints `interp_ns vm_ns` — the tree-walking interpreter AND the bytecode VM run-only on
//! the SAME statements, the two host tiers of the cross-tier speed table (interp → VM → silicon).
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tiers = args.iter().any(|a| a == "--tiers");
    let path = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| {
            eprintln!("usage: lgtime <file.lg> [--tiers]");
            std::process::exit(2);
        });
    let src = std::fs::read_to_string(&path).unwrap();
    let out = logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parse");
        let vm = logicaffeine_compile::vm::run_nanos(stmts, interner, Some(types), 3000)
            .expect("chip compile");
        if tiers {
            let interp = logicaffeine_compile::interpreter::interp_nanos(stmts, interner, 3000)
                .expect("interp run");
            format!("{interp} {vm}")
        } else {
            format!("{vm}")
        }
    });
    println!("{out}");
}
