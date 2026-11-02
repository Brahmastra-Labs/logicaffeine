//! lgvm — print the VM's golden output for a LOGOS `.lg` program (one `Show` value per line), so a
//! hardware run can be validated against the VM reference rather than a hand-computed guess:
//!
//!   cargo run -q -p logicaffeine-compile --example lgvm -- prog.lg
//!
//! This is the SAME `vm_outcome` the whole differential test suite locks the tree-walker and every
//! execution tier against — so matching it on silicon means the chip agrees with the reference semantics.
use logicaffeine_compile::compile::vm_outcome;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: lgvm <file.lg>");
            std::process::exit(2);
        }
    };
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("lgvm: cannot read {path}: {e}");
        std::process::exit(2);
    });
    let out = vm_outcome(&src);
    if let Some(err) = out.error {
        eprintln!("lgvm: VM error: {err}");
        std::process::exit(1);
    }
    print!("{}", out.output.trim());
}
