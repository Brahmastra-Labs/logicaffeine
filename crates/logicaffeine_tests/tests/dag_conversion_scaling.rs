//! What conversion costs grows with the question, not with how often its parts are asked again.
//!
//! `Nat.sub x k ≡ Nat.sub y k`, for two variables `x`, `y` and a literal `k`: `Nat.sub` recurses
//! on its second argument, so refuting the pair unfolds it `k` times, and at every level the two
//! sides are stuck on `x` against `y`. The refutation at each level is reached again from every
//! level above it; answering a pair already refuted (in the same `eagerReduce` mode) from memory
//! keeps the whole comparison linear in `k`. Refuting it afresh each time makes it quadratic — and,
//! once the terms outgrow the caches, cubic: Lean's `Init` asks it at `k = 65536` (the `Int16`
//! bounds), where that is a day.
//!
//! Measured in conversion steps, which are deterministic.
//!
//! A remembered refutation must never answer what recomputing it would answer otherwise: it is
//! an answer only in the `eagerReduce` mode that reached it, and only while nothing has been
//! proven since — a proof can make the pair equal, by congruence or through a sub-comparison.

use logicaffeine_kernel::dag::check::{Decl, DefnDecl, Header};
use logicaffeine_kernel::dag::env::{Hint, Safety};
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::ExprId;
use logicaffeine_kernel::dag::level::{LevelId, LevelsId};
use logicaffeine_kernel::dag::nat::Nat;
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

fn arena_case(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path)
}

fn prelude() -> Kernel {
    let mut k = Kernel::new();
    let input = std::fs::File::open(arena_case("good/init-prelude.ndjson")).unwrap();
    Reader::new(&mut k)
        .run(BufReader::new(input))
        .expect("the prelude is accepted");
    k
}

/// The conversion steps the classic engine takes to refute `Nat.sub x n ≡ Nat.sub y n`, in a
/// declaration of its own.
fn steps_to_refute_sub(k: &mut Kernel, n: u64) -> u64 {
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    let nat = k.s.names.dotted("Nat");
    let nat = k.s.cnst(nat, LevelsId::EMPTY);
    let sub = k.s.names.dotted("Nat.sub");
    let sub = k.s.cnst(sub, LevelsId::EMPTY);
    let x = k.push_local(nat, None);
    let y = k.push_local(nat, None);
    let lit = k.s.nat_lit(Nat::from_u64(n));
    let t = k.s.mk_app_n(sub, &[x, lit]);
    let s = k.s.mk_app_n(sub, &[y, lit]);
    let before = k.conversion_steps();
    let r = k.is_def_eq(t, s);
    let steps = k.conversion_steps() - before;
    k.end_decl();
    assert_eq!(
        r.map_err(|e| e.to_string()),
        Ok(false),
        "Nat.sub x {n} ≢ Nat.sub y {n}"
    );
    steps
}

#[test]
fn refuting_a_stuck_literal_recursion_is_linear_in_the_literal() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = prelude();
            steps_to_refute_sub(&mut k, 10);
            let [a, b, c] = [100, 200, 400].map(|n| steps_to_refute_sub(&mut k, n));
            eprintln!("conversion steps: k=100 {a}, k=200 {b}, k=400 {c}");
            assert!(a < b && b < c, "a larger literal costs more: {a}, {b}, {c}");
            assert!(
                c - b <= 3 * (b - a),
                "doubling the literal again costs more than three times what doubling it did — \
                 superlinear: {a}, {b}, {c}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Arena `bugs/rec-of-subst-prop`'s declarations, with one more: an axiom `q : ModeP`.
fn with_stuck_mode_proof() -> Kernel {
    let mut k = Kernel::new();
    let input = std::fs::File::open(arena_case("bad/bugs/rec-of-subst-prop.ndjson")).unwrap();
    let _ = Reader::new(&mut k).run(BufReader::new(input));
    let mode_p = k.s.names.dotted("Issue.ModeP");
    let ty = k.s.cnst(mode_p, LevelsId::EMPTY);
    let name = k.s.names.dotted("Issue.stuckMode");
    k.add_decl(Decl::Axiom {
        header: Header {
            name,
            lparams: Rc::from(Vec::new()),
            ty,
        },
        is_unsafe: false,
    })
    .expect("an axiom of a proposition");
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    k
}

/// `Nat.add (modeG q) 0 ≡ Nat.add (modeG modeQ) 0`, both closed, `q` an axiom of `ModeP` and
/// `modeG` eliminating `ModeP` with `Acc.rec`: the algorithm computes the right side to the
/// literal `0` and finds the left side stuck — not equal. But `modeG q ≡ modeG modeQ` holds (`q`
/// and `modeQ` are proofs of one proposition), and once it is proven the first pair is equal by
/// congruence. Asked a third time, the first pair is equal: the refutation remembered from the
/// first time does not answer it.
#[test]
fn a_proof_reached_since_answers_a_remembered_refutation() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = with_stuck_mode_proof();
            let [mode_g, mode_q, stuck, add] =
                ["Issue.modeG", "Issue.modeQ", "Issue.stuckMode", "Nat.add"].map(|n| {
                    let n = k.s.names.dotted(n);
                    k.s.cnst(n, LevelsId::EMPTY)
                });
            let zero = k.s.nat_lit(Nat::zero());
            let open = k.s.app(mode_g, stuck);
            let closed = k.s.app(mode_g, mode_q);
            let t = k.s.mk_app_n(add, &[open, zero]);
            let s = k.s.mk_app_n(add, &[closed, zero]);
            assert_eq!(
                k.is_def_eq(t, s).unwrap(),
                false,
                "computed first: the stuck side is not the literal"
            );
            assert_eq!(
                k.is_def_eq(open, closed).unwrap(),
                true,
                "argument by argument, by proof irrelevance"
            );
            assert_eq!(k.is_def_eq(t, s).unwrap(), true, "now equal by congruence");
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A refutation is an answer only in the `eagerReduce` mode that reached it. `endpointUse p h`
/// (from `bugs/rec-of-subst-prop`) wants `h : Nat.add (modeG p) 0 = Nat.add (modeG modeQ) 0`;
/// given `eagerReduce (Eq.refl (Nat.add (modeG modeQ) 0))`, its type is checked under eager
/// reduction, which computes the closed side to `0` and finds the open side stuck — the
/// application is rejected. The same pair asked afterwards, without eager reduction, is equal:
/// argument by argument, `p ≡ modeQ` by proof irrelevance.
#[test]
fn a_refutation_under_eager_reduction_is_not_an_answer_without_it() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = with_stuck_mode_proof();
            let [mode_p, mode_g, mode_q, add, nat, use_] = [
                "Issue.ModeP",
                "Issue.modeG",
                "Issue.modeQ",
                "Nat.add",
                "Nat",
                "Issue.endpointUse",
            ]
            .map(|n| {
                let n = k.s.names.dotted(n);
                k.s.cnst(n, LevelsId::EMPTY)
            });
            let zero = k.s.nat_lit(Nat::zero());
            let p = k.push_local(mode_p, None);
            let open = k.s.app(mode_g, p);
            let closed = k.s.app(mode_g, mode_q);
            let t = k.s.mk_app_n(add, &[open, zero]);
            let s = k.s.mk_app_n(add, &[closed, zero]);
            let one = k.s.levels.succ(LevelId::ZERO);
            let [eq, refl, eager] = ["Eq", "Eq.refl", "eagerReduce"].map(|n| k.s.names.dotted(n));
            let eq = k.s.cnst_levels(eq, &[one]);
            let refl = k.s.cnst_levels(refl, &[one]);
            let eager = k.s.cnst_levels(eager, &[LevelId::ZERO]);
            let s_eq_s = k.s.mk_app_n(eq, &[nat, s, s]);
            let rfl = k.s.mk_app_n(refl, &[nat, s]);
            let h = k.s.mk_app_n(eager, &[s_eq_s, rfl]);
            let e = k.s.mk_app_n(use_, &[p, h]);
            let checked = k.check(e).map_err(|e| e.to_string());
            assert!(
                checked
                    .as_ref()
                    .is_err_and(|m| m.contains("application type mismatch")),
                "under eager reduction the stuck side is not the literal: {checked:?}"
            );
            assert_eq!(
                k.is_def_eq(t, s).unwrap(),
                true,
                "without eager reduction, equal"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Adds `name : Nat → Nat := value` (a definition of height `height`) to `k`.
fn define_nat_function(k: &mut Kernel, name: &str, value: ExprId, height: u32) {
    let nat = k.s.names.dotted("Nat");
    let nat = k.s.cnst(nat, LevelsId::EMPTY);
    let ty = k.s.pi(nat, nat);
    let name = k.s.names.dotted(name);
    let header = Header {
        name,
        lparams: Rc::from(Vec::new()),
        ty,
    };
    k.add_decl(Decl::Defn(DefnDecl {
        header,
        value,
        hint: Hint::Regular(height),
        safety: Safety::Safe,
    }))
    .expect("a definition over Nat");
}

/// `with_stuck_mode_proof`, with `idNat := fun x => x` and two definitions over it, `viaH` and
/// `viaK`, both `fun x => idNat x`: different constants with one unfolding.
fn with_two_routes() -> Kernel {
    let mut k = with_stuck_mode_proof();
    k.end_decl();
    let nat = k.s.names.dotted("Nat");
    let nat = k.s.cnst(nat, LevelsId::EMPTY);
    let x = k.s.bvar(0);
    let id = k.s.lam(nat, x);
    define_nat_function(&mut k, "Issue.idNat", id, 1);
    let id_nat = k.s.names.dotted("Issue.idNat");
    let id_nat = k.s.cnst(id_nat, LevelsId::EMPTY);
    let body = k.s.app(id_nat, x);
    let via = k.s.lam(nat, body);
    define_nat_function(&mut k, "Issue.viaH", via, 2);
    define_nat_function(&mut k, "Issue.viaK", via, 2);
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    k
}

/// The refutation of `viaH (Nat.add (modeG q) 0) ≡ viaK (Nat.add (modeG modeQ) 0)` rests, after
/// both sides unfold, on the sub-comparison of the two `Nat.add` terms — refuted first (the
/// closed side computes to `0`, the other is stuck). Once `modeG q ≡ modeG modeQ` is proven that
/// sub-comparison is equal by congruence, and so the whole pair is equal; but the pair itself is
/// not congruent (`viaH` is not `viaK`). Asked a third time, the remembered refutation must not
/// answer: only re-running the comparison finds the new proof beneath the unfolding.
#[test]
fn a_proof_reached_since_retires_a_refutation_that_rested_on_its_absence() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = with_two_routes();
            let [mode_g, mode_q, stuck, add, via_h, via_k] = [
                "Issue.modeG",
                "Issue.modeQ",
                "Issue.stuckMode",
                "Nat.add",
                "Issue.viaH",
                "Issue.viaK",
            ]
            .map(|n| {
                let n = k.s.names.dotted(n);
                k.s.cnst(n, LevelsId::EMPTY)
            });
            let zero = k.s.nat_lit(Nat::zero());
            let open = k.s.app(mode_g, stuck);
            let closed = k.s.app(mode_g, mode_q);
            let t = k.s.mk_app_n(add, &[open, zero]);
            let s = k.s.mk_app_n(add, &[closed, zero]);
            let ht = k.s.app(via_h, t);
            let ks = k.s.app(via_k, s);
            assert_eq!(
                k.is_def_eq(ht, ks).unwrap(),
                false,
                "computed first: not equal"
            );
            assert_eq!(
                k.is_def_eq(open, closed).unwrap(),
                true,
                "argument by argument, by proof irrelevance"
            );
            assert_eq!(
                k.is_def_eq(ht, ks).unwrap(),
                true,
                "now equal, beneath the unfolding"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
