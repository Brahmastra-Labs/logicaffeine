//! fabric_pipe — the compiled residual circuit cut into register stages: a new input every clock, a short period.
//!
//! [`fabric_rtl`](super::fabric_rtl) lowers a dispatch-free residual to a dataflow circuit that resolves in one clock,
//! so its period is the propagation delay of the whole residual (for the Horner polynomial, two chained 64-bit
//! multiplies and three carry chains). This module keeps the same op vocabulary and pipelines that circuit:
//!
//! 1. **Carry-save lowering** ([`carry_save_lower`]). Every additive tree (the `Add`/`Sub`/`Mul` ops a value is built
//!    from, through intermediates read exactly once) becomes one set of rows. A variable multiply contributes one
//!    masked, shifted copy of its multiplicand per multiplier bit; a constant multiply one shifted row per canonical
//!    signed digit; a subtraction the inverted rows plus their +1s; constants fold into a single row. A tree of 3:2
//!    compressors reduces the rows to two, and one Kogge-Stone carry-propagate add resolves them. The lowered circuit
//!    holds only bitwise ops and constant shifts, so no op is more than one gate deep and a register can sit between
//!    any two of them.
//! 2. **Scheduling** ([`schedule_pipeline`]). Ops take the earliest stage whose accumulated depth stays within a
//!    per-stage budget, in the units of [`op_delay`]. Wiring ([`is_wiring`]: a constant shift, or a bitwise op with a
//!    constant) has no depth and no stage of its own: each reader rebuilds it from its operand's register, so only
//!    computed values cross registers.
//! 3. **Emission** ([`emit_fabric_pipelined_verilog`]). Inputs register into stage 0 ([`Inputs::Registered`]), or
//!    stage 0 reads the ports the registers upstream hold ([`Inputs::Ports`]); each value is delayed through as many
//!    registers as stages separate it from its last reader, and the result registers after its stage. The result of
//!    the input presented before clock `t` is on `result` after clock `t + latency - 1`.
//!
//! [`simulate_pipeline`] is the cycle model of exactly those registers, so a schedule is checked without a simulator,
//! and the lowering takes a width so our SAT can check it exhaustively at a narrow one.

use super::fabric_rtl::{
    emit_loop_fabric_verilog_pipelined, loop_fabric_from_source, poly_program, residual_from_source, AluOp, CmpKind,
    DecisionFabric, FabricOp, LoopCell, Src,
};

/// Wiring costs no logic depth: a shift by a constant moves bits, and a bitwise op with a constant operand is
/// constant bits or inverters. It takes its operands' stage, and each reader rebuilds it from their registers.
pub fn is_wiring(op: &FabricOp) -> bool {
    let konst = |s: &Src| matches!(s, Src::Const(_));
    match op.op {
        AluOp::Shl | AluOp::Shr | AluOp::UShr => konst(&op.b),
        AluOp::And | AluOp::Or | AluOp::Xor => konst(&op.a) || konst(&op.b),
        AluOp::Add | AluOp::Sub | AluOp::Mul => false,
    }
}

/// The logic depth of one op, in units of a two-input XOR at the synthesis corner. Wiring is zero. A carry chain or a
/// multiplier left unlowered is one indivisible op, so it occupies a stage of its own whatever the budget.
pub fn op_delay(op: &FabricOp) -> f64 {
    if is_wiring(op) {
        return 0.0;
    }
    match op.op {
        AluOp::Xor => 1.0,
        AluOp::And | AluOp::Or => 0.6,
        AluOp::Add | AluOp::Sub => 9.0,
        AluOp::Mul => 40.0,
        AluOp::Shl | AluOp::Shr | AluOp::UShr => 4.0,
    }
}

/// Builds the lowered op list, folding constants and dropping identities so no op is emitted that wiring replaces. A
/// sharing builder emits each op once: an op it has already emitted, its operands in either order when it commutes, is
/// read again rather than built twice.
struct Builder {
    ops: Vec<FabricOp>,
    shared: Option<std::collections::HashMap<FabricOp, Src>>,
}

impl Builder {
    fn new() -> Builder {
        Builder { ops: Vec::new(), shared: None }
    }

    fn sharing() -> Builder {
        Builder { ops: Vec::new(), shared: Some(std::collections::HashMap::new()) }
    }

    fn push(&mut self, op: AluOp, a: Src, b: Src) -> Src {
        let key = FabricOp { op, a, b };
        if let Some(seen) = &self.shared {
            let commutes = matches!(op, AluOp::Add | AluOp::Mul | AluOp::And | AluOp::Or | AluOp::Xor);
            if let Some(&s) = seen.get(&key).or_else(|| commutes.then(|| seen.get(&FabricOp { op, a: b, b: a })).flatten()) {
                return s;
            }
        }
        self.ops.push(key);
        let s = Src::Val(self.ops.len() - 1);
        if let Some(seen) = &mut self.shared {
            seen.insert(key, s);
        }
        s
    }

    /// Constant operands fold only through ops whose low bits depend on low bits alone, so a fold at 64 bits is the
    /// same fold at every narrower width; right shifts of constants are never formed.
    fn emit(&mut self, op: AluOp, a: Src, b: Src) -> Src {
        use AluOp::*;
        match (op, a, b) {
            (And | Or | Xor | Add | Sub | Mul | Shl, Src::Const(x), Src::Const(y)) => Src::Const(op.eval(x, y)),
            (And, Src::Const(0), _) | (And, _, Src::Const(0)) => Src::Const(0),
            (And, Src::Const(-1), s) | (And, s, Src::Const(-1)) => s,
            (Or | Xor, Src::Const(0), s) | (Or | Xor, s, Src::Const(0)) => s,
            (Shl | Shr | UShr, s, Src::Const(0)) => s,
            _ => self.push(op, a, b),
        }
    }
}

/// The canonical signed digits of `c` mod 2^`width`: `(shift, ±1)` pairs whose signed sum of powers of two is `c`,
/// no two adjacent. The top digit is taken positive, since ±2^(width-1) agree mod 2^width.
pub fn csd_digits(c: i64, width: u32) -> Vec<(u32, i64)> {
    let modulus = 1u128 << width;
    let mut v = (c as u64 as u128) % modulus;
    let mut digits = Vec::new();
    let mut k = 0u32;
    while v != 0 && k < width {
        if v & 1 == 1 {
            let d: i64 = if k + 1 == width || v & 3 == 1 { 1 } else { -1 };
            digits.push((k, d));
            v = if d == 1 { v - 1 } else { v + 1 };
        }
        v >>= 1;
        k += 1;
    }
    digits
}

fn width_mask(width: u32) -> i64 {
    if width == 64 {
        -1
    } else {
        ((1u64 << width) - 1) as i64
    }
}

/// The words of one additive tree before reduction: each a value and whether it is subtracted, plus one constant.
#[derive(Default)]
struct Rows {
    words: Vec<(Src, bool)>,
    constant: i64,
}

impl Rows {
    fn push(&mut self, s: Src, negated: bool) {
        match s {
            Src::Const(c) if negated => self.constant = self.constant.wrapping_sub(c),
            Src::Const(c) => self.constant = self.constant.wrapping_add(c),
            s => self.words.push((s, negated)),
        }
    }
}

struct Lowering<'a> {
    ops: &'a [FabricOp],
    width: u32,
    final_add: FinalAdd,
    resolves: Vec<bool>,
    /// A sum whose rows its additive readers take even though it resolves for its other readers.
    folds: Vec<bool>,
    lowered: Vec<Option<Src>>,
    out: Builder,
}

fn is_additive(op: &FabricOp) -> bool {
    matches!(op.op, AluOp::Add | AluOp::Sub | AluOp::Mul)
}

impl Lowering<'_> {
    fn leaf(&self, s: Src) -> Src {
        match s {
            Src::Val(k) => self.lowered[k].expect("a value is lowered before its readers"),
            other => other,
        }
    }

    fn expand(&mut self, s: Src, negated: bool, rows: &mut Rows) {
        match s {
            Src::Val(k) if is_additive(&self.ops[k]) && (!self.resolves[k] || self.folds[k]) => {
                self.expand_op(k, negated, rows)
            }
            other => {
                let leaf = self.leaf(other);
                rows.push(leaf, negated);
            }
        }
    }

    fn expand_op(&mut self, k: usize, negated: bool, rows: &mut Rows) {
        let op = self.ops[k];
        match op.op {
            AluOp::Add => {
                self.expand(op.a, negated, rows);
                self.expand(op.b, negated, rows);
            }
            AluOp::Sub => {
                self.expand(op.a, negated, rows);
                self.expand(op.b, !negated, rows);
            }
            AluOp::Mul => self.multiply_rows(op.a, op.b, negated, rows),
            _ => unreachable!("only additive ops expand into rows"),
        }
    }

    /// A multiply's rows. Its operands resolve to binary first: a variable multiplier is read bit by bit, and each bit
    /// masks one shifted copy of the multiplicand.
    fn multiply_rows(&mut self, a: Src, b: Src, negated: bool, rows: &mut Rows) {
        let w = self.width;
        match (self.leaf(a), self.leaf(b)) {
            (Src::Const(x), Src::Const(y)) => rows.push(Src::Const(x.wrapping_mul(y)), negated),
            (s, Src::Const(c)) | (Src::Const(c), s) => {
                for (k, d) in csd_digits(c, w) {
                    let row = self.out.emit(AluOp::Shl, s, Src::Const(k as i64));
                    rows.push(row, negated != (d < 0));
                }
            }
            (x, y) => {
                for i in 0..w {
                    let shifted = self.out.emit(AluOp::Shl, x, Src::Const(i as i64));
                    let top = self.out.emit(AluOp::Shl, y, Src::Const((w - 1 - i) as i64));
                    let mask = self.out.emit(AluOp::Shr, top, Src::Const((w - 1) as i64));
                    let row = self.out.emit(AluOp::And, shifted, mask);
                    rows.push(row, negated);
                }
            }
        }
    }

    /// One 3:2 compressor across whole words: the bitwise sum, and the bitwise majority moved up one place.
    fn compress(&mut self, a: Src, b: Src, c: Src) -> (Src, Src) {
        let ab = self.out.emit(AluOp::Xor, a, b);
        let sum = self.out.emit(AluOp::Xor, ab, c);
        let both = self.out.emit(AluOp::And, a, b);
        let carried = self.out.emit(AluOp::And, c, ab);
        let majority = self.out.emit(AluOp::Or, both, carried);
        let carry = self.out.emit(AluOp::Shl, majority, Src::Const(1));
        (sum, carry)
    }

    /// The Kogge-Stone carry-propagate add: group generate and propagate doubled in span each level, so the carry into
    /// every bit is ready after log2(width) levels.
    fn prefix_add(&mut self, a: Src, b: Src) -> Src {
        if let (Src::Const(x), Src::Const(y)) = (a, b) {
            return Src::Const(x.wrapping_add(y));
        }
        let mut generate = self.out.emit(AluOp::And, a, b);
        let half_sum = self.out.emit(AluOp::Xor, a, b);
        let mut propagate = half_sum;
        let mut span = 1u32;
        while span < self.width {
            let lower = self.out.emit(AluOp::Shl, generate, Src::Const(span as i64));
            let through = self.out.emit(AluOp::And, propagate, lower);
            generate = self.out.emit(AluOp::Or, generate, through);
            if 2 * span < self.width {
                let lower = self.out.emit(AluOp::Shl, propagate, Src::Const(span as i64));
                propagate = self.out.emit(AluOp::And, propagate, lower);
            }
            span *= 2;
        }
        let carries = self.out.emit(AluOp::Shl, generate, Src::Const(1));
        self.out.emit(AluOp::Xor, half_sum, carries)
    }

    fn resolve(&mut self, rows: Rows) -> Src {
        let mut constant = rows.constant;
        let mut words: Vec<Src> = Vec::with_capacity(rows.words.len() + 1);
        for (s, negated) in rows.words {
            if negated {
                words.push(self.out.emit(AluOp::Xor, s, Src::Const(-1)));
                constant = constant.wrapping_add(1);
            } else {
                words.push(s);
            }
        }
        if constant & width_mask(self.width) != 0 {
            words.push(Src::Const(constant));
        }
        if words.is_empty() {
            return Src::Const(0);
        }
        while words.len() > 2 {
            let mut next = Vec::with_capacity(words.len() * 2 / 3 + 2);
            let mut triples = words.chunks_exact(3);
            for t in &mut triples {
                let (sum, carry) = self.compress(t[0], t[1], t[2]);
                next.push(sum);
                next.push(carry);
            }
            next.extend_from_slice(triples.remainder());
            words = next;
        }
        if words.len() == 1 {
            words[0]
        } else if self.final_add == FinalAdd::Native {
            self.out.emit(AluOp::Add, words[0], words[1])
        } else {
            self.prefix_add(words[0], words[1])
        }
    }
}

/// How a sum that other sums read is lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumSharing {
    /// A sum read more than once resolves to binary once and every reader reads the resolved word: the fewest adders.
    Resolve,
    /// Every add or subtract that reads a sum of at most [`FOLD_ROW_LIMIT`] words takes the sum's rows into its own,
    /// so no carry-propagate add waits on another; the sum still resolves for any reader that is not an add or a
    /// subtract (a compare's overflow fix reads its sign), and every op is emitted once. More adders, each one carry
    /// chain from the inputs: the lowering for standard cells, where every carry chain in series costs its full delay.
    Fold,
}

/// The words a folded sum may bring into each reader's rows; a larger sum resolves once and is read as one word.
pub const FOLD_ROW_LIMIT: usize = 8;

/// How the two words a sum's compressor tree leaves are added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalAdd {
    /// A Kogge-Stone prefix network written out in bitwise ops: no op more than one gate deep, so a register can sit
    /// inside the add.
    PrefixTree,
    /// One add, left to synthesis, which picks the adder the clock needs (on standard cells, a smaller one than the
    /// widest prefix network wherever the clock allows).
    Native,
}

/// Carry-save lowering of a residual at datapath width `width` (64 on silicon; narrower so our SAT can check the
/// lowering exhaustively), each shared sum resolved once ([`SumSharing::Resolve`]). Only the values the result depends
/// on are lowered. Returns the new op list, which holds no `Add`, `Sub` or `Mul`, and the index of its result.
///
/// A value resolves to binary when it is the result, is read more than once, or is read by anything but an add or a
/// subtract: a multiply reads its operands bit by bit. Every other additive value is absorbed into its reader's rows.
pub fn carry_save_lower(ops: &[FabricOp], result: usize, width: u32) -> (Vec<FabricOp>, usize) {
    carry_save_lower_with(ops, result, width, SumSharing::Resolve, FinalAdd::PrefixTree)
}

/// [`carry_save_lower`] with the shared sums lowered as `sharing` says and each compressor tree's last two words added
/// as `final_add` says. Under [`SumSharing::Fold`] an add or subtract read only by adds and subtracts never resolves,
/// however many read it, and one its rows fold into resolves only for its other readers; a multiply read more than
/// once still resolves once. Under [`FinalAdd::Native`] the lowering holds one `Add` per resolved sum and nothing else
/// additive.
pub fn carry_save_lower_with(
    ops: &[FabricOp],
    result: usize,
    width: u32,
    sharing: SumSharing,
    final_add: FinalAdd,
) -> (Vec<FabricOp>, usize) {
    assert!((2..=64).contains(&width), "the datapath width is 2..=64 bits, not {width}");
    assert!(result < ops.len(), "the result names an op of the residual");
    let n = ops.len();
    let live = needed_by(ops, result);
    let mut readers = vec![0usize; n];
    let mut resolves = vec![false; n];
    resolves[result] = true;
    for op in ops.iter().zip(&live).filter(|(_, l)| **l).map(|(op, _)| op) {
        for s in [op.a, op.b] {
            if let Src::Val(k) = s {
                readers[k] += 1;
                if !matches!(op.op, AluOp::Add | AluOp::Sub) {
                    resolves[k] = true;
                }
            }
        }
    }
    let mut folds = vec![false; n];
    match sharing {
        SumSharing::Resolve => {
            for k in 0..n {
                resolves[k] |= readers[k] > 1;
            }
        }
        SumSharing::Fold => {
            // The words each sum brings into a reader's rows, in dataflow order: a folded or absorbed sum its own
            // rows, an unresolved multiply one row per multiplier bit, anything else one word.
            let mut words = vec![1usize; n];
            for k in 0..n {
                let term = |s: Src, words: &[usize], resolves: &[bool], folds: &[bool]| match s {
                    Src::Const(_) => 0,
                    Src::In(_) => 1,
                    Src::Val(j) if is_additive(&ops[j]) && (!resolves[j] || folds[j]) => words[j],
                    Src::Val(_) => 1,
                };
                match ops[k].op {
                    AluOp::Add | AluOp::Sub => {
                        words[k] = term(ops[k].a, &words, &resolves, &folds) + term(ops[k].b, &words, &resolves, &folds);
                        folds[k] = words[k] <= FOLD_ROW_LIMIT;
                        resolves[k] |= !folds[k] && readers[k] > 1;
                    }
                    AluOp::Mul => {
                        resolves[k] |= readers[k] > 1;
                        words[k] = width as usize;
                    }
                    _ => {}
                }
            }
        }
    }
    let mut lo = Lowering {
        ops,
        width,
        final_add,
        resolves,
        folds,
        lowered: vec![None; n],
        out: match sharing {
            SumSharing::Resolve => Builder::new(),
            SumSharing::Fold => Builder::sharing(),
        },
    };
    for i in 0..n {
        if !live[i] {
            continue;
        }
        let op = ops[i];
        if is_additive(&op) {
            if !lo.resolves[i] {
                continue;
            }
            let mut rows = Rows::default();
            lo.expand_op(i, false, &mut rows);
            let s = lo.resolve(rows);
            lo.lowered[i] = Some(s);
        } else {
            let (a, b) = (lo.leaf(op.a), lo.leaf(op.b));
            let s = lo.out.emit(op.op, a, b);
            lo.lowered[i] = Some(s);
        }
    }
    let result = match lo.lowered[result].expect("the result is lowered") {
        Src::Val(k) => k,
        other => {
            lo.out.push(AluOp::Or, other, Src::Const(0));
            lo.out.ops.len() - 1
        }
    };
    (lo.out.ops, result)
}

/// The stage budget a decision is cut to on a fabric with carry chains, in the units of [`op_delay`]: one unlowered
/// add or subtract (a carry chain, 9) and an AND. The XORs that finish a compare's sign start the next stage, so the
/// mask a select fans out to its 64 bits leaves a register, never the end of a carry chain. MEASURED (2026-10-17, K1's
/// `decide`, Artix-7 fabric map, yosys 0.33 synth_xilinx + nextpnr-xilinx, seeds 1-3, median): 12 (the compare and
/// its select in one stage) = 852 LUTs at 119.2 MHz, 10 = 848 at 147.3, 9.6 = 784 at 145.4, 6 = 848 at 147.8; at 8
/// and 9 one seed in three does not finish routing within 300 s.
pub const DECISION_CHAIN_BUDGET: f64 = 9.6;

/// The stage budget a decision is cut to on standard cells, in the units of [`op_delay`]: three carry chains and their
/// selects. A register is ~20 µm² of flip-flop a bit there, where on a fabric it rides in a LUT's slice, so a decision
/// takes few stages and reads its ports ([`Inputs::Ports`]); each compare takes the sums it reads into its own rows
/// ([`SumSharing::Fold`]), resolved by one add synthesis picks ([`FinalAdd::Native`]). MEASURED (2026-10-18, K1's
/// `decide` so lowered, its ports read, Sky130 HD: yosys 0.33 with OpenLane v1's `DELAY 0` ABC script at a 2 ns
/// target, OpenSTA at the typical corner, no wires): 18 = 5 stages, 44,306 µm² at 4.65 ns; 30 = 3 stages, 34,005 µm²
/// at 3.57 ns; 36 = 2 stages, 30,843 µm² at 5.03 ns; one stage, 32,013 µm² at 7.71 ns. Without the folds, 27 and 30
/// were 32,750 µm² at 4.10 ns and 31,559 µm² at 4.34 ns.
pub const DECISION_CELLS_BUDGET: f64 = 30.0;

/// A decision fabric ([`decision_fabric_from_source`](super::fabric_rtl::decision_fabric_from_source): arithmetic
/// cells and compare-selects) as a residual in the pipeline's op vocabulary, and the index of its result. A compare
/// becomes the mask of its outcome, all ones or zero ([`compare_mask`]); a select is `e ^ ((t ^ e) & mask)`. Every op is
/// emitted once, so a predicate many selects test has one mask. A fabric with any other cell (a functional unit, a
/// memory or heap tile) has no combinational form and is refused.
pub fn decision_ops(df: &DecisionFabric) -> Result<(Vec<FabricOp>, usize), String> {
    let mut out = Builder::sharing();
    let mut vals: Vec<Src> = Vec::with_capacity(df.cells.len());
    let at = |s: Src, vals: &[Src]| match s {
        Src::Val(k) => vals[k],
        other => other,
    };
    for (i, cell) in df.cells.iter().enumerate() {
        let v = match *cell {
            LoopCell::Arith { op, a, b } => out.emit(op, at(a, &vals), at(b, &vals)),
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let mask = compare_mask(&mut out, cmp, at(ca, &vals), at(cb, &vals));
                let (t, e) = (at(t, &vals), at(e, &vals));
                let differ = out.emit(AluOp::Xor, t, e);
                let taken = out.emit(AluOp::And, differ, mask);
                out.emit(AluOp::Xor, e, taken)
            }
            ref other => return Err(format!("cell {i} ({other:?}) has no combinational form in a pipeline")),
        };
        vals.push(v);
    }
    let result = match at(df.result, &vals) {
        Src::Val(k) => k,
        other => {
            out.push(AluOp::Or, other, Src::Const(0));
            out.ops.len() - 1
        }
    };
    Ok((out.ops, result))
}

/// All ones when `x cmp y` holds on the signed words, zero when it does not. `x < y` is the sign of the true
/// difference: the sign of `d = x − y`, flipped when the subtraction overflowed (`x` and `y` differ in sign and `d`
/// differs from `x`), spread across the word by an arithmetic shift. `x ≠ y` ORs every bit of `z = x ^ y` up into the
/// sign (`z | z << 1`, then `<< 2` … `<< 32`), an OR tree with no carry chain, and spreads it the same way. The other
/// compares swap or invert those.
fn compare_mask(out: &mut Builder, cmp: CmpKind, x: Src, y: Src) -> Src {
    fn less(out: &mut Builder, a: Src, b: Src) -> Src {
        let d = out.emit(AluOp::Sub, a, b);
        let signs = out.emit(AluOp::Xor, a, b);
        let moved = out.emit(AluOp::Xor, a, d);
        let overflow = out.emit(AluOp::And, signs, moved);
        let sign = out.emit(AluOp::Xor, d, overflow);
        out.emit(AluOp::Shr, sign, Src::Const(63))
    }
    fn differ(out: &mut Builder, a: Src, b: Src) -> Src {
        let mut any = out.emit(AluOp::Xor, a, b);
        for k in [1, 2, 4, 8, 16, 32] {
            let up = out.emit(AluOp::Shl, any, Src::Const(k));
            any = out.emit(AluOp::Or, any, up);
        }
        out.emit(AluOp::Shr, any, Src::Const(63))
    }
    let mask = match cmp {
        CmpKind::Lt | CmpKind::Ge => less(out, x, y),
        CmpKind::Gt | CmpKind::Le => less(out, y, x),
        CmpKind::Ne | CmpKind::Eq => differ(out, x, y),
    };
    match cmp {
        CmpKind::Ge | CmpKind::Le | CmpKind::Eq => out.emit(AluOp::Xor, mask, Src::Const(-1)),
        CmpKind::Lt | CmpKind::Gt | CmpKind::Ne => mask,
    }
}

/// Where a pipeline's first stage reads its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inputs {
    /// The pipeline registers its inputs, and stage 0 reads those registers, whatever drives the ports.
    Registered,
    /// Stage 0 reads the ports, which the registers driving them hold for the clock: a pipeline whose upstream
    /// registers what it presents (the comparative bench's harness, a register file) registers nothing twice.
    Ports,
}

/// Where every op of a residual computes, and which op leaves the pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineSchedule {
    /// The stage of each op; wiring carries its operands' stage.
    pub stage: Vec<usize>,
    pub result: usize,
    /// The per-stage depth budget the schedule was cut to, in the units of [`op_delay`].
    pub budget: f64,
    /// Where stage 0 reads the inputs.
    pub inputs: Inputs,
}

impl PipelineSchedule {
    /// The register boundaries between the inputs and the output register, plus one.
    pub fn stages(&self) -> usize {
        self.stage[self.result] + 1
    }

    /// Clocks from presenting an input to its result on the output register: the input register when there is one,
    /// the boundaries between stages, and the output register.
    pub fn latency(&self) -> usize {
        self.stage[self.result] + 1 + usize::from(self.inputs == Inputs::Registered)
    }

    /// The same stages, reading the inputs from `inputs`.
    pub fn reading(self, inputs: Inputs) -> PipelineSchedule {
        PipelineSchedule { inputs, ..self }
    }

    /// The deepest path inside any one stage of the emitted pipeline, in the units of [`op_delay`]: a value read in
    /// its own stage arrives when it is computed, and a value read in a later stage arrives from a register at depth
    /// zero. Ops the result does not need are never emitted and never counted.
    pub fn stage_depth(&self, ops: &[FabricOp]) -> f64 {
        let needed = needed_by(ops, self.result);
        stage_arrivals(ops, &self.stage)
            .into_iter()
            .zip(needed)
            .filter(|(_, n)| *n)
            .map(|(a, _)| a)
            .fold(0.0, f64::max)
    }

    /// The schedule respects dataflow: no op computes before an operand, and wiring sits in its operands' stage.
    pub fn check(&self, ops: &[FabricOp]) -> Result<(), String> {
        if self.stage.len() != ops.len() {
            return Err(format!("{} stages for {} ops", self.stage.len(), ops.len()));
        }
        for (i, op) in ops.iter().enumerate() {
            let mut latest = 0;
            for s in [op.a, op.b] {
                if let Src::Val(k) = s {
                    if k >= i {
                        return Err(format!("op {i} reads v{k}, which is not computed before it"));
                    }
                    if self.stage[k] > self.stage[i] {
                        return Err(format!(
                            "op {i} in stage {} reads v{k} from the later stage {}",
                            self.stage[i], self.stage[k]
                        ));
                    }
                    latest = latest.max(self.stage[k]);
                }
            }
            if is_wiring(op) && self.stage[i] != latest {
                return Err(format!(
                    "wiring op {i} sits in stage {} rather than its operands' stage {latest}",
                    self.stage[i]
                ));
            }
        }
        Ok(())
    }
}

/// The ops `result` depends on, itself included.
fn needed_by(ops: &[FabricOp], result: usize) -> Vec<bool> {
    let mut needed = vec![false; ops.len()];
    needed[result] = true;
    for i in (0..ops.len()).rev() {
        if needed[i] {
            for s in [ops[i].a, ops[i].b] {
                if let Src::Val(k) = s {
                    needed[k] = true;
                }
            }
        }
    }
    needed
}

fn stage_arrivals(ops: &[FabricOp], stage: &[usize]) -> Vec<f64> {
    let mut arrival = vec![0.0f64; ops.len()];
    for (i, op) in ops.iter().enumerate() {
        let start = [op.a, op.b]
            .into_iter()
            .filter_map(|s| match s {
                Src::Val(k) if stage[k] == stage[i] => Some(arrival[k]),
                _ => None,
            })
            .fold(0.0, f64::max);
        arrival[i] = start + op_delay(op);
    }
    arrival
}

/// The bits of each value its construction fixes, as (mask of known bits, their values): shifts bring in zeros,
/// masks and constants pin bits, a carry row's bit 0 is zero. A register on a known bit holds a constant, which
/// synthesis removes, so a schedule is charged only for the live bits it holds.
pub fn known_bits(ops: &[FabricOp]) -> Vec<(u64, u64)> {
    let mut known: Vec<(u64, u64)> = Vec::with_capacity(ops.len());
    for op in ops {
        let of = |s: Src, known: &[(u64, u64)]| match s {
            Src::Const(c) => (u64::MAX, c as u64),
            Src::In(_) => (0, 0),
            Src::Val(k) => known[k],
        };
        let ((ma, va), (mb, vb)) = (of(op.a, &known), of(op.b, &known));
        let low_zeros = |m: u64, v: u64| (!(!m | v)).trailing_ones();
        let (mask, value) = match (op.op, op.b) {
            (AluOp::And, _) => {
                let zeros = (ma & !va) | (mb & !vb);
                let mask = (ma & mb) | zeros;
                (mask, va & vb & mask)
            }
            (AluOp::Or, _) => {
                let ones = (ma & va) | (mb & vb);
                let mask = (ma & mb) | ones;
                (mask, (va | vb) & mask)
            }
            (AluOp::Xor, _) => (ma & mb, (va ^ vb) & ma & mb),
            (AluOp::Shl, Src::Const(k)) => {
                let k = (k & 63) as u32;
                let low = if k == 0 { 0 } else { u64::MAX >> (64 - k) };
                (ma.wrapping_shl(k) | low, va.wrapping_shl(k))
            }
            (AluOp::UShr, Src::Const(k)) => {
                let k = (k & 63) as u32;
                let high = if k == 0 { 0 } else { u64::MAX << (64 - k) };
                (ma.wrapping_shr(k) | high, va.wrapping_shr(k))
            }
            (AluOp::Shr, Src::Const(k)) => {
                let k = (k & 63) as u32;
                let sign_known = ma >> 63 == 1;
                let high = if k == 0 || !sign_known { 0 } else { u64::MAX << (64 - k) };
                let mask = ma.wrapping_shr(k) | high;
                (mask, ((va as i64).wrapping_shr(k) as u64) & mask)
            }
            (AluOp::Add | AluOp::Sub, _) => {
                let t = low_zeros(ma, va).min(low_zeros(mb, vb));
                (if t == 0 { 0 } else { u64::MAX >> (64 - t) }, 0)
            }
            (AluOp::Mul, _) => {
                let t = (low_zeros(ma, va) + low_zeros(mb, vb)).min(64);
                (if t == 0 { 0 } else { u64::MAX >> (64 - t.min(64)) }, 0)
            }
            _ => (0, 0),
        };
        known.push((mask, value & mask));
    }
    known
}

fn live_bits(known: (u64, u64)) -> u64 {
    64 - u64::from(known.0.count_ones())
}

/// A residual's stage-assignment problem under a depth budget: the computed values the result depends on, what
/// each reads through wiring, the difference constraints any legal assignment satisfies, and each value's earliest
/// and latest stage in the shortest pipeline.
struct StageProblem {
    /// The ops the result depends on.
    needed: Vec<bool>,
    /// For each needed computed op, the inputs and computed values it reads through wiring.
    reads: Vec<Vec<Held>>,
    /// The inputs and computed values the output register reads through wiring.
    result_reads: Vec<Held>,
    /// `(x, y, w)`: stage(y) ≥ stage(x) + w, over computed ops.
    constraints: Vec<(usize, usize, usize)>,
    earliest: Vec<usize>,
    latest: Vec<usize>,
    /// The stage the output register reads in: the last stage of the shortest pipeline.
    last: usize,
}

fn sources(ops: &[FabricOp], s: Src, memo: &mut Vec<Option<Vec<Held>>>) -> Vec<Held> {
    match s {
        Src::Const(_) => Vec::new(),
        Src::In(k) => vec![Held::Input(k)],
        Src::Val(k) if is_wiring(&ops[k]) => {
            if memo[k].is_none() {
                let mut v = sources(ops, ops[k].a, memo);
                v.extend(sources(ops, ops[k].b, memo));
                v.sort();
                v.dedup();
                memo[k] = Some(v);
            }
            memo[k].clone().expect("memoized")
        }
        Src::Val(k) => vec![Held::Value(k)],
    }
}

impl StageProblem {
    fn new(ops: &[FabricOp], result: usize, budget: f64) -> StageProblem {
        assert!(budget > 0.0, "a stage budget is positive");
        assert!(result < ops.len(), "the result names an op of the residual");
        let n = ops.len();
        let needed = needed_by(ops, result);
        let computed = |i: usize| needed[i] && !is_wiring(&ops[i]);
        let mut memo = vec![None; n];
        let mut reads = vec![Vec::new(); n];
        for i in (0..n).filter(|&i| computed(i)) {
            let mut v = sources(ops, ops[i].a, &mut memo);
            v.extend(sources(ops, ops[i].b, &mut memo));
            v.sort();
            v.dedup();
            reads[i] = v;
        }
        let result_reads = sources(ops, Src::Val(result), &mut memo);
        let mut readers = vec![Vec::new(); n];
        for i in (0..n).filter(|&i| computed(i)) {
            for h in &reads[i] {
                if let Held::Value(k) = h {
                    readers[*k].push(i);
                }
            }
        }

        let mut constraints = Vec::new();
        for i in (0..n).filter(|&i| computed(i)) {
            for h in &reads[i] {
                if let Held::Value(k) = h {
                    constraints.push((*k, i, 0));
                }
            }
        }
        let mut reach = vec![f64::NAN; n];
        for u in (0..n).filter(|&u| computed(u)) {
            let mut frontier = std::collections::BinaryHeap::new();
            let mut touched = vec![u];
            reach[u] = op_delay(&ops[u]);
            if reach[u] <= budget {
                frontier.extend(readers[u].iter().map(|&r| std::cmp::Reverse(r)));
            } else {
                constraints.extend(readers[u].iter().map(|&r| (u, r, 1)));
            }
            while let Some(std::cmp::Reverse(v)) = frontier.pop() {
                if !reach[v].is_nan() {
                    continue;
                }
                let start = reads[v]
                    .iter()
                    .filter_map(|h| match h {
                        Held::Value(p) if !reach[*p].is_nan() => Some(reach[*p]),
                        _ => None,
                    })
                    .fold(0.0, f64::max);
                reach[v] = start + op_delay(&ops[v]);
                touched.push(v);
                if reach[v] > budget {
                    constraints.push((u, v, 1));
                } else {
                    frontier.extend(readers[v].iter().map(|&r| std::cmp::Reverse(r)));
                }
            }
            for t in touched {
                reach[t] = f64::NAN;
            }
        }
        constraints.sort();
        constraints.dedup();
        let mut strongest: std::collections::BTreeMap<(usize, usize), usize> = Default::default();
        for &(x, y, w) in &constraints {
            let e = strongest.entry((x, y)).or_insert(0);
            *e = (*e).max(w);
        }
        let constraints: Vec<(usize, usize, usize)> = strongest.into_iter().map(|((x, y), w)| (x, y, w)).collect();

        let mut earliest = vec![0usize; n];
        let mut incoming: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        let mut outgoing: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
        for &(x, y, w) in &constraints {
            incoming[y].push((x, w));
            outgoing[x].push((y, w));
        }
        for i in (0..n).filter(|&i| computed(i)) {
            earliest[i] = incoming[i].iter().map(|&(x, w)| earliest[x] + w).max().unwrap_or(0);
        }
        let last = result_reads
            .iter()
            .map(|h| match h {
                Held::Value(k) => earliest[*k],
                Held::Input(_) => 0,
            })
            .max()
            .unwrap_or(0);
        let mut latest = vec![last; n];
        for i in (0..n).rev().filter(|&i| computed(i)) {
            for &(y, w) in &outgoing[i] {
                latest[i] = latest[i].min(latest[y] - w);
            }
            debug_assert!(earliest[i] <= latest[i], "op {i}: earliest {} after latest {}", earliest[i], latest[i]);
        }
        StageProblem {
            needed,
            reads,
            result_reads,
            constraints,
            earliest,
            latest,
            last,
        }
    }

    /// A schedule from the stages of the computed ops: wiring and the ops the result does not need take their
    /// operands' latest stage.
    fn schedule(&self, ops: &[FabricOp], result: usize, budget: f64, computed_stage: &[usize]) -> PipelineSchedule {
        let mut stage = vec![0usize; ops.len()];
        for (i, op) in ops.iter().enumerate() {
            stage[i] = if self.needed[i] && !is_wiring(op) {
                computed_stage[i]
            } else {
                [op.a, op.b]
                    .into_iter()
                    .filter_map(|s| match s {
                        Src::Val(k) => Some(stage[k]),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0)
            };
        }
        PipelineSchedule {
            stage,
            result,
            budget,
            inputs: Inputs::Registered,
        }
    }
}

/// Every computed value in its earliest stage: the shortest pipeline whose stages each stay within `budget`.
pub fn earliest_schedule(ops: &[FabricOp], result: usize, budget: f64) -> PipelineSchedule {
    let p = StageProblem::new(ops, result, budget);
    p.schedule(ops, result, budget, &p.earliest)
}

/// Every computed value in its latest stage of that same shortest pipeline.
pub fn latest_schedule(ops: &[FabricOp], result: usize, budget: f64) -> PipelineSchedule {
    let p = StageProblem::new(ops, result, budget);
    p.schedule(ops, result, budget, &p.latest)
}

/// Cut a residual into the shortest pipeline whose stages each stay within `budget` (in the units of [`op_delay`]),
/// placing every computed value so the pipeline holds the fewest live register bits. An op deeper than the budget
/// on its own still gets a stage, alone.
///
/// The placement is exact. A legal assignment of stages is a set of difference constraints (a reader never sits
/// before what it reads; two values joined by a path deeper than the budget sit in different stages), and the bits
/// a value holds are its live bits times the stages between it and its last reader. With each value's stage
/// bounded by its earliest and latest, "stage(x) ≥ k" becomes a boolean, every constraint an implication between
/// them, and the cost a weight on each: the cheapest assignment is a minimum-weight closure, found as a minimum cut.
pub fn schedule_pipeline(ops: &[FabricOp], result: usize, budget: f64) -> PipelineSchedule {
    let p = StageProblem::new(ops, result, budget);
    let n = ops.len();
    let known = known_bits(ops);
    let bits = |h: Held| -> i64 {
        match h {
            Held::Input(_) => 64,
            Held::Value(k) => live_bits(known[k]) as i64,
        }
    };

    // Variables: every computed op, and one mirror per held value standing for its last reader's stage.
    let mut lo: Vec<usize> = Vec::new();
    let mut hi: Vec<usize> = Vec::new();
    let mut weight: Vec<i64> = Vec::new();
    let mut var_of_op = vec![usize::MAX; n];
    for i in 0..n {
        if p.needed[i] && !is_wiring(&ops[i]) {
            var_of_op[i] = lo.len();
            lo.push(p.earliest[i]);
            hi.push(p.latest[i]);
            weight.push(0);
        }
    }
    let mut readers_of: std::collections::BTreeMap<Held, Vec<usize>> = Default::default();
    for i in 0..n {
        if var_of_op[i] != usize::MAX {
            for &h in &p.reads[i] {
                readers_of.entry(h).or_default().push(var_of_op[i]);
            }
        }
    }
    for &h in &p.result_reads {
        readers_of.entry(h).or_default();
    }
    let mut edges: Vec<(usize, usize, usize)> = p
        .constraints
        .iter()
        .map(|&(x, y, w)| (var_of_op[x], var_of_op[y], w))
        .collect();
    for (&h, readers) in &readers_of {
        let reads_result = p.result_reads.contains(&h);
        let mlo = readers
            .iter()
            .map(|&r| lo[r])
            .chain(reads_result.then_some(p.last))
            .chain(match h {
                Held::Value(k) => Some(p.earliest[k]),
                Held::Input(_) => Some(0),
            })
            .max()
            .unwrap_or(0);
        let mhi = readers
            .iter()
            .map(|&r| hi[r])
            .chain(reads_result.then_some(p.last))
            .max()
            .unwrap_or(mlo)
            .max(mlo);
        let m = lo.len();
        lo.push(mlo);
        hi.push(mhi);
        weight.push(bits(h));
        if let Held::Value(k) = h {
            weight[var_of_op[k]] -= bits(h);
        }
        edges.extend(readers.iter().map(|&r| (r, m, 0)));
    }

    // Closure graph: node (x, k) for k in (lo_x, hi_x] means stage(x) ≥ k.
    let mut level_base = Vec::with_capacity(lo.len());
    let mut nodes = 0usize;
    for x in 0..lo.len() {
        level_base.push(nodes);
        nodes += hi[x] - lo[x];
    }
    let node = |x: usize, k: usize| level_base[x] + (k - lo[x] - 1);
    let (source, sink) = (nodes, nodes + 1);
    let positive: i64 = (0..lo.len())
        .map(|x| (-weight[x]).max(0) * (hi[x] - lo[x]) as i64)
        .sum();
    let infinite = positive + 1;
    let mut flow = MaxFlow::new(nodes + 2);
    for x in 0..lo.len() {
        for k in lo[x] + 1..=hi[x] {
            let gain = -weight[x];
            if gain > 0 {
                flow.add(source, node(x, k), gain);
            } else if gain < 0 {
                flow.add(node(x, k), sink, -gain);
            }
            if k > lo[x] + 1 {
                flow.add(node(x, k), node(x, k - 1), infinite);
            }
        }
    }
    for &(x, y, w) in &edges {
        for k in lo[x] + 1..=hi[x] {
            let j = k + w;
            if j <= lo[y] {
                continue;
            }
            assert!(j <= hi[y], "stage bounds admit a constraint violation: {x} → {y} (+{w}) at level {k}");
            flow.add(node(x, k), node(y, j), infinite);
        }
    }
    flow.max_flow(source, sink);
    let chosen = flow.reachable(source);
    let mut computed_stage = vec![0usize; n];
    for i in 0..n {
        let x = var_of_op[i];
        if x != usize::MAX {
            computed_stage[i] = lo[x] + (lo[x] + 1..=hi[x]).filter(|&k| chosen[node(x, k)]).count();
        }
    }
    p.schedule(ops, result, budget, &computed_stage)
}

/// Dinic's maximum flow over integer capacities, with the residual reachability that gives the minimum cut.
struct MaxFlow {
    head: Vec<usize>,
    next: Vec<usize>,
    to: Vec<usize>,
    cap: Vec<i64>,
}

impl MaxFlow {
    const NONE: usize = usize::MAX;

    fn new(n: usize) -> MaxFlow {
        MaxFlow {
            head: vec![Self::NONE; n],
            next: Vec::new(),
            to: Vec::new(),
            cap: Vec::new(),
        }
    }

    fn add(&mut self, u: usize, v: usize, c: i64) {
        for (a, b, c) in [(u, v, c), (v, u, 0)] {
            self.next.push(self.head[a]);
            self.to.push(b);
            self.cap.push(c);
            self.head[a] = self.to.len() - 1;
        }
    }

    fn levels(&self, s: usize) -> Vec<usize> {
        let mut level = vec![usize::MAX; self.head.len()];
        let mut queue = std::collections::VecDeque::from([s]);
        level[s] = 0;
        while let Some(u) = queue.pop_front() {
            let mut e = self.head[u];
            while e != Self::NONE {
                if self.cap[e] > 0 && level[self.to[e]] == usize::MAX {
                    level[self.to[e]] = level[u] + 1;
                    queue.push_back(self.to[e]);
                }
                e = self.next[e];
            }
        }
        level
    }

    fn max_flow(&mut self, s: usize, t: usize) -> i64 {
        let mut total = 0;
        loop {
            let level = self.levels(s);
            if level[t] == usize::MAX {
                return total;
            }
            let mut cursor = self.head.clone();
            loop {
                let pushed = self.augment(s, t, i64::MAX, &level, &mut cursor);
                if pushed == 0 {
                    break;
                }
                total += pushed;
            }
        }
    }

    /// One blocking-flow augmenting path, found iteratively so deep level graphs never exhaust the stack.
    fn augment(&mut self, s: usize, t: usize, limit: i64, level: &[usize], cursor: &mut [usize]) -> i64 {
        let mut path: Vec<usize> = Vec::new();
        let mut u = s;
        loop {
            if u == t {
                let pushed = path.iter().map(|&e| self.cap[e]).fold(limit, i64::min);
                for &e in &path {
                    self.cap[e] -= pushed;
                    self.cap[e ^ 1] += pushed;
                }
                return pushed;
            }
            let mut advanced = false;
            while cursor[u] != Self::NONE {
                let e = cursor[u];
                let v = self.to[e];
                if self.cap[e] > 0 && level[v] == level[u] + 1 {
                    path.push(e);
                    u = v;
                    advanced = true;
                    break;
                }
                cursor[u] = self.next[e];
            }
            if !advanced {
                if u == s {
                    return 0;
                }
                let e = path.pop().expect("a non-source node is entered by an edge");
                let back = self.to[e ^ 1];
                cursor[back] = self.next[cursor[back]];
                u = back;
            }
        }
    }

    fn reachable(&self, s: usize) -> Vec<bool> {
        let mut seen = vec![false; self.head.len()];
        let mut stack = vec![s];
        seen[s] = true;
        while let Some(u) = stack.pop() {
            let mut e = self.head[u];
            while e != Self::NONE {
                if self.cap[e] > 0 && !seen[self.to[e]] {
                    seen[self.to[e]] = true;
                    stack.push(self.to[e]);
                }
                e = self.next[e];
            }
        }
        seen
    }
}

/// What a reader at some stage reads: an input or a computed value, possibly through a chain of delay registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Held {
    Input(usize),
    Value(usize),
}

/// The registers a schedule needs: for each input and computed value, how many stages past its own its last reader
/// sits, through wiring. Also which ops the result depends on.
struct Registers {
    depth: std::collections::BTreeMap<Held, usize>,
    needed: Vec<bool>,
}

fn registers(ops: &[FabricOp], sched: &PipelineSchedule) -> Registers {
    let n = ops.len();
    let needed = needed_by(ops, sched.result);
    let mut depth = std::collections::BTreeMap::new();
    fn want(
        ops: &[FabricOp],
        stage: &[usize],
        s: Src,
        at: usize,
        depth: &mut std::collections::BTreeMap<Held, usize>,
    ) {
        let (held, born) = match s {
            Src::Const(_) => return,
            Src::Val(k) if is_wiring(&ops[k]) => {
                want(ops, stage, ops[k].a, at, depth);
                want(ops, stage, ops[k].b, at, depth);
                return;
            }
            Src::Val(k) => (Held::Value(k), stage[k]),
            Src::In(k) => (Held::Input(k), 0),
        };
        let d = depth.entry(held).or_insert(0);
        *d = (*d).max(at - born);
    }
    for i in 0..n {
        if needed[i] && !is_wiring(&ops[i]) {
            want(ops, &sched.stage, ops[i].a, sched.stage[i], &mut depth);
            want(ops, &sched.stage, ops[i].b, sched.stage[i], &mut depth);
        }
    }
    want(
        ops,
        &sched.stage,
        Src::Val(sched.result),
        sched.stage[sched.result],
        &mut depth,
    );
    Registers { depth, needed }
}

impl PipelineSchedule {
    /// The live bits the emitted pipeline's registers hold: each input register, each value's live bits once per
    /// stage it is delayed across, and the result register's live bits.
    pub fn register_bits(&self, ops: &[FabricOp]) -> u64 {
        let known = known_bits(ops);
        let held: u64 = registers(ops, self)
            .depth
            .iter()
            .map(|(&h, &d)| match h {
                Held::Input(_) => 64 * (d as u64 + u64::from(self.inputs == Inputs::Registered)),
                Held::Value(k) => live_bits(known[k]) * d as u64,
            })
            .sum();
        held + live_bits(known[self.result])
    }

    /// The 64-bit registers the emitted pipeline holds: one per input when it registers them, one per stage each value
    /// is delayed across, and the result register.
    pub fn register_words(&self, ops: &[FabricOp]) -> usize {
        let regs = registers(ops, self);
        let inputs = match self.inputs {
            Inputs::Registered => regs.depth.keys().filter(|h| matches!(h, Held::Input(_))).count(),
            Inputs::Ports => 0,
        };
        inputs + regs.depth.values().sum::<usize>() + 1
    }
}

fn held_name(h: Held, inputs: Inputs) -> String {
    match (h, inputs) {
        (Held::Input(k), Inputs::Registered) => format!("in{k}_r"),
        (Held::Input(k), Inputs::Ports) => format!("in{k}"),
        (Held::Value(k), _) => format!("v{k}"),
    }
}

fn delayed_name(h: Held, d: usize, inputs: Inputs) -> String {
    if d == 0 {
        held_name(h, inputs)
    } else {
        format!("{}_d{d}", held_name(h, inputs))
    }
}

fn const_verilog(c: i64) -> String {
    format!("64'h{:016x}", c as u64)
}

struct Emission<'a> {
    ops: &'a [FabricOp],
    sched: &'a PipelineSchedule,
    rebuilt: std::collections::BTreeSet<(usize, usize)>,
    wires: String,
}

impl Emission<'_> {
    /// The Verilog name a reader in stage `at` uses for `s`, emitting the rebuilt wiring it goes through first.
    fn operand(&mut self, s: Src, at: usize) -> String {
        match s {
            Src::Const(c) => const_verilog(c),
            Src::In(k) => delayed_name(Held::Input(k), at, self.sched.inputs),
            Src::Val(k) if is_wiring(&self.ops[k]) => {
                if self.rebuilt.insert((k, at)) {
                    let op = self.ops[k];
                    let a = self.operand(op.a, at);
                    let b = self.operand(op.b, at);
                    self.wires.push_str(&format!(
                        "  wire [63:0] w{k}_s{at} = {};\n",
                        op.op.verilog(&a, &b)
                    ));
                }
                format!("w{k}_s{at}")
            }
            Src::Val(k) => delayed_name(Held::Value(k), at - self.sched.stage[k], self.sched.inputs),
        }
    }
}

/// Emit the scheduled residual as a pipeline: module `fabric_{name}` with ports `clk`, `in0..`, and the registered
/// `result`. Every assignment is its own net, so no operator's signedness leaks into another's.
pub fn emit_fabric_pipelined_verilog(
    name: &str,
    n_in: usize,
    ops: &[FabricOp],
    sched: &PipelineSchedule,
) -> String {
    sched
        .check(ops)
        .unwrap_or_else(|e| panic!("emitting an invalid schedule: {e}"));
    let regs = registers(ops, sched);
    let mut decls = String::new();
    let mut updates = String::new();
    let mode = sched.inputs;
    for (&held, &depth) in &regs.depth {
        if let (Held::Input(k), Inputs::Registered) = (held, mode) {
            decls.push_str(&format!("  reg [63:0] in{k}_r;\n"));
            updates.push_str(&format!("    in{k}_r <= in{k};\n"));
        }
        for d in 1..=depth {
            decls.push_str(&format!("  reg [63:0] {};\n", delayed_name(held, d, mode)));
            updates.push_str(&format!(
                "    {} <= {};\n",
                delayed_name(held, d, mode),
                delayed_name(held, d - 1, mode)
            ));
        }
    }
    let mut em = Emission {
        ops,
        sched,
        rebuilt: Default::default(),
        wires: String::new(),
    };
    for (i, op) in ops.iter().enumerate() {
        if !regs.needed[i] || is_wiring(op) {
            continue;
        }
        let at = sched.stage[i];
        let a = em.operand(op.a, at);
        let b = em.operand(op.b, at);
        em.wires.push_str(&format!(
            "  wire [63:0] v{i} = {};\n",
            op.op.verilog(&a, &b)
        ));
    }
    let out = em.operand(Src::Val(sched.result), sched.stage[sched.result]);
    updates.push_str(&format!("    result <= {out};\n"));
    let inputs = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let reads = match sched.inputs {
        Inputs::Registered => "",
        Inputs::Ports => "\n// Its first stage reads the ports: the registers that drive them hold its inputs, so none are registered twice.",
    };
    format!(
        "// fabric_{name} — a compiled dispatch-free residual as a {stages}-stage pipeline: a new input every clock,\n\
         // its result {latency} clocks later. Lowered from the REAL compiler's residual; multiplies and sums are\n\
         // carry-save rows reduced by 3:2 compressor trees and resolved by Kogge-Stone adds, cut into stages of at\n\
         // most {budget} XOR-depth units. 64-bit, so it wraps identically to the VM's Int; proven == the running VM by\n\
         // iverilog, streaming.{reads}\n\
         module fabric_{name}(input clk, {inputs}, output reg [63:0] result);\n\
         {decls}{wires}  always @(posedge clk) begin\n\
         {updates}  end\n\
         endmodule\n",
        stages = sched.stages(),
        latency = sched.latency(),
        budget = sched.budget,
        wires = em.wires,
    )
}

/// The cycle model of the emitted pipeline. Before each clock `t` the input vector `stream[t]` is presented; the
/// returned vector holds the result register after each clock. Registers start at zero.
pub fn simulate_pipeline(
    ops: &[FabricOp],
    sched: &PipelineSchedule,
    n_in: usize,
    stream: &[Vec<i64>],
) -> Vec<i64> {
    sched
        .check(ops)
        .unwrap_or_else(|e| panic!("simulating an invalid schedule: {e}"));
    let regs = registers(ops, sched);
    let mut held: std::collections::BTreeMap<(Held, usize), i64> = regs
        .depth
        .iter()
        .flat_map(|(&h, &depth)| {
            let first = if matches!(h, Held::Input(_)) && sched.inputs == Inputs::Registered { 0 } else { 1 };
            (first..=depth).map(move |d| ((h, d), 0i64))
        })
        .collect();
    let mut result_reg = 0i64;
    let mut out = Vec::with_capacity(stream.len());
    for inputs in stream {
        assert_eq!(inputs.len(), n_in, "every clock presents all {n_in} inputs");
        let mut comb = vec![0i64; ops.len()];
        let ports = (sched.inputs == Inputs::Ports).then_some(inputs.as_slice());
        #[allow(clippy::too_many_arguments)]
        fn read(
            ops: &[FabricOp],
            stage: &[usize],
            comb: &[i64],
            held: &std::collections::BTreeMap<(Held, usize), i64>,
            ports: Option<&[i64]>,
            s: Src,
            at: usize,
        ) -> i64 {
            match s {
                Src::Const(c) => c,
                Src::In(k) => match (ports, at) {
                    (Some(now), 0) => now[k],
                    _ => held[&(Held::Input(k), at)],
                },
                Src::Val(k) if is_wiring(&ops[k]) => {
                    let op = ops[k];
                    op.op.eval(
                        read(ops, stage, comb, held, ports, op.a, at),
                        read(ops, stage, comb, held, ports, op.b, at),
                    )
                }
                Src::Val(k) if at == stage[k] => comb[k],
                Src::Val(k) => held[&(Held::Value(k), at - stage[k])],
            }
        }
        for (i, op) in ops.iter().enumerate() {
            if regs.needed[i] && !is_wiring(op) {
                let at = sched.stage[i];
                comb[i] = op.op.eval(
                    read(ops, &sched.stage, &comb, &held, ports, op.a, at),
                    read(ops, &sched.stage, &comb, &held, ports, op.b, at),
                );
            }
        }
        let next_result = read(
            ops,
            &sched.stage,
            &comb,
            &held,
            ports,
            Src::Val(sched.result),
            sched.stage[sched.result],
        );
        let mut next = held.clone();
        for (&(h, d), v) in next.iter_mut() {
            *v = match (h, d) {
                (Held::Input(k), 0) => inputs[k],
                (Held::Input(k), 1) if sched.inputs == Inputs::Ports => inputs[k],
                (Held::Value(k), 1) => comb[k],
                (h, d) => held[&(h, d - 1)],
            };
        }
        held = next;
        result_reg = next_result;
        out.push(result_reg);
    }
    out
}

/// The stage budget `fabric_horner_seq` is cut to, in the units of [`op_delay`]. MEASURED (2026-10-17, the
/// synthesis sequence of OpenLane's synth.tcl under host yosys 0.33, AREA 0, abc target 1 ns, sky130_fd_sc_hd
/// tt_025C_1v80, worst stage by abc `stime`): budget 4 is 1.36 ns over 311,772 µm² of cells, 5 is 1.43 ns over
/// 258,289 µm², 6 is 1.99 ns, 7 is 2.65 ns; the single-stage residual is 13.85 ns. Five is the knee: the next
/// tighter budget buys 5% of period for 21% more area.
pub const HORNER_SEQ_BUDGET: f64 = 5.0;

/// The residual `fabric_horner_seq` hardens: the REAL compiler's residual of the Horner polynomial
/// ([`POLY_BODY`](super::fabric_rtl::POLY_BODY)), carry-save lowered at 64 bits and cut to [`HORNER_SEQ_BUDGET`].
pub fn horner_seq_pipeline() -> (Vec<FabricOp>, PipelineSchedule) {
    let (ops, result, _) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    let (lowered, result) = carry_save_lower(&ops, result, 64);
    let sched = schedule_pipeline(&lowered, result, HORNER_SEQ_BUDGET);
    (lowered, sched)
}

/// `benchmarks/silicon/fabric_horner_seq.v`: the RTL the flow hardens as `fabric_horner_seq`.
pub fn emit_horner_seq_verilog() -> String {
    let (ops, sched) = horner_seq_pipeline();
    emit_fabric_pipelined_verilog("horner_seq", 1, &ops, &sched)
}

/// The `u`-wide unrolled int8 dot product in LOGOS: each iteration accumulates `u` independent products, so the
/// compiler places `u` multiply tiles that fire together, `u` MACs per clock.
pub fn dot_unrolled_source(u: usize) -> String {
    let terms: Vec<String> = (0..u)
        .map(|j| {
            if j == 0 {
                "k * (n - k)".into()
            } else {
                format!("(k + {j}) * (n - k - {j})")
            }
        })
        .collect();
    format!(
        "## To dotU (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + {}.\n    Return s.\n",
        terms.join(" + ")
    )
}

/// A pin-light top for a MAC array's CGRA `cgra_{name}`, so placement and routing measure the array's paths rather
/// than its I/O: a serial input shifts into the loop bound, the finished result latches, its parity drives one pin.
pub fn matmul_array_top(name: &str) -> String {
    format!(
        "module matmul_array_top(input clk, input rst, input serin, output reg serout);\n\
        \x20 reg [63:0] in0 = 64'd0;\n  wire [63:0] result; wire done;\n  reg [63:0] acc = 64'd0;\n\
        \x20 cgra_{name} dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));\n\
        \x20 always @(posedge clk) begin\n    in0 <= {{in0[62:0], serin}};\n    if (done) acc <= result;\n    serout <= ^acc;\n  end\nendmodule\n"
    )
}

/// The int8 width of the GEMM array's multiply tiles.
pub const GEMM_MUL_BITS: u32 = 8;

/// The term stage budget of the GEMM array, in the units of `loop_cell_delay`: the operand arithmetic in one stage,
/// the int8 multiplies in the next.
pub const GEMM_TERM_BUDGET: f64 = 2.0;

/// `benchmarks/silicon/gemm_mac_array.v`: the eight-wide int8 MAC array `cgra_arr8` as the pipelined loop CGRA
/// (operands, then products, ahead of a carry-save accumulator), with its pin-light top.
pub fn gemm_mac_array_verilog() -> String {
    let lf = loop_fabric_from_source(&dot_unrolled_source(8), "dotU");
    let pipelined = emit_loop_fabric_verilog_pipelined("arr8", &lf, Some(GEMM_MUL_BITS), GEMM_TERM_BUDGET)
        .unwrap_or_else(|why| panic!("the dot-product loop takes the pipelined form: {why}"));
    format!("{}\n{}", pipelined.verilog, matmul_array_top("arr8"))
}

/// A parameterized Verilog module `{module}`: an unsigned AW×BW multiplier as a tree of 3:2 compressors over its
/// partial-product rows, registered after CUT tree levels and (with FINAL) once more before a Kogge-Stone final add.
/// One generator for every consumer, so the BigInt limb multiplier and the FPU carry the same tree under their own
/// names.
pub fn emit_csa_multiplier_verilog(module: &str) -> String {
    format!(
r#"// {module} — an unsigned AW×BW multiplier as a tree of 3:2 compressors over its BW partial-product rows. The
// compressor outputs register after CUT levels (CUT = 0 or beyond the tree's depth: no register inside it), the last
// two words register again when FINAL is set, and those two words resolve through a Kogge-Stone add. `p` is the
// product of the operands sampled as many clocks earlier as there are registers on its path.
module {module} #(parameter integer AW = 53, parameter integer BW = 53, parameter integer CUT = 4,
                 parameter integer FINAL = 1) (
  input                clk,
  input  [AW-1:0]      a,
  input  [BW-1:0]      b,
  output [AW+BW-1:0]   p
);
  localparam integer W = AW + BW;

  // The words entering tree level `level`: each level turns every three words into two and passes the rest.
  function integer words(input integer level);
    integer n, l;
    begin
      n = BW;
      for (l = 0; l < level; l = l + 1) n = 2 * (n / 3) + (n % 3);
      words = n;
    end
  endfunction

  function integer depth(input integer unused);
    integer n, l;
    begin
      n = BW; l = 0;
      while (n > 2) begin n = 2 * (n / 3) + (n % 3); l = l + 1; end
      depth = l;
    end
  endfunction

  localparam integer L = depth(0);

  // Level-major bus: level `l` holds its words at word slots l*BW .. l*BW + words(l) - 1.
  wire [W*BW*(L+1)-1:0] lv;

  genvar i, l, g;
  generate
    for (i = 0; i < BW; i = i + 1) begin : row
      assign lv[i*W +: W] = b[i] ? ({{{{BW{{1'b0}}}}, a}} << i) : {{W{{1'b0}}}};
    end
    for (l = 0; l < L; l = l + 1) begin : level
      localparam integer N = words(l);
      localparam integer G = N / 3;
      for (g = 0; g < G; g = g + 1) begin : compress
        wire [W-1:0] x = lv[((l*BW) + 3*g) * W +: W];
        wire [W-1:0] y = lv[((l*BW) + 3*g + 1) * W +: W];
        wire [W-1:0] z = lv[((l*BW) + 3*g + 2) * W +: W];
        wire [W-1:0] xy = x ^ y;
        wire [W-1:0] s = xy ^ z;
        wire [W-1:0] c = ((x & y) | (z & xy)) << 1;
        if (l + 1 == CUT) begin : registered
          reg [W-1:0] sr, cr;
          always @(posedge clk) begin sr <= s; cr <= c; end
          assign lv[(((l+1)*BW) + 2*g) * W +: W] = sr;
          assign lv[(((l+1)*BW) + 2*g + 1) * W +: W] = cr;
        end else begin : wired
          assign lv[(((l+1)*BW) + 2*g) * W +: W] = s;
          assign lv[(((l+1)*BW) + 2*g + 1) * W +: W] = c;
        end
      end
      for (g = 0; g < N % 3; g = g + 1) begin : carry_over
        if (l + 1 == CUT) begin : registered
          reg [W-1:0] r;
          always @(posedge clk) r <= lv[((l*BW) + 3*G + g) * W +: W];
          assign lv[(((l+1)*BW) + 2*G + g) * W +: W] = r;
        end else begin : wired
          assign lv[(((l+1)*BW) + 2*G + g) * W +: W] = lv[((l*BW) + 3*G + g) * W +: W];
        end
      end
    end
  endgenerate

  wire [W-1:0] s_word = lv[(L*BW) * W +: W];
  wire [W-1:0] c_word;
  generate
    if (BW == 1) begin : one_row
      assign c_word = {{W{{1'b0}}}};
    end else begin : two_rows
      assign c_word = lv[(L*BW + 1) * W +: W];
    end
  endgenerate
  reg  [W-1:0] s_r, c_r;
  always @(posedge clk) begin s_r <= s_word; c_r <= c_word; end
  wire [W-1:0] s_in = FINAL ? s_r : s_word;
  wire [W-1:0] c_in = FINAL ? c_r : c_word;

  // The Kogge-Stone add of the last two words: group generate and propagate doubled in span each level.
  function [W-1:0] prefix_add(input [W-1:0] u, input [W-1:0] v);
    reg [W-1:0] gg, pp, p0; integer d;
    begin
      gg = u & v; p0 = u ^ v; pp = p0;
      for (d = 1; d < W; d = d * 2) begin
        gg = gg | (pp & (gg << d));
        pp = pp & (pp << d);
      end
      prefix_add = p0 ^ (gg << 1);
    end
  endfunction

  assign p = prefix_add(s_in, c_in);
endmodule
"#
    )
}
