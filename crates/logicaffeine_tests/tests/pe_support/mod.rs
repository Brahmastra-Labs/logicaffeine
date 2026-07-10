//! Shared test harness for the partial-evaluator phases (work/PE_IMPROVE.md §4.5).
//!
//! A partial evaluator is the kind of program where a plausible-looking output is the
//! most dangerous failure mode, so the harness defends the three properties that matter:
//! **correctness** (the residual preserves observable behavior), **optimality** (static
//! work is actually gone — the Jones-optimality oracle [`count_dispatch`]), and
//! **totality** (the PE halts, enforced by [`with_budget`]).
//!
//! ## Triangulation (§4.2)
//!
//! ```text
//!   tree-walking interpreter (interpreter.rs)  ← independent ground truth (run_treewalk)
//!                  ║
//!   residual:  run( PE(p) )                     ← what the PE produced (run_p1)
//! ```
//!
//! Phase A exercises the **tree-walker vs PE-residual** legs — both fully library-backed
//! and sufficient for totality + preservation of the PE on real programs. The third leg
//! (running the LOGOS self-interpreter directly) requires promoting the self-interpreter
//! out of the inline `phase_futamura.rs` strings into a first-class `.logos` artifact;
//! that is gap **G8**, sequenced to Phase D. The seam is [`run_self_interp`].
//!
//! Every in-process evaluation runs inside a 256 MB-stack thread with a wall-clock
//! deadline: the genuine LOGOS PE recurses deeply (the cargo paths set
//! `RUST_MIN_STACK=268MB`; the in-process tree-walker does not), and a pathological PE
//! must fail the test at the deadline rather than hang the suite.
#![allow(dead_code)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use logicaffeine_compile::compile;

/// Default wall-clock budget for a single PE evaluation. Totality failures surface as
/// [`Outcome::Timeout`] at this deadline. Sized for a *loaded* CI runner, not the dev box:
/// at 60s the differential corpus/generative shards intermittently overshot under parallel
/// load, timed out to empty output, and diverged (flaky retries). 180s gives the ~3× headroom
/// a constrained runner needs while still catching a genuine non-terminating PE (an infinite
/// loop exceeds any finite deadline, and the suite's 30-min per-test terminate is the backstop).
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(180);

/// Stack size for in-process evaluation threads. Matches the `RUST_MIN_STACK` the cargo
/// execution paths use (`compile.rs:4232`); the genuine LOGOS PE needs it.
const STACK_SIZE: usize = 256 * 1024 * 1024;

/// The normalized result of running a program through one evaluator.
///
/// `Nothing` vs `Error` is a real distinction across evaluators: the self-interpreter's
/// `applyBinOp` returns `VNothing` on div-by-zero / type-mismatch, while the production
/// tree-walker raises. [`assert_same_behavior`] with [`CmpMode::Lenient`] is where that
/// divergence is absorbed (the self-interp ↔ tree-walk boundary, a Phase D concern); it
/// must never leak into the PE-residual comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Normal completion with an observable result (the trimmed output stream).
    Value(String),
    /// Completed but produced no observable value (empty output / `VNothing`).
    Nothing,
    /// Raised an error or failed to parse/encode. The message is diagnostic only.
    Error(String),
    /// Exceeded the time budget — the totality-failure signal.
    Timeout,
}

/// A full observation of one evaluation: the primary outcome, the accumulated output
/// stream, and any raw error text (kept for debugging, never compared by default).
#[derive(Debug, Clone)]
pub struct Observation {
    pub value: Outcome,
    pub output: String,
    pub error: Option<String>,
}

impl Observation {
    fn from_interp(result: Result<String, impl std::fmt::Debug>) -> Self {
        match result {
            Ok(out) => {
                let trimmed = out.trim().to_string();
                let value = if trimmed.is_empty() {
                    Outcome::Nothing
                } else {
                    Outcome::Value(trimmed.clone())
                };
                Observation { value, output: trimmed, error: None }
            }
            Err(e) => {
                let msg = format!("{:?}", e);
                Observation { value: Outcome::Error(msg.clone()), output: String::new(), error: Some(msg) }
            }
        }
    }

    fn timed_out() -> Self {
        Observation { value: Outcome::Timeout, output: String::new(), error: Some("budget exceeded".to_string()) }
    }

    fn errored(msg: String) -> Self {
        Observation { value: Outcome::Error(msg.clone()), output: String::new(), error: Some(msg) }
    }

    pub fn is_value(&self) -> bool {
        matches!(self.value, Outcome::Value(_) | Outcome::Nothing)
    }
}

/// How strictly to compare two observations' values. `Strict` for two evaluators of the
/// same engine family (PE-residual vs tree-walker); `Lenient` collapses `Nothing` and
/// `Error` for the self-interp ↔ tree-walk boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpMode {
    Strict,
    Lenient,
}

/// Why a budgeted evaluation did not produce a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    Timeout,
    Panicked,
}

/// Run `f` on a 256 MB-stack thread with a wall-clock deadline.
///
/// On timeout the worker thread is abandoned (Rust cannot kill threads) — it dies with
/// the process — and the caller gets [`Budget::Timeout`] so the test fails fast instead
/// of hanging. This is the test-side totality backstop layered over the PE's own internal
/// depth/whistle budget.
pub fn with_budget_for<T, F>(timeout: Duration, f: F) -> Result<T, Budget>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let result = f();
            let _ = tx.send(result);
        })
        .expect("failed to spawn evaluation thread");

    match rx.recv_timeout(timeout) {
        Ok(value) => {
            let _ = handle.join();
            Ok(value)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Budget::Timeout),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Budget::Panicked),
    }
}

/// [`with_budget_for`] with the [`DEFAULT_BUDGET`].
pub fn with_budget<T, F>(f: F) -> Result<T, Budget>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    with_budget_for(DEFAULT_BUDGET, f)
}

/// Run a program through the production tree-walking interpreter — the independent
/// semantic ground truth.
pub fn run_treewalk(program: &str) -> Observation {
    let program = program.to_string();
    match with_budget(move || Observation::from_interp(compile::interpret_program(&program))) {
        Ok(obs) => obs,
        Err(_) => Observation::timed_out(),
    }
}

/// Produce the genuine LOGOS PE residual of `program`, in-process.
///
/// This exercises `pe_source.logos` directly (via `projection1_source_real_fast`), which
/// is exactly the artifact the PE phases modify.
pub fn decompile(program: &str) -> Result<String, String> {
    compile::projection1_source_real_fast("", "", program)
}

/// The same genuine PE residual, but produced by running the PE engine on the register
/// bytecode VM ([`projection1_source_real_fast_on_vm`]) instead of the tree-walker. The
/// PE is a fixed program; the tier must not change its output. Locked byte-for-byte
/// against [`decompile`] by `futamura_tier_lock::pe_engine_residual_identical_on_vm`.
pub fn decompile_on_vm(program: &str) -> Result<String, String> {
    compile::projection1_source_real_fast_on_vm("", "", program)
}

/// Run the PE residual of `program` through the tree-walker.
pub fn run_p1(program: &str) -> Observation {
    let program = program.to_string();
    let result = with_budget(move || match compile::projection1_source_real_fast("", "", &program) {
        Ok(residual) => Observation::from_interp(compile::interpret_program(&residual)),
        Err(e) => Observation::errored(e),
    });
    match result {
        Ok(obs) => obs,
        Err(_) => Observation::timed_out(),
    }
}

/// Compile `program` through a reduced PE dialect (`bti=false` → pe_mini / the P2 subject;
/// `bti=true` → pe_bti / the P3 subject) IN-PROCESS, then run its residual — the behavioral
/// P2/P3 fidelity signal. Returns the trimmed output, or an error string. Runs on the budgeted
/// 256 MB-stack thread (the reduced PE still recurses deeply).
pub fn run_via_dialect(program: &str, bti: bool) -> Result<String, String> {
    let program = program.to_string();
    let out = with_budget(move || {
        let (pe, block_fn, core) = if bti {
            let core = compile::core_types_for_pe_source()
                .replace("specResults", "memoCache")
                .replace("onStack", "callGuard");
            (compile::pe_bti_source_text().to_string(), "peBlockB", core)
        } else {
            (compile::pe_mini_source_text().to_string(), "peBlockM", compile::core_types_for_pe_source().to_string())
        };
        let decompile = compile::decompile_source_text();
        let encoded = compile::encode_program_source(&program).map_err(|e| format!("encode: {e:?}"))?;
        let src = format!(
            "{core}\n{pe}\n{decompile}\n## Main\n{encoded}\n\
             Let fzState be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).\n\
             Let fzCompiled be {block_fn}(encodedMain, fzState).\n\
             Let fzOut be decompileBlock(fzCompiled, 0).\n\
             Show fzOut."
        );
        let residual = compile::interpret_program(&src).map_err(|e| format!("residual: {e:?}"))?;
        let wrapped = format!("## Main\n{}", residual.trim());
        compile::interpret_program(&wrapped)
            .map(|o| o.trim().to_string())
            .map_err(|e| format!("run: {e:?}"))
    });
    match out {
        Ok(r) => r,
        Err(_) => Err("budget exceeded".into()),
    }
}

/// The third triangulation leg — running the LOGOS self-interpreter directly on the
/// program. Lands when the self-interpreter becomes a first-class `.logos` artifact
/// (gap G8, Phase D); until then Phase A triangulates tree-walker vs PE-residual.
pub fn run_self_interp(_program: &str) -> Option<Observation> {
    None
}

/// All available evaluators for one program. Phase A: `[tree-walker, PE-residual]`.
pub fn run_all(program: &str) -> Vec<Observation> {
    let mut obs = vec![run_treewalk(program), run_p1(program)];
    if let Some(si) = run_self_interp(program) {
        obs.push(si);
    }
    obs
}

/// The Jones-optimality oracle: count units of surviving interpreter dispatch in a
/// residual (zero ⇔ the interpreter dissolved). See [`compile::count_dispatch`].
pub fn count_dispatch(residual: &str) -> usize {
    compile::count_dispatch(residual)
}

/// The behavioral comparison underlying [`assert_same_behavior`], returning the
/// human-readable difference instead of panicking. `None` ⇔ the two observations
/// describe the same observable behavior under `mode`.
///
/// The output stream must always match exactly (the strongest, engine-independent
/// signal). Values match per `mode`: `Strict` requires Value==Value / Nothing==Nothing /
/// Error==Error; `Lenient` additionally treats `Nothing` ≡ `Error` (the VNothing-vs-Err
/// boundary). A `Timeout` never matches anything. Sharing this with the asserting
/// wrapper lets callers collect *name-tagged* divergences across a corpus without
/// re-deriving (and risking drift from) the exact comparison semantics.
pub fn behavior_diff(a: &Observation, b: &Observation, mode: CmpMode) -> Option<String> {
    if a.output != b.output {
        return Some(format!("output streams differ:\n  a = {:?}\n  b = {:?}", a, b));
    }
    let same_value = match (&a.value, &b.value) {
        (Outcome::Value(x), Outcome::Value(y)) => x == y,
        (Outcome::Nothing, Outcome::Nothing) => true,
        (Outcome::Error(_), Outcome::Error(_)) => true,
        (Outcome::Timeout, _) | (_, Outcome::Timeout) => false,
        (Outcome::Nothing, Outcome::Error(_)) | (Outcome::Error(_), Outcome::Nothing) => {
            mode == CmpMode::Lenient
        }
        _ => false,
    };
    if !same_value {
        return Some(format!(
            "values differ under {:?}:\n  a = {:?}\n  b = {:?}",
            mode, a.value, b.value
        ));
    }
    None
}

/// Assert two observations describe the same observable behavior. Panics with the
/// exact divergence (see [`behavior_diff`] for the comparison semantics).
pub fn assert_same_behavior(a: &Observation, b: &Observation, mode: CmpMode) {
    if let Some(diff) = behavior_diff(a, b, mode) {
        panic!("{diff}");
    }
}

/// Convenience correctness gate: the tree-walker and the PE residual agree, and the
/// program's output is exactly `expected`.
pub fn assert_run_equals(program: &str, expected: &str) {
    let tw = run_treewalk(program);
    assert_eq!(
        tw.output.trim(),
        expected,
        "tree-walk output mismatch (error = {:?})",
        tw.error
    );
    let p1 = run_p1(program);
    assert_same_behavior(&tw, &p1, CmpMode::Strict);
}

/// Assert the PE halts on `program` within `timeout` and the residual is correct.
/// Returns the residual for further structural assertions.
pub fn assert_halts_and_correct(program: &str, expected: &str) -> String {
    let prog = program.to_string();
    let residual = match with_budget_for(DEFAULT_BUDGET, move || compile::projection1_source_real_fast("", "", &prog)) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => panic!("PE failed (did not halt cleanly): {}", e),
        Err(b) => panic!("PE did not halt within budget: {:?}", b),
    };
    assert_run_equals(program, expected);
    residual
}

// ---------------------------------------------------------------------------
// Corpus / generation. Phase A uses fixed shapes and a small curated list; the
// rich generative grammar (§4.3) is built in the B-phases.
// ---------------------------------------------------------------------------

/// Deterministic program shapes for the generator stub.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// `f(1) f(2) … f(n)` — distinct all-static calls (memo-pressure / key collisions).
    ManyDistinctStaticCalls(u32),
    /// A randomly-generated, well-typed, total Int program from `seed` — the generative
    /// differential corpus (Phase D §4). Pure `+ - *` over literals and prior bindings, so it
    /// always evaluates to a value (no div-by-zero, no type errors) and the PE residual must
    /// agree with the tree-walker. Deterministic: same seed ⇒ same program.
    RandomArith(u64),
}

/// A tiny deterministic PRNG (SplitMix64-style) — no `Math.random`, fully reproducible.
fn next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Generate a well-typed total Int expression over `vars`, bounded by `depth`.
fn gen_int_expr(rng: &mut u64, vars: &[String], depth: u32) -> String {
    // Leaf at depth 0 or ~1/3 of the time.
    if depth == 0 || next_rand(rng) % 3 == 0 {
        if !vars.is_empty() && next_rand(rng) % 2 == 0 {
            let idx = (next_rand(rng) as usize) % vars.len();
            return vars[idx].clone();
        }
        return format!("{}", next_rand(rng) % 10);
    }
    let op = match next_rand(rng) % 3 {
        0 => "+",
        1 => "-",
        _ => "*",
    };
    let l = gen_int_expr(rng, vars, depth - 1);
    let r = gen_int_expr(rng, vars, depth - 1);
    format!("({} {} {})", l, op, r)
}

/// Deterministic program generator (no `Math.random` — seeded). Fixed shapes for memo
/// pressure; `RandomArith` for the generative differential corpus.
pub fn gen_program(_seed: u64, shape: Shape) -> String {
    match shape {
        Shape::ManyDistinctStaticCalls(n) => {
            let mut body = String::from("## To id (x: Int) -> Int:\n    Return x.\n\n## Main\n");
            let mut sum_terms = Vec::new();
            for i in 1..=n {
                sum_terms.push(format!("id({})", i));
            }
            // Sum all calls so every distinct specialization is observable in the output.
            body.push_str(&format!("Let total be {}.\n", sum_terms.join(" + ")));
            body.push_str("Show total.\n");
            body
        }
        Shape::RandomArith(seed) => {
            // Bounded so products stay well inside i64 (depth 2, literals < 10, ≤5 bindings).
            let mut rng = seed ^ 0xD1B54A32D192ED03;
            let mut vars: Vec<String> = Vec::new();
            let mut body = String::from("## Main\n");
            let nstmts = 2 + (next_rand(&mut rng) % 4) as usize; // 2..=5 bindings
            for i in 0..nstmts {
                let e = gen_int_expr(&mut rng, &vars, 2);
                let v = format!("v{}", i);
                body.push_str(&format!("Let {} be {}.\n", v, e));
                vars.push(v);
            }
            let final_e = gen_int_expr(&mut rng, &vars, 2);
            body.push_str(&format!("Show {}.\n", final_e));
            body
        }
    }
}

/// Generate a diverse, well-typed, TOTAL Int program from `seed` — exercising let-bindings,
/// mutable `Set`, static-bound `Repeat` loops, and `If`/comparison control flow, all over
/// `+ - *` (no division, small literals ⇒ no i64 overflow). Every generated program terminates
/// with a defined Int output, so the PE residual MUST be dispatch-free (Jones-optimal) AND agree
/// with the tree-walker. Deterministic: same seed ⇒ same program. This is the full-language
/// generative surface — the answer to "you only proved Jones optimality on a curated corpus."
pub fn gen_diverse_program(seed: u64) -> String {
    let mut rng = seed ^ 0x2545F4914F6CDD1D;
    let mut vars: Vec<String> = Vec::new();
    let mut body = String::from("## Main\n");
    // Seed two mutable Int bindings so Set / If / loop targets always exist and are in scope.
    for i in 0..2u32 {
        let e = gen_int_expr(&mut rng, &vars, 2);
        body.push_str(&format!("Let mutable v{i} be {e}.\n"));
        vars.push(format!("v{i}"));
    }
    let cmps = ["is less than", "is greater than", "is at most", "is at least"];
    let nstmts = 4 + (next_rand(&mut rng) % 6) as usize; // 4..=9 more statements
    for _ in 0..nstmts {
        let target = vars[(next_rand(&mut rng) as usize) % vars.len()].clone();
        match next_rand(&mut rng) % 6 {
            0 => {
                let e = gen_int_expr(&mut rng, &vars, 2);
                body.push_str(&format!("Set {target} to {e}.\n"));
            }
            1 => {
                // If/Otherwise — each branch may set a DIFFERENT target, and read cross-vars, so
                // the else branch must see PRE-`If` values (exercises branch isolation).
                let cond_var = vars[(next_rand(&mut rng) as usize) % vars.len()].clone();
                let lit = next_rand(&mut rng) % 10;
                let cmp = cmps[(next_rand(&mut rng) as usize) % cmps.len()];
                let t2 = vars[(next_rand(&mut rng) as usize) % vars.len()].clone();
                let te = gen_int_expr(&mut rng, &vars, 1);
                let ee = gen_int_expr(&mut rng, &vars, 1);
                body.push_str(&format!(
                    "If {cond_var} {cmp} {lit}:\n    Set {target} to {te}.\nOtherwise:\n    Set {t2} to {ee}.\n"
                ));
            }
            2 => {
                let k = 1 + next_rand(&mut rng) % 4; // small loop — pe_source unrolls (< cap 64)
                body.push_str(&format!("Repeat for gi from 1 to {k}:\n    Set {target} to {target} + gi.\n"));
            }
            3 => {
                // LARGE loop (>= pe_source's unroll cap of 64) — forces pe_source to RESIDUALIZE,
                // driving its dynamic-regime path (the one the small-loop corpus never reached).
                let k = 64 + next_rand(&mut rng) % 40; // 64..=103
                body.push_str(&format!("Repeat for gi from 1 to {k}:\n    Set {target} to ({target} + 1).\n"));
            }
            4 => {
                // If INSIDE a loop — nested dynamic control flow (modvar handling under a dynamic
                // loop), bounded oscillation so tree-walk and residual agree exactly.
                let k = 48 + next_rand(&mut rng) % 40; // 48..=87 (mostly >= cap → residualized)
                let cmp = cmps[(next_rand(&mut rng) as usize) % cmps.len()];
                let lit = 40 + next_rand(&mut rng) % 40;
                body.push_str(&format!(
                    "Repeat for gi from 1 to {k}:\n    If {target} {cmp} {lit}:\n        Set {target} to ({target} - 1).\n    Otherwise:\n        Set {target} to ({target} + 2).\n"
                ));
            }
            _ => {
                let e = gen_int_expr(&mut rng, &vars, 2);
                let v = format!("v{}", vars.len());
                body.push_str(&format!("Let mutable {v} be {e}.\n"));
                vars.push(v);
            }
        }
    }
    let vf = vars[(next_rand(&mut rng) as usize) % vars.len()].clone();
    body.push_str(&format!("Show {vf}.\n"));
    body
}

/// A small curated set of pathological-but-correct programs, paired with expected output.
/// Grows per phase ("robust to absurdity").
pub fn adversarial_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "## To fact (n: Int) -> Int:\n    If n <= 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(5).",
            "120",
        ),
        (
            "## To fib (n: Int) -> Int:\n    If n <= 1:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(10).",
            "55",
        ),
        (
            "## Main\nLet x be 2 + 3 * 4.\nShow x.",
            "14",
        ),
    ]
}
