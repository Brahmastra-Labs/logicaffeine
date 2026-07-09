//! Run the small-`n` SAT-space census and print the per-orbit table.
//!
//!   cargo run -p logicaffeine-proof --example sat_census -- [max_n]
//!
//! For each `n` from 1 to `max_n` (default 4) it enumerates every minimal UNSAT formula up to the
//! hyperoctahedral group `Bₙ`, prints one row per orbit (symmetry, proof-complexity rung, router, the
//! audit gap), and a per-`n` summary. Timing is printed so the enumeration cost is visible.

use std::time::Instant;

use logicaffeine_proof::census::{census, coverage_summary, OrbitRecord};

fn main() {
    let max_n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    for n in 1..=max_n {
        let t = Instant::now();
        let orbits = census(n);
        let elapsed = t.elapsed();
        print_section(n, &orbits, elapsed);
    }
    println!("\n══════════════════════════════════════════════════════════════════════════════");
    println!("  HARDNESS SPECTRUM — how the minimal-UNSAT families distribute over the proof ladder");
    println!("══════════════════════════════════════════════════════════════════════════════");
    for n in 1..=max_n {
        let s = coverage_summary(n);
        println!(
            "  n={n}: {} orbits | structured(stab>1)={} rigid={} | max-NS-degree={}",
            s.orbits, s.structured, s.rigid, s.max_ns_degree
        );
        println!("        rungs: {:?}", s.by_rung);
        println!("        res-width: {:?}", s.by_resolution_width);
    }
}

fn print_section(n: usize, orbits: &[OrbitRecord], elapsed: std::time::Duration) {
    println!("\n══════════════════════════════════════════════════════════════════════════════");
    println!("  n = {n}   —   {} minimal-UNSAT orbits   (enumerated in {:?})", orbits.len(), elapsed);
    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(
        "  {:>3} {:>4} {:>6} {:>5} {:>9}  {:<18} {:<10} {:<11} {}",
        "cls", "orb", "stab", "resW", "rule:f/d", "rung", "shadow", "route", "audit"
    );
    let mut gap_count = 0;
    let mut underbroken = 0;
    for r in orbits {
        let mut audit = String::new();
        if r.router_beats_ladder() {
            gap_count += 1;
            audit.push_str("ROUTER>LADDER ");
        }
        if r.symmetry_underbroken() {
            underbroken += 1;
            audit.push_str("SYM-UNBROKEN");
        }
        println!(
            "  {:>3} {:>4} {:>6} {:>5} {:>9}  {:<18} {:<10} {:<11} {}",
            r.num_clauses,
            r.orbit_size,
            r.stabilizer_order,
            r.min_res_width,
            format!("{}/{}", r.full_rule_orbits, r.discovered_rule_orbits),
            format!("{:?}", r.rung),
            format!("{:?}", r.shadow),
            format!("{:?}", r.route),
            audit.trim()
        );
    }
    let by_rung = |pred: &dyn Fn(&OrbitRecord) -> bool| orbits.iter().filter(|r| pred(r)).count();
    println!("  ----");
    println!(
        "  trivial={} counting={} parity={} ns={} beyond={} | affine={} modp={} | router>ladder gaps={}",
        by_rung(&|r| matches!(r.rung, logicaffeine_proof::hypercube::ProofRung::Trivial)),
        by_rung(&|r| matches!(r.rung, logicaffeine_proof::hypercube::ProofRung::Counting)),
        by_rung(&|r| matches!(r.rung, logicaffeine_proof::hypercube::ProofRung::Parity)),
        by_rung(&|r| matches!(r.rung, logicaffeine_proof::hypercube::ProofRung::Nullstellensatz { .. })),
        by_rung(&|r| matches!(r.rung, logicaffeine_proof::hypercube::ProofRung::BeyondBudget)),
        by_rung(&|r| r.affine_explained),
        by_rung(&|r| r.modp_routed),
        gap_count
    );
}
