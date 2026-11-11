//! Rewrite AIGER circuits by exact synthesis ([`logicaffeine_proof::rewrite`]) and prove each result equal to its
//! input: a line per file with the And nodes before and after, the nodes rewritten, the passes, the NPN classes
//! synthesized and those proven least, and the seconds; `--out <dir>` writes each result there under its file name.
//!
//!     rewrite_check [--rounds <n>] [--cuts <n>] [--out <dir>] <file.aig>...

use std::time::Instant;

use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::rewrite::{rewrite, Options};

fn main() {
    let mut opts = Options::default();
    let (mut out, mut files) = (None, Vec::new());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |flag: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("rewrite_check: {flag} takes a value");
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--rounds" => opts.rounds = value("--rounds").parse().unwrap_or(opts.rounds),
            "--cuts" => opts.cuts = value("--cuts").parse().unwrap_or(opts.cuts),
            "--out" => out = Some(value("--out")),
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        eprintln!("usage: rewrite_check [--rounds <n>] [--cuts <n>] [--out <dir>] <file.aig>...");
        std::process::exit(2);
    }
    for path in &files {
        let read = std::fs::read(path).map_err(|e| e.to_string());
        let aig = match read.and_then(|b| Aig::parse(&b).map_err(|e| format!("{e:?}"))) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let start = Instant::now();
        let report = rewrite(&aig, &opts);
        let secs = start.elapsed().as_secs_f64();
        match report {
            Some(r) => {
                println!(
                    "{path} before={} after={} rewritten={} rounds={} classes={} minimal={} proven seconds={secs:.3}",
                    r.before, r.after, r.rewritten, r.rounds, r.classes, r.minimal
                );
                if let Some(dir) = &out {
                    let name = std::path::Path::new(path).file_name().expect("a file name");
                    if let Err(e) = std::fs::write(std::path::Path::new(dir).join(name), r.aig.to_binary()) {
                        eprintln!("{path}: {e}");
                    }
                }
            }
            None => println!("{path} not proven seconds={secs:.3}"),
        }
    }
}
