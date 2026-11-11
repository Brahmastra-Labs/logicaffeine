//! Reverse-mode differentiation of a tensor program, as a program.
//!
//! [`backward`] reads a forward program's tensor statements (the typed specs the front end lifts,
//! [`crate::spatial::passes::recognize_stages`]) and writes the statements that compute the gradient of a scalar
//! loss with respect to any tensor the forward reads. The backward pass is ORDINARY LOGOS — tensor sugar where the
//! language has the form (`matmul`, `transpose`, `plus`, `hadamard`, `scaled by`), row-major loops over declared
//! tensors where it does not — so the same front end, fusion and every backend take it, and the VM runs it in f64 as
//! the oracle a gradient is checked against.
//!
//! The loss is `Σ out ⊙ cotangent` for a declared `cotangent` tensor of the output's shape: the seed of reverse
//! mode is that tensor, and every gradient follows from it. A tensor read by several stages sums its gradients.
//!
//! Every op kind has a rule or a refusal that names it (`vjp_stage`, an exhaustive match over
//! [`TensorOpSpec`]). A refusal is either [`VjpRefusal::NoGradient`] — the op is piecewise constant in its
//! inputs (`argmax`) — or [`VjpRefusal::NotDerived`], an op whose rule is not written yet: a training program
//! that reaches it is refused by name, never differentiated as if the op were absent.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::spatial::passes::recognize_stages::{RecognizedProgram, RecognizedStage, TensorOpSpec};
use crate::tensor_recognize::{ElementwiseOp, Epsilon, RowReduceKind};

/// Why a stage has no backward rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VjpRefusal {
    /// The op is piecewise constant in its inputs: its gradient is zero almost everywhere and undefined at
    /// the jumps, so no training signal flows through it.
    NoGradient { op: &'static str, why: &'static str },
    /// The op's rule is not written yet.
    NotDerived { op: &'static str },
    /// The loss depends on `tensor` through `op`, whose rule does not differentiate that operand (a table of
    /// rotation angles, an integer index): asking for its gradient is refused rather than answered with zero.
    NotDifferentiated { tensor: String, op: &'static str },
    /// A tensor the backward pass needs has no shape the front end recorded.
    Shapeless { tensor: String },
    /// The forward program does not lift whole, so its backward cannot be derived from it.
    Unrecognized { statements: Vec<(usize, String)> },
    /// The output the loss is taken over is not a tensor any stage computes or the program declares.
    UnknownOutput { tensor: String },
    /// The parser refused the program.
    Parse(String),
}

/// A derived backward pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Backward {
    /// The statements, one per line, to append inside `## Main` after the forward.
    pub source: String,
    /// For every tensor the forward reads that the loss depends on: the tensor holding its gradient.
    pub grads: BTreeMap<String, String>,
}

/// Fresh names, so every tensor the backward binds is bound once — the front end refuses a second binding.
struct Names {
    next: usize,
}

impl Names {
    fn fresh(&mut self, stem: &str) -> String {
        self.next += 1;
        format!("{stem}_{}", self.next)
    }
}

/// The statements a backward pass is accumulating, and its gradient table.
struct Out {
    text: String,
    names: Names,
    grads: BTreeMap<String, String>,
    shapes: BTreeMap<String, (usize, usize)>,
    /// Operands the loss reaches through a rule that does not differentiate them, and that rule's op.
    undiff: BTreeMap<String, &'static str>,
}

impl Out {
    fn line(&mut self, depth: usize, s: impl AsRef<str>) {
        for _ in 0..depth {
            self.text.push_str("    ");
        }
        self.text.push_str(s.as_ref());
        self.text.push('\n');
    }

    /// Declare a zero-filled `rows × cols` tensor of Real.
    fn tensor(&mut self, stem: &str, (rows, cols): (usize, usize)) -> String {
        let name = self.names.fresh(stem);
        self.line(0, format!("Let {name} be a {rows} by {cols} tensor of Real."));
        self.shapes.insert(name.clone(), (rows, cols));
        name
    }

    /// Add `contribution` to the gradient of `tensor`: the first contribution is the gradient, a later one is
    /// summed into a fresh tensor.
    fn accumulate(&mut self, tensor: &str, contribution: String) {
        match self.grads.get(tensor).cloned() {
            None => {
                self.grads.insert(tensor.to_string(), contribution);
            }
            Some(previous) => {
                let sum = self.names.fresh(&format!("g_{tensor}"));
                self.line(0, format!("Let {sum} be {previous} plus {contribution}."));
                let shape = self.shapes[&previous];
                self.shapes.insert(sum.clone(), shape);
                self.grads.insert(tensor.to_string(), sum);
            }
        }
    }

    fn shape(&self, tensor: &str) -> Result<(usize, usize), VjpRefusal> {
        self.shapes.get(tensor).copied().ok_or_else(|| VjpRefusal::Shapeless { tensor: tensor.to_string() })
    }

    /// A loop over every element of a `rows × cols` tensor, binding the row `r`, the column `c` and the
    /// 1-based flat index `k`; `body` writes the inner statements at the given depth.
    fn each(&mut self, (rows, cols): (usize, usize), body: impl FnOnce(&mut Out, usize, &str, &str, &str)) {
        let r = self.names.fresh("r");
        let c = self.names.fresh("c");
        let k = self.names.fresh("k");
        self.line(0, format!("Repeat for {r} from 1 to {rows}:"));
        self.line(1, format!("Repeat for {c} from 1 to {cols}:"));
        self.line(2, format!("Let {k} be ({r} - 1) * {cols} + {c}."));
        body(self, 2, &r, &c, &k);
    }
}

/// The backward pass of `forward` (a program whose `## Main` holds tensor statements) for the loss
/// `Σ output ⊙ cotangent`, giving the gradient of every tensor in `wrt`. `cotangent` must be a declared tensor of
/// `output`'s shape. A tensor in `wrt` the loss does not depend on gets a zero gradient.
pub fn backward(forward: &str, output: &str, cotangent: &str, wrt: &[&str]) -> Result<Backward, VjpRefusal> {
    let rp = crate::spatial::passes::recognize_stages::recognize_stages(forward)
        .map_err(|e| VjpRefusal::Parse(format!("{e:?}")))?;
    backward_of(&rp, output, cotangent, wrt)
}

/// [`backward`] over a program the front end has already lifted.
pub fn backward_of(rp: &RecognizedProgram, output: &str, cotangent: &str, wrt: &[&str]) -> Result<Backward, VjpRefusal> {
    if !rp.unrecognized.is_empty() {
        return Err(VjpRefusal::Unrecognized { statements: rp.unrecognized.clone() });
    }
    let produced = rp.stages.iter().any(|s| s.out == output);
    let declared = rp.declared.iter().any(|d| d.name == output);
    if !produced && !declared {
        return Err(VjpRefusal::UnknownOutput { tensor: output.to_string() });
    }
    let mut out = Out {
        text: String::new(),
        names: Names { next: 0 },
        grads: BTreeMap::new(),
        shapes: rp.shapes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        undiff: BTreeMap::new(),
    };
    out.grads.insert(output.to_string(), cotangent.to_string());
    for st in rp.stages.iter().rev() {
        let Some(d_out) = out.grads.get(&st.out).cloned() else { continue };
        vjp_stage(rp, st, &d_out, &mut out)?;
    }
    for &t in wrt {
        if let Some(op) = out.undiff.get(t) {
            return Err(VjpRefusal::NotDifferentiated { tensor: t.to_string(), op });
        }
        if !out.grads.contains_key(t) {
            let shape = out.shape(t)?;
            let zero = out.tensor(&format!("g_{t}"), shape);
            out.grads.insert(t.to_string(), zero);
        }
    }
    let grads = wrt.iter().map(|&t| (t.to_string(), out.grads[t].clone())).collect();
    Ok(Backward { source: out.text, grads })
}

/// The rule of one stage: given `d_out`, the tensor holding the loss's gradient with respect to the stage's
/// output, add each of its inputs' gradients.
fn vjp_stage(rp: &RecognizedProgram, st: &RecognizedStage, d_out: &str, out: &mut Out) -> Result<(), VjpRefusal> {
    match &st.op {
        TensorOpSpec::Matmul(m) => {
            // C = A·B with A m×k, B k×n: dA = dC·Bᵀ, dB = Aᵀ·dC.
            let (a, b) = (rp.name(m.a).to_string(), rp.name(m.b).to_string());
            let bt = out.names.fresh(&format!("t_{b}"));
            out.line(0, format!("Let {bt} be transpose of {b}."));
            out.shapes.insert(bt.clone(), (m.n, m.k));
            let da = out.names.fresh(&format!("g_{a}"));
            out.line(0, format!("Let {da} be {d_out} matmul {bt}."));
            out.shapes.insert(da.clone(), (m.m, m.k));
            let at = out.names.fresh(&format!("t_{a}"));
            out.line(0, format!("Let {at} be transpose of {a}."));
            out.shapes.insert(at.clone(), (m.k, m.m));
            let db = out.names.fresh(&format!("g_{b}"));
            out.line(0, format!("Let {db} be {at} matmul {d_out}."));
            out.shapes.insert(db.clone(), (m.k, m.n));
            out.accumulate(&a, da);
            out.accumulate(&b, db);
            Ok(())
        }
        TensorOpSpec::Elementwise(e) => {
            let (a, b) = (rp.name(e.a).to_string(), rp.name(e.b).to_string());
            match e.op {
                ElementwiseOp::Add => {
                    out.accumulate(&a, d_out.to_string());
                    out.accumulate(&b, d_out.to_string());
                }
                ElementwiseOp::Mul => {
                    let shape = out.shape(&a)?;
                    let da = out.names.fresh(&format!("g_{a}"));
                    out.line(0, format!("Let {da} be {d_out} hadamard {b}."));
                    out.shapes.insert(da.clone(), shape);
                    let db = out.names.fresh(&format!("g_{b}"));
                    out.line(0, format!("Let {db} be {d_out} hadamard {a}."));
                    out.shapes.insert(db.clone(), shape);
                    out.accumulate(&a, da);
                    out.accumulate(&b, db);
                }
            }
            Ok(())
        }
        TensorOpSpec::Scale(c) => {
            // y = x·c: dx = dy·c.
            let x = rp.name(c.x).to_string();
            let shape = out.shape(&x)?;
            let dx = out.names.fresh(&format!("g_{x}"));
            out.line(0, format!("Let {dx} be {d_out} scaled by {:?}.", c.factor.get()));
            out.shapes.insert(dx.clone(), shape);
            out.accumulate(&x, dx);
            Ok(())
        }
        TensorOpSpec::Transpose(t) => {
            let x = rp.name(t.x).to_string();
            let shape = out.shape(&x)?;
            let dx = out.names.fresh(&format!("g_{x}"));
            out.line(0, format!("Let {dx} be transpose of {d_out}."));
            out.shapes.insert(dx.clone(), shape);
            out.accumulate(&x, dx);
            Ok(())
        }
        TensorOpSpec::FusedRow(r) => {
            let x = rp.name(r.x).to_string();
            let y = rp.name(r.y).to_string();
            let shape = (r.rows, r.cols);
            match r.reduce {
                RowReduceKind::Softmax => softmax_rule(out, &y, d_out, &x, shape),
                RowReduceKind::RmsNorm => rmsnorm_rule(rp, r, out, d_out, shape),
                RowReduceKind::LayerNorm => layernorm_rule(rp, r, out, d_out, shape),
            }
        }
        TensorOpSpec::Sigmoid(s) => {
            // y = σ(x): dx = dy · y · (1 − y).
            let (x, y) = (rp.name(s.x).to_string(), rp.name(s.out).to_string());
            pointwise(out, &x, d_out, &[&y], |v| format!("{0} * {1} * (1.0 - {1})", v[0], v[1]))
        }
        TensorOpSpec::Silu(s) => {
            // y = x·σ(x): dx = dy · (σ + x·σ·(1 − σ)).
            let x = rp.name(s.x).to_string();
            pointwise_with_sigma(out, &x, d_out, |dy, x, sg| format!("{dy} * ({sg} + {x} * {sg} * (1.0 - {sg}))"))
        }
        TensorOpSpec::Gelu(g) => {
            // y = x·s with s = σ(2z), z = √(2/π)·(x + 0.044715·x³): dy/dx = s + x·s·(1 − s)·2·√(2/π)·(1 + 3·0.044715·x²).
            let x = rp.name(g.x).to_string();
            gelu_rule(out, &x, d_out)
        }
        TensorOpSpec::SwiGlu(s) => {
            // y = silu(g)·u: dg = dy·u·silu'(g), du = dy·silu(g).
            let (g, u) = (rp.name(s.gate).to_string(), rp.name(s.up).to_string());
            swiglu_rule(out, &g, &u, d_out)
        }
        TensorOpSpec::L2Norm(l) => l2norm_rule(out, rp.name(l.x), l.eps, d_out, (l.rows, l.cols)),
        TensorOpSpec::RowGather(g) => {
            let table = rp.name(g.table).to_string();
            row_scatter_rule(rp, out, &table, d_out, &RowIndexText::of(rp, &g.index))
        }
        TensorOpSpec::RowsGather(g) => {
            let table = rp.name(g.table).to_string();
            let indices = rp.name(g.indices).to_string();
            out.undiff.insert(indices.clone(), "RowsGather");
            rows_scatter_rule(out, &table, &indices, d_out)
        }
        TensorOpSpec::ArgMax(_) => Err(VjpRefusal::NoGradient {
            op: "ArgMax",
            why: "the index of the largest element is piecewise constant in every input",
        }),
        TensorOpSpec::NgramIds(_) => Err(VjpRefusal::NoGradient {
            op: "NgramIds",
            why: "an n-gram's table rows are integers hashed from integer tokens",
        }),
        TensorOpSpec::Rope(r) => rope_rule(rp, r, out, d_out),
        TensorOpSpec::Decay(_) => Err(VjpRefusal::NotDerived { op: "Decay" }),
        TensorOpSpec::CausalConv(_) => Err(VjpRefusal::NotDerived { op: "CausalConv" }),
        TensorOpSpec::GatedDelta(_) => Err(VjpRefusal::NotDerived { op: "GatedDelta" }),
        TensorOpSpec::Softcap(c) => {
            // y = C·tanh(x/C): dx = dy · (1 − (y/C)²).
            let (x, y, cap) = (rp.name(c.x).to_string(), rp.name(c.out).to_string(), c.cap.get());
            pointwise(out, &x, d_out, &[&y], |v| format!("{0} * (1.0 - ({1} / {cap:?}) * ({1} / {cap:?}))", v[0], v[1]))
        }
        TensorOpSpec::Attention(a) => {
            let attended = match &a.pos {
                None => Attended::All(a.l),
                Some(p) => Attended::From(RowIndexText::of(rp, p).0),
            };
            let shape = AttentionShape {
                q: rp.name(a.q),
                k: rp.name(a.kc),
                v: rp.name(a.vc),
                rows: a.rows,
                h: a.h,
                kv: a.kv,
                hd: a.hd,
                vd: a.vd,
                l: a.l,
                attended,
                scale: a.scale.map_or(1.0 / (a.hd as f64).sqrt(), |s| s.get()),
                cap: a.softcap.map(|c| c.get()),
                window: a.window,
                sinks: a.sinks.map(|s| rp.name(s)),
            };
            attention_rule(out, shape, d_out)
        }
        TensorOpSpec::FullAttention(a) => attention_rule(
            out,
            AttentionShape {
                q: rp.name(a.q),
                k: rp.name(a.k),
                v: rp.name(a.v),
                rows: a.t,
                h: a.h,
                kv: a.h,
                hd: a.hd,
                vd: a.hd,
                l: a.t,
                attended: Attended::All(a.t),
                scale: 1.0 / (a.hd as f64).sqrt(),
                cap: None,
                window: None,
                sinks: None,
            },
            d_out,
        ),
        TensorOpSpec::Moe(_) => Err(VjpRefusal::NotDerived { op: "Moe" }),
        TensorOpSpec::SetRow(w) => set_row_rule(rp, w, out, d_out),
        TensorOpSpec::Scatter(_) => Err(VjpRefusal::NotDerived { op: "Scatter" }),
        TensorOpSpec::Map(_) => Err(VjpRefusal::NotDerived { op: "Map" }),
        TensorOpSpec::HeadSlice(h) => head_slice_rule(out, rp.name(h.x), h.every, h.from, h.width, h.n, d_out),
        TensorOpSpec::Join(j) => join_rule(out, (rp.name(j.a), rp.name(j.b)), (j.rows, j.heads, j.wa, j.wb), d_out),
        TensorOpSpec::LaneMean(m) => lane_mean_rule(out, rp.name(m.x), (m.rows, m.lanes, m.cols), d_out),
        TensorOpSpec::LaneSpread(p) => lane_spread_rule(out, (rp.name(p.x), p.by.map(|s| rp.name(s))), (p.rows, p.lanes, p.cols), d_out),
        TensorOpSpec::LaneSums(s) => lane_sums_rule(out, rp.name(s.x), (s.rows, s.lanes, s.cols), d_out),
        TensorOpSpec::SignedRoot(r) => {
            // y = sign(x)·√max(|x|, ε): dx = dy / (2√|x|) where |x| > ε, and 0 where the floor holds y constant.
            let x = rp.name(r.x).to_string();
            let eps = r.eps.get();
            let shape = out.shape(&x)?;
            let dx = out.tensor(&format!("g_{x}"), shape);
            let d_out = d_out.to_string();
            out.each(shape, |o, depth, _, _, k| {
                o.line(depth, format!("If abs(item {k} of {x}) > {eps:?}:"));
                o.line(depth + 1, format!("Set item {k} of {dx} to (item {k} of {d_out}) * 0.5 / sqrt(abs(item {k} of {x}))."));
            });
            out.accumulate(&x, dx);
            Ok(())
        }
        TensorOpSpec::HeadMatmul(m) => head_matmul_rule(out, (rp.name(m.x), rp.name(m.w)), (m.m, m.heads, m.a, m.b), d_out),
        TensorOpSpec::IntRequant(_) => Err(VjpRefusal::NoGradient {
            op: "IntRequant",
            why: "integer arithmetic: a floored shift of an integer is a step function of it",
        }),
        TensorOpSpec::IntClamp(_) => Err(VjpRefusal::NoGradient {
            op: "IntClamp",
            why: "integer arithmetic: a saturation of integers is a step function of them",
        }),
        TensorOpSpec::Relu(r) if r.int => Err(VjpRefusal::NoGradient {
            op: "Relu",
            why: "integer arithmetic: a ReLU of integers is a step function of them",
        }),
        TensorOpSpec::Relu(_) => Err(VjpRefusal::NotDerived { op: "Relu" }),
        TensorOpSpec::LutAttention(_) => Err(VjpRefusal::NoGradient {
            op: "LutAttention",
            why: "integer arithmetic: a table-weighted, floored mean of integers is a step function of them",
        }),
    }
}

/// `P[g·width + i] = X[g·every + from + i]` over the `n / every` groups of `X`: each slot it read takes the gradient of
/// the element it became, every other slot none.
fn head_slice_rule(out: &mut Out, x: &str, every: usize, from: usize, width: usize, n: usize, d_out: &str) -> Result<(), VjpRefusal> {
    let shape = out.shape(x)?;
    let dx = out.tensor(&format!("g_{x}"), shape);
    let (g, i, src, dst) = (out.names.fresh("g"), out.names.fresh("i"), out.names.fresh("k"), out.names.fresh("k"));
    out.line(0, format!("Repeat for {g} from 1 to {}:", n / every.max(1)));
    out.line(1, format!("Repeat for {i} from 1 to {width}:"));
    out.line(2, format!("Let {dst} be ({g} - 1) * {every} + {from} + {i}."));
    out.line(2, format!("Let {src} be ({g} - 1) * {width} + {i}."));
    out.line(2, format!("Set item {dst} of {dx} to item {src} of {d_out}."));
    out.accumulate(x, dx);
    Ok(())
}

/// `J` holds, per row and head, the head's part of `A` then its part of `B`: each part's gradient is the slice of `dJ`
/// it became.
fn join_rule(out: &mut Out, (a, b): (&str, &str), (rows, heads, wa, wb): (usize, usize, usize, usize), d_out: &str) -> Result<(), VjpRefusal> {
    let (sa, sb) = (out.shape(a)?, out.shape(b)?);
    let (da, db) = (out.tensor(&format!("g_{a}"), sa), out.tensor(&format!("g_{b}"), sb));
    let (ha, hb) = (wa / heads.max(1), wb / heads.max(1));
    let (r, h, i, j) = (out.names.fresh("r"), out.names.fresh("h"), out.names.fresh("i"), out.names.fresh("j"));
    let (at, ka, kj, kb, kk) = (out.names.fresh("k"), out.names.fresh("k"), out.names.fresh("k"), out.names.fresh("k"), out.names.fresh("k"));
    out.line(0, format!("Repeat for {r} from 1 to {rows}:"));
    out.line(1, format!("Repeat for {h} from 1 to {heads}:"));
    out.line(2, format!("Let {at} be ({r} - 1) * {} + ({h} - 1) * {}.", wa + wb, ha + hb));
    out.line(2, format!("Repeat for {i} from 1 to {ha}:"));
    out.line(3, format!("Let {ka} be ({r} - 1) * {wa} + ({h} - 1) * {ha} + {i}."));
    out.line(3, format!("Let {kj} be {at} + {i}."));
    out.line(3, format!("Set item {ka} of {da} to item {kj} of {d_out}."));
    out.line(2, format!("Repeat for {j} from 1 to {hb}:"));
    out.line(3, format!("Let {kb} be ({r} - 1) * {wb} + ({h} - 1) * {hb} + {j}."));
    out.line(3, format!("Let {kk} be {at} + {ha} + {j}."));
    out.line(3, format!("Set item {kb} of {db} to item {kk} of {d_out}."));
    out.accumulate(a, da);
    out.accumulate(b, db);
    Ok(())
}

/// `M[t][c] = Σ_l X[t][l·C + c] / L`: `dX[t][l·C + c] = dM[t][c] / L`, every lane its column's share.
fn lane_mean_rule(out: &mut Out, x: &str, (rows, lanes, cols): (usize, usize, usize), d_out: &str) -> Result<(), VjpRefusal> {
    let dx = out.tensor(&format!("g_{x}"), (rows, lanes * cols));
    let (t, l, c, k) = (out.names.fresh("t"), out.names.fresh("l"), out.names.fresh("c"), out.names.fresh("k"));
    out.line(0, format!("Repeat for {t} from 1 to {rows}:"));
    out.line(1, format!("Repeat for {l} from 1 to {lanes}:"));
    out.line(2, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(3, format!("Let {k} be ({t} - 1) * {} + ({l} - 1) * {cols} + {c}.", lanes * cols));
    out.line(3, format!("Set item {k} of {dx} to (item (({t} - 1) * {cols} + {c}) of {d_out}) / {lanes}.0."));
    out.accumulate(x, dx);
    Ok(())
}

/// `G[t][l] = Σ_c X[t][l·C + c]`: `dX[t][l·C + c] = dG[t][l]`, every column its lane's gradient.
fn lane_sums_rule(out: &mut Out, x: &str, (rows, lanes, cols): (usize, usize, usize), d_out: &str) -> Result<(), VjpRefusal> {
    let dx = out.tensor(&format!("g_{x}"), (rows, lanes * cols));
    let (t, l, c) = (out.names.fresh("t"), out.names.fresh("l"), out.names.fresh("c"));
    out.line(0, format!("Repeat for {t} from 1 to {rows}:"));
    out.line(1, format!("Repeat for {l} from 1 to {lanes}:"));
    out.line(2, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(
        3,
        format!(
            "Set item (({t} - 1) * {} + ({l} - 1) * {cols} + {c}) of {dx} to item (({t} - 1) * {lanes} + {l}) of {d_out}.",
            lanes * cols
        ),
    );
    out.accumulate(x, dx);
    Ok(())
}

/// `P[t][l·C + c] = O[t][c]·S[t][l]` (`S[t][l]` 1 without one): `dO[t][c] = Σ_l dP[t][l·C + c]·S[t][l]` and
/// `dS[t][l] = Σ_c dP[t][l·C + c]·O[t][c]`.
fn lane_spread_rule(
    out: &mut Out,
    (x, by): (&str, Option<&str>),
    (rows, lanes, cols): (usize, usize, usize),
    d_out: &str,
) -> Result<(), VjpRefusal> {
    let dx = out.tensor(&format!("g_{x}"), (rows, cols));
    let ds = by.map(|s| out.tensor(&format!("g_{s}"), (rows, lanes)));
    let (t, l, c, k, dp) = (out.names.fresh("t"), out.names.fresh("l"), out.names.fresh("c"), out.names.fresh("k"), out.names.fresh("y"));
    out.line(0, format!("Repeat for {t} from 1 to {rows}:"));
    out.line(1, format!("Repeat for {l} from 1 to {lanes}:"));
    out.line(2, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(3, format!("Let {dp} be item (({t} - 1) * {} + ({l} - 1) * {cols} + {c}) of {d_out}.", lanes * cols));
    out.line(3, format!("Let {k} be ({t} - 1) * {cols} + {c}."));
    match (by, &ds) {
        (Some(s), Some(ds)) => {
            let w = format!("(item (({t} - 1) * {lanes} + {l}) of {s})");
            out.line(3, format!("Set item {k} of {dx} to (item {k} of {dx}) + {dp} * {w}."));
            out.line(3, format!("Set item (({t} - 1) * {lanes} + {l}) of {ds} to (item (({t} - 1) * {lanes} + {l}) of {ds}) + {dp} * (item {k} of {x})."));
        }
        _ => out.line(3, format!("Set item {k} of {dx} to (item {k} of {dx}) + {dp}.")),
    }
    out.accumulate(x, dx);
    if let (Some(s), Some(ds)) = (by, ds) {
        out.accumulate(s, ds);
    }
    Ok(())
}

/// `Y[t][h·b + r] = Σ_e X[t][h·a + e]·W[h·a + e][r]`, one `a × b` block of `W` per head: `dX[t][h·a + e] =
/// Σ_r dY[t][h·b + r]·W[h·a + e][r]` and `dW[h·a + e][r] = Σ_t X[t][h·a + e]·dY[t][h·b + r]`.
fn head_matmul_rule(out: &mut Out, (x, w): (&str, &str), (m, heads, a, b): (usize, usize, usize, usize), d_out: &str) -> Result<(), VjpRefusal> {
    let (sx, sw) = (out.shape(x)?, out.shape(w)?);
    let (dx, dw) = (out.tensor(&format!("g_{x}"), sx), out.tensor(&format!("g_{w}"), sw));
    let (t, h, e, r) = (out.names.fresh("t"), out.names.fresh("h"), out.names.fresh("e"), out.names.fresh("r"));
    let (xk, wk, yk, dy) = (out.names.fresh("k"), out.names.fresh("k"), out.names.fresh("k"), out.names.fresh("y"));
    out.line(0, format!("Repeat for {t} from 1 to {m}:"));
    out.line(1, format!("Repeat for {h} from 1 to {heads}:"));
    out.line(2, format!("Repeat for {e} from 1 to {a}:"));
    out.line(3, format!("Let {xk} be ({t} - 1) * {} + ({h} - 1) * {a} + {e}.", heads * a));
    out.line(3, format!("Repeat for {r} from 1 to {b}:"));
    out.line(4, format!("Let {wk} be (({h} - 1) * {a} + {e} - 1) * {b} + {r}."));
    out.line(4, format!("Let {yk} be ({t} - 1) * {} + ({h} - 1) * {b} + {r}.", heads * b));
    out.line(4, format!("Let {dy} be item {yk} of {d_out}."));
    out.line(4, format!("Set item {xk} of {dx} to (item {xk} of {dx}) + {dy} * (item {wk} of {w})."));
    out.line(4, format!("Set item {wk} of {dw} to (item {wk} of {dw}) + {dy} * (item {xk} of {x})."));
    out.accumulate(x, dx);
    out.accumulate(w, dw);
    Ok(())
}

/// `dx = combine(dy, operands…)` element by element, every operand of `x`'s shape.
fn pointwise(out: &mut Out, x: &str, d_out: &str, operands: &[&str], combine: impl Fn(&[String]) -> String) -> Result<(), VjpRefusal> {
    let shape = out.shape(x)?;
    let dx = out.tensor(&format!("g_{x}"), shape);
    let operands: Vec<String> = operands.iter().map(|s| s.to_string()).collect();
    let d_out = d_out.to_string();
    out.each(shape, |o, depth, _, _, k| {
        let mut vals = vec![format!("(item {k} of {d_out})")];
        vals.extend(operands.iter().map(|t| format!("(item {k} of {t})")));
        o.line(depth, format!("Set item {k} of {dx} to {}.", combine(&vals)));
    });
    out.accumulate(x, dx);
    Ok(())
}

/// `σ(x)` as LOGOS: `1 / (1 + e^-x)` with `e` raised by `**`, the form the forward sugar uses.
fn sigma(x: &str) -> String {
    format!("(1.0 / (1.0 + {} ** (0.0 - {x})))", std::f64::consts::E)
}

fn pointwise_with_sigma(out: &mut Out, x: &str, d_out: &str, rule: impl Fn(&str, &str, &str) -> String) -> Result<(), VjpRefusal> {
    let shape = out.shape(x)?;
    let dx = out.tensor(&format!("g_{x}"), shape);
    let d_out = d_out.to_string();
    let x_name = x.to_string();
    out.each(shape, |o, depth, _, _, k| {
        let xv = o.names.fresh("x");
        let sg = o.names.fresh("s");
        o.line(depth, format!("Let {xv} be item {k} of {x_name}."));
        o.line(depth, format!("Let {sg} be {}.", sigma(&xv)));
        o.line(depth, format!("Set item {k} of {dx} to {}.", rule(&format!("(item {k} of {d_out})"), &xv, &sg)));
    });
    out.accumulate(x, dx);
    Ok(())
}

fn gelu_rule(out: &mut Out, x: &str, d_out: &str) -> Result<(), VjpRefusal> {
    const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
    const K: f64 = 0.044_715;
    let shape = out.shape(x)?;
    let dx = out.tensor(&format!("g_{x}"), shape);
    let d_out = d_out.to_string();
    let x_name = x.to_string();
    out.each(shape, |o, depth, _, _, k| {
        let xv = o.names.fresh("x");
        let z2 = o.names.fresh("z");
        let s = o.names.fresh("s");
        o.line(depth, format!("Let {xv} be item {k} of {x_name}."));
        o.line(depth, format!("Let {z2} be 2.0 * {SQRT_2_OVER_PI:?} * ({xv} + {K:?} * {xv} * {xv} * {xv})."));
        o.line(depth, format!("Let {s} be {}.", sigma(&z2)));
        o.line(
            depth,
            format!(
                "Set item {k} of {dx} to (item {k} of {d_out}) * ({s} + {xv} * {s} * (1.0 - {s}) * 2.0 * {SQRT_2_OVER_PI:?} * (1.0 + 3.0 * {K:?} * {xv} * {xv}))."
            ),
        );
    });
    out.accumulate(x, dx);
    Ok(())
}

fn swiglu_rule(out: &mut Out, g: &str, u: &str, d_out: &str) -> Result<(), VjpRefusal> {
    let shape = out.shape(g)?;
    let dg = out.tensor(&format!("g_{g}"), shape);
    let du = out.tensor(&format!("g_{u}"), shape);
    let (g_name, u_name, d_out) = (g.to_string(), u.to_string(), d_out.to_string());
    out.each(shape, |o, depth, _, _, k| {
        let gv = o.names.fresh("x");
        let sg = o.names.fresh("s");
        o.line(depth, format!("Let {gv} be item {k} of {g_name}."));
        o.line(depth, format!("Let {sg} be {}.", sigma(&gv)));
        o.line(
            depth,
            format!("Set item {k} of {dg} to (item {k} of {d_out}) * (item {k} of {u_name}) * ({sg} + {gv} * {sg} * (1.0 - {sg}))."),
        );
        o.line(depth, format!("Set item {k} of {du} to (item {k} of {d_out}) * {gv} * {sg}."));
    });
    out.accumulate(g, dg);
    out.accumulate(u, du);
    Ok(())
}

/// A loop over `rows` rows of `cols` elements in row-major storage — a tensor's own rows, or the heads of a per-head
/// norm's view of each row — binding the row `r` and the 1-based flat index `base` of the element before its first, so
/// element `c` of the row is `base + c`.
fn each_row(out: &mut Out, (rows, cols): (usize, usize)) -> (String, String) {
    let r = out.names.fresh("r");
    let base = out.names.fresh("b");
    out.line(0, format!("Repeat for {r} from 1 to {rows}:"));
    out.line(1, format!("Let {base} be ({r} - 1) * {cols}."));
    (r, base)
}

/// `Let mutable acc be 0.0.` then `Set acc to acc + term(k)` over the row's columns, at depth 1.
fn row_sum(out: &mut Out, cols: usize, base: &str, term: impl Fn(&str) -> String) -> String {
    let acc = out.names.fresh("a");
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    out.line(1, format!("Let mutable {acc} be 0.0."));
    out.line(1, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(2, format!("Let {k} be {base} + {c}."));
    out.line(2, format!("Set {acc} to {acc} + {}.", term(&k)));
    acc
}

/// Over the row's columns at depth 1: `Set item k of target to value(k)`.
fn row_write(out: &mut Out, cols: usize, base: &str, target: &str, value: impl Fn(&str) -> String) {
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    out.line(1, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(2, format!("Let {k} be {base} + {c}."));
    out.line(2, format!("Set item {k} of {target} to {}.", value(&k)));
}

fn softmax_rule(out: &mut Out, y: &str, d_out: &str, x: &str, shape: (usize, usize)) -> Result<(), VjpRefusal> {
    // Per row: dx = y ⊙ (dy − Σ dy⊙y).
    let dx = out.tensor(&format!("g_{x}"), out.shape(x)?);
    let (_, base) = each_row(out, shape);
    let (y, d_out) = (y.to_string(), d_out.to_string());
    let dot = row_sum(out, shape.1, &base, |k| format!("(item {k} of {d_out}) * (item {k} of {y})"));
    row_write(out, shape.1, &base, &dx, |k| format!("(item {k} of {y}) * ((item {k} of {d_out}) - {dot})"));
    out.accumulate(x, dx);
    Ok(())
}

fn l2norm_rule(out: &mut Out, x: &str, eps: Epsilon, d_out: &str, shape: (usize, usize)) -> Result<(), VjpRefusal> {
    // Per row, n = √(Σx² + ε): dx = dy/n − x·(Σ dy⊙x)/n³.
    let dx = out.tensor(&format!("g_{x}"), out.shape(x)?);
    let (_, base) = each_row(out, shape);
    let (x, d_out) = (x.to_string(), d_out.to_string());
    let ss = row_sum(out, shape.1, &base, |k| format!("(item {k} of {x}) * (item {k} of {x})"));
    let dot = row_sum(out, shape.1, &base, |k| format!("(item {k} of {d_out}) * (item {k} of {x})"));
    let n = out.names.fresh("n");
    out.line(1, format!("Let {n} be sqrt({ss} + {:?}).", eps.get()));
    row_write(out, shape.1, &base, &dx, |k| {
        format!("(item {k} of {d_out}) / {n} - (item {k} of {x}) * {dot} / ({n} * {n} * {n})")
    });
    out.accumulate(&x, dx);
    Ok(())
}

/// The weight's entry at column `c` of the row a rule's row loop is at, as LOGOS text: column `c` of the weight's one
/// row, or of the row's lane's row when the weight holds `weight_rows` rows — row `r` takes the weight's row
/// `(r − 1) mod weight_rows` ([`crate::tensor_recognize::FusedRowSpec::weight_rows`]), whose first entry this binds.
struct WeightRow(Option<String>);

impl WeightRow {
    fn of(out: &mut Out, row: &str, weight_rows: usize, cols: usize) -> WeightRow {
        WeightRow((weight_rows > 1).then(|| {
            let at = out.names.fresh("wb");
            out.line(1, format!("Let {at} be (({row} - 1) % {weight_rows}) * {cols}."));
            at
        }))
    }

    fn index(&self, c: &str) -> String {
        match &self.0 {
            Some(at) => format!("({at} + {c})"),
            None => c.to_string(),
        }
    }
}

fn rmsnorm_rule(
    rp: &RecognizedProgram,
    r: &crate::tensor_recognize::FusedRowSpec,
    out: &mut Out,
    d_out: &str,
    shape: (usize, usize),
) -> Result<(), VjpRefusal> {
    // u = x (+ residual); per row, q = 1/√(mean u² + ε); y = u·q·w.
    // du_i = q·(w_i dy_i) − q³/C · u_i · Σ_j w_j dy_j u_j;  dw_i = Σ_rows dy_i · u_i · q, over the rows that read w_i.
    let x = rp.name(r.x).to_string();
    let residual = r.residual.map(|s| rp.name(s).to_string());
    let weight = r.weight.map(|s| rp.name(s).to_string());
    let cols = shape.1;
    let du = out.tensor(&format!("g_{x}"), out.shape(&x)?);
    let dw = weight.as_ref().map(|w| {
        let s = out.shape(w).unwrap_or((r.weight_rows, cols));
        out.tensor(&format!("g_{w}"), s)
    });
    let d_out = d_out.to_string();
    let u_of = |k: &str| match &residual {
        Some(res) => format!("((item {k} of {x}) + (item {k} of {res}))"),
        None => format!("(item {k} of {x})"),
    };
    let (row, base) = each_row(out, shape);
    let lane = WeightRow::of(out, &row, r.weight_rows, cols);
    let w_of = |c_index: &str| match &weight {
        Some(w) => format!("(item {} of {w})", lane.index(c_index)),
        None => "1.0".to_string(),
    };
    let ss = row_sum(out, cols, &base, |k| format!("{} * {}", u_of(k), u_of(k)));
    let q = out.names.fresh("q");
    out.line(1, format!("Let {q} be 1.0 / sqrt({ss} / {cols}.0 + {:?}).", r.eps.get()));
    // Σ_j w_j dy_j u_j, with the column index recovered as k - base.
    let acc = out.names.fresh("a");
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    out.line(1, format!("Let mutable {acc} be 0.0."));
    out.line(1, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(2, format!("Let {k} be {base} + {c}."));
    out.line(2, format!("Set {acc} to {acc} + {} * (item {k} of {d_out}) * {}.", w_of(&c), u_of(&k)));
    let c2 = out.names.fresh("c");
    let k2 = out.names.fresh("k");
    out.line(1, format!("Repeat for {c2} from 1 to {cols}:"));
    out.line(2, format!("Let {k2} be {base} + {c2}."));
    out.line(
        2,
        format!(
            "Set item {k2} of {du} to {q} * {} * (item {k2} of {d_out}) - {q} * {q} * {q} / {cols}.0 * {} * {acc}.",
            w_of(&c2),
            u_of(&k2)
        ),
    );
    if let Some(dw) = &dw {
        let at = lane.index(&c2);
        out.line(2, format!("Set item {at} of {dw} to (item {at} of {dw}) + (item {k2} of {d_out}) * {} * {q}.", u_of(&k2)));
    }
    out.accumulate(&x, du.clone());
    if let Some(res) = residual {
        out.accumulate(&res, du);
    }
    if let (Some(w), Some(dw)) = (weight, dw) {
        out.accumulate(&w, dw);
    }
    Ok(())
}

fn layernorm_rule(
    rp: &RecognizedProgram,
    r: &crate::tensor_recognize::FusedRowSpec,
    out: &mut Out,
    d_out: &str,
    shape: (usize, usize),
) -> Result<(), VjpRefusal> {
    // Per row: μ = mean x, v = mean (x−μ)², q = 1/√(v + ε), x̂ = (x−μ)·q, y = x̂·w + b.
    // g = dy·w;  dx = q·(g − mean g − x̂·mean(g·x̂));  dw = Σ_rows dy·x̂;  db = Σ_rows dy.
    let x = rp.name(r.x).to_string();
    let weight = r.weight.map(|s| rp.name(s).to_string());
    let bias = r.bias.map(|s| rp.name(s).to_string());
    let cols = shape.1;
    let dx = out.tensor(&format!("g_{x}"), out.shape(&x)?);
    let dw = weight.as_ref().map(|w| {
        let s = out.shape(w).unwrap_or((r.weight_rows, cols));
        out.tensor(&format!("g_{w}"), s)
    });
    let db = bias.as_ref().map(|b| {
        let s = out.shape(b).unwrap_or((1, cols));
        out.tensor(&format!("g_{b}"), s)
    });
    let d_out = d_out.to_string();
    let (row, base) = each_row(out, shape);
    let lane = WeightRow::of(out, &row, r.weight_rows, cols);
    let w_of = |c_index: &str| match &weight {
        Some(w) => format!("(item {} of {w})", lane.index(c_index)),
        None => "1.0".to_string(),
    };
    let sum = row_sum(out, cols, &base, |k| format!("(item {k} of {x})"));
    let mu = out.names.fresh("m");
    out.line(1, format!("Let {mu} be {sum} / {cols}.0."));
    let var = row_sum(out, cols, &base, |k| format!("((item {k} of {x}) - {mu}) * ((item {k} of {x}) - {mu})"));
    let q = out.names.fresh("q");
    out.line(1, format!("Let {q} be 1.0 / sqrt({var} / {cols}.0 + {:?}).", r.eps.get()));
    let (g_sum, gx_sum) = (out.names.fresh("a"), out.names.fresh("a"));
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    out.line(1, format!("Let mutable {g_sum} be 0.0."));
    out.line(1, format!("Let mutable {gx_sum} be 0.0."));
    out.line(1, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(2, format!("Let {k} be {base} + {c}."));
    out.line(2, format!("Set {g_sum} to {g_sum} + (item {k} of {d_out}) * {}.", w_of(&c)));
    out.line(2, format!("Set {gx_sum} to {gx_sum} + (item {k} of {d_out}) * {} * ((item {k} of {x}) - {mu}) * {q}.", w_of(&c)));
    let c2 = out.names.fresh("c");
    let k2 = out.names.fresh("k");
    out.line(1, format!("Repeat for {c2} from 1 to {cols}:"));
    out.line(2, format!("Let {k2} be {base} + {c2}."));
    out.line(
        2,
        format!(
            "Set item {k2} of {dx} to {q} * ((item {k2} of {d_out}) * {} - {g_sum} / {cols}.0 - ((item {k2} of {x}) - {mu}) * {q} * {gx_sum} / {cols}.0).",
            w_of(&c2)
        ),
    );
    if let Some(dw) = &dw {
        let at = lane.index(&c2);
        out.line(2, format!("Set item {at} of {dw} to (item {at} of {dw}) + (item {k2} of {d_out}) * ((item {k2} of {x}) - {mu}) * {q}."));
    }
    if let Some(db) = &db {
        out.line(2, format!("Set item {c2} of {db} to (item {c2} of {db}) + (item {k2} of {d_out})."));
    }
    out.accumulate(&x, dx);
    if let (Some(w), Some(dw)) = (weight, dw) {
        out.accumulate(&w, dw);
    }
    if let (Some(b), Some(db)) = (bias, db) {
        out.accumulate(&b, db);
    }
    Ok(())
}

/// A row index as LOGOS text: a literal, or the name of the variable holding it.
struct RowIndexText(String);

impl RowIndexText {
    fn of(rp: &RecognizedProgram, index: &crate::tensor_recognize::qwen::RowIndex) -> RowIndexText {
        use crate::tensor_recognize::qwen::RowIndex;
        RowIndexText(match index {
            RowIndex::Literal(n) => n.to_string(),
            RowIndex::Named(s) => rp.name(*s).to_string(),
        })
    }
}

fn row_scatter_rule(_rp: &RecognizedProgram, out: &mut Out, table: &str, d_out: &str, index: &RowIndexText) -> Result<(), VjpRefusal> {
    // x = row i of T: dT[i, :] += dx.
    let shape = out.shape(table)?;
    let dt = out.tensor(&format!("g_{table}"), shape);
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    out.line(0, format!("Repeat for {c} from 1 to {}:", shape.1));
    out.line(1, format!("Let {k} be ({} - 1) * {} + {c}.", index.0, shape.1));
    out.line(1, format!("Set item {k} of {dt} to (item {k} of {dt}) + (item {c} of {d_out})."));
    out.accumulate(table, dt);
    Ok(())
}

fn rows_scatter_rule(out: &mut Out, table: &str, indices: &str, d_out: &str) -> Result<(), VjpRefusal> {
    // X = rows I of T: dT[I[r], :] += dX[r, :] for every r.
    let shape = out.shape(table)?;
    let (rows, _) = out.shape(d_out).unwrap_or((1, shape.1));
    let dt = out.tensor(&format!("g_{table}"), shape);
    let r = out.names.fresh("r");
    let c = out.names.fresh("c");
    let k = out.names.fresh("k");
    let src = out.names.fresh("k");
    let mut header = String::new();
    let _ = write!(header, "Repeat for {r} from 1 to {rows}:");
    out.line(0, header);
    out.line(1, format!("Repeat for {c} from 1 to {}:", shape.1));
    out.line(2, format!("Let {k} be ((item {r} of {indices}) - 1) * {} + {c}.", shape.1));
    out.line(2, format!("Let {src} be ({r} - 1) * {} + {c}.", shape.1));
    out.line(2, format!("Set item {k} of {dt} to (item {k} of {dt}) + (item {src} of {d_out})."));
    out.accumulate(table, dt);
    Ok(())
}

fn rope_rule(rp: &RecognizedProgram, r: &crate::tensor_recognize::qwen::RopeSpec, out: &mut Out, d_out: &str) -> Result<(), VjpRefusal> {
    // A rotation is orthogonal and R(θ)ᵀ = R(−θ): the gradient is the same rope with the sine table negated, so
    // every option — a partial slice, a row of angles per row, angles from a cache position — carries over as is.
    let (q, rc, rs) = (rp.name(r.q).to_string(), rp.name(r.rc).to_string(), rp.name(r.rs).to_string());
    let table = out.shape(&rs)?;
    let neg = out.tensor(&format!("n_{rs}"), table);
    let rs_name = rs.clone();
    out.each(table, |o, depth, _, _, k| {
        o.line(depth, format!("Set item {k} of {neg} to 0.0 - (item {k} of {rs_name})."));
    });
    let shape = out.shape(&q)?;
    let dq = out.names.fresh(&format!("g_{q}"));
    let mut stmt = format!("Let {dq} be rope of {d_out} with cos {rc} and sin {neg}");
    if let Some((index, _)) = &r.from {
        stmt.push_str(&format!(" from position {}", RowIndexText::of(rp, index).0));
    }
    if r.hd != 2 * r.half {
        stmt.push_str(&format!(" across {} heads", r.h));
    }
    stmt.push('.');
    out.line(0, stmt);
    out.shapes.insert(dq.clone(), shape);
    out.accumulate(&q, dq);
    out.undiff.insert(rc, "Rope");
    out.undiff.insert(rs, "Rope");
    Ok(())
}

/// How many cached positions a query row attends.
enum Attended {
    /// Every one of the `l` positions.
    All(usize),
    /// The first `P` (a literal or a variable) — and, in a block of query rows, row `r` (1-based) the first
    /// `P + r − 1`.
    From(String),
}

struct AttentionShape<'n> {
    q: &'n str,
    k: &'n str,
    v: &'n str,
    rows: usize,
    h: usize,
    kv: usize,
    hd: usize,
    /// The value head width: `hd`, or other.
    vd: usize,
    l: usize,
    attended: Attended,
    /// The factor every score takes: `1/√hd` unless the program states one.
    scale: f64,
    /// The soft cap every scaled score takes, `C·tanh(s/C)`.
    cap: Option<f64>,
    window: Option<crate::tensor_recognize::qwen::Window>,
    /// The tensor of each head's sink logit (1 by `h`), which joins every row's softmax.
    sinks: Option<&'n str>,
}

fn attention_rule(out: &mut Out, a: AttentionShape, d_out: &str) -> Result<(), VjpRefusal> {
    // Per query row r and head g over its attended positions j (from the window's floor), with kv head
    // g' = (g − 1)/(H/KV) + 1, the scale α, the cap C and the head's sink σ_g:
    //   u_j = α·q·k_j,  s_j = C·tanh(u_j/C) (u_j uncapped),  p = softmax(s, σ_g),  c = Σ p_j v_j
    //   dv_j += p_j·dc,  dp_j = dc·v_j,  ds_j = p_j·(dp_j − Σ_i p_i dp_i),  du_j = ds_j·(1 − (s_j/C)²),
    //   dq += α Σ_j du_j k_j,  dk_j += α du_j q,  dσ_g += −p_σ·Σ_i p_i dp_i.
    // The forward keeps no intermediates, so the scores and the softmax are recomputed here.
    let (q, k, v) = (a.q.to_string(), a.k.to_string(), a.v.to_string());
    let (width, owidth) = (a.h * a.hd, a.h * a.vd);
    let (kwidth, vwidth) = (a.kv * a.hd, a.kv * a.vd);
    let group = a.h / a.kv;
    let alpha = a.scale;
    let e = std::f64::consts::E;
    let dq = out.tensor(&format!("g_{q}"), (a.rows, width));
    let dk = out.tensor(&format!("g_{k}"), (a.l, kwidth));
    let dv = out.tensor(&format!("g_{v}"), (a.l, vwidth));
    let dsink = a.sinks.map(|s| out.tensor(&format!("g_{s}"), (1, a.h)));
    let n = |o: &mut Names| o.fresh("n");
    let (r, g, kvh, cnt) = (out.names.fresh("r"), out.names.fresh("h"), out.names.fresh("kh"), n(&mut out.names));
    let (qo, oo, j, ko, vo, d) =
        (out.names.fresh("qo"), out.names.fresh("oo"), out.names.fresh("j"), out.names.fresh("ko"), out.names.fresh("vo"), out.names.fresh("d"));
    let (lo, sc, ps, dps) = (out.names.fresh("lo"), out.names.fresh("sc"), out.names.fresh("ps"), out.names.fresh("dp"));
    let (acc, mx, den, dot) = (out.names.fresh("a"), out.names.fresh("m"), out.names.fresh("z"), out.names.fresh("t"));
    out.line(0, format!("Repeat for {r} from 1 to {}:", a.rows));
    out.line(1, format!("Repeat for {g} from 1 to {}:", a.h));
    out.line(2, format!("Let {kvh} be ({g} - 1) / {group} + 1."));
    match &a.attended {
        Attended::All(l) => out.line(2, format!("Let {cnt} be {l}.")),
        Attended::From(p) => out.line(2, format!("Let {cnt} be {p} + {r} - 1.")),
    }
    // The window's floor: the last W positions, or the query's own chunk of C.
    use crate::tensor_recognize::qwen::Window;
    match a.window {
        None => out.line(2, format!("Let {lo} be 1.")),
        Some(Window::Sliding(w)) => {
            out.line(2, format!("Let mutable {lo} be {cnt} - {w} + 1."));
            out.line(2, format!("If {lo} < 1:"));
            out.line(3, format!("Set {lo} to 1."));
        }
        Some(Window::Chunked(c)) => out.line(2, format!("Let {lo} be (({cnt} - 1) / {c}) * {c} + 1.")),
    }
    out.line(2, format!("Let {qo} be ({r} - 1) * {width} + ({g} - 1) * {}.", a.hd));
    out.line(2, format!("Let {oo} be ({r} - 1) * {owidth} + ({g} - 1) * {}.", a.vd));
    // scores and their max, the head's sink in the max where it has one
    out.line(2, format!("Let mutable {sc} be a new Seq of Real."));
    match a.sinks {
        Some(sinks) => out.line(2, format!("Let mutable {mx} be item {g} of {sinks}.")),
        None => out.line(2, format!("Let mutable {mx} be 0.0.")),
    }
    out.line(2, format!("Repeat for {j} from {lo} to {cnt}:"));
    out.line(3, format!("Let {ko} be ({j} - 1) * {kwidth} + ({kvh} - 1) * {}.", a.hd));
    out.line(3, format!("Let mutable {acc} be 0.0."));
    out.line(3, format!("Repeat for {d} from 1 to {}:", a.hd));
    out.line(4, format!("Set {acc} to {acc} + (item ({qo} + {d}) of {q}) * (item ({ko} + {d}) of {k})."));
    out.line(3, format!("Set {acc} to {acc} * {alpha:?}."));
    if let Some(c) = a.cap {
        out.line(3, format!("Set {acc} to {c:?} * (1.0 - 2.0 / ({e:?} ** ({acc} * {:?}) + 1.0)).", 2.0 / c));
    }
    let first = if a.sinks.is_some() { String::new() } else { format!("{j} == {lo} or ") };
    out.line(3, format!("If {first}{acc} > {mx}:"));
    out.line(4, format!("Set {mx} to {acc}."));
    out.line(3, format!("Push {acc} to {sc}."));
    // softmax, the sink's term in the denominator
    match a.sinks {
        Some(sinks) => out.line(2, format!("Let mutable {den} be {e:?} ** ((item {g} of {sinks}) - {mx}).")),
        None => out.line(2, format!("Let mutable {den} be 0.0.")),
    }
    out.line(2, format!("Let mutable {ps} be a new Seq of Real."));
    out.line(2, format!("Repeat for {j} from {lo} to {cnt}:"));
    out.line(3, format!("Let {acc}_e be {e:?} ** ((item ({j} - {lo} + 1) of {sc}) - {mx})."));
    out.line(3, format!("Set {den} to {den} + {acc}_e."));
    out.line(3, format!("Push {acc}_e to {ps}."));
    // dp, dv, and Σ p·dp
    out.line(2, format!("Let mutable {dps} be a new Seq of Real."));
    out.line(2, format!("Let mutable {dot} be 0.0."));
    out.line(2, format!("Repeat for {j} from {lo} to {cnt}:"));
    out.line(3, format!("Let {vo} be ({j} - 1) * {vwidth} + ({kvh} - 1) * {}.", a.vd));
    out.line(3, format!("Let {acc}_p be (item ({j} - {lo} + 1) of {ps}) / {den}."));
    out.line(3, format!("Let mutable {acc} be 0.0."));
    out.line(3, format!("Repeat for {d} from 1 to {}:", a.vd));
    out.line(4, format!("Set {acc} to {acc} + (item ({oo} + {d}) of {d_out}) * (item ({vo} + {d}) of {v})."));
    out.line(4, format!("Set item ({vo} + {d}) of {dv} to (item ({vo} + {d}) of {dv}) + {acc}_p * (item ({oo} + {d}) of {d_out})."));
    out.line(3, format!("Push {acc} to {dps}."));
    out.line(3, format!("Set {dot} to {dot} + {acc}_p * {acc}."));
    // dσ: the sink's weight against the row's expected dp
    if let (Some(sinks), Some(dsink)) = (a.sinks, &dsink) {
        out.line(2, format!("Let {acc}_sink be {e:?} ** ((item {g} of {sinks}) - {mx}) / {den}."));
        out.line(2, format!("Set item {g} of {dsink} to (item {g} of {dsink}) - {acc}_sink * {dot}."));
    }
    // ds through the cap, dq, dk
    out.line(2, format!("Repeat for {j} from {lo} to {cnt}:"));
    out.line(3, format!("Let {ko} be ({j} - 1) * {kwidth} + ({kvh} - 1) * {}.", a.hd));
    let through_cap = match a.cap {
        Some(c) => format!(" * (1.0 - ((item ({j} - {lo} + 1) of {sc}) / {c:?}) * ((item ({j} - {lo} + 1) of {sc}) / {c:?}))"),
        None => String::new(),
    };
    out.line(
        3,
        format!("Let {acc}_s be (item ({j} - {lo} + 1) of {ps}) / {den} * ((item ({j} - {lo} + 1) of {dps}) - {dot}){through_cap} * {alpha:?}."),
    );
    out.line(3, format!("Repeat for {d} from 1 to {}:", a.hd));
    out.line(4, format!("Set item ({qo} + {d}) of {dq} to (item ({qo} + {d}) of {dq}) + {acc}_s * (item ({ko} + {d}) of {k})."));
    out.line(4, format!("Set item ({ko} + {d}) of {dk} to (item ({ko} + {d}) of {dk}) + {acc}_s * (item ({qo} + {d}) of {q})."));
    out.accumulate(&q, dq);
    out.accumulate(&k, dk);
    out.accumulate(&v, dv);
    if let (Some(sinks), Some(dsink)) = (a.sinks, dsink) {
        out.accumulate(sinks, dsink);
    }
    Ok(())
}

fn set_row_rule(rp: &RecognizedProgram, w: &crate::tensor_recognize::qwen::SetRowSpec, out: &mut Out, d_out: &str) -> Result<(), VjpRefusal> {
    // `Set rows from P of T to X` replaces rows P … P + count − 1 of T: X's gradient is those rows of T's gradient
    // after the write, and T's gradient before the write is the same with those rows zeroed — the values they held
    // never reach the loss.
    let (table, source) = (rp.name(w.table).to_string(), rp.name(w.source).to_string());
    let p = RowIndexText::of(rp, &w.index).0;
    let cols = w.cols;
    let dx = out.tensor(&format!("g_{source}"), (w.count, cols));
    let before = out.tensor(&format!("g_{table}"), (w.rows, cols));
    let (r, c, k) = (out.names.fresh("r"), out.names.fresh("c"), out.names.fresh("k"));
    out.line(0, format!("Repeat for {r} from 1 to {}:", w.rows));
    out.line(1, format!("Repeat for {c} from 1 to {cols}:"));
    out.line(2, format!("Let {k} be ({r} - 1) * {cols} + {c}."));
    out.line(2, format!("If {r} >= {p} and {r} < {p} + {}:", w.count));
    out.line(3, format!("Set item (({r} - {p}) * {cols} + {c}) of {dx} to item {k} of {d_out}."));
    out.line(2, "Otherwise:".to_string());
    out.line(3, format!("Set item {k} of {before} to item {k} of {d_out}."));
    out.grads.insert(table.clone(), before);
    out.accumulate(&source, dx);
    if let crate::tensor_recognize::qwen::RowIndex::Named(s) = &w.index {
        out.undiff.insert(rp.name(*s).to_string(), "SetRow");
    }
    Ok(())
}
