//! Standalone re-checker for LOGOS proof certificates and proof packages — the De Bruijn criterion.
//!
//! ```text
//! recheck <certificate.json>     re-check one certificate, dispatched by its schema
//! recheck                        the same, reading the certificate from stdin
//! recheck --package <dir>        replay a Lean → LOGOS proof package from its bytes alone
//! ```
//!
//! A certificate is read by one door (`certificate::read_certificate`): an object with a `schema`
//! is a versioned certificate and must name one this checker implements; otherwise a `theorem`
//! field marks a theorem certificate and its absence a bare one. Unknown fields are refused, and a
//! certificate minted against a retired prelude is refused as incompatible. The trusted context is
//! rebuilt from scratch; nothing the certificate says is registered on its word.
//!
//! A package replay prints its `logos-lean-replay/1` receipt: every stage by name, with its status,
//! code and detail. The process is fresh by construction — it holds no importer state.
//!
//! Exit status: 0 verified (a package: every replay stage passed); 1 rejected; 2 checked by the
//! primary kernel only, or a replay stage unsupported — never reported as verified; 64 usage.
//!
//! The trusted surface of a re-check is the `logicaffeine-kernel` crate and `serde_json`. There is
//! no proof search and no SMT solver here: you do not have to trust how the proof was found.
//!
//! Run:
//!   cargo run -p logicaffeine-kernel --example recheck --features serde -- cert.json
//!   cargo run -p logicaffeine-kernel --example recheck --features serde -- --package <dir>

use std::io::Read;

use logicaffeine_kernel::certificate::{recheck_document, EnvironmentLimits, RecheckOutcome};
use logicaffeine_kernel::interface::lean::package::{self, PackageLimits, MANIFEST};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let code = match arguments.as_slice() {
        [flag, dir] if flag == "--package" => replay_package(std::path::Path::new(dir)),
        [flag, ..] if flag.starts_with("--") => usage(),
        [path] => match std::fs::read_to_string(path) {
            Ok(json) => recheck_certificate(&json),
            Err(e) => {
                eprintln!("REJECTED: could not read certificate: {e}");
                1
            }
        },
        [] => {
            let mut json = String::new();
            match std::io::stdin().read_to_string(&mut json) {
                Ok(_) => recheck_certificate(&json),
                Err(e) => {
                    eprintln!("REJECTED: could not read certificate: {e}");
                    1
                }
            }
        }
        _ => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!("usage: recheck [<certificate.json>] | recheck --package <dir>");
    64
}

fn recheck_certificate(json: &str) -> i32 {
    let report = recheck_document(json, &EnvironmentLimits::default());
    for line in &report.lines {
        match report.outcome {
            RecheckOutcome::Rejected => eprintln!("{line}"),
            _ => println!("{line}"),
        }
    }
    match report.outcome {
        RecheckOutcome::Verified => 0,
        RecheckOutcome::Rejected => 1,
        RecheckOutcome::SingleChecked => 2,
    }
}

fn replay_package(dir: &std::path::Path) -> i32 {
    let files = match package::read_directory(dir, &PackageLimits::default()) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("REJECTED: {e}");
            return 1;
        }
    };
    let receipt = package::replay(&files);
    let manifest_sha256 = files.get(MANIFEST).map(|bytes| package::sha256_hex(bytes));
    let document = package::receipt_document(&receipt, manifest_sha256.as_deref());
    println!(
        "{}",
        serde_json::to_string_pretty(&document).expect("a receipt serializes")
    );
    if receipt.replayed() {
        0
    } else if receipt.first_failure().is_some() {
        1
    } else {
        2
    }
}
