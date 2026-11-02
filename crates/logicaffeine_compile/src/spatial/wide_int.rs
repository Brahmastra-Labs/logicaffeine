//! 64-bit integers on a chip whose integer lanes are 32 bits wide.
//!
//! An RDU computes integers in INT32 ([`super::rdu_chip::RduChipFacts::index_datatype`]), and some of what a model
//! asks for needs 64 bits: Qwen3.8-Flash-Next's n-gram hash multiplies a token by a multiplier below 2^45 and reduces
//! the exclusive-or of such products by a prime ([`crate::model::ngram_hash`]), and an index past 2^31 needs a wider
//! type. [`lower`] turns each 64-bit operation ([`Op64`]) into a [`LaneProgram`]: straight-line 32-bit lane operations
//! (wrapping add and subtract, the low half of a product, bitwise operations, shifts by a constant), a 64-bit operand
//! travelling as its low and its high lane.
//!
//! The arithmetic lowerings keep every lane operation exact. An operand is cut into 16-bit limbs, so a limb product
//! fits a lane; sums are carried limb by limb; and a value is only ever cut by a shift and a mask. A subtraction adds
//! the complement plus one. The remainder by `d` is Barrett's: the quotient estimate `⌊x · m / 2^64⌋` with
//! `m = ⌊2^64 / d⌋`, the difference `x − q·d`, which lies in `[0, 2d)`, and one conditional subtraction of `d`.
//!
//! [`prove`] proves each lowering equal to its 64-bit definition. The arithmetic ones by their algebra: the program is
//! evaluated symbolically over the integers, with an interval for every lane that shows no operation wraps; every cut
//! `v = h · 2^k + l` keeps `h` as a symbol and writes `l` as `v − h · 2^k`; and what the outputs then are is a
//! polynomial identity — the result equals the definition plus `2^64` times an integer polynomial — that the kernel
//! checks by ring normalization (the first kernel's `try_ring` reduced to `DRingSolve` and its conclusion, by
//! `infer_type`). The second kernel re-checks each identity by exact evaluation at a Kronecker point where that fits its
//! bound on slots (one per symbol at degree one: the add's and subtract's identities, not the multiply's); it never
//! disagrees, and a proof records which identities it decided ([`KernelVerdict`]). The remainder's proof is a chain of
//! such identities, the intervals that bound Barrett's difference to `[0, 2d)`, and the two cases of its correction. The bitwise lowerings and the shifts are wiring, proved by a certified equivalence
//! check of their bit-blasted circuits ([`logicaffeine_proof::fraig`], its LRAT derivation replayed).

use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::target::SpatialTarget;
use logicaffeine_base::numeric::BigInt;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::tactic_cert::{syn_binary, syn_typed_eq, term_of_syn, CertifiablePrimitive, Syn};
use logicaffeine_kernel::{double_check, infer_type, Context, DoubleCheck, Term};
use logicaffeine_proof::aig::{Aig, AigLit};
use logicaffeine_proof::fraig;
use std::collections::BTreeMap;

/// Whether a 64-bit integer needs limbs on `target`: its datapath stores no 64-bit integer.
pub fn needs_limbs(target: &SpatialTarget) -> bool {
    !target.storable_ints().contains(&Precision::Int64)
}

/// A 64-bit operation on unsigned integers; a shift carries its distance and a remainder its divisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op64 {
    /// `a + b` modulo 2^64.
    Add,
    /// `a − b` modulo 2^64.
    Sub,
    /// The low 64 bits of `a · b`.
    Mul,
    And,
    Or,
    Xor,
    /// `a << k`, `k < 64`.
    Shl(u32),
    /// `a >> k`, `k < 64`.
    Shr(u32),
    /// `a mod d`, `d ≥ 1`.
    Rem(u32),
}

impl Op64 {
    /// The 64-bit operands it takes.
    pub fn arity(self) -> usize {
        match self {
            Op64::Shl(_) | Op64::Shr(_) | Op64::Rem(_) => 1,
            _ => 2,
        }
    }

    /// Its definition; a unary operation ignores `b`.
    pub fn apply(self, a: u64, b: u64) -> u64 {
        match self {
            Op64::Add => a.wrapping_add(b),
            Op64::Sub => a.wrapping_sub(b),
            Op64::Mul => a.wrapping_mul(b),
            Op64::And => a & b,
            Op64::Or => a | b,
            Op64::Xor => a ^ b,
            Op64::Shl(k) => a << k,
            Op64::Shr(k) => a >> k,
            Op64::Rem(d) => a % u64::from(d),
        }
    }

    fn check(self) {
        match self {
            Op64::Shl(k) | Op64::Shr(k) => assert!(k < 64, "a 64-bit shift by {k}"),
            Op64::Rem(d) => assert!(d >= 1, "a remainder by zero"),
            _ => {}
        }
    }
}

/// A register: the index of the operation that defines it.
pub type Reg = usize;

/// One operation of a 32-bit lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneOp {
    /// The `i`-th input lane: a 64-bit operand's low lane, then its high lane, operand by operand.
    Input(usize),
    Const(u32),
    /// Wrapping.
    Add(Reg, Reg),
    /// Wrapping.
    Sub(Reg, Reg),
    /// The low 32 bits of the product.
    MulLo(Reg, Reg),
    And(Reg, Reg),
    Or(Reg, Reg),
    Xor(Reg, Reg),
    /// By a constant below 32.
    Shl(Reg, u32),
    /// Logical, by a constant below 32.
    Shr(Reg, u32),
}

/// Where a remainder's program holds the values its proof speaks of.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RemMarks {
    /// The eight 16-bit limbs of the exact product `x · m`, least significant first.
    product: Vec<Reg>,
    /// The four limbs of `x − q·d` modulo 2^64.
    diff: Vec<Reg>,
    /// The four limbs of `diff + 2^34 − d` modulo 2^64.
    probe: Vec<Reg>,
    /// `probe >> 34`: one exactly when the difference is at least `d`.
    over: Reg,
}

/// A straight-line program over 32-bit lanes computing one 64-bit operation: its inputs are each operand's low and
/// high lane, its two outputs the result's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneProgram {
    op: Op64,
    ops: Vec<LaneOp>,
    outputs: [Reg; 2],
    rem: Option<RemMarks>,
}

impl LaneProgram {
    pub fn op(&self) -> Op64 {
        self.op
    }

    pub fn ops(&self) -> &[LaneOp] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The input lanes: two for each operand.
    pub fn inputs(&self) -> usize {
        2 * self.op.arity()
    }

    /// The output lanes: the result's low and high.
    pub fn outputs(&self) -> usize {
        self.outputs.len()
    }

    /// Every register's value on `lanes`, as a 32-bit lane computes it.
    pub fn eval(&self, lanes: &[u32]) -> Vec<u32> {
        assert_eq!(lanes.len(), self.inputs(), "{:?} takes {} lanes", self.op, self.inputs());
        let mut v: Vec<u32> = Vec::with_capacity(self.ops.len());
        for op in &self.ops {
            let x = match *op {
                LaneOp::Input(i) => lanes[i],
                LaneOp::Const(c) => c,
                LaneOp::Add(a, b) => v[a].wrapping_add(v[b]),
                LaneOp::Sub(a, b) => v[a].wrapping_sub(v[b]),
                LaneOp::MulLo(a, b) => v[a].wrapping_mul(v[b]),
                LaneOp::And(a, b) => v[a] & v[b],
                LaneOp::Or(a, b) => v[a] | v[b],
                LaneOp::Xor(a, b) => v[a] ^ v[b],
                LaneOp::Shl(a, k) => v[a] << k,
                LaneOp::Shr(a, k) => v[a] >> k,
            };
            v.push(x);
        }
        v
    }

    /// The program with operation `i` replaced: a variant to show that a proof is refused to a program that is not
    /// the lowering.
    pub fn with_op(&self, i: usize, op: LaneOp) -> LaneProgram {
        let mut p = self.clone();
        p.ops[i] = op;
        p
    }

    /// The 64-bit result on 64-bit operands (`b` unused by a unary operation).
    pub fn run(&self, a: u64, b: u64) -> u64 {
        let mut lanes = vec![a as u32, (a >> 32) as u32];
        if self.op.arity() == 2 {
            lanes.extend([b as u32, (b >> 32) as u32]);
        }
        let v = self.eval(&lanes);
        u64::from(v[self.outputs[0]]) | u64::from(v[self.outputs[1]]) << 32
    }
}

/// A 16-bit limb operand: a register below 2^16, or a constant one.
#[derive(Clone, Copy, Debug)]
enum Limb {
    Reg(Reg),
    K(u32),
}

struct Builder {
    ops: Vec<LaneOp>,
    consts: BTreeMap<u32, Reg>,
}

const MASK16: u32 = 0xFFFF;

impl Builder {
    fn new() -> Builder {
        Builder { ops: Vec::new(), consts: BTreeMap::new() }
    }

    fn push(&mut self, op: LaneOp) -> Reg {
        self.ops.push(op);
        self.ops.len() - 1
    }

    fn konst(&mut self, c: u32) -> Reg {
        if let Some(&r) = self.consts.get(&c) {
            return r;
        }
        let r = self.push(LaneOp::Const(c));
        self.consts.insert(c, r);
        r
    }

    fn lo16(&mut self, x: Reg) -> Reg {
        let m = self.konst(MASK16);
        self.push(LaneOp::And(x, m))
    }

    fn hi16(&mut self, x: Reg) -> Reg {
        self.push(LaneOp::Shr(x, 16))
    }

    /// A 64-bit operand's four limbs from its two lanes.
    fn limbs(&mut self, lo: Reg, hi: Reg) -> Vec<Limb> {
        vec![Limb::Reg(self.lo16(lo)), Limb::Reg(self.hi16(lo)), Limb::Reg(self.lo16(hi)), Limb::Reg(self.hi16(hi))]
    }

    /// A constant's 16-bit limbs, `count` of them.
    fn const_limbs(value: u128, count: usize) -> Vec<Limb> {
        (0..count).map(|i| Limb::K((value >> (16 * i)) as u32 & MASK16)).collect()
    }

    /// The register holding `limb`; a constant limb becomes a constant lane.
    fn reg(&mut self, limb: Limb) -> Reg {
        match limb {
            Limb::Reg(r) => r,
            Limb::K(k) => self.konst(k),
        }
    }

    /// Sums `terms` into one register; `None` for no terms.
    fn sum(&mut self, terms: &[Reg]) -> Option<Reg> {
        let mut acc: Option<Reg> = None;
        for &t in terms {
            acc = Some(match acc {
                None => t,
                Some(a) => self.push(LaneOp::Add(a, t)),
            });
        }
        acc
    }

    /// Carries a column sum per limb: limb `k` is `(column_k + carry) mod 2^16` and the carry its quotient. The carry
    /// out of the last column is computed too, so the algebra can name it, and is not returned.
    fn carry(&mut self, columns: Vec<Vec<Reg>>) -> Vec<Reg> {
        let mut out = Vec::with_capacity(columns.len());
        let mut carry: Option<Reg> = None;
        for mut terms in columns {
            terms.extend(carry);
            let s = match self.sum(&terms) {
                Some(s) => s,
                None => self.konst(0),
            };
            out.push(self.lo16(s));
            carry = Some(self.hi16(s));
        }
        out
    }

    /// `a + b` over `n` limbs, modulo 2^(16n).
    fn add_limbs(&mut self, a: &[Limb], b: &[Limb], n: usize) -> Vec<Reg> {
        let columns = (0..n)
            .map(|k| {
                let mut c = Vec::new();
                for x in [a.get(k), b.get(k)].into_iter().flatten() {
                    if !matches!(x, Limb::K(0)) {
                        c.push(self.reg(*x));
                    }
                }
                c
            })
            .collect();
        self.carry(columns)
    }

    /// `a − b` over `n` limbs, modulo 2^(16n): `a + (2^16 − 1 − b_k)` limb by limb, plus one.
    fn sub_limbs(&mut self, a: &[Limb], b: &[Limb], n: usize) -> Vec<Reg> {
        let mut columns: Vec<Vec<Reg>> = Vec::with_capacity(n);
        for k in 0..n {
            let mut c = Vec::new();
            if let Some(x) = a.get(k) {
                if !matches!(x, Limb::K(0)) {
                    c.push(self.reg(*x));
                }
            }
            let complement = match b.get(k) {
                Some(Limb::K(v)) => self.konst(MASK16 - v),
                Some(Limb::Reg(r)) => {
                    let m = self.konst(MASK16);
                    self.push(LaneOp::Sub(m, *r))
                }
                None => self.konst(MASK16),
            };
            c.push(complement);
            if k == 0 {
                c.push(self.konst(1));
            }
            columns.push(c);
        }
        self.carry(columns)
    }

    /// `a · b` over `n` limbs, modulo 2^(16n): every limb product cut in two, its halves added into their columns.
    fn mul_limbs(&mut self, a: &[Limb], b: &[Limb], n: usize) -> Vec<Reg> {
        let mut columns: Vec<Vec<Reg>> = vec![Vec::new(); n];
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.iter().enumerate() {
                if i + j >= n || matches!(x, Limb::K(0)) || matches!(y, Limb::K(0)) {
                    continue;
                }
                let (xr, yr) = (self.reg(*x), self.reg(*y));
                let p = self.push(LaneOp::MulLo(xr, yr));
                let (lo, hi) = (self.lo16(p), self.hi16(p));
                columns[i + j].push(lo);
                if i + j + 1 < n {
                    columns[i + j + 1].push(hi);
                }
            }
        }
        self.carry(columns)
    }

    /// Two lanes from four limbs.
    fn join(&mut self, l: &[Reg]) -> [Reg; 2] {
        let lo_hi = self.push(LaneOp::Shl(l[1], 16));
        let lo = self.push(LaneOp::Add(l[0], lo_hi));
        let hi_hi = self.push(LaneOp::Shl(l[3], 16));
        let hi = self.push(LaneOp::Add(l[2], hi_hi));
        [lo, hi]
    }
}

/// The program that computes `op` on 32-bit lanes.
pub fn lower(op: Op64) -> LaneProgram {
    op.check();
    let mut b = Builder::new();
    let a_lo = b.push(LaneOp::Input(0));
    let a_hi = b.push(LaneOp::Input(1));
    let (b_lo, b_hi) = if op.arity() == 2 { (b.push(LaneOp::Input(2)), b.push(LaneOp::Input(3))) } else { (0, 0) };
    let mut rem = None;
    let outputs = match op {
        Op64::Add | Op64::Sub | Op64::Mul => {
            let (x, y) = (b.limbs(a_lo, a_hi), b.limbs(b_lo, b_hi));
            let limbs = match op {
                Op64::Add => b.add_limbs(&x, &y, 4),
                Op64::Sub => b.sub_limbs(&x, &y, 4),
                _ => b.mul_limbs(&x, &y, 4),
            };
            b.join(&limbs)
        }
        Op64::And => [b.push(LaneOp::And(a_lo, b_lo)), b.push(LaneOp::And(a_hi, b_hi))],
        Op64::Or => [b.push(LaneOp::Or(a_lo, b_lo)), b.push(LaneOp::Or(a_hi, b_hi))],
        Op64::Xor => [b.push(LaneOp::Xor(a_lo, b_lo)), b.push(LaneOp::Xor(a_hi, b_hi))],
        Op64::Shl(k) => match k {
            0 => [a_lo, a_hi],
            1..=31 => {
                let up = b.push(LaneOp::Shl(a_hi, k));
                let across = b.push(LaneOp::Shr(a_lo, 32 - k));
                [b.push(LaneOp::Shl(a_lo, k)), b.push(LaneOp::Or(up, across))]
            }
            32 => [b.konst(0), a_lo],
            _ => [b.konst(0), b.push(LaneOp::Shl(a_lo, k - 32))],
        },
        Op64::Shr(k) => match k {
            0 => [a_lo, a_hi],
            1..=31 => {
                let down = b.push(LaneOp::Shr(a_lo, k));
                let across = b.push(LaneOp::Shl(a_hi, 32 - k));
                [b.push(LaneOp::Or(down, across)), b.push(LaneOp::Shr(a_hi, k))]
            }
            32 => [a_hi, b.konst(0)],
            _ => [b.push(LaneOp::Shr(a_hi, k - 32)), b.konst(0)],
        },
        Op64::Rem(1) => [b.konst(0), b.konst(0)],
        Op64::Rem(d) => {
            let m = (1u128 << 64) / u128::from(d);
            let x = b.limbs(a_lo, a_hi);
            let product = b.mul_limbs(&x, &Builder::const_limbs(m, 4), 8);
            let q: Vec<Limb> = product[4..].iter().map(|&r| Limb::Reg(r)).collect();
            let qd = b.mul_limbs(&q, &Builder::const_limbs(u128::from(d), 2), 4);
            let qd: Vec<Limb> = qd.into_iter().map(Limb::Reg).collect();
            let diff = b.sub_limbs(&x, &qd, 4);
            let diff_limbs: Vec<Limb> = diff.iter().map(|&r| Limb::Reg(r)).collect();
            let probe = b.add_limbs(&diff_limbs, &Builder::const_limbs((1u128 << 34) - u128::from(d), 4), 4);
            let over = b.push(LaneOp::Shr(probe[2], 2));
            let dk = b.konst(d);
            let correction = b.push(LaneOp::MulLo(over, dk));
            let correction = [Limb::Reg(b.lo16(correction)), Limb::Reg(b.hi16(correction))];
            let result = b.sub_limbs(&diff_limbs, &correction, 4);
            rem = Some(RemMarks { product, diff, probe, over });
            b.join(&result)
        }
    };
    LaneProgram { op, ops: b.ops, outputs, rem }
}

// =================================================================================================================
// The algebra
// =================================================================================================================

/// A symbol: an input lane, or the quotient or the remainder of the `j`-th cut of a value by a power of two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Sym {
    In(usize),
    Hi(usize),
    Lo(usize),
}

/// An integer polynomial in the symbols: each monomial (symbols with their powers, sorted) with its coefficient.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Poly(BTreeMap<Vec<(Sym, u32)>, BigInt>);

fn big(x: u128) -> BigInt {
    BigInt::from_u64((x >> 64) as u64).shl_bits(64).add(&BigInt::from_u64(x as u64))
}

fn pow2(k: u64) -> BigInt {
    BigInt::from_i64(1).shl_bits(k)
}

impl Poly {
    fn constant(c: BigInt) -> Poly {
        let mut m = BTreeMap::new();
        if !c.is_zero() {
            m.insert(Vec::new(), c);
        }
        Poly(m)
    }

    fn var(s: Sym) -> Poly {
        let mut m = BTreeMap::new();
        m.insert(vec![(s, 1)], BigInt::from_i64(1));
        Poly(m)
    }

    fn add(&self, o: &Poly) -> Poly {
        let mut m = self.0.clone();
        for (mono, c) in &o.0 {
            let e = m.entry(mono.clone()).or_insert_with(BigInt::zero);
            *e = e.add(c);
            if e.is_zero() {
                m.remove(mono);
            }
        }
        Poly(m)
    }

    fn scale(&self, k: &BigInt) -> Poly {
        if k.is_zero() {
            return Poly::default();
        }
        Poly(self.0.iter().map(|(mono, c)| (mono.clone(), c.mul(k))).collect())
    }

    fn sub(&self, o: &Poly) -> Poly {
        self.add(&o.scale(&BigInt::from_i64(-1)))
    }

    fn mul(&self, o: &Poly) -> Poly {
        let mut out = Poly::default();
        for (m1, c1) in &self.0 {
            for (m2, c2) in &o.0 {
                let mut powers: BTreeMap<Sym, u32> = m1.iter().copied().collect();
                for &(s, e) in m2 {
                    *powers.entry(s).or_insert(0) += e;
                }
                let mono: Vec<(Sym, u32)> = powers.into_iter().collect();
                out = out.add(&Poly(BTreeMap::from([(mono, c1.mul(c2))])));
            }
        }
        out
    }

    /// The polynomial divided by `k`, when every coefficient is a multiple of it.
    fn div_exact(&self, k: &BigInt) -> Option<Poly> {
        let mut m = BTreeMap::new();
        for (mono, c) in &self.0 {
            let (q, r) = c.div_rem(k)?;
            if !r.is_zero() {
                return None;
            }
            m.insert(mono.clone(), q);
        }
        Some(Poly(m))
    }

    /// An interval holding every value the polynomial takes with each symbol in its interval.
    fn interval(&self, iv: &dyn Fn(Sym) -> Interval) -> Interval {
        let mut out = Interval::point(BigInt::zero());
        for (mono, c) in &self.0 {
            let mut term = Interval::point(c.clone());
            for &(s, e) in mono {
                for _ in 0..e {
                    term = term.mul(&iv(s));
                }
            }
            out = out.add(&term);
        }
        out
    }

    /// The polynomial as reflected syntax: a sum of terms, each a coefficient times its symbols.
    fn syn(&self) -> Syn {
        let mut terms = self.0.iter().map(|(mono, c)| {
            let mut t = syn_int(c);
            for &(s, e) in mono {
                for _ in 0..e {
                    t = syn_binary("mul", t, syn_sym(s));
                }
            }
            t
        });
        match terms.next() {
            None => Syn::Lit(0),
            Some(first) => terms.fold(first, |acc, t| syn_binary("add", acc, t)),
        }
    }
}

fn syn_sym(s: Sym) -> Syn {
    Syn::Name(match s {
        Sym::In(i) => format!("in{i}"),
        Sym::Hi(j) => format!("hi{j}"),
        Sym::Lo(j) => format!("lo{j}"),
    })
}

/// An integer of any size as reflected syntax: literals below 2^32 combined in base 2^32.
fn syn_int(c: &BigInt) -> Syn {
    if let Some(v) = c.to_i64() {
        return Syn::Lit(v);
    }
    if c.is_negative() {
        return syn_binary("sub", Syn::Lit(0), syn_int(&c.negated()));
    }
    let base = BigInt::from_u64(1 << 32);
    let (q, r) = c.div_rem(&base).expect("a nonzero base");
    syn_binary("add", syn_binary("mul", syn_int(&q), Syn::Lit(1 << 32)), syn_int(&r))
}

/// A closed interval of integers.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Interval {
    lo: BigInt,
    hi: BigInt,
}

impl Interval {
    fn new(lo: BigInt, hi: BigInt) -> Interval {
        Interval { lo, hi }
    }

    fn point(v: BigInt) -> Interval {
        Interval { lo: v.clone(), hi: v }
    }

    fn add(&self, o: &Interval) -> Interval {
        Interval::new(self.lo.add(&o.lo), self.hi.add(&o.hi))
    }

    fn mul(&self, o: &Interval) -> Interval {
        let c = [self.lo.mul(&o.lo), self.lo.mul(&o.hi), self.hi.mul(&o.lo), self.hi.mul(&o.hi)];
        Interval::new(c.iter().min().unwrap().clone(), c.iter().max().unwrap().clone())
    }

    fn within(&self, lo: &BigInt, hi: &BigInt) -> bool {
        &self.lo >= lo && &self.hi <= hi
    }
}

/// A cut `value = hi · 2^k + lo`, `0 ≤ lo < 2^k`.
#[derive(Clone, Debug)]
struct Cut {
    of: Reg,
    k: u32,
    hi: Interval,
    lo: Interval,
}

/// A register's value: as a polynomial in the inputs and the cuts' quotients and remainders (`atom`, the form its
/// interval is read from), the same with every remainder written as `value − hi · 2^k` (`elim`, in the inputs and the
/// quotients only), and that form as the expression tree the kernel normalizes.
#[derive(Clone, Debug)]
struct Value {
    atom: Poly,
    elim: Poly,
    tree: Syn,
    iv: Interval,
}

/// The program evaluated symbolically, every operation shown not to wrap.
struct Algebra {
    values: Vec<Value>,
    cuts: Vec<Cut>,
}

fn lane_max() -> BigInt {
    BigInt::from_u64(u64::from(u32::MAX))
}

impl Algebra {
    fn symbol_interval(&self, s: Sym) -> Interval {
        match s {
            Sym::In(_) => Interval::new(BigInt::zero(), lane_max()),
            Sym::Hi(j) => self.cuts[j].hi.clone(),
            Sym::Lo(j) => self.cuts[j].lo.clone(),
        }
    }

    /// The interval of a polynomial in atom form.
    fn bound(&self, p: &Poly) -> Interval {
        p.interval(&|s| self.symbol_interval(s))
    }

    /// Evaluates `program`. A shift right named in `narrowed` has its quotient in the interval given there, which the
    /// caller has established from what precedes it; every other interval is the operation's own.
    fn run(program: &LaneProgram, narrowed: &BTreeMap<Reg, Interval>) -> Result<Algebra, String> {
        let mut alg = Algebra { values: Vec::new(), cuts: Vec::new() };
        let mut memo: BTreeMap<(Reg, u32), usize> = BTreeMap::new();
        let ops = &program.ops;
        for (r, op) in ops.iter().enumerate() {
            let exact = |iv: &Interval, what: &str| -> Result<(), String> {
                if iv.within(&BigInt::zero(), &lane_max()) {
                    Ok(())
                } else {
                    Err(format!("register {r} ({what}) can leave a 32-bit lane: [{}, {}]", iv.lo, iv.hi))
                }
            };
            let value = match *op {
                LaneOp::Input(i) => Value {
                    atom: Poly::var(Sym::In(i)),
                    elim: Poly::var(Sym::In(i)),
                    tree: syn_sym(Sym::In(i)),
                    iv: Interval::new(BigInt::zero(), lane_max()),
                },
                LaneOp::Const(c) => {
                    let c = BigInt::from_u64(u64::from(c));
                    Value { atom: Poly::constant(c.clone()), elim: Poly::constant(c.clone()), tree: syn_int(&c), iv: Interval::point(c) }
                }
                LaneOp::Add(a, b) => {
                    let (x, y) = (&alg.values[a], &alg.values[b]);
                    let iv = x.iv.add(&y.iv);
                    exact(&iv, "an add")?;
                    Value {
                        atom: x.atom.add(&y.atom),
                        elim: x.elim.add(&y.elim),
                        tree: syn_binary("add", x.tree.clone(), y.tree.clone()),
                        iv,
                    }
                }
                LaneOp::Sub(a, b) => {
                    let (x, y) = (&alg.values[a], &alg.values[b]);
                    let iv = Interval::new(x.iv.lo.sub(&y.iv.hi), x.iv.hi.sub(&y.iv.lo));
                    exact(&iv, "a subtract")?;
                    Value {
                        atom: x.atom.sub(&y.atom),
                        elim: x.elim.sub(&y.elim),
                        tree: syn_binary("sub", x.tree.clone(), y.tree.clone()),
                        iv,
                    }
                }
                LaneOp::MulLo(a, b) => {
                    let (x, y) = (&alg.values[a], &alg.values[b]);
                    let iv = x.iv.mul(&y.iv);
                    exact(&iv, "a multiply")?;
                    Value {
                        atom: x.atom.mul(&y.atom),
                        elim: x.elim.mul(&y.elim),
                        tree: syn_binary("mul", x.tree.clone(), y.tree.clone()),
                        iv,
                    }
                }
                LaneOp::Shl(a, k) => {
                    let x = &alg.values[a];
                    let f = pow2(u64::from(k));
                    let iv = x.iv.mul(&Interval::point(f.clone()));
                    exact(&iv, "a shift left")?;
                    Value { atom: x.atom.scale(&f), elim: x.elim.scale(&f), tree: syn_binary("mul", x.tree.clone(), syn_int(&f)), iv }
                }
                LaneOp::Shr(a, k) => {
                    let (hi, _) = alg.cut(&mut memo, a, k);
                    match narrowed.get(&r) {
                        None => hi,
                        Some(iv) => {
                            let j = memo[&(a, k)];
                            if !iv.within(&alg.cuts[j].hi.lo, &alg.cuts[j].hi.hi) {
                                return Err(format!("register {r}: a narrowing outside the quotient's own interval"));
                            }
                            alg.cuts[j].hi = iv.clone();
                            Value { iv: iv.clone(), ..hi }
                        }
                    }
                }
                LaneOp::And(a, m) => {
                    let mask = match ops[m] {
                        LaneOp::Const(c) if c != 0 && (c & c.wrapping_add(1)) == 0 => c,
                        _ => return Err(format!("register {r}: an and with no low mask is not arithmetic")),
                    };
                    alg.cut(&mut memo, a, 32 - mask.leading_zeros()).1
                }
                LaneOp::Or(..) | LaneOp::Xor(..) => return Err(format!("register {r}: a bitwise operation is not arithmetic")),
            };
            alg.values.push(value);
        }
        Ok(alg)
    }

    /// The cut of register `a` by `2^k`: its quotient's value and its remainder's.
    fn cut(&mut self, memo: &mut BTreeMap<(Reg, u32), usize>, a: Reg, k: u32) -> (Value, Value) {
        let j = match memo.get(&(a, k)) {
            Some(&j) => j,
            None => {
                let iv = &self.values[a].iv;
                let f = pow2(u64::from(k));
                let (hi_top, _) = iv.hi.div_rem(&f).expect("a power of two");
                let (hi_bottom, _) = iv.lo.div_rem(&f).expect("a power of two");
                let lo_top = if iv.hi < f { iv.hi.clone() } else { f.sub(&BigInt::from_i64(1)) };
                self.cuts.push(Cut {
                    of: a,
                    k,
                    hi: Interval::new(hi_bottom, hi_top),
                    lo: Interval::new(BigInt::zero(), lo_top),
                });
                memo.insert((a, k), self.cuts.len() - 1);
                self.cuts.len() - 1
            }
        };
        let f = pow2(u64::from(k));
        let hi = Value {
            atom: Poly::var(Sym::Hi(j)),
            elim: Poly::var(Sym::Hi(j)),
            tree: syn_sym(Sym::Hi(j)),
            iv: self.symbol_interval(Sym::Hi(j)),
        };
        let of = &self.values[a];
        let lo = Value {
            atom: Poly::var(Sym::Lo(j)),
            elim: of.elim.sub(&Poly::var(Sym::Hi(j)).scale(&f)),
            tree: syn_binary("sub", of.tree.clone(), syn_binary("mul", syn_sym(Sym::Hi(j)), syn_int(&f))),
            iv: self.cuts[j].lo.clone(),
        };
        (hi, lo)
    }

    /// `Σ limb_k · 2^(16k)` over registers, in each form.
    fn limb_sum(&self, limbs: &[Reg]) -> (Poly, Poly, Syn) {
        let mut atom = Poly::default();
        let mut elim = Poly::default();
        let mut tree: Option<Syn> = None;
        for (k, &r) in limbs.iter().enumerate() {
            let f = pow2(16 * k as u64);
            atom = atom.add(&self.values[r].atom.scale(&f));
            elim = elim.add(&self.values[r].elim.scale(&f));
            let t = syn_binary("mul", self.values[r].tree.clone(), syn_int(&f));
            tree = Some(match tree {
                None => t,
                Some(acc) => syn_binary("add", acc, t),
            });
        }
        (atom, elim, tree.unwrap_or(Syn::Lit(0)))
    }

    /// The two output lanes as one 64-bit value, in each form.
    fn result(&self, program: &LaneProgram) -> (Poly, Poly, Syn) {
        let [lo, hi] = program.outputs;
        let f = pow2(32);
        let atom = self.values[lo].atom.add(&self.values[hi].atom.scale(&f));
        let elim = self.values[lo].elim.add(&self.values[hi].elim.scale(&f));
        let tree = syn_binary("add", self.values[lo].tree.clone(), syn_binary("mul", self.values[hi].tree.clone(), syn_int(&f)));
        (atom, elim, tree)
    }
}

/// An operand as a polynomial in the input lanes: `in(2i) + in(2i+1) · 2^32`.
fn operand(i: usize) -> Poly {
    Poly::var(Sym::In(2 * i)).add(&Poly::var(Sym::In(2 * i + 1)).scale(&pow2(32)))
}

/// One identity the kernels check: `lhs = rhs` over the integers.
pub type Identity = (Syn, Syn);

/// `tree = spec + 2^64 · K` for the integer polynomial `K` that makes it hold, when there is one.
fn modular_identity(elim: &Poly, tree: Syn, spec: &Poly) -> Result<(Identity, Poly), String> {
    let modulus = pow2(64);
    let k = elim.sub(spec).div_exact(&modulus).ok_or("the result differs from its definition modulo 2^64")?;
    let rhs = spec.add(&k.scale(&modulus));
    Ok(((tree, rhs.syn()), k))
}

/// The identities that prove an add, subtract or multiply.
fn ring_identities(program: &LaneProgram) -> Result<Vec<Identity>, String> {
    let alg = Algebra::run(program, &BTreeMap::new())?;
    let (_, elim, tree) = alg.result(program);
    let (a, b) = (operand(0), operand(1));
    let spec = match program.op {
        Op64::Add => a.add(&b),
        Op64::Sub => a.sub(&b),
        Op64::Mul => a.mul(&b),
        op => return Err(format!("{op:?} is not a ring operation")),
    };
    Ok(vec![modular_identity(&elim, tree, &spec)?.0])
}

/// Whether two values congruent modulo 2^64 are equal: both lie in `[0, 2^64)`.
fn congruent_means_equal(a: &Interval, b: &Interval) -> bool {
    let top = pow2(64).sub(&BigInt::from_i64(1));
    a.within(&BigInt::zero(), &top) && b.within(&BigInt::zero(), &top)
}

/// The facts that prove a remainder by `d ≥ 2`: the identities the kernels check, and the intervals checked here that
/// turn congruences into equalities and fix the correction bit.
///
/// With `m = ⌊2^64/d⌋` and `e = 2^64 − d·m < d`:
/// 1. the product chain's limbs are `x·m − c₁·2^128`, `c₁` its carry out, and `c₁ = 0` because `x·m < 2^128` and
///    the limbs are not negative; so `x·m = q·2^64 + R` with `q` the high four limbs and `R ∈ [0, 2^64)` the low;
/// 2. Barrett's difference `T = x − q·d` satisfies `T·2^64 = x·e + d·R` (plus `d·c₁·2^128`, zero), so
///    `T ∈ [0, 2d)`;
/// 3. the program's difference is `T` modulo 2^64, and both are in `[0, 2^64)`: it is `T`;
/// 4. the probe is `T + 2^34 − d` modulo 2^64, both in `[0, 2^64)`: it is that, in `[2^34 − d, 2^34 + d)`, below
///    2^35; its limb 2 over 4 is the correction bit, then 0 or 1, and its top limb is zero;
/// 5. the result is the difference less `bit · d` modulo 2^64; with the bit 0 the probe is below 2^34 and `T < d`,
///    with the bit 1 it is at least 2^34 and `T ∈ [d, 2d)`; either way `T − bit·d ∈ [0, d)` and the result, in
///    `[0, 2^64)`, is it. So the result is `x − (q + bit)·d ∈ [0, d)`: `x mod d`.
fn remainder_identities(program: &LaneProgram, d: u32) -> Result<Vec<Identity>, String> {
    let marks = program.rem.as_ref().ok_or("a remainder's program without its marks")?;
    let one = BigInt::from_i64(1);
    let d_big = BigInt::from_u64(u64::from(d));
    let two_d = d_big.add(&d_big);
    let m = (1u128 << 64) / u128::from(d);
    let (m_big, e_big) = (big(m), big((1u128 << 64) - m * u128::from(d)));
    let (two64, two128) = (pow2(64), pow2(128));
    let x = operand(0);
    let mut identities = Vec::new();

    // The correction bit's interval, [0, 1], is established in step 4 from what precedes its use.
    let bit = Interval::new(BigInt::zero(), one.clone());
    let alg = Algebra::run(program, &BTreeMap::from([(marks.over, bit)]))?;

    // (1)
    let (p_atom, p_elim, p_tree) = alg.limb_sum(&marks.product);
    let top = *marks.product.last().ok_or("an empty product")?;
    let top_cut = match alg.values[top].atom.0.keys().next().map(|m| m.as_slice()) {
        Some([(Sym::Lo(j), 1)]) if alg.values[top].atom.0.len() == 1 => *j,
        _ => return Err("the product's top limb is not a cut's remainder".into()),
    };
    let carry_out = Poly::var(Sym::Hi(top_cut));
    let xm = x.scale(&m_big);
    let claim = xm.sub(&carry_out.scale(&two128));
    if p_elim != claim {
        return Err("the product chain is not x · m".into());
    }
    identities.push((p_tree, claim.syn()));
    if alg.bound(&xm).hi >= two128 || alg.bound(&p_atom).lo.is_negative() || alg.cuts[top_cut].hi.lo.is_negative() {
        return Err("the product's carry out is not shown zero".into());
    }

    // (2)
    let (q_atom, q_elim, q_tree) = alg.limb_sum(&marks.product[4..]);
    let (r_atom, r_elim, r_tree) = alg.limb_sum(&marks.product[..4]);
    let t_elim = x.sub(&q_elim.scale(&d_big));
    let barrett_lhs = t_elim.scale(&two64);
    let barrett_rhs = x.scale(&e_big).add(&r_elim.scale(&d_big)).add(&carry_out.scale(&d_big.mul(&two128)));
    if barrett_lhs != barrett_rhs {
        return Err("Barrett's identity does not hold".into());
    }
    identities.push((
        syn_binary("mul", syn_binary("sub", x.syn(), syn_binary("mul", q_tree, syn_int(&d_big))), syn_int(&two64)),
        syn_binary(
            "add",
            syn_binary("add", syn_binary("mul", x.syn(), syn_int(&e_big)), syn_binary("mul", syn_int(&d_big), r_tree)),
            syn_binary("mul", carry_out.syn(), syn_int(&d_big.mul(&two128))),
        ),
    ));
    let (x_bound, r_bound) = (alg.bound(&x), alg.bound(&r_atom));
    if x_bound.lo.is_negative() || r_bound.lo.is_negative() || r_bound.hi >= two64 || alg.bound(&q_atom).lo.is_negative() {
        return Err("Barrett's terms are not bounded".into());
    }
    // T · 2^64 = x·e + d·R, each term in [0, …]: T ∈ [0, ⌊(x_max·e + d·R_max) / 2^64⌋], which is below 2d as e < d.
    let (t_top, _) = x_bound.hi.mul(&e_big).add(&d_big.mul(&r_bound.hi)).div_rem(&two64).ok_or("a zero modulus")?;
    if t_top >= two_d {
        return Err("Barrett's difference is not shown below 2d".into());
    }
    let t = Interval::new(BigInt::zero(), t_top);

    // (3)
    let (diff_atom, diff_elim, diff_tree) = alg.limb_sum(&marks.diff);
    identities.push(modular_identity(&diff_elim, diff_tree, &t_elim)?.0);
    if !congruent_means_equal(&alg.bound(&diff_atom), &t) {
        return Err("the difference is not shown to be Barrett's".into());
    }

    // (4)
    let (probe_atom, probe_elim, probe_tree) = alg.limb_sum(&marks.probe);
    let shift = big((1u128 << 34) - u128::from(d));
    identities.push(modular_identity(&probe_elim, probe_tree, &diff_elim.add(&Poly::constant(shift.clone())))?.0);
    let probe = Interval::new(t.lo.add(&shift), t.hi.add(&shift));
    if !congruent_means_equal(&alg.bound(&probe_atom), &probe) || probe.hi >= pow2(35) {
        return Err("the probe is not shown exact and below 2^35".into());
    }
    let over = match alg.values[marks.over].atom.0.keys().next().map(|m| m.as_slice()) {
        Some([(Sym::Hi(j), 1)]) => *j,
        _ => return Err("the correction bit is not a cut's quotient".into()),
    };
    let limb2 = marks.probe[2];
    if alg.cuts[over].of != limb2 || alg.cuts[over].k != 2 {
        return Err("the correction bit is not the probe's limb 2 over 4".into());
    }
    // probe ≥ limb2 · 2^32 ≥ bit · 2^34, the other limbs not negative; with probe < 2^35 the bit is at most 1. And the
    // top limb is zero: top · 2^48 ≤ probe < 2^48.
    let (bit_top, _) = probe.hi.div_rem(&pow2(34)).ok_or("a zero modulus")?;
    if bit_top > one || probe.hi >= pow2(48) {
        return Err("the correction bit is not shown to be one bit".into());
    }

    // (5)
    let (out_atom, out_elim, out_tree) = alg.result(program);
    let over_sym = Poly::var(Sym::Hi(over));
    identities.push(modular_identity(&out_elim, out_tree, &diff_elim.sub(&over_sym.scale(&d_big)))?.0);
    let out_bound = alg.bound(&out_atom);
    for b in [0i64, 1] {
        // The probe with the bit b: limb 2 ∈ [4b, 4b + 3], limbs 0 and 1 below 2^16, the top limb zero.
        let bit_scale = BigInt::from_i64(b).mul(&pow2(34));
        let case = Interval::new(
            std::cmp::max(probe.lo.clone(), bit_scale.clone()),
            std::cmp::min(probe.hi.clone(), bit_scale.add(&pow2(34)).sub(&one)),
        );
        if case.lo > case.hi {
            continue;
        }
        // T = probe − 2^34 + d, less b·d.
        let less = d_big.mul(&BigInt::from_i64(b));
        let value = Interval::new(case.lo.sub(&shift).sub(&less), case.hi.sub(&shift).sub(&less));
        if value.lo.is_negative() || value.hi >= d_big || !congruent_means_equal(&out_bound, &value) {
            return Err(format!("with the correction bit {b}, the result is not shown in [0, d)"));
        }
    }
    Ok(identities)
}

/// What the kernels say of one identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelVerdict {
    /// The first kernel's `try_ring` reduces it to `DRingSolve` and its conclusion is the identity, and the second
    /// kernel re-checks the same.
    Both,
    /// The first kernel accepts it; the second's exact evaluation (a Kronecker substitution with a bound on its
    /// slots) declines it as past its bounds, which is no verdict and never a disagreement.
    FirstOnly,
    /// The first kernel rejects it, or the two disagree.
    Rejected,
}

/// The kernels' verdict on `lhs = rhs` over the integers: the witness that `concludes (try_ring goal)` is `goal` (the
/// first kernel reduces `try_ring` to `DRingSolve` by ring normalization, then `concludes` re-verifies it) checked by
/// `infer_type`, and re-checked by the second kernel through `double_check`.
fn kernels_on(ctx: &Context, (lhs, rhs): &Identity) -> KernelVerdict {
    let goal = term_of_syn(&syn_typed_eq("Int", lhs.clone(), rhs.clone()));
    let g = |n: &str| Term::Global(n.into());
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let conclusion = app(g("concludes"), app(g(CertifiablePrimitive::Ring.rule_name()), goal.clone()));
    let ty = app(app(app(g("Eq"), g("Syntax")), conclusion), goal.clone());
    let witness = app(
        Term::Lambda { param: "proof".into(), param_type: Box::new(ty), body: Box::new(Term::Var("proof".into())) },
        app(app(g("refl"), g("Syntax")), goal),
    );
    if infer_type(ctx, &witness).is_err() {
        return KernelVerdict::Rejected;
    }
    match double_check(ctx, &witness) {
        DoubleCheck::Agreed => KernelVerdict::Both,
        DoubleCheck::MainOnlyReCheckerIncomplete(_) => KernelVerdict::FirstOnly,
        DoubleCheck::Disagree(_) => KernelVerdict::Rejected,
    }
}

// =================================================================================================================
// The circuits
// =================================================================================================================

/// The program's outputs as a circuit over the operands' bits (`a0` … `a63`, `b0` … `b63`), outputs `o0` … `o63`.
fn program_circuit(program: &LaneProgram) -> Result<Aig, String> {
    let mut g = Aig::new();
    let mut operand_bits: Vec<Vec<AigLit>> = Vec::new();
    for o in 0..program.op.arity() {
        let name = if o == 0 { "a" } else { "b" };
        operand_bits.push((0..64).map(|i| g.add_input(Some(format!("{name}{i}")))).collect());
    }
    let mut lanes: Vec<Vec<AigLit>> = Vec::new();
    for (r, op) in program.ops.iter().enumerate() {
        let bits: Vec<AigLit> = match *op {
            LaneOp::Input(i) => operand_bits[i / 2][32 * (i % 2)..32 * (i % 2) + 32].to_vec(),
            LaneOp::Const(c) => (0..32).map(|i| if c >> i & 1 == 1 { AigLit::TRUE } else { AigLit::FALSE }).collect(),
            LaneOp::And(a, b) => (0..32).map(|i| g.and(lanes[a][i], lanes[b][i])).collect(),
            LaneOp::Or(a, b) => (0..32).map(|i| g.or(lanes[a][i], lanes[b][i])).collect(),
            LaneOp::Xor(a, b) => (0..32).map(|i| g.xor(lanes[a][i], lanes[b][i])).collect(),
            LaneOp::Shl(a, k) => (0..32).map(|i| if i >= k as usize { lanes[a][i - k as usize] } else { AigLit::FALSE }).collect(),
            LaneOp::Shr(a, k) => (0..32).map(|i| if i + (k as usize) < 32 { lanes[a][i + k as usize] } else { AigLit::FALSE }).collect(),
            LaneOp::Add(..) | LaneOp::Sub(..) | LaneOp::MulLo(..) => {
                return Err(format!("register {r}: arithmetic in a wiring program"));
            }
        };
        lanes.push(bits);
    }
    for (half, &r) in program.outputs.iter().enumerate() {
        for i in 0..32 {
            g.add_output(lanes[r][i], Some(format!("o{}", 32 * half + i)));
        }
    }
    Ok(g)
}

/// The operation's definition as a circuit over the same named bits.
fn definition_circuit(op: Op64) -> Aig {
    let mut g = Aig::new();
    let a: Vec<AigLit> = (0..64).map(|i| g.add_input(Some(format!("a{i}")))).collect();
    let b: Vec<AigLit> =
        if op.arity() == 2 { (0..64).map(|i| g.add_input(Some(format!("b{i}")))).collect() } else { Vec::new() };
    let out: Vec<AigLit> = (0..64)
        .map(|i| match op {
            Op64::And => g.and(a[i], b[i]),
            Op64::Or => g.or(a[i], b[i]),
            Op64::Xor => g.xor(a[i], b[i]),
            Op64::Shl(k) => {
                if i >= k as usize {
                    a[i - k as usize]
                } else {
                    AigLit::FALSE
                }
            }
            Op64::Shr(k) => {
                if i + (k as usize) < 64 {
                    a[i + k as usize]
                } else {
                    AigLit::FALSE
                }
            }
            _ => unreachable!("only wiring has a circuit definition"),
        })
        .collect();
    for (i, l) in out.into_iter().enumerate() {
        g.add_output(l, Some(format!("o{i}")));
    }
    g
}

/// Whether the program's circuit and the definition's are equal, with a derivation that replays.
fn circuits_agree(program: &LaneProgram) -> Result<bool, String> {
    let left = program_circuit(program)?;
    let right = definition_circuit(program.op);
    let report = fraig::equivalent(&left, &right, &fraig::Options::default()).map_err(|e| format!("{e:?}"))?;
    Ok(report.verdicts.len() == 64 && report.all_equal() && report.check())
}

// =================================================================================================================
// Proofs
// =================================================================================================================

/// How a lowering is proved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofKind {
    /// Identities over the integers, each with what the kernels said of it, and the intervals (checked here) that make
    /// every lane operation exact and turn the identities' congruences into equalities.
    Algebra(Vec<(Identity, KernelVerdict)>),
    /// A certified equivalence check of the bit-blasted program against the bit-blasted definition.
    Circuit,
}

/// A lowering's proof: the program, and how it is proved equal to its 64-bit definition.
#[derive(Clone, Debug)]
pub struct Proof {
    program: LaneProgram,
    kind: ProofKind,
}

impl Proof {
    pub fn kind(&self) -> &ProofKind {
        &self.kind
    }

    pub fn program(&self) -> &LaneProgram {
        &self.program
    }

    /// Checks the proof afresh: the program evaluated symbolically again, its identities and intervals derived again,
    /// the identities equal to those the proof carries, each accepted by the first kernel and never contradicted by the
    /// second, with the verdicts the proof records; or the circuits' equivalence decided again and its derivation
    /// replayed.
    pub fn check(&self) -> bool {
        if self.program != lower(self.program.op) {
            return false;
        }
        match &self.kind {
            ProofKind::Circuit => circuits_agree(&self.program).unwrap_or(false),
            ProofKind::Algebra(recorded) => {
                let derived = match identities_of(&self.program) {
                    Ok(Some(ids)) => ids,
                    _ => return false,
                };
                if derived.len() != recorded.len() || derived.iter().zip(recorded).any(|(d, (r, _))| d != r) {
                    return false;
                }
                KERNEL.with(|ctx| {
                    recorded.iter().all(|(id, verdict)| {
                        let now = kernels_on(ctx, id);
                        now != KernelVerdict::Rejected && now == *verdict
                    })
                })
            }
        }
    }

    /// The same program with `kind` in place of its proof, to be checked.
    pub fn with_kind(&self, kind: ProofKind) -> Proof {
        Proof { program: self.program.clone(), kind }
    }

    /// The identities both kernels decided, and all of them; zero and zero for a circuit proof.
    pub fn second_kernel_decided(&self) -> (usize, usize) {
        match &self.kind {
            ProofKind::Algebra(ids) => (ids.iter().filter(|(_, v)| *v == KernelVerdict::Both).count(), ids.len()),
            ProofKind::Circuit => (0, 0),
        }
    }
}

/// The identities that prove an arithmetic lowering; `None` for a wiring one.
fn identities_of(program: &LaneProgram) -> Result<Option<Vec<Identity>>, String> {
    match program.op {
        Op64::Add | Op64::Sub | Op64::Mul => ring_identities(program).map(Some),
        Op64::Rem(1) => {
            let alg = Algebra::run(program, &BTreeMap::new())?;
            let (_, elim, tree) = alg.result(program);
            if !elim.0.is_empty() {
                return Err("a remainder by one is not zero".into());
            }
            Ok(Some(vec![(tree, Syn::Lit(0))]))
        }
        Op64::Rem(d) => remainder_identities(program, d).map(Some),
        Op64::And | Op64::Or | Op64::Xor | Op64::Shl(_) | Op64::Shr(_) => Ok(None),
    }
}

thread_local! {
    /// The kernel's standard library, registered once a thread: building it is most of what checking one identity costs.
    static KERNEL: Context = {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        ctx
    };
}

/// The lowering of `op` and its proof.
pub fn prove(op: Op64) -> Result<Proof, String> {
    prove_program(lower(op))
}

/// A proof that `program` computes its operation, or why there is none. [`Proof::check`] accepts only a proof of the
/// lowering itself; this is the prover any program can be put to.
pub fn prove_program(program: LaneProgram) -> Result<Proof, String> {
    let op = program.op;
    let kind = match identities_of(&program)? {
        Some(ids) => {
            let mut verdicts = Vec::with_capacity(ids.len());
            for (i, id) in ids.into_iter().enumerate() {
                let verdict = KERNEL.with(|ctx| kernels_on(ctx, &id));
                if verdict == KernelVerdict::Rejected {
                    return Err(format!("{op:?}: the kernel rejects identity {i}"));
                }
                verdicts.push((id, verdict));
            }
            ProofKind::Algebra(verdicts)
        }
        None => {
            if !circuits_agree(&program)? {
                return Err(format!("{op:?}: the program's circuit and the definition's differ"));
            }
            ProofKind::Circuit
        }
    };
    let proof = Proof { program, kind };
    Ok(proof)
}
