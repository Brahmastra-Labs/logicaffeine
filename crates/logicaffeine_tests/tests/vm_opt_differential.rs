//! M10 gate: the RUN-PATH OPTIMIZER (the live Futamura residual) is
//! differentially gated — every benchmark program runs through
//! `with_optimized_program` + VM/JIT and must produce the outcome the RAW
//! tree-walker produces. This is the net that makes optimizer bugs
//! impossible to ship silently: PE, GVN, LICM, closed-form, deforestation,
//! interval-based dead-branch elimination, and DCE all sit between the two.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::optimization::REGISTRY;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Deterministic LCG (no `rand` dependency) so the fuzz's random config subsets
/// reproduce exactly from the logged seed.
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

/// The optimized-VM outcome with a given `LOGOS_OPT_OFF` keyword list (empty =
/// all-on). Set/clear is race-free: nextest isolates the test in its own process
/// and the fuzz loop is single-threaded.
fn outcome_with_opts_off(src: &str, argv: &[String], off: &str) -> (String, Option<String>) {
    if off.is_empty() {
        std::env::remove_var("LOGOS_OPT_OFF");
    } else {
        std::env::set_var("LOGOS_OPT_OFF", off);
    }
    let r = optimized_vm_outcome(src, argv);
    std::env::remove_var("LOGOS_OPT_OFF");
    r
}

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Optimized program → tiered VM, with a private tier.
fn optimized_vm_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    with_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => {
            let (output, error) = logicaffeine_compile::vm::run_to_outcome_with_args(
                stmts,
                interner,
                Some(types),
                Some(&policies),
                argv,
                Some(&tier as &dyn NativeTier),
            );
            (output, error)
        }
        Err(advice) => (String::new(), Some(advice)),
    })
}

fn assert_optimized_matches_raw(src: &str, argv: &[String]) {
    let (out, err) = optimized_vm_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "OPTIMIZED VM diverged from the raw tree-walker on:\n{src}"
    );
}

/// Every benchmark program, optimized, must match the raw tree-walker.
#[test]
fn corpus_optimized_vm_matches_raw_treewalker() {
    const CORPUS: &[(&str, &str)] = &[
        ("ackermann", "3"),
        ("array_fill", "2000"),
        ("array_reverse", "2000"),
        ("binary_trees", "6"),
        ("bubble_sort", "60"),
        ("coins", "500"),
        ("collatz", "300"),
        ("collect", "300"),
        ("counting_sort", "2000"),
        ("fannkuch", "5"),
        ("fib", "12"),
        ("fib_iterative", "500"),
        ("gcd", "60"),
        ("graph_bfs", "200"),
        ("heap_sort", "300"),
        ("histogram", "2000"),
        ("knapsack", "30"),
        ("loop_sum", "2000"),
        ("mandelbrot", "20"),
        ("matrix_mult", "8"),
        ("mergesort", "300"),
        ("nbody", "100"),
        ("nqueens", "5"),
        ("pi_leibniz", "2000"),
        ("prefix_sum", "2000"),
        ("primes", "500"),
        ("quicksort", "300"),
        ("sieve", "2000"),
        ("spectral_norm", "20"),
        ("string_search", "200"),
        ("strings", "200"),
        ("two_sum", "300"),
    ];
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            for &(name, size) in CORPUS {
                let path = format!(
                    "{}/../../benchmarks/programs/{}/main.lg",
                    env!("CARGO_MANIFEST_DIR"),
                    name
                );
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
                let argv = vec!["bench".to_string(), size.to_string()];
                let (out, err) = optimized_vm_outcome(&src, &argv);
                let tw = tw_outcome_with_args(&src, &argv);
                assert_eq!(
                    (norm(&out), &err),
                    (norm(&tw.output), &tw.error),
                    "OPTIMIZED VM diverged from raw tree-walker on '{name}' at {size}"
                );
            }
        })
        .expect("spawn")
        .join()
        .expect("corpus thread panicked");
}

/// PHASE D — clean-disable differential fuzz. The user's #1 worry: disabling a
/// toggle while OTHER optimizations stay on must never break or change codegen.
///
/// Property: for ANY subset of optimizations disabled (driven by `LOGOS_OPT_OFF`
/// / `LOGOS_OPT_PROFILE` / `LOGOS_OPT=off`), the OPTIMIZED VM+JIT must produce the
/// SAME (normalized stdout, error) as the config-independent raw tree-walker —
/// optimizations change speed, never observable output. Config classes: all-on,
/// all-off, leave-one-out (each opt removed alone — the user's exact scenario),
/// the named profiles, and seeded-random subsets. Programs run at tiny sizes
/// (output is deterministic regardless of size) so the ~1000 evaluations stay
/// fast enough for the every-CI fast tier.
#[test]
fn clean_disable_paths_preserve_output() {
    const FUZZ_CORPUS: &[(&str, &str)] = &[
        ("fib", "8"), ("ackermann", "2"), ("binary_trees", "4"), ("bubble_sort", "20"),
        ("mergesort", "30"), ("quicksort", "30"), ("nbody", "3"), ("mandelbrot", "5"),
        ("knapsack", "10"), ("two_sum", "30"), ("graph_bfs", "20"), ("histogram", "50"),
        ("nqueens", "5"), ("collatz", "30"), ("primes", "50"), ("gcd", "20"),
        ("matrix_mult", "4"), ("string_search", "20"), ("collect", "30"), ("pi_leibniz", "200"),
    ];
    const SEED: u64 = 0x5EED_C0FFEE;
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let mut rng = SEED;
            for &(name, size) in FUZZ_CORPUS {
                let path = format!(
                    "{}/../../benchmarks/programs/{}/main.lg",
                    env!("CARGO_MANIFEST_DIR"),
                    name
                );
                let src = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
                let argv = vec!["bench".to_string(), size.to_string()];

                // Config-independent oracle (the tree-walker ignores the optimizer).
                let tw = tw_outcome_with_args(&src, &argv);
                let oracle = (norm(&tw.output), tw.error.clone());

                let check = |off: &str, label: &str| {
                    let (out, err) = outcome_with_opts_off(&src, &argv, off);
                    assert_eq!(
                        &(norm(&out), err),
                        &oracle,
                        "CLEAN-DISABLE VIOLATION on '{name}' (size {size}), config [{label}] \
                         (LOGOS_OPT_OFF=\"{off}\", seed={SEED:#x}): the optimized VM diverged \
                         from the tree-walker. Disabling optimizations must never change output."
                    );
                };

                // (a) control: all-on, and master all-off.
                check("", "all-on");
                {
                    std::env::set_var("LOGOS_OPT", "off");
                    let (out, err) = optimized_vm_outcome(&src, &argv);
                    std::env::remove_var("LOGOS_OPT");
                    assert_eq!(&(norm(&out), err), &oracle, "all-off (LOGOS_OPT=off) changed output for '{name}'");
                }
                // (b) leave-one-out: disable exactly ONE optimization, the rest ON.
                for m in REGISTRY {
                    check(m.keyword, m.keyword);
                }
                // (c) named profiles (Memory / Safety).
                for prof in ["memory", "safety"] {
                    std::env::set_var("LOGOS_OPT_PROFILE", prof);
                    let (out, err) = optimized_vm_outcome(&src, &argv);
                    std::env::remove_var("LOGOS_OPT_PROFILE");
                    assert_eq!(&(norm(&out), err), &oracle, "profile '{prof}' changed output for '{name}'");
                }
                // (d) seeded-random subsets: disable a random ~half, the rest ON.
                for _ in 0..6 {
                    let off = REGISTRY
                        .iter()
                        .filter(|_| lcg(&mut rng) & 1 == 0)
                        .map(|m| m.keyword)
                        .collect::<Vec<_>>()
                        .join(",");
                    check(&off, "random");
                }
            }
        })
        .expect("spawn")
        .join()
        .expect("fuzz thread panicked");
}

/// Error-outcome parity through the optimizer: partial output + exact error.
#[test]
fn optimizer_preserves_error_outcomes() {
    assert_optimized_matches_raw(
        "## Main\nShow 1.\nLet mutable i be 1.\nWhile i is at most 500:\n\
         \x20   Let d be 150 - i.\n\
         \x20   Let q be 1000 / d.\n\
         \x20   Set i to i + 1 + q / 10000.\n\
         Show i.\n",
        &[],
    );
    assert_optimized_matches_raw("## Main\nShow 2.\nShow item 9 of [1, 2].\n", &[]);
}

/// Tiny pure functions INLINE on the run path: the spectral_norm shape
/// (a one-expression helper called per inner-loop element) must lose its
/// call — the residual keeps no reference to the helper at the call site,
/// so the loop region compiles to straight arithmetic instead of paying
/// the native call boundary half a million times.
#[test]
fn optimizer_inlines_tiny_pure_helpers() {
    use logicaffeine_compile::ast::stmt::{Expr, Stmt};
    let src = "## To aVal (i: Int, j: Int) -> Float:\n\
               \x20   Return 1.0 / ((i + j) * (i + j + 1) / 2 + i + 1).\n\
               \n\
               ## Main\n\
               Let mutable acc be 0.0.\n\
               Let mutable i be 0.\n\
               While i is less than 50:\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than 50:\n\
               \x20       Set acc to acc + aVal(i, j).\n\
               \x20       Set j to j + 1.\n\
               \x20   Set i to i + 1.\n\
               Show \"{acc:.9}\".\n";
    // Exactness first.
    assert_optimized_matches_raw(src, &[]);
    // Structure: no aVal CALL survives in Main's loops.
    fn expr_calls(e: &Expr, name: &str, interner: &logicaffeine_compile::intern::Interner) -> bool {
        match e {
            Expr::Call { function, args } => {
                interner.resolve(*function) == name
                    || args.iter().any(|a| expr_calls(a, name, interner))
            }
            Expr::BinaryOp { left, right, .. } => {
                expr_calls(left, name, interner) || expr_calls(right, name, interner)
            }
            Expr::Not { operand } => expr_calls(operand, name, interner),
            Expr::Index { collection, index } => {
                expr_calls(collection, name, interner) || expr_calls(index, name, interner)
            }
            _ => false,
        }
    }
    fn stmt_calls(s: &Stmt, name: &str, interner: &logicaffeine_compile::intern::Interner) -> bool {
        match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_calls(value, name, interner),
            Stmt::Show { object, .. } => expr_calls(object, name, interner),
            Stmt::While { cond, body, .. } => {
                expr_calls(cond, name, interner)
                    || body.iter().any(|b| stmt_calls(b, name, interner))
            }
            Stmt::If { cond, then_block, else_block } => {
                expr_calls(cond, name, interner)
                    || then_block.iter().any(|b| stmt_calls(b, name, interner))
                    || else_block
                        .map(|eb| eb.iter().any(|b| stmt_calls(b, name, interner)))
                        .unwrap_or(false)
            }
            _ => false,
        }
    }
    let survived = logicaffeine_compile::ui_bridge::with_optimized_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
            .any(|s| stmt_calls(s, "aVal", interner))
    });
    assert!(!survived, "aVal must inline at its call site on the run path");
}

/// Inlining must REFUSE what it cannot prove: recursion, effectful bodies
/// (Show), and list-mutating helpers keep their calls and stay exact.
#[test]
fn optimizer_inlining_fails_closed() {
    // Recursive — must not inline (and must not loop the optimizer).
    assert_optimized_matches_raw(
        "## To fib (n: Int) -> Int:\n\
         \x20   If n is less than 2:\n\
         \x20       Return n.\n\
         \x20   Return fib(n - 1) + fib(n - 2).\n\
         \n\
         ## Main\n\
         Show fib(12).\n",
        &[],
    );
    // Effectful — Show inside the body observes call ORDER and COUNT.
    assert_optimized_matches_raw(
        "## To noisy (n: Int) -> Int:\n\
         \x20   Show n.\n\
         \x20   Return n * 2.\n\
         \n\
         ## Main\n\
         Let mutable i be 0.\n\
         While i is less than 3:\n\
         \x20   Let r be noisy(i).\n\
         \x20   Set i to i + 1.\n\
         Show 99.\n",
        &[],
    );
    // Multi-statement with branches and a mutable local — exactness is the
    // contract whether or not it inlines.
    assert_optimized_matches_raw(
        "## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n\
         \x20   Let mutable r be x.\n\
         \x20   If r is less than lo:\n\
         \x20       Set r to lo.\n\
         \x20   If r is greater than hi:\n\
         \x20       Set r to hi.\n\
         \x20   Return r.\n\
         \n\
         ## Main\n\
         Let mutable s be 0.\n\
         Let mutable i be 0.\n\
         While i is less than 100:\n\
         \x20   Set s to s + clamp(i * 3 % 50, 10, 40).\n\
         \x20   Set i to i + 1.\n\
         Show s.\n",
        &[],
    );
}

/// Kernel modulo takes the SIGN OF THE DIVIDEND (-7 % 2 == -1, like Rust
/// i64 and bvsrem). The unguarded power-of-two mask rewrite broke this —
/// caught by phase_tv_encoder_sound the moment the optimizer went live.
#[test]
fn optimizer_preserves_negative_modulo_sign() {
    assert_optimized_matches_raw("## Main\nLet a be 0 - 7.\nShow a % 2.\n", &[]);
    assert_optimized_matches_raw("## Main\nLet a be 0 - 9.\nShow a % 4.\n", &[]);
    // and across a dynamic boundary (PE cannot fold argv):
    assert_optimized_matches_raw(
        "## To native args () -> Seq of Text\n\
         ## To native parseInt (s: Text) -> Int\n\
         \n\
         ## Main\n\
         Let n be parseInt(item 2 of args()).\n\
         Show (0 - n) % 8.\n",
        &["bench".to_string(), "21".to_string()],
    );
}

/// Algebraic deletions must not erase runtime errors: `x * 0`, `x && false`
/// and `x || true` all DELETE x from the residual — if x can error, the
/// error is the program's meaning.
#[test]
fn optimizer_never_erases_errors_via_algebraic_deletion() {
    assert_optimized_matches_raw("## Main\nShow 1.\nShow 10 / 0 * 0.\n", &[]);
    assert_optimized_matches_raw("## Main\nShow 2.\nShow 0 * (10 / 0).\n", &[]);
    assert_optimized_matches_raw(
        "## Main\nShow 3.\nLet xs be [1, 2].\nShow 10 / 0 > 0 and false.\n",
        &[],
    );
    assert_optimized_matches_raw("## Main\nShow 4.\nShow 10 / 0 > 0 or true.\n", &[]);
}

/// The optimizer must not break args-driven programs (PE sees `args()` as
/// dynamic — no constant-folding of runtime input).
#[test]
fn optimizer_keeps_argv_dynamic() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let n be parseInt(item 2 of arguments).\n\
               Show n * 2.\n";
    let argv = vec!["bench".to_string(), "21".to_string()];
    let (out, err) = optimized_vm_outcome(src, &argv);
    assert_eq!(err, None);
    assert_eq!(norm(&out), "42");
}

/// The kill switch: `LOGOS_OPT=off` must bypass the optimizer entirely (the
/// program still runs, identically).
#[test]
fn run_opt_kill_switch() {
    std::env::set_var("LOGOS_OPT", "off");
    let (out, err) = optimized_vm_outcome("## Main\nShow 6 * 7.\n", &[]);
    std::env::remove_var("LOGOS_OPT");
    assert_eq!(err, None);
    assert_eq!(norm(&out), "42");
}

/// The nbody force-loop shape, end-to-end through `optimize_for_run`: small
/// fixed-size `Seq of Float` arrays, a triangular nested loop, and IN-PLACE
/// read-modify-write velocity updates (`Set item i of bv to item i of bv - …`).
/// This pins TWO load-bearing properties of the run-path optimizer that the
/// nbody benchmark depends on (measured: the W16 indexed-load hoist is a 1.85×
/// run-path win on real nbody, and it must never silently change a float):
///
///   1. BIT-EXACT DIFFERENTIAL — the optimized VM (with the JIT region tier
///      that runs the force loop on the contiguous register-allocating backend)
///      produces output IDENTICAL to the independent tree-walker. Scalar
///      promotion / load motion across a float-RMW loop is sound ONLY when it
///      is value-preserving; a float reassociation or a misplaced write-back
///      would diverge here.
///
///   2. THE HOIST FIRES — the invariant `item i of <arr>` read (loop-invariant
///      w.r.t. the inner `j` loop, on an array the inner loop never mutates) is
///      lifted OUT of the inner loop by the run-path LICM, so the hot O(n²) body
///      reloads only its `j`-varying operands. The structural check proves the
///      lifted read no longer appears inside the inner `While j` body.
#[test]
fn nbody_force_loop_scalarizes_and_stays_bit_exact() {
    // A self-contained nbody kernel: 3 bodies, position array `bx`, mass array
    // `bm`, velocity array `bv`. The inner loop reads `item i of bx`/`item i of
    // bm` (invariant in j) and `item j of bx`/`item j of bm` (variant), and
    // updates `bv` in place at BOTH `i` and `j` — the exact force-loop shape.
    let src = "## Main\n\
               Let mutable bx be a new Seq of Float.\n\
               Let mutable bm be a new Seq of Float.\n\
               Let mutable bv be a new Seq of Float.\n\
               Push 1.0 to bx. Push 2.0 to bx. Push 4.0 to bx.\n\
               Push 0.5 to bm. Push 1.5 to bm. Push 2.5 to bm.\n\
               Push 0.0 to bv. Push 0.0 to bv. Push 0.0 to bv.\n\
               Let mutable i be 1.\n\
               While i is at most 3:\n\
               \x20   Let mutable j be i + 1.\n\
               \x20   While j is at most 3:\n\
               \x20       Let dx be item i of bx - item j of bx.\n\
               \x20       Let inv be 1.0 / (dx * dx + 0.001).\n\
               \x20       Set item i of bv to item i of bv - dx * item j of bm * inv.\n\
               \x20       Set item j of bv to item j of bv + dx * item i of bm * inv.\n\
               \x20       Set j to j + 1.\n\
               \x20   Set i to i + 1.\n\
               Show \"{item 1 of bv:.9}\".\n\
               Show \"{item 2 of bv:.9}\".\n\
               Show \"{item 3 of bv:.9}\".\n";

    // Property 1: bit-exact through the optimizer + tiered VM.
    assert_optimized_matches_raw(src, &[]);

    // Property 2: run-path SCALARIZATION (the interpreter's SROA, default-ON)
    // supersedes the old LICM invariant-hoist for this fixed-size-array force loop.
    // It unrolls the constant inner `While j` loop and replaces bx/bm/bv with
    // scalar locals, so NO inner force loop and NO `item _ of bx` array read survive
    // in the optimized residual — a strictly stronger result than hoisting one
    // invariant load out (it eliminates every bounds-checked array read). Property 1
    // above already guarantees the rewrite stays bit-exact. (LICM still hoists
    // invariants out of loops over RUNTIME-sized arrays, which scalarization leaves
    // untouched.)
    use logicaffeine_compile::ast::stmt::{Expr, Stmt};
    use logicaffeine_compile::intern::{Interner, Symbol};

    /// Does `e` read `item <Identifier(idx)> of <Identifier(coll)>` anywhere?
    fn reads_item_of(e: &Expr, coll: Symbol, idx: Symbol) -> bool {
        match e {
            Expr::Index { collection, index } => {
                let here = matches!(&**collection, Expr::Identifier(c) if *c == coll)
                    && matches!(&**index, Expr::Identifier(x) if *x == idx);
                here || reads_item_of(collection, coll, idx) || reads_item_of(index, coll, idx)
            }
            Expr::BinaryOp { left, right, .. } => {
                reads_item_of(left, coll, idx) || reads_item_of(right, coll, idx)
            }
            Expr::Not { operand } => reads_item_of(operand, coll, idx),
            Expr::Length { collection } => reads_item_of(collection, coll, idx),
            Expr::Call { args, .. } => args.iter().any(|a| reads_item_of(a, coll, idx)),
            _ => false,
        }
    }

    fn block_reads(stmts: &[Stmt], coll: Symbol, idx: Symbol) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => reads_item_of(value, coll, idx),
            Stmt::SetIndex { collection, index, value } => {
                reads_item_of(collection, coll, idx)
                    || reads_item_of(index, coll, idx)
                    || reads_item_of(value, coll, idx)
            }
            Stmt::If { cond, then_block, else_block } => {
                reads_item_of(cond, coll, idx)
                    || block_reads(then_block, coll, idx)
                    || else_block.map_or(false, |eb| block_reads(eb, coll, idx))
            }
            Stmt::While { cond, body, .. } => {
                reads_item_of(cond, coll, idx) || block_reads(body, coll, idx)
            }
            _ => false,
        })
    }

    /// Walk to the INNERMOST `While j …` loop (the force loop) and report
    /// whether its body reads `item <idx> of <coll>`. Returns None if no
    /// matching inner loop is found.
    fn inner_loop_reads(
        stmts: &[Stmt],
        coll: Symbol,
        i_sym: Symbol,
        j_sym: Symbol,
    ) -> Option<(bool, bool)> {
        for s in stmts {
            match s {
                // The inner loop is a `While` whose condition tests `j`.
                Stmt::While { cond, body, .. }
                    if matches!(cond, Expr::BinaryOp { left, .. }
                        if matches!(&**left, Expr::Identifier(x) if *x == j_sym)) =>
                {
                    return Some((block_reads(body, coll, i_sym), block_reads(body, coll, j_sym)));
                }
                Stmt::While { body, .. } => {
                    if let Some(r) = inner_loop_reads(body, coll, i_sym, j_sym) {
                        return Some(r);
                    }
                }
                Stmt::If { then_block, else_block, .. } => {
                    if let Some(r) = inner_loop_reads(then_block, coll, i_sym, j_sym) {
                        return Some(r);
                    }
                    if let Some(eb) = else_block {
                        if let Some(r) = inner_loop_reads(eb, coll, i_sym, j_sym) {
                            return Some(r);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    let inner = with_optimized_program(src, |parsed, interner: &Interner| {
        let (stmts, _t, _p) = parsed.expect("parse");
        let i_sym = interner.lookup("i").expect("loop var `i`");
        let j_sym = interner.lookup("j").expect("loop var `j`");
        // `bx` is scalarized away; its symbol stays interned but no statement uses
        // it, so the inner force loop reading it no longer exists. If for some
        // reason the array survives, fall through to the inner-loop scan so a
        // regression (loop NOT unrolled) is still caught.
        match interner.lookup("bx") {
            Some(coll) => inner_loop_reads(stmts, coll, i_sym, j_sym),
            None => None,
        }
    });

    assert!(
        inner.is_none(),
        "scalarization must unroll the constant inner force loop and replace the \
         fixed-size arrays with scalars — no inner `While j` loop reading `item _ of bx` \
         may survive in the optimized residual"
    );
}
