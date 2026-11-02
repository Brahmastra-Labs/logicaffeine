//! W_perf — the KERNEL'S NATIVE-RECURSION CEILING, measured rather than asserted.
//!
//! `lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically`
//! does not fail: it ABORTS. A stack overflow kills the process, so the corpus case can
//! only document the ceiling in prose ("past a 2 MB stack at depth 4000, into an aborted
//! process"). Prose is not a measurement, and the number in it is wrong.
//!
//! This file makes the ceiling an EXPERIMENT. Every probe runs in a CHILD PROCESS on a
//! thread of a declared stack size, so an overflow is a readable exit status instead of a
//! dead test run. Each probe is bisected to the exact depth at which it dies, which turns
//! "it overflows somewhere" into a table you can act on: which walk is the binding
//! constraint, by how much, and therefore which one has to become iterative first.
//!
//! The fast locks at the bottom run on every baseline. The bisection driver is `#[ignore]`d
//! because it spawns on the order of a hundred processes; run it on purpose:
//!
//! ```text
//! cargo nextest run -p logicaffeine-tests --test W_perf_extra --run-ignored all --no-capture
//! ```

use logicaffeine_base::BigInt;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{double_check, DoubleCheck};
use logicaffeine_kernel::{infer_type, is_subtype, normalize, Context, Term};

// ============================================================================================
// HELPERS — per-file, per house style. There is no shared kernel helper module.
// ============================================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// `(λx:Nat. x) ((λx:Nat. x) (… Zero))` nested `depth` deep — the corpus's timing spine,
/// built iteratively so CONSTRUCTION never itself touches the stack.
fn spine_at(depth: usize) -> Term {
    let id_nat = lam("x", nat(), var("x"));
    (0..depth).fold(g("Zero"), |acc, _| app(id_nat.clone(), acc))
}

/// A Π telescope `Nat → Nat → … → Nat`, `depth` arrows deep — the RIGHT-nested shape, so
/// the recursion runs down `body_type` rather than down `App`'s function slot.
fn arrow_at(depth: usize) -> Term {
    (0..depth).fold(nat(), |acc, _| pi("_", nat(), acc))
}

/// Node count — the corpus's own measure, reproduced here because it is itself a NATIVE
/// RECURSION and therefore itself has a ceiling worth knowing.
fn nodes(t: &Term) -> usize {
    1 + match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => 0,
        Term::Annotated { term, .. } => return nodes(term),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => nodes(param_type) + nodes(body_type),
        Term::Lambda {
            param_type, body, ..
        } => nodes(param_type) + nodes(body),
        Term::App(f, a) => nodes(f) + nodes(a),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => nodes(discriminant) + nodes(motive) + cases.iter().map(nodes).sum::<usize>(),
        Term::Fix { body, .. } => nodes(body),
        Term::MutualFix { defs, .. } => defs.iter().map(|(_, b)| nodes(b)).sum::<usize>(),
        Term::Let {
            ty, value, body, ..
        } => nodes(ty) + nodes(value) + nodes(body),
    }
}

// ============================================================================================
// THE PROBE — one operation, one depth, one declared stack, in a process of its own.
// ============================================================================================

/// Every operation the deep-spine case puts weight on. `Build` is the CONTROL: it is the
/// only one written iteratively, so if it dies the measurement is of the allocator, not of
/// recursion.
const PROBE_KINDS: &[&str] = &[
    "build",
    "drop",
    "clone",
    "eq",
    "display",
    "nodes",
    "normalize",
    "infer_app",
    "infer_pi",
];

fn run_probe(kind: &str, depth: usize) {
    match kind {
        "build" => {
            let t = spine_at(depth);
            std::mem::forget(t);
        }
        "drop" => {
            let t = spine_at(depth);
            drop(t);
        }
        "clone" => {
            let t = spine_at(depth);
            let c = t.clone();
            std::mem::forget(t);
            std::mem::forget(c);
        }
        "eq" => {
            let a = spine_at(depth);
            let b = spine_at(depth);
            assert!(a == b);
            std::mem::forget(a);
            std::mem::forget(b);
        }
        "display" => {
            let t = spine_at(depth);
            let s = format!("{t}");
            assert!(!s.is_empty());
            std::mem::forget(t);
        }
        "nodes" => {
            let t = spine_at(depth);
            assert!(nodes(&t) > 0);
            std::mem::forget(t);
        }
        "normalize" => {
            let ctx = std_ctx();
            let t = spine_at(depth);
            let nf = normalize(&ctx, &t);
            std::mem::forget(t);
            std::mem::forget(nf);
        }
        "infer_app" => {
            let ctx = std_ctx();
            let t = spine_at(depth);
            let ty = infer_type(&ctx, &t);
            std::mem::forget(t);
            std::mem::forget(ty);
        }
        "infer_pi" => {
            let ctx = std_ctx();
            let t = arrow_at(depth);
            let ty = infer_type(&ctx, &t);
            std::mem::forget(t);
            std::mem::forget(ty);
        }
        other => panic!("unknown probe kind {other}"),
    }
}

/// The child half. Inert unless `W_PERF_KIND` is set, so it costs a baseline run nothing.
///
/// `std::mem::forget` is deliberate everywhere above: `Drop` is itself one of the probes,
/// and letting it run at the end of a `clone` probe would measure two walks as one.
#[test]
fn w_perf_stack_probe_child() {
    let Ok(kind) = std::env::var("W_PERF_KIND") else {
        return;
    };
    let depth: usize = std::env::var("W_PERF_DEPTH").unwrap().parse().unwrap();
    let stack: usize = std::env::var("W_PERF_STACK").unwrap().parse().unwrap();
    if stack == 0 {
        // Stack 0 means "whatever libtest handed this thread" — the calibration probe.
        run_probe(&kind, depth);
    } else {
        let owned = kind.clone();
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || run_probe(&owned, depth))
            .unwrap()
            .join()
            .unwrap();
    }
    println!("W_PERF_OK {kind} {depth} {stack}");
    // Leave through the front door: libtest's own teardown would otherwise drop the
    // forgotten-nothing and, more importantly, a panic below this line would be read as
    // an overflow by the driver.
    std::process::exit(0);
}

/// Run one probe in a child process. `Ok(true)` = survived, `Ok(false)` = died (overflow or
/// any other fatal signal).
fn probe(kind: &str, depth: usize, stack: usize) -> bool {
    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "w_perf_stack_probe_child",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("W_PERF_KIND", kind)
        .env("W_PERF_DEPTH", depth.to_string())
        .env("W_PERF_STACK", stack.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("the probe child starts");
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains("W_PERF_OK")
}

/// The largest depth at which `kind` survives on a `stack`-byte stack: double until it
/// dies, then bisect. Returns `None` if it cannot even manage `lo`.
fn ceiling(kind: &str, stack: usize, lo: usize, cap: usize) -> Option<usize> {
    if !probe(kind, lo, stack) {
        return None;
    }
    let mut good = lo;
    let mut bad = None;
    let mut probe_at = lo.saturating_mul(2);
    while probe_at <= cap {
        if probe(kind, probe_at, stack) {
            good = probe_at;
            probe_at = probe_at.saturating_mul(2);
        } else {
            bad = Some(probe_at);
            break;
        }
    }
    let mut bad = match bad {
        Some(b) => b,
        None => return Some(good),
    };
    while bad - good > 1.max(good / 64) {
        let mid = good + (bad - good) / 2;
        if probe(kind, mid, stack) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Some(good)
}

// ============================================================================================
// THE MEASUREMENT — run on purpose, not on every baseline.
// ============================================================================================

/// Bisect every walk's native-recursion ceiling at the stack libtest actually hands a test,
/// and at a declared 2 MiB, and print the table.
#[test]
#[ignore = "measurement: spawns ~100 child processes; run on purpose"]
fn w_perf_measure_native_recursion_ceilings() {
    const TWO_MIB: usize = 2 * 1024 * 1024;
    println!("\n  kind            @libtest-default      @2 MiB");
    println!("  ------------------------------------------------");
    for kind in PROBE_KINDS {
        let native = ceiling(kind, 0, 8, 200_000);
        let two = ceiling(kind, TWO_MIB, 8, 200_000);
        println!(
            "  {kind:<14}  {:>16}  {:>10}",
            native
                .map(|d| d.to_string())
                .unwrap_or_else(|| "DIES AT 8".into()),
            two.map(|d| d.to_string())
                .unwrap_or_else(|| "DIES AT 8".into()),
        );
    }
    println!();
}

/// How the cost of `normalize` scales with a spine's depth — the second half of the corpus
/// case, measured below the stack ceiling so it can be read at all.
#[test]
#[ignore = "measurement: seconds of reduction; run on purpose"]
fn w_perf_measure_spine_cost_growth() {
    let ctx = std_ctx();
    // Warm the allocator and the instruction cache.
    let _ = normalize(&ctx, &spine_at(40));
    println!("\n  depth      normalize       vs. 3x-linear");
    println!("  ------------------------------------------------");
    let mut prev: Option<(usize, f64)> = None;
    for depth in [25usize, 50, 75, 100, 150, 200, 300] {
        let t = spine_at(depth);
        let start = std::time::Instant::now();
        let nf = normalize(&ctx, &t);
        let elapsed = start.elapsed().as_secs_f64();
        assert_eq!(nf, g("Zero"), "a depth-{depth} spine reduces to Zero");
        let note = match prev {
            Some((pd, pe)) if pe > 0.0 => {
                format!(
                    "{:.2}x for {:.2}x depth",
                    elapsed / pe,
                    depth as f64 / pd as f64
                )
            }
            _ => String::new(),
        };
        println!("  {depth:<9}  {:>10.6}s     {note}", elapsed);
        prev = Some((depth, elapsed));
    }
    println!();
}

// ============================================================================================
// THE `Nat` WALL — the kernel's single largest measured hotspot, and the lock its fix must pass
// ============================================================================================

/// MEASURED: `prelude_types_extra::the_nat_order_costs_the_smaller_operand` and
/// `a_char_admits_a_scalar_value_and_refuses_a_surrogate` are the two slowest tests in the
/// kernel crate by an order of magnitude — 61.7 s and 52.4 s of a 66 s suite on a loaded box,
/// against a next-slowest of 17 s and a median under 0.3 s. The cause is not conversion
/// checking: `Nat` has NO literal fast path. `try_primitive_reduce` matches `Literal::Int` and
/// `Literal::BigInt` only, because `term::lit_bigint` returns `None` for `Literal::Nat`, so
/// `natLe` peels one `Succ` per unit through the Peano bridge and `natLt m n = natLe (Succ m) n`
/// grinds until the budget is spent.
///
/// The fix is an O(1) primitive on `Literal::Nat`, the way Lean's `Nat.decLt` is GMP. It lives
/// in `reduction.rs`, which this lane does not own — so what this lane can leave behind is the
/// SPECIFICATION the fast path must satisfy, green today against the unary unfolding, so that
/// whoever adds the primitive cannot change an answer while making it fast.
///
/// A dense grid plus both boundaries. Every pair below is decided by the UNARY path today; the
/// same assertion must hold once a literal path short-circuits it.
#[test]
fn a_nat_order_fast_path_must_agree_with_the_unary_unfolding() {
    let ctx = std_ctx();
    let natl = |n: u64| Term::Lit(logicaffeine_kernel::Literal::Nat(BigInt::from_u64(n)));
    let tt = g("true");
    let ff = g("false");

    // The dense grid: every ordered pair below 12, both relations, against machine arithmetic.
    for m in 0u64..12 {
        for n in 0u64..12 {
            let le = normalize(&ctx, &app(app(g("natLe"), natl(m)), natl(n)));
            assert_eq!(
                le,
                if m <= n { tt.clone() } else { ff.clone() },
                "natLe {m} {n} must decide as the machine comparison does"
            );
            let lt = normalize(&ctx, &app(app(g("natLt"), natl(m)), natl(n)));
            assert_eq!(
                lt,
                if m < n { tt.clone() } else { ff.clone() },
                "natLt {m} {n} must decide as the machine comparison does"
            );
        }
    }

    // The boundaries that a fast path gets wrong: zero on each side, and off-by-one either way.
    for (m, n) in [
        (0u64, 0u64),
        (0, 1),
        (1, 0),
        (255, 255),
        (255, 256),
        (256, 255),
    ] {
        assert_eq!(
            normalize(&ctx, &app(app(g("natLe"), natl(m)), natl(n))),
            if m <= n { tt.clone() } else { ff.clone() },
            "natLe {m} {n} — the boundary a fast path is most likely to get wrong"
        );
        assert_eq!(
            normalize(&ctx, &app(app(g("natLt"), natl(m)), natl(n))),
            if m < n { tt.clone() } else { ff.clone() },
            "natLt {m} {n} — the boundary a fast path is most likely to get wrong"
        );
    }

    // And the bridge the fast path must not break: a `Nat` LITERAL and its Peano spelling are
    // interchangeable, so a primitive keyed on the literal must agree with the spine.
    let peano = |n: u64| (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    for m in 0u64..6 {
        for n in 0u64..6 {
            assert_eq!(
                normalize(&ctx, &app(app(g("natLe"), natl(m)), natl(n))),
                normalize(&ctx, &app(app(g("natLe"), peano(m)), peano(n))),
                "the literal and the Peano spine must decide natLe {m} {n} identically"
            );
        }
    }
}

/// The honest negative that must SURVIVE the fast path: `natLt` on two operands too large for
/// the unary path is a REFUSAL, never `true`. A fast path makes this decidable; until it lands,
/// the failure direction must stay the safe one, and afterwards the answer must be `false`
/// rather than a truncation dressed up as a verdict.
#[test]
fn an_undecided_nat_order_is_never_mistaken_for_true() {
    let ctx = std_ctx();
    let huge = Term::Lit(logicaffeine_kernel::Literal::Nat(
        BigInt::parse_decimal("18446744073709551616").expect("2^64 parses"),
    ));
    let stuck = normalize(&ctx, &app(app(g("natLt"), huge.clone()), huge.clone()));
    assert_ne!(
        stuck,
        g("true"),
        "2^64 < 2^64 is FALSE, and must never read as true"
    );
}

// ============================================================================================
// WAVE 2 · THE ABORT, PRICED AND REMOVED
// ============================================================================================
//
// The bisection above says WHERE the ceiling is. This section says what to do about it, and
// proves the mechanism against the corpus's own shapes.
//
// The kernel's depth door (`type_checker::MAX_TERM_DEPTH`, 1600) promises a REFUSAL — "the term
// is neither accepted nor rejected, because descending it would exhaust the native stack" — for
// anything deeper than it admits. MEASURED, the walk it guards dies more than an order of
// magnitude sooner: `infer_type`'s frame is 16,832 bytes in a debug build and libtest hands a
// test 2 MiB, so about 123 rungs is the real ceiling. Every term in the band between the two
// passes the door and then kills the process — the exact failure the door exists to prevent,
// and how `lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically`
// fails today: not by an assertion, by a corpse.
//
// `term_store::deepen_stack` closes the band from the other side. It measures the term once at
// the outermost entry — the same linear pass the door already pays — and runs the call on a
// scoped thread whose stack is SIZED FROM THAT DEPTH, so the door's advertised limit becomes
// the real one. The tests below are the specification for wiring it into the kernel's entry
// points, every one of which lives in a file this lane does not own.
//
// Every probe that could overflow runs in a CHILD PROCESS, for the same reason the bisection
// above does: an overflow is a readable exit status there and a dead test run here.

use logicaffeine_kernel::term_store::{
    deepen_stack, occurrence_count_in_term, occurrences_in_term, occurs_free_in_term,
    stack_bytes_for_depth, term_depth, Occurrences, StoreNormalForm, TermStore, INLINE_DEPTH,
};

fn eq_t(ty: Term, l: Term, r: Term) -> Term {
    app(app(app(g("Eq"), ty), l), r)
}
fn refl_t(ty: Term, x: Term) -> Term {
    app(app(g("refl"), ty), x)
}
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
fn let_in(n: &str, ty: Term, v: Term, b: Term) -> Term {
    Term::Let {
        name: n.to_string(),
        ty: Box::new(ty),
        value: Box::new(v),
        body: Box::new(b),
    }
}
fn app4(f: Term, w: Term, x: Term, y: Term, z: Term) -> Term {
    app(app(app(app(f, w), x), y), z)
}

/// The corpus's own dependent telescope, saturated — the statement that ABORTS the process
/// today, rebuilt here exactly as `lean_conformance_t00_terms` builds it.
fn corpus_dependent_telescope(deep: usize) -> Term {
    let mut body = var("n");
    for i in (0..deep).rev() {
        body = lam(&format!("p{i}"), eq_t(nat(), var("n"), var("n")), body);
    }
    let dep = lam("n", nat(), body);
    let mut applied = app(dep, peano(1));
    for _ in 0..deep {
        applied = app(applied, refl_t(nat(), peano(1)));
    }
    applied
}

// --------------------------------------------------------------------------------------------
// THE WAVE-2 PROBE — one kernel entry, one depth, one stack discipline, in a process of its own
// --------------------------------------------------------------------------------------------

/// What each probe kind asks the kernel for. `_bare` is the kernel as it stands; `_sized` is the
/// same call through [`deepen_stack`], which is the one-line wiring this lane is handing over.
const WAVE2_KINDS: &[&str] = &[
    "infer_arrow_bare",
    "infer_arrow_sized",
    "infer_tele_bare",
    "infer_tele_sized",
    "double_tele_bare",
    "double_tele_sized",
];

fn run_wave2_probe(kind: &str, depth: usize) -> String {
    let ctx = std_ctx();
    let verdict = |r: Result<Term, logicaffeine_kernel::KernelError>| -> String {
        match r {
            Ok(_) => "ACCEPTED".to_string(),
            Err(e) => {
                let text = format!("{e:?}");
                if text.contains("nests deeper") {
                    "REFUSED".to_string()
                } else {
                    "REJECTED".to_string()
                }
            }
        }
    };
    match kind {
        "infer_arrow_bare" => {
            let t = arrow_at(depth);
            let v = verdict(infer_type(&ctx, &t));
            std::mem::forget(t);
            v
        }
        "infer_arrow_sized" => {
            let t = arrow_at(depth);
            let v = verdict(deepen_stack(&t, || infer_type(&ctx, &t)).expect("outermost"));
            std::mem::forget(t);
            v
        }
        "infer_tele_bare" => {
            let t = corpus_dependent_telescope(depth);
            let v = verdict(infer_type(&ctx, &t));
            std::mem::forget(t);
            v
        }
        "infer_tele_sized" => {
            let t = corpus_dependent_telescope(depth);
            let v = verdict(deepen_stack(&t, || infer_type(&ctx, &t)).expect("outermost"));
            std::mem::forget(t);
            v
        }
        // The SECOND kernel. `double_check` calls `infer_type` first, so in the whole band
        // between the real ceiling and the advertised one the process dies before the
        // re-checker is ever consulted — the second kernel protects nothing there.
        "double_tele_bare" => {
            let t = corpus_dependent_telescope(depth);
            let v = format!("{:?}", double_check(&ctx, &t));
            std::mem::forget(t);
            v.split(['(', ' ']).next().unwrap().to_string()
        }
        "double_tele_sized" => {
            let t = corpus_dependent_telescope(depth);
            let v = format!(
                "{:?}",
                deepen_stack(&t, || double_check(&ctx, &t)).expect("outermost")
            );
            std::mem::forget(t);
            v.split(['(', ' ']).next().unwrap().to_string()
        }
        other => panic!("unknown wave-2 probe kind {other}"),
    }
}

/// The child half. Inert unless `W_PERF2_KIND` is set, so it costs a baseline run nothing.
#[test]
fn w_perf_wave2_probe_child() {
    let Ok(kind) = std::env::var("W_PERF2_KIND") else {
        return;
    };
    let depth: usize = std::env::var("W_PERF2_DEPTH").unwrap().parse().unwrap();
    let verdict = run_wave2_probe(&kind, depth);
    println!("W_PERF2_OK {kind} {depth} {verdict}");
    std::process::exit(0);
}

/// Run one wave-2 probe in a child process. `None` means the child DIED — an overflow or any
/// other fatal signal — which is the outcome this whole section exists to eliminate.
fn wave2_probe(kind: &str, depth: usize) -> Option<String> {
    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "w_perf_wave2_probe_child",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("W_PERF2_KIND", kind)
        .env("W_PERF2_DEPTH", depth.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("the probe child starts");
    if !out.status.success() {
        return None;
    }
    // The marker is not at the start of a line: with `--nocapture` libtest prints it right
    // after `test <name> ... `, so it is SEARCHED for rather than matched as a prefix.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let after = stdout.split("W_PERF2_OK ").nth(1)?;
    after.split_whitespace().nth(2).map(str::to_string)
}

// --------------------------------------------------------------------------------------------
// THE LOCKS
// --------------------------------------------------------------------------------------------

/// THE CASE, in one assertion. `infer_type` on the corpus's saturated sixty-deep dependent
/// telescope is the statement that dies: 125 nested `infer_type` frames against libtest's
/// 2 MiB, to the byte. On a stack sized from the term's own depth it simply answers.
///
/// Run in-process on purpose: the mechanism has to work HERE, on the thread a test was given,
/// or it is no use to a kernel entry point.
#[test]
fn a_sized_stack_checks_the_telescope_that_aborts_the_bare_kernel() {
    let ctx = std_ctx();
    let applied = corpus_dependent_telescope(60);

    // The switch must genuinely fire: this term is well past the inline band.
    assert!(
        term_depth(&applied) > INLINE_DEPTH,
        "the corpus telescope is {} rungs deep, past the inline band of {INLINE_DEPTH}",
        term_depth(&applied)
    );

    let answer = deepen_stack(&applied, || infer_type(&ctx, &applied))
        .expect("an outermost kernel call always runs");
    assert_eq!(
        answer.expect("the saturated telescope checks"),
        nat(),
        "each argument is substituted positionally into the remaining telescope"
    );
}

/// The same mechanism on the OTHER corpus shape — the β-spine at the depth the timing half of
/// the case uses. `normalize`'s per-step bookkeeping is what walks this one.
#[test]
fn a_sized_stack_normalises_the_corpus_spine_at_its_full_depth() {
    let ctx = std_ctx();
    let spine = spine_at(1500);
    assert!(term_depth(&spine) > INLINE_DEPTH);
    let nf = deepen_stack(&spine, || normalize(&ctx, &spine)).expect("an outermost call runs");
    assert_eq!(nf, g("Zero"), "a 1500-deep spine reaches its normal form");
}

/// THE ONE THAT MATTERS MOST — a term the door ADMITS must never be answered with a corpse.
///
/// The bare kernel fails this today at every depth in the band, which is why the probe runs in
/// a child process and why the assertion is made about the SIZED discipline: a verdict, of
/// either polarity, and a live process. It stays green once the wiring lands, because then the
/// bare call is the sized one.
#[test]
fn a_term_the_door_admits_is_never_answered_with_a_corpse() {
    for depth in [INLINE_DEPTH + 1, 256, 1024, 1500] {
        let verdict = wave2_probe("infer_arrow_sized", depth).unwrap_or_else(|| {
            panic!("a {depth}-rung telescope the door admits KILLED the process")
        });
        assert_eq!(
            verdict, "ACCEPTED",
            "a {depth}-rung arrow telescope is well formed and the door admits it"
        );
    }
}

/// BOTH KERNELS, on a term the bare stack kills. `double_check` calls `infer_type` FIRST, so
/// across the whole band between the kernel's real ceiling and the one its door advertises the
/// process dies before the re-checker is ever consulted — the second kernel, whose entire job is
/// to disagree with the first, protects nothing there. (The two have opposite stack disciplines:
/// `recheck.rs` carries a stack-address budget and answers `Unsupported` honestly; the main
/// kernel has none and aborts.) On a sized stack the comparison actually happens, and the two
/// kernels AGREE.
///
/// MEASURED, and the reason `term_store::kernel_stack_bytes` exists: on a sized stack `double_check`
/// agrees at depth 20, 52, 60 and 200, and at depth 1000 turns into
/// `MainOnlyReCheckerIncomplete` — because `recheck.rs`'s `RECHECK_STACK_BUDGET` is a hard-coded
/// 1 MiB chosen for a 2 MiB thread, so the second kernel gives up with fifty megabytes of stack
/// still unused underneath it.
#[test]
fn both_kernels_can_be_compared_on_a_term_the_bare_stack_kills() {
    let ctx = std_ctx();
    for deep in [60usize, 200] {
        let applied = corpus_dependent_telescope(deep);
        let verdict = deepen_stack(&applied, || double_check(&ctx, &applied)).expect("outermost");
        assert!(
            matches!(verdict, DoubleCheck::Agreed),
            "two independently written kernels must AGREE about a {deep}-deep dependent \
             telescope, and the bare stack dies at 52: {verdict:?}"
        );
    }
}

/// THE NEGATIVE: a sized stack must not turn the door into a rubber stamp. Past the door the
/// answer is still a REFUSAL — an error that says it is neither an acceptance nor a rejection —
/// and the process still lives.
#[test]
fn a_term_past_the_door_is_refused_and_not_merely_run() {
    assert_eq!(
        wave2_probe("infer_arrow_sized", 4096).expect("a refused term never kills the process"),
        "REFUSED",
        "past `MAX_TERM_DEPTH` the kernel refuses rather than descending"
    );
    // One rung either side of the cliff the door actually has.
    let cliff = (1..)
        .map(|k| 1024 + k * 128)
        .take(20)
        .find(|&d| wave2_probe("infer_arrow_sized", d).as_deref() == Some("REFUSED"))
        .expect("the door refuses somewhere below 3584 rungs");
    assert!(
        wave2_probe("infer_arrow_sized", cliff - 128).as_deref() == Some("ACCEPTED"),
        "the door is a cliff, not a gradient: {} rungs is admitted and {cliff} is refused",
        cliff - 128
    );
}

/// The switch must be INVISIBLE. The same term checked with and without it gives the identical
/// answer on both the accepting and the refusing side; a performance mechanism that changed a
/// verdict would be a soundness bug wearing a stopwatch.
#[test]
fn a_sized_stack_never_changes_an_answer() {
    let ctx = std_ctx();
    let shapes: Vec<Term> = vec![
        nat(),
        spine_at(8),
        arrow_at(8),
        corpus_dependent_telescope(4),
        app(g("Succ"), peano(3)),
        // Ill-typed on purpose: a rejection must survive the switch too.
        app(peano(1), peano(1)),
        app(g("Nat"), g("Nat")),
    ];
    for t in shapes {
        let direct = format!("{:?}", infer_type(&ctx, &t));
        let sized = format!(
            "{:?}",
            deepen_stack(&t, || infer_type(&ctx, &t)).expect("outermost")
        );
        assert_eq!(direct, sized, "the sized stack changed the answer for {t}");
        assert_eq!(
            normalize(&ctx, &t),
            deepen_stack(&t, || normalize(&ctx, &t)).expect("outermost"),
            "the sized stack changed the normal form of {t}"
        );
    }
}

/// The sizing has to cover the walk it is sizing for, priced against the MEASURED frame rather
/// than a hoped-for one: at every depth the door admits, the reservation holds one `infer_type`
/// frame per rung with room to spare.
#[test]
fn the_sizing_covers_a_measured_frame_at_every_admitted_depth() {
    const MEASURED_INFER_FRAME: usize = 16_832;
    for depth in [1usize, 33, 100, 500, 1000, 1600] {
        let reserved = stack_bytes_for_depth(depth);
        assert!(
            reserved >= depth * MEASURED_INFER_FRAME,
            "depth {depth} reserves {reserved} B, and one measured `infer_type` frame per rung \
             needs {} B",
            depth * MEASURED_INFER_FRAME
        );
    }
}

/// The measurement behind the constants: where each kernel entry dies bare, and where it dies
/// on a sized stack. This is the table the wiring decision is made from.
#[test]
#[ignore = "measurement: spawns ~100 child processes; run on purpose"]
fn w_perf_measure_the_bare_and_sized_kernel_ceilings() {
    /// `Ok(d)` — `d` is the deepest surviving depth below `cap`. `Err(cap)` — it survived the
    /// whole band, which for a `_sized` kind is the answer: nothing in the band the door admits
    /// kills the process.
    fn ceiling2(kind: &str, lo: usize, cap: usize) -> Result<usize, usize> {
        assert!(
            wave2_probe(kind, lo).is_some(),
            "{kind} dies at {lo}, which is not a ceiling"
        );
        let mut good = lo;
        let mut bad = None;
        let mut at = lo * 2;
        while at <= cap {
            match wave2_probe(kind, at) {
                Some(_) => {
                    good = at;
                    at *= 2;
                }
                None => {
                    bad = Some(at);
                    break;
                }
            }
        }
        // The doubling can overshoot the cap without ever finding a death; the cap itself is
        // the last thing worth asking about, since past it the door refuses anyway.
        let mut bad = match bad {
            Some(b) => b.min(cap + 1),
            None => match wave2_probe(kind, cap) {
                Some(_) => return Err(cap),
                None => cap,
            },
        };
        while bad - good > 1 {
            let mid = good + (bad - good) / 2;
            if wave2_probe(kind, mid).is_some() {
                good = mid;
            } else {
                bad = mid;
            }
        }
        Ok(good)
    }
    // 1600 is `MAX_TERM_DEPTH`: past it the door refuses, so no deeper term is ever walked and
    // no deeper measurement would mean anything.
    println!("\n  kind                 deepest term that does NOT kill the process   wall clock");
    println!("  ---------------------------------------------------------------------------");
    for kind in WAVE2_KINDS {
        let cap = 1600;
        let start = std::time::Instant::now();
        let c = ceiling2(kind, 4, cap);
        let took = start.elapsed();
        let shown = match c {
            Ok(d) => format!("{d}"),
            Err(cap) => format!("{cap} — the whole band the door admits"),
        };
        println!("  {kind:<20} {shown:<45} {took:.1?}");
    }
    println!();
}

// --------------------------------------------------------------------------------------------
// THE OCCURRENCE GATE — the specification for the ζ rule that has to stop copying
// --------------------------------------------------------------------------------------------

/// The independent oracle: free occurrences by the textbook rule, written from the definition
/// rather than from the implementation. Natively recursive ON PURPOSE — that is exactly the
/// ceiling the shipped one does not have, and it keeps the two from sharing a mistake.
fn oracle_occurrences(t: &Term, name: &str) -> u32 {
    match t {
        Term::Var(v) => u32::from(v == name),
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => 0,
        Term::App(f, a) => oracle_occurrences(f, name) + oracle_occurrences(a, name),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            oracle_occurrences(param_type, name)
                + if param == name {
                    0
                } else {
                    oracle_occurrences(body_type, name)
                }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            oracle_occurrences(param_type, name)
                + if param == name {
                    0
                } else {
                    oracle_occurrences(body, name)
                }
        }
        Term::Let {
            name: bound,
            ty,
            value,
            body,
        } => {
            oracle_occurrences(ty, name)
                + oracle_occurrences(value, name)
                + if bound == name {
                    0
                } else {
                    oracle_occurrences(body, name)
                }
        }
        Term::Fix { name: bound, body } => {
            if bound == name {
                0
            } else {
                oracle_occurrences(body, name)
            }
        }
        Term::MutualFix { defs, .. } => {
            if defs.iter().any(|(n, _)| n == name) {
                0
            } else {
                defs.iter().map(|(_, b)| oracle_occurrences(b, name)).sum()
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            oracle_occurrences(discriminant, name)
                + oracle_occurrences(motive, name)
                + cases
                    .iter()
                    .map(|c| oracle_occurrences(c, name))
                    .sum::<u32>()
        }
        Term::Annotated { term, .. } => oracle_occurrences(term, name),
    }
}

/// A fixed LCG, so a failure is reproducible from the seed the assertion prints.
struct Lcg(u64);

impl Lcg {
    fn pick(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn random_term(rng: &mut Lcg, depth: u32) -> Term {
    const NAMES: [&str; 3] = ["x", "y", "z"];
    if depth == 0 {
        return match rng.pick(3) {
            0 => var(NAMES[rng.pick(3) as usize]),
            1 => g("Nat"),
            _ => Term::Sort(logicaffeine_kernel::Universe::Type(0)),
        };
    }
    match rng.pick(8) {
        0 => app(random_term(rng, depth - 1), random_term(rng, depth - 1)),
        1 => lam(
            NAMES[rng.pick(3) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        2 => pi(
            NAMES[rng.pick(3) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        3 => let_in(
            NAMES[rng.pick(3) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        4 => Term::Fix {
            name: NAMES[rng.pick(3) as usize].to_string(),
            body: Box::new(random_term(rng, depth - 1)),
        },
        5 => Term::MutualFix {
            defs: vec![
                (
                    NAMES[rng.pick(3) as usize].to_string(),
                    random_term(rng, depth - 1),
                ),
                (
                    NAMES[rng.pick(3) as usize].to_string(),
                    random_term(rng, depth - 1),
                ),
            ],
            index: 0,
        },
        6 => Term::Match {
            discriminant: Box::new(random_term(rng, depth - 1)),
            motive: Box::new(random_term(rng, depth - 1)),
            cases: vec![random_term(rng, depth - 1)],
        },
        _ => random_term(rng, depth - 1),
    }
}

/// THREE implementations of one specification, over a thousand random terms: the `Term`-side
/// gate, the store's hash-consed counter, and the textbook oracle above. Three implementations
/// that agree is evidence; one implementation tested against itself is not.
#[test]
fn the_occurrence_gate_agrees_with_two_independent_oracles() {
    let mut store = TermStore::new();
    for seed in 0..1000u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
        let t = random_term(&mut rng, 4);
        let id = store.intern(&t);
        for name in ["x", "y", "z", "absent"] {
            let sym = store.symbol(name);
            let expected = oracle_occurrences(&t, name);
            assert_eq!(
                occurrence_count_in_term(&t, name),
                expected,
                "seed {seed}: the `Term`-side count for `{name}` in {t}"
            );
            assert_eq!(
                store.occurrence_count(id, sym),
                expected,
                "seed {seed}: the store's count for `{name}` in {t}"
            );
            assert_eq!(
                occurrences_in_term(&t, name),
                match expected {
                    0 => Occurrences::Zero,
                    1 => Occurrences::Once,
                    _ => Occurrences::Many,
                },
                "seed {seed}: the classified gate for `{name}` in {t}"
            );
            assert_eq!(occurs_free_in_term(&t, name), expected > 0, "seed {seed}");
        }
    }
}

/// The gate's VERDICT on the corpus's own `let`, and on each shape the suite already pins as
/// ζ-transparent. The first must be held back; the others must keep expanding, or `let_zeta.rs`
/// and the T00 transparency cases go red the moment the rule lands.
#[test]
fn the_occurrence_gate_holds_the_shared_let_and_releases_every_pinned_one() {
    let shared_body = app4(g("quad"), var("b"), var("b"), var("b"), var("b"));
    assert_eq!(
        occurrences_in_term(&shared_body, "b"),
        Occurrences::Many,
        "four uses of one binding is the shape the corpus measures"
    );

    // Single use — the `let_zeta` shape. Must still expand.
    assert_eq!(
        occurrences_in_term(&app(g("Succ"), var("b")), "b"),
        Occurrences::Once
    );
    // Zero uses — the ascription dodge `let _ := e in _`. Must still expand.
    assert_eq!(occurrences_in_term(&var("_"), "b"), Occurrences::Zero);
    // The bound VALUE is not a use of the binding, however often it mentions the name.
    let self_shadowing = let_in("b", nat(), app(var("b"), var("b")), var("b"));
    assert_eq!(
        occurrences_in_term(&self_shadowing, "b"),
        Occurrences::Many,
        "the two mentions in the VALUE are the OUTER `b`; only the body's is the bound one"
    );
    // And a `Let`'s own name is not free in it.
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let shared = let_in("b", nat(), big, shared_body);
    assert_eq!(
        occurrence_count_in_term(&shared, "u"),
        1,
        "`u` is free in the value, once"
    );
    assert_eq!(
        occurrences_in_term(&shared, "b"),
        Occurrences::Zero,
        "the `Let` binds its own"
    );
}

/// The ARITHMETIC of the ζ gate, EXECUTED rather than argued, and the kernel held to it. The
/// store implements the rule — expand a binding used at most once, keep one used more — and
/// `reduction.rs`'s `Term::Let` arm now applies the same gate (`term_store::zeta_or_share_opt`),
/// so the kernel's own `normalize` is measured with the corpus's own `nodes` walk against the
/// corpus's own ceiling, and must be the store's normal form.
///
/// 329 nodes is what eager ζ produces and 97 is the bound; the ζ-preserving normal form is 92.
/// The gap is not close, which is why no constant-factor fix reaches it.
#[test]
fn the_gated_zeta_normal_form_meets_the_corpus_node_bound() {
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let big_nodes = nodes(&big);
    assert_eq!(big_nodes, 81, "forty `Succ`s over a free variable");

    let shared = let_in(
        "b",
        nat(),
        big.clone(),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );

    // What eager ζ writes: the value at every occurrence.
    let eager = app4(
        g("quad"),
        big.clone(),
        big.clone(),
        big.clone(),
        big.clone(),
    );
    assert_eq!(
        nodes(&eager),
        4 * big_nodes + 5,
        "eager ζ copies the value four times"
    );
    assert!(
        nodes(&eager) > big_nodes + 16,
        "which is exactly what fails the corpus bound"
    );

    // What the kernel does: the gate holds the four-use binding as the sharing node.
    let ctx = std_ctx();
    let kernel_nf = normalize(&ctx, &shared);
    assert_eq!(
        nodes(&kernel_nf),
        big_nodes + 11,
        "the kernel's normal form keeps the binding: {kernel_nf}"
    );
    assert!(
        nodes(&kernel_nf) <= big_nodes + 16,
        "the kernel's normal form is {} nodes against a ceiling of {}",
        nodes(&kernel_nf),
        big_nodes + 16
    );

    // What the gate produces: the binding survives as the sharing node.
    let mut store = TermStore::new();
    let id = store.intern(&shared);
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 16) else {
        panic!("a β/ζ normal form exists for a term whose only redex is the `let`");
    };
    let gated = store.extract(nf);
    assert_eq!(
        nodes(&gated),
        big_nodes + 11,
        "one `Let`, its annotation, the value ONCE, and a four-argument head"
    );
    assert!(
        nodes(&gated) <= big_nodes + 16,
        "the gated normal form is {} nodes against a ceiling of {}",
        nodes(&gated),
        big_nodes + 16
    );
    assert_eq!(
        kernel_nf, gated,
        "the kernel's reducer and the store's model reach one normal form"
    );

    // AND IT IS THE SAME TERM. A smaller normal form that meant something else would be a
    // soundness bug, so the survivor is ζ-expanded on demand and compared.
    let expanded = store
        .zeta_expand_head(nf)
        .expect("the survivor is `Let`-headed");
    let StoreNormalForm::Reached(expanded_nf) = store.beta_zeta_normalize(expanded, 1 << 16) else {
        panic!("the expansion normalises");
    };
    assert_eq!(
        store.extract(expanded_nf),
        eager,
        "ζ on demand recovers exactly the term eager ζ produced"
    );
    let mut typed = std_ctx();
    typed.add_declaration(
        "quad",
        pi(
            "_",
            nat(),
            pi("_", nat(), pi("_", nat(), pi("_", nat(), nat()))),
        ),
    );
    let typed = typed.extend("u", nat());
    assert_eq!(
        infer_type(&typed, &shared).ok(),
        Some(nat()),
        "the shared term is a well-typed `Nat` once `quad` and `u` are in scope"
    );
    assert!(
        logicaffeine_kernel::defeq_for_test(&typed, &kernel_nf, &eager),
        "and the kernel's conversion agrees that its shared normal form is the eager one"
    );
    assert!(
        !logicaffeine_kernel::defeq_for_test(
            &typed,
            &kernel_nf,
            &app4(g("quad"), big.clone(), big.clone(), big.clone(), var("u"))
        ),
        "a term that drops one copy of the value is a different term"
    );
}

// --------------------------------------------------------------------------------------------
// THE REPRESENTATION, PRICED AGAINST THE TREE — the number the wiring decision turns on
// --------------------------------------------------------------------------------------------

/// The corpus case's second half asks for `ratio ≤ 5.0` when a spine's depth TRIPLES. The tree
/// reducer cannot reach it by any constant factor: `normalize_checked_at` pays THREE Θ(size)
/// walks per step — `substitute`'s copy of the remaining spine, `reduced == current`, and
/// `current == tortoise` — over Θ(size) steps, so its cost is Θ(size²) and tripling costs about
/// 9×. Removing two of the three still leaves 9×.
///
/// On the hash-consed store all three become `u32` work, so the same reduction is linear. Each
/// row is the MINIMUM of several repetitions after a warm-up, because a minimum over repetitions
/// is the closest a loaded box gets to the cost of the algorithm; and the store's column
/// INCLUDES interning, so it is the whole cost of the representation and not just the reduction.
#[test]
#[ignore = "measurement: seconds of reduction; run on purpose"]
fn w_perf_measure_the_spine_in_the_tree_and_in_the_store() {
    const REPS: usize = 5;
    let ctx = std_ctx();

    let time_tree = |t: &Term| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..REPS {
            let start = std::time::Instant::now();
            let nf = deepen_stack(t, || normalize(&ctx, t)).expect("outermost");
            let took = start.elapsed().as_secs_f64();
            assert_eq!(nf, g("Zero"));
            best = best.min(took);
        }
        best
    };
    let time_store = |t: &Term| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..REPS {
            let start = std::time::Instant::now();
            let mut store = TermStore::new();
            let id = store.intern(t);
            let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 24) else {
                panic!("the spine β-normalises")
            };
            let took = start.elapsed().as_secs_f64();
            assert_eq!(
                store.extract(nf),
                g("Zero"),
                "the store reaches the same normal form"
            );
            best = best.min(took);
        }
        best
    };

    // Warm the allocator and the instruction cache for BOTH paths before anything is timed.
    let warm = spine_at(40);
    let _ = time_tree(&warm);
    let _ = time_store(&warm);

    println!(
        "\n  depth     tree `normalize`   vs prev      store intern+β/ζ   vs prev     tree/store"
    );
    println!(
        "  ------------------------------------------------------------------------------------"
    );
    let mut prev: Option<(usize, f64, f64)> = None;
    for depth in [500usize, 1500] {
        let t = spine_at(depth);
        let tree = time_tree(&t);
        let store = time_store(&t);
        let growth = |now: f64, then: f64| -> String {
            if then > 0.0 {
                format!("{:>5.2}×", now / then)
            } else {
                "     -".to_string()
            }
        };
        let (tn, sn) = match prev {
            Some((_, pt, ps)) => (growth(tree, pt), growth(store, ps)),
            None => ("     -".to_string(), "     -".to_string()),
        };
        println!(
            "  {depth:<8}  {tree:>13.6}s   {tn}      {store:>13.6}s   {sn}     {:>7.1}×",
            tree / store
        );
        prev = Some((depth, tree, store));
    }
    println!("  (the corpus case asks the tree column for ≤ 5.00× at 3× the depth)\n");
}

/// The OTHER half of the hot path, measured: the kernel has no conversion cache of any kind, so
/// it re-does every conversion it has already decided.
///
/// `is_subtype` has one `alpha_eq` shortcut and then normalises BOTH sides to a FULL normal form
/// before comparing. Lean's `isDefEq` is whnf-driven with a `whnf` cache and an `isDefEq` cache,
/// so the second time a pair comes up it is a hash lookup. This prints what the missing cache
/// costs on a pair that is convertible but not α-equal — the shape every dependent application
/// produces.
#[test]
#[ignore = "measurement: seconds of conversion; run on purpose"]
fn w_perf_measure_what_the_missing_conversion_cache_costs() {
    let ctx = std_ctx();
    // Convertible, NOT α-equal: the left side needs the whole spine reduced away.
    let left = app(g("Succ"), spine_at(200));
    let right = app(g("Succ"), g("Zero"));
    assert!(
        is_subtype(&ctx, &left, &right),
        "the two sides are convertible"
    );

    println!("\n  repeats   is_subtype(same pair)     per call");
    println!("  --------------------------------------------------");
    for n in [1usize, 10, 100] {
        let start = std::time::Instant::now();
        for _ in 0..n {
            assert!(is_subtype(&ctx, &left, &right));
        }
        let took = start.elapsed().as_secs_f64();
        println!("  {n:<9} {took:>12.6}s       {:>12.6}s", took / n as f64);
    }
    println!(
        "  (a conversion cache would make every repeat after the first a hash lookup; the cost \n            per call above is FLAT, which is the measurement that says there is no cache)\n"
    );
}

/// The same claim as a LOCK rather than a printout, and counted in units that do not depend on
/// a loaded box: REDUCTION STEPS and DISTINCT NODES.
///
/// A `depth`-rung β-spine is exactly `depth` β-steps on the store — no more, proven by reaching
/// a normal form on a budget of exactly `depth`, and no fewer, proven by running out on one
/// less. Interning it is exactly one new node per rung. And β-reducing it allocates NOTHING,
/// because every term the reduction produces is already a node: that is what "the same subterm
/// is one node" buys, and it is precisely the Θ(size)-per-step copy the tree reducer cannot
/// avoid.
#[test]
fn the_stores_spine_costs_one_step_and_one_node_per_rung_and_nothing_to_reduce() {
    let mut interned: Vec<(usize, usize)> = Vec::new();
    for depth in [8usize, 50, 500, 1500] {
        let t = spine_at(depth);

        // Exactly `depth` steps: enough…
        let mut store = TermStore::new();
        let id = store.intern(&t);
        let before = store.len();
        interned.push((depth, before));
        assert!(
            matches!(
                store.beta_zeta_normalize(id, depth as u64),
                StoreNormalForm::Reached(_)
            ),
            "a {depth}-rung spine is {depth} β-steps"
        );
        // …and reducing it allocated no node at all.
        assert_eq!(
            store.len(),
            before,
            "every term the reduction produced was already a node at depth {depth}"
        );

        // …and not one fewer. A fresh store, because the normal-form cache remembers.
        let mut store = TermStore::new();
        let id = store.intern(&t);
        assert!(
            matches!(
                store.beta_zeta_normalize(id, depth as u64 - 1),
                StoreNormalForm::OutOfFuel { .. }
            ),
            "a {depth}-rung spine is not {} β-steps",
            depth - 1
        );
    }

    // One new node per rung, so tripling the depth triples the representation and no more —
    // the linearity the corpus case asks `normalize` for, on the representation side.
    let (near_d, near_n) = interned[2];
    let (far_d, far_n) = interned[3];
    assert_eq!(
        far_n - near_n,
        far_d - near_d,
        "interning is exactly one node per rung: {near_n} nodes at depth {near_d}, {far_n} at \
         depth {far_d}"
    );
    let ratio = far_n as f64 / near_n as f64;
    assert!(
        (2.5..3.5).contains(&ratio),
        "tripling the depth must triple the nodes; it multiplied them by {ratio:.2}"
    );
}

// ============================================================================================
// WAVE 3
// ============================================================================================
//
// Three things wave 2 could name but not settle, and one defect wave 2's own fix does not cover.
//
// 1. THE FLOAT RULE. The store interns `Literal::Float` by its raw bits and the kernel compares
//    it with IEEE `==`, and a hash-consed store that disagrees with the kernel about equality is
//    a soundness hole the moment it is wired in. Settled below, with the argument that leaves
//    exactly one option.
// 2. THE TYPE CACHE. `a_shared_subterm_is_checked_once_and_not_once_per_occurrence` is about the
//    CHECK, and the check is memoised by keying on term identity — except that a NAMED term
//    representation cannot key on the term alone, which is the half Lean does not have to think
//    about and we do.
// 3. THE ζ GATE, against the reducer `reduction.rs` actually has today.
// 4. A DEPTH NOBODY MEASURES. The door and the sized stack both measure the term they were
//    handed; `infer_type`'s `Var` arm walks the CONTEXT's type instead.

use logicaffeine_kernel::term_store::{
    deepen_stack_with_depth, literal_identity_eq, zeta_or_share, zeta_or_share_opt, DefEqCache,
    EnvId, KernelCall, TermId, TypeCache,
};
use logicaffeine_kernel::Literal;
use std::collections::HashMap;

fn flit(x: f64) -> Term {
    Term::Lit(Literal::Float(x))
}

/// Every float that can make id equality and `Term` equality disagree, plus enough that cannot
/// to prove the condition is not merely "there is a float in here".
///
/// The adversarial names are deliberately present: three DISTINCT NaN bit patterns (the default,
/// its negation, and one with a payload), and both zeros. A grid that held only `1.5` would let
/// every claim below pass vacuously.
const FLOAT_GRID: [f64; 11] = [
    f64::NAN,
    -f64::NAN,
    0.0,
    -0.0,
    1.5,
    -1.5,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::MIN_POSITIVE,
    5e-324,
    2.0,
];

/// The NaN with a payload, built by bits because no literal spells it.
fn payload_nan() -> f64 {
    f64::from_bits(0x7ff8_0000_0000_0001)
}

fn float_grid() -> Vec<f64> {
    let mut v = FLOAT_GRID.to_vec();
    v.push(payload_nan());
    v
}

// --------------------------------------------------------------------------------------------
// THE FLOAT RULE — decided, and the decision locked from both sides
// --------------------------------------------------------------------------------------------

/// **THE ARGUMENT, EXECUTED.** Hash-consing is a quotient, and a quotient exists only for an
/// EQUIVALENCE relation. `Literal`'s derived `PartialEq` is not one — this test runs the three
/// laws over the adversarial grid and finds reflexivity broken — so **no hash-consing scheme can
/// implement it**, not this one and not Lean's. `literal_identity_eq` IS one, over the same
/// grid. That is why there is one rule available and not two.
#[test]
fn only_one_of_the_two_literal_equalities_is_an_equivalence_relation() {
    let grid: Vec<Literal> = float_grid().into_iter().map(Literal::Float).collect();

    // The candidate rule satisfies all three laws.
    for a in &grid {
        assert!(literal_identity_eq(a, a), "reflexive at {a:?}");
        for b in &grid {
            assert_eq!(
                literal_identity_eq(a, b),
                literal_identity_eq(b, a),
                "symmetric at ({a:?}, {b:?})"
            );
            for c in &grid {
                if literal_identity_eq(a, b) && literal_identity_eq(b, c) {
                    assert!(
                        literal_identity_eq(a, c),
                        "transitive at ({a:?}, {b:?}, {c:?})"
                    );
                }
            }
        }
    }

    // The derived one does not, and the failure is REFLEXIVITY — the law `impl Eq for Literal {}`
    // claims by existing. Counted rather than asserted one way or the other, so this stays true
    // whichever rule `term.rs` ends up with.
    let irreflexive = grid.iter().filter(|a| *a != *a).count();
    println!("  literals that are not equal to THEMSELVES under the derived rule: {irreflexive}");
    assert_eq!(
        grid.iter().filter(|a| !literal_identity_eq(a, a)).count(),
        0,
        "the candidate rule is reflexive everywhere, which is what `Eq` means"
    );
}

/// The store's promise, stated as a decision procedure and checked in BOTH directions over
/// every ordered pair of the grid: **wherever the store says `eq_faithful`, its id equality and
/// `Term`'s `==` agree.**
///
/// This is green under BOTH rules. Under today's IEEE equality it is green because the five
/// adversarial members are exactly the ones `eq_faithful` refuses to vouch for; once `term.rs`
/// adopts `literal_identity_eq` it is green because they agree everywhere. A lock that survives
/// the fix is a lock that is measuring the invariant and not the bug.
#[test]
fn wherever_the_store_vouches_for_id_equality_it_agrees_with_the_kernel() {
    let grid = float_grid();
    let mut store = TermStore::new();
    let mut checked = 0usize;
    let mut vouched = 0usize;
    for &x in &grid {
        for &y in &grid {
            // Not bare literals only: the flag has to be COMPOSITIONAL, so each value is also
            // checked buried under an application and a binder.
            for wrap in 0..3 {
                let (ta, tb) = match wrap {
                    0 => (flit(x), flit(y)),
                    1 => (app(g("f"), flit(x)), app(g("f"), flit(y))),
                    _ => (lam("u", nat(), flit(x)), lam("u", nat(), flit(y))),
                };
                let a = store.intern(&ta);
                let b = store.intern(&tb);
                checked += 1;
                if store.eq_faithful(a) && store.eq_faithful(b) {
                    vouched += 1;
                    assert_eq!(
                        store.same(a, b),
                        ta == tb,
                        "the store vouched for id equality on ({ta}, {tb}) and then disagreed \
                         with `Term`'s own `==`"
                    );
                }
            }
        }
    }
    assert!(
        vouched > 0,
        "the grid must contain pairs the store DOES vouch for"
    );
    println!("  {vouched} of {checked} pairs vouched for, and every one of them agrees");
}

/// The other direction, which is the one that makes the flag worth having: **every disagreement
/// is at a term the store already refused to vouch for.** A disagreement the flag did not
/// predict would be the soundness hole.
///
/// The disagreement count is a RATCHET that may only go down: it is 5 ordered pairs today (three
/// NaN self-pairs, and `0.0`/`-0.0` in both orders) and 0 once `term.rs` adopts
/// `literal_identity_eq`.
#[test]
fn every_disagreement_about_a_float_is_one_the_store_predicted() {
    let grid = float_grid();
    let mut store = TermStore::new();
    let mut disagreements = 0usize;
    for &x in &grid {
        for &y in &grid {
            let ta = flit(x);
            let tb = flit(y);
            let a = store.intern(&ta);
            let b = store.intern(&tb);
            if store.same(a, b) != (ta == tb) {
                disagreements += 1;
                assert!(
                    !store.eq_faithful(a) || !store.eq_faithful(b),
                    "id equality and `Term` equality disagree about ({ta}, {tb}) and the store \
                     did NOT predict it — that is the hole `eq_faithful` exists to close"
                );
            }
        }
    }
    println!("  ordered pairs where the two equalities disagree: {disagreements}");
    assert!(
        disagreements <= 5,
        "this number may only go DOWN — it is 5 under IEEE equality (three NaN self-pairs and \
         `0.0`/`-0.0` in both orders) and 0 once `term.rs` adopts `literal_identity_eq`; it is \
         {disagreements}"
    );
}

/// `float_free` is the CONSERVATIVE guard and `eq_faithful` is the exact one, and the
/// relationship between them is an implication that must never invert: a term with no float at
/// all is always faithful, and a term may hold as many `1.5`s and infinities as it likes and
/// still be faithful even though it is not float-free.
#[test]
fn float_free_implies_eq_faithful_and_is_strictly_weaker() {
    let mut store = TermStore::new();
    let mut strictly_weaker = 0usize;
    for &x in &float_grid() {
        for t in [flit(x), app(g("f"), flit(x)), pi("u", flit(x), nat())] {
            let id = store.intern(&t);
            if store.float_free(id) {
                assert!(
                    store.eq_faithful(id),
                    "float-free must imply faithful, for {t}"
                );
            }
            if !store.float_free(id) && store.eq_faithful(id) {
                strictly_weaker += 1;
            }
        }
    }
    assert!(
        strictly_weaker > 0,
        "`eq_faithful` must admit terms `float_free` refuses, or it is the same flag twice"
    );
    // And every float-free term the rest of the suite builds is faithful.
    for t in [
        nat(),
        spine_at(8),
        arrow_at(8),
        peano(3),
        corpus_dependent_telescope(3),
    ] {
        let id = store.intern(&t);
        assert!(store.float_free(id) && store.eq_faithful(id), "{t}");
    }
}

/// **THE CROSS-LOCK, and the one that would catch a real drift.** The kernel's CONVERSION must
/// agree with `Term`'s own `==` on a pair of literals — whichever rule that is. Two subsystems
/// deciding literal equality differently is how `def_eq` starts certifying something `normalize`
/// would deny, and a float is where the two rules differ.
///
/// It is written as an agreement rather than as a verdict on purpose: it is green today, green
/// after `term.rs` adopts `literal_identity_eq`, and red only if `def_eq` and `==` ever part
/// company.
#[test]
fn the_kernels_conversion_agrees_with_term_equality_on_every_float_in_the_grid() {
    let ctx = std_ctx();
    let grid = float_grid();
    for &x in &grid {
        for &y in &grid {
            let ta = flit(x);
            let tb = flit(y);
            assert_eq!(
                logicaffeine_kernel::defeq_for_test(&ctx, &ta, &tb),
                ta == tb,
                "conversion and `Term` equality must be the SAME rule about ({ta}, {tb})"
            );
        }
    }
}

// --------------------------------------------------------------------------------------------
// THE TYPE CACHE — and the reason it may not be keyed the way Lean keys its own
// --------------------------------------------------------------------------------------------

/// **THE NEGATIVE, WRITTEN FIRST.** A cache keyed on term identity ALONE — the obvious port of
/// Lean's `inferType` cache — certifies a type the term does not have, because `Term` names its
/// variables and `Context::extend` SHADOWS.
///
/// Lean is not exposed to this: its `Expr` uses de Bruijn indices for bound variables and
/// globally unique `FVarId`s for free ones, so two occurrences under different binders are
/// DIFFERENT expressions. Ours are one node. Both kernels are asked about each scope, so the
/// premise is not one kernel's opinion.
#[test]
fn a_type_cache_keyed_on_the_term_alone_would_certify_the_wrong_type() {
    let base = std_ctx();
    let nat_scope = base.extend("x", nat());
    let bool_scope = base.extend("x", g("Bool"));
    let t = var("x");

    assert_eq!(infer_type(&nat_scope, &t).expect("x is bound"), nat());
    assert_eq!(infer_type(&bool_scope, &t).expect("x is bound"), g("Bool"));

    // BOTH KERNELS, on the closed witnesses of the same fact — `λx:Nat. x` and `λx:Bool. x`
    // differ only in the type their bound `x` gets, which is the hazard in closed form.
    //
    // It is stated on the CLOSED shape because the second kernel cannot see a `Context`'s local
    // bindings at all: `recheck::to_db` starts from an empty scope and rejects every free local
    // as "unbound", and `double_check` classifies that as `Disagree` — the alarm documented as
    // "must never fire on a valid proof" — for a judgement as plain as `x : Nat ⊢ x : Nat`.
    // That is reported to the re-checker's lane with the repro and the fix; asserting it here
    // would only pin the defect.
    for (ty, want) in [(nat(), nat()), (g("Bool"), g("Bool"))] {
        let closed = lam("x", ty.clone(), var("x"));
        assert!(
            matches!(double_check(&base, &closed), DoubleCheck::Agreed),
            "both kernels must agree about `λx:{ty}. x`"
        );
        assert_eq!(
            infer_type(&base, &closed).expect("well formed"),
            pi("x", ty, want),
            "and the bound `x` takes the binder's type, not the other binder's"
        );
    }

    // ONE node, both times. That is the whole hazard in one assertion.
    let mut store = TermStore::new();
    let id = store.intern(&t);
    assert_eq!(
        store.intern(&var("x")),
        id,
        "a named variable interns to one node whatever scope it occurs in"
    );

    // The naive cache, run rather than argued.
    let mut naive: HashMap<TermId, Term> = HashMap::new();
    naive.insert(id, infer_type(&nat_scope, &t).unwrap());
    assert_eq!(
        naive[&id],
        nat(),
        "the first scope's answer is cached under the term's identity"
    );
    assert_ne!(
        naive[&id],
        infer_type(&bool_scope, &t).unwrap(),
        "and it is the WRONG type in the second scope — a cache keyed on the term alone would \
         hand it over as the kernel's verdict"
    );

    // The environment key separates them, and it is the term's own free variables that do it.
    let nat_id = store.intern(&nat());
    let bool_id = store.intern(&g("Bool"));
    let e_nat = store.env_for(id, &mut |n| (n == "x").then_some(nat_id));
    let e_bool = store.env_for(id, &mut |n| (n == "x").then_some(bool_id));
    assert_ne!(
        e_nat, e_bool,
        "two scopes that type `x` differently are two environments"
    );
    assert_eq!(
        store.env_entries(e_nat).len(),
        1,
        "and the key holds `x` and nothing else"
    );
}

/// The BOUNDARY, one rung either side: a CLOSED subterm keys on the empty environment under
/// EVERY scope — which is where the cache pays — and a free variable the scope does not bind is
/// OMITTED rather than defaulted, so "unbound" and "bound" can never collide.
#[test]
fn the_environment_key_is_the_terms_free_variables_and_nothing_else() {
    let mut store = TermStore::new();
    let nat_id = store.intern(&nat());
    let bool_id = store.intern(&g("Bool"));

    // Closed: the same key under every scope, however rich.
    for closed in [nat(), peano(3), arrow_at(4), lam("x", nat(), var("x"))] {
        let id = store.intern(&closed);
        let bare = store.env_for(id, &mut |_| None);
        let rich = store.env_for(id, &mut |n| match n {
            "x" => Some(nat_id),
            "y" => Some(bool_id),
            _ => None,
        });
        assert_eq!(
            bare,
            EnvId::EMPTY,
            "a closed term keys on the empty environment: {closed}"
        );
        assert_eq!(
            bare, rich,
            "…under every scope, which is where the sharing pays"
        );
    }

    // Open: absent is not a type.
    let open = store.intern(&var("x"));
    let unbound = store.env_for(open, &mut |_| None);
    let bound = store.env_for(open, &mut |n| (n == "x").then_some(nat_id));
    assert_eq!(
        unbound,
        EnvId::EMPTY,
        "an unbound variable contributes no entry"
    );
    assert_ne!(
        unbound, bound,
        "so a refusal cached in one scope cannot leak into the other"
    );

    // A binding the term does not MENTION is not in its key — that is what makes the key
    // O(free variables) instead of O(context).
    let mentions_x = store.intern(&app(g("Succ"), var("x")));
    let just_x = store.env_for(mentions_x, &mut |n| (n == "x").then_some(nat_id));
    let x_and_y = store.env_for(mentions_x, &mut |n| match n {
        "x" => Some(nat_id),
        "y" => Some(bool_id),
        _ => None,
    });
    assert_eq!(
        just_x, x_and_y,
        "an unmentioned binding cannot change a term's type"
    );
}

/// Environments are hash-consed, so two contexts that agree are ONE key however they were built,
/// and a context that assigns one name two types is a caller that has not decided which scope it
/// is in.
#[test]
fn two_environments_that_agree_are_one_key_whatever_order_they_were_built_in() {
    let mut store = TermStore::new();
    let x = store.symbol("x");
    let y = store.symbol("y");
    let nat_id = store.intern(&nat());
    let bool_id = store.intern(&g("Bool"));

    let forwards = store.env(&[(x, nat_id), (y, bool_id)]);
    let backwards = store.env(&[(y, bool_id), (x, nat_id)]);
    assert_eq!(
        forwards, backwards,
        "an environment is a set of bindings, not a list"
    );
    assert_ne!(
        forwards,
        store.env(&[(x, bool_id), (y, nat_id)]),
        "…and the types matter"
    );
    assert_ne!(
        forwards,
        store.env(&[(x, nat_id)]),
        "…and so does what is absent"
    );
    assert_eq!(store.env(&[]), EnvId::EMPTY);
    assert_eq!(store.env_entries(forwards).len(), 2);
}

/// The refusal: an environment is a FUNCTION from names to types, and a duplicate is not
/// silently resolved in either direction.
#[test]
#[should_panic(expected = "one type per name")]
fn an_environment_that_assigns_one_name_two_types_is_refused() {
    let mut store = TermStore::new();
    let x = store.symbol("x");
    let nat_id = store.intern(&nat());
    let bool_id = store.intern(&g("Bool"));
    store.env(&[(x, nat_id), (x, bool_id)]);
}

/// **THE PROOF THE CACHE IS CORRECT**, run rather than argued: a memoised `infer_type` over
/// every combination of a term corpus and a scope corpus, compared against the kernel's own
/// answer on EVERY lookup — a hit that disagreed would be the kernel certifying an optimisation.
///
/// The scopes are chosen to be adversarial for a term-keyed cache: they shadow `x` at two
/// different types, they bind `y` and not `x`, and they bind neither.
///
/// "The same answer" is the same TYPE, which for a named kernel means α-equivalence. `infer_type`
/// opens a binder whose name is already in scope under a fresh spelling
/// (`type_checker::open_context_binder`), so `λ(x:Nat). x` is typed `Π(x:Nat). Nat` in the bare
/// scope and `Π(x':Nat). Nat` in a scope that binds `x`. The term has no free variable, so the
/// cache keys the two lookups identically, and the two answers are one type. A refusal is compared
/// exactly, and the comparison is shown to separate two types that differ in a domain.
#[test]
fn a_memoised_infer_type_never_answers_differently_from_the_kernel() {
    let base = std_ctx();
    let scopes: Vec<(&str, Context)> = vec![
        ("bare", base.clone()),
        ("x:Nat", base.extend("x", nat())),
        ("x:Bool", base.extend("x", g("Bool"))),
        (
            "x:Nat,y:Bool",
            base.extend("x", nat()).extend("y", g("Bool")),
        ),
        ("y:Nat", base.extend("y", nat())),
        // The SHADOW: the same name rebound over itself.
        (
            "x:Nat then x:Bool",
            base.extend("x", nat()).extend("x", g("Bool")),
        ),
    ];
    let terms: Vec<Term> = vec![
        var("x"),
        var("y"),
        var("z"),
        nat(),
        g("Bool"),
        peano(3),
        app(g("Succ"), var("x")),
        lam("x", nat(), var("x")),
        lam("w", nat(), var("x")),
        pi("_", nat(), var("x")),
        arrow_at(4),
        spine_at(3),
        Term::Lit(Literal::Int(7)),
    ];

    let call = KernelCall::begin();
    assert!(
        call.is_outermost(),
        "the cache belongs to the outermost call"
    );
    let mut store = TermStore::new();
    let mut cache: TypeCache<'_, Result<Term, String>> = TypeCache::new(&call);
    let mut agreements = 0usize;
    let mut respelled = 0usize;

    let same_answer =
        |store: &mut TermStore, a: &Result<Term, String>, b: &Result<Term, String>| match (a, b) {
            (Ok(a), Ok(b)) => {
                let (a, b) = (store.intern(a), store.intern(b));
                store.alpha_same(a, b)
            }
            (Err(a), Err(b)) => a == b,
            _ => false,
        };
    assert!(
        same_answer(
            &mut store,
            &Ok(pi("x", nat(), nat())),
            &Ok(pi("x'", nat(), nat()))
        ),
        "a binder spelled differently is the same type"
    );
    assert!(
        !same_answer(
            &mut store,
            &Ok(pi("x", nat(), nat())),
            &Ok(pi("x", g("Bool"), nat()))
        ),
        "a different domain is a different type, and the comparison must say so"
    );
    assert!(
        !same_answer(
            &mut store,
            &Ok(pi("x", nat(), var("x"))),
            &Ok(pi("y", nat(), var("x")))
        ),
        "a bound occurrence and a free one are different types"
    );
    assert!(
        !same_answer(
            &mut store,
            &Ok(nat()),
            &Err("UnboundVariable(\"z\")".to_string())
        ),
        "a type and a refusal are never the same answer"
    );

    for (label, ctx) in &scopes {
        // Exactly what a wired-in kernel would do: intern each binding's type ONCE as it enters
        // scope, so the key costs a lookup and not a walk.
        let mut bound: HashMap<&'static str, TermId> = HashMap::new();
        for name in ["x", "y", "z"] {
            if let Some(ty) = ctx.get(name) {
                let id = store.intern(ty);
                bound.insert(name, id);
            }
        }
        for t in &terms {
            let id = store.intern(t);
            let env = store.env_for(id, &mut |n| bound.get(n).copied());
            let direct = infer_type(ctx, t).map_err(|e| format!("{e:?}"));
            match cache.get(env, id).cloned() {
                Some(cached) => {
                    assert!(
                        same_answer(&mut store, &cached, &direct),
                        "the cache answered differently from the kernel for `{t}` in scope \
                         {label}: the cache says {cached:?}, the kernel says {direct:?}"
                    );
                    if cached != direct {
                        respelled += 1;
                    }
                    agreements += 1;
                }
                None => cache.insert(env, id, direct),
            }
        }
    }

    assert!(
        cache.hits() > 0,
        "the cache must actually be REUSED, or this proves nothing: {} lookups, {} decided",
        cache.hits() + cache.misses(),
        cache.misses()
    );
    println!(
        "  memoised infer_type: {} lookups, {} hits, {} decided, {agreements} verified against \
         the kernel, {respelled} of them with a binder spelled differently",
        cache.hits() + cache.misses(),
        cache.hits(),
        cache.misses()
    );
}

/// THE SHARING CLAIM, in the units the corpus case measures: the corpus's own shared term is one
/// `big` under the cache's key, not four.
///
/// `a_shared_subterm_is_checked_once_and_not_once_per_occurrence` measures the NORMAL FORM's node
/// count, which is `reduction.rs`'s ζ gate. This is the other half — the CHECK — and it is what
/// keying on `(environment, term)` buys: `quad b b b b` asks about `b` four times and the kernel
/// is consulted ONCE, because all four occurrences are one node under one environment.
#[test]
fn the_corpus_shared_term_is_checked_once_per_distinct_node_and_not_once_per_occurrence() {
    let base = std_ctx();
    let ctx = base.extend("u", nat());
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));

    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let mut cache: TypeCache<'_, String> = TypeCache::new(&call);

    let nat_id = store.intern(&nat());
    let big_id = store.intern(&big);
    let env = store.env_for(big_id, &mut |n| (n == "u").then_some(nat_id));

    // Four occurrences of the same value, exactly as the corpus's `let b := big in quad b b b b`
    // has once ζ has written them out.
    for _ in 0..4 {
        let direct = format!("{:?}", infer_type(&ctx, &big));
        match cache.get(env, big_id) {
            Some(cached) => assert_eq!(cached, &direct, "one node, one answer"),
            None => cache.insert(env, big_id, direct),
        }
    }
    assert_eq!(
        cache.misses(),
        1,
        "the kernel is consulted ONCE for four occurrences"
    );
    assert_eq!(cache.hits(), 3, "the other three are the cache");
    assert_eq!(cache.len(), 1, "and one node is one entry");

    // And the DAG says the same thing about the representation: 40 `Succ` applications share one
    // `Succ` node and one `u`, so the store holds far fewer nodes than a tree walk counts.
    assert_eq!(
        store.tree_size(big_id),
        nodes(&big) as u64,
        "the tree measure agrees"
    );
    assert!(
        store.dag_size(big_id) < nodes(&big),
        "the DAG is smaller than the tree it denotes: {} distinct nodes against {} tree nodes",
        store.dag_size(big_id),
        nodes(&big)
    );
}

/// The tripwire for the failure the lifetime rule exists to prevent: a cache told two answers
/// for one key SAYS SO rather than certifying one of them.
///
/// The lifetime itself is enforced by the borrow checker — `TypeCache<'call, _>` borrows the
/// `KernelCall` it was made under, so it cannot reach a `static` (that needs `'static`) or a
/// `Context` (that would need a lifetime parameter), and `KernelCall` is neither `Send` nor
/// `Sync` so it cannot be smuggled through a `Mutex`. This is the empirical half.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "two answers")]
fn a_type_cache_told_two_answers_for_one_key_says_so() {
    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let id = store.intern(&var("x"));
    let mut cache: TypeCache<'_, &'static str> = TypeCache::new(&call);
    cache.insert(EnvId::EMPTY, id, "Nat");
    cache.insert(EnvId::EMPTY, id, "Bool");
}

/// The call token's own bookkeeping, because a cache created at the wrong depth is a cache that
/// buys nothing and an `is_outermost` that lies is worse than none.
#[test]
fn a_kernel_call_knows_whether_it_is_the_outermost_one() {
    assert_eq!(
        KernelCall::depth(),
        0,
        "no kernel call is live in a fresh test thread"
    );
    let outer = KernelCall::begin();
    assert!(outer.is_outermost());
    assert_eq!(KernelCall::depth(), 1);
    {
        let inner = KernelCall::begin();
        assert!(
            !inner.is_outermost(),
            "a nested call is not the outermost one"
        );
        assert!(
            !outer.is_outermost(),
            "…and neither is the outer one while it is nested"
        );
        assert_eq!(KernelCall::depth(), 2);
    }
    assert_eq!(
        KernelCall::depth(),
        1,
        "the token is released on the way out"
    );
    assert!(outer.is_outermost());
    drop(outer);
    assert_eq!(KernelCall::depth(), 0);
}

// --------------------------------------------------------------------------------------------
// THE ζ GATE, against the reducer `reduction.rs` HAS — not the one it had
// --------------------------------------------------------------------------------------------

/// A toy reducer in exactly `reduce_step_opt`'s shape: `Option<Term>`, where `None` means THERE
/// WAS NO REDEX. Rewrites the leftmost `A` to `B` and nothing else, so every combination of
/// "the annotation stepped", "the value stepped", "the body stepped" and "nothing stepped" is
/// reachable.
fn toy_step(t: &Term) -> Option<Term> {
    match t {
        Term::Global(n) if n == "A" => Some(g("B")),
        Term::App(f, a) => match toy_step(f) {
            Some(nf) => Some(app(nf, (**a).clone())),
            None => toy_step(a).map(|na| app((**f).clone(), na)),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => match toy_step(param_type) {
            Some(nt) => Some(lam(param, nt, (**body).clone())),
            None => toy_step(body).map(|nb| lam(param, (**param_type).clone(), nb)),
        },
        _ => None,
    }
}

/// The two gates are ONE RULE. `reduce_step_at` is gone and `reduce_step_opt` reports progress,
/// so the arm `reduction.rs` should paste is `zeta_or_share_opt` — and it must decide exactly
/// what the `Term`-returning form decides, or the wiring would change the reducer's answer
/// rather than its cost.
///
/// The correspondence is exact and this checks both halves of it over a thousand random terms:
/// `Some(y)` means the `Term` form produces `y`, and `None` means the `Term` form hands its
/// input straight back — which is what "there was no redex" means.
#[test]
fn the_two_zeta_gates_are_the_same_rule() {
    let names = ["x", "y", "z", "b"];
    let mut kept = 0usize;
    let mut expanded = 0usize;
    for seed in 0..1000u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(7));
        let name = names[rng.pick(4) as usize];
        // Seed the corpus with `A`s so the toy reducer has something to do.
        let ty = if rng.pick(2) == 0 {
            g("A")
        } else {
            random_term(&mut rng, 2)
        };
        let value = if rng.pick(2) == 0 {
            app(g("A"), var("u"))
        } else {
            random_term(&mut rng, 2)
        };
        let body = match rng.pick(4) {
            // Many occurrences AND a redex — the arm that keeps the binding and still steps.
            0 => app4(g("quad"), var(name), var(name), var(name), g("A")),
            // Many occurrences and NOTHING to do — the arm that has to answer "no redex".
            1 => app4(g("quad"), var(name), var(name), var(name), var(name)),
            _ => random_term(&mut rng, 3),
        };

        let input = Term::Let {
            name: name.to_string(),
            ty: Box::new(ty.clone()),
            value: Box::new(value.clone()),
            body: Box::new(body.clone()),
        };
        let opt = zeta_or_share_opt(name, &ty, &value, &body, &toy_step);
        let plain = zeta_or_share(name, &ty, &value, &body, &|t| {
            toy_step(t).unwrap_or_else(|| t.clone())
        });
        match &opt {
            Some(y) => {
                assert_eq!(
                    *y, plain,
                    "seed {seed}: the two ζ gates produced different terms for {input}"
                );
                expanded += 1;
            }
            None => {
                assert_eq!(
                    plain, input,
                    "seed {seed}: the `Option` gate said THERE IS NO REDEX and the `Term` gate \
                     changed {input}"
                );
                kept += 1;
            }
        }
    }
    assert!(
        kept > 0,
        "some random `Let` must be a shared normal form, or the None arm is untested"
    );
    assert!(expanded > 0, "…and some must still step");
    println!("  ζ gate: {expanded} stepped, {kept} were already a shared normal form");
}

/// The three cases of the gate, named and pinned one at a time, because a random corpus proves
/// the two implementations agree and NOT that they implement the right rule.
#[test]
fn the_option_zeta_gate_decides_the_three_cases_it_claims_to() {
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));

    // (i) at most one occurrence → ζ fires on the UNREDUCED value, exactly as today.
    let once = app(g("Succ"), var("b"));
    assert_eq!(
        zeta_or_share_opt("b", &nat(), &big, &once, &toy_step),
        Some(app(g("Succ"), big.clone())),
        "a binding used once expands, and expands the value it was given"
    );
    assert_eq!(
        zeta_or_share_opt("b", &nat(), &big, &var("_"), &toy_step),
        Some(var("_")),
        "a binding used zero times expands too — the ascription dodge stays sugar"
    );

    // (ii) more than one occurrence, and something inside steps → the `Let` is KEPT and the
    // step is reported.
    let many_with_work = app4(g("quad"), var("b"), var("b"), var("b"), g("A"));
    let stepped = zeta_or_share_opt("b", &nat(), &big, &many_with_work, &toy_step)
        .expect("the body still has a redex");
    match &stepped {
        Term::Let {
            name, value, body, ..
        } => {
            assert_eq!(name, "b", "the binding survives as the sharing node");
            assert_eq!(**value, big, "and the value is written ONCE");
            assert_eq!(
                **body,
                app4(g("quad"), var("b"), var("b"), var("b"), g("B"))
            );
        }
        other => panic!("the shared binding must survive: {other}"),
    }

    // (iii) more than one occurrence and nothing steps → there is NO REDEX, said exactly.
    let many_normal = app4(g("quad"), var("b"), var("b"), var("b"), var("b"));
    assert_eq!(
        zeta_or_share_opt("b", &nat(), &big, &many_normal, &toy_step),
        None,
        "a shared `Let` whose parts are all normal IS the normal form"
    );

    // And the node count that is the corpus case's actual assertion.
    let shared = let_in("b", nat(), big.clone(), many_normal);
    assert_eq!(
        nodes(&shared),
        nodes(&big) + 11,
        "one `Let`, its annotation, the value ONCE, and a four-argument head"
    );
    assert!(
        nodes(&shared) <= nodes(&big) + 16,
        "which is inside the corpus ceiling of {}",
        nodes(&big) + 16
    );
}

// --------------------------------------------------------------------------------------------
// A DEPTH NOBODY MEASURES — the hole the sized stack does not close on its own
// --------------------------------------------------------------------------------------------

/// The wave-3 probe: one call, one context shape, in a process of its own, so a stack overflow is
/// an exit status here instead of a dead test run.
fn run_wave3_probe(kind: &str, depth: usize) -> String {
    let base = std_ctx();
    let ty = arrow_at(depth);
    let ctx = base.extend("x", ty);
    let t = var("x");
    let verdict = |r: Result<Term, logicaffeine_kernel::KernelError>| -> String {
        match r {
            Ok(v) => {
                // The ANSWER is itself `depth` deep; dropping it is a recursion too, and this
                // probe is about the call and not about `Drop`.
                std::mem::forget(v);
                "ACCEPTED".to_string()
            }
            Err(e) => {
                let text = format!("{e:?}");
                if text.contains("nests deeper") {
                    "REFUSED".to_string()
                } else {
                    "REJECTED".to_string()
                }
            }
        }
    };
    let answer = match kind {
        // The kernel as it stands: the door measured `x`, which is one node.
        "ctx_bare" => verdict(infer_type(&ctx, &t)),
        // Wave 2's sized stack, which also measures only the term it was handed.
        "ctx_sized" => verdict(
            deepen_stack(&t, || infer_type(&ctx, &t)).expect("an outermost call always runs"),
        ),
        // The sized stack TOLD about the depth the context carries.
        "ctx_told" => verdict(
            deepen_stack_with_depth(&t, depth, || infer_type(&ctx, &t))
                .expect("an outermost call always runs"),
        ),
        other => panic!("unknown wave-3 probe kind {other}"),
    };
    std::mem::forget(ctx);
    answer
}

/// The child half. Inert unless `W_PERF3_KIND` is set, so it costs a baseline run nothing.
#[test]
fn w_perf_wave3_probe_child() {
    let Ok(kind) = std::env::var("W_PERF3_KIND") else {
        return;
    };
    let depth: usize = std::env::var("W_PERF3_DEPTH").unwrap().parse().unwrap();
    let verdict = run_wave3_probe(&kind, depth);
    println!("W_PERF3_OK {kind} {depth} {verdict}");
    std::process::exit(0);
}

/// Run one wave-3 probe in a child process. `None` means the child DIED.
fn wave3_probe(kind: &str, depth: usize) -> Option<String> {
    let exe = std::env::current_exe().expect("the test binary knows its own path");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "w_perf_wave3_probe_child",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("W_PERF3_KIND", kind)
        .env("W_PERF3_DEPTH", depth.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("the probe child starts");
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let after = stdout.split("W_PERF3_OK ").nth(1)?;
    after.split_whitespace().nth(2).map(str::to_string)
}

/// **THE DEFECT, AND ITS FIX.** `infer_type`'s `Var` arm is `ctx.get(name).cloned()`, and
/// `Term`'s `Clone` is derived, so it recurses over the BINDING'S type — a depth neither
/// `MAX_TERM_DEPTH` nor [`deepen_stack`] ever saw, because both measure the term they were
/// handed and that term is `Var("x")`, one node deep.
///
/// This asserts the FIX rather than the defect, so it is green now and stays green when the
/// wiring lands: told the depth the context carries, the call answers. The bare and
/// term-sized verdicts are PRINTED beside it, so the log carries the evidence instead of a
/// claim, and the one hard cross-check is that no two of them ever ANSWER DIFFERENTLY.
#[test]
fn a_shallow_term_under_a_deep_binding_needs_a_stack_nobody_measured() {
    const DEEP: usize = 4000;
    let told = wave3_probe("ctx_told", DEEP)
        .expect("told the context's depth, a one-node term under a deep binding is answered");
    assert_eq!(
        told, "ACCEPTED",
        "`x : Nat → … → Nat` is well formed at any arity"
    );

    let bare = wave3_probe("ctx_bare", DEEP);
    let sized = wave3_probe("ctx_sized", DEEP);
    println!(
        "  a {DEEP}-deep BINDING under a one-node term: bare={bare:?} term-sized={sized:?} \
         told={told}"
    );
    for (label, other) in [("bare", &bare), ("term-sized", &sized)] {
        if let Some(v) = other {
            assert_eq!(
                *v, told,
                "the {label} call answered, and answered differently from the sized one"
            );
        }
    }
}

/// The boundary: the sizing has to come from the depth it was TOLD, and passing zero has to be
/// byte-identical to not passing anything — or a caller that does not have the number yet would
/// see its behaviour change.
#[test]
fn telling_the_sizer_nothing_is_exactly_the_old_behaviour() {
    let ctx = std_ctx();
    for t in [
        nat(),
        spine_at(8),
        arrow_at(64),
        corpus_dependent_telescope(4),
    ] {
        let plain = format!(
            "{:?}",
            deepen_stack(&t, || infer_type(&ctx, &t)).expect("outermost")
        );
        let zero = format!(
            "{:?}",
            deepen_stack_with_depth(&t, 0, || infer_type(&ctx, &t)).expect("outermost")
        );
        let told = format!(
            "{:?}",
            deepen_stack_with_depth(&t, 4096, || infer_type(&ctx, &t)).expect("outermost")
        );
        assert_eq!(
            plain, zero,
            "`extra_depth = 0` must be exactly `deepen_stack` for {t}"
        );
        assert_eq!(
            plain, told,
            "and a bigger stack must never change an answer for {t}"
        );
    }
}

/// The measurement behind the claim: where each discipline dies as the BINDING gets deeper.
/// This is the table the `Context`-side fix is argued from.
#[test]
#[ignore = "measurement: spawns ~60 child processes; run on purpose"]
fn w_perf_wave3_measure_the_context_depth_ceiling() {
    println!("\n  binding depth   bare          term-sized    told");
    println!("  ------------------------------------------------------------");
    for depth in [32usize, 128, 512, 1024, 2048, 3072, 4096, 8192] {
        let cell = |v: Option<String>| v.unwrap_or_else(|| "DIED".to_string());
        println!(
            "  {depth:<15} {:<13} {:<13} {}",
            cell(wave3_probe("ctx_bare", depth)),
            cell(wave3_probe("ctx_sized", depth)),
            cell(wave3_probe("ctx_told", depth)),
        );
    }
    println!(
        "  (the door admits all of these: it measured `Var(\"x\")`, which is one node deep)\n"
    );
}

// --------------------------------------------------------------------------------------------
// THE SECOND KERNEL AND THE LOCAL CONTEXT — a class of judgement where the alarm is noise
// --------------------------------------------------------------------------------------------

/// **A DEFECT THAT IS NOT IN THE CORPUS**, found while wiring the type cache and recorded here
/// with its evidence.
///
/// `recheck::to_db` lowers a named `Term` to de Bruijn starting from an EMPTY scope, and
/// `recheck` calls it as `to_db(term, &mut Vec::new())` with `infer(ctx, &[], &db)`. It
/// therefore cannot see a `Context`'s LOCAL bindings at all: every free local is "unbound local
/// variable", an `ReCheckError::ill`, which `double_check` classifies as
/// `DoubleCheck::Disagree` — the verdict whose own doc comment says "a soundness alarm that
/// must never fire on a valid proof". It fires on `x : Nat ⊢ x : Nat`.
///
/// It is a FALSE ALARM and not a hole, but it is the same shape as wave 2's finding about the
/// depth band: across an entire class of judgements — every term with a free variable the
/// context binds, which includes the corpus's own `a_shared_subterm_is_checked_once…` fixture
/// (`bctx = ctx.extend("u", nat())`) — the second kernel corroborates nothing, and it says so in
/// the voice reserved for a soundness failure. A reader who sees `Disagree` routinely stops
/// believing it.
///
/// What is asserted here is the half that is true under BOTH the current behaviour and the fix:
/// the second kernel covers the CLOSED form of each of these judgements and agrees about it. The
/// open-form census is PRINTED, because asserting it would pin the defect rather than the spec.
/// The spec — `assert!(!matches!(double_check(&scope, &t), DoubleCheck::Disagree(_)))` — is
/// handed to the re-checker's lane as its RED test.
#[test]
fn the_second_kernel_covers_the_closed_form_of_every_judgement_it_is_shown_here() {
    let base = std_ctx();
    let open: Vec<(&str, Term, Term)> = vec![
        ("x", nat(), var("x")),
        ("x", nat(), app(g("Succ"), var("x"))),
        (
            "u",
            nat(),
            (0..8).fold(var("u"), |acc, _| app(g("Succ"), acc)),
        ),
        ("x", nat(), lam("w", nat(), var("x"))),
        (
            "x",
            nat(),
            let_in("b", nat(), var("x"), app(g("Succ"), var("b"))),
        ),
    ];

    let mut alarms = 0usize;
    for (name, ty, body) in &open {
        let scope = base.extend(name, ty.clone());

        // The CLOSED transport of the same judgement: `Γ, x:A ⊢ t : B` iff `Γ ⊢ λx:A. t`.
        let closed = lam(name, ty.clone(), body.clone());
        assert!(
            infer_type(&base, &closed).is_ok(),
            "the closed transport of `{name} : {ty} ⊢ {body}` is well formed"
        );
        assert!(
            matches!(double_check(&base, &closed), DoubleCheck::Agreed),
            "both kernels must agree about the CLOSED form `λ{name}:{ty}. {body}`"
        );

        // …and the open form, which the first kernel also accepts.
        assert!(
            infer_type(&scope, body).is_ok(),
            "`{name} : {ty} ⊢ {body}` is well formed"
        );
        if matches!(double_check(&scope, body), DoubleCheck::Disagree(_)) {
            alarms += 1;
        }
    }
    println!(
        "  valid judgements under a local binding on which the SECOND kernel raises the \
         soundness alarm: {alarms} of {}",
        open.len()
    );
    println!(
        "  (root cause: `recheck::to_db(term, &mut Vec::new())` — the re-checker's scope is \
         seeded EMPTY, so a variable `Context` binds reads as unbound)"
    );
}

// --------------------------------------------------------------------------------------------
// T00-021's RATIO, DECOMPOSED — which Θ(size) walk costs what, and what is left if it goes
// --------------------------------------------------------------------------------------------

/// The four Θ(size) costs one β-step on a spine pays, as switches, so each can be priced alone.
#[derive(Clone, Copy)]
struct StepCosts {
    /// `type_checker::substitute` computes `free_vars(replacement)` before descending, so that
    /// `substitute_avoiding` can freshen a binder that would capture. On a spine the body is
    /// `Var("x")` — it has NO BINDER AT ALL — so the walk is pure waste, and it is a walk of the
    /// whole remaining spine.
    replacement_free_vars: bool,
    /// `substitute(Var("x"), "x", rest)` IS `rest.clone()`.
    substitute_clone: bool,
    /// The driver then drops the term it stepped from, and `Term`'s `Drop` is derived and
    /// recursive over its `Box`es.
    drop_previous: bool,
    /// Brent's cycle detection compares the new term against the saved tortoise, every step.
    tortoise_compare: bool,
}

/// One emulated pass down a spine, paying exactly the costs the switches select. The SHAPE is
/// the kernel's: peel the outermost `App`, produce the argument as the next term, drop the old
/// one, and run Brent's bookkeeping. Nothing here is the kernel's code — it is a MODEL, and the
/// measurement below checks the model against the kernel's own wall clock before reading
/// anything off it.
fn emulate_spine_reduction(depth: usize, costs: StepCosts) -> f64 {
    let mut current = spine_at(depth);
    let mut tortoise = spine_at(depth);
    let mut save_at = 1usize;
    let mut since_save = 0usize;
    let start = std::time::Instant::now();
    loop {
        let Term::App(f, a) = current else { break };
        if costs.replacement_free_vars {
            std::hint::black_box(occurs_free_in_term(&a, "\u{1}no-such-name"));
        }
        let next = if costs.substitute_clone {
            (*a).clone()
        } else {
            *a.clone()
        };
        if costs.drop_previous {
            drop(Term::App(f, a));
        } else {
            // The owned step: the shell is two nodes and the argument was MOVED, so the drop is
            // O(1) whatever the argument's size.
            drop(f);
            std::mem::forget(a);
        }
        current = next;
        if costs.tortoise_compare {
            std::hint::black_box(current == tortoise);
            since_save += 1;
            if since_save == save_at {
                tortoise = current.clone();
                since_save = 0;
                save_at *= 2;
            }
        }
    }
    start.elapsed().as_secs_f64()
}

/// **WHY T00-021's RATIO IS NOT A CONSTANT, ITEMISED.** The corpus case asks for `ratio ≤ 5.0`
/// when a spine's depth TRIPLES, and the kernel pays FOUR Θ(size) walks per β-step. Each is
/// priced here at both depths, so the cost of removing any one of them is a number rather than
/// an opinion — and so is the fact that removing three of the four leaves the ratio where it was,
/// because what is left is still Θ(size) per step over Θ(size) steps.
///
/// The row that matters most is the last: with all four gone the same reduction is LINEAR, and
/// the ratio is the 3× the case asks for. Three of the four can be removed inside `reduction.rs`
/// and `type_checker.rs`; the fourth — Brent's `current == tortoise` — cannot, because cycle
/// detection needs an equality test on the WHOLE term and a `Box`-recursive tree has no O(1) one.
/// A hash-consed store does: `same` is a `u32` compare and the structural hash is computed at
/// construction. That is the measured case for stage 2 of this module's wiring plan.
#[test]
#[ignore = "measurement: seconds of reduction; run on purpose"]
fn w_perf_wave3_measure_what_each_of_the_four_walks_costs() {
    let ctx = std_ctx();
    const NEAR: usize = 500;
    const FAR: usize = 1500;

    // Warm both paths.
    let warm = spine_at(64);
    let _ = deepen_stack(&warm, || normalize(&ctx, &warm));
    let _ = emulate_spine_reduction(
        64,
        StepCosts {
            replacement_free_vars: true,
            substitute_clone: true,
            drop_previous: true,
            tortoise_compare: true,
        },
    );

    let time_kernel = |depth: usize| -> f64 {
        let t = spine_at(depth);
        let mut best = f64::MAX;
        for _ in 0..9 {
            let start = std::time::Instant::now();
            let nf = deepen_stack(&t, || normalize(&ctx, &t)).expect("outermost");
            let took = start.elapsed().as_secs_f64();
            assert_eq!(nf, g("Zero"));
            best = best.min(took);
        }
        best
    };
    let emulate = |depth: usize, costs: StepCosts| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..9 {
            best = best.min(emulate_spine_reduction(depth, costs));
        }
        best
    };

    let all = StepCosts {
        replacement_free_vars: true,
        substitute_clone: true,
        drop_previous: true,
        tortoise_compare: true,
    };
    let rows: [(&str, StepCosts); 6] = [
        ("all four (the model of today)", all),
        (
            "— without substitute's free_vars(replacement)",
            StepCosts {
                replacement_free_vars: false,
                ..all
            },
        ),
        (
            "— without the substitute clone",
            StepCosts {
                substitute_clone: false,
                ..all
            },
        ),
        (
            "— without the recursive Drop",
            StepCosts {
                drop_previous: false,
                ..all
            },
        ),
        (
            "— without Brent's tortoise compare",
            StepCosts {
                tortoise_compare: false,
                ..all
            },
        ),
        (
            "OWNED step, no tortoise (the end state)",
            StepCosts {
                replacement_free_vars: false,
                substitute_clone: false,
                drop_previous: false,
                tortoise_compare: false,
            },
        ),
    ];

    let kn = time_kernel(NEAR);
    let kf = time_kernel(FAR);
    println!("\n  the kernel itself:");
    println!(
        "    normalize(spine {NEAR}) = {kn:.6}s   normalize(spine {FAR}) = {kf:.6}s   ratio \
         {:.2}× (the case asks for ≤ 5.00×)",
        kf / kn
    );
    println!("\n  the model, and what each walk costs:");
    println!(
        "    {:<46} {:>11} {:>11} {:>8}",
        "",
        format!("d={NEAR}"),
        format!("d={FAR}"),
        "ratio"
    );
    println!("    {}", "-".repeat(80));
    for (label, costs) in rows {
        let n = emulate(NEAR, costs);
        let f = emulate(FAR, costs);
        println!("    {label:<46} {n:>10.6}s {f:>10.6}s {:>7.2}×", f / n);
    }
    println!(
        "\n  (the model is a MODEL: it pays the same four walks per step and nothing else, so it \n   \
         should come in under the kernel's own time and grow the same way.)\n  \
         ON A LOADED BOX THESE SECONDS ARE NOISE: repeated runs of this identical binary have \n   \
         reported the kernel's own spine ratio anywhere from 6.9× to 52×. The number to read is \n   \
         in `the_spines_four_walks_are_quadratic_in_node_visits_and_the_owned_step_is_linear`, \n   \
         which counts NODE VISITS and is the same on an idle box and a hammered one.\n"
    );
}

/// The same decomposition in units a loaded box cannot move: **NODE VISITS**.
///
/// Wall clock on this machine is unusable while eleven other lanes are building — the timing
/// version above reported the kernel's own spine ratio anywhere between 6.9× and 52× on repeated
/// runs of the identical binary. Node visits are the algorithm, and they are the same number on
/// an idle box and a hammered one.
///
/// A `depth`-rung spine is `4·depth + 1` nodes and reduces in exactly `depth` β-steps, and at
/// step `k` the remaining spine is `4(depth−k)+1` nodes. Each of the four walks visits that many
/// nodes, per step. Summing gives Θ(depth²) — and the assertions below are that the four-walk
/// total really does grow as the square while the owned/descent total grows LINEARLY, which is
/// the whole of T00-021's second half in two numbers.
#[test]
fn the_spines_four_walks_are_quadratic_in_node_visits_and_the_owned_step_is_linear() {
    /// Nodes in `spine_at(d)`: one `App`, a three-node `λx:Nat. x`, and the rest.
    fn spine_nodes(d: usize) -> u64 {
        4 * d as u64 + 1
    }

    /// Node visits each walk performs over a whole reduction of a `depth`-rung spine.
    fn visits(depth: usize) -> (u64, u64, u64, u64, u64) {
        let (mut fv, mut clone, mut drops, mut tortoise, mut owned) = (0, 0, 0, 0, 0);
        let mut save_at = 1usize;
        let mut since_save = 0usize;
        let mut tortoise_depth = depth;
        for k in 0..depth {
            let remaining = depth - k; // the spine being stepped
            let produced = remaining - 1; // what the β-step produces
                                          // `substitute` walks the replacement to collect its free variables…
            fv += spine_nodes(produced);
            // …then `substitute(Var("x"), "x", rest)` IS `rest.clone()`…
            clone += spine_nodes(produced);
            // …and the driver drops the term it stepped from, whole.
            drops += spine_nodes(remaining);
            // Brent compares the new term against the saved tortoise, which agrees with it for
            // as many rungs as the shorter of the two has.
            tortoise += spine_nodes(produced.min(tortoise_depth));
            since_save += 1;
            if since_save == save_at {
                tortoise += spine_nodes(produced); // saving the tortoise is a clone
                tortoise_depth = produced;
                since_save = 0;
                save_at *= 2;
            }
            // An owned step moves the argument out and drops a two-node shell: O(1).
            owned += 4;
        }
        (fv, clone, drops, tortoise, owned)
    }

    let (n, f) = (500usize, 1500usize);
    let a = visits(n);
    let b = visits(f);
    let total = |v: (u64, u64, u64, u64, u64)| v.0 + v.1 + v.2 + v.3;

    println!("\n  node visits over a whole spine reduction");
    println!(
        "    {:<44} {:>14} {:>14} {:>8}",
        "",
        format!("depth {n}"),
        format!("depth {f}"),
        "ratio"
    );
    println!("    {}", "-".repeat(84));
    let row = |label: &str, x: u64, y: u64| {
        println!(
            "    {label:<44} {x:>14} {y:>14} {:>7.2}×",
            y as f64 / x as f64
        );
    };
    row("substitute's free_vars(replacement)", a.0, b.0);
    row("substitute's clone of the replacement", a.1, b.1);
    row("the driver's recursive Drop", a.2, b.2);
    row("Brent's tortoise compare (and its saves)", a.3, b.3);
    row("ALL FOUR", total(a), total(b));
    row("an OWNED step that moves instead of copying", a.4, b.4);
    println!(
        "    (the corpus case asks the kernel's wall clock for ≤ 5.00× at 3× the depth; a cost \n     \
         that grows 9× in node visits cannot be brought under it by any constant factor)\n"
    );

    // Each of the four is quadratic on its own, so removing any THREE leaves the ratio where it
    // was. That is the assertion that says "no constant factor reaches this".
    for (label, x, y) in [
        ("free_vars(replacement)", a.0, b.0),
        ("the substitute clone", a.1, b.1),
        ("the recursive Drop", a.2, b.2),
        ("Brent's tortoise compare", a.3, b.3),
    ] {
        let ratio = y as f64 / x as f64;
        assert!(
            (8.0..10.5).contains(&ratio),
            "{label} must be QUADRATIC — tripling the depth multiplies it by {ratio:.2}, and \
             the corpus case's ceiling is 5.00"
        );
    }

    let all_ratio = total(b) as f64 / total(a) as f64;
    assert!(
        all_ratio > 5.0,
        "the four together must exceed the corpus ceiling, or this test is measuring nothing: \
         {all_ratio:.2}×"
    );

    let owned_ratio = b.4 as f64 / a.4 as f64;
    assert!(
        (2.5..3.5).contains(&owned_ratio),
        "and an owned step must be LINEAR — tripling the depth must triple it, not {owned_ratio:.2}×"
    );
    assert!(
        owned_ratio <= 5.0,
        "which is inside the ceiling the corpus case asks for"
    );
}

/// **TWO IMPLEMENTATIONS OF ONE RULE, run against each other.** `term.rs`'s hand-written
/// `PartialEq for Literal` decides literal conversion in BOTH kernels, and
/// `term_store::literal_identity_eq` is a second implementation of the same rule written
/// separately — wildcard-free on the diagonal where `term.rs` falls through to `_ => false`.
/// They are compared over every ordered pair of a grid that contains three distinct NaN bit
/// patterns, both zeros, and one value of every other variant, so a drift between the store's
/// interning key and the kernel's conversion cannot be silent.
///
/// This is the cross-implementation half; `only_one_of_the_two_literal_equalities_is_an_
/// equivalence_relation` is the algebraic half, and `the_kernels_conversion_agrees_with_term_
/// equality_on_every_float_in_the_grid` is the kernel half.
#[test]
fn the_two_implementations_of_the_one_literal_rule_agree_on_every_pair() {
    let mut grid: Vec<Literal> = float_grid().into_iter().map(Literal::Float).collect();
    grid.extend([
        Literal::Int(0),
        Literal::Int(-1),
        Literal::Int(i64::MIN),
        Literal::Text(String::new()),
        Literal::Text("0".to_string()),
        Literal::Duration(0),
        Literal::Date(0),
        Literal::Moment(0),
        Literal::BigInt(BigInt::from_i64(i64::MAX)),
        Literal::Nat(BigInt::from_i64(0)),
        Literal::Nat(BigInt::from_i64(7)),
    ]);

    let mut store = TermStore::new();
    for a in &grid {
        for b in &grid {
            assert_eq!(
                literal_identity_eq(a, b),
                a == b,
                "the store's rule and `term.rs`'s rule disagree about ({a:?}, {b:?}) — the \
                 interning key and the kernel's conversion are then two different equalities"
            );
            // …and the STORE agrees with both, which is the property that lets id equality
            // stand in for `Term` equality.
            let ia = store.intern(&Term::Lit(a.clone()));
            let ib = store.intern(&Term::Lit(b.clone()));
            assert_eq!(
                store.same(ia, ib),
                literal_identity_eq(a, b),
                "interning must implement the same rule for ({a:?}, {b:?})"
            );
        }
    }
    // Different VARIANTS holding the same number are different literals, and stay so.
    assert!(!literal_identity_eq(
        &Literal::Int(0),
        &Literal::Nat(BigInt::from_i64(0))
    ));
    assert!(!literal_identity_eq(
        &Literal::Int(0),
        &Literal::Duration(0)
    ));
    assert!(!literal_identity_eq(&Literal::Date(0), &Literal::Moment(0)));
    assert!(!literal_identity_eq(&Literal::Float(0.0), &Literal::Int(0)));
}

/// The conversion cache gets the same lifetime discipline as the type cache, and the same
/// tripwire: a pair decided twice with different verdicts is a cache that has outlived the
/// environment it decided under, and it SAYS SO rather than certifying one of the two.
///
/// Conversion needs no [`EnvId`] in its key, and the reason is worth stating because it is what
/// makes the pair key safe: `def_eq` consults δ (the GLOBAL definitions) and the terms
/// themselves, never a local's type. Every kernel entry takes `&Context`, and a shared reference
/// cannot be displaced, extended in place or popped — so within one outermost call the global
/// environment is fixed, and one call is exactly how long this cache lives.
#[test]
fn a_scoped_conversion_cache_remembers_both_verdicts_and_lives_for_one_call() {
    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let a = store.intern(&app(g("Succ"), g("Zero")));
    let b = store.intern(&peano(1));
    let c = store.intern(&g("Nat"));
    assert_eq!(
        a, b,
        "`Succ Zero` and `peano(1)` are the same structure, so one node"
    );

    let mut conv = DefEqCache::scoped(&call);
    assert!(conv.is_empty());
    assert_eq!(conv.get(a, c), None, "an undecided pair is undecided");
    conv.insert(a, c, false);
    // NEGATIVES ARE KEPT — they are the expensive half, and Lean keeps them for the same reason.
    assert_eq!(conv.get(a, c), Some(false));
    assert_eq!(
        conv.get(c, a),
        Some(false),
        "conversion is symmetric, so the key is ordered"
    );
    conv.insert(a, b, true);
    assert_eq!(conv.get(b, a), Some(true));
    assert_eq!(conv.len(), 2);
    assert_eq!(conv.hits(), 3, "three lookups were answered from the cache");
    assert_eq!(conv.misses(), 1, "and one had to be decided");
}

/// The tripwire, fired.
#[test]
#[should_panic(expected = "both answers for one pair")]
fn a_scoped_conversion_cache_told_both_answers_for_one_pair_says_so() {
    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let a = store.intern(&g("Nat"));
    let b = store.intern(&g("Bool"));
    let mut conv = DefEqCache::scoped(&call);
    conv.insert(a, b, false);
    conv.insert(a, b, true);
}

/// PAST THE DOOR, SPEND NOTHING. A term deeper than `MAX_TERM_DEPTH` is refused by
/// `enter_kernel` before the kernel descends a rung, so sizing a stack for it reserves tens of
/// megabytes and starts a thread to be told "no". The sizer recognises that band and runs
/// inline, and the VERDICT is unchanged — which is the half that matters and the half asserted
/// here.
///
/// The reservation is therefore capped at `stack_bytes_for_depth(MAX_TERM_DEPTH)` however deep
/// the term handed in is, which is the property that keeps a hostile input from asking the
/// kernel to reserve a gigabyte.
#[test]
fn a_term_past_the_door_is_refused_without_a_stack_being_reserved_for_it() {
    let ctx = std_ctx();
    for depth in [2048usize, 4096, 16384] {
        let t = arrow_at(depth);
        let direct = format!("{:?}", infer_type(&ctx, &t));
        let sized = format!(
            "{:?}",
            deepen_stack(&t, || infer_type(&ctx, &t)).expect("outermost")
        );
        assert_eq!(
            direct, sized,
            "the sizer must not change a refusal at depth {depth}"
        );
        assert!(
            direct.contains("nests deeper"),
            "and the verdict is the door's REFUSAL"
        );
        // The answer arrives promptly because no thread was started for it; the assertion is on
        // the ANSWER, because a stopwatch on this box is not evidence.
        std::mem::forget(t);
    }
}

// --------------------------------------------------------------------------------------------
// THE ζ GATE MUST NOT STOP COMPUTATION — the regression this lane's own rule caused
// --------------------------------------------------------------------------------------------

/// **THE RED TEST FOR A DEFECT THIS LANE SHIPPED.** `lean_conformance_t02_declarations::a_let_rec_is_a_
/// fix_bound_by_a_let_and_a_plain_let_cannot_carry_it` is recorded `Native` in the verdicts and
/// went RED the moment the occurrence-gated ζ rule landed, and the cause is this lane's:
///
/// ```text
/// let twice := 4 in plus twice twice     ⟶  8
/// ```
///
/// `twice` occurs twice, so the gate holds the binding. Nothing inside then steps — `Nat` is
/// normal, `Succ⁴ Zero` is normal, and `plus twice twice` cannot ι-unroll because its structural
/// argument is a VARIABLE — so the rule reported "there is no redex" and `normalize` handed back
/// a stuck `Let` where the answer is `8`.
///
/// **"Many occurrences and nothing inside steps" is NOT "no redex".** ζ is always available, and
/// firing it can unblock ι exactly as it does here. The sharing may be kept only when the
/// EXPANSION is itself a normal form — which is the corpus's `quad b b b b`, whose head is a
/// declaration with no body and which cannot step however `b` is spelled.
#[test]
fn the_zeta_gate_expands_a_shared_binding_whose_expansion_can_still_compute() {
    let mut ctx = std_ctx();

    // `plus` as a `Fix` that matches on its first argument — the corpus's own shape.
    let plus = Term::Fix {
        name: "plus".to_string(),
        body: Box::new(lam(
            "n",
            nat(),
            lam(
                "m",
                nat(),
                Term::Match {
                    discriminant: Box::new(var("n")),
                    motive: Box::new(lam("_", nat(), nat())),
                    cases: vec![
                        var("m"),
                        lam(
                            "k",
                            nat(),
                            app(g("Succ"), app(app(var("plus"), var("k")), var("m"))),
                        ),
                    ],
                },
            ),
        )),
    };

    // `let twice := 4 in plus twice twice` — TWO occurrences, so the gate holds the binding, and
    // the body is stuck until it is released.
    let shared = let_in(
        "twice",
        nat(),
        peano(4),
        app(app(plus.clone(), var("twice")), var("twice")),
    );
    assert_eq!(
        occurrences_in_term(&app(app(plus.clone(), var("twice")), var("twice")), "twice"),
        Occurrences::Many,
        "the gate sees two occurrences, which is the shape that used to be held back"
    );
    assert_eq!(
        normalize(&ctx, &shared),
        peano(8),
        "a shared binding whose expansion can still COMPUTE must be expanded — ζ unblocks ι"
    );

    // …and the corpus's own shape must still be HELD, because its expansion is a normal form:
    // `quad` is a declaration with no body and cannot step however `b` is spelled.
    ctx.add_declaration(
        "quad",
        pi(
            "_",
            nat(),
            pi("_", nat(), pi("_", nat(), pi("_", nat(), nat()))),
        ),
    );
    let bctx = ctx.extend("u", nat());
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let held = let_in(
        "b",
        nat(),
        big.clone(),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    let nf = normalize(&bctx, &held);
    assert!(
        matches!(nf, Term::Let { .. }),
        "a shared binding whose expansion is ALREADY NORMAL keeps the binding: {nf}"
    );
    assert!(
        nodes(&nf) <= nodes(&big) + 16,
        "which is the corpus's node bound: {} against {}",
        nodes(&nf),
        nodes(&big) + 16
    );

    // The DIRECT gate, at the same two shapes, so the rule is pinned where it is written and not
    // only through `normalize`. The kernel's own one-step reducer is private, so the step handed
    // in here is "normalise and report whether anything moved" — which is all the gate asks of
    // it, and which is the kernel's real reduction rather than a toy.
    fn progress(ctx: &Context, t: &Term) -> Option<Term> {
        let n = normalize(ctx, t);
        if n == *t {
            None
        } else {
            Some(n)
        }
    }
    let plus_body = app(app(plus, var("twice")), var("twice"));
    assert!(
        zeta_or_share_opt("twice", &nat(), &peano(4), &plus_body, &|t| progress(
            &ctx, t
        ))
        .is_some(),
        "the gate must report a redex for a binding whose expansion can compute"
    );
    let quad_body = app4(g("quad"), var("b"), var("b"), var("b"), var("b"));
    assert_eq!(
        zeta_or_share_opt("b", &nat(), &big, &quad_body, &|t| progress(&bctx, t)),
        None,
        "and no redex for one whose expansion is already normal"
    );
}

/// The ζ probe must not be EXPONENTIAL in nesting. Each arm steps its body and then steps its
/// expansion, and without a guard those are two descents through the same nest of stuck shared
/// `Let`s — `T(k) = 2·T(k−1)`, which is a hang inside a single reduction step rather than a slow
/// answer, on input a caller chooses.
///
/// Sixteen nested shared bindings is `2^16` probes unguarded and a walk guarded. The assertion is
/// on the ANSWER and on the fact that it ARRIVES: a stopwatch on this box is not evidence, but a
/// test that never returns is unmistakable.
#[test]
fn a_nest_of_shared_bindings_does_not_make_the_zeta_probe_exponential() {
    let mut ctx = std_ctx();
    ctx.add_declaration("pair", pi("_", nat(), pi("_", nat(), nat())));
    let base = ctx.extend("u", nat());

    // `let b0 := Succ u in let b1 := pair b0 b0 in … let b15 := pair b14 b14 in pair b15 b15`
    // Every binding is used TWICE, so every one is held by the gate, and `pair` is a declaration
    // with no body so NOTHING in the nest can ever step.
    const NEST: usize = 16;
    let mut body = app(app(g("pair"), var("b15")), var("b15"));
    for i in (0..NEST).rev() {
        let value = if i == 0 {
            app(g("Succ"), var("u"))
        } else {
            app(
                app(g("pair"), var(&format!("b{}", i - 1))),
                var(&format!("b{}", i - 1)),
            )
        };
        body = let_in(&format!("b{i}"), nat(), value, body);
    }

    let nf = normalize(&base, &body);
    assert!(
        matches!(nf, Term::Let { .. }),
        "nothing in the nest can step, so the outermost binding is held: {nf}"
    );
    // Held ALL the way down: the normal form is still a nest, not an expansion.
    let mut depth = 0usize;
    let mut walk = &nf;
    while let Term::Let { body, .. } = walk {
        depth += 1;
        walk = body;
    }
    assert_eq!(depth, NEST, "every binding in the nest is still a binding");
    assert!(
        nodes(&nf) < 200,
        "and the shared form stays small: {} nodes for a nest whose expansion is 2^{NEST}",
        nodes(&nf)
    );
}

// ================================================================================================
// WAVE 4 · THE PURE β/ζ DOOR — the ALGORITHM change T00-021's ratio needs
// ================================================================================================
//
// `lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically` asks for
// ≤ 5× when a spine's depth TRIPLES. The tree reducer cannot reach it by any constant factor and
// the measurement says so: `w_perf_measure_the_spine_in_the_tree_and_in_the_store` prints 8.23×
// for the tree and 2.79× for the store on the SAME two depths. A ratio is immune to constant
// factors, so the representation has to change for that one assertion to move.
//
// What the store cannot do is δ, ι, the primitives, the quotient eliminator, the reflection
// builtins or `@[simp]`. So the door below does not try: it REFUSES any term those rules could
// touch and hands back `None`, and `reduction.rs` falls through to exactly the path it has today.
// A term it ADMITS is one whose only redexes are β and ζ — and on those the store is the same
// rule, the same normal form, and linear instead of quadratic.
//
// Lean's `Expr` is the source of the design. `Expr.Data` carries four "does this subterm contain
// X?" bits (`Lean/Expr.lean:121-124`) that `mkAppData` propagates with a single OR of the
// children's words (`Lean/Expr.lean:193`), and `Expr.lean:1661` shows what they are FOR —
// `if !e.hasFVar then false else …`, a traversal that stops before it starts. `ShapeMask` is that
// idea widened from four bits to one per node kind, and it is what makes the door's admission
// test O(1) rather than a walk.

use logicaffeine_kernel::term_store::{beta_zeta_normal_form, NodeShape};

/// The Globals the door's callers must treat as NOT inert, because `reduce_step_opt` has a rule
/// keyed on the NAME rather than on the shape: the reflection builtins, the native-evaluation
/// hook and the quotient eliminator. Reproduced here so the differential below cannot pass by
/// accident on a corpus that happens to avoid them.
const FOREIGN_HEADS: [&str; 12] = [
    "syn_size",
    "syn_max_var",
    "syn_step",
    "syn_beta",
    "syn_lift",
    "syn_subst",
    "syn_quote",
    "syn_diag",
    "syn_eval",
    "reduceBool",
    "Quot_lift",
    "Quot_rec",
];

/// The inertness predicate a caller owes the door: a `Global` is inert when δ cannot unfold it
/// and no name-keyed rule claims it.
fn inert_in(ctx: &Context) -> impl Fn(&str) -> bool + '_ {
    move |name: &str| ctx.get_definition_body(name).is_none() && !FOREIGN_HEADS.contains(&name)
}

/// **THE NEGATIVE, WRITTEN FIRST.** Every shape the store does not own must be REFUSED, not
/// silently mis-normalised. One case per refusing reason, and each names the rule it is ceding.
#[test]
fn the_pure_door_refuses_every_shape_whose_rules_it_does_not_own() {
    let ctx = std_ctx();
    let inert = inert_in(&ctx);
    let budget = 1u64 << 16;

    // A literal is what `try_primitive_reduce` keys on — it returns `None` unless the ARGUMENT
    // is a `Term::Lit`, so a term with no literal anywhere cannot reach a primitive.
    assert_eq!(
        beta_zeta_normal_form(
            &app(g("Succ"), Term::Lit(Literal::Nat(BigInt::from_i64(3)))),
            &inert,
            budget
        ),
        None,
        "a literal is the primitives' trigger and the store has no primitives"
    );
    // ι lives on `Match`.
    assert_eq!(
        beta_zeta_normal_form(
            &Term::Match {
                discriminant: Box::new(g("Zero")),
                motive: Box::new(nat()),
                cases: vec![g("Zero")],
            },
            &inert,
            budget
        ),
        None,
        "ι is the `Match` rule and the store has no ι"
    );
    // `Fix` and `MutualFix` are the termination guard's, not the store's.
    assert_eq!(
        beta_zeta_normal_form(
            &Term::Fix {
                name: "f".into(),
                body: Box::new(var("f"))
            },
            &inert,
            budget
        ),
        None,
        "unrolling asks `termination::structural_columns` which the store cannot call"
    );
    assert_eq!(
        beta_zeta_normal_form(
            &Term::MutualFix {
                defs: vec![("f".into(), var("f"))],
                index: 0
            },
            &inert,
            budget
        ),
        None
    );
    // A universe-polymorphic reference: δ with a level substitution, and `Illformed` → `Hole`.
    assert_eq!(
        beta_zeta_normal_form(
            &Term::Const {
                name: "id".into(),
                levels: vec![logicaffeine_kernel::Universe::Type(0)]
            },
            &inert,
            budget
        ),
        None,
        "`Const` is δ plus the level well-formedness test, neither of which the store has"
    );
    assert_eq!(beta_zeta_normal_form(&Term::Hole, &inert, budget), None);

    // A δ-REDUCIBLE global: the shape is admissible, the NAME is not.
    let mut with_def = std_ctx();
    with_def.add_definition("two".to_string(), nat(), peano(2));
    let inert_def = inert_in(&with_def);
    assert!(
        !inert_def("two"),
        "the predicate the caller owes must refuse a name δ can unfold"
    );
    assert_eq!(
        beta_zeta_normal_form(&app(g("Succ"), g("two")), &inert_def, budget),
        None,
        "δ would fire on `two` and the store would hand back an unreduced term as a normal form"
    );
    // …and with the SAME term under a context where the name has no body, the door opens.
    assert!(
        beta_zeta_normal_form(&app(g("Succ"), g("two")), &inert, budget).is_some(),
        "the refusal is about the NAME's meaning in this context, not about the spelling"
    );

    // Every reflection head is refused even though its shape is a bare `Global`.
    for head in FOREIGN_HEADS {
        assert_eq!(
            beta_zeta_normal_form(&app(g(head), var("x")), &inert, budget),
            None,
            "`{head}` has a name-keyed rule in `reduce_step_opt`"
        );
    }
}

/// **THE AGREEMENT.** Wherever the door opens, its normal form is the one `normalize` produces —
/// not α-equivalent to it, EQUAL to it. Over a thousand random terms drawn from the shapes it
/// admits, plus every shape the rest of this file builds.
///
/// The corpus is pre-filtered by the STORE, not by a stopwatch: a term the store normalises
/// within its budget is one whose β/ζ reduction terminates, and both reducers are head-first, so
/// `normalize` terminates on it too. A term the store cannot finish is skipped rather than
/// handed to a reducer that would grind to `MAX_REDUCTION_STEPS`.
#[test]
fn the_pure_door_agrees_with_the_kernel_on_every_term_it_admits() {
    let ctx = std_ctx();
    let inert = inert_in(&ctx);
    let mut admitted = 0usize;
    let mut refused = 0usize;

    let mut cases: Vec<Term> = vec![
        spine_at(1),
        spine_at(7),
        spine_at(64),
        arrow_at(9),
        app(
            lam("x", nat(), app(var("x"), var("x"))),
            lam("y", nat(), var("y")),
        ),
        let_in(
            "b",
            nat(),
            peano(3),
            app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
        ),
        let_in("b", nat(), peano(3), app(g("Succ"), var("b"))),
        // A β step that would CAPTURE without freshening: `(λf. λx. f x) x`.
        app(
            lam("f", nat(), lam("x", nat(), app(var("f"), var("x")))),
            var("x"),
        ),
        // Reduction under a binder, and in a Π domain.
        lam(
            "u",
            app(lam("x", nat(), var("x")), nat()),
            app(lam("x", nat(), var("x")), var("u")),
        ),
        pi("u", app(lam("x", nat(), var("x")), nat()), nat()),
    ];
    for seed in 0..1000u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(7));
        cases.push(pure_random_term(&mut rng, 4));
    }

    for t in &cases {
        // The store's own verdict, used as the termination filter.
        let mut store = TermStore::new();
        let id = store.intern(t);
        let terminates = matches!(
            store.beta_zeta_normalize(id, 1 << 14),
            StoreNormalForm::Reached(_)
        );
        match beta_zeta_normal_form(t, &inert, 1 << 14) {
            Some(fast) => {
                admitted += 1;
                assert!(
                    terminates,
                    "the door only answers when the store reached a normal form"
                );
                let slow = normalize(&ctx, t);
                assert_eq!(fast, slow, "the door and `normalize` disagree about {t}");
            }
            None => refused += 1,
        }
    }
    assert!(
        admitted > 500,
        "the door must ADMIT most of the corpus, not refuse it: {admitted}"
    );
    assert!(
        refused > 0,
        "and it must refuse some of it, or the filter is vacuous"
    );
    println!("  pure door: {admitted} admitted and checked against `normalize`, {refused} refused");
}

/// Random terms restricted to the shapes the door admits — no `Lit`, no `Match`, no `Fix`, no
/// `MutualFix`, no `Const`, no `Hole`, and only Globals that are inert in `std_ctx`.
fn pure_random_term(rng: &mut Lcg, depth: u32) -> Term {
    const NAMES: [&str; 3] = ["x", "y", "z"];
    const GLOBALS: [&str; 4] = ["Nat", "Zero", "Succ", "Bool"];
    if depth == 0 {
        return match rng.pick(3) {
            0 => var(NAMES[rng.pick(3) as usize]),
            1 => g(GLOBALS[rng.pick(4) as usize]),
            _ => Term::Sort(logicaffeine_kernel::Universe::Type(0)),
        };
    }
    match rng.pick(6) {
        0 | 1 => app(
            pure_random_term(rng, depth - 1),
            pure_random_term(rng, depth - 1),
        ),
        2 => lam(
            NAMES[rng.pick(3) as usize],
            pure_random_term(rng, depth - 1),
            pure_random_term(rng, depth - 1),
        ),
        3 => pi(
            NAMES[rng.pick(3) as usize],
            pure_random_term(rng, depth - 1),
            pure_random_term(rng, depth - 1),
        ),
        4 => let_in(
            NAMES[rng.pick(3) as usize],
            pure_random_term(rng, depth - 1),
            pure_random_term(rng, depth - 1),
            pure_random_term(rng, depth - 1),
        ),
        _ => pure_random_term(rng, depth - 1),
    }
}

/// **THE CORPUS BAR, THROUGH THE DOOR.** This is the assertion
/// `a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically` makes, measured on the path
/// the handoff installs: tripling the depth must cost at most 5×.
///
/// Both columns are a MINIMUM over repetitions after a warm-up, because a minimum is the closest
/// a loaded box gets to the cost of the algorithm, and the timed region includes interning and
/// extraction — the WHOLE cost of the representation, not just the reduction.
#[test]
fn the_pure_door_triples_the_corpus_spine_in_under_five() {
    const NEAR: usize = 500;
    const FAR: usize = 1500;
    const REPS: usize = 7;
    let ctx = std_ctx();
    let inert = inert_in(&ctx);

    let time = |t: &Term| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..REPS {
            let start = std::time::Instant::now();
            let nf = beta_zeta_normal_form(t, &inert, 1 << 24).expect("the spine is β-pure");
            let took = start.elapsed().as_secs_f64();
            assert_eq!(nf, g("Zero"), "and it reaches the corpus's normal form");
            best = best.min(took);
        }
        best
    };
    let _ = time(&spine_at(40));
    let near = time(&spine_at(NEAR));
    let far = time(&spine_at(FAR));
    let ratio = far / near;
    assert!(
        ratio <= 5.0,
        "tripling the spine's depth must cost at most 5× on the door's path; it cost {ratio:.2}× \
         ({near:.6}s at depth {NEAR}, {far:.6}s at depth {FAR})"
    );
    println!("  pure door on the corpus spine: {ratio:.2}× for 3× the depth (ceiling 5.00×)");
}

/// The door NEVER invents a binder name. The store's `substitute` freshens with a `✦`-suffixed
/// name when a substitution would capture, and a normal form carrying one is α-equivalent to the
/// kernel's but not EQUAL to it — so the door refuses rather than hand back a different spelling.
///
/// Both halves: a term whose reduction needs no freshening is admitted and matches `normalize`
/// exactly; one whose reduction does need it is either refused or still matches.
#[test]
fn the_pure_door_never_hands_back_a_name_the_kernel_would_not_have_written() {
    let ctx = std_ctx();
    let inert = inert_in(&ctx);
    // `(λf. λx. f x) x` — substituting the free `x` under the binder `x` must rename something.
    let capturing = app(
        lam("f", nat(), lam("x", nat(), app(var("f"), var("x")))),
        var("x"),
    );
    let slow = normalize(&ctx, &capturing);
    match beta_zeta_normal_form(&capturing, &inert, 1 << 16) {
        Some(fast) => assert_eq!(fast, slow, "an admitted answer is the kernel's answer"),
        None => {}
    }
    assert!(
        !format!("{slow}").contains('✦'),
        "and whatever the door did, the kernel's own answer never carries a store-fresh name"
    );

    // The shape that must stay admitted: nothing to capture, so nothing to rename.
    let plain = app(
        lam("f", nat(), lam("x", nat(), app(var("f"), var("x")))),
        var("q"),
    );
    assert_eq!(
        beta_zeta_normal_form(&plain, &inert, 1 << 16),
        Some(normalize(&ctx, &plain)),
        "a substitution that cannot capture must not be refused"
    );
}

/// The budget is EXACT and reported honestly: a `depth`-rung spine is `depth` β-steps, so one
/// less is a refusal and exactly `depth` is an answer. A door that rounded its budget would hide
/// a non-terminating reduction behind a normal form.
#[test]
fn the_pure_doors_budget_is_exact_at_the_rung() {
    let ctx = std_ctx();
    let inert = inert_in(&ctx);
    for depth in [1usize, 2, 17, 300] {
        let t = spine_at(depth);
        assert_eq!(
            beta_zeta_normal_form(&t, &inert, depth as u64 - 1),
            None,
            "a {depth}-rung spine cannot be normalised in {} steps",
            depth - 1
        );
        assert_eq!(
            beta_zeta_normal_form(&t, &inert, depth as u64),
            Some(g("Zero")),
            "and it is normalised in exactly {depth}"
        );
    }
}

/// **BOTH KERNELS.** The door's normal form is a term, and a term the main kernel certifies must
/// be a term the de Bruijn re-checker certifies too. Asserted on a term that is `Let`-headed in
/// its normal form, because a surviving `Let` is exactly what the ζ gate made possible and
/// exactly what the second kernel has to see through.
#[test]
fn both_kernels_agree_about_a_term_the_pure_door_normalised() {
    let mut ctx = std_ctx();
    ctx.add_declaration(
        "quad",
        pi(
            "_",
            nat(),
            pi("_", nat(), pi("_", nat(), pi("_", nat(), nat()))),
        ),
    );
    let inert = inert_in(&ctx);
    let shared = let_in(
        "b",
        nat(),
        peano(3),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    let nf = beta_zeta_normal_form(&shared, &inert, 1 << 16).expect("β/ζ-pure");
    assert_eq!(
        nf,
        normalize(&ctx, &shared),
        "the door and the kernel agree"
    );
    assert!(
        matches!(nf, Term::Let { .. }),
        "the gate held the binding: {nf}"
    );
    assert_eq!(
        double_check(&ctx, &nf),
        DoubleCheck::Agreed,
        "and the second kernel types the shared normal form"
    );
    assert_eq!(
        double_check(&ctx, &shared),
        DoubleCheck::Agreed,
        "as it does the term it came from"
    );
}

/// The annotation the door is built on, checked against an independent walk on every shape.
/// `ShapeMask` is an O(1) answer and a walk is the O(size) one; they must never differ.
#[test]
fn the_shape_mask_is_exactly_the_set_of_node_kinds_a_walk_would_find() {
    fn walk(t: &Term, seen: &mut Vec<NodeShape>) {
        let s = match t {
            Term::Annotated { term, .. } => return walk(term, seen),
            Term::Sort(_) => NodeShape::Sort,
            Term::Var(_) => NodeShape::Var,
            Term::Global(_) => NodeShape::Global,
            Term::Const { .. } => NodeShape::Const,
            Term::Pi { .. } => NodeShape::Pi,
            Term::Lambda { .. } => NodeShape::Lambda,
            Term::App(_, _) => NodeShape::App,
            Term::Match { .. } => NodeShape::Match,
            Term::Fix { .. } => NodeShape::Fix,
            Term::MutualFix { .. } => NodeShape::MutualFix,
            Term::Let { .. } => NodeShape::Let,
            Term::Lit(_) => NodeShape::Lit,
            Term::Hole => NodeShape::Hole,
        };
        if !seen.contains(&s) {
            seen.push(s);
        }
        match t {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole
            | Term::Annotated { .. } => {}
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                walk(param_type, seen);
                walk(body_type, seen);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                walk(param_type, seen);
                walk(body, seen);
            }
            Term::App(f, a) => {
                walk(f, seen);
                walk(a, seen);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                walk(discriminant, seen);
                walk(motive, seen);
                for c in cases {
                    walk(c, seen);
                }
            }
            Term::Fix { body, .. } => walk(body, seen),
            Term::MutualFix { defs, .. } => {
                for (_, b) in defs {
                    walk(b, seen);
                }
            }
            Term::Let {
                ty, value, body, ..
            } => {
                walk(ty, seen);
                walk(value, seen);
                walk(body, seen);
            }
        }
    }

    const EVERY: [NodeShape; 13] = [
        NodeShape::Sort,
        NodeShape::Var,
        NodeShape::Global,
        NodeShape::Const,
        NodeShape::Pi,
        NodeShape::Lambda,
        NodeShape::App,
        NodeShape::Match,
        NodeShape::Fix,
        NodeShape::MutualFix,
        NodeShape::Let,
        NodeShape::Lit,
        NodeShape::Hole,
    ];

    let mut store = TermStore::new();
    let mut covered: Vec<NodeShape> = Vec::new();
    let mut cases: Vec<Term> = vec![
        spine_at(5),
        arrow_at(4),
        peano(3),
        Term::Hole,
        Term::Lit(Literal::Nat(BigInt::from_i64(1))),
        Term::Const {
            name: "id".into(),
            levels: vec![logicaffeine_kernel::Universe::Type(0)],
        },
        corpus_dependent_telescope(2),
    ];
    for seed in 0..400u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(11));
        cases.push(random_term(&mut rng, 4));
    }
    for t in &cases {
        let id = store.intern(t);
        let mask = store.shapes(id);
        let mut seen = Vec::new();
        walk(t, &mut seen);
        for s in EVERY {
            assert_eq!(
                mask.contains(s),
                seen.contains(&s),
                "the mask and the walk disagree about {s:?} in {t}"
            );
            if seen.contains(&s) && !covered.contains(&s) {
                covered.push(s);
            }
        }
    }
    assert_eq!(
        covered.len(),
        EVERY.len(),
        "every shape must occur somewhere: {covered:?}"
    );
}

/// The other annotation: the set of `Global` and `Const` NAMES anywhere below — the δ surface
/// the door's inertness test is asked about. Checked against an independent walk, and required
/// to be a SET (no duplicates, sorted), because the door iterates it.
#[test]
fn the_constant_set_is_exactly_the_names_a_walk_would_find() {
    fn walk(t: &Term, acc: &mut Vec<String>) {
        match t {
            Term::Global(n) => acc.push(n.clone()),
            Term::Annotated { term, .. } => walk(term, acc),
            Term::Const { name, .. } => acc.push(name.clone()),
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                walk(param_type, acc);
                walk(body_type, acc);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                walk(param_type, acc);
                walk(body, acc);
            }
            Term::App(f, a) => {
                walk(f, acc);
                walk(a, acc);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                walk(discriminant, acc);
                walk(motive, acc);
                for c in cases {
                    walk(c, acc);
                }
            }
            Term::Fix { body, .. } => walk(body, acc),
            Term::MutualFix { defs, .. } => {
                for (_, b) in defs {
                    walk(b, acc);
                }
            }
            Term::Let {
                ty, value, body, ..
            } => {
                walk(ty, acc);
                walk(value, acc);
                walk(body, acc);
            }
        }
    }
    let mut store = TermStore::new();
    let mut cases: Vec<Term> = vec![
        spine_at(4),
        peano(5),
        corpus_dependent_telescope(2),
        Term::Const {
            name: "id".into(),
            levels: vec![logicaffeine_kernel::Universe::Type(0)],
        },
        var("x"),
    ];
    for seed in 0..400u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(13));
        cases.push(random_term(&mut rng, 4));
    }
    let mut nonempty = 0usize;
    for t in &cases {
        let id = store.intern(t);
        let got: Vec<String> = store.constants(id).iter().map(|s| s.to_string()).collect();
        let mut want = Vec::new();
        walk(t, &mut want);
        want.sort();
        want.dedup();
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted, "the constant set must be sorted, for {t}");
        assert_eq!(
            got, want,
            "the constant set and the walk disagree about {t}"
        );
        if !got.is_empty() {
            nonempty += 1;
        }
    }
    assert!(
        nonempty > 0,
        "the corpus must contain constants, or the check is vacuous"
    );
}

// ================================================================================================
// WAVE 4 · THE FLOAT RULE IS SETTLED — the store and the kernel now hold ONE opinion
// ================================================================================================
//
// `term.rs` adopted bit-exact `Float` equality this wave, which is the rule
// `literal_identity_eq` has always stated and the only one a hash-consed store can implement:
// interning is a QUOTIENT and a quotient exists only for an equivalence relation, which IEEE `==`
// is not (`NaN != NaN` breaks reflexivity; `0.0 == -0.0` holds between distinct encodings).
//
// LEAN AGREES, and does it by not having the question. `Expr.Literal` has exactly two
// constructors — `natVal` and `strVal` (`Lean/Expr.lean:16-21`) — with a DERIVED `BEq` and a
// structural `Hashable` (`:23-27`), so the key its `ShareCommon` hash-consing uses is total and
// reflexive by construction. `Float` is not a literal at all: it is a structure over an opaque
// `floatSpec.float` whose `beq` is an `@[extern]` opaque the kernel cannot reduce, carrying the
// comment "Note: this is not reflexive since `NaN != NaN`" (`Init/Data/Float.lean:59-60`). So in
// Lean, IEEE equality never enters definitional equality — exactly the separation we now have,
// reached from the other side.
//
// The consequence is that the `float_free` caveat `TermId`'s doc carries is RETIRED as a
// correctness caveat: id equality is now a faithful decision procedure for `Term` equality on
// EVERY term, with no exception. That is a stronger promise than the store has ever made, and
// these are the locks that hold it.

/// The ratchet `every_disagreement_about_a_float_is_one_the_store_predicted` leaves open, CLOSED.
/// It allows up to 5 disagreeing ordered pairs; there are now none, at any of the three
/// wrappings, and none is what the invariant actually says.
#[test]
fn id_equality_and_term_equality_now_agree_about_every_float_with_no_exception() {
    let grid = float_grid();
    let mut store = TermStore::new();
    let mut pairs = 0usize;
    for &x in &grid {
        for &y in &grid {
            for wrap in 0..4 {
                let (ta, tb) = match wrap {
                    0 => (flit(x), flit(y)),
                    1 => (app(g("f"), flit(x)), app(g("f"), flit(y))),
                    2 => (lam("u", nat(), flit(x)), lam("u", nat(), flit(y))),
                    _ => (
                        let_in("u", nat(), flit(x), app(var("u"), flit(x))),
                        let_in("u", nat(), flit(y), app(var("u"), flit(y))),
                    ),
                };
                let a = store.intern(&ta);
                let b = store.intern(&tb);
                pairs += 1;
                assert_eq!(
                    store.same(a, b),
                    ta == tb,
                    "id equality and `Term` equality disagree about ({ta}, {tb}) — the store and \
                     the kernel are holding two opinions about `Float` again"
                );
            }
        }
    }
    assert!(
        pairs > 500,
        "the grid must be large enough to contain the adversarial names"
    );
    println!("  {pairs} ordered pairs, zero disagreements — the float caveat is retired");
}

/// The same promise stated the way a caller uses it: **interning ROUND-TRIPS and identity is
/// reflexive**, on a corpus that deliberately buries every adversarial float under binders.
/// `float_free` is now a report about IEEE `==`, not a guard a caller has to consult.
#[test]
fn interning_round_trips_and_is_reflexive_on_every_float_the_grid_can_build() {
    let mut store = TermStore::new();
    for &x in &float_grid() {
        for t in [
            flit(x),
            app(g("f"), flit(x)),
            lam("u", nat(), flit(x)),
            pi("u", flit(x), nat()),
            let_in("u", nat(), flit(x), app(var("u"), flit(x))),
            app(app(g("f"), flit(x)), flit(-x)),
        ] {
            let id = store.intern(&t);
            assert_eq!(store.extract(id), t, "interning must round-trip {t}");
            assert!(store.same(id, id), "identity is reflexive");
            assert_eq!(
                t,
                t.clone(),
                "and so is `Term`'s own equality, now, for {t}"
            );
            assert_eq!(
                store.intern(&t),
                id,
                "re-interning the same bits is the same id"
            );
        }
    }
}

/// **THE STRONGEST FORM.** `same(a, b)` is a total, faithful decision procedure for `Term`'s
/// `==` — over a mixed corpus, on every ordered pair, with no `float_free` precondition.
#[test]
fn id_equality_decides_term_equality_on_every_ordered_pair_of_a_mixed_corpus() {
    let mut store = TermStore::new();
    let mut corpus: Vec<Term> = Vec::new();
    for &x in &float_grid() {
        corpus.push(flit(x));
        corpus.push(app(g("f"), flit(x)));
    }
    for seed in 0..60u64 {
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(17));
        corpus.push(random_term(&mut rng, 3));
    }
    let ids: Vec<_> = corpus.iter().map(|t| store.intern(t)).collect();
    let mut equal = 0usize;
    for (i, a) in corpus.iter().enumerate() {
        for (j, b) in corpus.iter().enumerate() {
            let same = store.same(ids[i], ids[j]);
            assert_eq!(same, a == b, "({a}) vs ({b})");
            if same {
                equal += 1;
            }
        }
    }
    assert!(
        equal >= corpus.len(),
        "reflexivity alone gives one equal pair per term"
    );
}

// ------------------------------------------------------------------------------------------------
// THE CONTEXT-AWARE DOOR — the form the handoff installs, with no policy left to the caller
// ------------------------------------------------------------------------------------------------

use logicaffeine_kernel::term_store::beta_zeta_normal_form_in_context;

/// Every head `reduce_step_opt` keys a rule on by NAME rather than by shape, harvested from
/// `reduction.rs` by reading its match arms. The list is evidence, not a mechanism: the door's
/// criterion is POSITIVE (constructor or inductive), and this is the executable check that the
/// positive criterion actually excludes all of them.
const NAME_KEYED_RULE_HEADS: [&str; 33] = [
    "syn_size",
    "syn_max_var",
    "syn_step",
    "syn_quote",
    "syn_diag",
    "syn_beta",
    "syn_lift",
    "syn_subst",
    "syn_eval",
    "concludes",
    "try_refl",
    "tact_fail",
    "try_compute",
    "try_ring",
    "try_lia",
    "try_cc",
    "try_simp",
    "try_omega",
    "try_auto",
    "try_decide",
    "try_assumption",
    "try_split",
    "try_left",
    "try_right",
    "try_bitblast",
    "try_tabulate",
    "try_hw_auto",
    "try_inversion",
    "try_induction",
    "try_rewrite",
    "reduceBool",
    "Quot_lift",
    "Quot_rec",
];

/// **THE ARGUMENT THE DOOR RESTS ON, EXECUTED.** The context-aware door admits a `Global` only
/// when it is a constructor or an inductive with no body. Every name-keyed rule head in
/// `reduce_step_opt` must fail that test, or the door would hand back a term with a live redex
/// in it and call it a normal form.
#[test]
fn the_conservative_inertness_criterion_refuses_every_name_keyed_rule_head() {
    let ctx = std_ctx();
    let inert = |n: &str| {
        (ctx.is_constructor(n) || ctx.is_inductive(n)) && ctx.get_definition_body(n).is_none()
    };
    for head in NAME_KEYED_RULE_HEADS {
        assert!(
            !inert(head),
            "`{head}` is a rule head keyed on the NAME, and the door's criterion admitted it"
        );
        // …and the door itself refuses a term that mentions it, at every position.
        for t in [
            g(head),
            app(g(head), var("x")),
            app(var("x"), g(head)),
            lam("u", g(head), var("u")),
        ] {
            assert_eq!(
                beta_zeta_normal_form_in_context(&ctx, &t, 1 << 16),
                None,
                "the door admitted a term mentioning `{head}`: {t}"
            );
        }
    }
    // And the criterion is not vacuously false: the corpus spine's own globals pass it.
    assert!(inert("Nat"), "`Nat` is an inductive with no body");
    assert!(inert("Zero"), "`Zero` is a constructor with no body");
    assert!(inert("Succ"), "`Succ` is a constructor with no body");
    assert!(
        !inert("nothing_declared_at_all"),
        "an unknown name is refused, not assumed inert"
    );
}

/// The context-aware door, end to end: the corpus spine is admitted and answered, a `@[simp]`
/// rule shuts it, and a definition anywhere in the term shuts it.
#[test]
fn the_context_aware_door_opens_on_the_corpus_spine_and_shuts_on_a_simp_rule() {
    let ctx = std_ctx();
    assert_eq!(
        beta_zeta_normal_form_in_context(&ctx, &spine_at(64), 1 << 16),
        Some(g("Zero")),
        "the corpus spine is exactly what the door is for"
    );
    assert_eq!(
        beta_zeta_normal_form_in_context(&ctx, &spine_at(64), 1 << 16),
        Some(normalize(&ctx, &spine_at(64))),
        "and its answer is the kernel's"
    );

    // A definition is δ-reducible, so the door must refuse the term that mentions it.
    let mut with_def = std_ctx();
    with_def.add_definition("two".to_string(), nat(), peano(2));
    assert_eq!(
        beta_zeta_normal_form_in_context(&with_def, &app(g("Succ"), g("two")), 1 << 16),
        None,
        "δ would fire on `two`"
    );
    // …and it refuses the SPINE too under that context only because of the name, not the shape:
    // the same spine still passes, since it mentions no definition.
    assert!(
        beta_zeta_normal_form_in_context(&with_def, &spine_at(8), 1 << 16).is_some(),
        "adding an unrelated definition must not shut the door on a term that never names it"
    );

    // A registered rewrite can fire on any shape at all, so the door shuts outright. `p` is an
    // opaque constant, so `p Zero = Zero` has no proof: the rule is an ASSUMED equation, and a
    // `refl` body offered as its proof is refused by the checked gateway.
    let mut refused = std_ctx();
    refused.add_declaration("p", pi("_", nat(), nat()));
    refused.add_definition(
        "p_rule".to_string(),
        eq_t(nat(), app(g("p"), g("Zero")), g("Zero")),
        refl_t(nat(), g("Zero")),
    );
    assert!(
        refused.get_global("p_rule").is_none()
            && refused
                .refused_declarations()
                .iter()
                .any(|(n, _)| n == "p_rule"),
        "`refl Nat Zero` does not prove `p Zero = Zero`, so the gateway registers nothing"
    );

    let mut simped = std_ctx();
    simped.add_declaration("p", pi("_", nat(), nat()));
    simped.add_declaration("p_rule", eq_t(nat(), app(g("p"), g("Zero")), g("Zero")));
    simped.add_simp_lemma("p_rule").expect("the rule compiles");
    assert!(!simped.simp_rules().is_empty(), "the rule is registered");
    assert_eq!(
        beta_zeta_normal_form_in_context(&simped, &spine_at(8), 1 << 16),
        None,
        "a registered rewrite can fire on any shape, so the door is shut outright"
    );
}

/// **THE CROSSOVER, MEASURED.** The door builds a `TermStore` per call, and that set-up cost is
/// real: below some depth the tree reducer wins and installing the door unconditionally would
/// be a small regression on the millions of tiny normalisations the kernel does. This prints
/// the two columns across the range so the handoff's depth gate is a measured number and not a
/// guess.
#[test]
#[ignore = "measurement: seconds of reduction across a depth sweep; run on purpose"]
fn w_perf_measure_where_the_store_starts_paying() {
    const REPS: usize = 40;
    let ctx = std_ctx();

    let time = |t: &Term, door: bool| -> f64 {
        let mut best = f64::MAX;
        for _ in 0..REPS {
            let start = std::time::Instant::now();
            if door {
                let nf = beta_zeta_normal_form_in_context(&ctx, t, 1 << 24).expect("admitted");
                let took = start.elapsed().as_secs_f64();
                assert_eq!(nf, g("Zero"));
                best = best.min(took);
            } else {
                let nf = deepen_stack(t, || normalize(&ctx, t)).expect("outermost");
                let took = start.elapsed().as_secs_f64();
                assert_eq!(nf, g("Zero"));
                best = best.min(took);
            }
        }
        best
    };
    // Warm both paths before anything is timed.
    let _ = time(&spine_at(4), false);
    let _ = time(&spine_at(4), true);

    println!("\n  depth     tree `normalize`      store door        door/tree");
    println!("  --------------------------------------------------------------");
    let mut crossover: Option<usize> = None;
    for depth in [1usize, 2, 4, 8, 16, 24, 32, 48, 64, 96, 128, 256, 512] {
        let t = spine_at(depth);
        let tree = time(&t, false);
        let door = time(&t, true);
        println!(
            "  {depth:<8}  {tree:>14.8}s  {door:>14.8}s   {:>8.2}×",
            door / tree
        );
        if crossover.is_none() && door < tree {
            crossover = Some(depth);
        }
    }
    println!("  the store door first wins at depth {crossover:?}\n");
}

/// The store's own ζ speculation must not be exponential in a nest of shared bindings, for the
/// same reason `zeta_or_share_opt`'s is not: the probe is suppressed while a probe is running.
/// Sixteen nested held bindings is `2^16` probes unguarded and a walk guarded, so a test that
/// never returns is the unmistakable failure.
#[test]
fn the_stores_zeta_speculation_is_not_exponential_in_a_nest_of_shared_bindings() {
    const NEST: usize = 16;
    let mut body = app(app(g("pair"), var("b15")), var("b15"));
    for i in (0..NEST).rev() {
        let value = if i == 0 {
            app(g("Succ"), var("u"))
        } else {
            app(
                app(g("pair"), var(&format!("b{}", i - 1))),
                var(&format!("b{}", i - 1)),
            )
        };
        body = let_in(&format!("b{i}"), nat(), value, body);
    }
    let mut store = TermStore::new();
    let id = store.intern(&body);
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 20) else {
        panic!("nothing in the nest can step, so a normal form exists");
    };
    let out = store.extract(nf);
    let mut depth = 0usize;
    let mut walk = &out;
    while let Term::Let { body, .. } = walk {
        depth += 1;
        walk = body;
    }
    assert_eq!(
        depth, NEST,
        "every binding in the nest is still a binding: {out}"
    );
    assert!(
        nodes(&out) < 200,
        "the shared form stays small: {} nodes for a nest whose expansion is 2^{NEST}",
        nodes(&out)
    );
}

/// A rebuild can put a redex AT a node: `(let b := λu. u in e) arg` holds its binding while `e`
/// mentions `b` twice, and if normalising `e` drops that to one the binding expands to a λ —
/// leaving a β-redex under an application that the weak-head pass already walked past.
///
/// This is the shape that made `beta_zeta_normalize` hand back a term with a live redex in it
/// and call it a normal form. Both halves: the store's answer has no redex left, and it is the
/// same term the kernel reaches.
#[test]
fn a_binding_that_expands_after_its_body_normalises_leaves_no_redex_behind() {
    let ctx = std_ctx();
    // `b` occurs twice in `(λw:Nat. b) b`, so the binding is held; the body β-reduces to `b`,
    // which is ONE occurrence, so the binding then expands to `λu:Nat. u`.
    let inner = let_in(
        "b",
        nat(),
        lam("u", nat(), var("u")),
        app(lam("w", nat(), var("b")), var("b")),
    );
    let whole = app(inner, g("Zero"));

    let mut store = TermStore::new();
    let id = store.intern(&whole);
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 16) else {
        panic!("a β/ζ normal form exists");
    };
    let out = store.extract(nf);
    assert_eq!(
        out,
        g("Zero"),
        "the whole thing reduces to `Zero`, with nothing left over"
    );
    assert_eq!(
        out,
        normalize(&ctx, &whole),
        "and that is the kernel's answer too"
    );

    // The door, which is the path the handoff installs, agrees.
    assert_eq!(
        beta_zeta_normal_form_in_context(&ctx, &whole, 1 << 16),
        Some(g("Zero"))
    );
}

/// The OTHER defect the differential found: a held binding whose parts are all normal is NOT
/// necessarily a normal form, because expanding it can unblock a redex that could not see past
/// the binder. `let x := λz. y in x x` is stuck while held and reduces to `y` expanded.
///
/// `zeta_or_share_opt` has always had this third case — the speculation — and
/// `beta_zeta_normalize` did not, so the two implementations of one rule disagreed.
#[test]
fn a_held_binding_whose_expansion_can_still_compute_is_released() {
    let ctx = std_ctx();
    let held = let_in(
        "x",
        nat(),
        lam("z", nat(), var("y")),
        app(var("x"), var("x")),
    );
    let mut store = TermStore::new();
    let id = store.intern(&held);
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 16) else {
        panic!("a β/ζ normal form exists");
    };
    assert_eq!(
        store.extract(nf),
        var("y"),
        "the binding is released because its expansion can still compute"
    );
    assert_eq!(
        store.extract(nf),
        normalize(&ctx, &held),
        "and the kernel says the same"
    );

    // The NEGATIVE, one rung the other side: a held binding whose expansion is ALREADY normal
    // stays held, because holding it is what the corpus node bound is about.
    let mut bctx = std_ctx();
    bctx.add_declaration(
        "quad",
        pi(
            "_",
            nat(),
            pi("_", nat(), pi("_", nat(), pi("_", nat(), nat()))),
        ),
    );
    let kept = let_in(
        "b",
        nat(),
        peano(3),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    let mut store2 = TermStore::new();
    let id2 = store2.intern(&kept);
    let StoreNormalForm::Reached(nf2) = store2.beta_zeta_normalize(id2, 1 << 16) else {
        panic!("a β/ζ normal form exists");
    };
    let out2 = store2.extract(nf2);
    assert!(
        matches!(out2, Term::Let { .. }),
        "the binding stands: {out2}"
    );
    assert_eq!(out2, normalize(&bctx, &kept), "and the kernel holds it too");
}
