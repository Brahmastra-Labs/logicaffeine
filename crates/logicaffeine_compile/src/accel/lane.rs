//! The lane machine: what a device's integer datapath can actually do, at the width it actually has.
//!
//! A device word is `word` bits (32 on a Plasticine PCU or an SN50 lane, two on the analog plane's digit
//! columns), and its multiplier takes operands of `mul` bits whose product fits one word. Every 64-bit integer
//! and binary64 operation a region needs is a [`Routine`] over these words, built once by the multi-word core
//! and the softfloat (`super::multiword`, `super::softfloat`) and executed by whichever [`Lanes`] the device
//! is: exact digital lanes, or the analog plane under its physics model. No routine assumes a unit the shape
//! does not declare.
//!
//! A routine is a small imperative program: instructions over lane registers, and fixed-count `Repeat`
//! blocks. Nothing in it branches on data; control that depends on a value is a [`LOp::Sel`], so the same
//! program is a straight-line datapath on a dataflow chip and a sequencer microprogram on the analog plane's
//! digital control side.

use serde::{Deserialize, Serialize};

/// The integer datapath a device has: `word`-bit words (a power of two dividing 32), and a multiplier whose
/// `mul`-bit operands give a product that fits one word (`2 × mul ≤ word`, `mul` dividing `word`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LaneShape {
    pub word: u32,
    pub mul: u32,
}

impl LaneShape {
    pub const fn new(word: u32, mul: u32) -> LaneShape {
        assert!(
            word >= 2 && word <= 32 && 32 % word == 0,
            "a lane word is 2, 4, 8, 16 or 32 bits"
        );
        assert!(
            mul >= 1 && 2 * mul <= word && word % mul == 0,
            "a lane multiplier's product fits one word"
        );
        LaneShape { word, mul }
    }

    /// The mask of one word.
    pub fn mask(self) -> u32 {
        if self.word == 32 {
            u32::MAX
        } else {
            (1u32 << self.word) - 1
        }
    }

    /// The words that hold `bits` bits.
    pub fn words_for(self, bits: u32) -> usize {
        bits.div_ceil(self.word) as usize
    }
}

/// A lane register: one word of the device's datapath.
pub type LReg = u32;

/// One lane operation. Operands are registers; results are words, carries and predicates are 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LOp {
    /// `dst = k`.
    Const(u32),
    /// `dst = a`.
    Mov(LReg),
    /// `dst = (a + b + cin) mod 2^w`, `dst2 = (a + b + cin) >> w`; `cin` is 0 or 1.
    Add {
        a: LReg,
        b: LReg,
        cin: LReg,
    },
    /// `dst = (a − b − bin) mod 2^w`, `dst2 = 1` when it borrowed; `bin` is 0 or 1.
    Sub {
        a: LReg,
        b: LReg,
        bin: LReg,
    },
    /// `dst = a × b` for `mul`-bit operands: the exact product, one word.
    Mul {
        a: LReg,
        b: LReg,
    },
    And {
        a: LReg,
        b: LReg,
    },
    Or {
        a: LReg,
        b: LReg,
    },
    Xor {
        a: LReg,
        b: LReg,
    },
    /// `dst = !a` within the word.
    Not {
        a: LReg,
    },
    /// `dst = (a << k) mod 2^w`, `k < w`.
    Shl {
        a: LReg,
        k: u32,
    },
    /// `dst = a >> k` (logical), `k < w`.
    Shr {
        a: LReg,
        k: u32,
    },
    /// `dst = 1` when `a < b` (unsigned), else 0.
    Ltu {
        a: LReg,
        b: LReg,
    },
    /// `dst = 1` when `a == b`, else 0.
    Eq {
        a: LReg,
        b: LReg,
    },
    /// `dst = c != 0 ? t : f`.
    Sel {
        c: LReg,
        t: LReg,
        f: LReg,
    },
}

/// An instruction: an operation and the register(s) it writes. `dst2` is the carry/borrow of `Add`/`Sub`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LInst {
    pub dst: LReg,
    pub dst2: Option<LReg>,
    pub op: LOp,
}

/// A statement: an instruction, or a body repeated a fixed number of times (registers carry across trips).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LStmt {
    Inst(LInst),
    Repeat { times: u32, body: Vec<LStmt> },
}

/// A lane routine: its inputs and outputs as registers, its body, and the register file size.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Routine {
    pub name: String,
    pub shape: LaneShape,
    pub regs: u32,
    pub inputs: Vec<LReg>,
    pub outputs: Vec<LReg>,
    pub body: Vec<LStmt>,
}

/// How many of each kind of lane operation a run performed: the cost a device pays for a routine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneCounts {
    pub add: u64,
    pub sub: u64,
    pub mul: u64,
    pub logic: u64,
    pub shift: u64,
    pub compare: u64,
    pub select: u64,
    pub moves: u64,
}

impl LaneCounts {
    pub fn total(&self) -> u64 {
        self.add
            + self.sub
            + self.mul
            + self.logic
            + self.shift
            + self.compare
            + self.select
            + self.moves
    }

    pub fn absorb(&mut self, o: &LaneCounts) {
        self.add += o.add;
        self.sub += o.sub;
        self.mul += o.mul;
        self.logic += o.logic;
        self.shift += o.shift;
        self.compare += o.compare;
        self.select += o.select;
        self.moves += o.moves;
    }
}

/// A device's datapath executing lane operations. The arithmetic ones are where devices differ (exact
/// digital units, or the analog plane settling a column); the rest are the digital side of every device.
pub trait Lanes {
    fn shape(&self) -> LaneShape;
    /// `(a + b + cin) mod 2^w` and its carry.
    fn add(&mut self, a: u32, b: u32, cin: u32) -> (u32, u32);
    /// `(a − b − bin) mod 2^w` and its borrow.
    fn sub(&mut self, a: u32, b: u32, bin: u32) -> (u32, u32);
    /// The exact product of two `mul`-bit operands.
    fn mul(&mut self, a: u32, b: u32) -> u32;
}

/// Exact digital lanes: the units a PCU or an RDU lane has, at their declared width.
#[derive(Clone, Copy, Debug)]
pub struct ExactLanes {
    pub shape: LaneShape,
}

impl Lanes for ExactLanes {
    fn shape(&self) -> LaneShape {
        self.shape
    }
    fn add(&mut self, a: u32, b: u32, cin: u32) -> (u32, u32) {
        let s = a as u64 + b as u64 + cin as u64;
        (
            (s & self.shape.mask() as u64) as u32,
            (s >> self.shape.word) as u32,
        )
    }
    fn sub(&mut self, a: u32, b: u32, bin: u32) -> (u32, u32) {
        let d = a as i64 - b as i64 - bin as i64;
        let borrow = u32::from(d < 0);
        ((d.rem_euclid(1i64 << self.shape.word)) as u32, borrow)
    }
    fn mul(&mut self, a: u32, b: u32) -> u32 {
        a * b
    }
}

/// Why a routine could not run: a builder bug the interpreter refuses to paper over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneFault {
    /// A word wider than the shape reached an operation.
    WideWord { op: &'static str, value: u32 },
    /// A multiplier operand wider than `mul` bits.
    WideMultiplicand { value: u32 },
    /// A carry-in or borrow-in that is not 0 or 1.
    NotABit { value: u32 },
    /// The routine was given the wrong number of input words.
    Arity { expected: usize, got: usize },
}

impl std::fmt::Display for LaneFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaneFault::WideWord { op, value } => {
                write!(f, "{op}: operand {value:#x} is wider than the lane word")
            }
            LaneFault::WideMultiplicand { value } => {
                write!(f, "mul: operand {value:#x} is wider than the multiplier")
            }
            LaneFault::NotABit { value } => write!(f, "carry {value} is not a bit"),
            LaneFault::Arity { expected, got } => {
                write!(f, "{got} input words where the routine takes {expected}")
            }
        }
    }
}

impl std::error::Error for LaneFault {}

/// Run `routine` on `lanes` with `inputs` (one word per input register); the output words, and what it cost.
pub fn run(
    routine: &Routine,
    lanes: &mut dyn Lanes,
    inputs: &[u32],
) -> Result<(Vec<u32>, LaneCounts), LaneFault> {
    if inputs.len() != routine.inputs.len() {
        return Err(LaneFault::Arity {
            expected: routine.inputs.len(),
            got: inputs.len(),
        });
    }
    let shape = lanes.shape();
    let mut regs = vec![0u32; routine.regs as usize];
    for (r, v) in routine.inputs.iter().zip(inputs) {
        if *v & !shape.mask() != 0 {
            return Err(LaneFault::WideWord {
                op: "input",
                value: *v,
            });
        }
        regs[*r as usize] = *v;
    }
    let mut counts = LaneCounts::default();
    exec(&routine.body, &mut regs, lanes, shape, &mut counts)?;
    Ok((
        routine.outputs.iter().map(|r| regs[*r as usize]).collect(),
        counts,
    ))
}

fn exec(
    body: &[LStmt],
    regs: &mut [u32],
    lanes: &mut dyn Lanes,
    shape: LaneShape,
    counts: &mut LaneCounts,
) -> Result<(), LaneFault> {
    for stmt in body {
        match stmt {
            LStmt::Repeat { times, body } => {
                for _ in 0..*times {
                    exec(body, regs, lanes, shape, counts)?;
                }
            }
            LStmt::Inst(inst) => step(inst, regs, lanes, shape, counts)?,
        }
    }
    Ok(())
}

fn step(
    inst: &LInst,
    regs: &mut [u32],
    lanes: &mut dyn Lanes,
    shape: LaneShape,
    counts: &mut LaneCounts,
) -> Result<(), LaneFault> {
    let mask = shape.mask();
    let r = |x: LReg| regs[x as usize];
    let bit = |v: u32| {
        if v > 1 {
            Err(LaneFault::NotABit { value: v })
        } else {
            Ok(v)
        }
    };
    let (v, v2) = match inst.op {
        LOp::Const(k) => {
            if k & !mask != 0 {
                return Err(LaneFault::WideWord {
                    op: "const",
                    value: k,
                });
            }
            counts.moves += 1;
            (k, None)
        }
        LOp::Mov(a) => {
            counts.moves += 1;
            (r(a), None)
        }
        LOp::Add { a, b, cin } => {
            counts.add += 1;
            let (s, c) = lanes.add(r(a), r(b), bit(r(cin))?);
            (s, Some(c))
        }
        LOp::Sub { a, b, bin } => {
            counts.sub += 1;
            let (d, bo) = lanes.sub(r(a), r(b), bit(r(bin))?);
            (d, Some(bo))
        }
        LOp::Mul { a, b } => {
            counts.mul += 1;
            let limit = 1u32 << shape.mul;
            for x in [r(a), r(b)] {
                if x >= limit {
                    return Err(LaneFault::WideMultiplicand { value: x });
                }
            }
            (lanes.mul(r(a), r(b)), None)
        }
        LOp::And { a, b } => {
            counts.logic += 1;
            (r(a) & r(b), None)
        }
        LOp::Or { a, b } => {
            counts.logic += 1;
            (r(a) | r(b), None)
        }
        LOp::Xor { a, b } => {
            counts.logic += 1;
            (r(a) ^ r(b), None)
        }
        LOp::Not { a } => {
            counts.logic += 1;
            (!r(a) & mask, None)
        }
        LOp::Shl { a, k } => {
            counts.shift += 1;
            ((r(a) << k) & mask, None)
        }
        LOp::Shr { a, k } => {
            counts.shift += 1;
            (r(a) >> k, None)
        }
        LOp::Ltu { a, b } => {
            counts.compare += 1;
            (u32::from(r(a) < r(b)), None)
        }
        LOp::Eq { a, b } => {
            counts.compare += 1;
            (u32::from(r(a) == r(b)), None)
        }
        LOp::Sel { c, t, f } => {
            counts.select += 1;
            (if r(c) != 0 { r(t) } else { r(f) }, None)
        }
    };
    regs[inst.dst as usize] = v;
    if let (Some(d2), Some(x)) = (inst.dst2, v2) {
        regs[d2 as usize] = x;
    }
    Ok(())
}

/// Split a 64-bit value into `n` little-endian words of `shape.word` bits.
pub fn split_u64(shape: LaneShape, x: u64, n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let at = i as u32 * shape.word;
            if at >= 64 {
                0
            } else {
                ((x >> at) as u32) & shape.mask()
            }
        })
        .collect()
}

/// Join little-endian words back into a 64-bit value (words past bit 63 are ignored).
pub fn join_u64(shape: LaneShape, words: &[u32]) -> u64 {
    let mut x = 0u64;
    for (i, w) in words.iter().enumerate() {
        let at = i as u32 * shape.word;
        if at < 64 {
            x |= (*w as u64) << at;
        }
    }
    x
}

/// Split a 128-bit value into `n` little-endian words.
pub fn split_u128(shape: LaneShape, x: u128, n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| {
            let at = i as u32 * shape.word;
            if at >= 128 {
                0
            } else {
                ((x >> at) as u32) & shape.mask()
            }
        })
        .collect()
}

/// Join little-endian words into a 128-bit value.
pub fn join_u128(shape: LaneShape, words: &[u32]) -> u128 {
    let mut x = 0u128;
    for (i, w) in words.iter().enumerate() {
        let at = i as u32 * shape.word;
        if at < 128 {
            x |= (*w as u128) << at;
        }
    }
    x
}
