//! DRAM dataflow composition — the spatial-dataflow layer of the compute-in-memory target.
//!
//! The DRAM plane's hardware basis is `{Copy, And, Or}` (unmodified ComputeDRAM: RowClone + the
//! sense-amp majority) plus `Not` (the Ambit tier: dual-contact cells). `{AND, OR, NOT}` is
//! FUNCTIONALLY COMPLETE, so every boolean function — XOR, NAND, NOR, XNOR, set difference, … — is
//! realized by CHAINING those primitives through intermediate rows. That chaining IS a spatial
//! dataflow graph: edges are DRAM rows, nodes are in-DRAM primitives, and the memory computes the
//! whole graph in place. Composed bit-serially over bitplanes, the same basis yields integer
//! ARITHMETIC (ripple-carry ADD from XOR+MAJ) — the SIMDRAM construction (Hajinazar et al., ASPLOS
//! 2021). Every composed op is proven ≡ its reference by executing the emitted prim sequence on the
//! Ambit-tier DRAM executor ([`exec_dram_ambit`]).

use crate::codegen_sva::unified_placement::{exec_dram_ambit, Prim, Row};

/// A straight-line in-DRAM dataflow graph: a growable row heap (operands then scratch) and the prim
/// sequence computed over it. `not`/`and`/`or`/`copy` append a node writing a FRESH scratch row and
/// return its index; higher-level gates compose those. This is the compiler-side spatial graph the
/// DRAM plane executes.
#[derive(Clone, Debug, Default)]
pub struct DramDataflow {
    next: usize,
    prog: Vec<Prim>,
}

impl DramDataflow {
    /// A dataflow over `inputs` operand rows (indices `0..inputs`); scratch rows are appended above.
    pub fn new(inputs: usize) -> Self {
        DramDataflow {
            next: inputs,
            prog: Vec::new(),
        }
    }

    fn fresh(&mut self) -> usize {
        let r = self.next;
        self.next += 1;
        r
    }

    /// Total rows (operands + scratch) the graph touches.
    pub fn rows(&self) -> usize {
        self.next
    }

    /// The number of in-DRAM primitive steps (dataflow nodes) — the cost of the kernel.
    pub fn node_count(&self) -> usize {
        self.prog.len()
    }

    /// The emitted prim sequence (the spatial dataflow, in execution order).
    pub fn program(&self) -> &[Prim] {
        &self.prog
    }

    // ── The hardware basis: {Copy, And, Or} (ComputeDRAM) + Not (Ambit). ──
    pub fn copy(&mut self, a: usize) -> usize {
        let d = self.fresh();
        self.prog.push(Prim::Copy { dst: d, src: a });
        d
    }
    pub fn not(&mut self, a: usize) -> usize {
        let d = self.fresh();
        self.prog.push(Prim::Not { dst: d, src: a });
        d
    }
    pub fn and(&mut self, a: usize, b: usize) -> usize {
        let d = self.fresh();
        self.prog.push(Prim::And { dst: d, a, b });
        d
    }
    pub fn or(&mut self, a: usize, b: usize) -> usize {
        let d = self.fresh();
        self.prog.push(Prim::Or { dst: d, a, b });
        d
    }

    // ── Composed gates (functional completeness from {AND, OR, NOT}). ──
    /// XOR = (a OR b) AND NOT(a AND b).
    pub fn xor(&mut self, a: usize, b: usize) -> usize {
        let or = self.or(a, b);
        let and = self.and(a, b);
        let nand = self.not(and);
        self.and(or, nand)
    }
    pub fn nand(&mut self, a: usize, b: usize) -> usize {
        let t = self.and(a, b);
        self.not(t)
    }
    pub fn nor(&mut self, a: usize, b: usize) -> usize {
        let t = self.or(a, b);
        self.not(t)
    }
    pub fn xnor(&mut self, a: usize, b: usize) -> usize {
        let x = self.xor(a, b);
        self.not(x)
    }
    /// Set difference a \ b = a AND NOT b.
    pub fn difference(&mut self, a: usize, b: usize) -> usize {
        let nb = self.not(b);
        self.and(a, nb)
    }
    /// Bitwise majority MAJ(a,b,c) = (a AND b) OR (a AND c) OR (b AND c) — the full-adder carry.
    pub fn majority(&mut self, a: usize, b: usize, c: usize) -> usize {
        let ab = self.and(a, b);
        let ac = self.and(a, c);
        let bc = self.and(b, c);
        let t = self.or(ab, ac);
        self.or(t, bc)
    }

    /// Realize an ARBITRARY 2-input boolean function from its 4-bit truth table (bit `2·a+b` =
    /// f(a,b)) as an OR of minterms over `{AND, OR, NOT}` — a constructive functional-completeness
    /// witness. An empty table yields constant 0 (a AND NOT a).
    pub fn boolean_fn(&mut self, a: usize, b: usize, truth: u8) -> usize {
        let mut acc: Option<usize> = None;
        for k in 0..4u8 {
            if (truth >> k) & 1 == 1 {
                let av = (k >> 1) & 1;
                let bv = k & 1;
                let ta = if av == 1 { a } else { self.not(a) };
                let tb = if bv == 1 { b } else { self.not(b) };
                let m = self.and(ta, tb);
                acc = Some(match acc {
                    None => m,
                    Some(p) => self.or(p, m),
                });
            }
        }
        match acc {
            Some(r) => r,
            None => {
                let na = self.not(a);
                self.and(a, na)
            }
        }
    }

    /// Execute the dataflow in-DRAM (Ambit tier): `inputs` fill rows `0..inputs.len()`, scratch rows
    /// are zero-initialized, and each node runs through [`exec_dram_ambit`]. Returns the final heap.
    pub fn run(&self, inputs: &[Row], width: usize) -> Vec<Row> {
        let mut heap: Vec<Row> = inputs.to_vec();
        heap.resize(self.next, vec![0u64; width]);
        for p in &self.prog {
            exec_dram_ambit(&mut heap, p)
                .expect("dataflow node must be a DRAM-realizable primitive");
        }
        heap
    }
}

/// Bit-serial ripple-carry ADD over `B` bitplanes (SIMD across the lanes packed in each row):
/// for each bit i, `sum_i = a_i XOR b_i XOR carry`, `carry = MAJ(a_i, b_i, carry)`. `a`/`b` are the
/// operand-plane row indices (LSB first); `carry0` is a zero row. Returns the `B` sum-plane rows.
/// Composed entirely from the `{AND, OR, NOT}` basis — arithmetic emerging from boolean dataflow.
pub fn bit_serial_add(
    df: &mut DramDataflow,
    a: &[usize],
    b: &[usize],
    carry0: usize,
) -> Vec<usize> {
    assert_eq!(a.len(), b.len(), "operands must have equal bit width");
    let mut carry = carry0;
    let mut sum = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let axb = df.xor(a[i], b[i]);
        sum.push(df.xor(axb, carry));
        if i + 1 < a.len() {
            carry = df.majority(a[i], b[i], carry);
        }
    }
    sum
}

/// Bit-serial two's-complement SUBTRACT: `a - b = a + NOT(b) + 1` (mod 2^B). Negates each `b` plane
/// and ripple-adds with carry-in 1 (`ones_carry` = a row with every lane's bit set). Returns the `B`
/// difference-plane rows. Arithmetic subtraction, still composed from the boolean basis in-DRAM.
pub fn bit_serial_sub(
    df: &mut DramDataflow,
    a: &[usize],
    b: &[usize],
    ones_carry: usize,
) -> Vec<usize> {
    let nb: Vec<usize> = b.iter().map(|&bi| df.not(bi)).collect();
    bit_serial_add(df, a, &nb, ones_carry)
}

/// Bit-serial unsigned MULTIPLY (shift-add): for each bit i of `b`, add `a` (masked by plane `b[i]`)
/// shifted up by i into an `out_width`-plane accumulator. Composed from AND + [`bit_serial_add`] —
/// integer multiply from the boolean basis. Result is `a·b` mod 2^out_width per lane; `zero` is a
/// zero row (accumulator seed + carry-in + high-plane filler).
pub fn bit_serial_mul(
    df: &mut DramDataflow,
    a: &[usize],
    b: &[usize],
    out_width: usize,
    zero: usize,
) -> Vec<usize> {
    let mut acc: Vec<usize> = vec![zero; out_width];
    for i in 0..b.len() {
        // partial = (a masked by b[i]) placed at bit offset i.
        let masked: Vec<usize> = a.iter().map(|&aj| df.and(aj, b[i])).collect();
        let addend: Vec<usize> = (0..out_width)
            .map(|p| {
                if p >= i && p - i < a.len() {
                    masked[p - i]
                } else {
                    zero
                }
            })
            .collect();
        acc = bit_serial_add(df, &acc, &addend, zero);
    }
    acc
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_dram/dataflow/tests.rs"]
mod tests;
