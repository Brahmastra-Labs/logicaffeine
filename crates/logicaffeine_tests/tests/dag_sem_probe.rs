//! Speculation in the value engine's conversion, and what bounds it.
//!
//! Two applications of one definition are first compared argument by argument, before either is
//! unfolded — cheap when the arguments are equal, arbitrarily expensive when they differ deep
//! inside. Each argument pair is given a budget of conversion steps (`Kernel::probe_steps`); a
//! pair that runs out abandons the probe and the definitions are unfolded, which decides. So the
//! budget may change what conversion costs, never what it answers: with a budget of zero every
//! probe is abandoned, and every Arena verdict must stand.
//!
//! Conversion must also not redo work exponentially: two applications of one constructor that
//! differ at one leaf of a deep nest are refuted in steps linear in the depth.

mod common;

use logicaffeine_kernel::dag::env::Safety;
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::level::LevelId;
use logicaffeine_kernel::dag::sem::value::{EnvId, V};
use logicaffeine_kernel::dag::sem::Sem;
use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_with, Kernel, Verdict};
use std::io::BufReader;
use std::path::PathBuf;
use std::rc::Rc;

enum Expect {
    Accept,
    Reject,
    Either,
}

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path)
}

fn check(file: PathBuf, steps: u32) -> Verdict {
    std::thread::Builder::new()
        .stack_size(8 << 30)
        .spawn(move || {
            let input = std::fs::File::open(file).expect("Arena case");
            check_export_with(BufReader::with_capacity(1 << 20, input), |k| {
                k.engine = Engine::Sem;
                k.probe_steps = steps;
            })
            .0
        })
        .unwrap()
        .join()
        .expect("checker thread panicked")
}

fn run_file(file: PathBuf, path: &'static str, expect: Expect) {
    for steps in [0, 1] {
        match (&expect, check(file.clone(), steps)) {
            (Expect::Accept, Verdict::Accept) | (Expect::Reject, Verdict::Reject(_)) => {}
            (Expect::Either, Verdict::Accept | Verdict::Reject(_)) => {}
            (Expect::Accept, Verdict::Reject(e)) => {
                panic!("{path} (probe budget {steps}): a valid proof was REJECTED: {e}")
            }
            (Expect::Reject, Verdict::Accept) => {
                panic!("{path} (probe budget {steps}): an invalid proof was ACCEPTED")
            }
            (_, Verdict::Decline(e)) => panic!("{path} (probe budget {steps}): declined: {e}"),
        }
    }
}

fn run(path: &'static str, expect: Expect) {
    run_file(fixture(path), path, expect)
}

fn run_heavy(path: &'static str, expect: Expect) {
    if !common::require_tool("reference-kernels") {
        return;
    }
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    assert!(full.exists(), "heavy Arena input {path} is missing");
    run_file(full, path, expect)
}

macro_rules! arena_case {
    ($(#[$m:meta])* $name:ident, $path:expr, $expect:ident) => {
        #[test]
        $(#[$m])*
        fn $name() {
            run($path, Expect::$expect)
        }
    };
}

macro_rules! arena_heavy_case {
    ($(#[$m:meta])* $name:ident, $path:expr, $expect:ident) => {
        #[test]
        $(#[$m])*
        fn $name() {
            run_heavy($path, Expect::$expect)
        }
    };
}

include!("arena_corpus/cases.rs");

fn load(path: &str) -> Kernel {
    let mut k = Kernel::new();
    Reader::new(&mut k)
        .run(BufReader::new(std::fs::File::open(fixture(path)).unwrap()))
        .expect("the case is accepted");
    k
}

fn konst(sem: &mut Sem, dotted: &str, ls: &[u64]) -> V {
    let n = sem.k.s.names.dotted(dotted);
    let ls: Vec<LevelId> = ls.iter().map(|l| sem.k.s.levels.of_nat(*l)).collect();
    let e = sem.k.s.cnst_levels(n, &ls);
    sem.eval(EnvId::EMPTY, e).unwrap()
}

fn app(sem: &mut Sem, f: V, args: &[V]) -> V {
    args.iter().fold(f, |f, a| sem.apply(f, *a).unwrap())
}

/// `⟨⟨…⟨leaf, x⟩…, x⟩, x⟩ : PProd (… PProd Nat Nat …) Nat`, `depth` constructors deep.
fn nest(sem: &mut Sem, leaf: V, x: V, depth: usize) -> V {
    let (nat, pprod, mk) = (
        konst(sem, "Nat", &[]),
        konst(sem, "PProd", &[1, 1]),
        konst(sem, "PProd.mk", &[1, 1]),
    );
    let (mut v, mut ty) = (leaf, nat);
    for _ in 0..depth {
        v = app(sem, mk, &[ty, nat, v, x]);
        ty = app(sem, pprod, &[ty, nat]);
    }
    v
}

#[test]
fn constructor_nests_differing_at_one_leaf_are_refuted_in_steps_linear_in_their_depth() {
    let depth = 24;
    let r = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let mut k = load("good/init-prelude.ndjson");
            k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
                k.with_sem(|sem| {
                    let nat = konst(sem, "Nat", &[]);
                    sem.under(nat, |sem, x| {
                        sem.under(nat, |sem, y| {
                            let t = nest(sem, x, x, depth);
                            let s = nest(sem, y, x, depth);
                            sem.is_def_eq_within(t, s, 64 * depth as u32)
                        })
                    })
                })
            })
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        r,
        Some(false),
        "a nest {depth} deep with one differing leaf is refuted within {} steps",
        64 * depth
    );
}

#[test]
fn equal_nests_are_equal_within_any_budget() {
    let r = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = load("good/init-prelude.ndjson");
            k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
                k.with_sem(|sem| {
                    let nat = konst(sem, "Nat", &[]);
                    sem.under(nat, |sem, x| {
                        let t = nest(sem, x, x, 8);
                        let s = nest(sem, x, x, 8);
                        sem.is_def_eq_within(t, s, 0)
                    })
                })
            })
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(r, Some(true), "one value is itself without a single step");
}

#[test]
fn a_comparison_given_fewer_steps_than_it_needs_is_abandoned_undecided() {
    let r = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut k = load("good/init-prelude.ndjson");
            k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
                k.with_sem(|sem| {
                    let nat = konst(sem, "Nat", &[]);
                    sem.under(nat, |sem, x| {
                        sem.under(nat, |sem, y| {
                            let t = nest(sem, x, x, 24);
                            let s = nest(sem, y, x, 24);
                            let starved = sem.is_def_eq_within(t, s, 2)?;
                            let decided = sem.is_def_eq_within(t, s, 64 * 24)?;
                            Ok((starved, decided))
                        })
                    })
                })
            })
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        r,
        (None, Some(false)),
        "two steps cannot refute a nest 24 deep; enough steps can, afterwards"
    );
}

/// A probe's budget bounds its work, not only its conversion steps. Refuting `id (id (… (id x)))`
/// against another variable `y` is ONE conversion step and 64 δ-unfoldings; were unfolding free, an
/// 8-step probe would decide the pair after unfolding all 64, and a probe abandoned "at its budget"
/// could have unfolded without bound (the Navier-Stokes export's `baseCommutator_bound` spent 188 G
/// instructions so, against the rival's 0.01 G).
#[test]
fn a_probe_spends_its_budget_on_unfolding_too() {
    let depth = 64;
    let r = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let mut k = load("good/init-prelude.ndjson");
            k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
                k.with_sem(|sem| {
                    let nat = konst(sem, "Nat", &[]);
                    let id = konst(sem, "id", &[1]);
                    sem.under(nat, |sem, x| {
                        sem.under(nat, |sem, y| {
                            let t = (0..depth).fold(x, |v, _| app(sem, id, &[nat, v]));
                            let starved = sem.is_def_eq_within(t, y, 8)?;
                            let decided = sem.is_def_eq_within(t, y, 4 * depth as u32)?;
                            Ok((starved, decided))
                        })
                    })
                })
            })
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        r,
        (None, Some(false)),
        "8 steps cannot unfold {depth} definitions; {} can, and refute",
        4 * depth
    );
}

/// `axiom T : Type`, `axiom a b : T`, `def f : T → T := fun x => x` (a regular definition, so
/// `f a ≡ f b` is first probed on `a ≡ b`), `axiom P : T → Prop`, `axiom h : P (f a)`, and
/// `theorem claim : P (f <arg>) := h`.
fn same_head_export(arg: &str) -> String {
    let arg = if arg == "a" { 4 } else { 5 };
    [
        r#"{"meta":{"exporter":{"name":"lean4export","version":"3.1.0"},"format":{"version":"3.1.0"},"lean":{"githash":"","version":"4.29.1"}}}"#,
        r#"{"in":1,"str":{"pre":0,"str":"T"}}"#,
        r#"{"in":2,"str":{"pre":0,"str":"a"}}"#,
        r#"{"in":3,"str":{"pre":0,"str":"b"}}"#,
        r#"{"in":4,"str":{"pre":0,"str":"f"}}"#,
        r#"{"in":5,"str":{"pre":0,"str":"P"}}"#,
        r#"{"in":6,"str":{"pre":0,"str":"h"}}"#,
        r#"{"in":7,"str":{"pre":0,"str":"claim"}}"#,
        r#"{"in":8,"str":{"pre":0,"str":"x"}}"#,
        r#"{"il":1,"succ":0}"#,
        r#"{"ie":0,"sort":1}"#,
        r#"{"ie":1,"sort":0}"#,
        r#"{"ie":2,"const":{"name":1,"us":[]}}"#,
        r#"{"ie":3,"forallE":{"binderInfo":"default","body":2,"name":8,"type":2}}"#,
        r#"{"ie":4,"const":{"name":2,"us":[]}}"#,
        r#"{"ie":5,"const":{"name":3,"us":[]}}"#,
        r#"{"ie":6,"bvar":0}"#,
        r#"{"ie":7,"lam":{"binderInfo":"default","body":6,"name":8,"type":2}}"#,
        r#"{"ie":8,"forallE":{"binderInfo":"default","body":1,"name":8,"type":2}}"#,
        r#"{"ie":9,"const":{"name":5,"us":[]}}"#,
        r#"{"ie":10,"const":{"name":4,"us":[]}}"#,
        r#"{"ie":11,"app":{"arg":4,"fn":10}}"#,
        r#"{"ie":12,"app":{"arg":11,"fn":9}}"#,
        r#"{"ie":13,"app":{"arg":5,"fn":10}}"#,
        r#"{"ie":14,"app":{"arg":13,"fn":9}}"#,
        r#"{"ie":15,"const":{"name":6,"us":[]}}"#,
        r#"{"axiom":{"isUnsafe":false,"levelParams":[],"name":1,"type":0}}"#,
        r#"{"axiom":{"isUnsafe":false,"levelParams":[],"name":2,"type":2}}"#,
        r#"{"axiom":{"isUnsafe":false,"levelParams":[],"name":3,"type":2}}"#,
        r#"{"def":{"all":[4],"hints":{"regular":1},"levelParams":[],"name":4,"safety":"safe","type":3,"value":7}}"#,
        r#"{"axiom":{"isUnsafe":false,"levelParams":[],"name":5,"type":8}}"#,
        r#"{"axiom":{"isUnsafe":false,"levelParams":[],"name":6,"type":12}}"#,
        &format!(r#"{{"thm":{{"all":[7],"levelParams":[],"name":7,"type":{},"value":15}}}}"#, if arg == 4 { 12 } else { 14 }),
    ]
    .join("\n")
        + "\n"
}

fn check_text(text: String, steps: u32) -> Verdict {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            check_export_with(
                BufReader::new(std::io::Cursor::new(text.into_bytes())),
                |k| {
                    k.engine = Engine::Sem;
                    k.probe_steps = steps;
                },
            )
            .0
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn an_abandoned_probe_never_counts_as_agreement() {
    for steps in [0, 1, 2048] {
        match check_text(same_head_export("a"), steps) {
            Verdict::Accept => {}
            v => panic!(
                "probe budget {steps}: h : P (f a) proves P (f a), got {:?}",
                verdict_text(v)
            ),
        }
        match check_text(same_head_export("b"), steps) {
            Verdict::Reject(_) => {}
            v => panic!("probe budget {steps}: f a ≢ f b for distinct axioms a b, so h cannot prove P (f b); got {:?}", verdict_text(v)),
        }
    }
}

fn verdict_text(v: Verdict) -> String {
    match v {
        Verdict::Accept => "ACCEPT".into(),
        Verdict::Reject(e) => format!("reject: {e}"),
        Verdict::Decline(e) => format!("decline: {e}"),
    }
}
