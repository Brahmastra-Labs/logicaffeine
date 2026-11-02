//! The Lean Kernel Arena benchmark protocol over `logicaffeine_kernel::interface::lean`:
//! `lean_arena <export.ndjson>` prints `ACCEPT` and exits 0 on acceptance, exits 1 on rejection
//! and 2 on decline.
fn main() {
    let code = logicaffeine_kernel::interface::lean::run_cli(
        std::env::args_os().skip(1),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    if code != 0 {
        std::process::exit(code);
    }
}
