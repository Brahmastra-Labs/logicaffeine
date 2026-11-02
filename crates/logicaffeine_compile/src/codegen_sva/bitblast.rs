//! Bit-blasting: `BoundedExpr` datapath (bitvector) operations → boolean `ProofExpr`.
//!
//! A bitvector is represented LSB-first as a `Vec<Bit>`, each `Bit` either a known constant
//! or a boolean `ProofExpr` (a signal bit `name#i`, or a gate). Every combinator
//! constant-folds, so a constant operand collapses gates instead of emitting them — the CNF
//! stays small. Each operation is the textbook circuit: ripple-carry add/sub, array
//! multiplier, barrel shifters, magnitude/signed comparators, slice and concat.
//!
//! This is the data-path half of the certified-prover seam: combined with the Boolean
//! lowering in [`super::sva_to_proof`], multi-bit hardware obligations reduce to a
//! propositional formula our CDCL→RUP tiers discharge — in the browser, no Z3.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::sva_to_verify::{BitVecBoundedOp, BoundedExpr, CmpBoundedOp};
use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::ProofExpr;

/// One blasted bit: a known constant, or a node of the shared gate DAG.
///
/// The dynamic payload is an `Rc<Gate>`, NOT a `ProofExpr` — because a datapath is a **DAG**, not a
/// tree. An array multiplier reads every bit of its operand `w` times, and a multiply CHAIN feeds
/// that already-shared result into the next multiplier: with an owned `ProofExpr` per bit, `clone()`
/// deep-copies the whole operand once per read and the blast is EXPONENTIAL in the chain length (a
/// six-step fast-doubling body reached 44 GB and was still climbing). Sharing the gate by pointer
/// makes `Bit::clone` O(1), so the blast is linear in the number of gate operations the circuit
/// performs — the DAG size, which [`BlastSize::dag_nodes`] reports.
///
/// The tree is materialised exactly ONCE, at the boundary, by [`bit_to_proof`], and the gate
/// variants are a transliteration of the `ProofExpr` shapes the tree-shaped blaster used to emit —
/// so the materialised formula is BYTE-IDENTICAL to the one it produced. Sharing is a performance
/// property here, never a semantic one.
#[derive(Clone)]
enum Bit {
    Const(bool),
    Dyn(Rc<Gate>),
}

/// A gate in the shared DAG. One variant per `ProofExpr` shape [`bit_to_proof`] emits.
enum Gate {
    Atom(String),
    Not(Bit),
    And(Bit, Bit),
    Or(Bit, Bit),
    Iff(Bit, Bit),
}

/// Tearing down the DAG is ITERATIVE, for the same reason every walk over it is: the DAG's depth
/// is the datapath's, and the derived `Drop` recurses once per level. A chained multiplier is tens
/// of thousands of gate levels deep, so letting the compiler write this would turn a successful
/// certificate into a stack overflow the moment the circuit went out of scope — a crash with no
/// bad input and no failing assertion to point at.
///
/// Each gate's children are MOVED onto an explicit worklist and replaced by leaves, so the gate
/// that is actually dropped owns nothing that can recurse. Only the LAST owner of a gate takes it
/// apart; a gate still shared by another root is left alone, exactly as the refcount says.
impl Drop for Gate {
    fn drop(&mut self) {
        let mut work: Vec<Bit> = Vec::new();
        take_children(self, &mut work);
        while let Some(b) = work.pop() {
            let Bit::Dyn(rc) = b else { continue };
            if let Some(mut g) = Rc::into_inner(rc) {
                take_children(&mut g, &mut work);
            }
        }
    }
}

/// Move a gate's DYNAMIC children onto `out`, leaving leaves behind. Constant children are left in
/// place — they own nothing, and pushing them would force the worklist to allocate for a gate that
/// cannot recurse anyway.
fn take_children(g: &mut Gate, out: &mut Vec<Bit>) {
    let take = |b: &mut Bit, out: &mut Vec<Bit>| {
        if matches!(b, Bit::Dyn(_)) {
            out.push(std::mem::replace(b, Bit::Const(false)));
        }
    };
    match g {
        Gate::Atom(_) => {}
        Gate::Not(x) => take(x, out),
        Gate::And(x, y) | Gate::Or(x, y) | Gate::Iff(x, y) => {
            take(x, out);
            take(y, out);
        }
    }
}

/// The reserved atom a folded constant is spelled over: `Or(c, ¬c)` is always true, `And(c, ¬c)`
/// always false. `ProofExpr` has no Boolean literal, so every materialisation of a constant — and
/// every clausification of one — goes through this one name.
const BIT_CONST_ATOM: &str = "__bit_const";

fn gate(g: Gate) -> Bit {
    count(|w| w.gates += 1);
    Bit::Dyn(Rc::new(g))
}

/// ★ THE BLASTER'S WORK, COUNTED — not timed. What a blast costs is three walks: the datapath expression it is handed
/// (every node blasted once — a node blasted twice doubles the work per level above it), the gates it allocates (no
/// gate is shared by value, so every allocation is a gate), and the memoised walk that prices the result (every gate
/// visited once per edge into it — a walk that forgot its memo walks the tree). Each is a count a lock can bound by the
/// size of what it walks, and none of them moves when the machine is busy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlastWork {
    /// Datapath expression nodes blasted.
    pub exprs: u64,
    /// Gates allocated.
    pub gates: u64,
    /// Steps of the memoised pricing walk.
    pub walked: u64,
}

thread_local! {
    static WORK: std::cell::Cell<BlastWork> = const { std::cell::Cell::new(BlastWork { exprs: 0, gates: 0, walked: 0 }) };
}

fn count(f: impl FnOnce(&mut BlastWork)) {
    WORK.with(|w| {
        let mut v = w.get();
        f(&mut v);
        w.set(v);
    });
}

/// Run `f` and return what it returns with the blaster work it did on this thread.
pub fn blast_work<T>(f: impl FnOnce() -> T) -> (T, BlastWork) {
    let before = WORK.with(|w| w.get());
    let out = f();
    let after = WORK.with(|w| w.get());
    (
        out,
        BlastWork {
            exprs: after.exprs - before.exprs,
            gates: after.gates - before.gates,
            walked: after.walked - before.walked,
        },
    )
}

fn b_atom(name: String) -> Bit {
    gate(Gate::Atom(name))
}

fn b_not(x: Bit) -> Bit {
    match x {
        Bit::Const(c) => Bit::Const(!c),
        e => gate(Gate::Not(e)),
    }
}

fn b_and(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(false), _) | (_, Bit::Const(false)) => Bit::Const(false),
        (Bit::Const(true), y) | (y, Bit::Const(true)) => y,
        (x, y) => gate(Gate::And(x, y)),
    }
}

fn b_or(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(true), _) | (_, Bit::Const(true)) => Bit::Const(true),
        (Bit::Const(false), y) | (y, Bit::Const(false)) => y,
        (x, y) => gate(Gate::Or(x, y)),
    }
}

/// XNOR / equality of two bits.
fn b_iff(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(c), Bit::Const(d)) => Bit::Const(c == d),
        (Bit::Const(true), y) | (y, Bit::Const(true)) => y,
        (Bit::Const(false), y) | (y, Bit::Const(false)) => b_not(y),
        (x, y) => gate(Gate::Iff(x, y)),
    }
}

fn b_xor(a: Bit, b: Bit) -> Bit {
    b_not(b_iff(a, b))
}

/// If-then-else (a 1-bit mux): `s ? t : e`.
fn b_ite(s: Bit, t: Bit, e: Bit) -> Bit {
    match s {
        Bit::Const(true) => t,
        Bit::Const(false) => e,
        s => b_or(b_and(s.clone(), t), b_and(b_not(s), e)),
    }
}

/// Force a bit into the DAG as a GATE, materialising a folded constant as the reserved-atom
/// literal (`c ∨ ¬c` / `c ∧ ¬c`). The predicate of a `BitVecIte` goes through here so a
/// constant-foldable predicate still emits the full mux — the formula the tree-shaped blaster
/// emitted, unchanged. Sharing is a performance property of this module, never a semantic one.
fn as_gate(b: Bit) -> Bit {
    match b {
        Bit::Dyn(_) => b,
        Bit::Const(v) => {
            let c = b_atom(BIT_CONST_ATOM.to_string());
            let nc = b_not(c.clone());
            if v {
                gate(Gate::Or(c, nc))
            } else {
                gate(Gate::And(c, nc))
            }
        }
    }
}

// ── Pricing the materialisation ─────────────────────────────────────────────────────────

/// The cost of turning one blasted obligation into a `ProofExpr`.
///
/// `dag_nodes` is what the blaster ALLOCATES (gates, shared); `tree_nodes` is what materialising it
/// into the `Box`-tree `ProofExpr` WOULD allocate — the two diverge exponentially on a multiply
/// chain, which is exactly the blow-up [`BLAST_BUDGET_NODES`] refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlastSize {
    /// Distinct gates in the shared DAG — the blaster's real allocation.
    pub dag_nodes: u64,
    /// Nodes the materialised `ProofExpr` tree would hold (saturating at `u128::MAX`).
    pub tree_nodes: u128,
}

impl BlastSize {
    /// The tree count hit the saturation ceiling: the real tree is at LEAST `tree_nodes` big.
    pub fn saturated(&self) -> bool {
        self.tree_nodes == u128::MAX
    }

    /// Would materialising this obligation exceed the production certificate budget?
    pub fn over_budget(&self) -> bool {
        self.tree_nodes > BLAST_BUDGET_NODES
    }
}

/// How many `ProofExpr` nodes we are willing to materialise for ONE bit-blasted obligation.
///
/// A `ProofExpr` node costs ~100 bytes once its `Box` is counted, so this ceiling is ~200 MB of
/// formula. It is calibrated from the measured blast of the RDU demo corpus's ALU bodies (the
/// `Eq(placed grid, body datapath)` obligation at the certified 8-bit bound):
///
/// | obligation                  | gates in the DAG | nodes in the tree |
/// |-----------------------------|-----------------:|------------------:|
/// | 8-bit MAC PE (`MatmulTiles`)|              851 |           107 247 |
/// | `mix` (2 inputs, 4 cells)   |            1 041 |            10 631 |
/// | fib, 1 doubling block       |               75 |                75 |
/// | fib, 2 doubling blocks      |            1 235 |            65 363 |
/// | fib, 3 doubling blocks      |           13 591 |       354 787 747 |
/// | fib, 4 doubling blocks      |          122 515 |    50 957 299 143 |
/// | fib, 6 — the demo `fib.lg`  |        9 350 207 | 102 116 468 174 507 |
///
/// ★ AND THE RECONFIG OBLIGATION, WHICH THE FIRST CALIBRATION MISSED:
///
/// | obligation                                      | gates in the DAG | nodes in the tree |
/// |-------------------------------------------------|-----------------:|------------------:|
/// | `certify_config_table`, 3 islands at width 8     |            8 600 |           173 430 |
/// | `certify_config_table`, 5 islands at width 8     |          167 366 |         2 767 042 |
///
/// That second row is why this constant is 8 000 000 and not 2 000 000. The ceiling was first chosen
/// from the ALU bodies ALONE, and `certify_config_table` blasts a much larger obligation on the same
/// ceiling — a time-mux of every island against one of them, growing SUPERLINEARLY in the island count
/// (three islands cost 173 k, five cost 2.77 M). At 2 000 000 a long-standing proof
/// (`fabric_whole_program_lock`) began failing, which is what a ceiling calibrated from one workload
/// does to another.
///
/// So the budget clears the widest obligation that certifies today (the 5-island reconfig, 2.77 M) by
/// ~2.9× and still refuses the third doubling block (3.5 × 10^8) by ~44×. There is deliberately no
/// ceiling that "just fits" a multiply chain: each doubling block multiplies the encoding by ~70, so
/// past the knee the only honest answer is a refusal BY NAME, and the band between a legitimate
/// obligation and a runaway is five orders of magnitude wide. The DAG is priced first
/// ([`BlastSize`]), so a refusal costs the DAG, never the tree. `bitblast_budget_lock` re-measures
/// every row of both tables, against the REAL corpus rather than a smaller stand-in.
pub const BLAST_BUDGET_NODES: u128 = 8_000_000;

/// Why a bit-blast produced no formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlastRefusal {
    /// The expression is not a supported bounded/bitvector obligation (wrong widths, an Int, a
    /// construct with no circuit).
    NotBounded,
    /// The circuit blasts, but its `ProofExpr` materialisation is bigger than `budget`.
    Budget { size: BlastSize, budget: u128 },
    /// The circuit itself — the SHARED gate DAG, not its unfolding — is bigger than `budget`.
    /// The ceiling that bounds a path which never materialises a tree.
    GateBudget { dag_nodes: u64, budget: u64 },
}

impl BlastRefusal {
    /// The prose a certificate carries when it declines this obligation — it names the cause AND
    /// the budget, so a reader can tell "we cannot express this" from "we will not pay for this".
    pub fn reason(&self) -> String {
        match self {
            BlastRefusal::NotBounded => {
                "the obligation escaped the propositional fragment".to_string()
            }
            BlastRefusal::Budget { size, budget } => format!(
                "the bit-blast of this ALU body exceeds the certificate budget ({}{} nodes > {} \
                 budget, from a {}-gate circuit) — a multiply chain blows up the propositional \
                 encoding",
                if size.saturated() { "≥" } else { "" },
                size.tree_nodes,
                budget,
                size.dag_nodes
            ),
            BlastRefusal::GateBudget { dag_nodes, budget } => format!(
                "the shared gate DAG of this ALU body exceeds the certificate budget \
                 ({dag_nodes} gates > {budget} budget) — the datapath is larger than one \
                 certificate is allowed to encode"
            ),
        }
    }
}

/// A gate's identity in the shared DAG — its `Rc` address. Two `Bit`s name the same gate exactly
/// when this matches, which is what every memoised walk below keys on.
fn gate_key(g: &Rc<Gate>) -> usize {
    Rc::as_ptr(g) as usize
}

/// The distinct gates reachable from `root`, in POST-ORDER: every gate appears after both of its
/// children and exactly once, in the same order a recursive descent of the materialised tree would
/// first reach them. That last clause is load-bearing — it is what makes a DAG-driven
/// clausification emit byte-identically to a tree-driven one.
///
/// Iterative, not recursive, because the DAG's depth is the datapath's: a chained 8-bit multiplier
/// is thousands of gate levels deep, and a recursive walk blows the thread stack long before the
/// circuit is large enough to matter. Every consumer of the DAG (pricing, clausification) folds
/// over this one order, so none of them can grow a stack bug the others do not have.
fn dag_postorder<'a>(roots: &[&'a Bit]) -> Vec<(usize, &'a Gate)> {
    let mut order: Vec<(usize, &Gate)> = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut stack: Vec<(&Bit, bool)> = roots.iter().rev().map(|b| (*b, false)).collect();
    while let Some((b, expanded)) = stack.pop() {
        count(|w| w.walked += 1);
        let Bit::Dyn(g) = b else { continue };
        let key = gate_key(g);
        if seen.contains(&key) {
            continue;
        }
        if !expanded {
            stack.push((b, true));
            // Pushed right-then-left so the LEFT child is popped first.
            match &**g {
                Gate::Atom(_) => {}
                Gate::Not(x) => stack.push((x, false)),
                Gate::And(x, y) | Gate::Or(x, y) | Gate::Iff(x, y) => {
                    stack.push((y, false));
                    stack.push((x, false));
                }
            }
            continue;
        }
        seen.insert(key);
        order.push((key, &**g));
    }
    order
}

/// Count the gates `roots` SHARE and the tree they would materialise into, visiting each shared
/// gate ONCE across all of them. Both counts come from one memoised walk, so pricing an obligation
/// costs the DAG, not the tree.
fn measure_all(roots: &[&Bit]) -> BlastSize {
    // A constant materialises as `Or(c, Not(c))` / `And(c, Not(c))`: 4 nodes.
    fn nodes_of(sizes: &HashMap<usize, u128>, b: &Bit) -> u128 {
        match b {
            Bit::Const(_) => 4,
            Bit::Dyn(g) => sizes.get(&gate_key(g)).copied().unwrap_or(4),
        }
    }
    let order = dag_postorder(roots);
    let mut sizes: HashMap<usize, u128> = HashMap::with_capacity(order.len());
    for (key, g) in &order {
        let n = match g {
            Gate::Atom(_) => 1,
            Gate::Not(x) => nodes_of(&sizes, x).saturating_add(1),
            Gate::And(x, y) | Gate::Or(x, y) | Gate::Iff(x, y) => nodes_of(&sizes, x)
                .saturating_add(nodes_of(&sizes, y))
                .saturating_add(1),
        };
        sizes.insert(*key, n);
    }
    let tree_nodes = roots
        .iter()
        .fold(0u128, |acc, r| acc.saturating_add(nodes_of(&sizes, r)));
    BlastSize {
        dag_nodes: order.len() as u64,
        tree_nodes,
    }
}

fn measure(b: &Bit) -> BlastSize {
    measure_all(&[b])
}

fn bit_to_proof(b: &Bit) -> ProofExpr {
    match b {
        // `ProofExpr` has no Boolean literal; encode constants as a tautology / contradiction
        // over a reserved atom (`Or(c,¬c)` is always true, `And(c,¬c)` always false).
        Bit::Const(true) => {
            let c = ProofExpr::Atom(BIT_CONST_ATOM.to_string());
            ProofExpr::Or(Box::new(c.clone()), Box::new(ProofExpr::Not(Box::new(c))))
        }
        Bit::Const(false) => {
            let c = ProofExpr::Atom(BIT_CONST_ATOM.to_string());
            ProofExpr::And(Box::new(c.clone()), Box::new(ProofExpr::Not(Box::new(c))))
        }
        Bit::Dyn(g) => match &**g {
            Gate::Atom(n) => ProofExpr::Atom(n.clone()),
            Gate::Not(x) => ProofExpr::Not(Box::new(bit_to_proof(x))),
            Gate::And(x, y) => ProofExpr::And(Box::new(bit_to_proof(x)), Box::new(bit_to_proof(y))),
            Gate::Or(x, y) => ProofExpr::Or(Box::new(bit_to_proof(x)), Box::new(bit_to_proof(y))),
            Gate::Iff(x, y) => ProofExpr::Iff(Box::new(bit_to_proof(x)), Box::new(bit_to_proof(y))),
        },
    }
}

// ── Bitvector circuits (operands LSB-first, equal width) ────────────────────────────────

/// Ripple-carry add: returns `(sum bits, carry-out)`.
fn ripple_add(a: &[Bit], b: &[Bit], carry_in: Bit) -> (Vec<Bit>, Bit) {
    let mut carry = carry_in;
    let mut out = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let axb = b_xor(a[i].clone(), b[i].clone());
        let sum = b_xor(axb.clone(), carry.clone());
        let carry_next = b_or(b_and(a[i].clone(), b[i].clone()), b_and(carry.clone(), axb));
        out.push(sum);
        carry = carry_next;
    }
    (out, carry)
}

fn bv_add(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    ripple_add(a, b, Bit::Const(false)).0
}

/// Two's-complement subtract: `a - b = a + ¬b + 1`.
fn bv_sub(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    let nb: Vec<Bit> = b.iter().cloned().map(b_not).collect();
    ripple_add(a, &nb, Bit::Const(true)).0
}

/// Shift-and-add array multiplier, truncated to the operand width.
fn bv_mul(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    let w = a.len();
    let mut acc: Vec<Bit> = vec![Bit::Const(false); w];
    for i in 0..w {
        // Partial product: (a << i) gated by b[i], truncated to width w.
        let mut pp: Vec<Bit> = vec![Bit::Const(false); w];
        for j in i..w {
            pp[j] = b_and(a[j - i].clone(), b[i].clone());
        }
        acc = bv_add(&acc, &pp);
    }
    acc
}

/// Shift left by a constant `k` (zero-fill), width-preserving.
fn shl_const(a: &[Bit], k: usize) -> Vec<Bit> {
    let w = a.len();
    (0..w)
        .map(|j| {
            if j >= k {
                a[j - k].clone()
            } else {
                Bit::Const(false)
            }
        })
        .collect()
}

/// Shift right by a constant `k`, filling the vacated top bits with `fill`.
fn shr_const(a: &[Bit], k: usize, fill: Bit) -> Vec<Bit> {
    let w = a.len();
    (0..w)
        .map(|j| {
            if j + k < w {
                a[j + k].clone()
            } else {
                fill.clone()
            }
        })
        .collect()
}

/// Barrel shifter for a variable amount: compose constant shifts of `1<<s` gated by the
/// amount's bit `s`. `fill` is the vacated-bit value (0 for logical, sign for arithmetic);
/// `left` selects direction.
fn barrel_shift(a: &[Bit], amount: &[Bit], left: bool, fill: Bit) -> Vec<Bit> {
    let w = a.len();
    let mut cur = a.to_vec();
    for (s, amt_bit) in amount.iter().enumerate() {
        let dist = 1usize << s;
        if dist >= w {
            // Any set bit at or beyond this position shifts everything out.
            let shifted = vec![fill.clone(); w];
            cur = (0..w)
                .map(|j| b_ite(amt_bit.clone(), shifted[j].clone(), cur[j].clone()))
                .collect();
            continue;
        }
        let shifted = if left {
            shl_const(&cur, dist)
        } else {
            shr_const(&cur, dist, fill.clone())
        };
        cur = (0..w)
            .map(|j| b_ite(amt_bit.clone(), shifted[j].clone(), cur[j].clone()))
            .collect();
    }
    cur
}

/// `a == b` over equal-width bitvectors → a single bit.
fn bv_eq(a: &[Bit], b: &[Bit]) -> Bit {
    let mut acc = Bit::Const(true);
    for i in 0..a.len() {
        acc = b_and(acc, b_iff(a[i].clone(), b[i].clone()));
    }
    acc
}

/// Unsigned `a < b` → a single bit (LSB→MSB magnitude comparator).
fn bv_ult(a: &[Bit], b: &[Bit]) -> Bit {
    let mut lt = Bit::Const(false);
    for i in 0..a.len() {
        // a<b on bits 0..=i  ≡  (a[i]<b[i]) ∨ (a[i]=b[i] ∧ a<b on bits below i)
        let bit_lt = b_and(b_not(a[i].clone()), b[i].clone());
        let bit_eq = b_iff(a[i].clone(), b[i].clone());
        lt = b_or(bit_lt, b_and(bit_eq, lt));
    }
    lt
}

/// Signed (two's-complement) `a < b` → a single bit.
fn bv_slt(a: &[Bit], b: &[Bit]) -> Bit {
    let w = a.len();
    let sa = a[w - 1].clone();
    let sb = b[w - 1].clone();
    // Differing signs: the negative one (sign bit 1) is smaller, so result = a's sign.
    // Same sign: the unsigned ordering coincides with the signed ordering.
    b_ite(b_xor(sa.clone(), sb), sa, bv_ult(a, b))
}

/// Two's-complement negate: `-x = ~x + 1`.
fn bv_neg(a: &[Bit]) -> Vec<Bit> {
    let notx: Vec<Bit> = a.iter().cloned().map(b_not).collect();
    let one: Vec<Bit> = (0..a.len()).map(|i| Bit::Const(i == 0)).collect();
    bv_add(&notx, &one)
}

/// UNSIGNED restoring division — `(quotient, remainder)`, LSB-first, `W` bits each. Standard
/// shift/compare/conditional-subtract: for each bit MSB→LSB bring it into a running remainder, and if
/// `rem ≥ b` subtract and set the quotient bit. Guarantees `q·b + r = a` and `r < b` (for `b ≠ 0`).
fn bv_udiv_urem(a: &[Bit], b: &[Bit]) -> (Vec<Bit>, Vec<Bit>) {
    let w = a.len();
    let mut q = vec![Bit::Const(false); w];
    let mut rem: Vec<Bit> = vec![Bit::Const(false); w]; // running remainder stays < b, fits in W bits
    for i in (0..w).rev() {
        // ext = (rem << 1) | a[i], as W+1 bits (LSB-first): [a[i], rem[0..w]]
        let mut ext: Vec<Bit> = Vec::with_capacity(w + 1);
        ext.push(a[i].clone());
        ext.extend(rem.iter().cloned());
        let mut bext: Vec<Bit> = b.to_vec();
        bext.push(Bit::Const(false)); // b zero-extended to W+1 bits
        let ge = b_not(bv_ult(&ext, &bext)); // ext >= b
        let sub = bv_sub(&ext, &bext);
        rem = (0..w)
            .map(|j| b_ite(ge.clone(), sub[j].clone(), ext[j].clone()))
            .collect();
        q[i] = ge;
    }
    (q, rem)
}

/// The our-SAT obligation that the SIGNED truncating divider (Rust/VM `/` and `%`) is CORRECT over all
/// `W`-bit inputs: for every `b ≠ 0` (excluding the single `MIN / −1` two's-complement overflow that
/// `/` itself rejects) the reconstruction `q·b + r == a` holds and the remainder is in range `|r| < |b|`.
/// `prove_unsat` REFUTES the returned expression iff the divider is correct. Pass `include_remainder =
/// false` to drop the `+ r` term — the obligation then becomes SATISFIABLE (a non-divisible input is a
/// counterexample), proving the check is not vacuous. Bit-level; needs no `BitVecBoundedOp` variant.
/// This is the `ProofExpr` tree of the obligation, which grows exponentially with `width` (the restoring
/// steps share their remainder bits, and the tree copies them); past W=3 refute
/// [`signed_divider_correct`] from the gate DAG instead.
pub(crate) fn signed_divider_obligation(width: usize, include_remainder: bool) -> ProofExpr {
    bit_to_proof(&signed_divider_bad(width, include_remainder))
}

/// The signed divider's correctness over all `W`-bit inputs as a gate-DAG predicate: it holds on every
/// input iff the divider is correct (the complement of [`signed_divider_obligation`]). `assert_not` it
/// into a `Cnf` and refute: the clauses are linear in the circuit, where the tree is exponential.
pub(crate) fn signed_divider_correct(width: usize, include_remainder: bool) -> Pred {
    Pred(b_not(signed_divider_bad(width, include_remainder)))
}

/// The input set on which the signed divider is wrong, as one gate of the shared DAG.
fn signed_divider_bad(width: usize, include_remainder: bool) -> Bit {
    let w = width;
    let a = blast_bits(&BoundedExpr::BitVecVar("a".into(), w as u32)).unwrap();
    let b = blast_bits(&BoundedExpr::BitVecVar("b".into(), w as u32)).unwrap();
    let sa = a[w - 1].clone();
    let sb = b[w - 1].clone();
    let abs = |x: &[Bit], s: &Bit| -> Vec<Bit> {
        let n = bv_neg(x);
        (0..w)
            .map(|j| b_ite(s.clone(), n[j].clone(), x[j].clone()))
            .collect()
    };
    let ua = abs(&a, &sa);
    let ub = abs(&b, &sb);
    let (uq, ur) = bv_udiv_urem(&ua, &ub);
    // Truncating-toward-zero signs: quotient negative iff signs differ; remainder sign = dividend sign.
    let qsign = b_xor(sa.clone(), sb.clone());
    let nq = bv_neg(&uq);
    let q: Vec<Bit> = (0..w)
        .map(|j| b_ite(qsign.clone(), nq[j].clone(), uq[j].clone()))
        .collect();
    let nr = bv_neg(&ur);
    let r: Vec<Bit> = (0..w)
        .map(|j| b_ite(sa.clone(), nr[j].clone(), ur[j].clone()))
        .collect();
    let recon = if include_remainder {
        bv_add(&bv_mul(&q, &b), &r)
    } else {
        bv_mul(&q, &b)
    };
    let ok = bv_eq(&recon, &a);
    let in_range = bv_ult(&ur, &ub); // |r| < |b|
    let bnz = b.iter().cloned().fold(Bit::Const(false), b_or); // b != 0
                                                               // overflow: a == MIN (100..0) and b == -1 (all ones) — excluded.
    let mut a_is_min = sa.clone();
    for j in 0..w - 1 {
        a_is_min = b_and(a_is_min, b_not(a[j].clone()));
    }
    let b_neg1 = b.iter().cloned().fold(Bit::Const(true), b_and);
    let overflow = b_and(a_is_min, b_neg1);
    let good = b_and(ok, in_range);
    b_and(b_and(bnz, b_not(overflow)), b_not(good))
}

/// SIGNED truncating div/mod (toward zero) as bits — `(q, r)` with `q·b + r = a`, `sign(r)=sign(a)`.
/// The shared core of the divider obligations (built on the unsigned restoring divider + sign fix-up).
fn signed_trunc_divmod(a: &[Bit], b: &[Bit]) -> (Vec<Bit>, Vec<Bit>) {
    let w = a.len();
    let sa = a[w - 1].clone();
    let sb = b[w - 1].clone();
    let abs = |x: &[Bit], s: &Bit| -> Vec<Bit> {
        let n = bv_neg(x);
        (0..w)
            .map(|j| b_ite(s.clone(), n[j].clone(), x[j].clone()))
            .collect()
    };
    let ua = abs(a, &sa);
    let ub = abs(b, &sb);
    let (uq, ur) = bv_udiv_urem(&ua, &ub);
    let qsign = b_xor(sa.clone(), sb.clone());
    let nq = bv_neg(&uq);
    let q: Vec<Bit> = (0..w)
        .map(|j| b_ite(qsign.clone(), nq[j].clone(), uq[j].clone()))
        .collect();
    let nr = bv_neg(&ur);
    let r: Vec<Bit> = (0..w)
        .map(|j| b_ite(sa.clone(), nr[j].clone(), ur[j].clone()))
        .collect();
    (q, r)
}

/// The our-SAT obligation that FLOOR division (`FloorDiv` — round toward −∞) is CORRECT: for every
/// `b ≠ 0` (MIN/−1 excluded) the floor quotient `fq = trunc − [rem≠0 ∧ signs differ]` and floor
/// remainder `fr = trunc_rem + [correction]·b` satisfy `fq·b + fr == a`, the remainder takes the
/// DIVISOR's sign (or is 0), and `|fr| < |b|` — the defining properties of floored division. `fq`/`fr`
/// are derived from the trunc divider's INDEPENDENT remainder (not from `a`), so the reconstruction is
/// a real check. `include_remainder = false` drops `+fr` → SATISFIABLE, proving non-vacuity.
pub(crate) fn floor_divider_obligation(width: usize, include_remainder: bool) -> ProofExpr {
    let w = width;
    let a = blast_bits(&BoundedExpr::BitVecVar("a".into(), w as u32)).unwrap();
    let b = blast_bits(&BoundedExpr::BitVecVar("b".into(), w as u32)).unwrap();
    let sa = a[w - 1].clone();
    let sb = b[w - 1].clone();
    let (tq, tr) = signed_trunc_divmod(&a, &b);
    // Correction: floor differs from trunc by 1 when the remainder is nonzero and the signs differ.
    let tr_nz = tr.iter().cloned().fold(Bit::Const(false), b_or);
    let signs_differ = b_xor(sa.clone(), sb.clone());
    let corr = b_and(tr_nz, signs_differ);
    let one: Vec<Bit> = (0..w).map(|i| Bit::Const(i == 0)).collect();
    let tq_m1 = bv_sub(&tq, &one);
    let fq: Vec<Bit> = (0..w)
        .map(|j| b_ite(corr.clone(), tq_m1[j].clone(), tq[j].clone()))
        .collect();
    // Floor remainder from the INDEPENDENT trunc remainder: fr = tr + (corr ? b : 0).
    let corr_b: Vec<Bit> = (0..w)
        .map(|j| b_ite(corr.clone(), b[j].clone(), Bit::Const(false)))
        .collect();
    let fr = bv_add(&tr, &corr_b);
    let recon = if include_remainder {
        bv_add(&bv_mul(&fq, &b), &fr)
    } else {
        bv_mul(&fq, &b)
    };
    let ok = bv_eq(&recon, &a);
    // Floor remainder takes the divisor's sign (or is 0), and |fr| < |b|.
    let fr_nz = fr.iter().cloned().fold(Bit::Const(false), b_or);
    let sign_ok = b_or(b_not(fr_nz), b_iff(fr[w - 1].clone(), sb.clone()));
    let abs = |x: &[Bit], s: &Bit| -> Vec<Bit> {
        let n = bv_neg(x);
        (0..w)
            .map(|j| b_ite(s.clone(), n[j].clone(), x[j].clone()))
            .collect()
    };
    let afr = abs(&fr, &fr[w - 1].clone());
    let ab = abs(&b, &sb);
    let range = bv_ult(&afr, &ab);
    let bnz = b.iter().cloned().fold(Bit::Const(false), b_or);
    let mut a_is_min = sa.clone();
    for j in 0..w - 1 {
        a_is_min = b_and(a_is_min, b_not(a[j].clone()));
    }
    let b_neg1 = b.iter().cloned().fold(Bit::Const(true), b_and);
    let overflow = b_and(a_is_min, b_neg1);
    let good = b_and(b_and(ok, sign_ok), range);
    let bad = b_and(b_and(bnz, b_not(overflow)), b_not(good));
    bit_to_proof(&bad)
}

/// The our-SAT obligation that the SHIFT-based `DivPow2` (signed `a / 2^k`, round toward zero via a
/// negative-bias then arithmetic shift) equals the reference signed truncating divider by `2^k`, for
/// every `k` in `1..W` over all W-bit `a`. Proves the cheap shift form is EXACTLY `Div` by a power of
/// two. `prove_unsat` REFUTES the returned expression iff the fast form is correct. `include_bias =
/// false` drops the round-toward-zero bias → SATISFIABLE (a negative non-divisible `a` is a
/// counterexample), proving the check is not vacuous.
pub(crate) fn divpow2_obligation(width: usize, include_bias: bool) -> ProofExpr {
    let w = width;
    let a = blast_bits(&BoundedExpr::BitVecVar("a".into(), w as u32)).unwrap();
    let sign = a[w - 1].clone();
    let mut bad = Bit::Const(false);
    // k in 1..w-1: `2^k` must be a POSITIVE signed W-bit value (the top bit `2^(w-1)` is the negative
    // sign bit, and the VM never emits `DivPow2` for a divisor that isn't a positive power of two).
    for k in 1..w - 1 {
        // bias = a<0 ? (2^k − 1) : 0  ==  low-k bits set to the sign bit (0 when `include_bias` is off).
        let bias: Vec<Bit> = (0..w)
            .map(|j| {
                if include_bias && j < k {
                    sign.clone()
                } else {
                    Bit::Const(false)
                }
            })
            .collect();
        let biased = bv_add(&a, &bias);
        // arithmetic shift right by k: bits down by k, fill the top k with `biased`'s sign bit.
        let fill = biased[w - 1].clone();
        let fast: Vec<Bit> = (0..w)
            .map(|j| {
                if j + k < w {
                    biased[j + k].clone()
                } else {
                    fill.clone()
                }
            })
            .collect();
        // reference: signed trunc div by 2^k.
        let two_k = blast_bits(&BoundedExpr::BitVecConst {
            width: w as u32,
            value: 1u64 << k,
        })
        .unwrap();
        let (refq, _refr) = signed_trunc_divmod(&a, &two_k);
        bad = b_or(bad, b_not(bv_eq(&fast, &refq)));
    }
    bit_to_proof(&bad)
}

/// Round-up unsigned magic for `a / d` at width `w`: the smallest total shift `s` and
/// `magic = ceil(2^s / d)` such that `(a * magic) >> s == a / d` for EVERY `w`-bit `a` (verified
/// exhaustively over the `2^w` inputs — `w` is small). This is the Granlund–Montgomery reciprocal,
/// the mathematical content of the VM's `MagicDivU`.
fn unsigned_magic(d: u64, w: usize) -> (u64, u32) {
    for s in (w as u32)..(4 * w as u32 + 4) {
        let magic = ((1u128 << s) + d as u128 - 1) / d as u128; // ceil(2^s / d)
        if magic >> (w + 3) != 0 {
            continue; // keep the multiply width bounded
        }
        let m = magic as u64;
        if (0..(1u64 << w)).all(|a| ((a as u128 * magic) >> s) as u64 == a / d) {
            return (m, s);
        }
    }
    panic!("no round-up magic for d={d} at w={w}");
}

/// The our-SAT obligation that the ROUND-UP MAGIC form of unsigned division by a constant `d`
/// (`(a * magic) >> s`) equals `a / d` (the reference `bv_udiv`) for every `w`-bit `a` — the semantics
/// of `MagicDivU` (the VM's mul-hi + Knuth-fixup encoding in `compiler.rs::magic_eval` is an
/// implementation of this identity). `prove_unsat` REFUTES it iff correct. `correct = false` shifts by
/// `s+1` (a wrong magic) → SATISFIABLE, proving non-vacuity.
pub(crate) fn magicdiv_obligation(width: usize, d: u64, correct: bool) -> ProofExpr {
    let w = width;
    let (magic, s0) = unsigned_magic(d, w);
    let s = if correct { s0 } else { s0 + 1 } as usize;
    let pw = s + w + 1; // product width holds `a * magic` and the `>> s`
    let a = blast_bits(&BoundedExpr::BitVecVar("a".into(), w as u32)).unwrap();
    let a_ext: Vec<Bit> = (0..pw)
        .map(|j| {
            if j < w {
                a[j].clone()
            } else {
                Bit::Const(false)
            }
        })
        .collect();
    let magic_bits = blast_bits(&BoundedExpr::BitVecConst {
        width: pw as u32,
        value: magic,
    })
    .unwrap();
    let prod = bv_mul(&a_ext, &magic_bits); // pw bits
    let q: Vec<Bit> = (0..w)
        .map(|j| {
            if j + s < pw {
                prod[j + s].clone()
            } else {
                Bit::Const(false)
            }
        })
        .collect();
    let d_bits = blast_bits(&BoundedExpr::BitVecConst {
        width: w as u32,
        value: d,
    })
    .unwrap();
    let (refq, _) = bv_udiv_urem(&a, &d_bits);
    bit_to_proof(&b_not(bv_eq(&q, &refq)))
}

/// `a^e` by NAIVE repeated multiply (the spec), wrapping at width `w`.
fn bv_pow_naive(a: &[Bit], e: u32) -> Vec<Bit> {
    let w = a.len();
    let mut acc: Vec<Bit> = (0..w).map(|i| Bit::Const(i == 0)).collect(); // 1
    for _ in 0..e {
        acc = bv_mul(&acc, a);
    }
    acc
}

/// `a^e` by SQUARE-AND-MULTIPLY (the efficient multi-cycle hardware form), wrapping at width `w`.
fn bv_pow_sqmul(a: &[Bit], e: u32) -> Vec<Bit> {
    let w = a.len();
    let mut result: Vec<Bit> = (0..w).map(|i| Bit::Const(i == 0)).collect(); // 1
    let mut base = a.to_vec();
    let mut ee = e;
    while ee > 0 {
        if ee & 1 == 1 {
            result = bv_mul(&result, &base);
        }
        base = bv_mul(&base, &base);
        ee >>= 1;
    }
    result
}

/// The our-SAT obligation that `Pow` by SQUARE-AND-MULTIPLY (the multi-cycle hardware) equals the naive
/// repeated-multiply spec (both wrapping — matching the VM's fixed-width `checked_pow` on a chip), for
/// every exponent `e` in `0..=maxe` over all `w`-bit `a`. `prove_unsat` REFUTES it iff correct.
/// `correct = false` compares against `a^(e+1)` → SATISFIABLE, proving non-vacuity. Heavy (chains of
/// multipliers) — run on purpose, not in the routine gate.
pub(crate) fn pow_obligation(width: usize, maxe: u32, correct: bool) -> ProofExpr {
    let a = blast_bits(&BoundedExpr::BitVecVar("a".into(), width as u32)).unwrap();
    let mut bad = Bit::Const(false);
    for e in 0..=maxe {
        let sq = bv_pow_sqmul(&a, e);
        let nv = bv_pow_naive(&a, if correct { e } else { e + 1 });
        bad = b_or(bad, b_not(bv_eq(&sq, &nv)));
    }
    bit_to_proof(&bad)
}

// ── BoundedExpr → bits / bool ───────────────────────────────────────────────────────────

/// Blast a bitvector-VALUED `BoundedExpr` into its bits (LSB-first), or `None` if it is not a
/// supported bitvector expression (e.g. an Int, a comparison, or mismatched widths).
fn blast_bits(e: &BoundedExpr) -> Option<Vec<Bit>> {
    count(|w| w.exprs += 1);
    match e {
        BoundedExpr::BitVecConst { width, value } => Some(
            (0..*width)
                .map(|i| Bit::Const((value >> i) & 1 == 1))
                .collect(),
        ),
        BoundedExpr::BitVecVar(name, width) => {
            Some((0..*width).map(|i| b_atom(format!("{name}#{i}"))).collect())
        }
        BoundedExpr::BitVecExtract { high, low, operand } => {
            let bits = blast_bits(operand)?;
            let (lo, hi) = (*low as usize, *high as usize);
            if hi >= bits.len() || lo > hi {
                return None;
            }
            Some(bits[lo..=hi].to_vec())
        }
        BoundedExpr::BitVecConcat(a, b) => {
            // SVA `{a, b}`: `a` is the high half. LSB-first ⇒ b's bits then a's bits.
            let mut bits = blast_bits(b)?;
            bits.extend(blast_bits(a)?);
            Some(bits)
        }
        BoundedExpr::BitVecIte { cond, then, else_ } => {
            // The predicated SELECT / mux tile: lower the boolean predicate to ONE bit, then mux
            // each bit of the two same-width operands through the existing 1-bit `b_ite`. Branchy
            // dataflow collapses to a per-bit select — no control divergence.
            let s = as_gate(lower_bit(cond)?);
            let t = blast_bits(then)?;
            let e = blast_bits(else_)?;
            if t.len() != e.len() {
                return None;
            }
            Some(
                t.into_iter()
                    .zip(e)
                    .map(|(tb, eb)| b_ite(s.clone(), tb, eb))
                    .collect(),
            )
        }
        BoundedExpr::BitVecBinary { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            match op {
                BitVecBoundedOp::Not => Some(a.into_iter().map(b_not).collect()),
                // Width-preserving binary circuits require matching widths.
                _ if a.len() != b.len() => None,
                BitVecBoundedOp::And => Some(zip_map(a, b, b_and)),
                BitVecBoundedOp::Or => Some(zip_map(a, b, b_or)),
                BitVecBoundedOp::Xor => Some(zip_map(a, b, b_xor)),
                BitVecBoundedOp::Add => Some(bv_add(&a, &b)),
                BitVecBoundedOp::Sub => Some(bv_sub(&a, &b)),
                BitVecBoundedOp::Mul => Some(bv_mul(&a, &b)),
                BitVecBoundedOp::Shl => Some(barrel_shift(&a, &b, true, Bit::Const(false))),
                BitVecBoundedOp::Shr => Some(barrel_shift(&a, &b, false, Bit::Const(false))),
                BitVecBoundedOp::AShr => {
                    let sign = a[a.len() - 1].clone();
                    Some(barrel_shift(&a, &b, false, sign))
                }
                // Comparison ops are boolean-valued — not a bit vector.
                BitVecBoundedOp::Eq | BitVecBoundedOp::ULt | BitVecBoundedOp::SLt => None,
            }
        }
        _ => None,
    }
}

fn zip_map(a: Vec<Bit>, b: Vec<Bit>, f: fn(Bit, Bit) -> Bit) -> Vec<Bit> {
    a.into_iter().zip(b).map(|(x, y)| f(x, y)).collect()
}

/// Lower a boolean-VALUED datapath comparison into the shared gate DAG, or `None` if unsupported.
/// Comparisons over bitvectors are unsigned unless the op is explicitly signed (`SLt`).
fn lower_bit(e: &BoundedExpr) -> Option<Bit> {
    count(|w| w.exprs += 1);
    let bit = match e {
        BoundedExpr::BitVecBinary { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            if a.len() != b.len() {
                return None;
            }
            match op {
                BitVecBoundedOp::Eq => bv_eq(&a, &b),
                BitVecBoundedOp::ULt => bv_ult(&a, &b),
                BitVecBoundedOp::SLt => bv_slt(&a, &b),
                _ => return None,
            }
        }
        BoundedExpr::Eq(l, r) => {
            let a = blast_bits(l)?;
            let b = blast_bits(r)?;
            if a.len() != b.len() {
                return None;
            }
            bv_eq(&a, &b)
        }
        BoundedExpr::Lt(l, r)
        | BoundedExpr::Gt(l, r)
        | BoundedExpr::Lte(l, r)
        | BoundedExpr::Gte(l, r) => bv_compare(e, l, r)?,
        BoundedExpr::Comparison { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            if a.len() != b.len() {
                return None;
            }
            match op {
                CmpBoundedOp::Lt => bv_ult(&a, &b),
                CmpBoundedOp::Gt => bv_ult(&b, &a),
                CmpBoundedOp::Lte => b_not(bv_ult(&b, &a)),
                CmpBoundedOp::Gte => b_not(bv_ult(&a, &b)),
            }
        }
        _ => return None,
    };
    Some(bit)
}

/// Lower a boolean-VALUED datapath comparison to a `ProofExpr`, or `None` if it has no circuit or
/// its materialisation is over [`BLAST_BUDGET_NODES`]. Use [`lower_bool_bounded`] when the caller
/// must tell those two apart by name.
pub fn lower_bool(e: &BoundedExpr) -> Option<ProofExpr> {
    lower_bool_bounded(e).ok()
}

/// [`lower_bool`] with the refusal NAMED. The circuit is built in the shared DAG and PRICED before
/// anything is materialised, so an obligation past the budget costs the DAG — never the tree, and
/// never the allocator.
pub fn lower_bool_bounded(e: &BoundedExpr) -> Result<ProofExpr, BlastRefusal> {
    lower_bool_within(e, BLAST_BUDGET_NODES)
}

/// [`lower_bool_bounded`] against an EXPLICIT ceiling. The production ceiling is
/// [`BLAST_BUDGET_NODES`]; a caller passing its own is how a lock proves the guard is live code —
/// shrink the ceiling and an obligation that certifies must come back `Budget`, with the same
/// arithmetic, not a special case.
pub fn lower_bool_within(e: &BoundedExpr, budget: u128) -> Result<ProofExpr, BlastRefusal> {
    let bit = lower_bit(e).ok_or(BlastRefusal::NotBounded)?;
    let size = measure(&bit);
    if size.tree_nodes > budget {
        return Err(BlastRefusal::Budget { size, budget });
    }
    Ok(bit_to_proof(&bit))
}

/// What blasting `e` costs — gates allocated vs. `ProofExpr` nodes a materialisation would hold —
/// without materialising it. `None` when `e` has no circuit. The instrument the budget reads, and
/// the one a lock reads to prove the blast stays linear in the datapath.
pub fn blast_size(e: &BoundedExpr) -> Option<BlastSize> {
    Some(measure(&lower_bit(e)?))
}

/// Unsigned magnitude comparison for the top-level `Lt/Gt/Lte/Gte` BoundedExpr variants.
fn bv_compare(e: &BoundedExpr, l: &BoundedExpr, r: &BoundedExpr) -> Option<Bit> {
    let a = blast_bits(l)?;
    let b = blast_bits(r)?;
    if a.len() != b.len() {
        return None;
    }
    Some(match e {
        BoundedExpr::Lt(..) => bv_ult(&a, &b),
        BoundedExpr::Gt(..) => bv_ult(&b, &a),
        BoundedExpr::Lte(..) => b_not(bv_ult(&b, &a)),
        BoundedExpr::Gte(..) => b_not(bv_ult(&a, &b)),
        _ => return None,
    })
}

// ── The DAG front door: build a datapath circuit WITHOUT a `BoundedExpr` tree ───────────
//
// `BoundedExpr` is a `Box` tree, so a producer that reads a subterm twice must CLONE it — and a
// dataflow body whose cells read each other's results is a DAG, so the tree is the DAG unfolded.
// `fib.lg`'s 75-cell fast-doubling body unfolds to 1 657 763 `BoundedExpr` nodes; ten cells more
// and there is no machine that holds it. The blaster's own sharing cannot help, because by the
// time it is called the explosion has already happened upstream.
//
// So the producers below (`fabric_grid::cells_to_gates`, `fabric_grid::grid_output_to_gates`)
// build into the SHARED gate DAG directly: a `Word` is a vector of `Bit`s, cloning one is a
// refcount bump per bit, and a body's bounded form stays linear in its CELLS. The same DAG is
// then clausified in place ([`assert_not`]) — the `ProofExpr` tree, which is exponentially larger
// than the circuit for a reconvergent datapath, is never materialised at all.
//
// This is a performance property, never a semantic one: [`assert_not`] emits the SAME clauses, in
// the same order, that `Cnf::assert(&Not(bit_to_proof(b)))` emits — proven by
// `the_gate_path_and_the_tree_path_clausify_identically` in this module's tests.

/// A blasted datapath VALUE — its bits, LSB-first, in the shared gate DAG. Cloning is a refcount
/// bump per bit, never a copy of the circuit.
#[derive(Clone)]
pub struct Word(Vec<Bit>);

impl Word {
    /// The width in bits.
    pub fn width(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Debug for Word {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Word({} bits)", self.0.len())
    }
}

/// A blasted PREDICATE — one bit of the shared gate DAG.
#[derive(Clone)]
pub struct Pred(Bit);

impl std::fmt::Debug for Pred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pred({:?})", measure(&self.0))
    }
}

/// A named input word: the bits `name#0 .. name#(width-1)`, the same signal spelling
/// [`blast_word`] gives a `BitVecVar` — so a circuit built here and one blasted from a
/// `BoundedExpr` share a variable namespace and can be mitered against each other.
pub fn word_var(name: &str, width: u32) -> Word {
    Word((0..width).map(|i| b_atom(format!("{name}#{i}"))).collect())
}

/// A constant word, truncated to `width`.
pub fn word_const(width: u32, value: u64) -> Word {
    Word(
        (0..width)
            .map(|i| Bit::Const((value >> i) & 1 == 1))
            .collect(),
    )
}

/// One of the width-preserving binary circuits (`And..AShr`, and unary `Not` on `a`). `None` when
/// the widths disagree or the op is boolean-valued — use [`word_compare`] for those.
pub fn word_binary(op: BitVecBoundedOp, a: &Word, b: &Word) -> Option<Word> {
    let (a, b) = (&a.0, &b.0);
    let bits = match op {
        BitVecBoundedOp::Not => a.iter().cloned().map(b_not).collect(),
        _ if a.len() != b.len() => return None,
        BitVecBoundedOp::And => zip_map(a.clone(), b.clone(), b_and),
        BitVecBoundedOp::Or => zip_map(a.clone(), b.clone(), b_or),
        BitVecBoundedOp::Xor => zip_map(a.clone(), b.clone(), b_xor),
        BitVecBoundedOp::Add => bv_add(a, b),
        BitVecBoundedOp::Sub => bv_sub(a, b),
        BitVecBoundedOp::Mul => bv_mul(a, b),
        BitVecBoundedOp::Shl => barrel_shift(a, b, true, Bit::Const(false)),
        BitVecBoundedOp::Shr => barrel_shift(a, b, false, Bit::Const(false)),
        BitVecBoundedOp::AShr => {
            let sign = a[a.len() - 1].clone();
            barrel_shift(a, b, false, sign)
        }
        BitVecBoundedOp::Eq | BitVecBoundedOp::ULt | BitVecBoundedOp::SLt => return None,
    };
    Some(Word(bits))
}

/// One of the boolean-valued comparisons (`Eq`, `ULt`, `SLt`). `None` for a value-producing op or
/// a width mismatch.
pub fn word_compare(op: BitVecBoundedOp, a: &Word, b: &Word) -> Option<Pred> {
    if a.0.len() != b.0.len() {
        return None;
    }
    Some(Pred(match op {
        BitVecBoundedOp::Eq => bv_eq(&a.0, &b.0),
        BitVecBoundedOp::ULt => bv_ult(&a.0, &b.0),
        BitVecBoundedOp::SLt => bv_slt(&a.0, &b.0),
        _ => return None,
    }))
}

/// The mux tile: `cond ? t : e`, bit by bit — the same 1-bit select `BitVecIte` blasts through, so
/// a predicated select built here and one built from a `BoundedExpr` are the same circuit.
pub fn word_select(cond: &Pred, t: &Word, e: &Word) -> Option<Word> {
    if t.0.len() != e.0.len() {
        return None;
    }
    let s = as_gate(cond.0.clone());
    Some(Word(
        t.0.iter()
            .cloned()
            .zip(e.0.iter().cloned())
            .map(|(tb, eb)| b_ite(s.clone(), tb, eb))
            .collect(),
    ))
}

/// The miter: `a == b` over every bit. `None` on a width mismatch.
pub fn words_equal(a: &Word, b: &Word) -> Option<Pred> {
    word_compare(BitVecBoundedOp::Eq, a, b)
}

/// What this predicate's circuit costs — gates allocated vs. the `ProofExpr` tree a
/// materialisation would hold. The instrument a budget reads.
pub fn pred_size(p: &Pred) -> BlastSize {
    measure(&p.0)
}

/// What a whole WORD's circuit costs, counting the DAG the bits SHARE exactly once — an adder's
/// carry chain feeds every bit above it, so pricing the bits separately would count the same gates
/// over and over. The instrument a datapath budget reads before a miter exists.
pub fn word_size(w: &Word) -> BlastSize {
    let roots: Vec<&Bit> = w.0.iter().collect();
    measure_all(&roots)
}

/// Blast a bitvector-VALUED `BoundedExpr` into the shared DAG — the bridge for a caller that still
/// speaks the tree form.
pub fn blast_word(e: &BoundedExpr) -> Option<Word> {
    Some(Word(blast_bits(e)?))
}

/// Blast a boolean-VALUED `BoundedExpr` into the shared DAG.
pub fn blast_pred(e: &BoundedExpr) -> Option<Pred> {
    Some(Pred(lower_bit(e)?))
}

/// Materialise a predicate's `ProofExpr` tree. The escape hatch for a consumer that genuinely
/// needs the tree (an SMT printer, a `ProofExpr`-shaped prover call) — and the thing every path
/// that only wants a CNF must NOT call, because the tree is exponentially larger than the circuit.
pub fn pred_to_proof(p: &Pred) -> ProofExpr {
    bit_to_proof(&p.0)
}

/// Tseitin-encode one gate DAG into `cnf`, defining each distinct gate EXACTLY once, and return
/// the literal that equals the root. Linear in the DAG.
fn encode_bit(root: &Bit, cnf: &mut Cnf) -> Lit {
    fn lit_of(cnf: &mut Cnf, lits: &HashMap<usize, Lit>, b: &Bit) -> Lit {
        match b {
            // `bit_to_proof` spells a constant `Or(c, ¬c)` / `And(c, ¬c)` over the reserved atom;
            // both are cached by the Tseitin table, so repeating one is free.
            Bit::Const(v) => {
                let c = cnf.atom_lit(BIT_CONST_ATOM);
                if *v {
                    cnf.encode_or(c, c.negated())
                } else {
                    cnf.encode_and(c, c.negated())
                }
            }
            Bit::Dyn(g) => lits[&gate_key(g)],
        }
    }
    let order = dag_postorder(&[root]);
    let mut lits: HashMap<usize, Lit> = HashMap::with_capacity(order.len());
    for (key, g) in &order {
        let l = match g {
            Gate::Atom(n) => cnf.atom_lit(n),
            Gate::Not(x) => lit_of(cnf, &lits, x).negated(),
            Gate::And(x, y) => {
                let (a, b) = (lit_of(cnf, &lits, x), lit_of(cnf, &lits, y));
                cnf.encode_and(a, b)
            }
            Gate::Or(x, y) => {
                let (a, b) = (lit_of(cnf, &lits, x), lit_of(cnf, &lits, y));
                cnf.encode_or(a, b)
            }
            Gate::Iff(x, y) => {
                let (a, b) = (lit_of(cnf, &lits, x), lit_of(cnf, &lits, y));
                cnf.encode_iff(a, b)
            }
        };
        lits.insert(*key, l);
    }
    lit_of(cnf, &lits, root)
}

/// The literals of the clause `b`, mirroring `Cnf::clause_lits` on `bit_to_proof(b)`: flatten a
/// disjunction, De-Morgan a negated conjunction, literalise an atom, and Tseitin anything else
/// into ONE auxiliary. Structure-aware clausification (Plaisted & Greenbaum) is what keeps the
/// certificate small, so the gate path must reproduce it exactly — not merely something sound.
fn clause_lits(b: &Bit, cnf: &mut Cnf) -> Vec<Lit> {
    match b {
        // `Or(c, ¬c)` flattens into a (tautological) two-literal clause.
        Bit::Const(true) => {
            let c = cnf.atom_lit(BIT_CONST_ATOM);
            vec![c, c.negated()]
        }
        // `And(c, ¬c)` is not a clause: one auxiliary.
        Bit::Const(false) => vec![encode_bit(b, cnf)],
        Bit::Dyn(g) => match &**g {
            Gate::Atom(n) => vec![cnf.atom_lit(n)],
            Gate::Or(x, y) => {
                let mut v = clause_lits(x, cnf);
                v.extend(clause_lits(y, cnf));
                v
            }
            Gate::Not(inner) => match inner {
                // ¬(a ∧ b) ≡ ¬a ∨ ¬b, and ¬¬x ≡ x — both stay ONE clause with no auxiliary.
                Bit::Dyn(ig) => match &**ig {
                    Gate::Atom(n) => vec![cnf.atom_lit(n).negated()],
                    Gate::And(x, y) => {
                        let mut v = clause_lits_of_negation(x, cnf);
                        v.extend(clause_lits_of_negation(y, cnf));
                        v
                    }
                    Gate::Not(x) => clause_lits(x, cnf),
                    _ => vec![encode_bit(b, cnf)],
                },
                Bit::Const(_) => vec![encode_bit(b, cnf)],
            },
            _ => vec![encode_bit(b, cnf)],
        },
    }
}

/// The literals of the clause `¬b` — `clause_lits` composed with `Cnf`'s `negate_expr`, which
/// cancels a leading `Not` rather than stacking one.
fn clause_lits_of_negation(b: &Bit, cnf: &mut Cnf) -> Vec<Lit> {
    match b {
        // ¬(c ∧ ¬c) De-Morgans into a (tautological) two-literal clause.
        Bit::Const(false) => {
            let c = cnf.atom_lit(BIT_CONST_ATOM);
            vec![c.negated(), c]
        }
        Bit::Const(true) => vec![encode_bit(b, cnf).negated()],
        Bit::Dyn(g) => match &**g {
            // negate_expr(Not(p)) = p
            Gate::Not(x) => clause_lits(x, cnf),
            Gate::Atom(n) => vec![cnf.atom_lit(n).negated()],
            Gate::And(x, y) => {
                let mut v = clause_lits_of_negation(x, cnf);
                v.extend(clause_lits_of_negation(y, cnf));
                v
            }
            _ => vec![encode_bit(b, cnf).negated()],
        },
    }
}

/// Assert `¬p` into `cnf` — the refutation obligation for a miter, clausified STRAIGHT FROM THE
/// GATE DAG. Emits exactly the clauses `Cnf::assert(&Not(pred_to_proof(p)))` emits, in the same
/// order, over the same variables; it just never builds the tree to read them off.
pub fn assert_not(p: &Pred, cnf: &mut Cnf) {
    let lits = clause_lits_of_negation(&p.0, cnf);
    cnf.add_clause(lits);
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/bitblast/tests.rs"]
mod tests;
