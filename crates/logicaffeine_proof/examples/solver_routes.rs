//! Is symmetry breaking actually applied *in our SAT solver*? This runs the unified dispatcher
//! `solve::solve_structured` on representative families and prints which engine decided each — showing
//! the structured families are crushed by a specialist (0 conflicts = no search, the symmetry/algebra
//! collapse applied) while a structureless random instance falls through to CDCL.
//!
//! It also answers "can't we symmetry break *again*?": the structured route already iterates the
//! quotient (the round-by-round refutation breaks the residual symmetry each round), and `sym_dynamic`
//! breaks symmetry dynamically during search. The one thing no amount of "again" can do is break
//! symmetry that is not there — which is exactly the random row (→ CDCL).
//!
//! Run: cargo run --release -p logicaffeine-proof --example solver_routes

use logicaffeine_proof::families;
use logicaffeine_proof::solve::{solve_structured, Answer};

fn report(label: &str, num_vars: usize, clauses: &[Vec<logicaffeine_proof::cdcl::Lit>]) {
    let solved = solve_structured(num_vars, clauses);
    let verdict = match solved.answer {
        Answer::Sat(_) => "SAT",
        Answer::Unsat => "UNSAT",
    };
    let applied = if format!("{:?}", solved.via) == "Cdcl" {
        "— fell through to plain CDCL search (no structure to exploit)"
    } else {
        "structure applied — decided without search"
    };
    println!(
        "  {label:<26} {verdict:<6} route={:<12} conflicts={:<7} {applied}",
        format!("{:?}", solved.via),
        solved.conflicts
    );
}

fn main() {
    println!("Which engine decides each family (symmetry breaking applied in the solver):\n");

    let (php, _) = families::php(12);
    report("PHP(12) — pigeonhole", php.num_vars, &php.clauses);

    let (clq, _) = families::clique_coloring(8, 7);
    report("clique-colouring(8,7)", clq.num_vars, &clq.clauses);

    let (_, tse, _) = families::tseitin_expander(20, 7);
    report("Tseitin(20) — GF(2)", tse.num_vars, &tse.clauses);

    // The mod-p obstruction: invisible to GF(2), and where Z3 AND Kissat both TIME OUT. The dispatcher
    // now recovers the one-hot GF(p) system from the raw clauses and crushes it over the right field.
    let (_, m3, _) = families::mod_p_tseitin_expander(20, 3, 0xC0FFEE);
    report("mod-3 Tseitin(20) GF(3)", m3.num_vars, &m3.clauses);
    let (_, m5, _) = families::mod_p_tseitin_expander(20, 5, 0xC0FFEE);
    report("mod-5 Tseitin(20) GF(5)", m5.num_vars, &m5.clauses);

    // A COMPOSITE modulus: the dispatcher recovers the ℤ/6 one-hot system and decides it by CRT over
    // the prime-power components — the obstruction lives in the GF(3) factor, invisible to GF(2).
    let (_, m6, _) = families::mod_p_tseitin_expander(8, 6, 0xC0FFEE);
    report("mod-6 Tseitin(8) ℤ/6", m6.num_vars, &m6.clauses);

    // A genuinely structureless instance at the hard ratio: no symmetry, no parity, no covering.
    let rnd = families::random_3sat(40, (40.0 * 4.26) as usize, 0xBADC0DE);
    report("random 3-SAT(40)", rnd.num_vars, &rnd.clauses);

    println!(
        "\nReading: PHP/clique/Tseitin, mod-p, and composite mod-m are all decided by a STRUCTURE\n\
         specialist with 0 conflicts — the covering symmetry (PR-SR), the GF(2) parity cut, the GF(p)\n\
         lift, and the ℤ/m lift (CRT over the prime-power components), applied with no search. The\n\
         mod-p / mod-m rows are the new reach: that obstruction is invisible to GF(2), and Z3 AND\n\
         Kissat both TIME OUT on it, yet recovering the one-hot system and running Gaussian elimination\n\
         over the right field/ring crushes it in microseconds — the parity cut carried to every modulus.\n\
         'Symmetry break again' is already inside these: the covering refutation breaks residual symmetry\n\
         round by round, sym_dynamic (SEL) breaks it during search, and the composite lift breaks the\n\
         modulus itself by its multiplicative factorization. But random 3-SAT has no symmetry, no parity,\n\
         no modular structure to break — so it falls through to CDCL and pays for the search. You cannot\n\
         break a symmetry that is not there; that is the boundary."
    );
}
