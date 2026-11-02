//! spatial_moe_options_lock — gpt-oss's, DeepSeek's and Llama 4's expert mixtures on a spatial chip. The router's
//! bias, the experts' gate, up and down biases, the clamped SwiGLU (`using a swiglu clamped at L with slope A`), the
//! weighting without renormalizing (`without renormalizing`), the sigmoid scores (`by sigmoid`) and the weights scaling
//! the experts' inputs (`weighting the inputs`), and DeepSeek-V3's choice — the selection bias (`biased by Bs`), the
//! group limit (`within the top G of N groups`), the routed scale (`scaled by S`) and the renormalizing sum clamped at
//! 2⁻¹⁴ — each lower onto the SN50, alone and together, and compute what the VM computes. A score that is NaN is never
//! routed ahead of a number, as the desugar's scan never picks one, and a mixture whose renormalized weights scale its
//! inputs is refused by name.
//!
//! Both forms of the mixture are covered: the FLAT form (widths not in whole tiles: one routing section, each round
//! moving its chosen expert's run of every expert table — the whole tiles around the run, read at its offset — or each
//! table once where that is fewer tiles) and the BLOCKED form (widths in whole tiles: the chosen experts' weights and
//! bias rows move by tile gathers addressed by the pick). The flat form's tiles moved are counted, every expert of a
//! gathered table is read from its own run, and mixtures whose tables a site could never hold whole — Llama 4's tiny
//! one on the fp32 twin, 64 experts on either chip — lower. Each case is held to the program-path ledger's two-part
//! rule, as `spatial_moe_real_width_lock`'s are:
//!
//!   • WIRING — the program on an fp32 twin of the chip is the VM's within a relative 2⁻¹⁶: a bias read from the wrong
//!     expert's row, a clamp on the wrong side, or a division the router does not ask for moves it by far more;
//!   • CHIP PRECISION — the bf16 program is the VM's within `sections + 1` bf16 ulps of the block's magnitude
//!     `Σ_r w_r·(Σ_i |Wd[o, i]·h_r[i]| + |Bd[o]|)`.
//!
//! Every input is a multiple of 1/64 (exact in bf16), every case's router decides its experts unambiguously, the VM
//! agrees with an f64 reference written out here, and each option moves that reference on the case's inputs.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_moe_options_lock`

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, eval_choreography_traced_with, Scheduler, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A chip with fp32 beside bf16 on the matmul, pointwise and row units.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// A value in `[-1, 1]` in steps of 1/64.
fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 129) as f64 - 64.0) / 64.0
}

fn dense(n: usize, salt: usize) -> Vec<f64> {
    (0..n).map(|p| val(p * 31 + salt)).collect()
}

/// An expert weight: dense where a row is a few inputs long, about one in three non-zero (spread over every expert,
/// row and chunk) where it is tiles long, so a row's sum stays the size of its entries.
fn weight(n: usize, row: usize, salt: usize) -> Vec<f64> {
    if row < 32 {
        return dense(n, salt);
    }
    (0..n).map(|p| if (p.wrapping_mul(2654435761).wrapping_add(salt * 97) >> 5) % 3 == 0 { val(p * 17 + salt) } else { 0.0 }).collect()
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    d: usize,
    dff: usize,
    ne: usize,
    k: usize,
}

/// The mixture's options: the four biases, the clamped activation's `(L, A)`, whether the kept weights are
/// renormalized, whether the experts are scored by their logits' sigmoids, whether a kept weight scales its expert's
/// input, whether the experts are chosen by their scores plus a selection bias, the group limit `(G, N)`, and the routed
/// scale.
#[derive(Clone, Copy, Debug)]
struct Opts {
    router_bias: bool,
    gate_bias: bool,
    up_bias: bool,
    down_bias: bool,
    clamp: Option<(f64, f64)>,
    renormalize: bool,
    sigmoid: bool,
    inputs: bool,
    bias: bool,
    groups: Option<(usize, usize)>,
    scale: Option<f64>,
}

const PLAIN: Opts = Opts {
    router_bias: false,
    gate_bias: false,
    up_bias: false,
    down_bias: false,
    clamp: None,
    renormalize: true,
    sigmoid: false,
    inputs: false,
    bias: false,
    groups: None,
    scale: None,
};
/// A clamp the cases' gates and ups cross: `L` well inside their range.
const CLAMP: Option<(f64, f64)> = Some((0.125, 1.702));
/// gpt-oss: every bias and the clamped gate, the kept weights renormalized.
const GPT_OSS: Opts = Opts { router_bias: true, gate_bias: true, up_bias: true, down_bias: true, clamp: CLAMP, ..PLAIN };

/// Each option alone, then gpt-oss's mixture and every option together.
fn option_sets() -> Vec<(&'static str, Opts)> {
    vec![
        ("the router's bias", Opts { router_bias: true, ..PLAIN }),
        ("the gate bias", Opts { gate_bias: true, ..PLAIN }),
        ("the up bias", Opts { up_bias: true, ..PLAIN }),
        ("the down bias", Opts { down_bias: true, ..PLAIN }),
        ("the clamped gate", Opts { clamp: CLAMP, ..PLAIN }),
        ("without renormalizing", Opts { renormalize: false, ..PLAIN }),
        ("gpt-oss's mixture", GPT_OSS),
        ("every option", Opts { renormalize: false, ..GPT_OSS }),
    ]
}

/// Llama 4: the experts scored by sigmoids, the kept weights not renormalized, each scaling its expert's input.
const LLAMA4: Opts = Opts { sigmoid: true, inputs: true, renormalize: false, ..PLAIN };

/// The routings: the sigmoid scores renormalized and not, the weights scaling the inputs alone, Llama 4's mixture, and
/// Llama 4's with every bias and the clamp.
fn routing_sets() -> Vec<(&'static str, Opts)> {
    vec![
        ("scored by sigmoid", Opts { sigmoid: true, ..PLAIN }),
        ("scored by sigmoid without renormalizing", Opts { sigmoid: true, renormalize: false, ..PLAIN }),
        ("weighting the inputs", Opts { inputs: true, renormalize: false, ..PLAIN }),
        ("Llama 4's mixture", LLAMA4),
        ("Llama 4's with every bias and the clamp", Opts { sigmoid: true, inputs: true, renormalize: false, ..GPT_OSS }),
    ]
}

/// DeepSeek-V3's choice: sigmoid scores, chosen by each score plus its selection bias among the top two of four groups,
/// the kept scores renormalized and scaled.
const DEEPSEEK_V3: Opts = Opts { sigmoid: true, bias: true, groups: Some((2, 4)), scale: Some(2.5), ..PLAIN };

/// The choices: each of DeepSeek-V3's clauses alone, a softmax limited to groups, DeepSeek-V3's, and DeepSeek-V3's with
/// every bias and the clamp.
fn choice_sets() -> Vec<(&'static str, Opts)> {
    vec![
        ("biased in its choice", Opts { sigmoid: true, bias: true, ..PLAIN }),
        ("within the top groups", Opts { sigmoid: true, groups: Some((2, 4)), ..PLAIN }),
        ("scaled", Opts { scale: Some(2.5), ..PLAIN }),
        ("a softmax within the top groups", Opts { groups: Some((2, 4)), ..PLAIN }),
        ("DeepSeek-V3's choice", DEEPSEEK_V3),
        ("DeepSeek-V3's with every bias and the clamp", Opts { router_bias: true, gate_bias: true, up_bias: true, down_bias: true, clamp: CLAMP, ..DEEPSEEK_V3 }),
    ]
}

struct Case {
    src: String,
    /// Per `Show`: the f64 reference and the magnitude it is summed from.
    reference: Vec<(f64, f64)>,
    /// Whether the clamp bit: a kept expert's gate above `L` and an up outside `[−L, L]`.
    clamped: (bool, bool),
}

fn declare(name: &str, rows: usize, cols: usize, v: &[f64]) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for (p, x) in v.iter().enumerate() {
        if *x != 0.0 {
            s += &format!("Set item {} of {name} to {:?}.\n", p + 1, x);
        }
    }
    s
}

/// A shape's inputs: `X`, the router `Wr` and its bias `Br`, and the experts' tables.
struct Data {
    x: Vec<f64>,
    wr: Vec<f64>,
    wg: Vec<f64>,
    wu: Vec<f64>,
    wd: Vec<f64>,
    br: Vec<f64>,
    bg: Vec<f64>,
    bu: Vec<f64>,
    bd: Vec<f64>,
    /// The selection bias.
    bs: Vec<f64>,
    /// The router's logits without its bias.
    plain: Vec<f64>,
}

fn data(m: Shape) -> Data {
    let Shape { d, dff, ne, .. } = m;
    let x = dense(d, 1);
    let wr = dense(d * ne, 2);
    let (wg, wu, wd) = (weight(ne * dff * d, d, 3), weight(ne * dff * d, d, 4), weight(ne * d * dff, dff, 5));
    let (mut br, bg, bu, bd) = (dense(ne, 6), dense(ne * dff, 7), dense(ne * dff, 8), dense(ne * d, 9));
    let plain: Vec<f64> = (0..ne).map(|e| (0..d).map(|t| x[t] * wr[t * ne + e]).sum::<f64>()).collect();
    // The router's bias lifts the expert the plain router ranks last to half a logit above the first (in steps of 1/64),
    // so it changes the choice and keeps the rest of the order.
    let (lo, hi) = plain.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), l| (lo.min(*l), hi.max(*l)));
    let last = (0..ne).min_by(|a, b| plain[*a].partial_cmp(&plain[*b]).unwrap()).expect("experts");
    br[last] = ((hi - lo + 0.5) * 64.0).ceil() / 64.0;
    let bs = dense(ne, 10);
    Data { x, wr, wg, wu, wd, br, bg, bu, bd, bs, plain }
}

/// The renormalizing sum's floor, 2⁻¹⁴, as llama.cpp clamps it.
const FLOOR: f64 = 6.103515625e-5;

/// The experts a mixture keeps, in order, from its scores `p` (each with its rounding bound `err`): chosen by each score
/// plus its selection bias where the options have one, among the experts of the top `G` groups where they limit the
/// groups — each group scored by its two best selection scores — every choice decided beyond the chip's rounding.
fn chosen(m: Shape, o: Opts, dt: &Data, p: &[f64], err: &[f64]) -> Vec<usize> {
    let Shape { ne, k, .. } = m;
    let sel: Vec<f64> = (0..ne).map(|e| p[e] + on(o.bias, dt.bs[e])).collect();
    let serr: Vec<f64> = (0..ne).map(|e| err[e] + 2f64.powi(-24) * sel[e].abs()).collect();
    let margin = |a: f64, ea: f64, eb: f64| 4.0 * (ea + eb) + 2f64.powi(-20) * a.abs().max(1.0);
    let mut allowed: Vec<usize> = (0..ne).collect();
    if let Some((kept, count)) = o.groups {
        let per = ne / count;
        let two_best = |g: usize| {
            let mut idx: Vec<usize> = (g * per..(g + 1) * per).collect();
            idx.sort_by(|a, b| sel[*b].partial_cmp(&sel[*a]).unwrap().then(a.cmp(b)));
            idx.iter().take(2).fold((0.0, 0.0), |(v, e), &i| (v + sel[i], e + serr[i]))
        };
        let scores: Vec<(f64, f64)> = (0..count).map(two_best).collect();
        let mut ranked: Vec<usize> = (0..count).collect();
        ranked.sort_by(|a, b| scores[*b].0.partial_cmp(&scores[*a].0).unwrap().then(a.cmp(b)));
        if kept < count {
            let (a, b) = (scores[ranked[kept - 1]], scores[ranked[kept]]);
            assert!(a.0 - b.0 > margin(a.0, a.1, b.1), "{m:?} {o:?}: the groups are ambiguous: {} and {}", a.0, b.0);
        }
        allowed = ranked[..kept].iter().flat_map(|g| g * per..(g + 1) * per).collect();
    }
    allowed.sort_by(|a, b| sel[*b].partial_cmp(&sel[*a]).unwrap().then(a.cmp(b)));
    for w in allowed.windows(2).take(k) {
        let (a, b) = (w[0], w[1]);
        assert!(sel[a] - sel[b] > margin(sel[a], serr[a], serr[b]), "{m:?} {o:?}: the choice is ambiguous between experts {a} and {b}");
    }
    allowed[..k].to_vec()
}

fn on(b: bool, v: f64) -> f64 {
    if b {
        v
    } else {
        0.0
    }
}

/// Expert `e`'s output on `X` scaled by `w_in`, before its router weight: per output, the value, the magnitude
/// `Σ_i |Wd[o, i]·h[i]| + |Bd[o]|` it is summed from, and whether the clamp bit on the gate and on the up.
fn expert(m: Shape, o: Opts, dt: &Data, e: usize, w_in: f64) -> (Vec<f64>, Vec<f64>, (bool, bool)) {
    let Shape { d, dff, .. } = m;
    let Data { x, wg, wu, wd, bg, bu, bd, .. } = dt;
    let mut clamped = (false, false);
    let h: Vec<f64> = (0..dff)
        .map(|r| {
            let g = w_in * (0..d).map(|t| x[t] * wg[(e * dff + r) * d + t]).sum::<f64>() + on(o.gate_bias, bg[e * dff + r]);
            let u = w_in * (0..d).map(|t| x[t] * wu[(e * dff + r) * d + t]).sum::<f64>() + on(o.up_bias, bu[e * dff + r]);
            match o.clamp {
                Some((l, a)) => {
                    clamped.0 |= g > l;
                    clamped.1 |= u.abs() > l;
                    let gc = g.min(l);
                    gc / (1.0 + (-a * gc).exp()) * (u.clamp(-l, l) + 1.0)
                }
                None => g / (1.0 + (-g).exp()) * u,
            }
        })
        .collect();
    let (mut y, mut mag) = (vec![0.0; d], vec![0.0; d]);
    for c in 0..d {
        let row = (e * d + c) * dff;
        let b = on(o.down_bias, bd[e * d + c]);
        y[c] = (0..dff).map(|i| wd[row + i] * h[i]).sum::<f64>() + b;
        mag[c] = (0..dff).map(|i| (wd[row + i] * h[i]).abs()).sum::<f64>() + b.abs();
    }
    (y, mag, clamped)
}

/// [`data`] with the router reading the one input of largest magnitude, its weights making the experts' logits fall in
/// four groups of `ne / 4`: the first group's first expert the best of all beside experts far below, every other
/// group's two best close behind it, each group a little below the one before — so the top two of four groups leave
/// out the best expert, and a group limit changes the choice. The logits sit at least 2/64 apart; the router's bias
/// lifts every odd expert by 1/64, so it moves them without making a tie; the selection bias lifts the last expert
/// alone by a quarter: into the experts chosen, and its group past every other. Every weight and bias is a multiple of
/// 1/64, as every input is.
fn grouped_data(m: Shape) -> Data {
    let Shape { d, ne, .. } = m;
    let mut dt = data(m);
    let per = ne / 4;
    let t = (0..d).max_by(|a, b| dt.x[*a].abs().partial_cmp(&dt.x[*b].abs()).unwrap().then(b.cmp(a))).expect("inputs");
    let w = |e: usize| match (e / per, e % per) {
        (0, 0) => 1.5,
        (0, _) => -3.0,
        (g, j) => 1.375 - 2.0 * (g - 1) as f64 / 64.0 - 8.0 * j as f64 / 64.0,
    };
    let sign = dt.x[t].signum();
    dt.wr = (0..d * ne).map(|i| if i / ne == t { sign * w(i % ne) } else { 0.0 }).collect();
    dt.plain = (0..ne).map(|e| (0..d).map(|u| dt.x[u] * dt.wr[u * ne + e]).sum::<f64>()).collect();
    dt.br = (0..ne).map(|e| (e % 2) as f64 / 64.0).collect();
    dt.bs = (0..ne).map(|e| if e + 1 == ne { 0.25 } else { 0.0 }).collect();
    dt
}

/// A shape's inputs when its router row is wider than an accumulator page (`dense` repeats every 129 positions, so its
/// router ties experts 129 apart): the router reads only the input of largest magnitude, through `K + 1` experts spread
/// over the row's blocks (the first in its last block) whose weights fall from `1` by eighths, `−1` for every other, so
/// each kept logit clears the next by an eighth of that input; the router's bias lifts an unkept expert above them all,
/// and the selection bias, `−1` for every expert, lifts the `K + 1`-th by a quarter past the `K`-th — every selection
/// score below zero, so a candidate slot no block fills can never be chosen in its place.
fn wide_data(m: Shape) -> Data {
    let Shape { d, ne, k, .. } = m;
    let mut dt = data(m);
    let t = (0..d).max_by(|a, b| dt.x[*a].abs().partial_cmp(&dt.x[*b].abs()).unwrap().then(b.cmp(a))).expect("inputs");
    let sign = dt.x[t].signum();
    let spread: Vec<usize> = (0..=k).map(|j| (ne - 1 - j * 37) % ne).collect();
    let w = |e: usize| spread.iter().position(|&s| s == e).map_or(-1.0, |j| 1.0 - j as f64 / 8.0);
    dt.wr = (0..d * ne).map(|i| if i / ne == t { sign * w(i % ne) } else { 0.0 }).collect();
    dt.plain = (0..ne).map(|e| (0..d).map(|u| dt.x[u] * dt.wr[u * ne + e]).sum::<f64>()).collect();
    let (lo, hi) = dt.plain.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), l| (lo.min(*l), hi.max(*l)));
    let lift = (0..ne).find(|e| !spread.contains(e)).expect("an unkept expert");
    dt.br = (0..ne).map(|e| if e == lift { ((hi - lo + 0.5) * 64.0).ceil() / 64.0 } else { 0.0 }).collect();
    dt.bs = (0..ne).map(|e| if e == spread[k] { -0.75 } else { -1.0 }).collect();
    dt
}

fn case(m: Shape, o: Opts) -> Case {
    case_of(m, o, data(m))
}

/// The case of shape `m` with options `o`, on the inputs `dt`.
fn case_of(m: Shape, o: Opts, dt: Data) -> Case {
    let Shape { d, dff, ne, k } = m;
    let Data { x, wr, wg, wu, wd, br, bg, bu, bd, plain, .. } = &dt;
    let logits: Vec<f64> = (0..ne).map(|e| plain[e] + on(o.router_bias, br[e])).collect();
    let p: Vec<f64> = if o.sigmoid {
        logits.iter().map(|l| 1.0 / (1.0 + (-l).exp())).collect()
    } else {
        let mx = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let z: f64 = logits.iter().map(|l| (l - mx).exp()).sum();
        logits.iter().map(|l| (l - mx).exp() / z).collect()
    };
    let mut order: Vec<usize> = (0..ne).collect();
    order.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap().then(a.cmp(b)));
    // ★ THE ROUTING IS DECIDED BEYOND THE CHIP'S ROUNDING. Both forms sum the router's products at f32 from operands
    // bf16 holds exactly (multiples of 1/64), so a logit is off by at most `γ_d·Σ_t |x_t·Wr_te|` (`γ_d = d·u/(1 − d·u)`,
    // `u = 2⁻²⁴`), one rounding more for a bias; the scores are monotone in the logits and computed at f32 too. Each
    // kept expert leads the next by four times both bounds and a relative 2⁻²⁰ more, so neither form can choose
    // otherwise than the VM.
    let u = 2f64.powi(-24);
    let gamma = d as f64 * u / (1.0 - d as f64 * u);
    let err = |e: usize| gamma * (0..d).map(|t| (x[t] * wr[t * ne + e]).abs()).sum::<f64>() + u * logits[e].abs();
    for w in order.windows(2).take(if o.bias || o.groups.is_some() { 0 } else { k }) {
        let (a, b) = (w[0], w[1]);
        let margin = 4.0 * (err(a) + err(b)) + 2f64.powi(-20) * logits[a].abs().max(1.0);
        assert!(logits[a] - logits[b] > margin, "{m:?} {o:?}: the router is ambiguous between experts {a} and {b}: logits {} and {} within {margin:e}", logits[a], logits[b]);
    }
    // ★ A CHOICE BY BIASED SCORES OR BY GROUPS IS DECIDED IN SCORE SPACE. A sigmoid moves a score by at most a quarter
    // of its logit's error, a softmax a score by its share of twice the largest; each computed at f32, a few roundings
    // more.
    let picked = (o.bias || o.groups.is_some()).then(|| {
        let err: Vec<f64> = if o.sigmoid {
            (0..ne).map(|e| err(e) / 4.0 + 8.0 * u * p[e]).collect()
        } else {
            let worst = (0..ne).map(err).fold(0.0, f64::max);
            (0..ne).map(|e| p[e] * (2.0 * worst + 16.0 * u)).collect()
        };
        chosen(m, o, &dt, &p, &err)
    });
    let kept = picked.as_deref().unwrap_or(&order[..k]);
    let total: f64 = if o.renormalize { kept.iter().map(|e| p[*e]).sum::<f64>().max(FLOOR) } else { 1.0 };
    let scale = o.scale.unwrap_or(1.0);
    let (mut out, mut mag) = (vec![0.0; d], vec![0.0; d]);
    let mut clamped = (false, false);
    for &e in kept {
        let w = p[e] / total * scale;
        // The weight scales the expert's input, or its output.
        let (w_in, w_out) = if o.inputs { (w, 1.0) } else { (1.0, w) };
        let (y, ymag, bit) = expert(m, o, &dt, e, w_in);
        clamped = (clamped.0 | bit.0, clamped.1 | bit.1);
        for c in 0..d {
            out[c] += w_out * y[c];
            mag[c] += w_out * ymag[c];
        }
    }
    let mut src = String::from("## Main\n");
    src += &declare("X", 1, d, x);
    src += &declare("Wr", d, ne, wr);
    src += &declare("Wg", ne * dff, d, wg);
    src += &declare("Wu", ne * dff, d, wu);
    src += &declare("Wd", ne * d, dff, wd);
    src += &declare("Br", 1, ne, br);
    src += &declare("Bg", ne, dff, bg);
    src += &declare("Bu", ne, dff, bu);
    src += &declare("Bd", ne, d, bd);
    src += &declare("Bs", 1, ne, &dt.bs);
    let plus = |b: bool, name: &str| if b { format!(" plus {name}") } else { String::new() };
    let score = format!(
        "{}{}{}",
        if o.sigmoid { " by sigmoid" } else { "" },
        if o.bias { " biased by Bs" } else { "" },
        o.groups.map_or(String::new(), |(kept, count)| format!(" within the top {kept} of {count} groups"))
    );
    let norm = format!("{}{}", if o.renormalize { "" } else { " without renormalizing" }, o.scale.map_or(String::new(), |s| format!(" scaled by {s:?}")));
    let inputs = if o.inputs { " weighting the inputs" } else { "" };
    let act = o.clamp.map_or(String::new(), |(l, a)| format!(" using a swiglu clamped at {l:?} with slope {a:?}"));
    src += &format!(
        "Let H be moe of X with router Wr{} gate Wg{} up Wu{} down Wd{} choosing {k} of {ne}{score}{norm}{inputs}{act}.\n",
        plus(o.router_bias, "Br"),
        plus(o.gate_bias, "Bg"),
        plus(o.up_bias, "Bu"),
        plus(o.down_bias, "Bd"),
    );
    for c in 0..d {
        src += &format!("Show round(item {} of H * 1000000).\n", c + 1);
    }
    Case { src, reference: out.iter().zip(&mag).map(|(a, b)| (*a, *b)).collect(), clamped }
}

/// The program lowered whole for `target`, certified, and its section count.
fn composed(src: &str, target: &SpatialTarget, what: &str) -> (SpatialProgram, usize) {
    let pl = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
    let sp = pl.whole().unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal())).clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the composed program certifies");
    (sp, pl.sections)
}

fn ulp_bf16(x: f64) -> f64 {
    let e = if x == 0.0 { -126 } else { (x.abs().log2().floor() as i32).max(-126) };
    2f64.powi(e - 7)
}

/// Whether the program took the blocked form: its routing section writes the pick table the tile gathers read.
fn blocked(sp: &SpatialProgram) -> bool {
    sp.tensors.iter().any(|t| t.name == "H__moe_picks")
}

/// The case on the fp32 twin and on the SN50, held to the two-part rule; `form` is whether it must take the blocked
/// form.
fn verdict(c: &Case, what: &str, form: bool) {
    let src = c.src.as_str();
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    assert_eq!(want.len(), c.reference.len(), "{what}: one Show an output");
    for (i, (v, (r, _))) in want.iter().zip(&c.reference).enumerate() {
        assert!(((*v as f64) - (r * 1e6).round()).abs() <= 1.0, "{what}: Show #{}: VM {v} vs the f64 reference {}", i + 1, r * 1e6);
    }
    let (twin, _) = composed(src, &fp32_twin(), what);
    assert_eq!(blocked(&twin), form, "{what}: the {} form", if form { "blocked" } else { "flat" });
    let out = eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills")).unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}"));
    let got = shown_from_outputs(src, &twin, &out).expect("shown");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        let bound = ((*v as f64).abs() / 65536.0).max(1.0);
        assert!(((g - v).abs() as f64) <= bound, "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v}\ntwin {got:?}\nvm   {want:?}", i + 1);
    }
    let (sp, sections) = composed(src, &SpatialTarget::sn50_rdu(), what);
    assert_eq!(blocked(&sp), form, "{what}: the {} form on the SN50", if form { "blocked" } else { "flat" });
    let out = eval_choreography(&sp, &inputs_from_fills(&sp).expect("fills")).unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}"));
    let got = shown_from_outputs(src, &sp, &out).expect("shown");
    for (i, ((g, v), (_, m))) in got.iter().zip(&want).zip(&c.reference).enumerate() {
        let bound = ((sections + 1) as f64 * ulp_bf16(*m) * 1e6).max(1.0);
        assert!(((g - v).abs() as f64) <= bound, "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (magnitude {m:.4}, bound {bound:.0})", i + 1);
    }
}

/// Every option set on every shape, each case named by its shape and options.
fn every_option_on(shapes: &[Shape], form: bool) {
    for m in shapes {
        for (name, o) in option_sets() {
            let what = format!("{name} at d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
            verdict(&case(*m, o), &what, form);
        }
    }
}

const FLAT: [Shape; 2] = [Shape { d: 5, dff: 7, ne: 4, k: 2 }, Shape { d: 8, dff: 3, ne: 6, k: 3 }];
const BLOCKED: [Shape; 2] = [Shape { d: 32, dff: 32, ne: 4, k: 2 }, Shape { d: 64, dff: 96, ne: 8, k: 3 }];
/// Flat mixtures whose rounds gather their experts' runs: Llama 4's tiny one (weight runs of whole tiles, tables a
/// site holds whole only in bf16), 64 experts with runs of whole tiles and with runs mid-tile (tables no site holds
/// whole), and runs of whole tiles beside a down bias of whole tiles.
const GATHERED: [Shape; 4] = [
    Shape { d: 16, dff: 8, ne: 4, k: 1 },
    Shape { d: 16, dff: 8, ne: 64, k: 2 },
    Shape { d: 5, dff: 7, ne: 64, k: 2 },
    Shape { d: 32, dff: 8, ne: 8, k: 2 },
];

#[test]
fn every_option_computes_the_vms_mixture_in_the_flat_form() {
    every_option_on(&FLAT, false);
}

#[test]
fn every_option_computes_the_vms_mixture_in_the_blocked_form() {
    every_option_on(&BLOCKED, true);
}

#[test]
fn every_option_moves_the_mixture_on_these_inputs() {
    // Not vacuous: each option changes the reference by far more than the bounds above, on every shape, and the
    // clamp crosses both the gate's and the up's limits.
    for m in FLAT.iter().chain(&BLOCKED) {
        let plain = case(*m, PLAIN).reference;
        for (name, o) in option_sets() {
            let c = case(*m, o);
            let moved = c.reference.iter().zip(&plain).map(|((a, _), (b, _))| (a - b).abs()).fold(0.0, f64::max);
            let scale = plain.iter().map(|(_, mag)| *mag).fold(0.0, f64::max);
            assert!(moved > 0.05 * scale, "{name} at {m:?} moves the mixture by {moved}, against a magnitude of {scale}");
            if o.clamp.is_some() {
                assert_eq!(c.clamped, (true, true), "{name} at {m:?}: the clamp bites on the gate and the up");
            }
        }
    }
}

/// Every routing on every shape.
fn every_routing_on(shapes: &[Shape], form: bool) {
    for m in shapes {
        for (name, o) in routing_sets() {
            let what = format!("{name} at d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
            verdict(&case(*m, o), &what, form);
        }
    }
}

#[test]
fn every_routing_computes_the_vms_mixture_in_the_flat_form() {
    every_routing_on(&FLAT, false);
}

#[test]
fn every_routing_computes_the_vms_mixture_in_the_blocked_form() {
    every_routing_on(&BLOCKED, true);
}

#[test]
fn every_routing_moves_the_mixture_on_these_inputs() {
    // Not vacuous: scoring by sigmoid and weighting the inputs each change the reference by at least sixteen times the
    // WIRING bound, the check that holds the routing's arithmetic (the CHIP bound holds the bf16 datapath, and where two
    // kept experts compute alike a renormalized score moves the mixture by less than it).
    for m in FLAT.iter().chain(&BLOCKED) {
        let scale = case(*m, PLAIN).reference.iter().map(|(_, mag)| *mag).fold(0.0, f64::max);
        for (name, o) in routing_sets() {
            let c = case(*m, o).reference;
            for (flip, other) in [("the score", Opts { sigmoid: !o.sigmoid, ..o }), ("the weighting", Opts { inputs: !o.inputs, ..o })] {
                let moved = c.iter().zip(&case(*m, other).reference).map(|((a, _), (b, _))| (a - b).abs()).fold(0.0, f64::max);
                assert!(moved > 16.0 * scale / 65536.0, "{name} at {m:?}: flipping {flip} moves the mixture by {moved}, against {scale}");
            }
        }
    }
}

#[test]
fn every_option_computes_the_vms_mixture_where_the_rounds_gather_their_experts() {
    every_option_on(&GATHERED, false);
}

#[test]
fn every_routing_computes_the_vms_mixture_where_the_rounds_gather_their_experts() {
    every_routing_on(&GATHERED, false);
}

/// Mixtures whose router row is wider than an accumulator page, scored and routed in blocks of the row: a last block
/// shorter than the first, and the top 10 of 512 (more picks than a page holds a tile each).
const WIDE: [Shape; 2] = [Shape { d: 32, dff: 32, ne: 300, k: 2 }, Shape { d: 32, dff: 32, ne: 512, k: 10 }];

/// Every set of `sets` on every wide shape, on [`wide_data`], in the blocked form.
fn every_wide(sets: Vec<(&'static str, Opts)>) {
    for m in WIDE {
        for (name, o) in &sets {
            let what = format!("{name} at d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
            verdict(&case_of(m, *o, wide_data(m)), &what, true);
        }
    }
}

#[test]
fn every_option_computes_the_vms_mixture_where_the_router_row_is_wider_than_a_page() {
    every_wide(option_sets());
}

#[test]
fn every_routing_computes_the_vms_mixture_where_the_router_row_is_wider_than_a_page() {
    every_wide(routing_sets());
}

#[test]
fn every_choice_without_a_group_limit_computes_the_vms_mixture_where_the_router_row_is_wider_than_a_page() {
    every_wide(choice_sets().into_iter().filter(|(_, o)| o.groups.is_none()).collect());
}

#[test]
fn the_wide_options_move_the_mixture_on_these_inputs() {
    // Not vacuous: every option, routing and choice moves the wide reference by far more than the bounds above — not
    // renormalizing, the softmax's row maximum and sum among them.
    for m in WIDE {
        let plain = case_of(m, PLAIN, wide_data(m)).reference;
        let scale = plain.iter().map(|(_, mag)| *mag).fold(0.0, f64::max);
        for (name, o) in option_sets().into_iter().chain(routing_sets()).chain(choice_sets().into_iter().filter(|(_, o)| o.groups.is_none())) {
            let c = case_of(m, o, wide_data(m)).reference;
            let moved = c.iter().zip(&plain).map(|((a, _), (b, _))| (a - b).abs()).fold(0.0, f64::max);
            assert!(moved > 16.0 * scale / 65536.0, "{name} at {m:?} moves the wide mixture by {moved}, against a magnitude of {scale}");
        }
    }
}

/// Tiles a flat row of `n` elements spans.
fn tiles(n: usize, tw: usize) -> usize {
    n.div_ceil(tw)
}

/// The tiles one round moves of a table of `ne` runs of `span` elements: the whole tiles around the longest reach of a
/// run from its first element's offset in its tile, `(e·span mod tw) + span`, and never more than the table.
fn window(ne: usize, span: usize, tw: usize) -> usize {
    (0..ne).map(|e| (e * span % tw + span).div_ceil(tw)).max().expect("experts").min(tiles(ne * span, tw))
}

#[test]
fn a_flat_mixture_moves_its_chosen_experts_runs_or_each_table_once_whichever_is_fewer_tiles() {
    // ★ ONLY THE CHOSEN EXPERTS' RUNS MOVE, unless the table is fewer tiles. Expert `e`'s run of a table of `ne` runs of
    // `span` elements starts at element `e·span`; a round moves the `W` whole tiles around it, so `k` rounds move `k·W`
    // of the table's `T` — or the table moves once, where `k·W` is not fewer. Counted over the run, per table, on both
    // chips: the weights (runs of `dff·d`), the gate and up biases (runs of `dff`), the down bias (runs of `d`).
    for target in [fp32_twin(), SpatialTarget::sn50_rdu()] {
        let tw = target.tile.cols;
        for m in FLAT.iter().chain(&GATHERED) {
            let what = format!("d{} dff{} e{} k{} on {}", m.d, m.dff, m.ne, m.k, target.name);
            let c = case(*m, GPT_OSS);
            let (sp, _) = composed(&c.src, &target, &what);
            assert!(!blocked(&sp), "{what}: the flat form");
            let (_, trace) = eval_choreography_traced_with(&sp, &inputs_from_fills(&sp).expect("fills"), Scheduler::RoundRobin)
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            for (table, span) in [("Wg", m.dff * m.d), ("Wu", m.dff * m.d), ("Wd", m.d * m.dff), ("Bg", m.dff), ("Bu", m.dff), ("Bd", m.d)] {
                let id = sp.tensors.iter().find(|t| t.name == table).unwrap_or_else(|| panic!("{what}: `{table}`")).id;
                let moved: u64 = trace.tiles_read.iter().filter(|((t, _), _)| *t == id).map(|(_, n)| *n).sum();
                let (w, t) = (window(m.ne, span, tw), tiles(m.ne * span, tw));
                assert_eq!(moved, (m.k * w).min(t) as u64, "{what}: `{table}` ({} runs of {span}: windows of {w}, a table of {t})", m.ne);
            }
        }
    }
}

#[test]
fn the_gathering_shapes_cover_every_form_of_a_table() {
    // Not vacuous: among the flat shapes' tables a table moves whole, and runs of whole tiles and runs mid-tile are each
    // gathered, for a weight and for a bias.
    let tw = SpatialTarget::sn50_rdu().tile.cols;
    let mut forms = std::collections::BTreeSet::new();
    for m in FLAT.iter().chain(&GATHERED) {
        for (part, span) in [("weight", m.dff * m.d), ("bias", m.dff), ("bias", m.d)] {
            let gathered = m.k * window(m.ne, span, tw) < tiles(m.ne * span, tw);
            forms.insert(match (gathered, span % tw == 0) {
                (false, _) => ("whole", ""),
                (true, true) => ("whole tiles", part),
                (true, false) => ("mid-tile", part),
            });
        }
    }
    let want: std::collections::BTreeSet<_> =
        [("whole", ""), ("whole tiles", "weight"), ("whole tiles", "bias"), ("mid-tile", "weight"), ("mid-tile", "bias")].into();
    assert_eq!(forms, want);
}

#[test]
fn every_expert_is_read_from_its_own_run() {
    // Each expert in turn is forced to the top by its router bias — the last ones' windows clamped to end inside the
    // table, every run read at its own offset — and the mixture is that expert's alone, within WIRING's bound.
    for m in &GATHERED {
        let m = Shape { k: 1, ..*m };
        let o = Opts { router_bias: true, gate_bias: true, up_bias: true, down_bias: true, ..PLAIN };
        let src = case(m, o).src;
        let what = format!("d{} dff{} e{}", m.d, m.dff, m.ne);
        let (twin, _) = composed(&src, &fp32_twin(), &what);
        let br = twin.tensors.iter().find(|t| t.name == "Br").expect("the router's bias").id;
        let dt = data(m);
        for e in 0..m.ne {
            let mut inputs = inputs_from_fills(&twin).expect("fills");
            let Some(Value::Float(v)) = inputs.tensors.get_mut(&br) else { panic!("{what}: a float router bias") };
            v[e] = 1024.0;
            let out = eval_choreography(&twin, &inputs).unwrap_or_else(|err| panic!("{what}: expert {e}: {err:?}"));
            let got = shown_from_outputs(&src, &twin, &out).expect("shown");
            let (want, _, _) = expert(m, o, &dt, e, 1.0);
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                let w = (w * 1e6).round();
                assert!(((*g as f64) - w).abs() <= (w.abs() / 65536.0).max(1.0), "{what}: expert {e}: Show #{}: {g} vs {w}", i + 1);
            }
        }
    }
}

/// Flat shapes for the choices: groups of one expert, groups of two, and 64 experts gathered by their runs.
const CHOICE_FLAT: [Shape; 3] = [Shape { d: 5, dff: 7, ne: 4, k: 2 }, Shape { d: 5, dff: 7, ne: 8, k: 3 }, Shape { d: 16, dff: 8, ne: 64, k: 2 }];
const CHOICE_BLOCKED: [Shape; 2] = [Shape { d: 32, dff: 32, ne: 8, k: 3 }, Shape { d: 64, dff: 96, ne: 16, k: 4 }];

/// Every choice on every shape.
fn every_choice_on(shapes: &[Shape], form: bool) {
    for m in shapes {
        for (name, o) in choice_sets() {
            let what = format!("{name} at d{} dff{} e{} k{}", m.d, m.dff, m.ne, m.k);
            verdict(&case_of(*m, o, grouped_data(*m)), &what, form);
        }
    }
}

#[test]
fn every_choice_computes_the_vms_mixture_in_the_flat_form() {
    every_choice_on(&CHOICE_FLAT, false);
}

#[test]
fn every_choice_computes_the_vms_mixture_in_the_blocked_form() {
    every_choice_on(&CHOICE_BLOCKED, true);
}

#[test]
fn every_choice_moves_the_mixture_on_these_inputs() {
    // Not vacuous: the selection bias, the group limit and the scale each change the reference by at least sixteen
    // times the WIRING bound, on every shape. A group of one expert is scored by that expert alone, so keeping the top
    // `G` groups and then the top `K ≤ G` experts is the top `K` of all: there the group limit cannot move it.
    for m in CHOICE_FLAT.iter().chain(&CHOICE_BLOCKED) {
        let case = |o: Opts| case_of(*m, o, grouped_data(*m)).reference;
        let scale = case(PLAIN).iter().map(|(_, mag)| *mag).fold(0.0, f64::max);
        for (name, o) in choice_sets() {
            let c = case(o);
            let flips = [
                ("the selection bias", Opts { bias: !o.bias, ..o }),
                ("the group limit", Opts { groups: if o.groups.is_some() { None } else { Some((2, 4)) }, ..o }),
                ("the scale", Opts { scale: if o.scale.is_some() { None } else { Some(2.5) }, ..o }),
            ];
            for (flip, other) in flips {
                if flip == "the group limit" && m.ne / 4 == 1 {
                    continue;
                }
                let moved = c.iter().zip(&case(other)).map(|((a, _), (b, _))| (a - b).abs()).fold(0.0, f64::max);
                assert!(moved > 16.0 * scale / 65536.0, "{name} at {m:?}: flipping {flip} moves the mixture by {moved}, against {scale}");
            }
        }
    }
}

#[test]
fn a_renormalizing_sum_below_two_to_the_minus_fourteen_is_clamped_on_the_chip() {
    // Every router bias set on the chip to twelve below the least whole number past every logit: the kept sigmoids sum
    // below 2⁻¹⁴, so each weight is its score over 2⁻¹⁴, in both forms. The reference is the case's mixture at those
    // logits.
    for (m, form) in CHOICE_FLAT.iter().map(|m| (m, false)).chain(CHOICE_BLOCKED.iter().map(|m| (m, true))) {
        let o = Opts { sigmoid: true, router_bias: true, ..PLAIN };
        let src = case(*m, o).src;
        let (twin, _) = composed(&src, &fp32_twin(), "clamped");
        assert_eq!(blocked(&twin), form, "{m:?}: the form");
        let mut inputs = inputs_from_fills(&twin).expect("fills");
        let br = twin.tensors.iter().find(|t| t.name == "Br").expect("the router's bias").id;
        let Some(Value::Float(v)) = inputs.tensors.get_mut(&br) else { panic!("{m:?}: a float router bias") };
        let shift = clamp_shift(*m);
        v.iter_mut().for_each(|b| *b = -shift);
        let out = eval_choreography(&twin, &inputs).unwrap_or_else(|e| panic!("{m:?}: {e:?}"));
        let got = shown_from_outputs(&src, &twin, &out).expect("shown");
        let want = clamped_reference(*m);
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let w = (w * 1e6).round();
            assert!(((*g as f64) - w).abs() <= (w.abs() / 65536.0).max(1.0), "{m:?}: Show #{}: {g} vs {w}", i + 1);
        }
    }
}

#[test]
fn ties_keep_the_first_groups_and_the_first_experts_on_the_chip() {
    // A router that reads nothing ties every score and every group's score: the chip keeps the first groups and the
    // first experts, as the VM's scans do, in both forms.
    for (m, form) in CHOICE_FLAT.iter().map(|m| (m, false)).chain(CHOICE_BLOCKED.iter().map(|m| (m, true))) {
        for (name, o) in choice_sets() {
            let what = format!("{name} at {m:?}, every score tied");
            let src: String = case_of(*m, o, grouped_data(*m))
                .src
                .lines()
                .filter(|l| !l.contains(" of Wr to ") && !l.contains(" of Bs to ") && !l.contains(" of Br to "))
                .map(|l| format!("{l}\n"))
                .collect();
            let vm = vm_outcome(&src);
            assert!(vm.error.is_none(), "{what}: VM: {:?}", vm.error);
            let want = oracle_ints(&vm.output).expect("the VM shows integers");
            let (twin, _) = composed(&src, &fp32_twin(), &what);
            assert_eq!(blocked(&twin), form, "{what}: the form");
            let out = eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills")).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            let got = shown_from_outputs(&src, &twin, &out).expect("shown");
            for (i, (g, v)) in got.iter().zip(&want).enumerate() {
                assert!(((g - v).abs() as f64) <= ((*v as f64).abs() / 65536.0).max(1.0), "{what}: Show #{}: fp32 twin {g} vs VM {v}", i + 1);
            }
        }
    }
}

#[test]
fn a_nan_score_under_a_group_limit_is_never_picked() {
    // Expert 0's router bias is NaN on the chip: its score is NaN, its group scores only by its other experts' (the
    // NaN never the best, nor the second), and it is never picked. The reference is DeepSeek-V3's choice with expert 0's
    // selection score below every other.
    for (m, form) in CHOICE_FLAT.iter().skip(1).map(|m| (m, false)).chain(CHOICE_BLOCKED.iter().map(|m| (m, true))) {
        let o = Opts { router_bias: true, ..DEEPSEEK_V3 };
        let dt = grouped_data(*m);
        let src = case_of(*m, o, grouped_data(*m)).src;
        let (twin, _) = composed(&src, &fp32_twin(), "nan under groups");
        assert_eq!(blocked(&twin), form, "{m:?}: the form");
        let mut inputs = inputs_from_fills(&twin).expect("fills");
        let br = twin.tensors.iter().find(|t| t.name == "Br").expect("the router's bias").id;
        let Some(Value::Float(v)) = inputs.tensors.get_mut(&br) else { panic!("{m:?}: a float router bias") };
        v[0] = f64::NAN;
        let out = eval_choreography(&twin, &inputs).unwrap_or_else(|e| panic!("{m:?}: {e:?}"));
        let got = shown_from_outputs(&src, &twin, &out).expect("shown");
        let want = nan_grouped_reference(*m, o, &dt);
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            let w = (w * 1e6).round();
            assert!(((*g as f64) - w).abs() <= (w.abs() / 65536.0).max(1.0), "{m:?}: Show #{}: {g} vs {w}", i + 1);
        }
    }
}

/// DeepSeek-V3's choice on `dt` with expert 0's selection score below every other: the groups scored by their two best
/// selection scores among the rest (a group of expert 0 and one other by that other's alone, expert 0 counting as
/// −1e30), the top two groups kept, the top `K` of their experts, renormalized over a sum clamped at 2⁻¹⁴ and scaled.
fn nan_grouped_reference(m: Shape, o: Opts, dt: &Data) -> Vec<f64> {
    let Shape { d, ne, k, .. } = m;
    let (kept, count) = o.groups.expect("a group limit");
    let per = ne / count;
    let logits: Vec<f64> = (0..ne).map(|e| dt.plain[e] + dt.br[e]).collect();
    let p: Vec<f64> = logits.iter().map(|l| 1.0 / (1.0 + (-l).exp())).collect();
    let mut sel: Vec<f64> = (0..ne).map(|e| p[e] + dt.bs[e]).collect();
    sel[0] = -1.0e30;
    let score = |g: usize| {
        let mut v: Vec<f64> = sel[g * per..(g + 1) * per].to_vec();
        v.sort_by(|a, b| b.partial_cmp(a).unwrap());
        v[0] + v[1]
    };
    let mut ranked: Vec<usize> = (0..count).collect();
    ranked.sort_by(|a, b| score(*b).partial_cmp(&score(*a)).unwrap().then(a.cmp(b)));
    let mut allowed: Vec<usize> = ranked[..kept].iter().flat_map(|g| g * per..(g + 1) * per).collect();
    allowed.sort_by(|a, b| sel[*b].partial_cmp(&sel[*a]).unwrap().then(a.cmp(b)));
    let chosen = &allowed[..k];
    let total = chosen.iter().map(|e| p[*e]).sum::<f64>().max(FLOOR);
    let mut out = vec![0.0; d];
    for &e in chosen {
        let (y, _, _) = expert(m, o, dt, e, 1.0);
        for c in 0..d {
            out[c] += p[e] / total * o.scale.unwrap_or(1.0) * y[c];
        }
    }
    out
}

/// Twelve past the least whole number at or above every plain logit: the shift that sinks every sigmoid below 2⁻¹².
fn clamp_shift(m: Shape) -> f64 {
    data(m).plain.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() + 12.0
}

/// [`case`]'s mixture scored by sigmoids of its logits less [`clamp_shift`], renormalized over a sum the clamp lifts to
/// 2⁻¹⁴.
fn clamped_reference(m: Shape) -> Vec<f64> {
    let dt = data(m);
    let o = Opts { sigmoid: true, router_bias: true, ..PLAIN };
    let shift = clamp_shift(m);
    let p: Vec<f64> = dt.plain.iter().map(|l| 1.0 / (1.0 + (shift - l).exp())).collect();
    let mut order: Vec<usize> = (0..m.ne).collect();
    order.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap().then(a.cmp(b)));
    for w in order.windows(2).take(m.k) {
        assert!(p[w[0]] - p[w[1]] > 1e-3 * p[w[0]], "{m:?}: the low scores are ambiguous");
    }
    let total: f64 = order[..m.k].iter().map(|e| p[*e]).sum();
    assert!(total < FLOOR, "{m:?}: the kept scores sum to {total:e}, not under 2⁻¹⁴");
    let mut out = vec![0.0; m.d];
    for &e in &order[..m.k] {
        let (y, _, _) = expert(m, o, &dt, e, 1.0);
        for c in 0..m.d {
            out[c] += p[e] / FLOOR * y[c];
        }
    }
    out
}

#[test]
fn renormalized_weights_scaling_the_inputs_are_refused_by_name() {
    // Each round is weighed as it is picked, before the kept weights' sum is known; the VM computes it, the chip says
    // why it does not.
    for m in FLAT.iter().chain(&BLOCKED) {
        let c = case(*m, Opts { renormalize: true, ..LLAMA4 });
        let pl = compile_program_to_spatial(&c.src, &SpatialTarget::sn50_rdu()).expect("the tensor program walks");
        assert!(pl.whole().is_none(), "{m:?}: lowered a renormalized weighting of the inputs");
        assert!(pl.refusal().contains("renormalized weights scale"), "{m:?}: {}", pl.refusal());
    }
}

#[test]
fn a_nan_score_ahead_of_the_rest_is_never_picked() {
    // Expert 0's router bias is NaN on the chip: its sigmoid score is NaN and the first the arg-max reads, so a fold
    // seeded by the first score would keep it; the desugar's scan never picks a NaN. The reference is the case's
    // mixture over the other experts.
    for (m, form) in FLAT.iter().map(|m| (m, false)).chain(BLOCKED.iter().map(|m| (m, true))) {
        let o = Opts { router_bias: true, ..LLAMA4 };
        let src = case(*m, o).src;
        let (twin, _) = composed(&src, &fp32_twin(), "nan routing");
        assert_eq!(blocked(&twin), form, "{m:?}: the form");
        let mut inputs = inputs_from_fills(&twin).expect("fills");
        let br = twin.tensors.iter().find(|t| t.name == "Br").expect("the router's bias").id;
        let Some(Value::Float(v)) = inputs.tensors.get_mut(&br) else { panic!("{m:?}: a float router bias") };
        v[0] = f64::NAN;
        let out = eval_choreography(&twin, &inputs).unwrap_or_else(|e| panic!("{m:?}: {e:?}"));
        let got = shown_from_outputs(&src, &twin, &out).expect("shown");
        let want = nan_first_reference(*m, o);
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(((*g as f64) - (w * 1e6).round()).abs() <= ((w * 1e6).abs() / 65536.0).max(1.0), "{m:?}: Show #{}: {g} vs {}", i + 1, w * 1e6);
        }
    }
}

/// [`case`]'s mixture under Llama 4's routing (scored by sigmoid, each kept weight scaling its expert's input) with
/// expert 0's score NaN: the kept experts are the top `k` of the others.
fn nan_first_reference(m: Shape, o: Opts) -> Vec<f64> {
    assert!(o.sigmoid && o.inputs && !o.renormalize, "{o:?}: the reference is Llama 4's routing");
    let dt = data(m);
    let p: Vec<f64> = (0..m.ne).map(|e| 1.0 / (1.0 + (-(dt.plain[e] + on(o.router_bias, dt.br[e]))).exp())).collect();
    let mut order: Vec<usize> = (1..m.ne).collect();
    order.sort_by(|a, b| p[*b].partial_cmp(&p[*a]).unwrap().then(a.cmp(b)));
    let mut out = vec![0.0; m.d];
    for &e in &order[..m.k] {
        let (y, _, _) = expert(m, o, &dt, e, p[e]);
        for c in 0..m.d {
            out[c] += y[c];
        }
    }
    out
}
