//! `logos-kernel`: the LOGOS kernel core as a Lean 4 checker, the binary `cargo install logicaffeine-kernel` installs,
//! running the check the Lean Kernel Arena entry and `largo lean check` run. It reads one lean4export NDJSON stream (a
//! path, or standard input) and checks it with the value engine, or with `--dual` with both engines, which must agree
//! on every declaration; it exits 0 when the export is accepted, 1 when rejected and 2 when declined, the reason on
//! standard error ([`logicaffeine_kernel::dag::arena`]).

use logicaffeine_kernel::dag::arena;
use logicaffeine_kernel::dag::tc::Engine;

fn main() {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let engine = match args.iter().position(|a| a == "--dual") {
        Some(i) => {
            args.remove(i);
            Engine::Dual
        }
        None => Engine::Sem,
    };
    if args.len() > 1 {
        eprintln!("declined: logos-kernel takes [--dual] and one export (a path, or none for standard input)");
        std::process::exit(arena::DECLINED);
    }
    arena::run(args.pop().map(std::path::PathBuf::from), engine)
}
