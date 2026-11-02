//! The NbE conversion engine agrees with the classic one, question for question.
//!
//! Every accepted Arena file is loaded into a kernel; then, for each definition and theorem, both
//! engines are asked whether the value's inferred type is its declared type (always yes), and
//! whether its type is the next declaration's type (usually no). Any disagreement is a bug in one
//! of them, reported with the declaration.
//!
//! Beside the sweeps, single questions pin what the answer depends on: the `eagerReduce` mode a
//! question is asked in, and the order in which two rigid applications' arguments are compared.
#![cfg(feature = "nbe")]

mod common;

use logicaffeine_kernel::dag::check::{Decl, DefnDecl, Header};
use logicaffeine_kernel::dag::env::{Hint, Kind, Safety};
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::ExprId;
use logicaffeine_kernel::dag::level::LevelsId;
use logicaffeine_kernel::dag::nat::Nat;
use logicaffeine_kernel::dag::nbe;
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/arena_corpus")
}

fn accepted_files() -> Vec<PathBuf> {
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(corpus().join("manifest.json")).unwrap())
            .unwrap();
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["location"] == "vendored" && c["outcome"] == "accept")
        .filter(|c| {
            !c["name"].as_str().unwrap().starts_with("perf/magma") && c["name"] != "perf/app-lam"
        })
        .map(|c| corpus().join(c["path"].as_str().unwrap()))
        .collect()
}

#[derive(Default, Clone, Copy)]
struct Tally {
    asked: u64,
    both_true: u64,
    both_false: u64,
}

struct Disagreement {
    file: String,
    decl: String,
    question: &'static str,
    classic: String,
    nbe: String,
}

fn check_file(path: PathBuf) -> (Vec<Disagreement>, Tally) {
    let mut k = Kernel::new();
    Reader::new(&mut k)
        .run(BufReader::new(std::fs::File::open(&path).unwrap()))
        .expect("file is accepted");
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let mut decls: Vec<_> = (0..k.s.names.len() as u32)
        .filter_map(|i| {
            k.env
                .get(logicaffeine_kernel::dag::name::NameId(i))
                .cloned()
        })
        .collect();
    decls.sort_by_key(|c| c.name.0);
    let mut out = Vec::new();
    let mut tally = Tally::default();
    for (i, c) in decls.iter().enumerate() {
        let value = match c.kind {
            Kind::Defn { value, .. } | Kind::Thm { value } => value,
            _ => continue,
        };
        let name = k.s.names.display(c.name);
        let next_ty = decls
            .get(i + 1)
            .filter(|n| n.lparams == c.lparams)
            .map(|n| n.ty);
        k.begin_decl(c.lparams.clone(), Safety::Unsafe);
        let vt = k.infer(value, true);
        let mut st = nbe::State::new();
        if let Ok(vt) = vt {
            let classic = k.is_def_eq(vt, c.ty).map_err(|e| e.to_string());
            let fast = nbe::is_def_eq(&mut k, &mut st, vt, c.ty).map_err(|e| e.to_string());
            count(&mut tally, &classic, &fast);
            if format!("{classic:?}") != format!("{fast:?}") {
                out.push(Disagreement {
                    file: file.clone(),
                    decl: name.clone(),
                    question: "type(value) ≡ declared type",
                    classic: format!("{classic:?}"),
                    nbe: format!("{fast:?}"),
                });
            }
        }
        if let Some(nt) = next_ty {
            let classic = k.is_def_eq(c.ty, nt).map_err(|e| e.to_string());
            let fast = nbe::is_def_eq(&mut k, &mut st, c.ty, nt).map_err(|e| e.to_string());
            count(&mut tally, &classic, &fast);
            if format!("{classic:?}") != format!("{fast:?}") {
                out.push(Disagreement {
                    file: file.clone(),
                    decl: name,
                    question: "type ≡ next declaration's type",
                    classic: format!("{classic:?}"),
                    nbe: format!("{fast:?}"),
                });
            }
        }
        k.end_decl();
    }
    (out, tally)
}

fn count(t: &mut Tally, classic: &Result<bool, String>, fast: &Result<bool, String>) {
    t.asked += 1;
    match (classic, fast) {
        (Ok(true), Ok(true)) => t.both_true += 1,
        (Ok(false), Ok(false)) => t.both_false += 1,
        _ => {}
    }
}

#[test]
fn nbe_conversion_agrees_with_classic_conversion_on_the_arena_corpus() {
    let mut all = Vec::new();
    let mut total = Tally::default();
    for f in accepted_files() {
        let (d, t) = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || check_file(f))
            .unwrap()
            .join()
            .unwrap();
        all.extend(d);
        total.asked += t.asked;
        total.both_true += t.both_true;
        total.both_false += t.both_false;
    }
    eprintln!(
        "asked {} questions: {} both true, {} both false",
        total.asked, total.both_true, total.both_false
    );
    assert!(
        total.asked > 2_500,
        "the differential asked only {} questions",
        total.asked
    );
    assert!(
        total.both_false > 500,
        "too few genuinely-false questions ({}) to test refutation",
        total.both_false
    );
    let report: Vec<String> = all
        .iter()
        .take(40)
        .map(|d| {
            format!(
                "{} :: {} :: {} — classic {} / nbe {}",
                d.file, d.decl, d.question, d.classic, d.nbe
            )
        })
        .collect();
    assert!(
        all.is_empty(),
        "{} disagreements:\n{}",
        all.len(),
        report.join("\n")
    );
}

/// The same differential over the whole of Lean's `Init` (≈53k declarations).
#[test]
#[ignore = "heavy: minutes; run with the ignored set"]
fn nbe_conversion_agrees_with_classic_conversion_on_init() {
    if !common::require_tool("reference-kernels") {
        return;
    }
    let init = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference-kernels/lean-kernel-arena/_build/tests/init.ndjson");
    assert!(
        init.exists(),
        "the regenerated Arena `init` export is required: {}",
        init.display()
    );
    let (d, t) = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || check_file(init))
        .unwrap()
        .join()
        .unwrap();
    eprintln!(
        "asked {} questions: {} both true, {} both false",
        t.asked, t.both_true, t.both_false
    );
    assert!(
        t.asked > 50_000,
        "the differential asked only {} questions",
        t.asked
    );
    let report: Vec<String> = d
        .iter()
        .take(40)
        .map(|d| {
            format!(
                "{} :: {} :: {} — classic {} / nbe {}",
                d.file, d.decl, d.question, d.classic, d.nbe
            )
        })
        .collect();
    assert!(
        d.is_empty(),
        "{} disagreements:\n{}",
        d.len(),
        report.join("\n")
    );
}

/// Runs `f` on a thread with the stack the checker threads get.
fn on_checker_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// A kernel holding every declaration of `file` the checker admits, inside a fresh declaration
/// scope with no universe parameters.
fn kernel_of(file: &str) -> Kernel {
    let mut k = Kernel::new();
    let _ = Reader::new(&mut k).run(BufReader::new(
        std::fs::File::open(corpus().join(file)).unwrap(),
    ));
    k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
    k
}

fn constant(k: &mut Kernel, name: &str) -> ExprId {
    let n = k.s.names.dotted(name);
    assert!(k.env.get(n).is_some(), "'{name}' is admitted");
    k.s.cnst(n, LevelsId::EMPTY)
}

/// The gadget of Arena `bugs/rec-of-subst-prop`: `Nat.add (modeG p) 0 ≡ Nat.add (modeG modeQ) 0`
/// for a variable `p : ModeP`, where `ModeP := Acc modeRel ()` is a proposition and `modeG`
/// eliminates it with `Acc.rec`. Without `eagerReduce`, Lean compares the two `Nat.add`
/// applications argument by argument, and `p ≡ modeQ` by proof irrelevance: equal. Under
/// `eagerReduce` it computes the closed side to `0` first, and the open side stays a stuck
/// `Acc.rec`: not equal.
fn eager_gadget() -> (Kernel, ExprId, ExprId) {
    let mut k = kernel_of("bad/bugs/rec-of-subst-prop.ndjson");
    let mode_p = constant(&mut k, "Issue.ModeP");
    let mode_g = constant(&mut k, "Issue.modeG");
    let mode_q = constant(&mut k, "Issue.modeQ");
    let add = constant(&mut k, "Nat.add");
    let zero = k.s.nat_lit(Nat::zero());
    let p = k.push_local(mode_p, None);
    let open = k.s.app(mode_g, p);
    let closed = k.s.app(mode_g, mode_q);
    let t = k.s.mk_app_n(add, &[open, zero]);
    let s = k.s.mk_app_n(add, &[closed, zero]);
    (k, t, s)
}

#[test]
fn the_eager_gadget_is_equal_without_eager_reduction_and_unequal_under_it() {
    on_checker_stack(|| {
        let (mut k, t, s) = eager_gadget();
        assert_eq!(
            k.is_def_eq(t, s).unwrap(),
            true,
            "the classic engine, without eagerReduce"
        );
        let mut weak = nbe::State::new();
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut weak, t, s).unwrap(),
            true,
            "NbE without eagerReduce"
        );
        let mut eager = nbe::State::new();
        eager.eager = true;
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut eager, t, s).unwrap(),
            false,
            "NbE under eagerReduce"
        );
    })
}

/// A refutation is an answer only in the mode that reached it: Lean keeps no refuted pairs, and
/// the same pair asked without `eagerReduce` is equal.
#[test]
fn a_refutation_under_eager_reduction_is_not_an_answer_without_it() {
    on_checker_stack(|| {
        let (mut k, t, s) = eager_gadget();
        let mut st = nbe::State::new();
        st.eager = true;
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).unwrap(),
            false,
            "under eagerReduce"
        );
        st.eager = false;
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).unwrap(),
            true,
            "the same pair, asked afterwards without it"
        );
    })
}

/// A proof is shared by every later question, whatever its mode, as Lean's equivalence manager
/// shares it: the pair proven without `eagerReduce` stays equal under it.
#[test]
fn a_proof_is_shared_by_both_reduction_modes() {
    on_checker_stack(|| {
        let (mut k, t, s) = eager_gadget();
        let mut st = nbe::State::new();
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).unwrap(),
            true,
            "without eagerReduce"
        );
        st.eager = true;
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).unwrap(),
            true,
            "the same pair, asked afterwards under it"
        );
    })
}

/// `Nat.succ` applied `n` times to `x`.
fn succ_chain(k: &mut Kernel, n: u32, x: ExprId) -> ExprId {
    let succ = constant(k, "Nat.succ");
    (0..n).fold(x, |e, _| k.s.app(succ, e))
}

/// Two rigid applications of one head are compared argument by argument from the first, as
/// Lean's `is_def_eq_app` compares them: `@Eq Nat 0 a ≡ @Eq Nat 1 b` is refuted by `0 ≢ 1`,
/// without ever comparing `a` with `b` — here two chains of `Nat.succ` deeper than the checker's
/// recursion bound, which no order but the first-to-last one can compare.
#[test]
fn rigid_arguments_are_compared_from_the_first() {
    on_checker_stack(|| {
        let mut k = kernel_of("good/init-prelude.ndjson");
        let nat = constant(&mut k, "Nat");
        let eq = k.s.names.dotted("Eq");
        let one_level =
            k.s.levels
                .succ(logicaffeine_kernel::dag::level::LevelId::ZERO);
        let eq = k.s.cnst_levels(eq, &[one_level]);
        let x = k.push_local(nat, None);
        let y = k.push_local(nat, None);
        let a = succ_chain(&mut k, 250_000, x);
        let b = succ_chain(&mut k, 250_000, y);
        let zero = k.s.nat_lit(Nat::zero());
        let one = k.s.nat_lit(Nat::from_u64(1));
        let t = k.s.mk_app_n(eq, &[nat, zero, a]);
        let s = k.s.mk_app_n(eq, &[nat, one, b]);
        assert_eq!(k.is_def_eq(t, s).unwrap(), false, "the classic engine");
        let mut st = nbe::State::new();
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).map_err(|e| e.to_string()),
            Ok(false),
            "NbE"
        );
    })
}

/// A question deeper than both engines' recursion bounds is declined by both, in the same words:
/// the bound is one resource with one name, whichever engine reaches it. The literal `300000`
/// against `Nat.succ` applied 300 000 times to a variable is decided one successor at a time.
#[test]
fn a_question_beyond_the_recursion_bound_is_declined_alike() {
    on_checker_stack(|| {
        let mut k = kernel_of("good/init-prelude.ndjson");
        let nat = constant(&mut k, "Nat");
        let x = k.push_local(nat, None);
        let a = k.s.nat_lit(Nat::from_u64(300_000));
        let b = succ_chain(&mut k, 300_000, x);
        let classic = k.is_def_eq(a, b).map_err(|e| e.to_string());
        let mut st = nbe::State::new();
        let fast = nbe::is_def_eq(&mut k, &mut st, a, b).map_err(|e| e.to_string());
        assert!(classic.is_err(), "the classic engine declines: {classic:?}");
        assert_eq!(fast, classic, "NbE declines in the same words");
    })
}

/// A proven pair answers every pair built from it, as the classic engine's equivalence manager
/// answers them: two closures of one expression over values proven equal are equal, without their
/// bodies being compared. Here `x + 0 ≡ x` is proven first; then `let y := x + 0; fun _ => s y`
/// and `let y := x; fun _ => s y` are compared, `s` being `Nat.succ` applied 300 000 times — a body
/// beyond the recursion bound, which only congruence over the proven pair can decide.
#[test]
fn closures_over_proven_equal_values_are_equal_without_comparing_their_bodies() {
    on_checker_stack(|| {
        let mut k = kernel_of("good/init-prelude.ndjson");
        let nat = constant(&mut k, "Nat");
        let add = constant(&mut k, "Nat.add");
        let x = k.push_local(nat, None);
        let zero = k.s.nat_lit(Nat::zero());
        let a = k.s.mk_app_n(add, &[x, zero]);
        let y = k.s.bvar(1);
        let body = succ_chain(&mut k, 300_000, y);
        let lam = k.s.lam(nat, body);
        let t = k.s.let_(nat, a, lam);
        let s = k.s.let_(nat, x, lam);
        let mut st = nbe::State::new();
        assert_eq!(
            k.is_def_eq(a, x).unwrap(),
            true,
            "the classic engine proves x + 0 ≡ x"
        );
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, a, x).unwrap(),
            true,
            "NbE proves x + 0 ≡ x"
        );
        assert_eq!(
            k.is_def_eq(t, s).map_err(|e| e.to_string()),
            Ok(true),
            "the classic engine, by congruence"
        );
        assert_eq!(
            nbe::is_def_eq(&mut k, &mut st, t, s).map_err(|e| e.to_string()),
            Ok(true),
            "NbE, by congruence"
        );
    })
}

/// The conversion steps NbE takes to refute `Nat.sub x n ≡ Nat.sub y n` in a fresh state.
fn steps_to_refute_sub(k: &mut Kernel, n: u64) -> u64 {
    let nat = constant(k, "Nat");
    let sub = constant(k, "Nat.sub");
    let x = k.push_local(nat, None);
    let y = k.push_local(nat, None);
    let lit = k.s.nat_lit(Nat::from_u64(n));
    let t = k.s.mk_app_n(sub, &[x, lit]);
    let s = k.s.mk_app_n(sub, &[y, lit]);
    let mut st = nbe::State::new();
    let r = nbe::is_def_eq(k, &mut st, t, s).map_err(|e| e.to_string());
    assert_eq!(r, Ok(false), "Nat.sub x {n} ≢ Nat.sub y {n}");
    st.conversion_steps()
}

/// `Nat.sub x k ≡ Nat.sub y k` unfolds `Nat.sub` `k` times, stuck at every level on `x` against
/// `y`, and every level's refutation is reached again from the levels above it: remembered, the
/// comparison is linear in `k`; refuted afresh each time, quadratic (see `dag_conversion_scaling`,
/// the same family for the classic engine). Measured in conversion steps: the values are shared
/// either way, so what grows is the comparing.
#[test]
fn nbe_refutes_a_stuck_literal_recursion_in_linear_work() {
    on_checker_stack(|| {
        let mut k = kernel_of("good/init-prelude.ndjson");
        let [a, b, c] = [100, 200, 400].map(|n| steps_to_refute_sub(&mut k, n));
        eprintln!("conversion steps: k=100 {a}, k=200 {b}, k=400 {c}");
        assert!(a < b && b < c, "a larger literal costs more: {a}, {b}, {c}");
        assert!(c - b <= 3 * (b - a), "superlinear: {a}, {b}, {c}");
    })
}

/// `dag_conversion_scaling`'s pair for NbE: `Nat.add (modeG q) 0 ≢ Nat.add (modeG modeQ) 0` as
/// computed first (`q` an axiom of `ModeP`), `modeG q ≡ modeG modeQ` proven next, and then the
/// first pair equal by congruence — the refutation remembered from the first time does not
/// answer it, and both engines agree on all three.
#[test]
fn a_proof_reached_since_answers_a_remembered_refutation_in_both_engines() {
    on_checker_stack(|| {
        let mut k = Kernel::new();
        let _ = Reader::new(&mut k).run(BufReader::new(
            std::fs::File::open(corpus().join("bad/bugs/rec-of-subst-prop.ndjson")).unwrap(),
        ));
        let mode_p = constant(&mut k, "Issue.ModeP");
        let name = k.s.names.dotted("Issue.stuckMode");
        k.add_decl(Decl::Axiom {
            header: Header {
                name,
                lparams: Rc::from(Vec::new()),
                ty: mode_p,
            },
            is_unsafe: false,
        })
        .expect("an axiom of a proposition");
        k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
        let mode_g = constant(&mut k, "Issue.modeG");
        let mode_q = constant(&mut k, "Issue.modeQ");
        let stuck = constant(&mut k, "Issue.stuckMode");
        let add = constant(&mut k, "Nat.add");
        let zero = k.s.nat_lit(Nat::zero());
        let open = k.s.app(mode_g, stuck);
        let closed = k.s.app(mode_g, mode_q);
        let t = k.s.mk_app_n(add, &[open, zero]);
        let s = k.s.mk_app_n(add, &[closed, zero]);
        let mut st = nbe::State::new();
        for (a, b, expected) in [(t, s, false), (open, closed, true), (t, s, true)] {
            assert_eq!(k.is_def_eq(a, b).unwrap(), expected, "the classic engine");
            assert_eq!(
                nbe::is_def_eq(&mut k, &mut st, a, b).unwrap(),
                expected,
                "NbE"
            );
        }
    })
}

/// `dag_conversion_scaling`'s refutation that rested on a proof's absence, in both engines:
/// `viaH (Nat.add (modeG q) 0) ≢ viaK (Nat.add (modeG modeQ) 0)` as computed first (`viaH` and
/// `viaK` two definitions with one unfolding, `idNat`), `modeG q ≡ modeG modeQ` proven next, and
/// then the first pair equal — beneath the unfolding, where no congruence at the top reaches.
#[test]
fn a_proof_reached_since_retires_a_refutation_that_rested_on_its_absence_in_both_engines() {
    on_checker_stack(|| {
        let mut k = Kernel::new();
        let _ = Reader::new(&mut k).run(BufReader::new(
            std::fs::File::open(corpus().join("bad/bugs/rec-of-subst-prop.ndjson")).unwrap(),
        ));
        let mode_p = constant(&mut k, "Issue.ModeP");
        let name = k.s.names.dotted("Issue.stuckMode");
        k.add_decl(Decl::Axiom {
            header: Header {
                name,
                lparams: Rc::from(Vec::new()),
                ty: mode_p,
            },
            is_unsafe: false,
        })
        .expect("an axiom of a proposition");
        let nat = constant(&mut k, "Nat");
        let nat_to_nat = k.s.pi(nat, nat);
        let x = k.s.bvar(0);
        let define = |k: &mut Kernel, n: &str, value: ExprId, height: u32| {
            let name = k.s.names.dotted(n);
            let header = Header {
                name,
                lparams: Rc::from(Vec::new()),
                ty: nat_to_nat,
            };
            k.add_decl(Decl::Defn(DefnDecl {
                header,
                value,
                hint: Hint::Regular(height),
                safety: Safety::Safe,
            }))
            .expect("a definition over Nat");
        };
        let id = k.s.lam(nat, x);
        define(&mut k, "Issue.idNat", id, 1);
        let id_nat = constant(&mut k, "Issue.idNat");
        let body = k.s.app(id_nat, x);
        let via = k.s.lam(nat, body);
        define(&mut k, "Issue.viaH", via, 2);
        define(&mut k, "Issue.viaK", via, 2);
        k.begin_decl(Rc::from(Vec::new()), Safety::Safe);
        let [mode_g, mode_q, stuck, add, via_h, via_k] = [
            "Issue.modeG",
            "Issue.modeQ",
            "Issue.stuckMode",
            "Nat.add",
            "Issue.viaH",
            "Issue.viaK",
        ]
        .map(|n| constant(&mut k, n));
        let zero = k.s.nat_lit(Nat::zero());
        let open = k.s.app(mode_g, stuck);
        let closed = k.s.app(mode_g, mode_q);
        let t = k.s.mk_app_n(add, &[open, zero]);
        let s = k.s.mk_app_n(add, &[closed, zero]);
        let ht = k.s.app(via_h, t);
        let ks = k.s.app(via_k, s);
        let mut st = nbe::State::new();
        for (a, b, expected) in [(ht, ks, false), (open, closed, true), (ht, ks, true)] {
            assert_eq!(k.is_def_eq(a, b).unwrap(), expected, "the classic engine");
            assert_eq!(
                nbe::is_def_eq(&mut k, &mut st, a, b).unwrap(),
                expected,
                "NbE"
            );
        }
    })
}
