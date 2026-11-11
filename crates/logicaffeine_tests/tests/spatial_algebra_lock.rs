//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE ANTI-ACCRETION RATCHET — a new tensor workload is a COMPOSITION, never a variant.
//!
//! ★ THE PROPERTY, IN ONE SENTENCE: every future workload must be expressible as a chain of the
//! [`ComputeOp`]s that already exist, so the cost of teaching LOGOS a new tensor operation is a pass
//! that composes — never one enum variant plus one print arm in every backend.
//!
//! ★ WHY THIS FILE IS THE CENTRE OF THE ONE-SPATIAL-IR CAMPAIGN. The default gravity of a
//! multi-backend compiler is accretion: someone needs `gelu`, so they add `ComputeOp::Gelu`, and
//! then `cuda.rs`, `fabric.rs`, `tensix/kernel.rs` and `tensix/host.rs` each grow an arm — four
//! arms for one workload, times four backends the day a fifth lands. Do that thirty times and the
//! backend IS a hundred print arms: a case analysis over workloads, which is precisely the thing
//! the spatial IR was built to abolish. The asymmetry is already visible in the tree. The FRONT end
//! is general — `FusedRowSpec` carries `reduce: RowReduceKind` plus `weight`/`residual`/`bias`
//! options, so rmsnorm, fused-add-rmsnorm, softmax and layernorm are all *parameter values* of one
//! recognized shape. The BACK end is a case analysis — 67 `ComputeOp::` tokens across four printer
//! files. This lock freezes that number so the two ends converge from the back end's side: the
//! front end may keep growing sideways, the printers may not.
//!
//! ★ THE THREE TEETH.
//!   1. `the_compute_taxonomy_is_closed` makes a new variant cost a WRITTEN CLASSIFICATION — you
//!      must say which of three things you are adding and why, in this file, before it goes green.
//!   2. `no_backend_enumerates_compute_shapes` makes the print arm itself impossible: per-file
//!      `ComputeOp::` counts under `spatial/lower/` are frozen, and a new arm raises one.
//!   3. `the_newest_workload_is_a_composition_not_a_variant` is the half that stops a FALSE ratchet.
//!      Freezing counts proves nothing if the only way to add a workload were to add a variant — the
//!      ratchet would just be a wall. So the door is proven open by walking through it: rmsnorm, the
//!      newest tensor workload, is shown to be a chain of ops that already existed.
//!
//! Together they leave exactly one door open, which is the door the campaign wants used — write a
//! legalization pass in `spatial/passes/` that lowers your workload into the existing algebra, the
//! way `from_tensor_op::from_fused_row` lowers rmsnorm into square → reduce → rsqrt → rescale →
//! weight without a single new variant or a single new print arm.
//!
//! ★ THE THREE CLASSES, and why three is the right number:
//!   • GENERAL FORM      — the one growth point. The op is a *shape* whose content is a parameter
//!                         (`Elementwise`'s `ElementwiseOp`, `Scalar`'s `ScalarBody`). New work
//!                         arrives as a new parameter value and costs ZERO print arms.
//!   • HARDWARE INTRINSIC— the op names a physical unit: the matrix engine, the reduce engine, the
//!                         SFPU transform slot, a broadcast datapath, the packer's convert. A
//!                         backend either has the unit or refuses BY NAME. Its print arm is a
//!                         driver for silicon, not a workload. This class grows when silicon does.
//!   • SPECIFICATION OP  — the op states a whole workload's meaning so the oracle in `spatial/eval.rs`
//!                         can evaluate it and a composition can be DIFFED against it. A pass
//!                         legalizes it away; no backend needs to know it exists.
//!
//! ★ ON THE FROZEN NUMBERS. They are today's counts, so this file is green the hour it lands. It
//! becomes load-bearing the moment anyone adds an arm — which is the point: a ratchet is worth
//! nothing on the day it is written and everything on the day it is tested. Raising a frozen number
//! is not forbidden, it is *expensive*: it requires a written justification in the table beside the
//! number saying which silicon unit appeared, because that is the only reason a printer should ever
//! learn one more shape. Re-freezing a number DOWNWARD needs no ceremony at all — that direction is
//! the campaign finishing.
//!
//! ★ WHAT THIS FILE DOES NOT CLAIM, said plainly so nobody reads more into a green run than is
//! there. (a) Its scope is `src/spatial/lower/` — a case analysis over compute shapes written
//! somewhere else entirely (a helper in `codegen_tt`, say) is out of this ratchet's reach;
//! `spatial_one_ir_lock` in `logicaffeine-tests` is the lock that owns bespoke bodies outside the
//! printers. (b) It reads source text, so it ratchets the SHAPE of the back end, never the
//! correctness of a lowering — `spatial_tensix_p4_lock` and `spatial_one_ir_lock` own that.
//!
//! Run: `cargo nextest run -p logicaffeine-compile -E 'binary(spatial_algebra_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use logicaffeine_compile::spatial::ir::{ComputeOp, IterKind, Phase, StageKind, TensorBody};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// `crates/logicaffeine_compile/src/spatial` — this test reads the compiler's own source text,
/// because a second hand-maintained list of what the enum contains is exactly the drift this lock
/// exists to prevent.
fn spatial_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_compile/src")
        .join("spatial")
}

/// A file's text with whole-line comments removed.
///
/// ★ EVERY TEXT PREDICATE IN THIS FILE RUNS ON CODE, NEVER ON PROSE, and both directions of that
/// matter. A doc comment saying `[`ComputeOp::Pack`]` must NOT be able to stand in for the arm that
/// evaluates it (the check would pass while the oracle went blind), and a comment that happens to
/// spell `ComputeOp::` must NOT inflate a printer's frozen count — an inflated ceiling is headroom,
/// and headroom means an arm can be added by deleting a sentence. Stripping the prose costs nothing:
/// every count in `FROZEN_LOWER_ARMS` is identical with and without it today.
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_code(p: &Path) -> String {
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| {
        panic!("{}: {e}\n\nthis lock reads the compiler's own source; if the path moved, retarget `spatial_src()` in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs.", p.display())
    });
    code_only(&text)
}

// ────────────────────────────────────────────────────────────────────────────────────────────────
// (a) THE CLOSED TAXONOMY
// ────────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// The one growth point: content is a parameter, so growth costs no print arms.
    GeneralForm,
    /// A physical unit. A backend has it or refuses by name.
    HardwareIntrinsic,
    /// Evaluated for the oracle, legalized by a pass, never lowered as itself.
    SpecificationOp,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::GeneralForm => "GENERAL FORM",
            Class::HardwareIntrinsic => "HARDWARE INTRINSIC",
            Class::SpecificationOp => "SPECIFICATION OP",
        }
    }
}

/// ★ THE FROZEN TABLE. Every `ComputeOp` variant, classified once, with the sentence that earns it
/// its class. The third column is not decoration — `the_compute_taxonomy_is_closed` refuses an
/// empty or perfunctory justification, so the only way past this table is to have thought about
/// which of the three things you are adding.
///
/// ★ ADDING A ROW IS THE EXPENSIVE PATH AND IS MEANT TO FEEL THAT WAY. Before you add one, ask the
/// question this campaign exists to ask: can the workload be a chain of rows that are ALREADY here?
/// If it can, it must be — write the pass in `spatial/passes/from_tensor_op.rs` instead, the way
/// rmsnorm is written, and this table does not change at all.
const TAXONOMY: &[(&str, Class, &str)] = &[
    (
        "MatmulTiles",
        Class::HardwareIntrinsic,
        "the matrix engine itself — `matmul_tiles` on Tensix, an MMA fragment on CUDA, the systolic \
         array on the fabric. Nothing composes into a matrix unit; a target has one or it does not.",
    ),
    (
        "Generic",
        Class::GeneralForm,
        "★ THE GROWTH POINT. An iteration space, an affine access map per operand, and a scalar body. \
         A reduction is an ITERATOR KIND here and a broadcast is an ABSENT TERM in a map, so the two \
         things that used to need operators need none: `RowRescale` and `RowBroadcast` are one map \
         read two ways, and the printer recovers which broadcast the silicon wants from which axis is \
         missing. A new workload is a new COMPOSITION of these — new `SfpuKind`s and new bodies at \
         most — and costs zero print arms. If you are about to add a row to this table, the question \
         to answer first is why your workload is not a chain of these.",
    ),
    (
        "Requant",
        Class::HardwareIntrinsic,
        "the integer requantize epilogue (`UnaryKind::Requant`) — a fixed multiply-and-shift in the \
         fabric's output stage, not something a general elementwise chain can express at that width.",
    ),
    (
        "Bias",
        Class::HardwareIntrinsic,
        "the accumulate-add epilogue (`UnaryKind::Bias`) that runs inside the fabric's output stage, \
         before anything is packed out and could be read back by a general form.",
    ),
    (
        "Relu",
        Class::HardwareIntrinsic,
        "the activation epilogue (`UnaryKind::Relu`) fused into that same output stage — the clamp \
         the datapath performs for free on the way out.",
    ),
    (
        "Scalar",
        Class::GeneralForm,
        "an arbitrary scalar dataflow body (`ScalarBody`: cells plus a result `Src`) applied \
         element-wise. The universal escape hatch — a pointwise workload with no name at all can be \
         spelled here without touching this table or any printer.",
    ),
];

/// How many variants each class may hold, frozen with the table above.
///
/// ★ THE INTRINSIC COUNT IS A CLAIM ABOUT SILICON, NOT ABOUT CONVENIENCE. Raising it says a new
/// physical unit exists that nothing composes into. Raising it because a workload was easier to
/// print than to legalize is the exact accretion this file forbids.
///
/// ★ THESE MUST SUM TO `TAXONOMY.len()`, and the test asserts it. Without that one line a fourth
/// `Class` could be invented, given rows, and left out of this table — and every row wearing it
/// would be frozen at nothing, which is a classification lock that has quietly stopped classifying.
const FROZEN_CLASS_SIZES: &[(Class, usize)] = &[
    (Class::GeneralForm, 2),
    (Class::HardwareIntrinsic, 4),
    // ★ ZERO, AND THAT IS THE FINISHED STATE. `RowReduce` was the last specification op; once softmax
    // and layernorm became `Generic` chains nothing constructed it, so a variant only the printers
    // would ever see was deleted rather than kept for sentiment. The class stays in the taxonomy
    // because the CONCEPT is how a future high-level op (attention, conv) should enter — stated whole
    // for the oracle, legalized away by a pass — not because anything occupies it today.
    (Class::SpecificationOp, 0),
];

/// The variant names `pub enum ComputeOp` actually declares, read out of `ir.rs`'s source text.
///
/// ★ PARSED, NEVER LISTED. A hand-written copy of the enum's contents would be a second source of
/// truth that drifts silently — and a taxonomy lock whose idea of the enum is stale asserts nothing.
/// Walking the text means the enum itself is the input, so a variant added anywhere in it arrives
/// here whether or not its author remembered this file existed.
fn compute_op_variants(ir_src: &str) -> Vec<String> {
    let head = ir_src.find("pub enum ComputeOp").expect(
        "`pub enum ComputeOp` must be findable in crates/logicaffeine_compile/src/spatial/ir.rs — if \
         the enum was renamed or moved, point this parser at its new home. Never delete this lock to \
         get past a rename.",
    );
    let open = ir_src[head..]
        .find('{')
        .expect("the enum declaration must open a body")
        + head
        + 1;
    let mut names = Vec::new();
    let mut depth = 0usize;
    for line in ir_src[open..].lines() {
        let t = line.trim();
        // Comment and attribute lines carry brackets and braces in prose (`[`ComputeOp::ReduceRows`]`,
        // `Y[r,c]`), so they are skipped whole rather than counted — the depth walk must see code only.
        if t.is_empty() || t.starts_with("//") || t.starts_with("#[") {
            continue;
        }
        if depth == 0 && t.starts_with(|c: char| c.is_ascii_uppercase()) {
            names.push(
                t.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>(),
            );
        }
        for c in t.chars() {
            match c {
                '{' | '(' => depth += 1,
                '}' | ')' => {
                    if depth == 0 {
                        return names; // the enum's closing brace
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    names
}

/// A `ComputeOp`'s variant name as the value itself reports it, so a runtime program can be checked
/// against the same table the source text is checked against.
fn variant_name(op: &ComputeOp) -> String {
    format!("{op:?}")
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn class_of(name: &str) -> Option<Class> {
    TAXONOMY
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, c, _)| *c)
}

#[test]
fn the_compute_taxonomy_is_closed() {
    let ir = spatial_src().join("ir.rs");
    let text = std::fs::read_to_string(&ir).unwrap_or_else(|e| panic!("{}: {e}\n\nthis lock reads the compiler's own source; if the path moved, retarget `spatial_src()`.", ir.display()));
    let found = compute_op_variants(&text);

    // ── The parser must not be the reason this passes. A silent empty parse would make every
    //    assertion below vacuous, so the parse itself is checked before it is trusted.
    // ★ THE FLOOR TRACKS THE ENUM DOWNWARD, BUT ONLY DELIBERATELY. It was 10 when the enum held 13
    //    variants; the S2 collapse retired six of them for one general form, so 10 became
    //    unreachable. It is NOT a knob to turn whenever this fails: a floor above zero is the only
    //    thing standing between a broken parser and a lock that silently asserts nothing. Lower it
    //    only alongside a deletion you can name, and never below the number of hardware intrinsics —
    //    those cannot collapse further, so they are the honest bottom.
    assert!(
        found.len() >= 6,
        "the ComputeOp parser found only {} variant(s) in {} — it has stopped understanding the \
         enum's shape. Fix `compute_op_variants` in tests/spatial_algebra_lock.rs; never soften this \
         floor, because a parser that finds nothing makes the whole taxonomy lock vacuous.",
        found.len(),
        ir.display()
    );
    assert!(
        found.iter().any(|v| v == "MatmulTiles"),
        "the ComputeOp parser did not find `MatmulTiles`, which the enum certainly declares — the \
         parse is wrong. Fix `compute_op_variants` in tests/spatial_algebra_lock.rs, not this check."
    );
    assert!(
        found.iter().all(|v| !v.is_empty()),
        "the ComputeOp parser produced an empty variant name from {} — it mis-read a line. Fix \
         `compute_op_variants` in tests/spatial_algebra_lock.rs.",
        ir.display()
    );
    let mut uniq = found.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(
        uniq.len(),
        found.len(),
        "the ComputeOp parser reported a duplicate variant name ({found:?}) — it is counting field \
         lines as variants. Fix `compute_op_variants` in tests/spatial_algebra_lock.rs."
    );

    // ── The set equality: the enum and the table are the same set, exactly.
    let table: BTreeSet<&str> = TAXONOMY.iter().map(|(n, _, _)| *n).collect();
    let live: BTreeSet<&str> = found.iter().map(|s| s.as_str()).collect();

    // A name listed twice would collapse in `table` and hide a second, contradictory classification
    // behind a set comparison that still looked exact.
    assert_eq!(
        table.len(),
        TAXONOMY.len(),
        "TAXONOMY (crates/logicaffeine_tests/tests/spatial_algebra_lock.rs) lists {} row(s) but \
         only {} distinct variant names — one variant is classified twice. Delete the duplicate row; \
         a variant gets exactly one class and exactly one justification.",
        TAXONOMY.len(),
        table.len()
    );

    let unclassified: Vec<&str> = live.difference(&table).copied().collect();
    assert!(
        unclassified.is_empty(),
        "\n★ A NEW ComputeOp VARIANT WAS ADDED WITHOUT A CLASSIFICATION: {}\n\n\
         Before this goes green, answer the question this campaign exists to ask, in \
         crates/logicaffeine_tests/tests/spatial_algebra_lock.rs::TAXONOMY:\n\
         \x20 • Can this workload be a CHAIN of variants that already exist? If it can, it must be — \
         delete the variant and write the composition in `spatial/passes/from_tensor_op.rs`, the way \
         `from_fused_row` builds rmsnorm out of square → reduce → rsqrt → rescale → weight with no \
         new variant and no new print arm.\n\
         \x20 • If it truly cannot, add a row naming its class (GENERAL FORM / HARDWARE INTRINSIC / \
         SPECIFICATION OP) and the sentence that earns it that class, and raise that class's entry \
         in FROZEN_CLASS_SIZES.\n\n\
         The fix is a classification or a composition — it is NEVER to delete this assertion, and \
         never to widen it to accept an unclassified variant.\n",
        unclassified.join(", ")
    );

    let stale: Vec<&str> = table.difference(&live).copied().collect();
    assert!(
        stale.is_empty(),
        "\nthe taxonomy classifies variant(s) that `pub enum ComputeOp` no longer declares: {}\n\n\
         If they were legalized away — the direction of progress — delete their rows from TAXONOMY \
         in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs and lower the matching entry \
         in FROZEN_CLASS_SIZES. If they were merely renamed, rename the row; a rename must never be \
         used to smuggle in a variant the old row's justification does not cover.\n",
        stale.join(", ")
    );

    // ── A row without a real justification is an unclassified variant wearing a label.
    for (name, class, why) in TAXONOMY {
        assert!(
            why.len() >= 40,
            "the taxonomy row for `{name}` ({}) carries no real justification. Write, in \
             crates/logicaffeine_tests/tests/spatial_algebra_lock.rs::TAXONOMY, the sentence that \
             earns it its class — which silicon unit it names, or which parameter carries its \
             growth. A blank excuse is how a hundred print arms start; do not shorten this floor to \
             get past it.",
            class.label()
        );
    }

    // ── Class sizes, frozen with the table. The sum is checked FIRST, because it is what makes the
    //    per-class checks exhaustive: a row wearing a class that FROZEN_CLASS_SIZES never mentions
    //    passes every per-class assertion below by simply not being looked at.
    assert_eq!(
        FROZEN_CLASS_SIZES.iter().map(|(_, n)| *n).sum::<usize>(),
        TAXONOMY.len(),
        "FROZEN_CLASS_SIZES sums to {} but TAXONOMY holds {} row(s) \
         (crates/logicaffeine_tests/tests/spatial_algebra_lock.rs). Every row must be counted by \
         exactly one frozen class, or the classification lock has rows it never checks. If you added \
         a new `Class`, give it an entry here; never fix this by deleting the sum.",
        FROZEN_CLASS_SIZES.iter().map(|(_, n)| *n).sum::<usize>(),
        TAXONOMY.len()
    );
    for (class, frozen) in FROZEN_CLASS_SIZES {
        let n = TAXONOMY.iter().filter(|(_, c, _)| c == class).count();
        assert_eq!(
            n,
            *frozen,
            "the {} class now holds {n} variant(s), frozen at {frozen}. Adjust the number in \
             FROZEN_CLASS_SIZES (crates/logicaffeine_tests/tests/spatial_algebra_lock.rs) ONLY \
             with a written justification beside it: raising GENERAL FORM means a genuinely new \
             *shape* whose content is a parameter, raising HARDWARE INTRINSIC is a claim that new \
             silicon exists that nothing composes into, and raising SPECIFICATION OP means a new \
             reference semantics the oracle must state so a composition can be diffed against it. \
             Lowering any of them needs no ceremony — that is the campaign finishing.",
            class.label()
        );
    }

    // ── Every op must be EVALUATED. The oracle in `spatial/eval.rs` is what makes a composition
    //    provable against the thing it replaces; an op it cannot evaluate can never be legalized.
    let eval = spatial_src().join("eval.rs");
    let eval_code = read_code(&eval);
    assert!(
        eval_code.len() > 1000,
        "no code was read from {} — every arm check below would be vacuous. Retarget `spatial_src()` \
         in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs.",
        eval.display()
    );
    for (name, class, _) in TAXONOMY {
        assert!(
            eval_code.contains(&format!("ComputeOp::{name}")),
            "`ComputeOp::{name}` ({}) is named nowhere in the CODE of {} — the oracle cannot say what \
             it MEANS. (Prose is stripped before this check on purpose: a doc comment mentioning the \
             op must never stand in for the arm that evaluates it.)\n\n\
             Add the arm to `Eval::compute` there. Every op must be evaluable or a legalization pass \
             has nothing to diff its composition against, which is the whole mechanism by which a \
             workload becomes a chain instead of a variant. The fix is never to exempt an op here.",
            class.label(),
            eval.display()
        );
    }

    // ── A SPECIFICATION OP must be named by a pass.
    let passes = spatial_src().join("passes");
    let mut passes_code = String::new();
    for e in std::fs::read_dir(&passes).unwrap_or_else(|e| panic!("{}: {e}", passes.display())) {
        let p = e.expect("a readable dir entry").path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            passes_code.push_str(&read_code(&p));
            passes_code.push('\n');
        }
    }
    assert!(
        passes_code.len() > 1000,
        "no pass code was read from {} — the check below would be vacuous. Retarget `spatial_src()` \
         in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs.",
        passes.display()
    );
    for (name, class, _) in TAXONOMY {
        if *class != Class::SpecificationOp {
            continue;
        }
        assert!(
            passes_code.contains(&format!("ComputeOp::{name}")),
            "`ComputeOp::{name}` is classified SPECIFICATION OP but no pass under {} names it in \
             code. A specification op is a thing the PASS LAYER owns — it is built there by the \
             recognizer and legalized away there; one no pass mentions is a variant only the printers \
             will ever see, which is one print arm per backend for one workload.\n\n\
             ★ WHAT THIS CHECKS, EXACTLY, so nobody reads more into it than is here: that the pass \
             layer NAMES the op — not that a pass legalizes it. The two differ today. `RowReduce` is \
             genuinely legalized (rmsnorm becomes a stage chain, and \
             `the_newest_workload_is_a_composition_not_a_variant` in this file proves it by running \
             the compiler). `SwiGlu` is only CONSTRUCTED by the recognizer and is still lowered \
             directly by `spatial/lower/tensix/kernel.rs` and `host.rs` — that debt is written into \
             its TAXONOMY row, and retiring it LOWERS two frozen counts in FROZEN_LOWER_ARMS.\n\n\
             Either write the legalization in `spatial/passes/from_tensor_op.rs`, or re-classify the \
             op in TAXONOMY with the justification that class demands. Never relax this assertion.",
            passes.display()
        );
    }

    println!(
        "compute algebra: {} variants — {} general form, {} hardware intrinsic, {} specification op",
        TAXONOMY.len(),
        TAXONOMY.iter().filter(|(_, c, _)| *c == Class::GeneralForm).count(),
        TAXONOMY.iter().filter(|(_, c, _)| *c == Class::HardwareIntrinsic).count(),
        TAXONOMY.iter().filter(|(_, c, _)| *c == Class::SpecificationOp).count(),
    );
}

// ────────────────────────────────────────────────────────────────────────────────────────────────
// (b) NO BACKEND ENUMERATES COMPUTE SHAPES
// ────────────────────────────────────────────────────────────────────────────────────────────────

/// The token that marks a backend performing a case analysis over workload shapes.
const ARM_TOKEN: &str = "ComputeOp::";

/// Spellings that would let a printer keep its case analysis while [`ARM_TOKEN`] stopped seeing it.
///
/// ★ THE COUNTER IS ONLY AS HONEST AS THE SPELLING IT COUNTS. `use …::ComputeOp::*;` (or naming the
/// variants one by one, or `ComputeOp as Op`) lets `Gelu { .. } =>` be a perfectly good match arm
/// that this file scores as ZERO — and because shrinking is allowed and celebrated here, that
/// evasion would read as the campaign WINNING while the printers grew. So the evasion is refused
/// outright rather than counted: inside `spatial/lower/`, a compute op is spelled with its enum.
const BLIND_SPOTS: &[&str] = &["ComputeOp::*", "ComputeOp::{", "ComputeOp as "];

/// (file under `spatial/lower/`, `ComputeOp::` tokens in code) — frozen on 2026-09-09 at exactly
/// what the printers carry today, so this lock is green the hour it lands and bites the hour anyone
/// adds an arm. A file absent from this table is frozen at ZERO: a NEW backend that case-analyses
/// compute shapes fails immediately, which is the point — the fifth backend is the one that would
/// have turned four arms per workload into five.
///
/// The counts include refusal arms, deliberately. A refusal is still the printer knowing the name of
/// a workload, and a printer that must be taught to say "no" to every new op is still a case
/// analysis over ops — it has just been given a polite failure mode. The way a count goes DOWN is
/// the way the campaign wins: an op is legalized into the algebra by a pass, and then neither the
/// lowering arm nor the refusal arm needs to exist.
///
/// ★ RAISING A NUMBER HERE REQUIRES A WRITTEN JUSTIFICATION BESIDE IT naming the silicon unit the
/// new arm drives — that is the only legitimate reason a printer learns one more shape. "The
/// workload was easier to print than to legalize" is not a justification, it is the failure mode.
/// LOWERING a number needs no ceremony at all; re-freeze downward and move on.
const FROZEN_LOWER_ARMS: &[(&str, usize)] = &[
    // ★ RE-FROZEN DOWNWARD AGAIN AFTER S3 — AND THIS IS THE CAMPAIGN'S WHOLE CLAIM, MEASURED.
    //   Softmax and layernorm were ADDED as workloads in the same change that took these counts from
    //   36 to 32. Adding two workloads did not cost one print arm; it retired the last specification
    //   op, so it paid four back. 63 arms across four printers at the start, 32 now, with two more
    //   workloads lowering than when we began.
    ("cuda.rs", 10),
    ("fabric.rs", 8),
    ("tensix/host.rs", 7),
    ("tensix/kernel.rs", 7),
    // ★ THE FIFTH PRINTER, AND WHY ITS NUMBER IS TWO. `spatial_lang.rs` prints the verified program in
    //   Spatial — the RDU's own front-door language (Plasticine's PCU/PMU lineage; the public Spatial →
    //   PIR → Tungsten chain). The silicon units it drives: the PCU's lane-parallel reduction tree
    //   (`Reduce`/`MemReduce` — the matrix engine's dot products) and the PMU's N-buffered SRAM. It
    //   spells the compute enum in exactly ONE place — a `Shape` view every other site matches on —
    //   so the count is the two variants a printer must know (the intrinsic `MatmulTiles`, the general
    //   `Generic`); the epilogues and scalar bodies are refused through that same view, by name, with
    //   no arm of their own. A new variant costs this file nothing: the view's `_ =>` refuses it.
    ("spatial_lang.rs", 2),
];

#[test]
fn no_backend_enumerates_compute_shapes() {
    // ── The counting machinery is checked against a literal first. There is deliberately NO floor
    //    on the real counts below (unlike `spatial_one_ir_lock`, whose printers must keep their
    //    choreography vocabulary) because here shrinking to zero is the goal — so vacuity has to be
    //    ruled out some other way, and this plus `BLIND_SPOTS` plus the MatmulTiles probe is it.
    assert_eq!(
        code_only("// ComputeOp::Ignored\nComputeOp::A ComputeOp::B")
            .matches(ARM_TOKEN)
            .count(),
        2,
        "the `{ARM_TOKEN}` counter does not count, or `code_only` does not strip prose — every \
         assertion in this test would be measuring the wrong thing. Fix them in \
         crates/logicaffeine_tests/tests/spatial_algebra_lock.rs."
    );

    let lower = spatial_src().join("lower");
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).unwrap_or_else(|e| panic!("{}: {e}", d.display())) {
            let p = e.expect("a readable dir entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    walk(&lower, &mut files);
    files.sort();

    assert!(
        files.len() >= 8,
        "only {} rust file(s) found under {} — the walk found the wrong directory and every count \
         below would be a vacuous zero. Retarget `spatial_src()` in \
         crates/logicaffeine_tests/tests/spatial_algebra_lock.rs.",
        files.len(),
        lower.display()
    );

    let mut violations: Vec<String> = Vec::new();
    let mut shrunk: Vec<String> = Vec::new();
    let mut blinded: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut drives_the_matrix_unit = false;
    let mut total = 0usize;

    for f in &files {
        let rel = f
            .strip_prefix(&lower)
            .expect("a file under lower/")
            .to_string_lossy()
            .replace('\\', "/");
        let code = read_code(f);
        let count = code.matches(ARM_TOKEN).count();
        total += count;
        let frozen = FROZEN_LOWER_ARMS
            .iter()
            .find(|(file, _)| *file == rel)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        if count > frozen {
            violations.push(format!(
                "  {rel}: `{ARM_TOKEN}` ×{count} (frozen {frozen}, +{})",
                count - frozen
            ));
        } else if count < frozen {
            shrunk.push(format!(
                "  {rel}: `{ARM_TOKEN}` ×{count} (frozen {frozen}, −{})",
                frozen - count
            ));
        }
        for spot in BLIND_SPOTS {
            if code.contains(spot) {
                blinded.push(format!("  {rel}: `{spot}`"));
            }
        }
        if code.contains("ComputeOp::MatmulTiles") {
            drives_the_matrix_unit = true;
        }
        seen.insert(rel);
    }

    for (file, _) in FROZEN_LOWER_ARMS {
        assert!(
            seen.contains(*file),
            "the frozen printer `{file}` was not found under {} — a file this ratchet watches was \
             moved or renamed, and an unwatched printer is an unratcheted one. Retarget its row in \
             FROZEN_LOWER_ARMS (crates/logicaffeine_tests/tests/spatial_algebra_lock.rs) at the \
             new path; never delete the row, because deleting it silently re-freezes that printer at \
             zero and hides every arm it holds.",
            lower.display()
        );
    }

    assert!(
        blinded.is_empty(),
        "\n★ A PRINTER IMPORTED `ComputeOp`'s VARIANTS INSTEAD OF SPELLING THEM — which would make \
         every count in this file read zero while the case analysis stayed exactly where it was:\n{}\n\n\
         Inside `spatial/lower/`, spell a compute op with its enum (`ComputeOp::MatmulTiles {{ .. }}`). \
         The fix is to change the import, never to add a spelling to BLIND_SPOTS' exemption — there \
         is no exemption list, because a counter with exemptions is a counter that can be walked \
         around.\n",
        blinded.join("\n")
    );

    assert!(
        drives_the_matrix_unit,
        "no printer under {} names `ComputeOp::MatmulTiles` in code any more. Someone must drive the \
         matrix engine, so the driver moved rather than vanished — point this non-vacuity check at \
         its new home in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs. Do not delete the \
         check: it is what proves the walk above is reading real printers.",
        lower.display()
    );

    assert!(
        violations.is_empty(),
        "\n★ A BACKEND GREW A COMPUTE-SHAPE ARM — the printers are becoming a case analysis over \
         workloads:\n{}\n\n\
         The fix is a PASS, not a print arm. Legalize the workload into the existing algebra in \
         `crates/logicaffeine_compile/src/spatial/passes/from_tensor_op.rs` — that is how rmsnorm \
         reaches four backends without any of them knowing the word 'rmsnorm' — and the count here \
         does not move. If the front end grew a shape (`FusedRowSpec` and friends are already \
         general), that growth belongs entirely in the passes.\n\n\
         The fix is NEVER to raise the frozen number to match, unless you write a justification \
         beside it in FROZEN_LOWER_ARMS naming the SILICON UNIT the new arm drives. One arm per \
         workload per backend is the hundred-print-arm compiler this lock exists to prevent; a \
         number raised without that sentence is that compiler arriving one arm at a time.\n",
        violations.join("\n")
    );

    if !shrunk.is_empty() {
        // Progress, not a failure — but it is announced, because an unrecorded shrink means the
        // ratchet is looser than the code and the next regression slips in under the old ceiling.
        println!(
            "↓ the printers shrank — re-freeze DOWNWARD in FROZEN_LOWER_ARMS (no justification \
             needed, this direction is the campaign finishing):\n{}",
            shrunk.join("\n")
        );
    }

    let ceiling: usize = FROZEN_LOWER_ARMS.iter().map(|(_, n)| *n).sum();
    println!(
        "printer compute-shape arms: {total} (ceiling {ceiling}) across {} files",
        files.len()
    );
}

// ────────────────────────────────────────────────────────────────────────────────────────────────
// (c) THE POSITIVE PROOF — the growth point is real, and it has already been used
// ────────────────────────────────────────────────────────────────────────────────────────────────

/// Weighted rmsnorm — the most recently added tensor workload, and the one that proves the property.
/// Its sugar is shared verbatim with `spatial_tensix_p4_lock`, so the two locks agree on what
/// "the newest workload" is.
const SUGAR_RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 1.0.
Set item 2 of W to 1.0.
Set item 3 of W to 1.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// The composition weighted rmsnorm legalizes to, written out here rather than read back from the
/// compiler: square (Elementwise Mul) → reduce_rows (ReduceRows) → rsqrt (RowXform) → rescale
/// (RowRescale) → weight (RowBroadcast Mul), built by `from_fused_row`.
///
/// ★ THIS IS AN INDEPENDENT REFERENCE, NOT AN ECHO. Asking the compiler what it emitted and then
/// asserting that it emitted that would lock in nothing at all; the whole value of the list is that
/// a human wrote it down from the pass's intent, so a silent change of shape has to be re-argued
/// here before it can go green.
/// ★ NOW THAT EVERY STAGE IS `Generic`, THE NAMES NO LONGER DISCRIMINATE — so this reference moved
/// down a level, to the BODIES. Asserting five `Generic`s would be nearly vacuous; asserting the
/// sequence of bodies still says exactly what rmsnorm is: square, reduce, rsqrt, rescale, weight.
/// ★ FIVE STAGES, AND `fuse_generic` IS RIGHT NOT TO COLLAPSE THEM. The rsqrt runs on the ONE tile a
/// reduction produced; the rescale runs across all `cols`. Their iteration spaces genuinely differ, so
/// fusing would recompute the reciprocal square root once per column instead of once per row. The pass
/// declines, and that is the cost model of fusion being honest rather than eager.
const RMSNORM_CHAIN: &[&str] = &["Mul", "Reduce", "Rsqrt", "Mul", "Mul"];

#[test]
fn the_newest_workload_is_a_composition_not_a_variant() {
    // ★ THE HALF THAT STOPS A FALSE RATCHET. Freezing counts proves nothing if the only way to add a
    // workload were to add a variant — the ratchet would just be a wall. This is the door: rmsnorm,
    // the newest tensor workload, is a CHAIN of ops that already existed, with no `ComputeOp::RmsNorm`
    // anywhere. If this test ever goes red because the chain was replaced by a fused variant, the
    // campaign has been reversed.
    //
    // Scope, stated exactly: this inspects the verified spatial PROGRAM. That the chain then reaches
    // each backend without a new print arm is the conjunction of test (b) above (no printer grew an
    // arm) and `spatial_tensix_p4_lock` (the printers really do lower it) — this test asserts
    // neither of those on its own.
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(SUGAR_RMSNORM, &wh)
        .unwrap_or_else(|e| panic!("the rmsnorm sugar must verify on wormhole — `spatial_front_end_lock` asserts it does. Got {e:?}"));
    assert_eq!(
        p.phase(),
        Phase::Verified,
        "the rmsnorm pipeline must reach Verified"
    );

    let ops: Vec<String> = p
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(op) => Some(variant_name(op)),
            _ => None,
        })
        .collect();

    // ★ WHAT EACH STAGE COMPUTES, not what it is called. Every stage of the legalized chain is now a
    // `Generic`, so the variant names stopped discriminating — which is exactly the success condition
    // (the printer no longer knows a workload's name), and it means this reference has to move down a
    // level to keep its teeth. A reduction is named by its FOLDED AXIS, a transform by its SFPU kind,
    // an arithmetic stage by its operator.
    let bodies: Vec<String> = p
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::Generic { iters, body, .. }) => Some(
                if iters
                    .get(1)
                    .map(|a| matches!(a.kind, IterKind::Reduce { .. }))
                    .unwrap_or(false)
                {
                    "Reduce".to_string()
                } else {
                    // ★ THE NAME MUST SURVIVE FUSION. Once `fuse_generic` nests a transform inside
                    // arithmetic, naming a stage by its OUTERMOST node alone stops discriminating —
                    // `Mul(Rsqrt(v), x)` and a plain multiply would both read "Mul". The transforms
                    // inside are appended so the chain reference keeps saying what rmsnorm IS.
                    fn kinds_in(b: &TensorBody, acc: &mut Vec<String>) {
                        match b {
                            TensorBody::Unary { kind, arg } => {
                                acc.push(
                                    format!("{kind:?}")
                                        .split_whitespace()
                                        .next()
                                        .unwrap_or("?")
                                        .to_string(),
                                );
                                kinds_in(arg, acc);
                            }
                            TensorBody::Add(x, y)
                            | TensorBody::Sub(x, y)
                            | TensorBody::Mul(x, y)
                            | TensorBody::Min(x, y)
                            | TensorBody::Max(x, y)
                            | TensorBody::FloorDiv(x, y) => {
                                kinds_in(x, acc);
                                kinds_in(y, acc);
                            }
                            TensorBody::Shr { arg, .. } => kinds_in(arg, acc),
                            _ => {}
                        }
                    }
                    let head = match body {
                        TensorBody::Add(..) => "Add",
                        TensorBody::Sub(..) => "Sub",
                        TensorBody::Mul(..) => "Mul",
                        TensorBody::Min(..) => "Min",
                        TensorBody::Max(..) => "Max",
                        TensorBody::FloorDiv(..) => "FloorDiv",
                        TensorBody::Shr { .. } => "Shr",
                        TensorBody::Unary { .. } => "",
                        TensorBody::Operand(_) => "Copy",
                        TensorBody::Const(_) => "Const",
                        TensorBody::Select { .. } => "Select",
                    };
                    let mut ks = Vec::new();
                    kinds_in(body, &mut ks);
                    match (head.is_empty(), ks.is_empty()) {
                        (true, _) => ks.join("+"),
                        (false, true) => head.to_string(),
                        (false, false) => format!("{head}[{}]", ks.join("+")),
                    }
                },
            ),
            StageKind::Compute(op) => Some(variant_name(op)),
            _ => None,
        })
        .collect();

    assert!(
        !ops.is_empty(),
        "the rmsnorm program carries no compute stages at all, so every check below is vacuous — \
         something upstream stopped building the chain. Fix `from_fused_row` in \
         `spatial/passes/from_tensor_op.rs`, never this assertion."
    );

    // Every op a real program uses must be one the taxonomy classifies — the table is checked
    // against the enum's TEXT in the test above and against a running program here.
    for op in &ops {
        assert!(
            class_of(op).is_some(),
            "the rmsnorm program uses `ComputeOp::{op}`, which TAXONOMY does not classify. Add its \
             row in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs — and read the \
             unclassified-variant message there first, because the answer is usually a composition."
        );
    }

    assert!(
        !ops.iter().any(|o| o == "RowReduce"),
        "\n★ rmsnorm reverted to the FUSED `ComputeOp::RowReduce` variant: {ops:?}\n\n\
         `RowReduce` is a SPECIFICATION OP — it exists so `spatial/eval.rs` can state the reference \
         semantics the chain is diffed against, and every printer refuses it by name. Lowering it \
         directly means each backend must learn the word 'rmsnorm', which is one print arm per \
         backend for one workload. Restore the legalization in `from_fused_row` \
         (`spatial/passes/from_tensor_op.rs`); never fix this by teaching a printer the fused op.\n"
    );

    for want in RMSNORM_CHAIN {
        assert!(
            bodies.iter().any(|o| o == want),
            "\nrmsnorm's legalized chain no longer contains `ComputeOp::{want}` — got {ops:?}.\n\n\
             The chain is square (Elementwise Mul) → reduce_rows (ReduceRows) → rsqrt (RowXform) → \
             rescale (RowRescale) → weight (RowBroadcast Mul), built by `from_fused_row` in \
             `spatial/passes/from_tensor_op.rs`. If the composition legitimately changed shape, \
             update RMSNORM_CHAIN to the new composition — but if it changed because a fused \
             variant replaced it, the fix is in the pass, not here.\n"
        );
    }

    // ★ AND NOTHING ELSE. Presence alone is a one-way check: a fused `ComputeOp::RmsNorm` bolted on
    // BESIDE the chain would satisfy every assertion above while re-introducing exactly the variant
    // this file exists to refuse. The compute vocabulary of this program is the chain, exactly.
    let extra: BTreeSet<&str> = bodies
        .iter()
        .map(|s| s.as_str())
        .filter(|o| !RMSNORM_CHAIN.contains(o))
        .collect();
    assert!(
        extra.is_empty(),
        "\n★ rmsnorm's program grew a compute op that is not part of its legalized chain: {:?}\n\
         (all compute stages: {ops:?})\n\n\
         The chain in RMSNORM_CHAIN is the whole compute vocabulary of this workload. A new op here \
         means a stage was added to `from_fused_row` — legitimate, if it is one of the ops that \
         already exist and the chain genuinely changed shape, in which case add it to RMSNORM_CHAIN \
         with the reason. If it is instead a fused workload-shaped op, the fix is in the pass: \
         nothing about rmsnorm may reach a printer as a single op named after the workload.\n",
        extra
    );

    // ── And the decisive property: nothing in the chain is a workload-shaped op. Every stage is
    //    either the growth point (a parameterized general form) or a driver for a physical unit.
    for op in &ops {
        let class = class_of(op).expect("classified above");
        assert_ne!(
            class,
            Class::SpecificationOp,
            "the rmsnorm chain still contains the SPECIFICATION OP `ComputeOp::{op}` — it was not \
             fully legalized, so the printers must still know a workload by name. Finish the \
             composition in `from_fused_row` (`spatial/passes/from_tensor_op.rs`); the fix is never \
             to re-classify the op here to make this pass."
        );
    }

    println!("rmsnorm legalizes to: {}", ops.join(" → "));
}

// ────────────────────────────────────────────────────────────────────────────────────────────────
// (d) THE PRIMITIVE TAXONOMY — B-P3: what the closed compute algebra is MADE OF is closed too
// ────────────────────────────────────────────────────────────────────────────────────────────────
//
// ★ THE COMPUTE TAXONOMY ABOVE CLOSES `ComputeOp`. It says nothing about the PARAMETERS the general
// form is built from — the iterator kinds, the pooling functions, the SFPU repertoire, the body
// grammar, the tensor roles and memory spaces. Those are where a workload's novelty actually lands
// once `ComputeOp` is closed, and they are where the Qwen3.8 decode step (B-P3..B-P8) had to grow the
// algebra: a signed flat-affine access map (transpose, head slicing, rope pair reads), a reduction
// over EITHER axis, an index-valued fold (`argmax`), a piecewise body (`Select`, the branch-free mask),
// the natural log (`decay = exp(R·ln(1+e^u))`), a value-dependent access (the gather), and the roles a
// persistent decoder needs (activations between sections, state across steps, an SRAM tier).
//
// ★ THE TEETH ARE THE SAME THREE, ONE LEVEL DOWN. Every variant of every primitive enum in `ir.rs` is
// read out of the source text and must be classified here with a sentence saying why it is a
// PRIMITIVE — why no composition of the others expresses it (`the_primitive_taxonomy_is_closed`);
// every compute-side primitive must be EVALUATED by the oracle in `eval.rs`, in code, not prose
// (`every_primitive_is_evaluated`); and the whole expansion must have cost `ComputeOp` NOTHING
// (`the_compute_taxonomy_did_not_grow_for_qwen`) — the printers still know six shapes, exactly as
// `FROZEN_LOWER_ARMS` freezes them.
//
// ★ MUTATION PROBE: comment out the `("SfpuKind", "Ln", ..)` row below → `the_primitive_taxonomy_is_closed`
//   goes red naming `SfpuKind::Ln` as an unclassified variant (run 2026-09-18, red as expected, restored).
// ★ MUTATION PROBE: delete the `SfpuKind::Ln` arm from `apply` in `spatial/eval.rs` →
//   `every_primitive_is_evaluated` goes red naming `SfpuKind::Ln` (run 2026-09-18, red, restored).
// ★ MUTATION PROBE: add a seventh row `("Gelu", Class::HardwareIntrinsic, ..)` to `TAXONOMY` — the row anyone
//   growing `ComputeOp` must add to get past test (a) — → `the_compute_taxonomy_did_not_grow_for_qwen` goes red at
//   `TAXONOMY.len()` (run 2026-09-18, red, restored). (A bare new enum variant cannot even compile: every printer's
//   exhaustive match refuses it first, which is `FROZEN_LOWER_ARMS` doing its job one level earlier.)

/// (enum, variant, why it is a primitive — why no composition of the other primitives expresses it).
/// The third column is checked for length, so a row cannot be a label wearing a justification.
const PRIMITIVE_TAXONOMY: &[(&str, &str, &str)] = &[
    // ── IterKind: what an axis of the iteration space DOES ──
    (
        "IterKind",
        "Parallel",
        "an axis that survives into the output — the ordinary map. Every other kind is defined by \
         contrast with it, so it cannot be built out of them.",
    ),
    (
        "IterKind",
        "Reduce",
        "an axis folded away by a pool. A fold is not a body: no expression at one point of the \
         iteration space can see the other points, so folding is a property of the ITERATOR and of \
         nothing else. B-P3 made it axis-general — the row axis folds as readily as the column axis.",
    ),
    // ── ReducePool: the fold ──
    (
        "ReducePool",
        "Sum",
        "the additive fold — Σ over the reduced axis. `Max` cannot be composed into a sum and a sum \
         cannot be composed out of anything pointwise, so the two are distinct primitives.",
    ),
    (
        "ReducePool",
        "Max",
        "the lattice fold — the row max softmax subtracts before it exponentiates. Not a sum of \
         anything; a separate pooling unit on every reduce engine.",
    ),
    (
        "ReducePool",
        "ArgMax",
        "the INDEX-valued fold: the first position at which the maximum occurs (the VM's `>` scan, \
         ties to the lowest index). A `Max` fold forgets where it found its value, and no pointwise \
         body can recover a position from a value, so `next = argmax(logits)` and MoE top-k need it.",
    ),
    // ── SfpuKind: the transcendental repertoire ──
    (
        "SfpuKind",
        "Exp",
        "e^x — the transcendental every softmax and every gate rests on; no finite composition of \
         add/mul/const reaches it exactly, and the SFPU has one instruction for it.",
    ),
    (
        "SfpuKind",
        "Recip",
        "1/x — the divide, kept as its own transform so a normalization divides ONCE where the \
         specification says, rather than smearing a scaler across every term of a reduction.",
    ),
    (
        "SfpuKind",
        "Rsqrt",
        "1/sqrt(x/over + eps) — the divide-once contract of rmsnorm and layernorm lives here (see \
         `RowXformKind::Rsqrt` for what folding it into the reduction's scaler cost in bf16).",
    ),
    (
        "SfpuKind",
        "Silu",
        "x·sigmoid(x) — one fused SFPU instruction on Tensix (`silu_tile`); composing it from Exp \
         and Recip is a different rounding sequence than the fused instruction the printer emits.",
    ),
    (
        "SfpuKind",
        "Relu",
        "max(x, 0) — the clamp, an SFPU instruction of its own and cheaper than any Select that \
         would spell it.",
    ),
    (
        "SfpuKind",
        "Ln",
        "the natural log — the inverse of Exp, which no composition of Exp/Recip/Rsqrt reaches. \
         The GDN decay `(1+e^u)^R` is `exp(R·ln(1+e^u))`: a real power needs a real log.",
    ),
    (
        "SfpuKind",
        "Sign",
        "1 above zero, −1 below, 0 at zero — a comparison of a VALUE (Select compares a position): \
         `sign_tile` on Tensix's SFPU, a compare and a mux on a PCU. `x/|x|` divides by zero at zero \
         and a clamp of `x·K` is a sign only above `1/K`; the signed root `sign(g)·√max(|g|, ε)` of \
         Qwen3.8's n-gram gate needs it exact below its floor.",
    ),
    // ── TensorBody: the scalar grammar at one point of the iteration space ──
    (
        "TensorBody",
        "Operand",
        "a read of operand i through its own access map — the leaf every body is built from.",
    ),
    (
        "TensorBody",
        "Const",
        "a literal, as f64 bits so the body stays `Hash + Eq` — a mean's `1/cols`, a mask's zero.",
    ),
    (
        "TensorBody",
        "Add",
        "the additive node of the field; with Mul it is the ring every polynomial body is written in.",
    ),
    (
        "TensorBody",
        "Sub",
        "subtraction kept distinct from `Add(x, Mul(y, -1))` because the printers have `sub_tiles` \
         and the fused row chain (`x - rowmax`) must lower to it, not to a multiply and an add.",
    ),
    (
        "TensorBody",
        "Mul",
        "the multiplicative node — the rescale, the weight, the gate; with Add it closes the ring.",
    ),
    (
        "TensorBody",
        "Min",
        "the smaller of two values. Add, Sub and Mul make a ring, and no polynomial over a ring decides an order; \
         Select decides by POSITION, never by value; and Unary's kinds are float transcendentals, inexact on an \
         int8 model's integers. So the order on values needs its own nodes: a clamp is one Min and one Max.",
    ),
    (
        "TensorBody",
        "Max",
        "the larger of two values — Min's dual, for the same reason a primitive: a ReLU is `Max(x, 0)`, and the lattice \
         fold ReducePool::Max is this node folded along an axis.",
    ),
    (
        "TensorBody",
        "FloorDiv",
        "integer division rounding toward −∞, the VM's `//`. No ring polynomial divides, and Unary's reciprocal is a \
         float that cannot be exact on i64; the LUT softmax's normalization `Σ w·v ÷ Σ w` must be exact to match the \
         model's golden tokens.",
    ),
    (
        "TensorBody",
        "Shr",
        "the arithmetic shift `⌊a / 2^k⌋` — kept distinct from `FloorDiv(a, Const(2^k))` for the reason Sub is kept \
         distinct from `Add(x, Mul(y, -1))`: every integer datapath prints it as wires, where a FloorDiv by a value is a \
         divider unit (a stage per quotient bit on the fabric, kept to one lane), and an int8 requant's power-of-two \
         scale must lower to the wires.",
    ),
    (
        "TensorBody",
        "Unary",
        "the transcendental slot — one `SfpuKind` applied to a sub-body. The SFPU repertoire \
         grows HERE, as a new kind, never as a new node.",
    ),
    (
        "TensorBody",
        "Select",
        "the piecewise node: `cond ? then : else` on an AXIS predicate, evaluated at both branches \
         (branch-free, so an unselected branch may read outside the page and yield 0.0). A rope \
         window, a conv ring shift and a `j < pos` mask are all one of these; no arithmetic on \
         values can decide by POSITION, which is why it is a primitive.",
    ),
    // ── Pred / PredRhs: what a Select decides on ──
    (
        "Pred",
        "AxisLt",
        "`axis < bound` — the half-open window's lower side; the position mask `j < pos` is exactly it.",
    ),
    (
        "Pred",
        "AxisGe",
        "`axis >= bound` — the complement, kept as its own variant so a rope window's two halves \
         are each one predicate rather than a negation the printers would have to spell.",
    ),
    (
        "PredRhs",
        "Const",
        "a bound fixed by the program — the rotary half-width, the conv tap count.",
    ),
    (
        "PredRhs",
        "Operand",
        "a bound READ FROM AN OPERAND at the current point — data-dependent but branch-free, which \
         is how `j < pos` follows the token position without recompiling the program.",
    ),
    // ── TensorRole / TensorSpace: what a tensor IS to the program ──
    (
        "TensorRole",
        "Input",
        "supplied to the program and only ever loaded — the activations that arrive.",
    ),
    (
        "TensorRole",
        "Output",
        "produced by the program and only ever stored — what leaves.",
    ),
    (
        "TensorRole",
        "Param",
        "a weight: loaded, never stored, constant across every step and every item.",
    ),
    (
        "TensorRole",
        "Activation",
        "produced by one section of a program and consumed by a later one — stored AND loaded \
         inside a single program, which `Input` and `Output` each forbid. The persistent-decoder \
         form (B-P7) is made of these; `validate_tensors` refuses them by name until it lands.",
    ),
    (
        "TensorRole",
        "State",
        "carried ACROSS steps — the GDN memory, the conv ring, the KV cache — read at the start of a \
         step and written once at its end. Neither an input nor an output of any one step; the \
         cross-token recurrence (B-P6) gives it its semantics and `validate_tensors` refuses it until then.",
    ),
    (
        "TensorSpace",
        "Dram",
        "the off-chip default: every tensor of today's single-op programs.",
    ),
    (
        "TensorSpace",
        "Hbm",
        "the high-bandwidth off-chip tier a fit report places weights in.",
    ),
    (
        "TensorSpace",
        "Sram",
        "the on-chip tier a state tensor must live in to persist between steps without a round trip \
         — the SN50's 432 MB is what makes GDN state resident. Named now, refused by `validate_tensors` \
         until B-P6 gives state its semantics.",
    ),
];

/// The two value-dependent access structs B-P3 adds as TYPES (their evaluation is B-P5): a struct is
/// closed by its FIELD LIST, exactly as an enum is closed by its variant list.
const PRIMITIVE_STRUCTS: &[(&str, &[&str])] = &[
    ("IndirectTerm", &["operand", "stride", "modulus"]),
    (
        "IndirectTile",
        &["index", "tile_stride", "elems_per_tile", "bound"],
    ),
];

/// How many variants each primitive enum holds, frozen with the table above. Every enum the table
/// names has an entry here (the test asserts it), so a variant cannot hide in an enum this list
/// forgot to count.
const FROZEN_PRIMITIVE_SIZES: &[(&str, usize)] = &[
    ("IterKind", 2),
    ("ReducePool", 3),
    ("SfpuKind", 7),
    ("TensorBody", 11),
    ("Pred", 2),
    ("PredRhs", 2),
    ("TensorRole", 5),
    ("TensorSpace", 3),
];

/// The variant names `pub enum <name> {` declares in `ir.rs`, read out of the source text — the
/// generalization of [`compute_op_variants`] to any enum. The `{` in the needle is what keeps
/// `Pred` from matching `PredRhs`.
fn enum_variants(ir_src: &str, name: &str) -> Vec<String> {
    let needle = format!("pub enum {name} {{");
    let head = ir_src.find(&needle).unwrap_or_else(|| {
        panic!(
            "`{needle}` must be findable in crates/logicaffeine_compile/src/spatial/ir.rs — if the enum was \
             renamed or moved, retarget PRIMITIVE_TAXONOMY / this parser. Never delete this lock to get past a rename."
        )
    });
    let open = head + needle.len();
    let mut names = Vec::new();
    let mut depth = 0usize;
    for line in ir_src[open..].lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("#[") {
            continue;
        }
        if depth == 0 && t.starts_with(|c: char| c.is_ascii_uppercase()) {
            names.push(
                t.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>(),
            );
        }
        for c in t.chars() {
            match c {
                '{' | '(' => depth += 1,
                '}' | ')' => {
                    if depth == 0 {
                        return names;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    names
}

/// The `pub` field names `pub struct <name> {` declares in `ir.rs`, read out of the source text.
fn struct_fields(ir_src: &str, name: &str) -> Vec<String> {
    let needle = format!("pub struct {name} {{");
    let head = ir_src.find(&needle).unwrap_or_else(|| {
        panic!("`{needle}` must be findable in crates/logicaffeine_compile/src/spatial/ir.rs — B-P3 adds it; if it moved, retarget PRIMITIVE_STRUCTS.")
    });
    let open = head + needle.len();
    let mut fields = Vec::new();
    let mut depth = 0usize;
    for line in ir_src[open..].lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("#[") {
            continue;
        }
        if depth == 0 {
            if let Some(rest) = t.strip_prefix("pub ") {
                fields.push(
                    rest.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect::<String>(),
                );
            }
        }
        for c in t.chars() {
            match c {
                '{' | '(' | '<' => depth += 1,
                '}' | ')' | '>' => {
                    if depth == 0 {
                        return fields;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    fields
}

fn ir_source() -> String {
    let ir = spatial_src().join("ir.rs");
    std::fs::read_to_string(&ir).unwrap_or_else(|e| panic!("{}: {e}\n\nthis lock reads the compiler's own source; if the path moved, retarget `spatial_src()`.", ir.display()))
}

#[test]
fn the_primitive_taxonomy_is_closed() {
    let text = ir_source();

    // ── The generalized walker must agree with the original on the enum the original was written for,
    //    or every count below is measuring a different parser than test (a) trusts.
    assert_eq!(
        enum_variants(&text, "ComputeOp"),
        compute_op_variants(&text),
        "`enum_variants` and `compute_op_variants` disagree on `ComputeOp` — the generalized walker mis-reads the \
         enum shape. Fix `enum_variants` in crates/logicaffeine_tests/tests/spatial_algebra_lock.rs."
    );

    let enums: BTreeSet<&str> = PRIMITIVE_TAXONOMY.iter().map(|(e, _, _)| *e).collect();
    let frozen: BTreeSet<&str> = FROZEN_PRIMITIVE_SIZES.iter().map(|(e, _)| *e).collect();
    assert_eq!(
        enums, frozen,
        "PRIMITIVE_TAXONOMY names the enums {enums:?} but FROZEN_PRIMITIVE_SIZES freezes {frozen:?} — every \
         classified enum must be counted and every counted enum classified, or a variant can hide in the gap."
    );

    for e in &enums {
        let live: Vec<String> = enum_variants(&text, e);
        assert!(
            live.len() >= 2,
            "the enum parser found only {} variant(s) in `pub enum {e}` — it has stopped understanding the enum's \
             shape. Fix `enum_variants` in tests/spatial_algebra_lock.rs; never soften this floor.",
            live.len()
        );
        let mut uniq = live.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), live.len(), "the enum parser reported a duplicate variant name in `{e}` ({live:?}) — it is counting field lines as variants.");

        let table: BTreeSet<&str> = PRIMITIVE_TAXONOMY
            .iter()
            .filter(|(en, _, _)| en == e)
            .map(|(_, v, _)| *v)
            .collect();
        let live_set: BTreeSet<&str> = live.iter().map(|s| s.as_str()).collect();
        let unclassified: Vec<&str> = live_set.difference(&table).copied().collect();
        assert!(
            unclassified.is_empty(),
            "\n★ A NEW PRIMITIVE WAS ADDED TO `{e}` WITHOUT A CLASSIFICATION: {}\n\n\
             Before this goes green, write the sentence that makes it a PRIMITIVE in PRIMITIVE_TAXONOMY \
             (crates/logicaffeine_tests/tests/spatial_algebra_lock.rs): why no composition of the other \
             {e} variants (or of the bodies, maps and iterators that already exist) expresses it. If a \
             composition does express it, delete the variant and write the composition. The fix is NEVER \
             to delete this assertion.\n",
            unclassified.join(", ")
        );
        let stale: Vec<&str> = table.difference(&live_set).copied().collect();
        assert!(
            stale.is_empty(),
            "\nPRIMITIVE_TAXONOMY classifies `{e}` variant(s) the enum no longer declares: {}\n\nDelete the rows (a \
             legalized-away primitive is progress) and lower the entry in FROZEN_PRIMITIVE_SIZES.\n",
            stale.join(", ")
        );
        let (_, n) = FROZEN_PRIMITIVE_SIZES
            .iter()
            .find(|(en, _)| en == e)
            .expect("frozen above");
        assert_eq!(
            live.len(),
            *n,
            "`{e}` now holds {} variant(s), frozen at {n}. Raising the number in FROZEN_PRIMITIVE_SIZES needs a \
             PRIMITIVE_TAXONOMY row whose justification says why the new variant is not a composition; lowering \
             it needs no ceremony.",
            live.len()
        );
    }

    for (e, v, why) in PRIMITIVE_TAXONOMY {
        assert!(
            why.len() >= 40,
            "the PRIMITIVE_TAXONOMY row for `{e}::{v}` carries no real justification — write the sentence that \
             makes it a primitive (why no composition expresses it). Do not shorten this floor to get past it."
        );
    }

    // ── The value-dependent access structs, closed by their field lists.
    for (name, fields) in PRIMITIVE_STRUCTS {
        let live = struct_fields(&text, name);
        assert_eq!(
            live,
            fields.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            "`pub struct {name}` in ir.rs declares the fields {live:?}; PRIMITIVE_STRUCTS freezes {fields:?}. A \
             gather's shape is part of the closed algebra: add the field to the table with the reason, or remove it."
        );
    }

    println!(
        "primitive algebra: {} variants across {} enums, {} value-dependent structs",
        PRIMITIVE_TAXONOMY.len(),
        enums.len(),
        PRIMITIVE_STRUCTS.len()
    );
}

/// The enums whose every variant the ORACLE must evaluate: a body node, an iterator kind, a pool, a
/// transform or a predicate the evaluator cannot name is one no composition can be proven against.
/// Roles and spaces are not evaluated by compute; their gate is `validate_tensors`, checked below.
const EVALUATED_ENUMS: &[&str] = &[
    "IterKind",
    "ReducePool",
    "SfpuKind",
    "TensorBody",
    "Pred",
    "PredRhs",
];

#[test]
fn every_primitive_is_evaluated() {
    let text = ir_source();
    let eval = spatial_src().join("eval.rs");
    let eval_code = read_code(&eval);
    assert!(
        eval_code.len() > 1000,
        "no code was read from {} — every check below would be vacuous.",
        eval.display()
    );

    for e in EVALUATED_ENUMS {
        assert!(PRIMITIVE_TAXONOMY.iter().any(|(en, _, _)| en == e), "EVALUATED_ENUMS names `{e}`, which PRIMITIVE_TAXONOMY does not classify — the two lists drifted.");
        for v in enum_variants(&text, e) {
            let spelled = format!("{e}::{v}");
            assert!(
                eval_code.contains(&spelled),
                "`{spelled}` is named nowhere in the CODE of {} — the oracle cannot say what it MEANS. Add the arm \
                 to the `Generic` evaluation in `Eval::compute` (prose is stripped first: a doc comment never stands \
                 in for the arm). The fix is never to exempt a primitive here.",
                eval.display()
            );
        }
    }

    // ★ THE FOUR B-P3 NAMES, SPELLED OUT — so this test says exactly what the expansion delivered.
    for needle in [
        "ReducePool::ArgMax",
        "TensorBody::Select",
        "SfpuKind::Ln",
        ".indirect",
    ] {
        assert!(
            eval_code.contains(needle),
            "`{needle}` is not in the code of {} — the B-P3 primitive is declared but the oracle ignores it. An \
             ignored field is worse than a missing one: `.indirect` must be REFUSED by name by the evaluator until \
             B-P5 gives gathers their semantics, never silently dropped.",
            eval.display()
        );
    }

    // ── The deferred roles and space are refused BY NAME, naming the phase that will admit them.
    let ir_code = code_only(&text);
    for (needle, phase) in [
        ("TensorRole::Activation", "B-P7"),
        ("TensorRole::State", "B-P6"),
        ("TensorSpace::Sram", "B-P6"),
    ] {
        assert!(
            ir_code.contains(needle),
            "`{needle}` is declared but named nowhere in the CODE of ir.rs — `validate_tensors` must refuse it by \
             name (typed, naming {phase}) until that phase lands. A role the validator does not mention is a role \
             that silently passes."
        );
        assert!(
            ir_code.contains(&format!("\"{phase}\"")),
            "ir.rs must name the phase `{phase}` in code (the typed refusal's `until` field) so a deferred role's \
             error says when it stops being deferred."
        );
    }
}

/// ★ THE EXPANSION COST `ComputeOp` NOTHING. Seven kinds of new capability (signed flat-affine maps,
/// axis-general reduction, argmax, select, ln, indirect access, activation/state roles) entered the
/// algebra as PARAMETERS of the general form. The printers still know six shapes.
#[test]
fn the_compute_taxonomy_did_not_grow_for_qwen() {
    let text = ir_source();
    assert_eq!(
        TAXONOMY.len(),
        6,
        "TAXONOMY holds {} rows. It held 6 (MatmulTiles, Generic, Requant, Bias, Relu, Scalar) when the Qwen3.8 \
         expansion began, and the expansion was DESIGNED to add none: every new capability is a parameter of \
         `Generic`. A seventh row means a workload was printed instead of legalized — read the (a) message.",
        TAXONOMY.len()
    );
    let live = enum_variants(&text, "ComputeOp");
    assert_eq!(
        live.len(),
        6,
        "`pub enum ComputeOp` declares {} variants ({live:?}); it declared 6 before the expansion and must still. \
         The B-P3 primitives (`ReducePool::ArgMax`, `TensorBody::Select`, `SfpuKind::Ln`, the indirect access \
         terms, the new roles) live INSIDE `Generic` and the tensor declarations — never beside them.",
        live.len()
    );
}
