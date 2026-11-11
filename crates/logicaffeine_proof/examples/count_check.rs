//! Count the models of DIMACS CNF files exactly ([`logicaffeine_proof::count`]): a line per file with the count, the
//! variables preprocessing settled, the decisions, the components counted, the cache hits, the conflicts, and the
//! seconds.
//!
//!     count_check <file.cnf>...

use std::time::Instant;

use logicaffeine_proof::count::count_models;
use logicaffeine_proof::dimacs;

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: count_check <file.cnf>...");
        std::process::exit(2);
    }
    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let cnf = match dimacs::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{path}: {e:?}");
                continue;
            }
        };
        let start = Instant::now();
        let c = count_models(cnf.num_vars, &cnf.clauses);
        println!(
            "{path} models={} settled={} decisions={} components={} cache_hits={} conflicts={} seconds={:.3}",
            c.models,
            c.settled,
            c.decisions,
            c.components,
            c.cache_hits,
            c.conflicts,
            start.elapsed().as_secs_f64()
        );
    }
}
