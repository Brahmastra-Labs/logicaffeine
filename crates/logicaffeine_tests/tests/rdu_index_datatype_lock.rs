//! An RDU computes indices in INT32 — the widest integer its PCU has — and nothing lowered onto one may use a
//! wider integer.
//!
//! The SN40L's PCU "supports BF16, FP32, INT32 and INT8 compute data types" (arXiv:2405.07518); the SN50's public
//! page names only BF16 and FP8, so the SN50 inherits the SN40L's integer and says so (`Modeled`). The promises:
//!
//!   1. THE FACT IS CITED AND DERIVED. Both RDUs carry `index_datatype = Int32` — published for the SN40L,
//!      inherited for the SN50 — and the descriptor's elementwise datapath and integer accumulator are derived from
//!      it, while the matrix dial stays exactly the witnessed `{Int8, Bf16}`.
//!   2. EVERY INDEX ON THE PROGRAM PATH IS INT32. A gathered row, a written row, an arg-max and an attention's block
//!      start and limits all lower for the SN50 with every integer tensor and buffer at `Int32`, and evaluate to
//!      exactly what the VM computes.
//!   3. AN INDEX SPACE PAST INT32 IS REFUSED, NEVER WRAPPED. A declared index the chip's integer cannot hold is
//!      refused by name as the precision it would need (`MissingPrecision { needed: Int64 }`).
//!   4. THE GATE HAS TEETH. A program whose `Generic` stage holds an `Int64` buffer is refused before any pass runs.
//!
//! ★ MUTATION PROBE (run 2026-09-24 — break, observe red, restore byte-identical): `generic_holds` answering `true`
//! for every precision → `a_generic_stage_holding_an_int64_buffer_is_refused_before_any_pass` reds with `left: Ok(())`
//! — the Int64 buffer is accepted onto the chip.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_index_datatype_lock`
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{ComputeOp, Phase, SpatialProgram, StageKind};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{check_datatypes, compile_program_to_spatial};
use logicaffeine_compile::spatial::rdu_chip::{Provenance, RduChipFacts};
use logicaffeine_compile::spatial::target::{precision_is_int, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn composed(src: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, &sn50())
        .unwrap_or_else(|e| panic!("walk refused: {e:?}\n{src}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("not lowered whole: {}\n{src}", pl.refusal()))
        .clone();
    assert_eq!(
        sp.phase(),
        Phase::Verified,
        "the composed program certifies:\n{src}"
    );
    sp
}

/// Every integer precision a program's tensors and buffers carry.
fn integers(sp: &SpatialProgram) -> Vec<(String, Precision)> {
    let t = sp
        .tensors
        .iter()
        .filter(|t| precision_is_int(t.dtype))
        .map(|t| (format!("tensor {}", t.name), t.dtype));
    let b = sp
        .buffers
        .iter()
        .filter(|b| precision_is_int(b.dtype))
        .map(|b| (format!("buffer {}", b.name), b.dtype));
    t.chain(b).collect()
}

/// The program lowers for the SN50 with integers, every one of them `Int32`, and evaluates to the VM's answer.
fn int32_and_exact(src: &str, what: &str) {
    let sp = composed(src);
    let ints = integers(&sp);
    assert!(!ints.is_empty(), "{what}: the program carries an index");
    for (name, p) in &ints {
        assert_eq!(
            *p,
            Precision::Int32,
            "{what}: {name} is {p:?} — the SN50 computes integers in INT32 and nothing wider"
        );
    }
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills"))
        .unwrap_or_else(|e| panic!("{what}: eval: {e:?}"));
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: VM {:?}", vm.error);
    assert_eq!(
        shown_from_outputs(src, &sp, &out).expect("shown"),
        oracle_ints(&vm.output).expect("vm"),
        "{what}"
    );
}

// ── 1. the fact is cited and derived ─────────────────────────────────────────────────────────────────

#[test]
fn both_rdus_carry_int32_as_their_index_datatype_with_its_provenance() {
    let sn40l = RduChipFacts::sn40l().index_datatype;
    assert_eq!(
        (sn40l.value, sn40l.provenance),
        (Precision::Int32, Provenance::Published)
    );
    assert!(
        sn40l.cite.contains("arXiv:2405.07518") && sn40l.cite.contains("INT32"),
        "the SN40L's integer is cited: {}",
        sn40l.cite
    );
    let sn50 = RduChipFacts::sn50().index_datatype;
    assert_eq!(
        (sn50.value, sn50.provenance),
        (Precision::Int32, Provenance::Modeled)
    );
    assert!(
        sn50.cite.contains("inherited") && sn50.cite.contains("SN40L"),
        "the SN50 says where its integer comes from: {}",
        sn50.cite
    );
}

#[test]
fn the_descriptor_derives_its_integers_from_the_fact_and_keeps_the_matrix_dial_witnessed() {
    for t in [SpatialTarget::sn40l_rdu(), SpatialTarget::sn50_rdu()] {
        assert_eq!(
            t.elementwise_precisions(),
            [Precision::Int8, Precision::Int32, Precision::Bf16]
                .into_iter()
                .collect(),
            "{}: the tile datapath",
            t.name
        );
        assert!(
            t.matmul_supports(Precision::Int8) && t.matmul_supports(Precision::Bf16),
            "{}",
            t.name
        );
        assert!(
            !t.matmul_supports(Precision::Int32) && !t.matmul_supports(Precision::Int64),
            "{}: the matrix dial stays the witnessed set",
            t.name
        );
        assert_eq!(
            t.matmul_acc_int_bits(),
            32,
            "{}: an INT8 product accumulates in the chip's INT32",
            t.name
        );
        assert_eq!(
            t.index_precision_for(248_319),
            Ok(Precision::Int32),
            "{}: a Qwen3.8 token id",
            t.name
        );
        assert_eq!(
            t.index_precision_for(262_144 * 1024),
            Ok(Precision::Int32),
            "{}: a full-context cache offset",
            t.name
        );
        assert_eq!(
            t.index_precision_for(i32::MAX as u64),
            Ok(Precision::Int32),
            "{}: the last index INT32 holds",
            t.name
        );
        assert_eq!(
            t.index_precision_for(i32::MAX as u64 + 1),
            Err(Precision::Int64),
            "{}: one past it needs a precision the chip does not have",
            t.name
        );
    }
}

// ── 2. every index on the program path is int32 ──────────────────────────────────────────────────────

fn table(name: &str, rows: usize, cols: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (i, p) in [0, cols + 1, rows * cols - 1].into_iter().enumerate() {
        s += &format!(
            "Set item {} of {name} to {}.\n",
            p + 1,
            1_000_000_007 + i as i64 * 11
        );
    }
    s
}

#[test]
fn a_gathered_row_is_int32_and_exact_at_every_width() {
    // 3 columns is the element gather; 64 is the tile gather.
    for cols in [3usize, 64] {
        let src = format!("## Main\n{}Let tok be 2.\nLet x be row tok of T.\nShow item 1 of x.\nShow item 2 of x.\n", table("T", 4, cols));
        int32_and_exact(&src, &format!("row gather, {cols} columns"));
    }
}

#[test]
fn a_written_row_is_int32_and_exact_at_every_width() {
    for cols in [3usize, 64] {
        let mut src = format!("## Main\n{}Let src be a 1 by {cols} tensor of Int.\nSet item 1 of src to 99.\nSet item {cols} of src to 77.\n", table("T", 4, cols));
        src += &format!("Let p be 3.\nSet row p of T to src.\nShow item {} of T.\nShow item {} of T.\nShow item {} of T.\n", 2 * cols + 1, 3 * cols, cols + 2);
        int32_and_exact(&src, &format!("row write, {cols} columns"));
    }
}

#[test]
fn an_argmax_is_int32_and_exact() {
    let src = "## Main\nLet X be a 1 by 70 tensor of Real.\nSet item 41 of X to 3.5.\nSet item 69 of X to 3.5.\nLet t be argmax of X.\nShow t.\n";
    int32_and_exact(src, "argmax");
}

#[test]
fn an_attentions_block_start_and_limits_are_int32() {
    let mut src = String::from("## Main\nLet Q be a 1 by 64 tensor of Real.\nLet K be a 40 by 32 tensor of Real.\nLet V be a 40 by 32 tensor of Real.\n");
    src += "Set item 3 of Q to 0.5.\nSet item 70 of K to 0.25.\nSet item 1000 of V to 0.75.\n";
    src += "Let C be attention of Q over K and V with 2 heads 1 groups.\nShow round(item 1 of C * 1000000).\n";
    let sp = composed(&src);
    let ints = integers(&sp);
    // The fold's index arithmetic: each row's limit, the running block start and the block's own limit.
    for carrier in ["row_limit", "block_start", "block_limit"] {
        assert!(
            ints.iter().any(|(n, _)| n.contains(carrier)),
            "the attention's {carrier} rides an integer buffer: {ints:?}"
        );
    }
    for (name, p) in &ints {
        assert_eq!(*p, Precision::Int32, "{name} is {p:?}");
    }
}

// ── 3. an index space past int32 is refused ──────────────────────────────────────────────────────────

#[test]
fn a_declared_index_past_int32_is_refused_as_the_precision_it_would_need() {
    for (what, index) in [
        ("gather", "Let x be row tok of T.\nShow item 1 of x."),
        ("write", "Set row tok of T to src.\nShow item 1 of T."),
    ] {
        let src = format!(
            "## Main\n{}Let src be a 1 by 64 tensor of Int.\nLet tok be 3000000000.\n{index}\n",
            table("T", 4, 64)
        );
        let pl = compile_program_to_spatial(&src, &sn50())
            .unwrap_or_else(|e| panic!("walk refused: {e:?}"));
        assert!(
            pl.whole().is_none(),
            "{what}: an index the chip cannot hold must not lower"
        );
        let why = pl.refusal();
        assert!(
            why.contains("MissingPrecision") && why.contains("Int64"),
            "{what}: refused by name as the precision it needs, got: {why}"
        );
    }
}

// ── 4. the gate has teeth ────────────────────────────────────────────────────────────────────────────

#[test]
fn a_generic_stage_holding_an_int64_buffer_is_refused_before_any_pass() {
    let sp = composed("## Main\nLet X be a 1 by 70 tensor of Real.\nSet item 41 of X to 3.5.\nLet t be argmax of X.\nShow t.\n");
    assert_eq!(
        check_datatypes(&sp, &sn50()),
        Ok(()),
        "the lowered program holds only what the chip computes in"
    );
    let mut widened = sp.clone();
    let victim = widened
        .stages
        .iter()
        .find(|s| {
            matches!(s.kind, StageKind::Compute(ComputeOp::Generic { .. }))
                && s.outs.iter().any(|b| precision_is_int(sp.buffer(*b).dtype))
        })
        .and_then(|s| s.outs.first().copied())
        .expect("an integer Generic output");
    widened.buffers[victim.idx()].dtype = Precision::Int64;
    assert_eq!(
        check_datatypes(&widened, &sn50()),
        Err(SpatialReject::MissingPrecision {
            needed: Precision::Int64,
            target: "sn50-rdu"
        })
    );
}
