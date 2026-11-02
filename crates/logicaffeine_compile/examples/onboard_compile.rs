//! onboard_compile — the compiler the board's own CPU runs, as a command: LOGOS source → register bytecode → the
//! fabric engine's wire image ([`codegen_sva::onboard`]), the VM's golden proven against the engine model first.
//! `scripts/hw/build-onboard-firmware.sh onboard_compile` builds it for the hart (`riscv64gc-unknown-linux-musl`), and
//! `onboard_compiler_lock` holds that build's every report byte-identical to the host build's.
//!
//!   onboard_compile --program prog.lg (--main | --fn f --args 5,7) [--profile hw|stream|generic] [--caps NAME=N,…]
//!   onboard_compile --library library.txt [--profile …] [--caps …]
//!
//! The report goes to stdout ([`onboard::report`]): `lifter=`, `golden=` or `golden_json=`, `bytecode=` and `wire=`
//! in hex; a refused program prints `error=` and exits 1. `--library` compiles every entry of a library file (its
//! programs named relative to the file), each report under an `entry=` line with the entry's title, and exits 1 if
//! any entry is refused. `--caps` defaults to Board B's engine ([`onboard::hw_caps`]).

use logicaffeine_compile::codegen_sva::engine_build::Profile;
use logicaffeine_compile::codegen_sva::onboard::{self, Run};

fn usage(why: &str) -> ! {
    eprintln!("onboard_compile: {why}");
    eprintln!("usage: onboard_compile --program prog.lg (--main | --fn f --args a,b) [--profile P] [--caps C]");
    std::process::exit(2);
}

fn main() {
    let mut program: Option<String> = None;
    let mut library: Option<String> = None;
    let mut func: Option<String> = None;
    let mut args_csv = String::new();
    let mut main = false;
    let mut profile = Profile::Hw;
    let mut caps = onboard::hw_caps();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--program" => program = it.next(),
            "--library" => library = it.next(),
            "--fn" => func = it.next(),
            "--args" => args_csv = it.next().unwrap_or_default(),
            "--main" => main = true,
            "--profile" => {
                let name = it.next().unwrap_or_default();
                profile = Profile::from_name(&name)
                    .unwrap_or_else(|| usage(&format!("unknown --profile {name:?} (known: hw, stream, generic)")));
            }
            "--caps" => caps = it.next().unwrap_or_default(),
            other => usage(&format!("unknown argument {other}")),
        }
    }
    if let Some(path) = library {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| usage(&format!("cannot read {path}: {e}")));
        let entries = onboard::library(&text).unwrap_or_else(|e| usage(&e));
        let dir = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("."));
        let mut refused = false;
        for entry in entries {
            let file = dir.join(&entry.program);
            let src = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| usage(&format!("cannot read {}: {e}", file.display())));
            let result = onboard::compile(&src, &entry.run, profile, &caps);
            refused |= result.is_err();
            print!("entry={}\n{}", entry.title, onboard::report(&result));
        }
        if refused {
            std::process::exit(1);
        }
        return;
    }
    let Some(path) = program else { usage("--program or --library is required") };
    let run = match (main, func) {
        (true, None) if args_csv.trim().is_empty() => Run::Main,
        (true, _) => usage("--main takes no --fn or --args (Main has no parameters)"),
        (false, Some(func)) => Run::Call {
            func,
            args: args_csv
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(|a| a.parse::<i64>().unwrap_or_else(|_| usage(&format!("bad --args value {a:?}"))))
                .collect(),
        },
        (false, None) => usage("one of --main or --fn is required"),
    };
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| usage(&format!("cannot read {path}: {e}")));
    let result = onboard::compile(&src, &run, profile, &caps);
    print!("{}", onboard::report(&result));
    if result.is_err() {
        std::process::exit(1);
    }
}
