//! Head-to-head SAT harness — our certified symmetry/SDCL solver vs the field (SaDiCaL, Kissat),
//! with proof export for independent verification (drat-trim / lrat-check / dpr-trim).
//!
//! Subcommands (all instances generated from the SAME `families::php`, so every engine attacks a
//! byte-identical formula):
//!
//! - `gen-php <n>`            — print PHP(n) DIMACS to stdout (feed to any external solver).
//! - `ours-php <n> [stem]`    — our short PR/SR refutation of PHP(n): timing, self-check, and
//!                              emit DRAT+LRAT+DPR proofs to `<stem>.{drat,lrat,dpr}` (default
//!                              stem `/tmp/logos_php<n>`).
//! - `solve <file>`           — plain certified CDCL on a DIMACS file: verdict + timing.
//!
//! Timings are wall-clock for the certified solve INCLUDING building a re-checkable proof — the
//! honest number, since certification is the whole point.

use std::time::Instant;

use logicaffeine_proof::cdcl::{SolveResult, Solver};
use logicaffeine_proof::dimacs::{self, DimacsCnf};
use logicaffeine_proof::families;
use logicaffeine_proof::proof_emit;
use logicaffeine_proof::sym_certify;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen-php") => {
            let n: usize = args[2].parse().expect("usage: gen-php <n>");
            let (cnf, _) = families::php(n);
            print!("{}", dimacs::print(&cnf));
        }
        Some("ours-php") => {
            let n: usize = args[2].parse().expect("usage: ours-php <n> [stem]");
            let stem = args.get(3).cloned().unwrap_or_else(|| format!("/tmp/logos_php{n}"));
            ours_php(n, &stem);
        }
        Some("solve") => {
            let path = args.get(2).expect("usage: solve <file>");
            let text = std::fs::read_to_string(path).expect("read DIMACS");
            let cnf = dimacs::parse(&text).expect("parse DIMACS");
            solve(&cnf);
        }
        Some("prof-php") => {
            let n: usize = args[2].parse().expect("usage: prof-php <n>");
            prof_php(n);
        }
        Some("sel-php") => {
            let n: usize = args[2].parse().expect("usage: sel-php <n>");
            sel_php(n);
        }
        Some("discover-php") => {
            // The HONEST engine: handed only opaque clauses (no n, no layout, no symmetry), it
            // must DISCOVER the structure and refute — the fair "fed CNF" comparison.
            let n: usize = args[2].parse().expect("usage: discover-php <n>");
            discover(&format!("PHP({n})"), families::php(n).0);
        }
        Some("discover-clique") => {
            let n: usize = args[2].parse().expect("usage: discover-clique <n> <k>");
            let k: usize = args[3].parse().expect("usage: discover-clique <n> <k>");
            discover(&format!("clique({n},{k})"), families::clique_coloring(n, k).0);
        }
        Some("lyapunov-php") => {
            let n: usize = args[2].parse().expect("usage: lyapunov-php <n>");
            show_lyapunov(&format!("PHP({n})"), families::php(n).0);
        }
        Some("lyapunov-unified") => {
            let n: usize = args[2].parse().expect("usage: lyapunov-unified <n>");
            show_unified_lyapunov(n);
        }
        Some("auto-collapse") => {
            // THE unified agent: read any DIMACS, recognize which physics collapses it, report.
            let path = args.get(2).expect("usage: auto-collapse <file>");
            let text = std::fs::read_to_string(path).expect("read DIMACS");
            let cnf = dimacs::parse(&text).expect("parse DIMACS");
            run_auto_collapse(path, &cnf);
        }
        Some("discover-file") => {
            // Fully airtight fair fight: read the SAME DIMACS file the external solvers get, and
            // discover + refute from those opaque clauses alone.
            let path = args.get(2).expect("usage: discover-file <file>");
            let text = std::fs::read_to_string(path).expect("read DIMACS");
            let cnf = dimacs::parse(&text).expect("parse DIMACS");
            discover(path, cnf);
        }
        Some("sdcl") => {
            // The general SDCL solver (positive reduct), also fed only opaque clauses.
            let path = args.get(2).expect("usage: sdcl <file>");
            let text = std::fs::read_to_string(path).expect("read DIMACS");
            let cnf = dimacs::parse(&text).expect("parse DIMACS");
            run_sdcl(&cnf);
        }
        Some("cert-php") => {
            let n: usize = args[2].parse().expect("usage: cert-php <n>");
            use logicaffeine_proof::sym_certify::heule_php_ranked;
            let (cnf, _) = families::php(n);
            let ranked = heule_php_ranked(n);
            match ranked.certify(cnf.num_vars, &cnf.clauses) {
                Some(b) => println!(
                    "PHP({n}): CORRECT + certified size ≤ {} (={} levels × {} width); actual {} steps; n²={}",
                    b.bound, b.levels, b.max_width, b.actual, n * n
                ),
                None => println!("PHP({n}): certificate failed"),
            }
        }
        Some("gen-clique") => {
            let n: usize = args[2].parse().expect("usage: gen-clique <n> <k>");
            let k: usize = args[3].parse().expect("usage: gen-clique <n> <k>");
            let (cnf, _) = families::clique_coloring(n, k);
            print!("{}", dimacs::print(&cnf));
        }
        Some("ours-clique") => {
            let n: usize = args[2].parse().expect("usage: ours-clique <n> <k>");
            let k: usize = args[3].parse().expect("usage: ours-clique <n> <k>");
            ours_clique(n, k);
        }
        Some("steer-clique") => {
            let n: usize = args[2].parse().expect("usage: steer-clique <n> <k>");
            let k: usize = args[3].parse().expect("usage: steer-clique <n> <k>");
            steer_clique(n, k);
        }
        Some("gen-tseitin") => {
            let n: usize = args[2].parse().expect("usage: gen-tseitin <n> <seed>");
            let seed: u64 = args[3].parse().expect("usage: gen-tseitin <n> <seed>");
            let (_, cnf) = tseitin_expander(n, seed);
            print!("{}", dimacs::print(&cnf));
        }
        Some("ours-tseitin") => {
            let n: usize = args[2].parse().expect("usage: ours-tseitin <n> <seed>");
            let seed: u64 = args[3].parse().expect("usage: ours-tseitin <n> <seed>");
            ours_tseitin(n, seed);
        }
        Some("gen-modp") => {
            // mod-p counting obstruction (Count_p / mod-p Tseitin): UNSAT, and resolution — so
            // Kissat/CaDiCaL AND even Z3 — needs 2^Ω(n). Our GF(p) Gaussian route refutes it in µs.
            let n: usize = args[2].parse().expect("usage: gen-modp <n> <p> <seed>");
            let p: u64 = args[3].parse().expect("usage: gen-modp <n> <p> <seed>");
            let seed: u64 = args[4].parse().expect("usage: gen-modp <n> <p> <seed>");
            let (_, cnf, _) = families::mod_p_tseitin_expander(n, p, seed);
            print!("{}", dimacs::print(&cnf));
        }
        Some("gen-chessboard") => {
            // Mutilated chessboard: UNSAT by a bipartite-matching (Hall) parity argument, exponential
            // for resolution; our covering route certifies it in polynomial time.
            let n: usize = args[2].parse().expect("usage: gen-chessboard <even n>");
            let (cnf, _) = families::mutilated_chessboard(n);
            print!("{}", dimacs::print(&cnf));
        }
        Some("gen-ordering") => {
            // Linear ordering principle GT(n): UNSAT, a canonical resolution-hard family.
            let n: usize = args[2].parse().expect("usage: gen-ordering <n>");
            let (cnf, _) = families::ordering_principle(n);
            print!("{}", dimacs::print(&cnf));
        }
        Some("gen-modp-sat") => {
            // Satisfiable control over the same graph/encoding (charges sum to 0 mod p): our GF(p)
            // route returns a model, the field must search the one-hot CNF.
            let n: usize = args[2].parse().expect("usage: gen-modp-sat <n> <p> <seed>");
            let p: u64 = args[3].parse().expect("usage: gen-modp-sat <n> <p> <seed>");
            let seed: u64 = args[4].parse().expect("usage: gen-modp-sat <n> <p> <seed>");
            let (_, cnf, _) = families::mod_p_consistent_onehot(n, p, seed);
            print!("{}", dimacs::print(&cnf));
        }
        Some("gen-weakphp") => {
            // Weak pigeonhole PHP^h_p (any hole count) — UNSAT iff p > h, resolution-exponential.
            let p: usize = args[2].parse().expect("usage: gen-weakphp <pigeons> <holes>");
            let h: usize = args[3].parse().expect("usage: gen-weakphp <pigeons> <holes>");
            print!("{}", dimacs::print(&families::weak_php(p, h).0));
        }
        Some("gen-fphp") => {
            // Functional pigeonhole FPHP(n): PHP + "≤ 1 hole per pigeon" — still resolution-exponential.
            let n: usize = args[2].parse().expect("usage: gen-fphp <n>");
            print!("{}", dimacs::print(&families::functional_php(n).0));
        }
        Some("gen-ontophp") => {
            // Onto (bijective) pigeonhole: FPHP + "every hole filled" — the maximal PHP clause set.
            let n: usize = args[2].parse().expect("usage: gen-ontophp <n>");
            print!("{}", dimacs::print(&families::onto_php(n).0));
        }
        Some("gen-modcount") => {
            // The modular counting principle Count_q(n): exact q-cover of [n], UNSAT iff q ∤ n. q=2 is
            // perfect matching on K_n (UNSAT for odd n) — resolution-exponential.
            let n: usize = args[2].parse().expect("usage: gen-modcount <n> <q>");
            let q: usize = args[3].parse().expect("usage: gen-modcount <n> <q>");
            print!("{}", dimacs::print(&families::mod_counting(n, q).0));
        }
        Some("gen-ramsey") => {
            // Ramsey(s,t;n): 2-colour K_n avoiding red K_s / blue K_t — UNSAT iff n ≥ R(s,t).
            let s: usize = args[2].parse().expect("usage: gen-ramsey <s> <t> <n>");
            let t: usize = args[3].parse().expect("usage: gen-ramsey <s> <t> <n>");
            let n: usize = args[4].parse().expect("usage: gen-ramsey <s> <t> <n>");
            print!("{}", dimacs::print(&families::ramsey(s, t, n).0));
        }
        Some("gen-kxor") => {
            // Random k-XOR (parity): UNSAT, exponential for resolution, crushed by GF(2) Gaussian in µs.
            let k: usize = args[2].parse().expect("usage: gen-kxor <k> <n> <m> <seed>");
            let n: usize = args[3].parse().expect("usage: gen-kxor <k> <n> <m> <seed>");
            let m: usize = args[4].parse().expect("usage: gen-kxor <k> <n> <m> <seed>");
            let seed: u64 = args[5].parse().expect("usage: gen-kxor <k> <n> <m> <seed>");
            print!("{}", dimacs::print(&families::random_kxor(k, n, m, seed).1));
        }
        Some("gen-pebbling") => {
            // Pebbling contradiction on the pyramid DAG of the given height — the resolution-space family.
            let h: usize = args[2].parse().expect("usage: gen-pebbling <height>");
            print!("{}", dimacs::print(&families::pebbling_pyramid(h).0));
        }
        Some("route") => {
            // The full certified DISPATCHER on an opaque DIMACS file: it discovers which specialist
            // (counting / parity / mod-p / Horn / …) collapses the formula — the honest "ours" number
            // for any family, handed only the clauses with no layout or hint.
            let path = args.get(2).expect("usage: route <file>");
            let text = std::fs::read_to_string(path).expect("read DIMACS");
            let cnf = dimacs::parse(&text).expect("parse DIMACS");
            route_solve(path, &cnf);
        }
        _ => {
            eprintln!("usage: satbench <gen-php|ours-php|solve|route|gen-{{weakphp,fphp,ontophp,modcount,ramsey,kxor,pebbling}}> ...");
            std::process::exit(2);
        }
    }
}

/// The full certified dispatcher on an opaque DIMACS instance: discovers the collapsing structure and
/// reports verdict + the route taken + wall-clock. This is the fair "ours" measurement for an arbitrary
/// family — it is fed only the clauses (no `n`, no layout, no symmetry hint), exactly like the field.
fn route_solve(path: &str, cnf: &DimacsCnf) {
    use logicaffeine_proof::solve::{solve_structured, Answer};
    let t = Instant::now();
    let solved = solve_structured(cnf.num_vars, &cnf.clauses);
    let dt = t.elapsed();
    let verdict = match solved.answer {
        Answer::Unsat => "UNSAT",
        Answer::Sat(_) => "SAT",
    };
    println!(
        "ours  {path}  vars={} clauses={}  {verdict}  via={:?}  solve={dt:?}",
        cnf.num_vars,
        cnf.clauses.len(),
        solved.via
    );
}

/// Our certified short refutation of PHP(n): symmetry/SR proof, timed, self-checked, exported.
fn ours_php(n: usize, stem: &str) {
    let (cnf, _) = families::php(n);
    let nv = cnf.num_vars;

    let t = Instant::now();
    let cr = sym_certify::heule_php_refutation(n);
    let solve_dt = t.elapsed();

    println!(
        "ours  PHP({n})  vars={nv} clauses={}  refuted={}  sbp(PR/SR)={} steps={}  solve+certify={:?}",
        cnf.clauses.len(),
        cr.refuted,
        cr.sbp_clauses,
        cr.steps.len(),
        solve_dt
    );

    std::fs::write(format!("{stem}.cnf"), dimacs::print(&cnf)).ok();

    // The short symmetry proof in DPR where it is genuinely PR (honest when irreducibly SR).
    match proof_emit::emit_dpr(nv, &cnf.clauses, &cr.steps) {
        Ok(dpr) => {
            std::fs::write(format!("{stem}.dpr"), &dpr).ok();
            println!("  wrote {stem}.dpr ({} bytes) — fully PR, dpr-trim-checkable", dpr.len());
        }
        Err(e) => println!("  short SR proof not standard-DPR-expressible ({e:?}) — uses SR power"),
    }

    // The universally-checkable RUP export is the exponential resolution path; only feasible for
    // tiny n, and offered purely as external-checker cross-validation, never as the timing.
    if n <= 6 {
        let rup_steps = logicaffeine_proof::sdcl::plain_cdcl_refutation(nv, &cnf.clauses);
        if let Ok(drat) = proof_emit::emit_drat(nv, &cnf.clauses, &rup_steps) {
            std::fs::write(format!("{stem}.drat"), &drat).ok();
        }
        if let Ok(lrat) = proof_emit::emit_lrat(nv, &cnf.clauses, &rup_steps) {
            std::fs::write(format!("{stem}.lrat"), &lrat).ok();
        }
        println!("  (small-n) wrote RUP DRAT/LRAT cross-check proofs");
    }
}

/// Replicate the swap-pigeons automorphism PHP(n) uses.
fn swap_pigeons(holes: usize, npigeons: usize, i: usize, j: usize) -> logicaffeine_proof::proof::Perm {
    use logicaffeine_proof::cdcl::Lit;
    let images: Vec<Lit> = (0..npigeons * holes)
        .map(|v| {
            let (p, h) = (v / holes, v % holes);
            let np = if p == i {
                j
            } else if p == j {
                i
            } else {
                p
            };
            Lit::pos((np * holes + h) as u32)
        })
        .collect();
    logicaffeine_proof::proof::Perm::from_images(images)
}

/// Profile the PHP(n) certified refutation on the REAL indexed path: split the per-step cost into
/// the incremental automorphism re-verification vs the propagation-to-conflict, to find the next
/// lever.
fn prof_php(n: usize) {
    use logicaffeine_proof::cdcl::Lit;
    use logicaffeine_proof::proof::Witness;
    use logicaffeine_proof::symmetry_detect::AutomorphismIndex;
    use std::time::Duration;

    let (cnf, _) = families::php(n);
    let nv = cnf.num_vars;
    let holes = n - 1;
    let mut db = cnf.clauses.clone();
    let mut index = AutomorphismIndex::with_clauses(nv, &cnf.clauses);
    let (mut t_auto, mut t_prop) = (Duration::ZERO, Duration::ZERO);
    let mut steps = 0usize;

    for m in (2..=n).rev() {
        let hole = m - 2;
        let last = m - 1;
        for i in 0..last {
            let clause = vec![Lit::neg((i * holes + hole) as u32)];
            let sigma = swap_pigeons(holes, n, i, last);
            let t = Instant::now();
            let is_auto = index.is_automorphism(&sigma);
            t_auto += t.elapsed();
            // The propagation half of the SR check, timed on its own.
            let t = Instant::now();
            let ok = is_auto
                && logicaffeine_proof::pr::is_pr_indexed(
                    nv,
                    &db,
                    &mut index,
                    &clause,
                    &Witness::Substitution(sigma.clone()),
                );
            t_prop += t.elapsed();
            let _ = &sigma;
            if ok {
                db.push(clause.clone());
                index.insert(clause);
                steps += 1;
            }
        }
    }
    println!(
        "PHP({n}) builder: steps={steps}  index.is_automorphism={t_auto:?}  propagation(+recheck)={t_prop:?}",
    );
}

/// THE unified auto-collapse agent: given any opaque DIMACS, recognize which physics collapses it
/// and report the Lyapunov-certified verdict.
fn run_auto_collapse(path: &str, cnf: &DimacsCnf) {
    use logicaffeine_proof::lyapunov::{auto_collapse, verify_lyapunov, AutoCollapse};
    let t = Instant::now();
    let result = auto_collapse(cnf.num_vars, &cnf.clauses);
    let dt = t.elapsed();
    match result {
        AutoCollapse::Geometric { measure, ranked } => {
            let ok = ranked.certify(cnf.num_vars, &cnf.clauses).is_some();
            println!(
                "{path}: COVERING/GEOMETRIC collapse — discovered {}×{}, refuted={} certifies={}  ({dt:?})",
                measure.items, measure.bins, ranked.refuted, ok
            );
        }
        AutoCollapse::Cardinality { trajectory, reached_goal, constraints } => {
            let ok = verify_lyapunov(&trajectory, reached_goal).is_some();
            println!(
                "{path}: CARDINALITY/CUTTING-PLANES collapse — {constraints} constraints summed, valid_lyapunov={}  ({dt:?})",
                ok
            );
        }
        AutoCollapse::Algebraic { trajectory, reached_goal, xor_equations } => {
            let ok = verify_lyapunov(&trajectory, reached_goal).is_some();
            println!(
                "{path}: PARITY/ALGEBRAIC collapse — {xor_equations} XOR constraints, dim {}→{}, valid_lyapunov={}  ({dt:?})",
                trajectory.first().copied().unwrap_or(0),
                trajectory.last().copied().unwrap_or(0),
                ok
            );
        }
        AutoCollapse::None => {
            println!("{path}: no covering or parity collapse recognized (bounded impossibility)  ({dt:?})")
        }
    }
}

/// THE unified demonstration: one Lyapunov framework, two structurally different collapses. Show the
/// geometric (symmetry) and algebraic (GF(2) parity) potentials descending to ⊥ under the SAME
/// machine-checked axioms.
fn show_unified_lyapunov(n: usize) {
    use logicaffeine_proof::lyapunov::{
        gaussian_lyapunov, lyapunov_of_symmetry, solve_by_measure_synthesis, verify_lyapunov,
    };
    let fmt = |traj: &[u64]| -> String {
        let mut d: Vec<u64> = traj.to_vec();
        d.dedup();
        d.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ▸ ")
    };

    println!("ONE Lyapunov framework, TWO physics — both checked by the same `verify_lyapunov`:\n");

    // (1) GEOMETRIC collapse — symmetry. Discovered from opaque CNF.
    let (cnf, _) = families::php(n);
    if let Some((m, ranked)) = solve_by_measure_synthesis(cnf.num_vars, &cnf.clauses) {
        if let Some(c) = lyapunov_of_symmetry(&ranked) {
            println!("  [GEOMETRIC]  PHP({n}) — potential V = active items remaining (discovered {}×{})", m.items, m.bins);
            println!("    descent:  {}", fmt(&ranked.ranks));
            println!(
                "    axioms ✓: monotone={} strict_descent={} reaches_goal={};  levels={} × width={} ⇒ size ≤ {} (actual {})",
                c.monotone, c.strict_descent, c.reaches_goal, c.levels, c.max_dissipation, c.size_bound, c.total_steps
            );
        }
    }

    // (2) ALGEBRAIC collapse — parity (Gaussian elimination over GF(2)).
    let (eqs, tcnf, _) = families::tseitin_expander(n.max(4) - (n % 2), 7);
    let (traj, reached) = gaussian_lyapunov(&eqs, tcnf.num_vars);
    if let Some(c) = verify_lyapunov(&traj, reached) {
        println!("\n  [ALGEBRAIC]  Tseitin(expander) — potential V = dimension of the unsolved GF(2) system");
        println!("    descent:  {}", fmt(&traj));
        println!(
            "    axioms ✓: monotone={} strict_descent={} reaches_goal={};  V bottoms at {} = the 0=1 contradiction",
            c.monotone, c.strict_descent, c.reaches_goal, c.minimum
        );
    }
    println!("\n  ⇒ Same four Lyapunov-stability axioms, same checker, two different exponential collapses.");
}

/// Discover, then **decode and prove** the Lyapunov function: show the potential, its monotone
/// descent, the per-level dissipation, and the checked certificate that the descent bounds the size.
fn show_lyapunov(label: &str, cnf: DimacsCnf) {
    use logicaffeine_proof::lyapunov::solve_by_measure_synthesis;
    use std::collections::BTreeMap;
    match solve_by_measure_synthesis(cnf.num_vars, &cnf.clauses) {
        Some((m, ranked)) => {
            let mut counts: BTreeMap<u64, u64> = BTreeMap::new();
            for &r in &ranked.ranks {
                *counts.entry(r).or_insert(0) += 1;
            }
            // The descent: distinct potential values, high → low.
            let mut levels: Vec<u64> = counts.keys().copied().collect();
            levels.sort_unstable_by(|a, b| b.cmp(a));
            let descent: Vec<String> = levels.iter().map(|r| r.to_string()).collect();
            let work: Vec<u64> = levels.iter().map(|r| counts[r]).collect();
            println!("{label}: DISCOVERED Lyapunov potential  V(state) = active items remaining");
            println!("  layout recovered from opaque CNF:  {} items × {} bins", m.items, m.bins);
            println!("  monotone descent:   V = {}", descent.join(" ▸ "));
            println!(
                "  per-level dissipation (clauses shed):  {:?}   (Σ = {} steps)",
                work,
                work.iter().sum::<u64>()
            );
            match ranked.certify(cnf.num_vars, &cnf.clauses) {
                Some(b) => {
                    println!(
                        "  PROVEN: descent is strictly non-increasing ⇒ valid Lyapunov fn;  levels={} × max_width={}  ⇒  size ≤ {} (actual {} ≤ {} ✓)",
                        b.levels, b.max_width, b.bound, b.actual, b.bound
                    );
                    println!("  ⇒ the potential dissipates to ⊥ ⇒ certified UNSAT, and the descent bounds the proof at O(n²).");
                }
                None => println!("  descent FAILED to certify (not a valid Lyapunov function)"),
            }
        }
        None => println!("{label}: no Lyapunov potential found in the covering class (bounded impossibility)"),
    }
}

/// The HONEST engine: discover the collapsing structure from opaque clauses (no n, no layout),
/// then let the proof fall out — the fair "fed only CNF" measurement.
fn discover(label: &str, cnf: DimacsCnf) {
    use logicaffeine_proof::lyapunov::solve_by_measure_synthesis;
    let t = Instant::now();
    let result = solve_by_measure_synthesis(cnf.num_vars, &cnf.clauses);
    let dt = t.elapsed();
    match result {
        Some((m, ranked)) => {
            let ok = ranked.certify(cnf.num_vars, &cnf.clauses).is_some();
            println!(
                "discover {label}  vars={} clauses={}  DISCOVERED items={} bins={}  refuted={} certifies={}  time={dt:?}",
                cnf.num_vars,
                cnf.clauses.len(),
                m.items,
                m.bins,
                ranked.refuted,
                ok
            );
        }
        None => println!("discover {label}  no covering measure found (honest bounded impossibility)  time={dt:?}"),
    }
}

/// The general SDCL solver (positive reduct) on opaque clauses — the SaDiCaL-class baseline.
fn run_sdcl(cnf: &DimacsCnf) {
    use logicaffeine_proof::sdcl::{solve_certified, CertifiedOutcome};
    let t = Instant::now();
    let out = solve_certified(cnf.num_vars, &cnf.clauses);
    let dt = t.elapsed();
    let v = match &out {
        CertifiedOutcome::Unsat { steps, discovered } => format!("UNSAT (discovered={discovered}, {} steps)", steps.len()),
        CertifiedOutcome::Sat(_) => "SAT".to_string(),
    };
    println!("sdcl  vars={} clauses={}  {v}  time={dt:?}", cnf.num_vars, cnf.clauses.len());
}

/// The GENERAL certified crusher (auto symmetry detection) on clique-coloring(n,k) — a
/// non-pigeonhole symmetric family, to prove the win is not PHP-overfit.
fn ours_clique(n: usize, k: usize) {
    use logicaffeine_proof::sdcl::{solve_certified, CertifiedOutcome};
    let (cnf, _) = families::clique_coloring(n, k);
    let nv = cnf.num_vars;
    let t = Instant::now();
    let out = solve_certified(nv, &cnf.clauses);
    let dt = t.elapsed();
    let (verdict, steps) = match &out {
        CertifiedOutcome::Unsat { steps, discovered } => (format!("UNSAT (PR-discovered={discovered})"), steps.len()),
        CertifiedOutcome::Sat(_) => ("SAT".to_string(), 0),
    };
    println!("ours  clique({n},{k})  vars={nv} clauses={}  {verdict}  steps={steps}  solve+certify={dt:?}", cnf.clauses.len());
    std::fs::write(format!("/tmp/clique_{n}_{k}.cnf"), dimacs::print(&cnf)).ok();
}

/// A random 3-regular graph on `n` (even) vertices via the configuration model with rejection of
/// self-loops/multi-edges — an expander w.h.p., which is what makes the Tseitin formula
/// resolution-hard.
fn random_3regular(n: usize, seed: u64) -> Vec<(usize, usize)> {
    let mut state = seed ^ 0x9E3779B97F4A7C15;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    for _attempt in 0..2000 {
        let mut stubs: Vec<usize> = (0..n).flat_map(|v| [v, v, v]).collect();
        for i in (1..stubs.len()).rev() {
            let j = (next() as usize) % (i + 1);
            stubs.swap(i, j);
        }
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut ok = true;
        for c in stubs.chunks(2) {
            let (a, b) = (c[0].min(c[1]), c[0].max(c[1]));
            if a == b || !seen.insert((a, b)) {
                ok = false;
                break;
            }
            edges.push((a, b));
        }
        if ok {
            return edges;
        }
    }
    panic!("could not build a simple 3-regular graph on {n} vertices");
}

/// The Tseitin formula on a random 3-regular graph with an odd total charge (UNSAT): one Boolean
/// per edge, and per vertex the XOR of its incident edges equals that vertex's charge. Exponentially
/// hard for resolution on an expander, trivially solved by Gaussian elimination over GF(2). Returns
/// the XOR system and its CNF expansion.
fn tseitin_expander(n: usize, seed: u64) -> (Vec<logicaffeine_proof::xorsat::XorEquation>, DimacsCnf) {
    use logicaffeine_proof::cdcl::Lit;
    use logicaffeine_proof::xorsat::XorEquation;
    assert!(n % 2 == 0, "3-regular needs an even vertex count");
    let edges = random_3regular(n, seed);
    let m = edges.len(); // = 3n/2 edge variables
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (e, &(a, b)) in edges.iter().enumerate() {
        incident[a].push(e);
        incident[b].push(e);
    }
    // Odd total charge ⇒ UNSAT: vertex 0 charged, the rest uncharged.
    let charge = |v: usize| v == 0;

    let mut eqs: Vec<XorEquation> = Vec::new();
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for v in 0..n {
        let inc = &incident[v];
        let r = charge(v);
        eqs.push(XorEquation::new(inc.clone(), r));
        // CNF expansion of XOR(inc) = r: forbid every assignment of `inc` whose parity ≠ r.
        let d = inc.len();
        for mask in 0u32..(1u32 << d) {
            let parity = (mask.count_ones() % 2) == 1;
            if parity != r {
                let clause: Vec<Lit> = (0..d)
                    .map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0))
                    .collect();
                clauses.push(clause);
            }
        }
    }
    (eqs, DimacsCnf { num_vars: m, clauses })
}

/// Solve the expander-Tseitin XOR system with Gaussian elimination over GF(2) — the polynomial
/// collapse of an exponentially resolution-hard formula.
fn ours_tseitin(n: usize, seed: u64) {
    use logicaffeine_proof::xorsat::{solve, XorOutcome};
    let (eqs, cnf) = tseitin_expander(n, seed);
    let t = Instant::now();
    let outcome = solve(&eqs, cnf.num_vars);
    let dt = t.elapsed();
    let verdict = match outcome {
        XorOutcome::Unsat(_) => "UNSAT (Gaussian refutation)",
        XorOutcome::Sat(_) => "SAT",
    };
    println!(
        "ours  tseitin(n={n})  edge-vars={} clauses={}  {verdict}  solve={dt:?}",
        cnf.num_vars,
        cnf.clauses.len()
    );
    std::fs::write(format!("/tmp/tseitin_{n}_{seed}.cnf"), dimacs::print(&cnf)).ok();
}

/// Symmetry STEERING on clique-coloring: build the color-swap and vertex-swap generators
/// structurally (free — no graph-automorphism search) and break them with certified SR, the way
/// the family's symmetry is known a priori rather than rediscovered.
fn steer_clique(n: usize, k: usize) {
    use logicaffeine_proof::sym_certify::heule_clique_refutation;
    let (cnf, _) = families::clique_coloring(n, k);
    let t = Instant::now();
    let cr = heule_clique_refutation(n, k);
    let dt = t.elapsed();
    println!(
        "steer clique({n},{k})  vars={} clauses={}  refuted={}  sbp={} steps={}  time={dt:?}",
        cnf.num_vars,
        cnf.clauses.len(),
        cr.refuted,
        cr.sbp_clauses,
        cr.steps.len()
    );
}

/// Dynamic symmetry breaking (SEL) vs plain CDCL on PHP(n): the conflict-count collapse.
fn sel_php(n: usize) {
    use logicaffeine_proof::cdcl::{SolveResult, Solver};
    use logicaffeine_proof::sym_dynamic::{sel_refute, SelOutcome};
    let (cnf, _) = families::php(n);
    let mut plain = Solver::new(cnf.num_vars);
    for c in &cnf.clauses {
        plain.add_clause(c.clone());
    }
    let t = Instant::now();
    let plain_v = plain.solve();
    let plain_dt = t.elapsed();
    let plain_c = plain.conflicts();
    assert_eq!(plain_v, SolveResult::Unsat);

    let t = Instant::now();
    let out = sel_refute(cnf.num_vars, &cnf.clauses);
    let sel_dt = t.elapsed();
    match out {
        SelOutcome::Unsat { conflicts, amplified, .. } => {
            println!(
                "PHP({n}): plain CDCL = {plain_c} conflicts ({plain_dt:?}) | SEL = {conflicts} conflicts (+{amplified} sym clauses, {sel_dt:?}) | {:.1}x fewer conflicts",
                plain_c as f64 / conflicts.max(1) as f64
            );
        }
        other => println!("PHP({n}): SEL did not refute ({other:?})"),
    }
}

/// Plain certified CDCL on an arbitrary instance: verdict + timing.
fn solve(cnf: &DimacsCnf) {
    let mut solver = Solver::new(cnf.num_vars);
    for c in &cnf.clauses {
        solver.add_clause(c.clone());
    }
    let t = Instant::now();
    let verdict = match solver.solve() {
        SolveResult::Sat(_) => "SATISFIABLE",
        SolveResult::Unsat => "UNSATISFIABLE",
    };
    let dt = t.elapsed();
    println!("ours  vars={} clauses={}  {verdict}  time={:?}", cnf.num_vars, cnf.clauses.len(), dt);
}
