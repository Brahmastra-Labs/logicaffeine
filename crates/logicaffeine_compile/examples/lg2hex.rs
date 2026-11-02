//! lg2hex — compile a LOGOS `.lg` program to soft_core_scalar instruction words (`.hex`) so it can be
//! streamed to the Nexys A7 FPGA and executed on real silicon:
//!
//!   cargo run -q -p logicaffeine-compile --example lg2hex -- prog.lg > prog.hex
//!   sudo ./scripts/run-on-nexys-scalar.sh prog.hex
//!
//! Only the function-free scalar subset is chip-runnable; anything else exits non-zero with the reason.
use logicaffeine_compile::codegen_sva::full_asm;
use logicaffeine_compile::codegen_sva::scalar_asm::{compile_to_scalar_words, to_hex};

fn main() {
    // `lg2hex prog.lg`         → scalar core (function-free subset)
    // `lg2hex --full prog.lg`  → WIDE full core (functions, div/mod, collections — the benchmark subset)
    let mut full = false;
    let mut path = None;
    for a in std::env::args().skip(1) {
        if a == "--full" {
            full = true;
        } else {
            path = Some(a);
        }
    }
    let path = path.unwrap_or_else(|| {
        eprintln!("usage: lg2hex [--full] <file.lg>   (writes soft-core .hex to stdout)");
        std::process::exit(2);
    });
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("lg2hex: cannot read {path}: {e}");
        std::process::exit(2);
    });
    let result = if full {
        full_asm::compile_to_full_words(&src)
    } else {
        compile_to_scalar_words(&src)
    };
    match result {
        Ok(words) => {
            eprintln!(
                "lg2hex: {} instruction words ({} core)",
                words.len(),
                if full { "full" } else { "scalar" }
            );
            print!("{}", to_hex(&words));
        }
        Err(e) => {
            eprintln!(
                "lg2hex: {path} is not runnable on the {} chip: {e}",
                if full { "full" } else { "scalar" }
            );
            std::process::exit(1);
        }
    }
}
