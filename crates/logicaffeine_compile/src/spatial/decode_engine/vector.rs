//! The vector unit's instruction set: sixteen f32 lanes over vector and scalar registers, every instruction one of
//! the engine's f32 operations in the order [`super::numerics`] defines it, so a micro-program computes what the
//! numerics function it is written from computes, bit for bit.
//!
//! An instruction over a vector runs element `i` in lane `i mod 16`: elementwise ones lane by lane, `Dot` and `Sum`
//! accumulating each lane in index order and folding the lanes by the pairwise tree ([`super::numerics::fdot`]),
//! `Max`, `AbsMax` and `ArgMax` each lane's largest folded the same way, an order the result does not depend on
//! ([`super::numerics::fmax`], [`super::numerics::absmax`], [`super::numerics::argmax`]). Scalars are the
//! reductions' results and the constants a program loads. The micro-programs here are the norms, the softmax, the
//! gated products, the decay a Gated-DeltaNet block takes, the rotary turn, a cache row's int8 codes, the causal
//! convolution and the greedy token, each the numerics function it replaces.

use super::numerics::{
    absmax, argmax, exp, f32_to_f16, fadd, fdot, fmax, fsub, fsum, hadamard, int8_codes, ldexp, ln1p, recip, rope, rsqrt,
    sigmoid, silu, softplus, FpUnit, INV_127,
};
use crate::model::quant::f16_to_f32;

/// A vector register.
pub type VReg = u8;
/// A scalar register.
pub type SReg = u8;

/// A function a lane applies to one element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Exp,
    Recip,
    Rsqrt,
    Sigmoid,
    Silu,
    Softplus,
    Ln1p,
    /// The sign flipped, exactly.
    Neg,
    /// Rounded to binary16 and read back ([`super::numerics::f32_to_f16`]): a value as an f16 store keeps it.
    F16,
}

impl Func {
    /// `f(x)`, its products on `fp`.
    pub fn apply(self, fp: FpUnit, x: f32) -> f32 {
        match self {
            Func::Exp => exp(fp, x),
            Func::Recip => recip(fp, x),
            Func::Rsqrt => rsqrt(fp, x),
            Func::Sigmoid => sigmoid(fp, x),
            Func::Silu => silu(fp, x),
            Func::Softplus => softplus(fp, x),
            Func::Ln1p => ln1p(fp, x),
            Func::Neg => -x,
            Func::F16 => f16_to_f32(f32_to_f16(x)),
        }
    }
}

/// A two-operand f32 operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bin {
    Mul,
    Add,
    Sub,
}

impl Bin {
    /// `a ∘ b`, a product on `fp`.
    pub fn apply(self, fp: FpUnit, a: f32, b: f32) -> f32 {
        match self {
            Bin::Mul => fp.mul(a, b),
            Bin::Add => fadd(a, b),
            Bin::Sub => fsub(a, b),
        }
    }
}

/// One instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// `s[d] = Σ v[a]_i·v[b]_i`, lanes then the tree.
    Dot { a: VReg, b: VReg, d: SReg },
    /// `s[d] = Σ v[a]_i`, lanes then the tree.
    Sum { a: VReg, d: SReg },
    /// `s[d] = max_i v[a]_i` ([`super::numerics::fmax`]).
    Max { a: VReg, d: SReg },
    /// `v[d]_i = v[a]_i ∘ v[b]_i`.
    Vec { op: Bin, a: VReg, b: VReg, d: VReg },
    /// `v[d]_i = v[a]_i ∘ s[s]`.
    VecScalar { op: Bin, a: VReg, s: SReg, d: VReg },
    /// `v[d]_i = f(v[a]_i)`.
    Map { f: Func, a: VReg, d: VReg },
    /// `s[d] = s[a] ∘ s[b]`.
    Scalar { op: Bin, a: SReg, b: SReg, d: SReg },
    /// `s[d] = f(s[a])`.
    ScalarMap { f: Func, a: SReg, d: SReg },
    /// `s[d] = value`.
    Const { value: f32, d: SReg },
    /// `v[d] = v[a]` rotated by the Walsh–Hadamard transform, unnormalized.
    Hadamard { a: VReg, d: VReg },
    /// `v[d] = v[a]` with its first `2·len(v[cos])` elements turned in pairs by the rotary table `v[cos]`, `v[sin]`
    /// ([`super::numerics::rope`]): pair `i` is `(i, i + len)` under NEOX, `(2i, 2i + 1)` otherwise.
    Rotate { a: VReg, cos: VReg, sin: VReg, d: VReg, neox: bool },
    /// `s[d]` = the index of `v[a]`'s largest element, the first of equals, as an f32 ([`super::numerics::argmax`]).
    ArgMax { a: VReg, d: SReg },
    /// `s[d]` = the largest magnitude in `v[a]` ([`super::numerics::absmax`]).
    AbsMax { a: VReg, d: SReg },
    /// `v[d]_i` = the int8 code of `v[a]_i` under the inverse scale `s[s]`, as an f32
    /// ([`super::numerics::int8_codes`]).
    Codes { a: VReg, s: SReg, d: VReg },
    /// `v[d]_i = v[a]_i·2^n` ([`super::numerics::ldexp`]): exact while normal, infinite past the top, a zero of its sign
    /// below the normals — not a product, which rounds a value just below the normals up into them.
    Scale { a: VReg, n: i8, d: VReg },
    /// `v[d]_i = v[c]_i + v[a]_i·v[b]_i`: the product rounded, then the sum — a `Mul` then an `Add`, in one pass.
    MulAdd { a: VReg, b: VReg, c: VReg, d: VReg },
    /// `v[d]_i = v[a]_i·v[b]_i + s[s]`: the product rounded, then the sum.
    MulAddScalar { a: VReg, b: VReg, s: SReg, d: VReg },
}

impl Op {
    /// The vector and scalar registers the instruction reads.
    pub fn reads(self) -> (Vec<VReg>, Vec<SReg>) {
        match self {
            Op::Dot { a, b, .. } | Op::Vec { a, b, .. } => (vec![a, b], vec![]),
            Op::Sum { a, .. } | Op::Max { a, .. } | Op::Map { a, .. } | Op::Hadamard { a, .. } | Op::ArgMax { a, .. } | Op::AbsMax { a, .. } | Op::Scale { a, .. } => (vec![a], vec![]),
            Op::VecScalar { a, s, .. } | Op::Codes { a, s, .. } => (vec![a], vec![s]),
            Op::Scalar { a, b, .. } => (vec![], vec![a, b]),
            Op::ScalarMap { a, .. } => (vec![], vec![a]),
            Op::Const { .. } => (vec![], vec![]),
            Op::Rotate { a, cos, sin, .. } => (vec![a, cos, sin], vec![]),
            Op::MulAdd { a, b, c, .. } => (vec![a, b, c], vec![]),
            Op::MulAddScalar { a, b, s, .. } => (vec![a, b], vec![s]),
        }
    }

    /// The scalar register the instruction writes, if it writes one.
    pub fn writes_scalar(self) -> Option<SReg> {
        match self {
            Op::Dot { d, .. } | Op::Sum { d, .. } | Op::Max { d, .. } | Op::Scalar { d, .. } | Op::ScalarMap { d, .. } | Op::Const { d, .. } | Op::ArgMax { d, .. } | Op::AbsMax { d, .. } => Some(d),
            Op::Vec { .. }
            | Op::VecScalar { .. }
            | Op::Map { .. }
            | Op::Hadamard { .. }
            | Op::Rotate { .. }
            | Op::Codes { .. }
            | Op::Scale { .. }
            | Op::MulAdd { .. }
            | Op::MulAddScalar { .. } => None,
        }
    }

    /// The vector register the instruction writes, if it writes one.
    pub fn writes(self) -> Option<VReg> {
        match self {
            Op::Vec { d, .. }
            | Op::VecScalar { d, .. }
            | Op::Map { d, .. }
            | Op::Hadamard { d, .. }
            | Op::Rotate { d, .. }
            | Op::Codes { d, .. }
            | Op::Scale { d, .. }
            | Op::MulAdd { d, .. }
            | Op::MulAddScalar { d, .. } => Some(d),
            Op::Dot { .. } | Op::Sum { .. } | Op::Max { .. } | Op::Scalar { .. } | Op::ScalarMap { .. } | Op::Const { .. } | Op::ArgMax { .. } | Op::AbsMax { .. } => None,
        }
    }
}

/// The registers a program runs over.
#[derive(Clone, Debug, Default)]
pub struct Regs {
    pub v: Vec<Vec<f32>>,
    pub s: Vec<f32>,
}

impl Regs {
    pub fn new(vectors: usize, scalars: usize) -> Regs {
        Regs { v: vec![Vec::new(); vectors], s: vec![0.0; scalars] }
    }
}

/// Run `program` over `regs`, every product on `fp`.
pub fn run(fp: FpUnit, program: &[Op], regs: &mut Regs) {
    for &op in program {
        match op {
            Op::Dot { a, b, d } => regs.s[d as usize] = fdot(fp, &regs.v[a as usize], &regs.v[b as usize]),
            Op::Sum { a, d } => regs.s[d as usize] = fsum(&regs.v[a as usize]),
            Op::Max { a, d } => regs.s[d as usize] = fmax(&regs.v[a as usize]),
            Op::Vec { op, a, b, d } => {
                let out: Vec<f32> = regs.v[a as usize].iter().zip(&regs.v[b as usize]).map(|(&x, &y)| op.apply(fp, x, y)).collect();
                regs.v[d as usize] = out;
            }
            Op::VecScalar { op, a, s, d } => {
                let k = regs.s[s as usize];
                let out: Vec<f32> = regs.v[a as usize].iter().map(|&x| op.apply(fp, x, k)).collect();
                regs.v[d as usize] = out;
            }
            Op::Map { f, a, d } => {
                let out: Vec<f32> = regs.v[a as usize].iter().map(|&x| f.apply(fp, x)).collect();
                regs.v[d as usize] = out;
            }
            Op::Scalar { op, a, b, d } => regs.s[d as usize] = op.apply(fp, regs.s[a as usize], regs.s[b as usize]),
            Op::ScalarMap { f, a, d } => regs.s[d as usize] = f.apply(fp, regs.s[a as usize]),
            Op::Const { value, d } => regs.s[d as usize] = value,
            Op::Hadamard { a, d } => {
                let mut out = regs.v[a as usize].clone();
                hadamard(&mut out);
                regs.v[d as usize] = out;
            }
            Op::Rotate { a, cos, sin, d, neox } => {
                let mut out = regs.v[a as usize].clone();
                rope(fp, &mut out, &regs.v[cos as usize], &regs.v[sin as usize], neox);
                regs.v[d as usize] = out;
            }
            Op::ArgMax { a, d } => regs.s[d as usize] = argmax(&regs.v[a as usize]) as f32,
            Op::AbsMax { a, d } => regs.s[d as usize] = absmax(&regs.v[a as usize]),
            Op::Codes { a, s, d } => {
                let out: Vec<f32> = int8_codes(fp, &regs.v[a as usize], regs.s[s as usize]).into_iter().map(f32::from).collect();
                regs.v[d as usize] = out;
            }
            Op::Scale { a, n, d } => {
                let out: Vec<f32> = regs.v[a as usize].iter().map(|&x| ldexp(x, n as i32)).collect();
                regs.v[d as usize] = out;
            }
            Op::MulAdd { a, b, c, d } => {
                let (x, y, z) = (&regs.v[a as usize], &regs.v[b as usize], &regs.v[c as usize]);
                let out: Vec<f32> = x.iter().zip(y).zip(z).map(|((&x, &y), &z)| Bin::Add.apply(fp, z, Bin::Mul.apply(fp, x, y))).collect();
                regs.v[d as usize] = out;
            }
            Op::MulAddScalar { a, b, s, d } => {
                let k = regs.s[s as usize];
                let out: Vec<f32> = regs.v[a as usize].iter().zip(&regs.v[b as usize]).map(|(&x, &y)| Bin::Add.apply(fp, Bin::Mul.apply(fp, x, y), k)).collect();
                regs.v[d as usize] = out;
            }
        }
    }
}

// ── The micro-programs ──────────────────────────────────────────────────────────────────────────────

/// RMSNorm of `v[x]` by the weights `v[w]` into `v[out]`, `n` the vector's length: `rsqrt(Σx²·(1/n) + ε)`, then each
/// `x·r·w`, `1/n` the reciprocal at `fp` held as a constant. Scalars 0..2 are its scratch, `v[t]` too.
pub fn rmsnorm(fp: FpUnit, x: VReg, w: VReg, out: VReg, t: VReg, n: usize, eps: f32) -> Vec<Op> {
    vec![
        Op::Dot { a: x, b: x, d: 0 },
        Op::Const { value: recip(fp, n as f32), d: 1 },
        Op::Scalar { op: Bin::Mul, a: 0, b: 1, d: 0 },
        Op::Const { value: eps, d: 2 },
        Op::Scalar { op: Bin::Add, a: 0, b: 2, d: 0 },
        Op::ScalarMap { f: Func::Rsqrt, a: 0, d: 0 },
        Op::VecScalar { op: Bin::Mul, a: x, s: 0, d: t },
        Op::Vec { op: Bin::Mul, a: t, b: w, d: out },
    ]
}

/// L2 normalization of `v[x]` into `v[out]`: `x·rsqrt(Σx² + ε)`. Scalars 0..1 are its scratch.
pub fn l2norm(x: VReg, out: VReg, eps: f32) -> Vec<Op> {
    vec![
        Op::Dot { a: x, b: x, d: 0 },
        Op::Const { value: eps, d: 1 },
        Op::Scalar { op: Bin::Add, a: 0, b: 1, d: 0 },
        Op::ScalarMap { f: Func::Rsqrt, a: 0, d: 0 },
        Op::VecScalar { op: Bin::Mul, a: x, s: 0, d: out },
    ]
}

/// The softmax of the scores `v[x]` into `v[out]`: the maximum, each `exp(s − max)`, their sum, one reciprocal, the
/// products. Scalars 0..1 are its scratch.
pub fn softmax(x: VReg, out: VReg) -> Vec<Op> {
    vec![
        Op::Max { a: x, d: 0 },
        Op::VecScalar { op: Bin::Sub, a: x, s: 0, d: out },
        Op::Map { f: Func::Exp, a: out, d: out },
        Op::Sum { a: out, d: 1 },
        Op::ScalarMap { f: Func::Recip, a: 1, d: 1 },
        Op::VecScalar { op: Bin::Mul, a: out, s: 1, d: out },
    ]
}

/// The FFN's gated product `silu(v[g])·v[u]` into `v[out]`.
pub fn swiglu(g: VReg, u: VReg, out: VReg) -> Vec<Op> {
    vec![Op::Map { f: Func::Silu, a: g, d: out }, Op::Vec { op: Bin::Mul, a: out, b: u, d: out }]
}

/// Attention's output gate `v[o]·σ(v[g])` into `v[out]`, `v[t]` its scratch.
pub fn sigmoid_gate(o: VReg, g: VReg, out: VReg, t: VReg) -> Vec<Op> {
    vec![Op::Map { f: Func::Sigmoid, a: g, d: t }, Op::Vec { op: Bin::Mul, a: o, b: t, d: out }]
}

/// A Gated-DeltaNet head's decay and gate from the scalars `s[alpha]`, `s[dt]`, `s[a]`, `s[beta]`: the decay
/// exponent `softplus(α + dt)·a` into `s[log_decay]`, the decay `exp` of it into `s[decay]`, the gate `σ(β)` into
/// `s[gate]`.
pub fn gdn_gates(alpha: SReg, dt: SReg, a: SReg, beta: SReg, log_decay: SReg, decay: SReg, gate: SReg) -> Vec<Op> {
    vec![
        Op::Scalar { op: Bin::Add, a: alpha, b: dt, d: log_decay },
        Op::ScalarMap { f: Func::Softplus, a: log_decay, d: log_decay },
        Op::Scalar { op: Bin::Mul, a: log_decay, b: a, d: log_decay },
        Op::ScalarMap { f: Func::Exp, a: log_decay, d: decay },
        Op::ScalarMap { f: Func::Sigmoid, a: beta, d: gate },
    ]
}

/// Every value head's decay and gate at once from the vectors `v[alpha]`, `v[dt]`, `v[a]`, `v[beta]`: the decay
/// exponents `softplus(α + dt)·a` into `v[log_decay]`, the decays into `v[decay]`, the gates `σ(β)` into `v[gate]`.
pub fn delta_gates(alpha: VReg, dt: VReg, a: VReg, beta: VReg, log_decay: VReg, decay: VReg, gate: VReg) -> Vec<Op> {
    vec![
        Op::Vec { op: Bin::Add, a: alpha, b: dt, d: log_decay },
        Op::Map { f: Func::Softplus, a: log_decay, d: log_decay },
        Op::Vec { op: Bin::Mul, a: log_decay, b: a, d: log_decay },
        Op::Map { f: Func::Exp, a: log_decay, d: decay },
        Op::Map { f: Func::Sigmoid, a: beta, d: gate },
    ]
}

/// A head turned by its position's rotary table into `v[out]` ([`super::numerics::rope`]).
pub fn rotary(x: VReg, cos: VReg, sin: VReg, out: VReg, neox: bool) -> Vec<Op> {
    vec![Op::Rotate { a: x, cos, sin, d: out, neox }]
}

/// A cache row as int8 ([`super::numerics::cache_row`]): the stored scale `amax/127` rounded to f16 into
/// `s[scale]`, the codes under its reciprocal into `v[codes]`. Scalar `s[scale] + 1` is its scratch.
pub fn cache_row(x: VReg, codes: VReg, scale: SReg) -> Vec<Op> {
    let t = scale + 1;
    vec![
        Op::AbsMax { a: x, d: scale },
        Op::Const { value: INV_127, d: t },
        Op::Scalar { op: Bin::Mul, a: scale, b: t, d: scale },
        Op::ScalarMap { f: Func::F16, a: scale, d: scale },
        Op::ScalarMap { f: Func::Recip, a: scale, d: t },
        Op::Codes { a: x, s: t, d: codes },
    ]
}

/// The causal convolution of a Gated-DeltaNet block's channels through SiLU into `v[out]`
/// ([`super::numerics::conv`]): the ring's rows `v[ring[j]]`, oldest first, against the kernel's taps `v[taps[j]]`,
/// then the token's row `v[x]` against the last tap, each channel's products summed in tap order from `+0`, a
/// multiply-add a tap. The kernel is held tap by tap (`taps[j]_ch` the channel's tap `j`); `v[acc]` is its scratch,
/// scalar 0 the zero.
pub fn conv(ring: &[VReg], x: VReg, taps: &[VReg], acc: VReg, out: VReg) -> Vec<Op> {
    assert_eq!(taps.len(), ring.len() + 1, "a tap for every ring row and the token's own");
    let mut p = vec![Op::Const { value: 0.0, d: 0 }];
    let rows = ring.iter().copied().chain(std::iter::once(x));
    for (j, (row, &tap)) in rows.zip(taps).enumerate() {
        p.push(if j == 0 { Op::MulAddScalar { a: row, b: tap, s: 0, d: acc } } else { Op::MulAdd { a: row, b: tap, c: acc, d: acc } });
    }
    p.push(Op::Map { f: Func::Silu, a: acc, d: out });
    p
}

/// A Gated-DeltaNet head's read normalized by the weights `v[w]` and gated by `silu(v[z])` into `v[out]`, `n` the
/// head's width. `v[t]` and scalars 0..2 are its scratch.
pub fn gdn_out(fp: FpUnit, o: VReg, w: VReg, z: VReg, out: VReg, t: VReg, n: usize, eps: f32) -> Vec<Op> {
    let mut p = rmsnorm(fp, o, w, out, t, n, eps);
    p.push(Op::Map { f: Func::Silu, a: z, d: t });
    p.push(Op::Vec { op: Bin::Mul, a: out, b: t, d: out });
    p
}

/// The greedy token of the logits `v[x]` into `s[d]`.
pub fn greedy(x: VReg, d: SReg) -> Vec<Op> {
    vec![Op::ArgMax { a: x, d }]
}
