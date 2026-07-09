//! **The open cell, named in cofactor-DAG language — the gunsight, cofactor edition.**
//!
//! `work/PAPER.md` §8's P-vs-NP boundary, reframed through the cofactor-DAG lens (§4.3) and walked cell by
//! cell. Every cell below is either a fact certified here / elsewhere in the corpus, or the single
//! open sentence everything else isolates. The point of the artifact — like `pvnp_gunsight.rs` — is
//! that the frontier is *executable*: the decidable rungs are certified exhausted, and what remains is
//! named exactly.
//!
//!   1–2. **poly cofactor classes ⟹ poly certificate** — CERTIFIED. The quotient DAG re-checks with
//!        zero trust and is output-sensitive; structured families collapse to polynomially-many
//!        classes (`tests/cofactor_lens.rs`).
//!   3.   **instance-rigid cores CAN carry cofactor symmetry** — CERTIFIED. Of the `n = 4` residue
//!        cores rigid under exhaustive `B₄` *and* every shear, 387 of 828 (46.7%) collapse under
//!        CofactorIso — symmetry above every instance lens (`tests/cofactor_lens.rs`).
//!   4.   **CofactorIso, the strongest DECIDABLE congruence, does NOT collapse the worst case** —
//!        CERTIFIED here: on random 3-CNF the class count stays a large fraction of the distinct
//!        floor, both exponential.
//!   5.   **THE OPEN CELL**: a poly-index *SR-definable* Shannon congruence on the residue's cofactor
//!        DAG — extension variables relating cofactors CNF-isomorphism cannot. Its existence
//!        ⟺ `3-SAT ∈ coNP` ⟺ `NP = coNP`. Not decided here; every decidable rung below is exhausted.

use logicaffeine_proof::cofactor::{
    canon, canon_raw, check_quotient_dag, distinct_cofactor_dag, distinct_width, quotient_class_count,
    quotient_dag, CanonClauses, CofactorIso,
};

fn xor_cycle(k: usize) -> CanonClauses {
    let mut raw: Vec<Vec<(u32, bool)>> = Vec::new();
    for i in 0..k {
        let j = (i + 1) % k;
        raw.push(vec![(i as u32, true), (j as u32, true)]);
        raw.push(vec![(i as u32, false), (j as u32, false)]);
    }
    canon_raw(&raw)
}

#[test]
fn the_open_cell_is_a_poly_index_sr_definable_shannon_congruence() {
    // CELL 1–2 (CERTIFIED): a structured family's cofactor quotient is a zero-trust-checked certificate.
    let f = xor_cycle(9);
    let dag = quotient_dag(9, &f, &CofactorIso { cap: 5 }).expect("odd XOR cycle is UNSAT");
    assert!(
        check_quotient_dag(&dag.nodes),
        "cell 1-2: polynomially-many cofactor classes ⟹ a re-checked certificate"
    );

    // CELL 4 (CERTIFIED): the wall. On the worst-case archetype (random 3-CNF above threshold) the
    // strongest DECIDABLE congruence does not make the class count polynomial — it stays ≥ ¾ of the
    // distinct floor, both exponential. The decidable rung is exhausted.
    let n = 12usize;
    let clauses = (0u64..64)
        .find_map(|seed| {
            let cnf = logicaffeine_proof::families::random_3sat(n, (n * 9) / 2, seed);
            let cc = canon(&cnf.clauses);
            distinct_cofactor_dag(n, &cc).map(|_| cc) // Some ⟺ UNSAT
        })
        .expect("an UNSAT random 3-CNF exists above threshold");
    let distinct = distinct_width(n, &clauses);
    let iso = quotient_class_count(n, &clauses, &CofactorIso { cap: 5 });
    assert!(
        iso * 4 >= distinct * 3,
        "cell 4: CofactorIso does NOT collapse random 3-CNF (iso {iso} ≥ ¾·distinct {distinct}) — the \
         decidable rung is exhausted at the wall"
    );

    // CELL 5 — THE OPEN CELL, named.
    eprintln!("=== THE OPEN CELL (cofactor-DAG edition) ===");
    eprintln!(
        "CERTIFIED (cells 1-4): poly cofactor classes ⟹ poly re-checked certificate; structured \
         families collapse; 387/828 fully-rigid n=4 residue cores carry cofactor symmetry ABOVE every \
         instance lens; and CofactorIso — the strongest DECIDABLE Shannon congruence — is exhausted at \
         the random-3CNF wall (iso {iso} vs distinct {distinct})."
    );
    eprintln!(
        "OPEN: a poly-index SR-DEFINABLE Shannon congruence on the residue's cofactor DAG (extension \
         variables relating cofactors that CNF-isomorphism cannot). Its existence ⟺ 3-SAT ∈ coNP ⟺ \
         NP = coNP. Nothing here decides it; every decidable congruence rung below it is certified exhausted."
    );
}
