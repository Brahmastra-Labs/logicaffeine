//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A HYPER-CONNECTED RESIDUAL'S LANES ARE NORMED EACH ON ITS OWN, EACH BY ITS OWN ROW OF THE WEIGHT.
//!
//! Qwen3.8-Flash-Next carries its residual as `hc_count` 4 lanes of the hidden width and RMS-norms each lane over its
//! own entries before it mixes them, by a weight as wide as all four (`hc_norm.weight`, 4·2560 entries): lane `l` of
//! every token takes row `l` of it. `Let Y be rmsnorm of H with weight W in L lanes` says so, for `H` an `R by L·C`
//! tensor and `W` one of `L·C` entries:
//!
//!     Y[t, l·C + c] = H[t, l·C + c] / √(mean_c' H[t, l·C + c']² + ε) · W[l·C + c]
//!
//!   • the VM and the tree-walker compute that formula, for one token and for several, on lanes whose weights differ;
//!   • the front end lifts it as the per-row norm of `R·L` rows of `C` whose weight holds `L` rows of `C`
//!     (`FusedRowSpec::weight_rows`), and a norm with one weight row (`with weight W`, `across H heads`) with 1; a
//!     hand-written norm is lifted only when it reads its input at its row's index and its weight at a column's or a
//!     lane's column's, so no backend computes one that reads elsewhere as a norm;
//!   • the tensor graph's evaluator computes it as the VM does, value for value;
//!   • the spatial chips compute it as the VM does: alone, as a section of a composition, and in the blocked form a row
//!     wider than a site takes (`tensor_graph_vjp_lock` holds its gradients, `cuda_fused_row_lock` the single-op CUDA
//!     kernel's values on the card); the blocked form loads each row by whole tiles, so a row norm in it whose rows do
//!     not start on tile boundaries — lanes or heads — is refused by name, never read from its neighbours' tiles;
//!   • every shim of a chip whose norm kernel reads one weight row refuses it by name — the CUDA lane at every knob
//!     point, the forward at each of its activations, the D5005 engine — and so does the SOL bridge, and no rule of
//!     the fuser takes it as a norm whose weight is one row;
//!   • every file of the compiler that reads a fused row's spec is named here with what it does with the weight's rows:
//!     a norm computed with the wrong row of its weight is a model computed wrong with no error.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::compile::{compile_to_cuda_tensor_op, compile_to_sol_solution, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::passes::from_qwen_ops::row_needs_blocking;
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::eval::{eval, Data};
use logicaffeine_compile::tensor_graph::fuse::norm;
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::link::resolve;
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use spatial_support::inputs_from_fills;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

/// Entry `c` of lane `l` of token `t` of `H`: each lane on a scale of its own, so a norm over the whole row is not a
/// norm over each lane.
fn h(t: usize, l: usize, c: usize) -> f64 {
    ((((t * 37 + l * 11 + c * 5) % 29) as f64 / 29.0) * 3.0 - 1.4) * (1.0 + l as f64)
}

/// Entry `c` of lane `l`'s row of the weight: every lane's row differs from every other's.
fn w(l: usize, c: usize) -> f64 {
    0.5 + ((l * 7 + c * 3) % 13) as f64 / 13.0 + 0.25 * l as f64
}

/// One lane norm: `R` tokens of `L` lanes `C` wide, at `eps`.
#[derive(Clone, Copy, Debug)]
struct Lanes {
    r: usize,
    l: usize,
    c: usize,
    eps: Option<f64>,
}

const fn lanes(r: usize, l: usize, c: usize) -> Lanes {
    Lanes { r, l, c, eps: None }
}

impl Lanes {
    fn width(self) -> usize {
        self.l * self.c
    }

    /// The clause binding `Y`.
    fn op(self) -> String {
        let eps = self.eps.map_or(String::new(), |e| format!(" with epsilon {e:?}"));
        format!("rmsnorm of H with weight W in {} lanes{eps}", self.l)
    }

    /// `H` and `W` declared and filled, then `Y` bound by `op`, then `tail`.
    fn program_with(self, op: &str, tail: &str) -> String {
        let n = self.width();
        let mut s = format!("## Main\nLet H be a {} by {n} tensor of Real.\n", self.r);
        for t in 0..self.r {
            for k in 0..n {
                s.push_str(&format!("Set item {} of H to {:?}.\n", t * n + k + 1, h(t, k / self.c, k % self.c)));
            }
        }
        s.push_str(&format!("Let W be a 1 by {n} tensor of Real.\n"));
        for k in 0..n {
            s.push_str(&format!("Set item {} of W to {:?}.\n", k + 1, w(k / self.c, k % self.c)));
        }
        s.push_str(&format!("Let Y be {op}.\n{tail}"));
        s
    }

    fn program(self) -> String {
        self.program_with(&self.op(), "")
    }

    /// The formula, in the order the sugar computes it.
    fn oracle(self) -> Vec<f64> {
        let eps = self.eps.unwrap_or(1e-6);
        let mut y = Vec::new();
        for t in 0..self.r {
            for l in 0..self.l {
                let mut ss = 0.0f64;
                for c in 0..self.c {
                    ss = ss + h(t, l, c) * h(t, l, c);
                }
                let d = (ss / self.c as f64 + eps).sqrt();
                for c in 0..self.c {
                    y.push(h(t, l, c) / d * w(l, c));
                }
            }
        }
        y
    }
}

/// `src` with each of `out`'s `n` entries shown.
fn shown(src: &str, out: &str, n: usize) -> String {
    let mut s = src.to_string();
    for i in 1..=n {
        s.push_str(&format!("Show item {i} of {out}.\n"));
    }
    s
}

/// `out`'s `n` entries as the VM computes them, the tree-walker agreeing.
fn on_the_vm(src: &str, out: &str, n: usize) -> Vec<f64> {
    let prog = shown(src, out, n);
    let vm = vm_outcome(&prog);
    assert!(vm.error.is_none(), "the VM runs the program: {:?}\n{prog}", vm.error);
    let tw = tw_outcome(&prog);
    assert!(tw.error.is_none(), "the tree-walker runs the program: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("a number")).collect()
}

const CASES: [Lanes; 5] = [lanes(1, 4, 3), lanes(2, 4, 3), lanes(3, 2, 5), lanes(2, 3, 4), lanes(1, 1, 6)];

#[test]
fn the_vm_and_the_tree_walker_compute_each_lane_by_its_own_weight_row() {
    let mut cases = CASES.to_vec();
    cases.push(Lanes { eps: Some(0.01), ..lanes(2, 4, 3) });
    for k in cases {
        let got = on_the_vm(&k.program(), "Y", k.r * k.width());
        let want = k.oracle();
        assert_eq!(got.len(), want.len(), "{k:?}: Y's size");
        for (i, (g, o)) in got.iter().zip(&want).enumerate() {
            assert!((g - o).abs() <= 1e-12 * o.abs().max(1.0), "{k:?}: Y[{i}] is {g} on the VM, the formula's {o}");
        }
    }
    // Not vacuous: normed in its lanes, the residual is not the residual normed over its whole row by the same weight.
    let k = lanes(2, 4, 3);
    let whole = on_the_vm(&k.program_with("rmsnorm of H with weight W", ""), "Y", k.r * k.width());
    let laned = on_the_vm(&k.program(), "Y", k.r * k.width());
    assert!(whole.iter().zip(&laned).any(|(a, b)| (a - b).abs() > 0.05), "lanes normed on their own move the result");
}

/// The fused row `Y` lifts as, with its operands.
fn lifted(src: &str) -> (logicaffeine_compile::tensor_recognize::FusedRowSpec, Vec<String>) {
    let rp = recognize_stages(src).expect("parses");
    assert!(rp.unrecognized.is_empty(), "the norm is lifted whole: {:?}", rp.unrecognized);
    let st = rp.stages.into_iter().find(|s| s.out == "Y").expect("a stage writes Y");
    match st.op {
        TensorOpSpec::FusedRow(f) => (f, st.ins),
        other => panic!("Y is lifted as {other:?}, not a fused row"),
    }
}

#[test]
fn the_front_end_lifts_the_lanes_as_the_rows_of_the_weight() {
    for k in CASES {
        let (f, ins) = lifted(&k.program());
        assert_eq!(
            (f.rows, f.cols, f.weight_rows, f.weight.is_some(), f.residual.is_some(), f.bias.is_some()),
            (k.r * k.l, k.c, k.l, true, false, false),
            "{k:?}: R·L rows of C, the weight L rows of C"
        );
        assert_eq!(ins, ["H", "W"], "{k:?}: H and W its operands");
    }
    let k = lanes(2, 4, 3);
    let (f, _) = lifted(&k.program_with("rmsnorm of H with weight W", ""));
    assert_eq!((f.rows, f.cols, f.weight_rows), (2, 12, 1), "one weight row for the whole row");
    let per_head = "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet G be a 1 by 4 tensor of Real.\nLet Y be rmsnorm of Q with weight G across 2 heads.\n";
    let (f, _) = lifted(per_head);
    assert_eq!((f.rows, f.cols, f.weight_rows), (2, 4, 1), "every head shares one weight row");
}

/// A hand-written per-row weighted norm of `H` (1 by 6, read as 2 rows of 3) into `Y`, reading `H` at `x_index` and `W`
/// (1 by 7: two lane rows of 3 and one entry past them, so every read below stays in range) at `w_index`, its row
/// counter `r` and column counter `j`.
fn hand_written(x_index: &str, w_index: &str) -> String {
    let mut s = String::from("## Main\nLet H be a 1 by 6 tensor of Real.\n");
    for k in 0..6 {
        s.push_str(&format!("Set item {} of H to {:?}.\n", k + 1, h(0, k / 3, k % 3)));
    }
    s.push_str("Let W be a 1 by 7 tensor of Real.\n");
    for k in 0..7 {
        s.push_str(&format!("Set item {} of W to {:?}.\n", k + 1, w(k / 3, k % 3)));
    }
    s.push_str(&format!(
        "Let mutable Y be a new Seq of Real.
Let mutable r be 1.
Let mutable ss be 0.0.
Let mutable i be 1.
Let mutable d be 0.0.
Let mutable j be 1.
While r is at most 2:
    Set ss to 0.0.
    Set i to 1.
    While i is at most 3:
        Set ss to ss + (item ((r - 1) * 3 + i) of H) * (item ((r - 1) * 3 + i) of H).
        Set i to i + 1.
    Set d to sqrt(ss / 3.0 + 0.000001).
    Set j to 1.
    While j is at most 3:
        Push (item ({x_index}) of H) / d * (item ({w_index}) of W) to Y.
        Set j to j + 1.
    Set r to r + 1.
"
    ));
    s
}

/// `Y` of a single-op program on the SN50, or the reason it is refused.
fn on_the_sn50(src: &str, out: &str) -> Result<Vec<f64>, String> {
    let prog = compile_program_to_spatial(src, &SpatialTarget::sn50_rdu())
        .map_err(|why| format!("{why:?}"))?
        .into_whole()
        .map_err(|why| why.to_string())?;
    Ok(every_tensor(&prog, out))
}

fn every_tensor(prog: &logicaffeine_compile::spatial::ir::SpatialProgram, out: &str) -> Vec<f64> {
    let values = eval_choreography_every_tensor(prog, &inputs_from_fills(prog)).expect("the program evaluates");
    let id = prog.tensors.iter().find(|t| t.name == out).unwrap_or_else(|| panic!("the program holds {out}")).id;
    match values.tensors.get(&id).unwrap_or_else(|| panic!("the evaluation hands back {out}")) {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// Every entry of `chip` within `frac` of the largest of `vm`.
fn close(what: &str, chip: &[f64], vm: &[f64], frac: f64) {
    assert_eq!(chip.len(), vm.len(), "{what}: the size");
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    for (i, (a, b)) in chip.iter().zip(vm).enumerate() {
        assert!((a - b).abs() <= frac * scale, "{what}: entry {i} is {a}, the VM's {b}\nchip {chip:?}\nvm   {vm:?}");
    }
}

#[test]
fn a_hand_written_norm_is_lifted_only_when_it_reads_its_rows() {
    // Read as the sugar reads them: lifted, by the CUDA kernel and by the spatial chip, and computed as the VM does.
    for (x, wi, what) in [
        ("(r - 1) * 3 + j", "j", "one weight row"),
        ("(r - 1) * 3 + j", "((r - 1) % 2) * 3 + j", "a weight row per lane"),
    ] {
        let src = hand_written(x, wi);
        let vm = on_the_vm(&src, "Y", 6);
        let cu = compile_to_cuda_tensor_op(&src).unwrap_or_else(|why| panic!("{what}: the CUDA backend lowers it: {why}"));
        assert!(cu.contains("fused_row"), "{what}: lowered as the fused-row kernel");
        let chip = on_the_sn50(&src, "Y").unwrap_or_else(|why| panic!("{what}: the SN50 lowers it: {why}"));
        close(what, &chip, &vm, 0.02);
    }
    // Read anywhere else: no backend computes it as a norm.
    for (x, wi, what) in [
        ("(r - 1) * 3 + j", "r", "a weight read by the row"),
        ("(r - 1) * 3 + j", "j + 1", "a weight read a column over"),
        ("(r - 1) * 3 + j", "((r - 1) % 2) * 2 + j", "lane rows strided other than the row's width"),
        ("(r - 1) * 3 + j", "((r - 1) % 2) * 3 + j + 1", "a lane's row read a column over"),
        ("j", "j", "every row reading the first row's input"),
        ("(r - 1) * 2 + j", "j", "an input strided other than the row's width"),
    ] {
        let src = hand_written(x, wi);
        let vm = on_the_vm(&src, "Y", 6);
        if let Ok(cu) = compile_to_cuda_tensor_op(&src) {
            assert!(!cu.contains("fused_row"), "{what}: the CUDA backend lowers it as a norm:\n{cu}");
        }
        if let Ok(chip) = on_the_sn50(&src, "Y") {
            close(what, &chip, &vm, 0.02);
        }
    }
}

#[test]
fn the_graph_evaluator_computes_the_lanes_as_the_vm_does() {
    for k in CASES {
        let src = k.program();
        let g = TensorGraph::lift(&recognize_stages(&src).expect("parses"), None, None, &|_| Storage::Values)
            .unwrap_or_else(|why| panic!("{k:?} lifts: {why:?}"));
        let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|why| panic!("the evaluator computes {k:?}: {why}"));
        let Some(Data::Real(values)) = env.get("Y") else { panic!("{k:?}: the evaluator has no real `Y`") };
        let printed: Vec<String> = on_the_vm(&src, "Y", k.r * k.width()).iter().map(|x| format!("{x}")).collect();
        let evaluated: Vec<String> = values.iter().map(|x| format!("{x}")).collect();
        assert_eq!(evaluated, printed, "{k:?}: the evaluator's `Y` is the VM's, value for value");
    }
}

#[test]
fn the_spatial_chips_compute_the_lanes_as_the_vm_does() {
    let sn50 = SpatialTarget::sn50_rdu();
    let blocked = [lanes(1, 4, 96), lanes(1, 2, 256)];
    for k in &blocked {
        assert!(
            row_needs_blocking(k.width(), &sn50) && !row_needs_blocking(k.c, &sn50) && k.c % sn50.tile.cols == 0,
            "{k:?}: on the SN50 the residual needs the blocked form, one lane does not, and each lane is whole tiles"
        );
    }
    for target in [sn50.clone(), SpatialTarget::wormhole()] {
        let cases: Vec<Lanes> = CASES.iter().copied().chain(if target.name == sn50.name { blocked.to_vec() } else { Vec::new() }).collect();
        for k in cases {
            let n = k.r * k.width();
            let vm = on_the_vm(&k.program(), "Y", n);
            // Alone: the one op the program is.
            let prog = compile_program_to_spatial(&k.program(), &target)
                .unwrap_or_else(|why| panic!("{k:?} alone lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            close(&format!("{k:?} alone on {}", target.name), &every_tensor(&prog, "Y"), &vm, 0.02);
            // A section: the norm and the sum after it, composed — over several tokens the sum is a dense table, whose
            // layout the composition bridges to the norm's flat rows.
            let src = k.program_with(&k.op(), "Let Z be Y plus Y.\n");
            let prog = compile_program_to_spatial(&src, &target)
                .unwrap_or_else(|why| panic!("{k:?} as a section lowers for {}: {why:?}", target.name))
                .into_whole()
                .unwrap_or_else(|why| panic!("{why}"));
            let doubled: Vec<f64> = vm.iter().map(|y| y + y).collect();
            close(&format!("{k:?}'s sum on {}", target.name), &every_tensor(&prog, "Z"), &doubled, 0.02);
        }
    }
    // Not vacuous: on the chip, each lane's own weight row moves the result past the datapath's error.
    let k = lanes(1, 4, 96);
    let laned = on_the_sn50(&k.program(), "Y").expect("the lanes lower");
    let whole = on_the_vm(&k.program_with("rmsnorm of H with weight W", ""), "Y", k.width());
    let scale = whole.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    assert!(laned.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 0.05 * scale), "the lanes move the chip's result");
}

/// `Q` (1 by `heads·hd`) normed across its `heads` heads by `G` (1 by `hd`), then summed with itself: a per-head norm
/// as a section of a composition.
fn per_head(heads: usize, hd: usize) -> String {
    let n = heads * hd;
    let mut s = format!("## Main\nLet Q be a 1 by {n} tensor of Real.\n");
    for k in 0..n {
        s.push_str(&format!("Set item {} of Q to {:?}.\n", k + 1, h(0, k / hd, k % hd)));
    }
    s.push_str(&format!("Let G be a 1 by {hd} tensor of Real.\n"));
    for c in 0..hd {
        s.push_str(&format!("Set item {} of G to {:?}.\n", c + 1, w(0, c)));
    }
    s.push_str(&format!("Let Y be rmsnorm of Q with weight G across {heads} heads.\nLet Z be Y plus Y.\n"));
    s
}

#[test]
fn a_blocked_row_norm_whose_rows_do_not_start_on_tiles_is_refused_by_name() {
    let sn50 = SpatialTarget::sn50_rdu();
    let refused = |src: &str, what: &str| {
        let why = match compile_program_to_spatial(src, &sn50) {
            Err(why) => format!("{why:?}"),
            Ok(p) => match p.into_whole() {
                Err(why) => why.to_string(),
                Ok(_) => panic!("{what}: the SN50 lowers rows read off their tiles"),
            },
        };
        assert!(why.contains("tile boundaries"), "{what}: the refusal names the rows' tiles: {why}");
    };
    // Lanes 100 wide on 32-wide tiles: lane 1 starts 4 entries into tile 3.
    let k = lanes(1, 4, 100);
    assert!(row_needs_blocking(k.width(), &sn50) && !row_needs_blocking(k.c, &sn50), "the blocked form");
    refused(&k.program(), "lanes alone");
    refused(&k.program_with(&k.op(), "Let Z be Y plus Y.\n"), "lanes as a section");
    refused(&per_head(4, 100), "heads 100 wide");
    // Two heads of 250 pad to the tensor's own 16 tiles, so nothing overlaps: the second head's tiles hold its last 244
    // values and none of its first six, and nothing but this refusal stops it.
    let k = lanes(1, 2, 250);
    assert!(row_needs_blocking(k.width(), &sn50) && !row_needs_blocking(k.c, &sn50), "the blocked form");
    refused(&k.program_with(&k.op(), "Let Z be Y plus Y.\n"), "two lanes of 250 as a section");
    refused(&per_head(2, 250), "two heads of 250");
    // Heads on tile boundaries are computed in the blocked form, as the VM computes them.
    let src = per_head(4, 96);
    assert!(row_needs_blocking(4 * 96, &sn50) && !row_needs_blocking(96, &sn50) && 96 % sn50.tile.cols == 0, "the blocked form");
    let vm = on_the_vm(&src, "Y", 4 * 96);
    let prog = compile_program_to_spatial(&src, &sn50).expect("heads on tiles lower").into_whole().unwrap_or_else(|why| panic!("{why}"));
    close("heads 96 wide", &every_tensor(&prog, "Y"), &vm, 0.02);
}

#[test]
fn a_flat_row_norm_of_more_rows_than_a_tile_runs_a_tile_of_rows_at_a_time() {
    // A page holds a tile's rows, so the rows' sums and scales run a tile's rows at a time: 40 heads of 4 on the SN50's
    // 32-row tiles (32 and then 8), 33 of 7 (32 and then 1), and 20 of 8 on the RTX 3060's 16-row tiles (16 and then 4).
    for (target, heads, hd) in [(SpatialTarget::sn50_rdu(), 40, 4), (SpatialTarget::sn50_rdu(), 33, 7), (SpatialTarget::rtx3060(), 20, 8)] {
        let what = format!("{heads} heads of {hd} on {}", target.name);
        assert!(heads > target.tile.rows && !row_needs_blocking(heads * hd, &target), "{what}: the flat form, past a tile's rows");
        let src = per_head(heads, hd);
        let vm = on_the_vm(&src, "Y", heads * hd);
        let prog = compile_program_to_spatial(&src, &target)
            .unwrap_or_else(|why| panic!("{what}: lowers: {why:?}"))
            .into_whole()
            .unwrap_or_else(|why| panic!("{what}: lowers whole: {why}"));
        close(&what, &every_tensor(&prog, "Y"), &vm, 0.02);
    }
}

#[test]
fn a_blocked_row_norm_whose_rows_straddle_tiles_is_computed_a_tile_aligned_group_at_a_time() {
    // Rows `cols` wide start on a tile once every `tile / gcd(cols, tile)` rows: a group of that many rows is whole
    // tiles, so the blocked form loads it as one and norms each of its rows by its own sum.
    let sn50 = SpatialTarget::sn50_rdu();
    let tc = sn50.tile.cols;
    for (heads, hd, group) in [(32, 24, 4), (8, 40, 4), (16, 48, 2), (64, 8, 4)] {
        let what = format!("{heads} heads of {hd}");
        assert!(row_needs_blocking(heads * hd, &sn50) && hd % tc != 0, "{what}: the blocked form, rows off their tiles");
        assert_eq!(group * hd % tc, 0, "{what}: {group} rows are whole tiles");
        assert_eq!(heads % group, 0, "{what}: whole groups");
        let src = per_head(heads, hd);
        let vm = on_the_vm(&src, "Y", heads * hd);
        let prog = compile_program_to_spatial(&src, &sn50)
            .unwrap_or_else(|why| panic!("{what}: lowers: {why:?}"))
            .into_whole()
            .unwrap_or_else(|why| panic!("{what}: lowers whole: {why}"));
        close(&what, &every_tensor(&prog, "Y"), &vm, 0.02);
        let z: Vec<f64> = vm.iter().map(|y| y + y).collect();
        close(&format!("{what}, as a section"), &every_tensor(&prog, "Z"), &z, 0.02);
    }
}

/// The shims of the chips whose norm kernels read one weight row: the CUDA lane at every knob point, the forward at
/// each of its activations, the D5005 engine for both of its tiny models.
fn shims_with_one_weight_row() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for fuse_quant in [true, false] {
            for batch in [false, true] {
                let knobs = CudaKnobs { activations, fuse_quant, ..CudaKnobs::default() };
                out.push(Box::new(LaneShim { knobs, batch, held: Vec::new() }));
            }
        }
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    for tiny in [Tiny::Qwen35, Tiny::Qwen3] {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny.spec(24))), numerics: EngineNumerics::precise() }));
    }
    out
}

fn graph_of(src: &str) -> TensorGraph {
    TensorGraph::lift(&recognize_stages(src).expect("parses"), None, None, &|_| Storage::Values).expect("lifts")
}

#[test]
fn every_shim_whose_norm_reads_one_weight_row_refuses_the_lanes_by_name() {
    let k = lanes(1, 4, 3);
    let g = graph_of(&k.program());
    assert!(g.ops.iter().any(|o| o.kind() == "FusedRow"), "the lanes lift as a fused row");
    for shim in shims_with_one_weight_row() {
        let unresolved = resolve(&g, shim.as_ref()).expect_err(&format!("{} refuses the lanes", shim.name()));
        let refused = unresolved
            .iter()
            .find(|u| u.kind == "FusedRow")
            .unwrap_or_else(|| panic!("{} refuses the lane norm: {unresolved:?}", shim.name()));
        assert!(refused.why.contains("lanes"), "{} names the lanes it refuses: {}", shim.name(), refused.why);
    }
    // Not vacuous: the CUDA lane and the forward take the norm with one weight row.
    let plain = graph_of(&k.program_with("rmsnorm of H with weight W", ""));
    for shim in shims_with_one_weight_row().into_iter().filter(|s| !s.name().contains("engine")) {
        resolve(&plain, shim.as_ref()).unwrap_or_else(|why| panic!("{} takes one weight row: {why:?}", shim.name()));
    }
    for target in [SpatialTarget::sn50_rdu(), SpatialTarget::wormhole()] {
        let shim = SpatialShim { target: Box::leak(Box::new(target)) };
        resolve(&g, &shim).unwrap_or_else(|why| panic!("{} resolves the lanes: {why:?}", shim.name()));
    }
    resolve(&g, &ReferenceShim).unwrap_or_else(|why| panic!("the reference resolves the lanes: {why:?}"));
}

#[test]
fn no_fused_norm_takes_the_lanes() {
    // A fused norm's weight is one row, and every rule that folds a norm into its neighbours reads it as one: the lanes
    // stay an op of their own, which every chip either computes or refuses by name.
    let k = lanes(1, 4, 3);
    let fused_norms = |src: &str| graph_of(src).ops.iter().filter(|o| o.kind() == "FusedRow").map(|o| norm(o).is_some()).collect::<Vec<_>>();
    assert_eq!(fused_norms(&k.program()), [false], "the lanes are no fused norm");
    assert_eq!(fused_norms(&k.program_with("rmsnorm of H with weight W", "")), [true], "one weight row is");
}

#[test]
fn the_sol_bridge_refuses_the_lanes_by_name() {
    let k = lanes(2, 4, 3);
    let refusal = compile_to_sol_solution(&k.program(), "lane_rmsnorm", &["H", "W"], "Y").expect_err("the SOL kernel reads one weight row");
    assert!(refusal.contains("lanes"), "the SOL bridge names the lanes it refuses: {refusal}");
    compile_to_sol_solution(&k.program_with("rmsnorm of H with weight W", ""), "rmsnorm", &["H", "W"], "Y")
        .expect("the SOL bridge takes one weight row");
}

/// Every file of the compiler that reads a fused row's spec, and what it does with the weight's rows. A file that lifts
/// or computes a fused row, or refuses what it does not compute, names `weight_rows`; one that only routes a fused row
/// by its kind or its size does not need to.
const CONSUMERS: &[(&str, &str)] = &[
    ("tensor_recognize/mod.rs", "lifts"),
    ("tensor_graph/front.rs", "routes"),
    ("tensor_graph/graph.rs", "routes"),
    ("tensor_graph/numerics.rs", "routes"),
    ("spatial/lower/rdu_lanes.rs", "routes"),
    ("codegen_cuda/matrix.rs", "routes"),
    ("tensor_graph/eval.rs", "computes"),
    ("tensor_graph/vjp.rs", "computes"),
    ("spatial/pipeline.rs", "computes"),
    ("spatial/link.rs", "computes"),
    ("spatial/passes/from_qwen_ops.rs", "computes"),
    ("codegen_cuda/fused_row.rs", "computes"),
    ("codegen_cpu/link.rs", "computes"),
    ("spatial/passes/from_tensor_op.rs", "refuses"),
    ("tensor_graph/fuse.rs", "refuses"),
    ("codegen_cuda/forward_shim.rs", "refuses"),
    ("codegen_cuda/qwen_emit.rs", "refuses"),
    ("codegen_cuda/lane_link.rs", "refuses"),
    ("codegen_cuda/lane_shim.rs", "refuses"),
    ("codegen_cuda/solbench.rs", "refuses"),
    ("compile.rs", "refuses"),
    ("spatial/decode_engine/lower_graph.rs", "refuses"),
    ("spatial/decode_engine/shim.rs", "refuses"),
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_consumer_of_a_fused_row_is_held_to_its_weight_rows() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let reads = |text: &str| {
        text.contains("FusedRowSpec")
            || text.contains("TensorOpSpec::FusedRow(")
            || text.contains(" T::FusedRow(")
            || text.contains("recognize_fused_row(")
    };
    let found: BTreeSet<String> = files
        .iter()
        .filter(|p| reads(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().to_string())
        .collect();
    let named: BTreeSet<String> = CONSUMERS.iter().map(|(f, _)| f.to_string()).collect();
    assert_eq!(found, named, "the files that read a fused row's spec are the ones this census names");
    for (file, role) in CONSUMERS {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        if matches!(*role, "lifts" | "computes" | "refuses") {
            assert!(text.contains("weight_rows"), "{file} {role} a fused row, so it reads its weight's rows: it never names `weight_rows`");
        }
    }
}
