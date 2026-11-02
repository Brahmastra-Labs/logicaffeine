//! # THE ROOT OF UNDECIDABILITY — Cantor's diagonal, and why we can't just "decide" Hilbert's 10th
//!
//! "Undecidable" (Hilbert's 10th, the halting problem) is a *proven impossibility*, not an open question: it
//! is a theorem that NO algorithm can decide the problem — stronger than "we haven't found one." All of these
//! results — Cantor's theorem, Gödel's incompleteness, Turing's halting problem, and (by reduction)
//! Matiyasevich's theorem that Hilbert's 10th is undecidable — share ONE engine: **diagonalization /
//! self-reference**. You assume a decider exists, then build an object that differs from every case it
//! handles, contradicting it. We can *demonstrate that one move* concretely and honestly; we cannot mechanically
//! re-derive Matiyasevich's theorem (a deep result — its formal proof took others years), so the Book cites it.

#[test]
fn cantor_diagonalization_is_the_root_of_every_undecidability_theorem() {
    // A "list of programs" — total predicates ℕ → bool. Take ANY finite prefix of any such enumeration.
    let family: Vec<fn(usize) -> bool> = vec![|n| n % 2 == 0, |n| n < 3, |n| n * n > 10, |_n| true];

    // The diagonal spoiler g(i) = ¬ family[i](i). By construction it disagrees with row i AT position i, so g
    // — a perfectly good total predicate — appears in NO row. No enumeration of predicates is complete.
    let g = |i: usize| !family[i](i);
    for i in 0..family.len() {
        assert_ne!(
            g(i),
            family[i](i),
            "the diagonal escapes row {i} (differs at position {i})"
        );
    }

    // That single move — construct the object that differs from every candidate — is exactly:
    //   • Cantor:  no list enumerates all subsets of ℕ  (g ∉ the list);
    //   • Turing:  if HALT were decidable, the diagonal program halts ⟺ it loops  (contradiction);
    //   • Gödel:   the self-referential "I am not provable" is true but unprovable;
    //   • H10:     every RE set is Diophantine (Matiyasevich) ⇒ deciding integer solutions would decide
    //              halting — impossible. We cite Matiyasevich; the diagonal ROOT is what we demonstrate here.
    // The diagonal g is total and escapes every row: the shared engine of all four impossibility theorems.
    assert_eq!(
        (0..family.len()).filter(|&i| g(i) == family[i](i)).count(),
        0,
        "g escapes every row — no list is complete"
    );
}

#[test]
fn undecidable_is_not_the_same_as_open_or_hard() {
    // Three genuinely different statuses the Book must never conflate:
    //   OPEN        — nobody knows (Riemann, P vs NP): a decider MIGHT exist, we just don't know.
    //   INFEASIBLE  — a decider exists but is astronomically slow (Gröbner/EXPSPACE, #P-hard).
    //   UNDECIDABLE — PROVEN that no decider can EVER exist (halting, Hilbert's 10th).
    // The engine tags each honestly; this test just pins the distinction so nothing drifts.
    #[derive(PartialEq, Debug)]
    enum Status {
        Open,
        Infeasible,
        Undecidable,
    }
    let riemann = Status::Open; // a decider might exist
    let groebner = Status::Infeasible; // decidable, but double-exponential
    let hilbert10 = Status::Undecidable; // proven: NO algorithm exists (Matiyasevich 1970)
    assert_ne!(
        riemann, hilbert10,
        "OPEN ≠ UNDECIDABLE — 'unknown' is not 'proven impossible'"
    );
    assert_ne!(
        groebner, hilbert10,
        "INFEASIBLE ≠ UNDECIDABLE — 'slow' is not 'impossible'"
    );
    assert_eq!(
        hilbert10,
        Status::Undecidable,
        "Hilbert's 10th is proven undecidable, not merely open"
    );
}
