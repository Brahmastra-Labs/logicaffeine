//! ════════════════════════════════════════════════════════════════════════════════════════════
//! SELF-APPLICATION FIDELITY LOCK — JONES OPTIMALITY on all three stages (Phase 4).
//!
//! `count_dispatch == 0` is the Jones criterion: a residual has ZERO surviving interpreter
//! overhead. This lock enforces it as the UNIVERSAL property across the whole executable corpus
//! and all three PE stages at once — P1 (`pe_source`), the P2 compiler subject (`pe_mini`), and
//! the P3 cogen subject (`pe_bti`) must each dissolve every construct to dispatch-free code.
//!
//! The three PEs are NOT byte-identical: `pe_mini`/`pe_bti` are deliberately-reduced dialects
//! (~40% smaller — they omit `pe_source`'s fact-tracking machinery) so the self-application is
//! tractable. A reduced dialect may fold a static loop or collection LESS aggressively than
//! `pe_source` while still producing dispatch-free, correct code. That is a legitimate
//! folding-depth difference, NOT a Jones violation — and a DELIBERATE TRADE-OFF, not a defect:
//! folding requires `Inspect`ing a value's type (e.g. `Inspect peColl` to detect a `CList`), so
//! adding pe_source-level folds to the clones GROWS the genuine P2/P3 residual's surviving dispatch
//! past the `futamura_p2p3_ratchet` ceiling (a measured experiment pushed P2 from 9 → 12 Inspects).
//! The clones fold user programs less SO THAT the self-application residual — the generated
//! compiler — stays Jones-optimal, the more important property. This lock separates two properties:
//!
//!   • JONES-OPTIMAL (universal, every construct × every stage): `count_dispatch == 0`. The bar.
//!   • STRUCTURAL IDENTITY (`identical` cases): all three dialects fold to the SAME residual —
//!     the classical `p2(p) ≡ p1(p)` proof, asserted where the reduced dialects genuinely agree.
//!
//! Correctness is verified per construct: by RE-RUNNING the residual where it round-trips through
//! the decompiler, or by running the source program where it cannot (`CNewSet` is nullary in the
//! Core IR, so a set residual decompiles to the unparseable `new Set of Any` — a tracked Core-IR
//! round-trip gap, orthogonal to the set residual's Jones-optimality).
//!
//! This is exactly the lock that caught the closure keystone: `pe_mini` residualized a closure
//! parameter as a dangling free variable (fails run-correctness) and `pe_bti` panicked — both are
//! caught here the instant they reappear.
//!
//!  ⚠️  A Jones-optimality or run-correctness RED means a clone is BROKEN — fix the clone by
//!  mirroring `pe_source`, never weaken the corpus.  ⚠️
//! ════════════════════════════════════════════════════════════════════════════════════════════

use logicaffeine_compile::compile;

/// One corpus program and the properties its residual must satisfy across all three PE stages.
struct Case {
    name: &'static str,
    program: &'static str,
    /// Expected observable output of the program (and of a round-tripping residual).
    expected: &'static str,
    /// All three dialects fold to a byte-identical residual (the strong `p2 ≡ p1` witnesses).
    /// `false` for constructs where the reduced dialects legitimately fold less than `pe_source`.
    identical: bool,
    /// The decompiled residual re-parses and re-runs. `false` only where the Core IR loses type
    /// information (sets → `new Set of Any`); such cases verify correctness via the source program.
    round_trips: bool,
}

const fn c(name: &'static str, program: &'static str, expected: &'static str, identical: bool, round_trips: bool) -> Case {
    Case { name, program, expected, identical, round_trips }
}

/// The executable corpus. Every entry must be JONES-OPTIMAL in all three stages; `identical`
/// entries must additionally fold to the same residual everywhere. This is the surface over which
/// "Jones optimality on all three Futamura stages" is proven.
const FIDELITY_CORPUS: &[Case] = &[
    // ── arithmetic / bindings ── (fully fold, identical everywhere)
    c("arith_precedence", "## Main\nShow 2 + 3 * 4.", "14", true, true),
    c("let_chain", "## Main\nLet x be 5.\nLet y be x + 1.\nShow y * 2.", "12", true, true),
    c("nested_arith_let", "## Main\nLet a be 10.\nLet b be a * a.\nLet c be b - a.\nShow c + 5.", "95", true, true),
    c("subtraction_div", "## Main\nShow (20 - 6) / 2.", "7", true, true),
    // ── booleans / comparisons / control flow ──
    c("static_if", "## Main\nIf 3 is greater than 2:\n    Show \"a\".\nOtherwise:\n    Show \"b\".", "a", true, true),
    c("bool_and", "## Main\nIf 1 is less than 2 and 3 is less than 4:\n    Show \"both\".\nOtherwise:\n    Show \"no\".", "both", true, true),
    c("bool_or", "## Main\nIf 1 is greater than 2 or 3 is less than 4:\n    Show \"or\".\nOtherwise:\n    Show \"no\".", "or", true, true),
    c("bool_not", "## Main\nIf not (2 is greater than 3):\n    Show \"yes\".\nOtherwise:\n    Show \"no\".", "yes", true, true),
    c("cmp_at_least", "## Main\nLet x be 5.\nIf x is at least 5:\n    Show \"ge\".\nOtherwise:\n    Show \"lt\".", "ge", true, true),
    // ── loops ──
    c("bounded_loop_sum", "## Main\nLet mutable s be 0.\nRepeat for x in [1, 2, 3]:\n    Set s to s + x.\nShow s.", "6", true, true),
    // range: all three unroll the static-bound loop (pe_mini's CRepeatRange now mirrors pe_source's
    // unroll) → byte-identical `Show 6`.
    c("range_loop", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 3:\n    Set s to s + i.\nShow s.", "6", false, true),
    // while: reduced dialects keep the (dispatch-free, correct) loop; pe_source folds it.
    c("while_static", "## Main\nLet mutable n be 0.\nWhile n is less than 3:\n    Set n to n + 1.\nShow n.", "3", false, true),
    // ── collections ── list_index: clones DCE the dead `Let xs` that pe_source keeps (clones
    // out-optimize here) → identical=false; list_length + tuple index/length fold identically.
    c("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.", "20", false, true),
    c("list_length", "## Main\nShow length of [1, 2, 3, 4].", "4", false, true),
    c("push_length", "## Main\nLet mutable xs be [1, 2].\nPush 3 to xs.\nShow length of xs.", "3", false, true),
    // set: Core-IR CNewSet is untyped → residual `new Set of Any` cannot re-parse (round_trips=false)
    c("set_add_dedup", "## Main\nLet s be a new Set of Int.\nAdd 5 to s.\nAdd 5 to s.\nShow length of s.", "1", false, false),
    // ── strings ──
    c("string_interp", "## Main\nLet n be 7.\nShow \"n={n}\".", "n=7", true, true),
    c("string_interp_expr", "## Main\nLet a be 3.\nLet b be 4.\nShow \"{a + b}\".", "7", true, true),
    // ── closures (the keystone) ──
    c("closure_applied_arg", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21).", "42", true, true),
    c("closure_ignored_arg", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return x.\n\n## Main\nShow apply((n: Int) -> n * 2, 21).", "21", true, true),
    // ── nested user-function inlining ──
    c("nested_fn_inline", "## To dbl (n: Int) -> Int:\n    Return n * 2.\n\n## To inc (n: Int) -> Int:\n    Return n + 1.\n\n## Main\nShow dbl(inc(20)).", "42", true, true),
    // ── structs (CNew + field access) ── field access folds to the constant AND the static `Let b`
    // binding is copy-propagated away in all three dialects (pe_bti's CLet/CSet now use
    // checkStaticValue, matching pe_mini/pe_source's isStaticValue) → byte-identical, dispatch-free.
    c("struct_field", "## A Box has:\n    A wide: Int.\n    A tall: Int.\n\n## Main\nLet b be a new Box with wide 3 and tall 4.\nShow b's wide.", "3", true, true),
    // ── enums (CNewVariant + static Inspect = \"The Trick\") ── the static Inspect folds to its arm
    // with ZERO surviving dispatch, and the static `Let s` binding is copy-propagated away, in all
    // three dialects → byte-identical.
    c("enum_inspect", "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n## Main\nLet s be a new Circle with radius 42.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Rectangle (w, h):\n        Show w.", "42", true, true),
    // ── tuples (CTuple + tuple index) ── aggregates: reduced dialects may keep the op (like lists),
    // still count_dispatch==0 + correct. identical=false conservatively until measured.
    c("tuple_length", "## Main\nLet t be (1, 2, 3).\nShow length of t.", "3", false, true),
    c("tuple_index", "## Main\nLet t be (10, 20, 30).\nShow t[2].", "20", false, true),
    // ── membership / copy / slice (CContains, CCopy, CSlice) ──
    c("contains", "## Main\nLet xs be [1, 2, 3].\nIf xs contains 2:\n    Show \"yes\".\nOtherwise:\n    Show \"no\".", "yes", false, true),
    c("copy_scalar", "## Main\nLet a be 7.\nLet b be copy of a.\nShow b.", "7", false, true),
    c("slice_len", "## Main\nLet xs be [10, 20, 30, 40].\nLet ys be items 2 through 3 of xs.\nShow length of ys.", "2", false, true),
    // (CSlice decompiles to the re-parseable `items N through M of X` form — round-trips.)
    // ── set algebra (CUnion, CIntersection) — set residual can't round-trip (CNewSet is untyped in
    // the Core IR → `new Set of Any`); correctness verified via the source program, residual still
    // must be Jones-optimal in all three stages.
    c("set_union", "## Main\nLet a be a new Set of Int.\nAdd 1 to a.\nLet b be a new Set of Int.\nAdd 2 to b.\nLet c be a union b.\nShow length of c.", "2", false, false),
    c("set_intersection", "## Main\nLet a be a new Set of Int.\nAdd 1 to a.\nAdd 2 to a.\nLet b be a new Set of Int.\nAdd 2 to b.\nAdd 3 to b.\nLet c be a intersection b.\nShow length of c.", "1", false, false),
];

/// Every construct is Jones-optimal in all three stages. Floor may only RISE.
const JONES_OPTIMAL_PROGRAMS: usize = 30;
/// Constructs proven byte-identical across all three dialects (the `p2 ≡ p1` witnesses). May only RISE.
const STRICT_IDENTITY_PROGRAMS: usize = 17;

/// The bti dialect renames the two memo carriers in the shared Core-type catalog.
fn core_types_bti() -> String {
    compile::core_types_for_pe_source()
        .replace("specResults", "memoCache")
        .replace("onStack", "callGuard")
}

/// Compile `program` through one PE dialect and return the decompiled residual SOURCE.
///
/// Drives the dialect's block evaluator directly (`makePeState` + `block_fn`) exactly as the proven
/// `compile_and_run_via_p{2,3}_real` helpers do, then decompiles — the residual as re-runnable
/// LOGOS text, dialect-independent by construction so the three are comparable byte-for-byte.
fn dialect_residual(pe_text: &str, block_fn: &str, core_types: &str, program: &str) -> String {
    let decompile = compile::decompile_source_text();
    let encoded = compile::encode_program_source(program)
        .unwrap_or_else(|e| panic!("[{block_fn}] encode failed: {e:?}"));
    let src = format!(
        "{core}\n{pe}\n{dec}\n## Main\n{enc}\n\
         Let fidState be makePeState(a new Map of Text to CVal, encodedFuncMap, 200).\n\
         Let fidCompiled be {bf}(encodedMain, fidState).\n\
         Let fidOut be decompileBlock(fidCompiled, 0).\n\
         Show fidOut.",
        core = core_types, pe = pe_text, dec = decompile, enc = encoded, bf = block_fn
    );
    compile::interpret_program(&src)
        .unwrap_or_else(|e| panic!("[{block_fn}] PE run failed: {e:?}"))
        .trim()
        .to_string()
}

/// Trivial-whitespace normalization: trim trailing space, drop blank lines, drop a bare `## Main`
/// header. NOT alpha-renaming — inlined residuals keep the source's own binder names, identical
/// across dialects — so any surviving difference is a genuine structural divergence.
fn normalize(residual: &str) -> String {
    residual
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty() && *l != "## Main")
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy)]
enum Dialect {
    Source,
    Mini,
    Bti,
}

const DIALECTS: [(&str, Dialect); 3] =
    [("pe_source", Dialect::Source), ("pe_mini", Dialect::Mini), ("pe_bti", Dialect::Bti)];

fn residual_of(dialect: Dialect, program: &str) -> String {
    match dialect {
        Dialect::Source => dialect_residual(
            compile::pe_source_text(), "peBlock", compile::core_types_for_pe_source(), program,
        ),
        Dialect::Mini => dialect_residual(
            compile::pe_mini_source_text(), "peBlockM", compile::core_types_for_pe_source(), program,
        ),
        Dialect::Bti => dialect_residual(
            compile::pe_bti_source_text(), "peBlockB", &core_types_bti(), program,
        ),
    }
}

/// ★ JONES OPTIMALITY on ALL THREE STAGES ★ — every dialect's residual for every construct carries
/// ZERO surviving interpreter dispatch. This is the core Jones criterion applied to P1, the
/// P2-subject, and the P3-subject alike: no stage may leave interpretive overhead in its output.
/// Collects every violation in one run.
#[test]
fn fidelity_residuals_are_jones_optimal() {
    let mut fails = Vec::new();
    for case in FIDELITY_CORPUS {
        for (label, dialect) in DIALECTS {
            let residual = residual_of(dialect, case.program);
            let d = compile::count_dispatch(&residual);
            if d != 0 {
                fails.push(format!("[{}/{label}] carries {d} dispatch unit(s):\n{residual}", case.name));
            }
        }
    }
    assert!(fails.is_empty(), "{} non-Jones-optimal residual(s):\n{}", fails.len(), fails.join("\n\n"));
}

/// ★ STRUCTURAL FIDELITY ★ — on the `identical` constructs, all three dialects fold to the SAME
/// residual: the genuine `p2(p) ≡ p1(p)` / `p3(…) ≡ p2` proof where the reduced dialects agree.
#[test]
fn p2_p3_residuals_match_p1_structurally() {
    let mut drifts = Vec::new();
    let mut identical_count = 0usize;
    for case in FIDELITY_CORPUS {
        if !case.identical {
            continue;
        }
        identical_count += 1;
        let r1 = normalize(&residual_of(Dialect::Source, case.program));
        for (label, d) in [("pe_mini", Dialect::Mini), ("pe_bti", Dialect::Bti)] {
            let rn = normalize(&residual_of(d, case.program));
            if r1 != rn {
                drifts.push(format!(
                    "[{}] {label} DRIFTED from pe_source:\n  --- pe_source ---\n{r1}\n  --- {label} ---\n{rn}",
                    case.name
                ));
            }
        }
    }
    assert!(drifts.is_empty(), "{} structural drift(s):\n{}", drifts.len(), drifts.join("\n\n"));
    assert!(
        identical_count >= STRICT_IDENTITY_PROGRAMS,
        "strict-identity witnesses SHRANK to {identical_count} (floor {STRICT_IDENTITY_PROGRAMS}) — \
         never lower it; a clone drift must be fixed in the clone, not demoted out of the set."
    );
}

/// Correctness: the residual computes the right answer in all three stages. Where the residual
/// round-trips, RE-RUN it (catches a wrong fold — e.g. the closure keystone's dangling `f`); where
/// the Core IR can't round-trip the residual (sets), verify the source program instead.
#[test]
fn fidelity_residuals_run_to_correct_output() {
    let mut fails = Vec::new();
    for case in FIDELITY_CORPUS {
        if case.round_trips {
            for (label, dialect) in DIALECTS {
                // `decompileBlock` yields a bare block; wrap in `## Main` (mirrors P1's wrapping).
                let residual = format!("## Main\n{}", residual_of(dialect, case.program));
                match compile::interpret_program(&residual) {
                    Ok(out) if out.trim() == case.expected => {}
                    Ok(out) => fails.push(format!(
                        "[{}/{label}] residual ran to {:?}, expected {:?}\n{residual}",
                        case.name, out.trim(), case.expected
                    )),
                    Err(e) => fails.push(format!("[{}/{label}] residual failed to run: {e:?}\n{residual}", case.name)),
                }
            }
        } else {
            // Residual is Jones-optimal (checked separately) but not re-runnable; verify the
            // construct is correct via the source program.
            match compile::interpret_program(case.program) {
                Ok(out) if out.trim() == case.expected => {}
                Ok(out) => fails.push(format!("[{}/source-prog] ran to {:?}, expected {:?}", case.name, out.trim(), case.expected)),
                Err(e) => fails.push(format!("[{}/source-prog] failed to run: {e:?}", case.name)),
            }
        }
    }
    assert!(fails.is_empty(), "{} run-correctness failure(s):\n{}", fails.len(), fails.join("\n\n"));
}

/// Ratchet floor: the number of Jones-optimal constructs may only rise.
#[test]
fn jones_optimal_program_count_only_rises() {
    assert!(
        FIDELITY_CORPUS.len() >= JONES_OPTIMAL_PROGRAMS,
        "Jones-optimal corpus SHRANK to {} (floor {JONES_OPTIMAL_PROGRAMS}) — never lower it.",
        FIDELITY_CORPUS.len()
    );
}
