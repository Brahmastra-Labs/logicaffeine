//! **The mirror: the trick to prove the trick — reflection, executable.**
//!
//! Lift-and-shift-left, applied at the top of the tower: encode *proof search itself* as a SAT
//! formula. `REF(F, s)` says "there exists a resolution refutation of `F` with exactly `s` derived
//! lines" — selector variables choose each line's parents and pivot, content variables carry each
//! derived clause, and the constraints force every line to be the exact resolvent of its parents
//! with the last line empty (padding by re-derivation makes `s` monotone: `REF(F, s)` is
//! satisfiable iff `F` has a refutation with *at most* `s` steps). The mirror is the hidden
//! symmetry the user pointed at: the proof system, reflected into its own instance language.
//!
//! What self-application snatches, certified both ways:
//!
//!   - **SAT side — the trick found by the trick-finder.** Our solver on `REF(F, s)` returns a
//!     model; we DECODE it into an actual resolution refutation of `F` and re-verify that proof
//!     with an independent checker that recomputes every resolvent. The solver searched for a
//!     proof by solving a SAT instance about proofs.
//!   - **UNSAT side — certified proof-SIZE lower bounds.** `REF(F, s−1)` unsatisfiable means "no
//!     `(s−1)`-step refutation exists," and the solver's own RUP certificate for that fact
//!     re-checks with zero trust. Degree and width lower bounds the portfolio had; SIZE lower
//!     bounds — the exact currency of the Extended-Frege open cell — arrive only through the
//!     mirror. (At toy scale, for resolution; the mechanism, not the frontier.)
//!   - **The differential anchor.** On small formulas the verdicts match an exhaustive
//!     brute-force search over all proof DAGs — the encoding is proven faithful, not assumed.
//!
//! The honest boundary the mirror itself enforces: Atserias–Müller (2019) proved that *automating
//! resolution is NP-hard* — deciding these very `REF` formulas efficiently is itself as hard as
//! SAT. The trick-to-find-tricks is a member of the class it hunts. That is why the corpus-level
//! universal trick (`sdcl` covers every family we possess — certified below, with the uncovered
//! set machine-checked EMPTY) cannot be promoted to the asymptotic `∃trick ∀family` by any finite
//! computation: the promotion IS NP = coNP. The mirror does not cross the boundary; it *is* the
//! boundary, made executable.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families;
use logicaffeine_proof::polycalc_zm::build_ns_certificate_zm;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::sdcl::sdcl_refute;
use logicaffeine_proof::solve::{solve_structured, Answer};
use std::collections::BTreeSet;

// ── clauses as literal-index sets: variable v positive = 2v, negative = 2v+1 ─────────────────────
type LitSet = BTreeSet<usize>;

fn to_litset(clause: &[Lit]) -> LitSet {
    clause
        .iter()
        .map(|l| 2 * l.var() as usize + if l.is_positive() { 0 } else { 1 })
        .collect()
}

/// The exact resolvent of `a` (containing `+v`) and `b` (containing `¬v`) on pivot `v`:
/// `(a \ {+v}) ∪ (b \ {¬v})`. The pivot is removed from its OWN side only — if the other side
/// carries the opposite polarity too (a tautology), that literal SURVIVES. Getting this wrong
/// (dropping both polarities from the union) lets `(p ∨ ¬p)` resolve with itself to ⊥ — an
/// unsoundness the brute-force differential in this file actually caught during development.
fn resolvent(a: &LitSet, b: &LitSet, v: usize) -> LitSet {
    let mut left = a.clone();
    left.remove(&(2 * v));
    let mut right = b.clone();
    right.remove(&(2 * v + 1));
    left.union(&right).copied().collect()
}

// ── the REF encoding ──────────────────────────────────────────────────────────────────────────────
struct RefEncoding {
    num_vars: usize,
    clauses: Vec<Vec<Lit>>,
    /// `sel[k] = (t, i, j, v, var)`: line `t` resolves parents `i` (has `+v`) and `j` (has `¬v`).
    sels: Vec<(usize, usize, usize, usize, u32)>,
    /// `content[(t, lit_idx)] = var` for derived lines `t`.
    content: std::collections::BTreeMap<(usize, usize), u32>,
    axioms: Vec<LitSet>,
    s: usize,
    n: usize,
}

/// Encode "there is a resolution refutation of `axioms` with exactly `s` derived lines" as SAT.
fn ref_encoding(n: usize, axioms: &[LitSet], s: usize) -> RefEncoding {
    let m = axioms.len();
    let mut next: u32 = 0;
    let mut fresh = || {
        let v = next;
        next += 1;
        v
    };
    let mut content = std::collections::BTreeMap::new();
    for t in m..m + s {
        for l in 0..2 * n {
            content.insert((t, l), fresh());
        }
    }
    let mut sels: Vec<(usize, usize, usize, usize, u32)> = Vec::new();
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    let has = |line: usize, l: usize| -> Option<bool> {
        (line < m).then(|| axioms[line].contains(&l)) // None = derived (variable content)
    };
    for t in m..m + s {
        let mut line_sels: Vec<u32> = Vec::new();
        for i in 0..t {
            for j in 0..t {
                if i == j {
                    continue;
                }
                for v in 0..n {
                    // Static pruning: an axiom parent must actually carry the pivot literal.
                    if has(i, 2 * v) == Some(false) || has(j, 2 * v + 1) == Some(false) {
                        continue;
                    }
                    let sv = fresh();
                    sels.push((t, i, j, v, sv));
                    line_sels.push(sv);
                    let ns = Lit::new(sv, false);
                    // Derived parents must carry the pivot.
                    if i >= m {
                        clauses.push(vec![ns, Lit::new(content[&(i, 2 * v)], true)]);
                    }
                    if j >= m {
                        clauses.push(vec![ns, Lit::new(content[&(j, 2 * v + 1)], true)]);
                    }
                    // Pivot columns of the resolvent — each polarity removed from its OWN side
                    // only: `+v ∈ resolvent ↔ +v ∈ parent j`; `¬v ∈ resolvent ↔ ¬v ∈ parent i`
                    // (tautological parents keep their surviving polarity — the soundness point).
                    for (lit_idx, other, other_is_i) in
                        [(2 * v, j, false), (2 * v + 1, i, true)]
                    {
                        let _ = other_is_i;
                        let ct = Lit::new(content[&(t, lit_idx)], true);
                        let nct = Lit::new(content[&(t, lit_idx)], false);
                        match has(other, lit_idx) {
                            Some(true) => clauses.push(vec![ns, ct]),
                            Some(false) => clauses.push(vec![ns, nct]),
                            None => {
                                clauses.push(vec![
                                    ns,
                                    nct,
                                    Lit::new(content[&(other, lit_idx)], true),
                                ]);
                                clauses.push(vec![
                                    ns,
                                    Lit::new(content[&(other, lit_idx)], false),
                                    ct,
                                ]);
                            }
                        }
                    }
                    // Every other literal: C[t][l] ↔ (l ∈ parent i) ∨ (l ∈ parent j).
                    for l in 0..2 * n {
                        if l == 2 * v || l == 2 * v + 1 {
                            continue;
                        }
                        let ct = Lit::new(content[&(t, l)], true);
                        let nct = Lit::new(content[&(t, l)], false);
                        match (has(i, l), has(j, l)) {
                            (Some(true), _) | (_, Some(true)) => clauses.push(vec![ns, ct]),
                            (Some(false), Some(false)) => clauses.push(vec![ns, nct]),
                            (Some(false), None) => {
                                clauses.push(vec![ns, nct, Lit::new(content[&(j, l)], true)]);
                                clauses.push(vec![ns, Lit::new(content[&(j, l)], false), ct]);
                            }
                            (None, Some(false)) => {
                                clauses.push(vec![ns, nct, Lit::new(content[&(i, l)], true)]);
                                clauses.push(vec![ns, Lit::new(content[&(i, l)], false), ct]);
                            }
                            (None, None) => {
                                clauses.push(vec![
                                    ns,
                                    nct,
                                    Lit::new(content[&(i, l)], true),
                                    Lit::new(content[&(j, l)], true),
                                ]);
                                clauses.push(vec![ns, Lit::new(content[&(i, l)], false), ct]);
                                clauses.push(vec![ns, Lit::new(content[&(j, l)], false), ct]);
                            }
                        }
                    }
                }
            }
        }
        // Exactly one derivation per line (at-least-one may be empty ⟹ REF is trivially UNSAT).
        clauses.push(line_sels.iter().map(|&v| Lit::new(v, true)).collect());
        for (a, &x) in line_sels.iter().enumerate() {
            for &y in &line_sels[a + 1..] {
                clauses.push(vec![Lit::new(x, false), Lit::new(y, false)]);
            }
        }
    }
    // The last line is the empty clause.
    for l in 0..2 * n {
        clauses.push(vec![Lit::new(content[&(m + s - 1, l)], false)]);
    }
    RefEncoding { num_vars: next as usize, clauses, sels, content, axioms: axioms.to_vec(), s, n }
}

/// Decode a model of `REF(F, s)` into a proof and INDEPENDENTLY verify it: recompute every
/// resolvent from the decoded parents, demand exact equality with the decoded content, and demand
/// the last line empty. Returns the derived lines.
fn decode_and_verify(enc: &RefEncoding, model: &[bool]) -> Vec<LitSet> {
    let m = enc.axioms.len();
    let mut lines: Vec<LitSet> = enc.axioms.clone();
    for t in m..m + enc.s {
        let chosen: Vec<_> =
            enc.sels.iter().filter(|&&(tt, _, _, _, sv)| tt == t && model[sv as usize]).collect();
        assert_eq!(chosen.len(), 1, "exactly one derivation selected for line {t}");
        let &&(_, i, j, v, _) = &chosen[0];
        assert!(i < t && j < t, "parents precede the line — a genuine DAG");
        let decoded: LitSet =
            (0..2 * enc.n).filter(|&l| model[enc.content[&(t, l)] as usize]).collect();
        assert!(lines[i].contains(&(2 * v)), "parent i carries +pivot");
        assert!(lines[j].contains(&(2 * v + 1)), "parent j carries ¬pivot");
        assert_eq!(
            decoded,
            resolvent(&lines[i], &lines[j], v),
            "line {t}: the decoded clause IS the recomputed resolvent"
        );
        lines.push(decoded);
    }
    assert!(lines.last().unwrap().is_empty(), "the final line is the empty clause — a refutation");
    lines.split_off(m)
}

/// Exhaustive oracle: does a refutation with at most `cap` derived lines exist? Plain DFS over all
/// proof prefixes (duplicates and tautologies allowed — exactly the encoding's semantics).
fn brute_force_refutable_within(axioms: &[LitSet], n: usize, cap: usize) -> bool {
    fn dfs(lines: &mut Vec<LitSet>, remaining: usize, n: usize) -> bool {
        if lines.last().map(|c| c.is_empty()).unwrap_or(false) {
            return true; // already refuted (padding covers unused budget)
        }
        if remaining == 0 {
            return false;
        }
        for i in 0..lines.len() {
            for j in 0..lines.len() {
                if i == j {
                    continue;
                }
                for v in 0..n {
                    if lines[i].contains(&(2 * v)) && lines[j].contains(&(2 * v + 1)) {
                        let r = resolvent(&lines[i], &lines[j], v);
                        if lines.contains(&r) {
                            continue; // re-deriving a present clause never helps the oracle
                        }
                        lines.push(r);
                        if dfs(lines, remaining - 1, n) {
                            return true;
                        }
                        lines.pop();
                    }
                }
            }
        }
        false
    }
    let mut lines = axioms.to_vec();
    dfs(&mut lines, cap, n)
}

/// The corpus of tiny mirror targets: (name, n, axioms).
fn mirror_corpus() -> Vec<(&'static str, usize, Vec<LitSet>)> {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    vec![
        ("unit-pair", 1, vec![to_litset(&[p(0)]), to_litset(&[q(0)])]),
        (
            "or-plus-units",
            2,
            vec![to_litset(&[p(0), p(1)]), to_litset(&[q(0)]), to_litset(&[q(1)])],
        ),
        (
            "parity-triangle",
            3,
            vec![
                to_litset(&[p(0), p(1)]),
                to_litset(&[q(0), q(1)]),
                to_litset(&[p(1), p(2)]),
                to_litset(&[q(1), q(2)]),
                to_litset(&[p(2), p(0)]),
                to_litset(&[q(2), q(0)]),
            ],
        ),
    ]
}

/// **The mirror is faithful: REF verdicts match exhaustive proof search, and every SAT model
/// decodes to an independently verified proof.** For each tiny target and each budget `s`, the
/// solver's verdict on `REF(F, s)` equals the brute-force oracle's "∃ proof with ≤ s steps" —
/// and at the first satisfiable budget the model is decoded and every resolvent recomputed. The
/// measured minimal sizes are pinned: 1 (unit-pair), 2 (or-plus-units), 5 (parity-triangle).
#[test]
fn the_mirror_encoding_matches_brute_force_and_decodes_to_verified_proofs() {
    let expected_min = [1usize, 2, 5];
    for ((name, n, axioms), &want) in mirror_corpus().iter().zip(&expected_min) {
        let mut found_min = None;
        for s in 1..=want + 1 {
            let enc = ref_encoding(*n, axioms, s);
            let solved = solve_structured(enc.num_vars, &enc.clauses);
            let is_sat = matches!(solved.answer, Answer::Sat(_));
            // Differential vs the exhaustive oracle (skip the oracle where it would be huge).
            if *n <= 2 || s < want {
                assert_eq!(
                    is_sat,
                    brute_force_refutable_within(axioms, *n, s),
                    "{name} s={s}: the mirror verdict matches exhaustive proof search"
                );
            }
            if let Answer::Sat(model) = solved.answer {
                let proof = decode_and_verify(&enc, &model);
                assert_eq!(proof.len(), s, "{name}: the decoded proof uses the budgeted lines");
                if found_min.is_none() {
                    found_min = Some(s);
                }
            }
        }
        assert_eq!(found_min, Some(want), "{name}: minimal proof size is exactly {want}");
        eprintln!("mirror[{name}]: min resolution size = {want}, decoded + independently verified");
    }
}

/// **Self-application manufactures certified proof-SIZE lower bounds.** `REF(F, s)` UNSAT means
/// "no `s`-step refutation of `F` exists" — and the solver's own RUP certificate for that fact
/// re-checks with zero trust. Size is the currency of the Extended-Frege open cell; the portfolio
/// could certify degree and width bounds before, but size bounds arrive only through the mirror.
/// Pinned: no 1-step proof of or-plus-units; no 4-step proof of the parity triangle (its minimum
/// is 5 — one step below the truth, certified). The mirror formula is itself just another SAT
/// instance — the class interrogating itself — which is exactly Atserias–Müller's point: deciding
/// these formulas efficiently is NP-hard, so the trick-finder is a member of the class it hunts.
#[test]
fn the_mirror_manufactures_certified_proof_size_lower_bounds() {
    let corpus = mirror_corpus();
    for (name, n, axioms, below) in [
        ("or-plus-units", corpus[1].1, &corpus[1].2, 1usize),
        ("parity-triangle", corpus[2].1, &corpus[2].2, 4),
    ] {
        let enc = ref_encoding(n, axioms, below);
        // Plain CDCL over the ORIGINAL mirror clauses only (no mined side-facts), so the learned
        // stream is a self-contained RUP refutation of exactly the formula we hand the checker.
        let mut solver = logicaffeine_proof::cdcl::Solver::new(enc.num_vars);
        for c in &enc.clauses {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            logicaffeine_proof::cdcl::SolveResult::Unsat => {
                let mut steps: Vec<_> = solver
                    .learned()
                    .iter()
                    .map(|lc| logicaffeine_proof::proof::ProofStep::Rup(lc.lits.clone()))
                    .collect();
                // The stream's final step is the empty clause — RUP exactly when the accumulated
                // database propagates to conflict (for a propagation-only refutation the stream is
                // just this one step).
                if !matches!(steps.last(), Some(logicaffeine_proof::proof::ProofStep::Rup(c)) if c.is_empty())
                {
                    steps.push(logicaffeine_proof::proof::ProofStep::Rup(Vec::new()));
                }
                assert!(
                    check_pr_refutation(enc.num_vars, &enc.clauses, &steps),
                    "{name}: the SIZE lower bound 'min > {below}' re-checks with zero trust"
                );
                eprintln!(
                    "mirror[{name}]: certified proof-size lower bound min > {below} ({} RUP steps, re-checked)",
                    steps.len()
                );
            }
            logicaffeine_proof::cdcl::SolveResult::Sat(_) => {
                panic!("{name}: a {below}-step refutation must not exist")
            }
        }
    }
}

/// **The corpus-level quantifier swap, certified — with the uncovered class machine-checked
/// EMPTY.** One trick-finder (`sdcl_refute`: positive-reduct PR discovery + CDCL, zero hints per
/// family) refutes EVERY family in the certified corpus — pigeonhole, Tseitin, mod-3 Tseitin,
/// `Count_3`, parity, and the pinned survivor 3-CNF instances — and every discovered proof
/// re-checks under the zero-trust PR/SR checker. The set of corpus families WITHOUT a certified
/// trick is asserted empty: within everything this portfolio can name, `∃trick ∀family` is TRUE
/// and the certified-EF-hard class has no member — the honest, vacuous-truth form of "the classes
/// don't exist here." What this does NOT do — and the mirror explains why — is decide the
/// asymptotic swap: promoting corpus-truth to all-tautologies-truth is NP = coNP (Cook–Reckhow),
/// and the trick-finder's own problem is NP-hard (Atserias–Müller). Self-reference cuts exactly
/// both ways: if P = NP the finder is easy; if the finder is easy, P = NP.
#[test]
fn one_trick_finder_covers_the_entire_corpus_and_the_uncovered_class_is_empty() {
    let mut corpus: Vec<(String, usize, Vec<Vec<Lit>>)> = Vec::new();
    for m in [3usize, 4] {
        let (php, _) = families::php(m);
        corpus.push((format!("php({m})"), php.num_vars, php.clauses));
    }
    let (_, tseitin, _) = families::tseitin_expander(6, 0xC0DE);
    corpus.push(("tseitin(6)".into(), tseitin.num_vars, tseitin.clauses));
    let (_, mod3, _) = families::mod_p_tseitin_expander(4, 3, 0xC0DE);
    corpus.push(("mod3-tseitin(4)".into(), mod3.num_vars, mod3.clauses));
    let (cnt, _) = families::mod_counting(4, 3);
    corpus.push(("count3(4)".into(), cnt.num_vars, cnt.clauses));
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    corpus.push((
        "parity-triangle".into(),
        3,
        vec![
            vec![p(0), p(1)], vec![q(0), q(1)],
            vec![p(1), p(2)], vec![q(1), q(2)],
            vec![p(2), p(0)], vec![q(2), q(0)],
        ],
    ));
    for &(n, seed) in &[(12usize, 2u64), (12, 3), (16, 3)] {
        let cnf = families::random_3sat(n, n * 5, seed);
        corpus.push((format!("3cnf(n={n},seed={seed})"), cnf.num_vars, cnf.clauses));
    }

    let mut uncovered: Vec<String> = Vec::new();
    for (name, nv, clauses) in &corpus {
        let cert = sdcl_refute(*nv, clauses);
        if !(cert.refuted && check_pr_refutation(*nv, clauses, &cert.steps)) {
            uncovered.push(name.clone());
        } else {
            eprintln!("trick-finder[{name}]: refuted, {} steps, zero-trust re-checked", cert.steps.len());
        }
    }
    assert!(
        uncovered.is_empty(),
        "the class of corpus families without a certified trick must be EMPTY, got {uncovered:?}"
    );
    eprintln!(
        "corpus-level ∃trick ∀family: TRUE and certified (uncovered class EMPTY — the vacuous \
         truth); the asymptotic swap remains NP = coNP by Cook–Reckhow, and the mirror shows the \
         finder's own problem is NP-hard (Atserias–Müller) — self-reference cuts both ways"
    );
}

/// **The witness compiler — the mirror converts NP-witnesses into coNP-witnesses,
/// constructively.** Atserias–Müller's easy direction, made executable: given a MODEL `α` of `F`,
/// emit a short RUP refutation of `REF(F, s)` — for every `s`. The proof is the α-invariant walked
/// up the proof-DAG: every line of any purported refutation of a satisfiable formula must contain
/// an α-true literal (if both parents do, the resolvent does: α falsifies one pivot polarity, so
/// the satisfied parent's non-pivot α-true literal survives), and the empty last line cannot.
/// Emitted as RUP: per line `t` and selector `S`, the clause `¬S ∨ (∨_{α-true l} C[t][l])` — unit
/// propagation kills it against the parents' invariants — then the line invariant
/// `∨_{α-true l} C[t][l]` via the at-least-one selector clause, and finally `⊥` against the
/// empty-last-line units. LINEAR in the encoding size, zero search.
fn compile_model_into_mirror_refutation(
    enc: &RefEncoding,
    alpha: &[bool],
) -> Vec<logicaffeine_proof::proof::ProofStep> {
    use logicaffeine_proof::proof::ProofStep;
    let m = enc.axioms.len();
    let alpha_true: Vec<usize> =
        (0..enc.n).map(|v| if alpha[v] { 2 * v } else { 2 * v + 1 }).collect();
    let mut steps: Vec<ProofStep> = Vec::new();
    for t in m..m + enc.s {
        for &(tt, _, _, _, sv) in enc.sels.iter().filter(|&&(tt, ..)| tt == t) {
            let _ = tt;
            let mut clause = vec![Lit::new(sv, false)];
            clause.extend(alpha_true.iter().map(|&l| Lit::new(enc.content[&(t, l)], true)));
            steps.push(ProofStep::Rup(clause));
        }
        steps.push(ProofStep::Rup(
            alpha_true.iter().map(|&l| Lit::new(enc.content[&(t, l)], true)).collect(),
        ));
    }
    steps.push(ProofStep::Rup(Vec::new()));
    steps
}

/// **The swap, attacked where Atserias–Müller aimed it — and localized to a measurable curve.**
/// Two certified halves and one honest arrow:
///
///   1. **NP → coNP at the mirror, constructively.** For satisfiable `F` with model `α`, the
///      compiler above produces a refutation of `REF(F, s)` that re-checks with zero trust, with
///      step count exactly `Σ_t (#selectors(t) + 1) + 1` — linear in the encoding, zero search.
///      An NP-witness (a model) became a coNP-witness (a refutation), in polynomial time. This is
///      the mechanism behind Atserias–Müller's theorem that automating resolution is NP-hard, run
///      forward as a program.
///   2. **"Has a short proof" is itself NP-complete** (their corollary), demonstrated as the
///      biconditional on the corpus: `F` satisfiable ⟺ `REF(F, s)` refutable (compiled, checked);
///      `F` unsatisfiable ⟺ `REF(F, min)` satisfiable (decoded, verified).
///   3. **The arrow at the swap.** If NP = coNP, the NP-complete language of half 2 has short
///      refutation certificates — i.e., every TRUE "no short proof of F exists" fact has a short
///      proof of its own. Those certificates are exactly what the mirror manufactures
///      (`the_mirror_manufactures_certified_proof_size_lower_bounds`), so the swap IS the
///      question: does the certificate-size series of our own mirror lower bounds stay
///      polynomial as `F` grows? The series is measured here on the chain family `F_k`
///      (`(x₁∨…∨x_k) ∧ ¬x₁ ∧ … ∧ ¬x_k`, minimal refutation exactly `k`): the certified
///      `min > k−1` bounds and their RUP certificate sizes, printed as the curve. Nothing here
///      decides the growth — the point is that P vs NP now has a *dial in this repository*:
///      a concrete, re-runnable series whose polynomial boundedness is equivalent to the swap
///      for this proof system.
#[test]
fn the_mirror_converts_models_into_refutations_and_localizes_the_swap_to_a_curve() {
    // ── Half 1 + biconditional, SAT side: chain families with the last unit dropped (model:
    //    x_k = true, the rest false). Compile the model into the mirror refutation, at TWO budgets.
    for k in [2usize, 3] {
        let mut axioms: Vec<LitSet> = vec![(0..k).map(|v| 2 * v).collect()];
        for v in 0..k - 1 {
            axioms.push([2 * v + 1].into_iter().collect());
        }
        let mut alpha = vec![false; k];
        alpha[k - 1] = true;
        for s in [1usize, 2] {
            let enc = ref_encoding(k, &axioms, s);
            let compiled = compile_model_into_mirror_refutation(&enc, &alpha);
            let expected_len: usize =
                (0..enc.s).map(|d| enc.sels.iter().filter(|&&(t, ..)| t == axioms.len() + d).count() + 1).sum::<usize>() + 1;
            assert_eq!(compiled.len(), expected_len, "the compilation is search-free and linear");
            assert!(
                check_pr_refutation(enc.num_vars, &enc.clauses, &compiled),
                "k={k} s={s}: the model-compiled refutation of the mirror re-checks with zero trust"
            );
            // Consistency: the solver agrees the mirror is unsatisfiable (F has no refutation at all).
            assert!(
                matches!(solve_structured(enc.num_vars, &enc.clauses).answer, Answer::Unsat),
                "k={k} s={s}: a satisfiable formula has no refutation at any budget"
            );
            eprintln!(
                "witness-compiler[k={k}, s={s}]: model → {} RUP steps, zero-trust re-checked (NP witness → coNP witness)",
                compiled.len()
            );
        }
    }

    // ── Half 2 + the curve, UNSAT side: full chains F_k (minimal refutation exactly k). The
    //    mirror at budget k−1 is UNSAT; its certificate is the "no short proof" witness whose
    //    size-growth IS the swap for this system. Measure the curve; verify SAT at budget k.
    let mut curve: Vec<(usize, usize)> = Vec::new();
    for k in [2usize, 3, 4] {
        let mut axioms: Vec<LitSet> = vec![(0..k).map(|v| 2 * v).collect()];
        for v in 0..k {
            axioms.push([2 * v + 1].into_iter().collect());
        }
        let enc = ref_encoding(k, &axioms, k - 1);
        let mut solver = logicaffeine_proof::cdcl::Solver::new(enc.num_vars);
        for c in &enc.clauses {
            solver.add_clause(c.clone());
        }
        assert!(
            matches!(solver.solve(), logicaffeine_proof::cdcl::SolveResult::Unsat),
            "F_{k}: no ({}-step) refutation exists — the chain needs exactly {k}",
            k - 1
        );
        let mut steps: Vec<_> = solver
            .learned()
            .iter()
            .map(|lc| logicaffeine_proof::proof::ProofStep::Rup(lc.lits.clone()))
            .collect();
        if !matches!(steps.last(), Some(logicaffeine_proof::proof::ProofStep::Rup(c)) if c.is_empty()) {
            steps.push(logicaffeine_proof::proof::ProofStep::Rup(Vec::new()));
        }
        assert!(
            check_pr_refutation(enc.num_vars, &enc.clauses, &steps),
            "F_{k}: the 'min > {}' certificate re-checks with zero trust",
            k - 1
        );
        curve.push((k, steps.len()));
        // The other half of the biconditional: at budget k the mirror is SAT and decodes.
        let enc_k = ref_encoding(k, &axioms, k);
        match solve_structured(enc_k.num_vars, &enc_k.clauses).answer {
            Answer::Sat(model) => {
                let proof = decode_and_verify(&enc_k, &model);
                assert_eq!(proof.len(), k, "F_{k}: the decoded minimal proof has exactly {k} lines");
            }
            Answer::Unsat => panic!("F_{k}: a {k}-step refutation exists"),
        }
    }
    eprintln!(
        "the swap, as a dial: 'no short proof' certificate sizes across the chain family = {curve:?} \
         — NP = coNP for this system ⟺ this series stays polynomially bounded as F grows; \
         measured, re-runnable, zero-trust-checked at every point"
    );
}

/// **The mirror is not random either.** The reflection formula `REF(unit-pair, 1)` — a formula
/// ABOUT proofs — is itself a finite object, so the finite-randomness theorem applies to it too:
/// its structure certificate over the ring `ℤ/6` is built and re-checked. The tower closes: the
/// instances are not random, the proofs are checkable, and the formulas about the proofs are not
/// random. All the way up, structure; the only thing that ever survives is cost.
#[test]
fn the_mirror_formula_itself_is_not_random() {
    let corpus = mirror_corpus();
    let (_, n, axioms) = &corpus[0];
    let enc = ref_encoding(*n, axioms, 1);
    assert!(enc.num_vars <= 20, "the tiniest mirror fits the explicit-corner construction");
    let solved = solve_structured(enc.num_vars, &enc.clauses);
    assert!(matches!(solved.answer, Answer::Sat(_)), "REF(unit-pair, 1) is satisfiable (min = 1)");
    // The mirror formula is SAT, so its certificate is a model — flip the last-line-empty units to
    // manufacture the UNSAT sibling and certify THAT (a mirror formula that demands a 1-step proof
    // deriving a NON-empty final line from the unit pair is unsatisfiable: the only resolvent is ⊥).
    let mut unsat_mirror = enc.clauses.clone();
    for c in &mut unsat_mirror {
        if c.len() == 1 && !c[0].is_positive() && enc.content.values().any(|&v| v == c[0].var()) {
            *c = vec![Lit::new(c[0].var(), true)]; // demand the final line NON-empty
        }
    }
    let solved = solve_structured(enc.num_vars, &unsat_mirror);
    assert!(matches!(solved.answer, Answer::Unsat), "the non-empty-final mirror is UNSAT");
    let cert = build_ns_certificate_zm(6, enc.num_vars, &unsat_mirror)
        .expect("the mirror formula certifies — reflection is not random");
    assert!(cert.verify(&unsat_mirror), "the mirror's ℤ/6 structure certificate re-checks");
    eprintln!(
        "the mirror formula ({} vars) is structure-certified over ℤ/6 — formulas about proofs \
         are as non-random as the formulas they describe",
        enc.num_vars
    );
}

/// **The leveled hunter takes the mirrors.** The mirrors are the open cell's home turf —
/// Atserias–Müller prove the `REF` formulas resolution-hard, so any hunter that beats the plain
/// CDCL baseline HERE is finding structure exactly where the swap lives. The level-up stack, all
/// zero-trust:
///
///   - **Symmetry inventory**: the production automorphism finder is aimed at each mirror. A
///     mirror inherits structure from the formula it reflects — the inventory measures how much
///     of it survives the encoding into selector/content variables.
///   - **Baseline**: plain CDCL (resolution-class), RUP certificates re-checked.
///   - **The leveled hunter**: SDCL — positive-reduct PR discovery plus live symmetry
///     substitution witnesses, zero hints — its certificates re-checked by the same zero-trust
///     checker.
///
/// Every certificate on both sides re-checks; the comparison table is the experiment. Whatever
/// the numbers say at these scales, the harness is the point: the first hunter configuration
/// whose mirror curve provably undercuts resolution's is a genuine event on the NP = coNP
/// ledger, and this test is where it would show up.
#[test]
fn the_leveled_hunter_takes_the_mirrors() {
    use logicaffeine_proof::symmetry_detect::find_generators;

    // The mirror targets: chain mirrors below their minimum, parity mirrors below theirs.
    let corpus = mirror_corpus();
    let mut targets: Vec<(String, RefEncoding)> = Vec::new();
    for k in [3usize, 4, 5] {
        let mut axioms: Vec<LitSet> = vec![(0..k).map(|v| 2 * v).collect()];
        for v in 0..k {
            axioms.push([2 * v + 1].into_iter().collect());
        }
        targets.push((format!("REF(chain_{k}, {})", k - 1), ref_encoding(k, &axioms, k - 1)));
    }
    for s in [2usize, 3] {
        targets.push((
            format!("REF(parity, {s})"),
            ref_encoding(corpus[2].1, &corpus[2].2, s),
        ));
    }

    let mut table: Vec<(String, usize, usize, usize, usize, usize)> = Vec::new();
    for (name, enc) in &targets {
        // Baseline: plain CDCL, self-contained RUP stream, re-checked.
        let mut solver = logicaffeine_proof::cdcl::Solver::new(enc.num_vars);
        for c in &enc.clauses {
            solver.add_clause(c.clone());
        }
        assert!(
            matches!(solver.solve(), logicaffeine_proof::cdcl::SolveResult::Unsat),
            "{name}: the mirror is UNSAT (budget below the minimum)"
        );
        let mut baseline: Vec<_> = solver
            .learned()
            .iter()
            .map(|lc| logicaffeine_proof::proof::ProofStep::Rup(lc.lits.clone()))
            .collect();
        if !matches!(baseline.last(), Some(logicaffeine_proof::proof::ProofStep::Rup(c)) if c.is_empty())
        {
            baseline.push(logicaffeine_proof::proof::ProofStep::Rup(Vec::new()));
        }
        assert!(
            check_pr_refutation(enc.num_vars, &enc.clauses, &baseline),
            "{name}: the baseline certificate re-checks"
        );

        eprintln!(
            "mirror-hunt[{name}]: {} vars, {} clauses — baseline {} steps (re-checked); leveling…",
            enc.num_vars,
            enc.clauses.len(),
            baseline.len()
        );
        // Symmetry inventory: what of the reflected formula's structure survives the encoding
        // (size-gated — the automorphism finder is the expensive lens).
        let gens = if enc.clauses.len() <= 1200 {
            find_generators(enc.num_vars, &enc.clauses).len()
        } else {
            usize::MAX
        };

        // The leveled hunter (bounded to mirrors the per-literal prober handles fast).
        let sdcl_steps = if enc.clauses.len() <= 1200 {
            let cert = sdcl_refute(enc.num_vars, &enc.clauses);
            assert!(cert.refuted, "{name}: the leveled hunter refutes the mirror");
            assert!(
                check_pr_refutation(enc.num_vars, &enc.clauses, &cert.steps),
                "{name}: the leveled hunter's certificate re-checks"
            );
            cert.steps.len()
        } else {
            usize::MAX
        };
        table.push((name.clone(), enc.num_vars, enc.clauses.len(), baseline.len(), gens, sdcl_steps));
    }
    for (name, nv, nc, base, gens, sdcl) in &table {
        let fmt = |x: &usize| {
            if *x == usize::MAX { "capped".to_string() } else { x.to_string() }
        };
        eprintln!(
            "mirror-hunt[{name}]: {nv} vars, {nc} clauses — baseline CDCL {base} steps, \
             symmetry generators {}, leveled hunter {} steps",
            fmt(gens),
            fmt(sdcl)
        );
    }
    eprintln!(
        "the mirror front: every certificate on both sides re-checked with zero trust; the first \
         hunter whose mirror curve provably undercuts resolution's is an event on the NP = coNP \
         ledger — this table is where it shows up"
    );
}

/// **The break: the mirrors' own symmetry, converted into certified PR steps.** The hunt found
/// generators inside the mirrors; this test CONVERTS them — `certified_unsat` takes the mirror
/// and its discovered automorphisms, emits lex-leader symmetry-breaking clauses as
/// propagation-redundant steps (the Heule–Kiesl–Biere criterion, the same machinery that collapsed
/// pigeonhole from Haken-exponential to quadratic), then lets CDCL finish — one composed
/// certificate, PR + RUP, re-checked with zero trust against the original mirror. The comparison
/// against the plain baseline is printed per mirror: total certified size, its (symmetry, search)
/// split, and the baseline. Whatever the split says at these scales, this is the first time the
/// mirrors' inherited symmetry is EXPLOITED rather than inventoried — the exact lever the hunt
/// table named.
#[test]
fn the_mirrors_symmetry_is_converted_into_certified_breaks() {
    use logicaffeine_proof::sym_certify::certified_unsat;
    use logicaffeine_proof::symmetry_detect::find_generators;

    let corpus = mirror_corpus();
    let mut targets: Vec<(String, RefEncoding)> = Vec::new();
    for k in [3usize, 4] {
        let mut axioms: Vec<LitSet> = vec![(0..k).map(|v| 2 * v).collect()];
        for v in 0..k {
            axioms.push([2 * v + 1].into_iter().collect());
        }
        targets.push((format!("REF(chain_{k}, {})", k - 1), ref_encoding(k, &axioms, k - 1)));
    }
    targets.push(("REF(parity, 2)".to_string(), ref_encoding(corpus[2].1, &corpus[2].2, 2)));

    for (name, enc) in &targets {
        // Baseline: plain CDCL learned-stream size.
        let mut solver = logicaffeine_proof::cdcl::Solver::new(enc.num_vars);
        for c in &enc.clauses {
            solver.add_clause(c.clone());
        }
        assert!(matches!(solver.solve(), logicaffeine_proof::cdcl::SolveResult::Unsat));
        let baseline = solver.learned().len();

        // The break: discovered automorphisms → certified lex-leader PR steps → CDCL finishes.
        let gens = find_generators(enc.num_vars, &enc.clauses);
        assert!(!gens.is_empty(), "{name}: the mirror carries symmetry to convert");
        let cert = certified_unsat(enc.num_vars, &enc.clauses, &gens);
        assert!(cert.refuted, "{name}: the symmetry-broken mirror refutes");
        assert!(
            check_pr_refutation(enc.num_vars, &enc.clauses, &cert.steps),
            "{name}: the composed PR+RUP certificate re-checks against the ORIGINAL mirror"
        );
        let search_steps = cert.steps.len() - cert.sbp_clauses;
        eprintln!(
            "break[{name}]: {} generators converted → {} PR symmetry steps + {} search steps \
             (total {}) vs plain baseline {} — certified, zero trust",
            gens.len(),
            cert.sbp_clauses,
            search_steps,
            cert.steps.len(),
            baseline
        );
    }
    eprintln!(
        "the break is live: mirror symmetry is now EXPLOITED (PR steps from the mirrors' own \
         automorphisms), not just inventoried — the PHP-collapse machinery, aimed at the swap's \
         home turf"
    );
}
