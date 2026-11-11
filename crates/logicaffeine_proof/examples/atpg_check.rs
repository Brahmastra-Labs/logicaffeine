//! Generate tests for the single stuck-at faults of AIGER circuits ([`logicaffeine_proof::atpg`]), every verdict
//! replayed: a line per file with the faults, those detected, proven untestable and aborted, the tests, the faults
//! random patterns detected, the solves, whether [`logicaffeine_proof::atpg::Report::check`] passed, and the seconds
//! (the check's apart).
//!
//!     atpg_check [--random-words <n>] [--budget <conflicts>] <file.aig>...

use std::time::Instant;

use logicaffeine_proof::aig::Aig;
use logicaffeine_proof::atpg::{generate, Options};

/// `text`, the value of `flag`, as a number, or the usage error.
fn number(flag: &str, text: &str) -> u64 {
    text.parse().unwrap_or_else(|_| {
        eprintln!("atpg_check: {flag} takes a number");
        std::process::exit(2)
    })
}

fn main() {
    let mut opts = Options::default();
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |flag: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("atpg_check: {flag} takes a value");
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--random-words" => opts.random_words = number("--random-words", &value("--random-words")) as usize,
            "--budget" => opts.budget = number("--budget", &value("--budget")),
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        eprintln!("usage: atpg_check [--random-words <n>] [--budget <conflicts>] <file.aig>...");
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
        let report = generate(&aig, &opts);
        let secs = start.elapsed().as_secs_f64();
        let checking = Instant::now();
        let checked = report.check(&aig);
        let (detected, untestable, aborted) = report.counts();
        println!(
            "{path} faults={} detected={detected} untestable={untestable} aborted={aborted} tests={} by_random={} \
             solves={} checked={checked} seconds={secs:.3} check_seconds={:.3}",
            report.faults.len(),
            report.tests.len(),
            report.by_random,
            report.solves,
            checking.elapsed().as_secs_f64()
        );
    }
}
