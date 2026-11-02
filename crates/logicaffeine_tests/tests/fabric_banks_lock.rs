//! fabric_banks_lock — a fabric site's memories as interleaved banks, a run of elements moving as many banks as it is
//! aligned to in a cycle.
//!
//! A site's local memory is `SiteSpec::mem_banks` banks, element `e` in bank `e mod L` at row `e div L`, and every bank of
//! a port reads and writes the one row its address sits at. A run of `w` elements — `w` a power of two up to `L` — that
//! starts at a multiple of `w` lies in one row, its `j`-th element in bank `start | j`, so one cycle moves it with no
//! lane rotated past its own word. Every tensor memory and every FIFO page is banked, and a page starts on a word. A tile
//! walk, an accumulator walk and a matmul's loads move the widest run every row they start keeps aligned, in each memory
//! they touch; anywhere else an element a cycle. The XC7 parts bank their memories as wide as their PE arrays. This lock
//! holds:
//!
//! - at 1, 2, 4, 8 and 16 banks, programs whose rows keep every alignment — the decode step, a LUT attention, matmuls
//!   whose rows are 3, 5, 7, 13, 6, 8 and 12 wide, a row gather and a row write by a runtime index, an odd-width requant
//!   and clamp — run in RTL to the evaluator's answer in exactly the twin's cycles;
//! - each walk moves the widest run its rows keep: a matmul's 8-wide rows move 8 a cycle; its 12-wide weight rows, kept
//!   at a pitch of 16 where the banks are wider than four, 4 at four banks, 8 at eight and a whole row at sixteen; its
//!   6-wide rows of results 2, however many banks there are past that;
//! - a tile walk moves a bank's worth a cycle: eight banks read the decode step's tiles in at most a quarter of one bank's
//!   cycles;
//! - every doubling of the banks takes the decode step fewer cycles, through sixteen;
//! - the Nexys parts bank as wide as their PE arrays;
//! - a row that is not a whole word moves as the next whole word does: a tensor of more than one row whose rows are not
//!   whole words, and a matmul's B page, keep each row at a pitch of the word it moves in, where every walk keeps within
//!   a row; a walk keeps only the address terms that move it; under the same blocks a matmul over such rows reads, loads
//!   and writes in no more cycles than over the next whole word's;
//! - pitched operands and results, and a pitched page filled an element a cycle from a weight that keeps its rows end to
//!   end, run in RTL to the evaluator's answer in exactly the twin's cycles;
//! - every tensor's image fills its memory: each bank's file holds the rows its memory declares at the tensor's pitch,
//!   for values and zeros alike.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_banks_lock` (iverilog-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{ChoreoOp, Pred, PredRhs, SpatialProgram, TensorBody, TensorRole};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
use logicaffeine_compile::spatial::ir::LinkPoint;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_breakdown, fabric_cycle_model, fabric_cycle_segments, fabric_micro_listing, Activity, FabricBackend};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::ir::{BlockShape, NumericsPolicy};
use logicaffeine_compile::spatial::passes::tile::candidates;
use logicaffeine_compile::spatial::pipeline::{build_candidate, compile_program_to_spatial, template_from_source_with};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::{elem_map, inputs_from_fills, one_generic_on_one_input, parallel};

/// The XC7A100T's fabric with its memories in `banks` banks.
fn banked(banks: usize) -> SpatialTarget {
    let mut t = SpatialTarget::xc7a100t_fabric(16, 64);
    t.site.mem_banks = banks;
    t
}

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

fn tensor(name: &str, rows: usize, cols: usize, vals: &[i64]) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Int.\n");
    for (i, v) in vals.iter().enumerate() {
        if *v != 0 {
            s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
        }
    }
    s
}

fn spread(n: usize, lo: i64, hi: i64, seed: u64) -> Vec<i64> {
    (0..n as u64).map(|i| lo + ((i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 7) % (hi - lo + 1) as u64) as i64).collect()
}

/// Programs whose rows start at every offset into a bank: rows of 3, 5, 6, 7, 8, 12 and 13 elements, a gather and a
/// write at a runtime row, a LUT attention over three cached rows of two heads.
fn unit_programs() -> Vec<(&'static str, String)> {
    let lut = {
        let mut s = String::from("## Main\n");
        s.push_str(&tensor("Q", 1, 4, &[100, 0, 0, -77]));
        s.push_str(&tensor("K", 4, 4, &spread(16, -128, 127, 3)));
        s.push_str(&tensor("V", 4, 4, &spread(16, -128, 127, 4)));
        s.push_str(&tensor("E", 1, 4, &[32768, 28672, 25088, 21952]));
        s.push_str("Let at be 3.\nLet C be lut attention of Q over K and V with 2 heads 2 groups up to at with table E and scale 21076 shifted right by 6.\n");
        s
    };
    vec![
        ("lut attention", lut),
        (
            "matmul 3x5 by 5x7",
            format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 3, 5, &spread(15, -128, 127, 5)), tensor("W", 5, 7, &spread(35, -128, 127, 6))),
        ),
        (
            "matmul 2x13 by 13x3",
            format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 2, 13, &spread(26, -128, 127, 7)), tensor("W", 13, 3, &spread(39, -128, 127, 8))),
        ),
        (
            "matmul 2x8 by 8x12",
            format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 2, 8, &spread(16, -128, 127, 13)), tensor("W", 8, 12, &spread(96, -128, 127, 14))),
        ),
        (
            "row gather",
            format!("## Main\n{}Let at be 4.\nLet r be row at of T.\n", tensor("T", 5, 7, &spread(35, -900, 900, 9))),
        ),
        (
            "row write",
            format!(
                "## Main\n{}{}Let y be x clamped between -128 and 127.\nLet at be 3.\nSet row at of K to y.\nLet r be row at of K.\n",
                tensor("K", 6, 5, &spread(30, -100, 100, 10)),
                tensor("x", 1, 5, &[300, -7, -400, 12, 127])
            ),
        ),
        (
            "requant 13",
            format!("## Main\n{}Let Y be X times 104593 shifted right by 24.\n", tensor("X", 1, 13, &spread(13, -258_064, 258_064, 11))),
        ),
        (
            "clamp 13",
            format!("## Main\n{}Let Y be X clamped between -128 and 127.\n", tensor("X", 1, 13, &spread(13, -400, 400, 12))),
        ),
    ]
}

fn lowered(label: &str, src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{label}: a tensor program: {e:?}"));
    l.program.unwrap_or_else(|| panic!("{label}@{}: lowers whole: {:?}", target.name, l.missing.first()))
}

/// `src` on `target` under iverilog: every output and state tensor the evaluator's, in exactly the twin's cycles.
fn holds(label: &str, src: &str, target: &SpatialTarget) {
    holds_program(label, &lowered(label, src, target), target);
}

/// `p` on `target` under iverilog: every output and state tensor the evaluator's, in exactly the twin's cycles.
fn holds_program(label: &str, p: &SpatialProgram, target: &SpatialTarget) {
    let p = p.clone();
    let inputs = inputs_from_fills(&p);
    let want = eval_choreography(&p, &inputs).unwrap_or_else(|e| panic!("{label}: the evaluator: {e:?}"));
    let art = match FabricBackend::new(target.clone()).lower(&p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{label}: the fabric lowers it: {other:?}"),
    };
    let dir = std::env::temp_dir().join(format!("fabric_banks_{}_{}_{}", label.replace(' ', "_"), target.site.mem_banks, std::process::id()));
    let run = fabric_rtl::simulate(&dir, &p, &art, &inputs);
    assert!(run.done, "{label}: the design finishes\n{}", run.log);
    for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
        let got = run.out.get(&t.id.idx()).unwrap_or_else(|| panic!("{label}: the testbench prints {}", t.name));
        assert_eq!(got, w, "{label}: {} in RTL is the evaluator's", t.name);
    }
    assert_eq!(run.cycles, fabric_cycle_model(&p, target), "{label}: the RTL's cycles are the twin's");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_bank_count_runs_runs_that_start_anywhere() {
    if !common::require_tool("iverilog") {
        return;
    }
    for banks in [1, 2, 4, 8, 16] {
        let target = banked(banks);
        for (label, src) in unit_programs() {
            holds(&format!("{label} at {banks} banks"), &src, &target);
        }
    }
}

/// A tile walk moves runs no wider than every term of its address keeps aligned, its base among them. The front end puts
/// a row's offset in the base (row `r` of a `C`-column tensor reads from `r·C`, aligned as `C` is), so a base off its
/// row's word is an address the IR allows and the front end does not yet write. Here the read of `row 1 of T`, eight
/// elements of a 4 by 8 tensor, is moved to every base from 1 to 24; the evaluator and the RTL run the same rewritten
/// program.
#[test]
fn a_read_from_any_base_runs_at_its_base_s_alignment() {
    if !common::require_tool("iverilog") {
        return;
    }
    let src = format!("## Main\n{}Let r be row 1 of T.\n", tensor("T", 4, 8, &spread(32, -900, 900, 21)));
    for banks in [2, 4, 8, 16] {
        let target = banked(banks);
        let template = lowered("row 1 of T", &src, &target);
        let t = template.tensors.iter().find(|t| t.name == "T").expect("T").id;
        for base in 1..=24 {
            let mut p = template.clone();
            let mut moved = 0;
            for prog in &mut p.schedule.as_mut().expect("a scheduled program").programs {
                for op in &mut prog.ops {
                    if let ChoreoOp::ReadTiles { tensor, at, .. } = &mut op.op {
                        if *tensor == t {
                            at.base = base;
                            moved += 1;
                        }
                    }
                }
            }
            assert_eq!(moved, 1, "one read of T");
            holds_program(&format!("row 1 of T from {base} at {banks} banks"), &p, &target);
        }
    }
}

/// Two slots of one port share a read only when a select keeps them apart: each is read on one side only. A slot read on
/// both sides of the select reads on every point, and shares with none. Here three slots read one input at offsets 0, 8
/// and 16, and `y[c] = a[c] + (c < 4 ? a[c + 8] : a[c + 16])`: the second and third share, the first reads alone.
#[test]
fn a_slot_read_on_both_sides_of_a_select_shares_no_read() {
    if !common::require_tool("iverilog") {
        return;
    }
    let a: Vec<i64> = spread(24, -500, 500, 31);
    let op = |i: usize| Box::new(TensorBody::Operand(i));
    let body = TensorBody::Select {
        cond: Pred::AxisLt(LV_GEN_COL, PredRhs::Const(4)),
        then: Box::new(TensorBody::Add(op(0), op(1))),
        otherwise: Box::new(TensorBody::Add(op(0), op(2))),
    };
    for banks in [1, 2, 4, 8, 16] {
        let target = banked(banks);
        let t = target.tile.rows;
        let p = one_generic_on_one_input(
            &target,
            "a",
            24,
            a.clone(),
            8,
            vec![parallel(LV_GEN_ROW, t), parallel(LV_GEN_COL, 8)],
            vec![elem_map(0, &[(LV_GEN_COL, 1)]), elem_map(8, &[(LV_GEN_COL, 1)]), elem_map(16, &[(LV_GEN_COL, 1)])],
            elem_map(0, &[(LV_GEN_COL, 1)]),
            body.clone(),
        );
        let want = eval_choreography(&p, &inputs_from_fills(&p)).expect("the evaluator");
        let y = p.tensors.iter().find(|t| t.name == "y").expect("y").id;
        let Some(Value::Int(got)) = want.tensors.get(&y) else { panic!("the evaluator writes y") };
        let expect: Vec<i64> = (0..8).map(|c| a[c] + if c < 4 { a[c + 8] } else { a[c + 16] }).collect();
        assert_eq!(&got[..8], &expect[..], "the evaluator computes the select");
        holds_program(&format!("a select over one input at {banks} banks"), &p, &target);
    }
}

/// A tile walk's runs end at its row's end: the first run is no longer than the row, and each run after it no longer than
/// what is left of it. They bind where nothing else narrows a run: a row written at a word with no stride in its address
/// moves runs as wide as the banks, wider than a row of 5 and not dividing a row of 13. Here `row 1 of K` is written
/// through such an address and `row 2 of K` read back: an element of a run past its row would land in row 2.
#[test]
fn a_run_lands_no_element_past_its_row() {
    if !common::require_tool("iverilog") {
        return;
    }
    for cols in [5usize, 13] {
        let src = format!(
            "## Main\n{}{}Let y be x clamped between -128 and 127.\nSet row 1 of K to y.\nLet r be row 2 of K.\n",
            tensor("K", 3, cols, &spread(3 * cols, -100, 100, 41)),
            tensor("x", 1, cols, &spread(cols, -300, 300, 42))
        );
        for banks in [2, 4, 8, 16] {
            let target = banked(banks);
            let mut p = lowered("row write", &src, &target);
            let k = p.tensors.iter().find(|t| t.name == "K").expect("K").id;
            let mut wrote = 0;
            for prog in &mut p.schedule.as_mut().expect("a scheduled program").programs {
                for op in &mut prog.ops {
                    if let ChoreoOp::WriteTiles { tensor, at, .. } = &mut op.op {
                        if *tensor == k {
                            assert_eq!(at.base, 0, "row 1 of K starts at 0");
                            at.steps.clear();
                            wrote += 1;
                        }
                    }
                }
            }
            assert_eq!(wrote, 1, "one write of K");
            holds_program(&format!("row 1 of K, {cols} wide, at {banks} banks"), &p, &target);
        }
    }
}

/// The field `name: <usize>` of a micro listing line.
fn field(line: &str, name: &str) -> Option<usize> {
    let at = line.find(&format!("{name}: "))? + name.len() + 2;
    line[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// The number after ` word ` in a micro listing line.
fn suffix(line: &str, word: &str) -> Option<usize> {
    let at = line.rfind(&format!(" {word} "))? + word.len() + 2;
    line[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// The shapes the walks' guards are written for and that every program the planner gives a banked fabric keeps. Each
/// guard below changes nothing while its shape holds, so a mutant dropping it survives every run; this lock holds the
/// shapes, and names the guard a planner breaking one must then test by running it:
/// - a tile walk of several rows moves whole rows of its tensor, or steps along them by its own block's width. Where the
///   tensor keeps its rows end to end, a term of its alignment is then already its page rows' (`run`'s page-row term);
///   where it keeps them at a pitch, the page-row term is the one in force, and running it holds it: a walk that drops it
///   fails `every_bank_count_runs_runs_that_start_anywhere`;
/// - an accumulator walk moves whole rows of its region, so no run ends early at the region's end (`ar_ln`);
/// - a pack's page holds no more tiles than its buffer's own page (`page_extents`' pack term);
/// - no chain of guards reserves one FIFO twice (the early check's own-reservation count).
#[test]
fn the_walks_keep_the_shapes_their_guards_are_written_for() {
    let mut programs = unit_programs();
    programs.push(("decode step", decode_step()));
    let mut walks = 0;
    for banks in [2, 4, 8, 16] {
        let target = banked(banks);
        for (label, src) in &programs {
            let p = lowered(label, src, &target);
            for (_, engine, lines) in fabric_micro_listing(&p, &target).expect("the fabric lowers it") {
                let mut chain: Vec<usize> = Vec::new();
                for l in &lines {
                    let at = format!("{label} at {banks} banks, {engine:?}: {l}");
                    if l.starts_with("read tiles") || l.starts_with("write tiles") {
                        let (h, w, sh) = (field(l, "block_h").unwrap(), field(l, "block_w").unwrap(), field(l, "stride_h").unwrap());
                        let tensor: usize = l.find("tensor: TensorId(").map(|i| l[i + 17..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()).unwrap();
                        let cols = p.tensors[tensor].tiles.cols_t;
                        // A walk of several rows moves whole rows of its tensor — rows `stride_h` apart, end to end or at
                        // the tensor's pitch — or steps along its rows by its own block's width.
                        let steps_by_block = l.contains(&format!(", {w})]")) || l.contains(&format!(", {w}), "));
                        if h > 1 {
                            assert!(w == sh || w == cols || steps_by_block, "a walk of several rows moves whole rows or steps by its block (the run's page-row term): {at}");
                        }
                        walks += 1;
                    }
                    if l.starts_with("acquire") || l.starts_with("pack") {
                        let (tiles, region, run) = (field(l, "tiles").unwrap(), suffix(l, "region").unwrap(), suffix(l, "run").unwrap());
                        if run > 1 {
                            assert_eq!(tiles % region, 0, "an accumulator walk moves whole rows of its region (ar_ln): {at}");
                        }
                        if l.starts_with("pack") {
                            let to = field(l, "to: BufferId(").or_else(|| {
                                let i = l.find("BufferId(")? + 9;
                                l[i..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
                            }).unwrap();
                            assert!(tiles <= p.buffers[to].kind.page_tiles(), "a pack's page fits its buffer's page (page_extents' pack term): {at}");
                        }
                        walks += 1;
                    }
                    if l.starts_with("reserve") {
                        let b = l.find("BufferId(").map(|i| l[i + 9..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<usize>().unwrap()).unwrap();
                        assert!(!chain.contains(&b), "no chain of guards reserves one FIFO twice (the early check's own reservations): {at}");
                        chain.push(b);
                    } else if !(l.starts_with("wait") || l.starts_with("semaphore wait")) {
                        chain.clear();
                    }
                }
            }
        }
    }
    assert!(walks > 0);
}

#[test]
fn every_bank_count_runs_the_decode_step() {
    if !common::require_tool("iverilog") {
        return;
    }
    let src = decode_step();
    for banks in [1, 2, 4, 8, 16] {
        holds(&format!("decode step at {banks} banks"), &src, &banked(banks));
    }
}

/// The decode step's cycles, and the cycles its engines spend reading tiles, at `banks` banks.
fn decode_cycles(banks: usize) -> (u64, u64) {
    let target = banked(banks);
    let p = lowered("decode step", &decode_step(), &target);
    let b = fabric_cycle_breakdown(&p, &target).expect("the fabric lowers the step");
    let reads = b.engines.iter().map(|e| e.cycles.get(&Activity::Busy("read tiles")).copied().unwrap_or(0)).sum();
    (b.total, reads)
}

/// A(2×8)·W(8×12) on the XC7A100T at `banks` banks, under the plan of 2×6 output blocks over the whole inner 8 — the
/// plan held fixed, so what moves is the walks alone and not the planner's choice: its engines' cycles reading tiles,
/// writing tiles, acquiring, packing and in the matmul.
fn walks_of_a_matmul(banks: usize) -> [u64; 5] {
    let src = format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 2, 8, &spread(16, -128, 127, 13)), tensor("W", 8, 12, &spread(96, -128, 127, 14)));
    let target = banked(banks);
    let template = template_from_source_with(&src, &target, NumericsPolicy::Exact).expect("a fabric template");
    let plan = candidates(&template, &target)
        .into_iter()
        .find(|c| c.plan.block == BlockShape { osh: 2, osw: 6, ibw: 8 } && c.plan.grid == (1, 1))
        .expect("the 2×6 block over the whole inner 8 is a candidate");
    let p = build_candidate(&template, &plan, &target).expect("the plan builds");
    let b = fabric_cycle_breakdown(&p, &target).expect("the fabric lowers it");
    let busy = |kind: &'static str| -> u64 { b.engines.iter().map(|e| e.cycles.get(&Activity::Busy(kind)).copied().unwrap_or(0)).sum() };
    [busy("read tiles"), busy("write tiles"), busy("acquire"), busy("pack"), busy("matmul")]
}

/// A's rows of 8 keep runs of 8; W's rows of 12, at a pitch of 16 past four banks, runs of 4 at four banks and of as many
/// as the banks past that; C's rows of 6 — the page the pack fills, the matmul's partial and B rows — runs of 2; the
/// accumulator's own zeroing walks whole words of its rows. With `w` the run a walk keeps, capped by the banks `b`, at a
/// read latency of one:
///
/// - read tiles: A `2·⌈8/w⌉ + 1`, W `8·⌈12/w⌉ + 1`;
/// - write tiles, two sub-blocks: `2·(2·⌈6/w⌉ + 1)`;
/// - acquire, two sub-blocks: `2·2·⌈6/b⌉`; pack: `2·2·⌈6/w⌉`;
/// - matmul, two sub-blocks: `2·(2·⌈8/w⌉ + 8·⌈6/w⌉ + 25)`, its loads at the run C's rows keep.
#[test]
fn each_walk_moves_the_widest_run_its_rows_keep() {
    let want: [(usize, [u64; 5]); 5] = [
        (1, [17 + 97, 26, 24, 24, 178]),
        (2, [9 + 49, 14, 12, 12, 114]),
        (4, [5 + 25, 14, 8, 12, 114]),
        (8, [3 + 17, 14, 4, 12, 114]),
        (16, [3 + 9, 14, 4, 12, 114]),
    ];
    for (banks, cycles) in want {
        assert_eq!(walks_of_a_matmul(banks), cycles, "at {banks} banks: read tiles, write tiles, acquire, pack, matmul");
    }
}

#[test]
fn a_tile_walk_moves_a_bank_s_worth_a_cycle() {
    let (_, one) = decode_cycles(1);
    let (_, eight) = decode_cycles(8);
    assert!(one > 0, "the step reads tiles");
    assert!(4 * eight <= one, "eight banks read the step's tiles in {eight} cycles, one bank in {one}");
}

#[test]
fn every_doubling_of_the_banks_takes_the_decode_step_fewer_cycles() {
    let cycles: Vec<u64> = [1, 2, 4, 8, 16].iter().map(|b| decode_cycles(*b).0).collect();
    for w in cycles.windows(2) {
        assert!(w[1] < w[0], "doubling the banks takes the step fewer cycles: {cycles:?}");
    }
}

#[test]
fn the_nexys_parts_bank_as_wide_as_their_pe_arrays() {
    assert_eq!(SpatialTarget::xc7a100t_fabric(16, 64).site.mem_banks, 8);
    assert_eq!(SpatialTarget::xc7a200t_fabric(16, 64).site.mem_banks, 16);
    assert_eq!(SpatialTarget::sky_fabric(16, 64).site.mem_banks, 1, "the sky130 fabric's flop memories answer an element a cycle");
}

/// A pack may move fewer of the accumulator's tiles than a row holds (the evaluator packs the first `tiles`). Its page
/// then ends inside the row, and the run walk stops there, its last run as short as the page leaves it. The pipeline
/// packs whole accumulators, so the program is a clamped row written into a kept tensor, with the pack and the write
/// of its page cut to `k` tiles; the tensor keeps its own values past them.
#[test]
fn a_pack_whose_page_ends_inside_a_row_stops_at_the_page() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (cols, k) in [(16usize, 13usize), (13, 6), (24, 9)] {
        let src = format!(
            "## Main\n{}{}Let y be x clamped between -128 and 127.\nSet row 1 of K to y.\nLet r be row 2 of K.\n",
            tensor("K", 2, cols, &spread(2 * cols, -100, 100, 5)),
            tensor("x", 1, cols, &spread(cols, -300, 300, 7))
        );
        for banks in [2, 4, 8, 16] {
            let target = banked(banks);
            let mut p = lowered("partial pack", &src, &target);
            let kt = p.tensors.iter().find(|t| t.name == "K").expect("K").id;
            let mut page = None;
            for prog in &mut p.schedule.as_mut().expect("a scheduled program").programs {
                for op in &mut prog.ops {
                    if let ChoreoOp::WriteTiles { from, tensor, block_h, block_w, .. } = &mut op.op {
                        if *tensor == kt {
                            assert_eq!((*block_h, *block_w), (1, cols), "row 1 of K is written whole");
                            assert!(page.is_none(), "one write of K");
                            *block_w = k;
                            page = Some(*from);
                        }
                    }
                }
            }
            let page = page.expect("row 1 of K is written");
            let mut packs = 0;
            for prog in &mut p.schedule.as_mut().unwrap().programs {
                for op in &mut prog.ops {
                    if let ChoreoOp::PackAcc { to, tiles, .. } = &mut op.op {
                        if *to == page {
                            assert_eq!(*tiles, cols, "the clamp packs its whole row");
                            *tiles = k;
                            packs += 1;
                        }
                    }
                }
            }
            assert_eq!(packs, 1, "one pack into the page K is written from");
            holds_program(&format!("a pack of {k} of a {cols}-wide row at {banks} banks"), &p, &target);
        }
    }
}

/// An add of two rows the site holds is one work item over the row — the form a row map has — at every bank count,
/// and holds under iverilog: every element the evaluator's, in the twin's cycles.
#[test]
fn a_row_add_is_one_item_over_its_row() {
    if !common::require_tool("iverilog") {
        return;
    }
    for n in [5usize, 13, 16] {
        let src = format!(
            "## Main\n{}{}Let z be x plus y.\n",
            tensor("x", 1, n, &spread(n, -3000, 3000, n as u64)),
            tensor("y", 1, n, &spread(n, -3000, 3000, n as u64 + 7))
        );
        for banks in [1usize, 8, 16] {
            let target = banked(banks);
            let p = lowered(&format!("add {n}"), &src, &target);
            assert_eq!(p.work.items.len(), 1, "add {n} on {banks} banks: one item over the row, not one a tile");
            holds_program(&format!("add {n}"), &p, &target);
        }
    }
}

/// Each section's cycles in `p`: a halt after every section, whose counts at the halts — one cycle more a halt — mark
/// where each section ends.
fn section_cycles(p: &SpatialProgram, target: &SpatialTarget) -> Vec<(String, u64)> {
    let mut q = p.clone();
    q.links = (0..p.sections.len().saturating_sub(1)).map(|k| LinkPoint { after: Some(k), sends: vec![], receives: vec![] }).collect();
    let seg = fabric_cycle_segments(&q, target).expect("the twin");
    let mut prev = 0u64;
    seg.iter()
        .enumerate()
        .map(|(k, c)| {
            let at = c - k as u64;
            let d = at - prev;
            prev = at;
            (p.sections[k].name.clone(), d)
        })
        .collect()
}

/// On the decode step at every bank count, every add of two rows takes at most twice the cycles a requant of a row of
/// its width takes: an add is a row map with a second operand to read, never a walk of an element at a time.
#[test]
fn a_row_add_costs_no_more_than_two_row_maps_of_its_width() {
    for banks in [1usize, 2, 4, 8, 16] {
        let target = banked(banks);
        let cycles = section_cycles(&lowered("decode step", &decode_step(), &target), &target);
        let adds: Vec<&(String, u64)> = cycles.iter().filter(|(n, _)| n.starts_with("elementwise_+_")).collect();
        assert!(!adds.is_empty(), "{banks} banks: the decode step adds rows");
        for (name, c) in adds {
            let width = name.trim_start_matches("elementwise_+_");
            let map = cycles
                .iter()
                .find(|(n, _)| *n == format!("int_requant_{width}"))
                .unwrap_or_else(|| panic!("{banks} banks: a requant of a row of {width}"))
                .1;
            assert!(*c <= 2 * map, "{banks} banks: {name} takes {c} cycles, a requant of its row {map}");
        }
    }
}

/// The word a row of `width` elements moves in at `banks` banks: the narrowest power of two that holds it, no wider than
/// the banks.
fn row_word(width: usize, banks: usize) -> usize {
    width.next_power_of_two().min(banks)
}

/// The cycles x(1×k)·W(k×n) spends reading tiles, writing them and in its matmuls on the XC7A100T at `banks` banks,
/// under the plan of blocks `osw` columns wide over `ibw` of the inner `k`, on one site — the plan held fixed, so what
/// moves is the walks and the loads alone, not the planner's choice — or `None` where that block is not a candidate.
fn walks_under(k: usize, n: usize, banks: usize, osw: usize, ibw: usize) -> Option<(u64, u64, u64)> {
    let src = format!("## Main\n{}{}Let C be x matmul W.\n", tensor("x", 1, k, &spread(k, -128, 127, 21)), tensor("W", k, n, &spread(k * n, -128, 127, 22)));
    let target = banked(banks);
    let template = template_from_source_with(&src, &target, NumericsPolicy::Exact).expect("a fabric template");
    let plan = candidates(&template, &target).into_iter().find(|c| c.plan.block == BlockShape { osh: 1, osw, ibw } && c.plan.grid == (1, 1))?;
    let p = build_candidate(&template, &plan, &target).expect("the plan builds");
    let b = fabric_cycle_breakdown(&p, &target).expect("the fabric lowers it");
    let busy = |kind: &'static str| -> u64 { b.engines.iter().map(|e| e.cycles.get(&Activity::Busy(kind)).copied().unwrap_or(0)).sum() };
    Some((busy("read tiles"), busy("write tiles"), busy("matmul")))
}

/// ★ A ROW STARTS ON A WORD. A matmul whose weight rows are not a whole word — the decode step's tied head is sixteen
/// rows of fourteen — reads its weights and loads them in no more cycles than the matmul whose rows are the next whole
/// word, under the same blocks, at every bank count: the weight tensor and the page its rows land in keep each row at a
/// pitch of whole words, so a row moves in the runs of the next whole word, not in the runs its width's alignment allows.
/// Its one row of results is written as the whole word's is: a walk keeps only the address terms that move it, so the
/// stride of a loop that never runs past its first trip narrows no run. Two block choices are compared: the widest both
/// widths share — columns that divide both, the deepest inner run both take — and, where the array takes it, a block as
/// wide as each width, whose one sub-block spans every column, so that its B rows and its row of results move whole.
#[test]
fn a_row_that_is_not_a_whole_word_moves_as_the_next_whole_word_does() {
    let gcd = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let mut full = 0;
    for banks in [2usize, 4, 8, 16] {
        for (k, n) in [(16usize, 14usize), (16, 6), (16, 10), (16, 12), (8, 5), (12, 7), (8, 6), (8, 7)] {
            let whole = n.next_multiple_of(row_word(n, banks));
            if whole == n {
                continue;
            }
            let shared = gcd(n, whole);
            let deepest = |osw: usize, osw_whole: usize| {
                (1..=k)
                    .rev()
                    .filter(|ibw| k % ibw == 0)
                    .find_map(|ibw| Some((ibw, walks_under(k, n, banks, osw, ibw)?, walks_under(k, whole, banks, osw_whole, ibw)?)))
            };
            let walks = deepest(shared, shared)
                .unwrap_or_else(|| panic!("{banks} banks: a block {shared} wide is a candidate for W({k}×{n}) and W({k}×{whole})"));
            let wide = deepest(n, whole);
            full += wide.is_some() as usize;
            for (blocks, (ibw, (read, write, mm), (read_whole, write_whole, mm_whole))) in
                std::iter::once((format!("1×{shared}"), walks)).chain(wide.map(|w| (format!("1×{n} against 1×{whole}"), w)))
            {
                assert!(
                    read <= read_whole && write <= write_whole && mm <= mm_whole,
                    "{banks} banks, blocks {blocks}×{ibw}: x(1×{k})·W({k}×{n}) reads tiles {read} cycles, writes {write} and \
                     multiplies {mm}; x·W({k}×{whole}) {read_whole}, {write_whole} and {mm_whole}"
                );
            }
        }
    }
    assert!(full >= 6, "blocks as wide as each width are compared at {full} shapes");
}

/// Matmuls of several rows whose operands and results have rows that are not whole words — A, W and C each kept at a
/// pitch — run in RTL to the evaluator's answer in exactly the twin's cycles at every bank count, and so do the row ops
/// of the same widths beside them, over tensors the front end holds as one row (`row at of T`, an elementwise op of a
/// matrix), which take no pitch.
#[test]
fn pitched_operands_and_results_run_to_the_evaluator_s_answer() {
    if !common::require_tool("iverilog") {
        return;
    }
    let programs = [
        ("matmul 3x16 by 16x14", format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 3, 16, &spread(48, -128, 127, 31)), tensor("W", 16, 14, &spread(224, -128, 127, 32)))),
        ("matmul 2x6 by 6x10", format!("## Main\n{}{}Let C be A matmul W.\n", tensor("A", 2, 6, &spread(12, -128, 127, 33)), tensor("W", 6, 10, &spread(60, -128, 127, 34)))),
        ("gather of a row of 11", format!("## Main\n{}Let at be 3.\nLet r be row at of T.\n", tensor("T", 5, 11, &spread(55, -900, 900, 35)))),
        (
            "write of a row of 6",
            format!("## Main\n{}{}Let at be 2.\nSet row at of K to x.\nLet r be row at of K.\n", tensor("K", 4, 6, &spread(24, -100, 100, 36)), tensor("x", 1, 6, &[9, -8, 7, -6, 5, -4])),
        ),
        ("clamp of a 3 by 5", format!("## Main\n{}Let Y be X clamped between -128 and 127.\n", tensor("X", 3, 5, &spread(15, -400, 400, 37)))),
        ("requant of a 2 by 7", format!("## Main\n{}Let Y be X times 104593 shifted right by 24.\n", tensor("X", 2, 7, &spread(14, -258_064, 258_064, 38)))),
    ];
    for banks in [2usize, 4, 8, 16] {
        let target = banked(banks);
        for (label, src) in &programs {
            holds(&format!("{label} at {banks} banks"), src, &target);
        }
    }
}


/// The rows the RTL declares memory `mem` with: its `reg [..] <mem> [0:R-1];`.
fn declared_rows(rtl: &str, mem: &str) -> Option<usize> {
    let key = format!(" {mem} [0:");
    let line = rtl.lines().find(|l| l.contains("reg [") && l.contains(&key))?;
    let at = line.find(&key)? + key.len();
    line[at..].split(']').next()?.parse::<usize>().ok().map(|last| last + 1)
}

/// ★ A TENSOR'S IMAGE FILLS ITS MEMORY. Every image file a tensor ships — one per bank and lane, whether its words are
/// values, a pattern or zeros — holds exactly the rows its memory declares, its rows at the tensor's pitch: no row of
/// the memory is left for `$readmemh` to leave unknown. Here a zero weight and a weight of values, both of rows of
/// twelve kept at a pitch of sixteen, and the decode step's state, at every bank count; the zero weight's product then
/// runs to the evaluator's answer.
#[test]
fn every_tensor_image_fills_its_memory_at_its_pitch() {
    let a = tensor("A", 1, 8, &[17, -5, 99, -128, 4, 0, 127, -1]);
    let programs = [
        ("a zero weight of rows of twelve", format!("## Main\n{a}Let Z be a 8 by 12 tensor of Int.\nLet C be A matmul Z.\n")),
        ("a weight of rows of twelve", format!("## Main\n{a}{}Let C be A matmul W.\n", tensor("W", 8, 12, &spread(96, -128, 127, 46)))),
        ("the decode step", decode_step()),
    ];
    for banks in [2usize, 4, 8, 16] {
        let target = banked(banks);
        for (label, src) in &programs {
            let p = lowered(label, src, &target);
            let art = match FabricBackend::new(target.clone()).lower(&p) {
                Ok(SpatialArtifact::Verilog(v)) => v,
                other => panic!("{label} at {banks} banks: lowers: {other:?}"),
            };
            let mut images = 0;
            for (name, body) in art.hex.iter().filter(|(n, _)| n.starts_with('t') && n.ends_with(".hex")) {
                let mem = format!("mem_{}", name.trim_end_matches(".hex"));
                let rows = declared_rows(&art.rtl, &mem).unwrap_or_else(|| panic!("{label} at {banks} banks: the RTL declares {mem} for {name}"));
                assert_eq!(body.lines().count(), rows, "{label} at {banks} banks: {name} holds every row of {mem}");
                images += 1;
            }
            assert!(images > 0, "{label} at {banks} banks: the tensors ship images");
        }
        let z = tensor_pitch_rows(&lowered("zero weight", &programs[0].1, &target), "Z", banks);
        assert!(banks < 8 || z > 8 * 12 / banks, "at {banks} banks the zero weight's rows of twelve take a pitch, {z} rows a bank");
    }
    if common::require_tool("iverilog") {
        holds("a zero weight of rows of twelve at 8 banks", &programs[0].1, &banked(8));
    }
}

/// The rows a bank of tensor `name`'s memory takes in `p`'s RTL on `banks` banks.
fn tensor_pitch_rows(p: &SpatialProgram, name: &str, banks: usize) -> usize {
    let id = p.tensors.iter().find(|t| t.name == name).expect("the tensor").id.idx();
    let art = match FabricBackend::new(banked(banks)).lower(p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{name} lowers: {other:?}"),
    };
    let stem = if banks == 1 { format!("mem_t{id}") } else { format!("mem_t{id}_b0") };
    declared_rows(&art.rtl, &stem).or_else(|| declared_rows(&art.rtl, &format!("{stem}_l0"))).expect("the memory is declared")
}

/// Every tile read of tensor `name` in `p`'s engine programs, at any depth of loops.
fn reads_of<'a>(p: &'a mut SpatialProgram, name: &str) -> Vec<&'a mut ChoreoOp> {
    fn walk<'b>(ops: &'b mut [logicaffeine_compile::spatial::ir::TaggedOp], t: logicaffeine_compile::spatial::ir::TensorId, out: &mut Vec<&'b mut ChoreoOp>) {
        for op in ops {
            if matches!(&op.op, ChoreoOp::ReadTiles { tensor, .. } if *tensor == t) {
                out.push(&mut op.op);
            } else if let ChoreoOp::Loop { body, .. } = &mut op.op {
                walk(body, t, out);
            }
        }
    }
    let t = p.tensors.iter().find(|t| t.name == name).expect("the tensor").id;
    let mut out = Vec::new();
    for prog in &mut p.schedule.as_mut().expect("a scheduled program").programs {
        walk(&mut prog.ops, t, &mut out);
    }
    out
}

/// A B page whose rows keep a pitch, filled an element a cycle. x(1×4)·W(4×6) under the plan of one block of every
/// column over the whole inner four: the B page holds four rows of six, kept at a pitch of eight. W's read is rewritten to
/// take its rows five apart — each row overlapping the last's final element, every address inside W — so the walk runs
/// across W's rows, W keeps its rows end to end, and its runs are aligned to one: the walk moves an element a cycle and
/// still steps past each page row's padding. The evaluator and the RTL run the same rewritten program, to the same
/// answer, in the twin's cycles.
#[test]
fn a_pitched_page_filled_an_element_a_cycle_runs_to_the_evaluator_s_answer() {
    if !common::require_tool("iverilog") {
        return;
    }
    let src = format!("## Main\n{}{}Let C be x matmul W.\n", tensor("x", 1, 4, &[17, -5, 99, -128]), tensor("W", 4, 6, &spread(24, -128, 127, 45)));
    for banks in [4, 8, 16] {
        let target = banked(banks);
        let template = template_from_source_with(&src, &target, NumericsPolicy::Exact).expect("a fabric template");
        let plan = candidates(&template, &target)
            .into_iter()
            .find(|c| c.plan.block == BlockShape { osh: 1, osw: 6, ibw: 4 } && c.plan.grid == (1, 1))
            .expect("one block of every column over the whole inner four is a candidate");
        let mut p = build_candidate(&template, &plan, &target).expect("the plan builds");
        let mut moved = 0;
        for op in reads_of(&mut p, "W") {
            if let ChoreoOp::ReadTiles { block_h, stride_h, .. } = op {
                assert_eq!((*block_h, *stride_h), (4, 6), "W's read moves its four rows of six");
                *stride_h = 5;
                moved += 1;
            }
        }
        assert_eq!(moved, 1, "one read of W");
        let w = p.tensors.iter().find(|t| t.name == "W").expect("W").id;
        let lines: Vec<String> = fabric_micro_listing(&p, &target).expect("the fabric lowers it").into_iter().flat_map(|(_, _, l)| l).collect();
        assert!(
            lines.iter().any(|l| l.starts_with("read tiles") && l.contains(&format!("tensor: {w:?},")) && l.ends_with(" run 1")),
            "{banks} banks: W's read moves an element a cycle:\n{}",
            lines.join("\n")
        );
        holds_program(&format!("x·W, W's rows five apart, at {banks} banks"), &p, &target);
    }
}
