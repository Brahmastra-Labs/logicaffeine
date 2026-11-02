//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU OP-COVERAGE LOCK — the RDU column of the 120-op fabric coverage ratchet.
//!
//! `fabric_coverage_lock` pins WHICH ISA ops the spatial fabric realizes (`codegen_sva::fabric_tile`, the
//! single source of truth) and welds every realized op to an `eval == VM` witness. This lock asks the
//! next question for the SN50: which of those realized ops does the SN50 descriptor's per-PCU envelope
//! HOST — its ALU op set, its functional units, its state plane — and proves the same witnesses through
//! that envelope, so the "lower any LOGOS program to the chip" claim of `largo emit rdu` can never outrun
//! the descriptor it is made on.
//!
//!  A. `the_rdu_realized_floor_is_pinned_exactly` — `TOTAL_OPS` (the ISA) and `RDU_REALIZED_FLOOR` (the
//!     fabric-realized ops the SN50 envelope hosts, `fabric_tile::hosted_ops`) are pinned EXACTLY: a lost
//!     capability on the descriptor AND an un-bumped graduation both go RED. Raise-only (rule 14).
//!  B. `every_rdu_realized_op_is_proven_equivalent_to_the_vm` — the SAME registry `fabric_coverage_lock`
//!     runs (`common/fabric_witnesses.rs`) is invoked here, and every source-authored witness is placed
//!     THROUGH the SN50 envelope: the two-argument ALU / comparison / division witnesses are routed by
//!     `place_with_target` onto `sn50_shaped()` and the PLACED GRID is evaluated against the VM; the rich
//!     corpus and the whole-program corpus pass the envelope's cell-capability gate and evaluate ≡ VM.
//!  C. `the_rdu_frontier_is_explicit_and_shrinking` — the ops the envelope does NOT host are listed by
//!     name, each with the reason `fabric_tile::frontier_reason` states, and hosted + frontier PARTITION
//!     the ISA. Shrink-only.
//!  D. `the_sn40l_column_agrees_with_the_sn50_column_except_where_the_facts_differ` — the SN40L envelope
//!     hosts exactly the same ops; the two descriptors differ only where `RduChipFacts` differ.
//!
//! ★ MUTATION PROBES (each run once, 2026-09-18; the lock went red, the impl was restored byte-identical):
//!   - shrink the SN50 ALU op set (drop `AluOp::Mul` from `RduChipFacts::fabric_target`'s `alu_ops`) → A
//!     (`hosted 114 ≠ 115`), B (`Mul: the SN50 envelope must host `fab_mul`: MissingCapability { needed:
//!     Alu(Mul) }` out of `place_with_target`) AND C (an ALU op the fabric realizes but the descriptor no
//!     longer hosts falls into the frontier, so it is no longer `["Serve", "Tcp"]`) all go red;
//!   - map an unrealized op to a no-op tile (`Op::Tcp { .. } => FabricTile::Control` in `fabric_tile`) →
//!     C (`the RDU frontier is pinned by name … a no-op tile for a host-IO op is not a graduation`) and
//!     A (hosted 116 ≠ 115) go red;
//!   - drop `frontier_reason`'s `Serve` arm → C (`Serve: a frontier op must state why the fabric has no
//!     realization of it`) goes red.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_op_coverage_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/fabric_witnesses.rs"]
mod fabric_witnesses;

use std::collections::{BTreeMap, BTreeSet};

use fabric_witnesses::*;
use logicaffeine_compile::codegen_sva::fabric_grid::eval_grid_all_with_heap;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, function_op_names, whole_program_fabric_from_source,
};
use logicaffeine_compile::codegen_sva::fabric_target::{place_with_target, FabricTarget};
use logicaffeine_compile::codegen_sva::fabric_tile::{
    fabric_realized_ops, fabric_tile, frontier_reason, hosted_ops,
};
use logicaffeine_compile::spatial::rdu_chip::{RduChip, RduChipFacts};
use logicaffeine_compile::vm::Op;
use serde_json::Value;

/// The ISA op count — pinned here as the RDU column's own denominator (a new op must be classified for
/// the fabric AND for the descriptor).
const TOTAL_OPS: usize = 120;
/// The fabric-realized ops the SN50 per-PCU envelope hosts. The envelope carries every ALU op, every
/// functional unit and a state plane, so it hosts the whole realized set: 118. RAISE as ops graduate onto
/// a tile the envelope provides; NEVER lower (rule 14).
const RDU_REALIZED_FLOOR: usize = 118;
/// The ops the envelope does not host, by name — host IO the dataflow chip has no realization of.
const RDU_FRONTIER: &[&str] = &["Serve", "Tcp"];

fn sn50() -> FabricTarget {
    FabricTarget::sn50_shaped()
}

// ── A. the floor, pinned exactly ─────────────────────────────────────────────────────────────────────
#[test]
fn the_rdu_realized_floor_is_pinned_exactly() {
    assert_eq!(
        Op::all_variants().len(),
        TOTAL_OPS,
        "the ISA op count changed — classify the new op for the fabric AND the RDU column"
    );
    let hosted = hosted_ops(&sn50());
    assert_eq!(
        hosted.len(),
        RDU_REALIZED_FLOOR,
        "the SN50 envelope hosts {} realized ops (floor {RDU_REALIZED_FLOOR}) — RAISE the floor as ops graduate, NEVER lower it; hosted = {hosted:?}",
        hosted.len()
    );
    // Hosted ⊆ realized: an op the descriptor hosts is one the fabric realizes (never the other way round).
    let realized: BTreeSet<&str> = fabric_realized_ops().into_iter().collect();
    for op in &hosted {
        assert!(
            realized.contains(op),
            "{op} is hosted by the SN50 envelope but not realized by the fabric"
        );
    }
    println!(
        "RDU column: {}/{} ops hosted by the SN50 envelope ({:.1}%)",
        hosted.len(),
        TOTAL_OPS,
        hosted.len() as f64 / TOTAL_OPS as f64 * 100.0
    );
}

// ── B. the same witnesses, through the descriptor ───────────────────────────────────────────────────
#[test]
fn every_rdu_realized_op_is_proven_equivalent_to_the_vm() {
    let target = sn50();
    let hosted: BTreeSet<String> = hosted_ops(&target)
        .into_iter()
        .map(str::to_string)
        .collect();
    let compute: BTreeSet<String> = Op::all_variants()
        .iter()
        .filter(|o| fabric_tile(o).is_some_and(|t| t.is_compute()))
        .map(|o| o.name().to_string())
        .filter(|n| hosted.contains(n))
        .collect();
    let mut proven: BTreeSet<String> = FABRIC_EQUIV.iter().map(|(n, _)| n.to_string()).collect();
    for cf in CORPUS {
        for op in function_op_names(cf.src, cf.fn_name) {
            proven.insert(op.to_string());
        }
    }
    let unproven: Vec<&String> = compute.difference(&proven).collect();
    assert!(unproven.is_empty(), "hosted compute op(s) {unproven:?} have no witness — a coverage claim on the RDU MUST come with its proof");

    // The registry itself: declaration is not proof.
    for (name, run) in FABRIC_EQUIV {
        println!("witness: {name}");
        run();
    }

    // The two-argument witnesses PLACED on the SN50 envelope: every cell's capability is provided, the routed
    // body fits one PCU datapath, and the PLACED GRID computes what the VM computes.
    let registry: BTreeSet<&str> = FABRIC_EQUIV.iter().map(|(n, _)| *n).collect();
    let two_arg: Vec<(&str, String, String)> = ARITH_WITNESSES
        .iter()
        .map(|(op, name, expr)| (*op, name.to_string(), arith_source(name, expr)))
        .chain(
            CMP_WITNESSES
                .iter()
                .map(|(op, name, cmp)| (*op, name.to_string(), cmp_source(name, cmp))),
        )
        .collect();
    for (op, name, src) in &two_arg {
        assert!(
            registry.contains(op),
            "{op}: the placed witness table names an op the registry does not carry"
        );
        let ops = function_op_names(src, name);
        assert!(
            ops.contains(op),
            "{op}: its placed witness `{name}` does not contain the op ({ops:?})"
        );
        let df = decision_fabric_from_source(src, name);
        assert!(!df.escaped_first_return, "{name}: a single-return witness");
        let grid = place_with_target(&df.cells, df.n_in, &[df.result], &target)
            .unwrap_or_else(|e| panic!("{op}: the SN50 envelope must host `{name}`: {e:?}"));
        assert_eq!(
            (grid.rows, grid.cols),
            (target.rows, target.cols),
            "{name}: padded onto the PCU envelope"
        );
        let mut st = 0x5D50_u64 ^ (name.bytes().map(|c| c as u64).sum::<u64>());
        for _ in 0..120 {
            let a = (mix(&mut st) % 20_000) as i64 - 10_000;
            let mut b = (mix(&mut st) % 200) as i64 + 1;
            if mix(&mut st) & 1 == 0 {
                b = -b;
            }
            let got = eval_grid_all_with_heap(&grid, &[a, b], &mut Vec::new())[0];
            let want = vm_call2(src, name, a, b);
            assert_eq!(
                got, want,
                "{op}: the grid placed on the SN50 envelope != VM on ({a}, {b})"
            );
        }
        println!(
            "placed on sn50-rdu: {op} via `{name}` ({} cells)",
            df.cells.len()
        );
    }

    // The rich corpus and the whole-program corpus: every cell's capability is one the envelope provides
    // (these span more than one PCU datapath, so the geometric fit is the mesh's, not one envelope's), and
    // each evaluates ≡ VM.
    for cf in CORPUS {
        let df = decision_fabric_from_source(cf.src, cf.fn_name);
        target.require_cells(&df.cells).unwrap_or_else(|e| {
            panic!(
                "{}: the SN50 envelope must provide every cell's capability: {e:?}",
                cf.fn_name
            )
        });
        println!(
            "corpus on sn50-rdu: {} ({} cells)",
            cf.fn_name,
            df.cells.len()
        );
        run_corpus_fn(cf);
    }
    for src in WHOLE_CORPUS {
        let wpf = whole_program_fabric_from_source(src);
        target.require_cells(&wpf.cells).unwrap_or_else(|e| panic!("whole-program corpus: the SN50 envelope must provide every cell's capability: {e:?}\n{src}"));
        run_whole_fn(src);
    }
}

// ── C. the honest, shrinking frontier ────────────────────────────────────────────────────────────────
#[test]
fn the_rdu_frontier_is_explicit_and_shrinking() {
    let hosted: BTreeSet<&str> = hosted_ops(&sn50()).into_iter().collect();
    let mut frontier: Vec<&str> = Op::all_variants()
        .iter()
        .map(|o| o.name())
        .filter(|n| !hosted.contains(n))
        .collect();
    frontier.sort_unstable();
    assert_eq!(frontier, RDU_FRONTIER, "the RDU frontier is pinned by name — it may only SHRINK, and a no-op tile for a host-IO op is not a graduation");
    assert_eq!(
        hosted.len() + frontier.len(),
        TOTAL_OPS,
        "hosted + frontier must PARTITION the ISA"
    );
    for op in Op::all_variants() {
        let name = op.name();
        let reason = frontier_reason(&op);
        if frontier.contains(&name) {
            let reason = reason.unwrap_or_else(|| {
                panic!("{name}: a frontier op must state why the fabric has no realization of it")
            });
            assert!(
                reason.contains(name),
                "{name}: the reason names the op: {reason}"
            );
            assert!(!reason.trim().is_empty());
        } else {
            assert!(
                reason.is_none(),
                "{name} is hosted, so it carries no frontier reason"
            );
            assert!(fabric_tile(&op).is_some(), "{name}");
        }
    }
    println!("RDU frontier ({}): {frontier:?}", frontier.len());
}

// ── D. the two chips, one envelope ────────────────────────────────────────────────────────────────────
fn leaves(v: &Value, path: &mut Vec<String>, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                path.push(k.clone());
                leaves(c, path, out);
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                path.push(i.to_string());
                leaves(c, path, out);
                path.pop();
            }
        }
        _ => {
            out.insert(path.join("."), v.clone());
        }
    }
}

#[test]
fn the_sn40l_column_agrees_with_the_sn50_column_except_where_the_facts_differ() {
    let a = FabricTarget::rdu_shaped();
    let b = sn50();
    assert_eq!(
        hosted_ops(&a),
        hosted_ops(&b),
        "both RDU envelopes host the same realized ops"
    );
    let (mut la, mut lb) = (BTreeMap::new(), BTreeMap::new());
    leaves(&serde_json::to_value(&a).unwrap(), &mut Vec::new(), &mut la);
    leaves(&serde_json::to_value(&b).unwrap(), &mut Vec::new(), &mut lb);
    assert_eq!(
        la.keys().collect::<Vec<_>>(),
        lb.keys().collect::<Vec<_>>(),
        "the same descriptor shape"
    );
    let differing: BTreeSet<&str> = la
        .iter()
        .filter(|(k, v)| lb[*k] != **v)
        .map(|(k, _)| k.as_str())
        .collect();
    let expected: BTreeSet<&str> = [
        "name",
        "mem.sram_kib",
        "mem.ddr_gib",
        "silicon.peak_bf16_pflops",
    ]
    .into_iter()
    .collect();
    assert_eq!(differing, expected, "the SN40L and SN50 envelopes differ exactly where the published facts differ (name, SRAM, DDR ceiling, the bf16 roofline)");
    let (fa, fb) = (
        RduChipFacts::of(RduChip::Sn40l),
        RduChipFacts::of(RduChip::Sn50),
    );
    assert_eq!(la["mem.sram_kib"], Value::from(fa.sram_mib.value * 1024));
    assert_eq!(lb["mem.sram_kib"], Value::from(fb.sram_mib.value * 1024));
    assert_eq!(la["mem.ddr_gib"], Value::from(fa.ddr_gib_max.value));
    assert_eq!(lb["mem.ddr_gib"], Value::from(fb.ddr_gib_max.value));
}
