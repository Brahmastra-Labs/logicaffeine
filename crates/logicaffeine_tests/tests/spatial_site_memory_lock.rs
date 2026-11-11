//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A ROW ONE SITE CANNOT HOLD IS WORK FOR SEVERAL ITEMS — every row op, at every width, on every spatial preset, lowers
//! whole and computes what the VM computes.
//!
//! A whole-row form keeps its row, its intermediates and its result resident in one site. Two bounds end it: the
//! accumulator's page (`row_needs_blocking`), and the site's memory, which on a part of small tiles comes first — the
//! RTX 3060's 16 × 16 tiles fill a block's memory at 28 tiles of a row, well inside its 64-tile page. Past either bound
//! the op takes the form that computes a head, a tile or a block of the row an item, and the values stay the VM's.
//!
//!   • the unary maps (sigmoid, silu), the maps of two rows (plus, hadamard, swiglu), the norms of a whole row (rmsnorm,
//!     layernorm), the norms of heads (rmsnorm and l2norm, over heads a tile wide and over heads of 16, which straddle a
//!     32-wide tile), the softmax of a row a page holds and the arg-max;
//!   • at widths on both sides of the page, through the RTX 3060's memory bound and past twice the page;
//!   • each lowered whole for the preset and held to the VM — within 2% of the largest value (the bf16 datapath's bound),
//!     the arg-max exactly — or refused only for a precision or an engine the part does not have; a part with a float
//!     elementwise datapath lowers every map at every width.
//!
//! A KV cache's append — a row, or a block of rows, into a table read on as a table or not — is held the same way: its
//! column block is the largest a page holds whose section a site holds.
//!
//! The 7-row prefill a GPU hands to the RDU (`spatial_disaggregation_lock`) is one consumer: its rotations and per-head
//! norms of 28 heads of 16 are past the RTX 3060's memory and inside its page.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_site_memory_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/spatial.rs"]
#[allow(dead_code)]
mod spatial_support;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::passes::from_qwen_ops::row_needs_blocking;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use spatial_support::inputs_from_fills;

/// A `1 × n` tensor's declaration and fill: multiples of 1/8 in [−0.75, 0.75], which every datapath holds exactly, varied
/// within a tile, across tiles and by `salt`.
fn tensor(name: &str, n: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a 1 by {n} tensor of Real.\n");
    for k in 0..n {
        let v = -0.75 + ((k * 7 + salt * 3 + k / 16 * 5) % 13) as f64 / 8.0;
        s.push_str(&format!("Set item {} of {name} to {v:?}.\n", k + 1));
    }
    s
}

/// One row op over `n` values: its name, the program, the tensor it computes and how many values that tensor has.
struct Case {
    what: String,
    src: String,
    out: &'static str,
    len: usize,
}

fn cases(n: usize, target: &SpatialTarget) -> Vec<Case> {
    let tile_cols = target.tile.cols;
    let case = |what: String, body: String, out: &'static str, len: usize| Case { what, src: format!("## Main\n{body}"), out, len };
    let x = tensor("X", n, 0);
    let v = tensor("V", n, 1);
    let mut out = vec![
        case("sigmoid".into(), format!("{x}Let Y be sigmoid of X.\n"), "Y", n),
        case("silu".into(), format!("{x}Let Y be silu of X.\n"), "Y", n),
        case("plus".into(), format!("{x}{v}Let Y be X plus V.\n"), "Y", n),
        case("hadamard".into(), format!("{x}{v}Let Y be X hadamard V.\n"), "Y", n),
        case("swiglu".into(), format!("{x}{v}Let Y be swiglu of X and V.\n"), "Y", n),
        case("rmsnorm".into(), format!("{x}{}Let Y be rmsnorm of X with weight W.\n", tensor("W", n, 2)), "Y", n),
        case(
            "layernorm".into(),
            format!("{x}{}{}Let Y be layernorm of X with weight W and bias B.\n", tensor("W", n, 2), tensor("B", n, 3)),
            "Y",
            n,
        ),
        case("argmax".into(), format!("{x}Let t be argmax of X.\n"), "t", 1),
    ];
    if !row_needs_blocking(n, target) {
        out.push(case("softmax".into(), format!("{x}Let Y be softmax of X.\n"), "Y", n));
    }
    let mut head_widths = vec![16, tile_cols];
    head_widths.dedup();
    for hd in head_widths {
        if n % hd != 0 {
            continue;
        }
        let heads = n / hd;
        out.push(case(
            format!("rmsnorm across {heads} heads of {hd}"),
            format!("{x}{}Let Y be rmsnorm of X with weight G across {heads} heads.\n", tensor("G", hd, 2)),
            "Y",
            n,
        ));
        out.push(case(format!("l2norm across {heads} heads of {hd}"), format!("{x}Let Y be l2norm of X across {heads} heads.\n"), "Y", n));
    }
    out
}

/// The widths `target` is held at, in tiles: one tile, a few, the page and one past it, twice the page and three more,
/// and the RTX 3060's 28, 40 and 64 — each capped at 2,304 values so the VM's program stays small.
fn widths(target: &SpatialTarget) -> Vec<usize> {
    let tc = target.tile.cols;
    let page = (1..).find(|t| row_needs_blocking(t * tc, target)).expect("a page") - 1;
    let mut w = vec![1, 3, page, page + 1, 2 * page + 3, 28, 40, 64];
    w.retain(|t| *t >= 1 && t * tc <= 2304);
    w.sort_unstable();
    w.dedup();
    w
}

/// `out`'s values as the VM computes them.
fn on_the_vm(c: &Case) -> Vec<f64> {
    let shows: String = if c.out == "t" {
        "Show t.\n".to_string()
    } else {
        (1..=c.len).map(|i| format!("Show item {i} of {}.\n", c.out)).collect()
    };
    let vm = vm_outcome(&format!("{}{shows}", c.src));
    assert!(vm.error.is_none(), "{}: the VM runs it: {:?}", c.what, vm.error);
    vm.output.split_whitespace().map(|t| t.parse().expect("a number")).collect()
}

/// How a case ends when it is not wrong.
#[derive(PartialEq)]
enum Held {
    /// Lowered whole, the VM's values.
    Lowered,
    /// Refused for a precision or an engine the part does not have.
    Lacks,
}

/// The verdict on one case: held, or what is wrong.
fn verdict(c: &Case, target: &SpatialTarget) -> Result<Held, String> {
    let prog = match compile_program_to_spatial(&c.src, target).map_err(|why| format!("{why:?}")).and_then(|p| p.into_whole().map_err(|why| why.to_string())) {
        Ok(p) => p,
        Err(why) if why.contains("MissingPrecision") || why.contains("MissingCompute") => return Ok(Held::Lacks),
        Err(why) => return Err(format!("not lowered whole: {why}")),
    };
    let values = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).map_err(|e| format!("the evaluation faults: {e:?}"))?;
    let t = prog.tensors.iter().find(|t| t.name == c.out).ok_or_else(|| format!("no tensor `{}`", c.out))?;
    let got: Vec<f64> = match values.tensors.get(&t.id) {
        Some(Value::Float(v)) => v.clone(),
        Some(Value::Int(v)) => v.iter().map(|x| *x as f64).collect(),
        None => return Err(format!("the evaluation hands back no `{}`", c.out)),
    };
    let want = on_the_vm(c);
    if got.len() != want.len() {
        return Err(format!("{} values where the VM has {}", got.len(), want.len()));
    }
    let scale = want.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    let bound = if c.out == "t" { 0.0 } else { 0.02 * scale };
    let (i, worst) = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).enumerate().fold((0, 0.0f64), |w, (i, e)| if e > w.1 { (i, e) } else { w });
    if worst > bound {
        return Err(format!("value {i} is {} where the VM's is {} (bound {bound:.3e})", got[i], want[i]));
    }
    Ok(Held::Lowered)
}

/// Whether `target` has a float elementwise datapath: every map then lowers.
fn computes_floats(target: &SpatialTarget) -> bool {
    target.compute.iter().any(|c| match c {
        ComputeCap::Elementwise { precisions } => precisions.iter().any(|p| matches!(p, Precision::Bf16 | Precision::Fp16 | Precision::Fp32)),
        _ => false,
    })
}

fn every_row_op_at_every_width_is_the_vms_on(target: SpatialTarget) {
    let ws = widths(&target);
    assert!(ws.len() >= 5, "{}: the widths reach both sides of the page: {ws:?}", target.name);
    let mut wrong = Vec::new();
    let (mut lowered, mut lacks) = (0usize, 0usize);
    for tiles in &ws {
        let n = tiles * target.tile.cols;
        for c in cases(n, &target) {
            match verdict(&c, &target) {
                Ok(Held::Lowered) => lowered += 1,
                Ok(Held::Lacks) => lacks += 1,
                Err(why) => wrong.push(format!("{tiles} tiles ({n} values), {}: {why}", c.what)),
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{}: {} of {} cases are not the VM's:\n  {}",
        target.name,
        wrong.len(),
        wrong.len() + lowered + lacks,
        wrong.join("\n  ")
    );
    if computes_floats(&target) {
        assert!(
            lowered >= 5 * ws.len(),
            "{}: a float datapath lowers its five maps at every width; {lowered} cases lowered, {lacks} refused for what the part lacks",
            target.name
        );
    }
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_the_sn50() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::sn50_rdu());
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_the_sn40l() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::sn40l_rdu());
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_the_rtx_3060() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::rtx3060());
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_wormhole() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::wormhole());
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_blackhole() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::blackhole());
}

#[test]
fn every_row_op_at_every_width_is_the_vms_on_the_sky_fabric() {
    every_row_op_at_every_width_is_the_vms_on(SpatialTarget::sky_fabric(64, 64));
}

/// A `rows × cols` table with `count` rows written from row 2, a named position — a KV cache's append — and, `dense`,
/// read on as a table (the form a cache an attention reads takes): the table after the write.
fn row_write(rows: usize, cols: usize, count: usize, dense: bool) -> Case {
    let matrix = |name: &str, r: usize, c: usize, salt: usize| {
        let mut s = format!("Let {name} be a {r} by {c} tensor of Real.\n");
        for k in 0..r * c {
            let v = -0.75 + ((k * 7 + salt * 3 + k / 16 * 5) % 13) as f64 / 8.0;
            s.push_str(&format!("Set item {} of {name} to {v:?}.\n", k + 1));
        }
        s
    };
    let write = if count == 1 { "Set row P of T to X.\n" } else { "Set rows from P of T to X.\n" };
    let read = if dense { format!("{}Let Y be T matmul W.\n", matrix("W", cols, 16, 2)) } else { String::new() };
    Case {
        what: format!("{count} of {rows} rows of {cols}{}", if dense { ", read as a table" } else { "" }),
        src: format!("## Main\n{}{}Let P be 2.\n{write}{read}", matrix("T", rows, cols, 0), matrix("X", count, cols, 1)),
        out: "T",
        len: rows * cols,
    }
}

#[test]
fn a_row_write_into_a_table_past_a_site_is_the_vms() {
    // A row and a block of rows into tables of 8 positions of 128 up to 32 of 512: past an RTX 3060 block's memory once
    // the block of the rows a page holds is, though a site holds a block of the row.
    for target in [SpatialTarget::rtx3060(), SpatialTarget::sn50_rdu()] {
        let mut wrong = Vec::new();
        for (rows, cols, count) in [(8, 128, 1), (8, 128, 3), (16, 256, 7), (32, 512, 1)] {
            for dense in [false, true] {
                let c = row_write(rows, cols, count, dense);
                match verdict(&c, &target) {
                    Ok(Held::Lowered) => {}
                    Ok(Held::Lacks) => wrong.push(format!("{}: refused for what the part lacks", c.what)),
                    Err(why) => wrong.push(format!("{}: {why}", c.what)),
                }
            }
        }
        assert!(wrong.is_empty(), "{}: {} writes are not the VM's:\n  {}", target.name, wrong.len(), wrong.join("\n  "));
    }
}

#[test]
fn every_spatial_preset_is_held_here() {
    let held = ["sn50-rdu", "sn40l-rdu", "rtx3060", "wormhole_b0", "blackhole", "sky130-fabric"];
    for t in SpatialTarget::presets() {
        assert!(
            held.iter().any(|h| t.name.starts_with(h)),
            "{}: a preset no test of this lock holds — add its own `every_row_op_at_every_width_is_the_vms_on_…`",
            t.name
        );
    }
}
