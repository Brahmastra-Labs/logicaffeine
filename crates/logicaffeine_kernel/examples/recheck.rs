//! Standalone re-checker for LOGOS proof certificates — the De Bruijn criterion.
//!
//! Reads a certificate as JSON (from a file argument or stdin), rebuilds the
//! trusted axiom context from scratch, and re-validates the proof. Prints
//! `VERIFIED` and exits 0 on success; prints `REJECTED: <reason>` and exits 1
//! otherwise.
//!
//! The entire trusted surface of a re-check is this crate — the Calculus of
//! Constructions kernel plus its standard prelude (seven ring axioms + a small
//! type-checker) — and `serde_json` to read the file. There is no parser, no
//! proof-search engine, and no SMT solver here. You do not have to trust how the
//! proof was *found*; you only re-check that it *holds*.
//!
//! Run:
//!   cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json

use std::io::Read;

use logicaffeine_kernel::certificate::{recheck, Certificate};

fn main() {
    let json = match read_input() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("REJECTED: could not read certificate: {}", e);
            std::process::exit(1);
        }
    };

    let cert: Certificate = match serde_json::from_str(&json) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("REJECTED: malformed certificate: {}", e);
            std::process::exit(1);
        }
    };

    match recheck(&cert) {
        Ok(()) => {
            println!("VERIFIED");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("REJECTED: {}", e);
            std::process::exit(1);
        }
    }
}

/// Read the certificate JSON from the first CLI argument (a path) or stdin.
fn read_input() -> std::io::Result<String> {
    if let Some(path) = std::env::args().nth(1) {
        std::fs::read_to_string(path)
    } else {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    }
}
