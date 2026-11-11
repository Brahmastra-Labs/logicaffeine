//! DPLL(XOR) — the live GF(2) reasoning engine.
//!
//! A system of XOR (parity) constraints `x_{i1} ⊕ … ⊕ x_{ik} = b` is a linear system over GF(2).
//! Plain CDCL refutes it only through resolution, which is exponential on parity (Tseitin/par
//! families); Gaussian elimination decides it in polynomial time. This engine carries that linear
//! reasoning *into* the search: given the solver's current partial assignment it derives every
//! XOR-forced literal — and detects contradiction — by Gaussian elimination over the unassigned
//! variables, so the solver never has to rediscover linear consequences by resolution.
//!
//! **Soundness is the whole game.** Every clause this engine hands back to CDCL is the gadget clause
//! of a *derived* equation `E* = Σ_{i∈P} E_i` (a GF(2) sum of recovered equations, tracked by the
//! provenance set `P`). Each `E_i` is a logical consequence of the formula (a full XOR gadget's
//! clauses imply its equation), so `E*` is too, and so is the single gadget clause we emit — which is
//! exactly unit (one unassigned literal) when `E*` forces a variable, or fully falsified when `E*`
//! is violated. The engine therefore can never make the solver unsound; it can only make it faster.
//!
//! This module is the correctness-validated core (an exhaustive brute-force oracle checks that the
//! derived forced-literals/conflicts are precisely the GF(2) consequences, and that every emitted
//! clause is implied and correctly unit/falsified). The engines the search runs with — [`IncXor`],
//! and [`WatchedXor`], whose work per assignment is the rows the assignment can affect — are held
//! to the same consequences under solver-shaped trails.

use crate::cdcl::{Lit, Var};
use crate::xorsat::XorEquation;

/// One parity constraint over a variable set, carrying the provenance (which original equations it
/// is a GF(2) sum of) so a derived implication can be explained by an implied clause.
#[derive(Clone)]
struct Row {
    /// Bitset over variables: bit `v` set ⇔ variable `v` occurs in this equation.
    vars: Vec<u64>,
    /// Right-hand side parity.
    rhs: bool,
    /// Bitset over ORIGINAL equation indices: which equations XOR to this row.
    prov: Vec<u64>,
}

/// The result of consulting the engine at a Boolean fixpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XorStep {
    /// The linear system is violated under the current assignment; the clause is implied by the
    /// formula and fully falsified (a conflict).
    Conflict(Vec<Lit>),
    /// `lit` is forced; the clause is implied and unit (its only unassigned literal is `lit`).
    Imply { lit: Lit, reason: Vec<Lit> },
    /// No XOR-forced literal and no contradiction at this assignment.
    Sat,
}

/// A GF(2) constraint system with Gaussian reasoning under a partial assignment.
pub struct XorEngine {
    num_vars: usize,
    var_words: usize,
    eq_words: usize,
    /// The original equations as variable bitsets (for building explanation clauses).
    orig_vars: Vec<Vec<u64>>,
    orig_rhs: Vec<bool>,
}

#[inline]
fn words_for(n: usize) -> usize {
    n.div_ceil(64).max(1)
}
#[inline]
fn bit_set(bits: &mut [u64], i: usize) {
    bits[i / 64] |= 1u64 << (i % 64);
}
#[inline]
fn bit_flip(bits: &mut [u64], i: usize) {
    bits[i / 64] ^= 1u64 << (i % 64);
}
#[inline]
fn bit_get(bits: &[u64], i: usize) -> bool {
    (bits[i / 64] >> (i % 64)) & 1 == 1
}
#[inline]
fn xor_into(dst: &mut [u64], src: &[u64]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d ^= *s;
    }
}
#[inline]
fn bit_clear(bits: &mut [u64], i: usize) {
    bits[i / 64] &= !(1u64 << (i % 64));
}
/// Remove `row` from an occurrence list (unordered set; rows are sparse so the scan is short).
#[inline]
fn occ_remove(list: &mut Vec<usize>, row: usize) {
    if let Some(p) = list.iter().position(|&r| r == row) {
        list.swap_remove(p);
    }
}
#[inline]
fn is_zero(bits: &[u64]) -> bool {
    bits.iter().all(|&w| w == 0)
}
#[inline]
fn popcount(bits: &[u64]) -> u32 {
    bits.iter().map(|w| w.count_ones()).sum()
}
/// Index of the lowest set bit, or `None` if all zero.
fn lowest_set(bits: &[u64]) -> Option<usize> {
    for (wi, &w) in bits.iter().enumerate() {
        if w != 0 {
            return Some(wi * 64 + w.trailing_zeros() as usize);
        }
    }
    None
}
fn set_bits(bits: &[u64]) -> Vec<usize> {
    let mut out = Vec::new();
    for (wi, &w) in bits.iter().enumerate() {
        let mut w = w;
        while w != 0 {
            let b = w.trailing_zeros() as usize;
            out.push(wi * 64 + b);
            w &= w - 1;
        }
    }
    out
}

impl XorEngine {
    /// Build the engine from recovered XOR equations over `num_vars` variables. Equations with a
    /// variable out of range are dropped (defensive; callers pass in-range systems). A variable named
    /// twice in an equation cancels, as over GF(2).
    pub fn new(num_vars: usize, eqs: &[XorEquation]) -> Self {
        let var_words = words_for(num_vars);
        let eq_words = words_for(eqs.len());
        let mut orig_vars = Vec::with_capacity(eqs.len());
        let mut orig_rhs = Vec::with_capacity(eqs.len());
        for e in eqs {
            let mut vars = vec![0u64; var_words];
            let mut ok = true;
            for &v in &e.vars {
                if v >= num_vars {
                    ok = false;
                    break;
                }
                bit_flip(&mut vars, v);
            }
            if !ok {
                continue;
            }
            orig_vars.push(vars);
            orig_rhs.push(e.rhs);
        }
        XorEngine {
            num_vars,
            var_words,
            eq_words,
            orig_vars,
            orig_rhs,
        }
    }

    /// Is this system non-trivial enough to be worth running (≥1 equation)?
    pub fn is_active(&self) -> bool {
        !self.orig_vars.is_empty()
    }

    /// The derived equation `E* = Σ_{i∈prov} E_i`: its full variable bitset and parity.
    fn derived(&self, prov: &[u64]) -> (Vec<u64>, bool) {
        let mut vars = vec![0u64; self.var_words];
        let mut rhs = false;
        for i in set_bits(prov) {
            xor_into(&mut vars, &self.orig_vars[i]);
            rhs ^= self.orig_rhs[i];
        }
        (vars, rhs)
    }

    /// Build the gadget clause of derived equation `E*` (variable bitset `dv`, parity `drhs`) that is
    /// falsified by the current assignment together with `forced` (when `forced` is `Some((v, val))`,
    /// `v` is the one unassigned variable and `val` its implied value). The clause's literal for each
    /// variable `u` is the negation of `u`'s value in that falsifying assignment, so the clause is
    /// fully false there — i.e. unit on `forced`'s literal, or an outright conflict when `forced` is
    /// `None`.
    fn gadget_clause(
        &self,
        dv: &[u64],
        assign: &[Option<bool>],
        forced: Option<(usize, bool)>,
    ) -> Vec<Lit> {
        let mut clause = Vec::new();
        for u in set_bits(dv) {
            match forced {
                // The forced variable's literal is `Lit(v, val)`: it is the clause's single
                // unassigned literal, so unit propagation drives `v` to `val`.
                Some((fv, fval)) if fv == u => clause.push(Lit::new(u as u32, fval)),
                // Every other variable is assigned; its literal is false under that value, so the
                // clause is all-false on the forbidden (wrong-parity) row — unit on `forced`, or a
                // full conflict when `forced` is `None`.
                _ => {
                    let val = assign[u].expect("gadget var must be assigned");
                    clause.push(Lit::new(u as u32, !val));
                }
            }
        }
        clause
    }

    /// The core query: under partial assignment `assign` (`None` = unassigned), return the first
    /// XOR-forced literal or a conflict, each with an implied, correctly-shaped clause. Complete —
    /// it finds every linear consequence, not just per-equation ones — via Gaussian elimination over
    /// the unassigned variables with provenance tracking.
    pub fn analyze(&self, assign: &[Option<bool>]) -> XorStep {
        match self.all_consequences(assign) {
            Err(conflict) => XorStep::Conflict(conflict),
            Ok(forced) => match forced.into_iter().next() {
                Some((lit, reason)) => XorStep::Imply { lit, reason },
                None => XorStep::Sat,
            },
        }
    }

    /// Every XOR-forced literal under `assign` (with implied unit reasons), or `Err(conflict clause)`
    /// if the system is contradicted. This is the complete, oracle-checkable consequence set.
    pub fn all_consequences(
        &self,
        assign: &[Option<bool>],
    ) -> Result<Vec<(Lit, Vec<Lit>)>, Vec<Lit>> {
        // Reduce each equation by the current assignment: move assigned variables to the rhs, keeping
        // only the unassigned variables — and remember the provenance so we can rebuild E*.
        let mut rows: Vec<Row> = Vec::with_capacity(self.orig_vars.len());
        for (i, ov) in self.orig_vars.iter().enumerate() {
            let mut vars = vec![0u64; self.var_words];
            let mut rhs = self.orig_rhs[i];
            for v in set_bits(ov) {
                match assign[v] {
                    Some(val) => rhs ^= val,       // assigned: fold into the parity
                    None => bit_set(&mut vars, v), // unassigned: stays a live coefficient
                }
            }
            let mut prov = vec![0u64; self.eq_words];
            bit_set(&mut prov, i);
            rows.push(Row { vars, rhs, prov });
        }

        // Gaussian elimination over the unassigned variables, carrying provenance through every XOR.
        // pivot_for[v] = index in `basis` of the row whose pivot (lowest live var) is `v`.
        let mut basis: Vec<Row> = Vec::new();
        let mut pivot_at: Vec<Option<usize>> = vec![None; self.num_vars];
        let mut forced: Vec<(Lit, Vec<Lit>)> = Vec::new();

        for mut row in rows {
            // Reduce `row` against EVERY pivot column it still contains (not merely its lowest bit —
            // a higher pivot column left unreduced is what hides a forced variable).
            loop {
                let pivot_col = set_bits(&row.vars)
                    .into_iter()
                    .find(|&p| pivot_at[p].is_some());
                match pivot_col {
                    Some(p) => {
                        let b = &basis[pivot_at[p].unwrap()];
                        xor_into(&mut row.vars, &b.vars);
                        row.rhs ^= b.rhs;
                        xor_into(&mut row.prov, &b.prov);
                    }
                    None => break,
                }
            }
            if is_zero(&row.vars) {
                if row.rhs {
                    // 0 = 1: contradiction. Explain with the derived equation's gadget under `assign`.
                    let (dv, _) = self.derived(&row.prov);
                    return Err(self.gadget_clause(&dv, assign, None));
                }
                continue; // 0 = 0: redundant.
            }
            let p = lowest_set(&row.vars).unwrap();
            // Gauss-JORDAN: eliminate the new pivot `p` from every existing basis row, keeping the
            // basis in REDUCED row echelon form. Without this back-substitution a variable forced
            // only after a later pivot is found stays hidden inside an earlier row (incompleteness).
            for b in basis.iter_mut() {
                if bit_get(&b.vars, p) {
                    xor_into(&mut b.vars, &row.vars);
                    b.rhs ^= row.rhs;
                    xor_into(&mut b.prov, &row.prov);
                }
            }
            pivot_at[p] = Some(basis.len());
            basis.push(row);
        }

        // A basis row with a single live variable forces it; explain with the derived gadget.
        for row in &basis {
            if popcount(&row.vars) == 1 {
                let v = lowest_set(&row.vars).unwrap();
                let val = row.rhs;
                let (dv, _) = self.derived(&row.prov);
                let reason = self.gadget_clause(&dv, assign, Some((v, val)));
                forced.push((Lit::new(v as u32, val), reason));
            }
        }
        Ok(forced)
    }
}

/// Incremental GF(2) engine — the *fast* DPLL(XOR) core.
///
/// The matrix is held in reduced row-echelon form and updated in O(affected-rows) work per variable
/// assignment (each assignment is substituted out, with at most one re-pivot), not by re-running
/// Gaussian over the whole system every call. A first-touch undo trail makes backtrack O(touched).
/// Provenance is carried through every row XOR, so the forced/conflict explanation clauses are the
/// same implied gadget clauses the recompute [`XorEngine`] produces. It is differentially tested
/// against that recompute oracle under random assign/unassign sequences — the fast engine is proven
/// equivalent to the proven-correct one before it ever drives the solver.
pub struct IncXor {
    num_vars: usize,
    var_words: usize,
    orig_vars: Vec<Vec<u64>>,
    rows: Vec<IncRow>,
    /// `pivot_row[v] = Some(r)` ⇔ free variable `v` is the pivot of row `r`.
    pivot_row: Vec<Option<usize>>,
    /// Occurrence index: `occ[v]` lists the rows whose bitset currently contains variable `v`. The
    /// reduced matrix is SPARSE (avg ~17 of 3176 vars/row on par32), so this lets each assignment
    /// touch only the handful of rows that mention the variable instead of scanning all rows — the
    /// watch-layer of the engine.
    occ: Vec<Vec<usize>>,
    /// `weight[r]` = popcount of row `r`'s bitset, maintained incrementally. A weight of 1 is a
    /// forced unit, 0 (with `rhs`) a conflict — so propagation reads the cheap counter instead of
    /// re-popcounting every row.
    weight: Vec<u32>,
    /// Candidate queue: a superset of the rows currently at weight ≤ 1. Pushed whenever a row drops
    /// to that range; [`IncXor::state`] drains it, reports the genuine units/conflicts, and keeps only
    /// the still-low rows — so propagation is O(touched), not O(rows). `in_low` dedups it.
    low: Vec<usize>,
    in_low: Vec<bool>,
    value: Vec<Option<bool>>,
    applied: Vec<usize>,
    undo: Vec<Frame>,
}

#[derive(Clone)]
struct IncRow {
    bits: Vec<u64>,
    rhs: bool,
    prov: Vec<u64>,
}

/// First-touch snapshots for undoing one assignment.
struct Frame {
    var: usize,
    rows: Vec<(usize, IncRow)>,
    pivots: Vec<(usize, Option<usize>)>,
    touched_rows: std::collections::HashSet<usize>,
    touched_pivots: std::collections::HashSet<usize>,
}

impl IncXor {
    /// Build from recovered equations, reducing the matrix to RREF once. A variable named twice in an
    /// equation cancels, as over GF(2).
    pub fn new(num_vars: usize, eqs: &[XorEquation]) -> Self {
        let var_words = words_for(num_vars);
        let eq_words = words_for(eqs.len());
        let mut orig_vars = Vec::new();
        let mut staged: Vec<IncRow> = Vec::new();
        for e in eqs {
            let mut bits = vec![0u64; var_words];
            let mut ok = true;
            for &v in &e.vars {
                if v >= num_vars {
                    ok = false;
                    break;
                }
                bit_flip(&mut bits, v);
            }
            if !ok {
                continue;
            }
            let idx = orig_vars.len();
            let mut prov = vec![0u64; eq_words];
            bit_set(&mut prov, idx);
            orig_vars.push(bits.clone());
            staged.push(IncRow {
                bits,
                rhs: e.rhs,
                prov,
            });
        }

        // Gauss-Jordan to RREF.
        let mut rows: Vec<IncRow> = Vec::new();
        let mut pivot_row: Vec<Option<usize>> = vec![None; num_vars];
        for mut row in staged {
            loop {
                let pc = set_bits(&row.bits)
                    .into_iter()
                    .find(|&p| pivot_row[p].is_some());
                match pc {
                    Some(p) => {
                        let b = rows[pivot_row[p].unwrap()].clone();
                        xor_into(&mut row.bits, &b.bits);
                        row.rhs ^= b.rhs;
                        xor_into(&mut row.prov, &b.prov);
                    }
                    None => break,
                }
            }
            if is_zero(&row.bits) {
                if row.rhs {
                    rows.push(row); // a permanent 0 = 1 conflict row.
                }
                continue;
            }
            let p = lowest_set(&row.bits).unwrap();
            let idx = rows.len();
            for b in rows.iter_mut() {
                if bit_get(&b.bits, p) {
                    xor_into(&mut b.bits, &row.bits);
                    b.rhs ^= row.rhs;
                    xor_into(&mut b.prov, &row.prov);
                }
            }
            pivot_row[p] = Some(idx);
            rows.push(row);
        }

        let mut occ = vec![Vec::new(); num_vars];
        let mut weight = vec![0u32; rows.len()];
        let mut low = Vec::new();
        let mut in_low = vec![false; rows.len()];
        for (ri, row) in rows.iter().enumerate() {
            for v in set_bits(&row.bits) {
                occ[v].push(ri);
            }
            weight[ri] = popcount(&row.bits);
            if weight[ri] <= 1 {
                low.push(ri);
                in_low[ri] = true;
            }
        }

        IncXor {
            num_vars,
            var_words,
            orig_vars,
            rows,
            pivot_row,
            occ,
            weight,
            low,
            in_low,
            value: vec![None; num_vars],
            applied: Vec::new(),
            undo: Vec::new(),
        }
    }

    /// Record that row `r`'s weight changed; queue it if it is now a unit/conflict candidate.
    #[inline]
    fn note_weight(&mut self, r: usize) {
        if self.weight[r] <= 1 && !self.in_low[r] {
            self.in_low[r] = true;
            self.low.push(r);
        }
    }

    pub fn is_active(&self) -> bool {
        !self.orig_vars.is_empty()
    }

    pub fn assigned_len(&self) -> usize {
        self.applied.len()
    }

    fn snap_row(frame: &mut Frame, rows: &[IncRow], i: usize) {
        if frame.touched_rows.insert(i) {
            frame.rows.push((i, rows[i].clone()));
        }
    }
    fn snap_pivot(frame: &mut Frame, pivot_row: &[Option<usize>], v: usize) {
        if frame.touched_pivots.insert(v) {
            frame.pivots.push((v, pivot_row[v]));
        }
    }

    /// Assign `var = val` (it must currently be free), updating the matrix incrementally.
    pub fn assign(&mut self, var: usize, val: bool) {
        let mut frame = Frame {
            var,
            rows: Vec::new(),
            pivots: Vec::new(),
            touched_rows: std::collections::HashSet::new(),
            touched_pivots: std::collections::HashSet::new(),
        };
        if let Some(r0) = self.pivot_row[var] {
            // `var` is a pivot: in RREF it occurs only in row r0. Substitute it out, then re-pivot.
            Self::snap_pivot(&mut frame, &self.pivot_row, var);
            Self::snap_row(&mut frame, &self.rows, r0);
            self.rows[r0].rhs ^= val;
            bit_clear(&mut self.rows[r0].bits, var);
            self.weight[r0] -= 1;
            self.note_weight(r0);
            occ_remove(&mut self.occ[var], r0);
            self.pivot_row[var] = None;
            self.re_pivot(r0, &mut frame);
        } else {
            // free non-pivot: clear it from every row that mentions it (pivots are untouched). The
            // occurrence index gives exactly those rows — `var` vanishes from the matrix, so its
            // list empties out.
            let rows_with_var = std::mem::take(&mut self.occ[var]);
            for i in rows_with_var {
                if bit_get(&self.rows[i].bits, var) {
                    Self::snap_row(&mut frame, &self.rows, i);
                    self.rows[i].rhs ^= val;
                    bit_clear(&mut self.rows[i].bits, var);
                    self.weight[i] -= 1;
                    self.note_weight(i);
                }
            }
        }
        self.value[var] = Some(val);
        self.applied.push(var);
        self.undo.push(frame);
    }

    /// XOR `pivot` into row `j`, keeping the occurrence index and row weight in lock-step: every
    /// variable the XOR toggles is added to / removed from its row list, and the weight tracks the
    /// net change. Returns the new weight of row `j`.
    fn xor_row(
        rows: &mut [IncRow],
        occ: &mut [Vec<usize>],
        weight: &mut [u32],
        j: usize,
        pivot: &IncRow,
    ) {
        let mut delta: i64 = 0;
        for b in set_bits(&pivot.bits) {
            if bit_get(&rows[j].bits, b) {
                occ_remove(&mut occ[b], j);
                delta -= 1;
            } else {
                occ[b].push(j);
                delta += 1;
            }
        }
        xor_into(&mut rows[j].bits, &pivot.bits);
        rows[j].rhs ^= pivot.rhs;
        xor_into(&mut rows[j].prov, &pivot.prov);
        weight[j] = (weight[j] as i64 + delta) as u32;
    }

    /// Re-establish a pivot for row `r0` (whose pivot was just substituted out) and keep RREF. Only
    /// the rows that actually contain the new pivot column `q` are reduced — the occurrence index
    /// hands them over directly instead of a full-matrix scan.
    fn re_pivot(&mut self, r0: usize, frame: &mut Frame) {
        let Some(q) = lowest_set(&self.rows[r0].bits) else {
            return; // empty row: 0=0 (redundant) or 0=1 (conflict, detected in `state`).
        };
        let pivot = self.rows[r0].clone();
        let targets = self.occ[q].clone();
        for j in targets {
            if j != r0 && bit_get(&self.rows[j].bits, q) {
                Self::snap_row(frame, &self.rows, j);
                Self::xor_row(&mut self.rows, &mut self.occ, &mut self.weight, j, &pivot);
                self.note_weight(j);
            }
        }
        Self::snap_pivot(frame, &self.pivot_row, q);
        self.pivot_row[q] = Some(r0);
    }

    /// Undo the most recent `assign`. Each touched row is rolled back to its snapshot, and the
    /// occurrence index is restored by diffing the row's current bits against that snapshot — every
    /// variable whose membership flipped is re-added or removed for that row.
    pub fn unassign(&mut self) {
        let Some(frame) = self.undo.pop() else { return };
        for (i, old) in frame.rows.into_iter().rev() {
            for w in 0..self.var_words {
                let mut diff = self.rows[i].bits[w] ^ old.bits[w];
                while diff != 0 {
                    let b = w * 64 + diff.trailing_zeros() as usize;
                    diff &= diff - 1;
                    if bit_get(&old.bits, b) {
                        self.occ[b].push(i); // present in the snapshot, absent now: re-add.
                    } else {
                        occ_remove(&mut self.occ[b], i); // absent in the snapshot, present now: drop.
                    }
                }
            }
            self.weight[i] = popcount(&old.bits);
            self.note_weight(i);
            self.rows[i] = old;
        }
        for (v, old) in frame.pivots.into_iter().rev() {
            self.pivot_row[v] = old;
        }
        self.value[frame.var] = None;
        self.applied.pop();
    }

    fn derived(&self, prov: &[u64]) -> Vec<u64> {
        let mut vars = vec![0u64; self.var_words];
        for i in set_bits(prov) {
            xor_into(&mut vars, &self.orig_vars[i]);
        }
        vars
    }

    fn gadget(&self, dv: &[u64], forced: Option<(usize, bool)>) -> Vec<Lit> {
        let mut clause = Vec::new();
        for u in set_bits(dv) {
            match forced {
                Some((fv, fval)) if fv == u => clause.push(Lit::new(u as u32, fval)),
                _ => clause.push(Lit::new(
                    u as u32,
                    !self.value[u].expect("gadget var must be assigned"),
                )),
            }
        }
        clause
    }

    /// Matrix density after the initial reduction: (rows, average row weight, max row weight). A
    /// dense reduced matrix (high average weight) is why eager Gauss-Jordan is slow and why the
    /// watch-based engine must avoid full densification.
    pub fn density(&self) -> (usize, f64, usize) {
        let weights: Vec<usize> = self
            .rows
            .iter()
            .map(|r| popcount(&r.bits) as usize)
            .collect();
        let n = weights.len().max(1);
        let sum: usize = weights.iter().sum();
        (
            weights.len(),
            sum as f64 / n as f64,
            weights.iter().copied().max().unwrap_or(0),
        )
    }

    /// The next variable to branch ("break") on: an unassigned **non-pivot** — a free/kernel degree
    /// of freedom the linear system does not determine. Deciding it lets Gaussian propagation force a
    /// fresh batch of pivots, collapsing the residual. Prefers a free variable that actually occurs in
    /// the matrix (a true kernel direction) over one the linear system never mentions.
    pub fn next_branch(&self) -> Option<usize> {
        let mut fallback = None;
        for v in 0..self.num_vars {
            if self.value[v].is_some() || self.pivot_row[v].is_some() {
                continue;
            }
            if self.rows.iter().any(|r| bit_get(&r.bits, v)) {
                return Some(v);
            }
            if fallback.is_none() {
                fallback = Some(v);
            }
        }
        fallback
    }

    /// The variables CDCL should branch on under DPLL(XOR): every NON-pivot variable. The pivots are
    /// determined by Gaussian propagation (the theory forces them), so the search only ranges over the
    /// kernel free variables (plus any variables the linear system never mentions).
    pub fn decision_vars(&self) -> Vec<usize> {
        (0..self.num_vars)
            .filter(|&v| self.pivot_row[v].is_none())
            .collect()
    }

    /// The short *implied* clauses the Gaussian reduction derives — every reduced row of width
    /// `1..=max_width` re-expressed as its CNF gadget. These are no-goods the linear system entails
    /// but resolution would not find (e.g. the 244 derived units of par32-1); injecting them into the
    /// CNF shares the XOR strategy's discovered structure with CDCL. Sound: each reduced row is a
    /// GF(2) sum of implied equations, so its gadget clauses are implied by the formula.
    pub fn derived_clauses(&self, max_width: usize) -> Vec<Vec<Lit>> {
        let mut out = Vec::new();
        for row in &self.rows {
            let vars = set_bits(&row.bits);
            let k = vars.len();
            if k == 0 || k > max_width || k > 31 {
                continue;
            }
            for mask in 0u32..(1u32 << k) {
                if ((mask.count_ones() % 2) == 1) != row.rhs {
                    out.push(
                        (0..k)
                            .map(|i| Lit::new(vars[i] as u32, (mask >> i) & 1 == 0))
                            .collect(),
                    );
                }
            }
        }
        out
    }

    /// The number of pivots (= rank of the system under the current assignment).
    pub fn rank(&self) -> usize {
        (0..self.num_vars)
            .filter(|&v| self.value[v].is_none() && self.pivot_row[v].is_some())
            .count()
    }

    /// Free/kernel variables that occur in the matrix — the dimension of the choice space (each is a
    /// candidate break point).
    pub fn kernel_dim(&self) -> usize {
        (0..self.num_vars)
            .filter(|&v| {
                self.value[v].is_none()
                    && self.pivot_row[v].is_none()
                    && self.rows.iter().any(|r| bit_get(&r.bits, v))
            })
            .count()
    }

    /// Assert the occurrence index is in perfect agreement with the matrix: `i ∈ occ[v]` exactly
    /// when row `i` contains variable `v`, with no duplicate or stale entries. The watch layer is an
    /// acceleration, so any drift here is a silent miscompile — this is the gate that catches it.
    #[cfg(test)]
    fn check_occ(&self) {
        use std::collections::BTreeSet;
        for v in 0..self.num_vars {
            let mut seen = BTreeSet::new();
            for &i in &self.occ[v] {
                assert!(seen.insert(i), "occ[{v}] has duplicate row {i}");
                assert!(
                    bit_get(&self.rows[i].bits, v),
                    "occ[{v}] lists row {i} which lacks {v}"
                );
            }
            for (i, row) in self.rows.iter().enumerate() {
                if bit_get(&row.bits, v) {
                    assert!(
                        seen.contains(&i),
                        "row {i} contains {v} but occ[{v}] omits it"
                    );
                }
            }
        }
        // The weight counter mirrors the true popcount, and `low` is a superset of every current
        // unit/conflict row (so `state` never misses one). `in_low` agrees with `low`'s membership.
        let in_low_set: std::collections::BTreeSet<usize> = self.low.iter().copied().collect();
        for (r, row) in self.rows.iter().enumerate() {
            assert_eq!(self.weight[r], popcount(&row.bits), "weight[{r}] desynced");
            if self.weight[r] <= 1 {
                assert!(self.in_low[r], "row {r} is weight ≤1 but in_low is false");
            }
            assert_eq!(
                self.in_low[r],
                in_low_set.contains(&r),
                "in_low[{r}] disagrees with low"
            );
        }
    }

    /// The current forced literals (with implied unit reasons), or `Err(conflict clause)`. Reads only
    /// the low-weight queue — the rows that became units/conflicts since the last call — and keeps the
    /// still-low ones for next time, so a propagation round is O(touched rows), not O(all rows).
    pub fn state(&mut self) -> Result<Vec<(Lit, Vec<Lit>)>, Vec<Lit>> {
        let mut forced = Vec::new();
        let candidates = std::mem::take(&mut self.low);
        let mut keep = Vec::with_capacity(candidates.len());
        let mut conflict: Option<Vec<Lit>> = None;
        for r in candidates {
            let w = self.weight[r];
            if w > 1 {
                self.in_low[r] = false; // no longer a unit/conflict candidate.
                continue;
            }
            keep.push(r); // still low — re-check it next round.
            if conflict.is_some() {
                continue;
            }
            match w {
                0 => {
                    if self.rows[r].rhs {
                        conflict = Some(self.gadget(&self.derived(&self.rows[r].prov), None));
                    }
                }
                _ => {
                    let v = lowest_set(&self.rows[r].bits).unwrap();
                    let rhs = self.rows[r].rhs;
                    forced.push((
                        Lit::new(v as u32, rhs),
                        self.gadget(&self.derived(&self.rows[r].prov), Some((v, rhs))),
                    ));
                }
            }
        }
        self.low = keep;
        match conflict {
            Some(c) => Err(c),
            None => Ok(forced),
        }
    }
}

/// DPLL(XOR): the engine plugs into CDCL's theory hook. At each Boolean fixpoint it hands back the
/// first XOR-forced unit clause, or a conflict clause, or nothing — every clause implied by the
/// formula, so the solver stays sound while gaining Gaussian reasoning resolution cannot do.
impl crate::cdcl::Theory for XorEngine {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        let mut a: Vec<Option<bool>> = vec![None; self.num_vars];
        for &l in trail {
            a[l.var() as usize] = Some(l.is_positive());
        }
        match self.all_consequences(&a) {
            Err(conflict) => vec![conflict],
            Ok(forced) => forced.into_iter().map(|(_, reason)| reason).collect(),
        }
    }
}

/// The fast path: [`IncXor`] syncs its incremental matrix to the solver's trail each call — undoing
/// the divergent suffix (backtrack) and applying the new literals (forward) — then returns the first
/// XOR-forced unit clause or a conflict. O(work-per-assignment), not O(system-per-fixpoint).
impl crate::cdcl::Theory for IncXor {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        // The longest prefix of the trail applied as it stands: each variable, with its value.
        let keep = self
            .applied
            .iter()
            .zip(trail)
            .take_while(|&(&v, l)| v == l.var() as usize && self.value[v] == Some(l.is_positive()))
            .count();
        self.sync(trail, keep)
    }

    fn propagate_kept(&mut self, trail: &[Lit], kept: usize) -> Vec<Vec<Lit>> {
        self.sync(trail, kept.min(self.applied.len()))
    }
}

impl IncXor {
    /// Bring the matrix to `trail`, the first `keep` literals of which are the ones applied, and return what it forces
    /// or refutes.
    fn sync(&mut self, trail: &[Lit], keep: usize) -> Vec<Vec<Lit>> {
        while self.applied.len() > keep {
            self.unassign();
        }
        for &l in &trail[keep..] {
            let v = l.var() as usize;
            if self.value[v].is_none() {
                self.assign(v, l.is_positive());
            }
        }
        // Batch every forced literal from one matrix pass (amortise the scan over the round).
        match self.state() {
            Err(conflict) => vec![conflict],
            Ok(forced) => forced.into_iter().map(|(_, reason)| reason).collect(),
        }
    }
}

/// The most cells (rows × columns) one component's matrix may hold; a larger component is left out, to its clauses.
const WATCHED_CELLS: usize = 1 << 26;

/// No row, column or watch.
const NONE: u32 = u32::MAX;

/// **Watched Gauss–Jordan elimination**: the XOR equations as a CDCL theory whose work per assignment is the rows the
/// assignment can affect (Han & Jiang, *When Boolean Satisfiability Meets Gaussian Elimination in a Simplex Way*, CAV
/// 2012; Soos, Gocht & Meel, *Tinted, Detached, and Lazy CNF-XOR Solving and Its Applications to Counting and
/// Sampling*, CAV 2020).
///
/// Each connected component of the equations is a bit matrix in reduced row echelon form: every row has a basic column
/// no other row holds, and watches one more column that is neither basic nor assigned. While a row watches such a
/// column it can neither propagate nor conflict, so an assignment visits only the rows watching its column and the one
/// row it is basic in:
///
/// - a watched column assigned: the row moves its watch to another unassigned non-basic column, or, finding none, has
///   only its basic column unassigned and propagates it;
/// - a basic column assigned: the row trades it for an unassigned non-basic column, which becomes basic once the row is
///   added into every other row holding it; with none to trade, the row is assigned throughout and is checked.
///
/// So a row holding an unassigned column has its basic column unassigned, and the matrix restricted to the unassigned
/// columns stays in echelon form: an assignment the equations force, or one they refute, is a single row's, and the
/// propagation is complete. Row operations are equivalences, so backtracking undoes no elimination: it unassigns, and a
/// row left without a watch watches again once the assignment that emptied it is undone. A row is a sum of the
/// equations, so each clause returned, a propagated basic variable with the current values of its row's other columns
/// or a refuted row's current values negated, is a clause of an implied equation's gadget: a consequence of the
/// formula the equations were read from. Every component within [`WATCHED_CELLS`] is held, a single equation too, so
/// over those equations the propagation is complete whether the formula keeps their clauses or not. A literal a row
/// forces can go to the search as the row's token, explained when asked: a row with no free column is never added
/// into nor pivots, so its clause stays the same while that literal is assigned.
pub struct WatchedXor {
    matrices: Vec<Matrix>,
    /// Per variable, its matrix and column, `(NONE, NONE)` when no matrix holds it.
    place: Vec<(u32, u32)>,
    /// Per variable, `1` true, `-1` false, `0` unassigned, as the trail last given has it.
    value: Vec<i8>,
    /// The trail last given.
    synced: Vec<Lit>,
    /// The rows left without a watch, in the order they lost it, each with the trail length at that point.
    unwatched: Vec<(usize, u32, u32)>,
    /// The rows holding their basic column alone: unit equations, reported by every call that leaves them unassigned.
    units: Vec<(u32, u32)>,
    /// Whether the equations sum to `0 = 1`: every call then returns the empty clause.
    inconsistent: bool,
    /// Per matrix, the token of its first row: a row's token is its matrix's base plus its index.
    row_base: Vec<u32>,
    /// Whether the call in progress hands forced literals out with tokens, and the ones it has.
    lazy: bool,
    forced: Vec<(Lit, u32)>,
    /// The calls so far, and those that ended in a refutation.
    calls: u64,
    refutations: u64,
}

/// What a row says under the current values.
enum Says {
    Nothing,
    /// Its one unassigned column, its basic one, is forced: the literal.
    Forces(Lit),
    /// It is assigned throughout and violated.
    Refutes,
}

/// One component's equations as a bit matrix.
struct Matrix {
    /// Column to variable.
    vars: Vec<Var>,
    words: usize,
    /// The rows, `words` words each.
    bits: Vec<u64>,
    rhs: Vec<bool>,
    /// Row to its basic column.
    basic: Vec<u32>,
    /// Column to the row it is basic in, or `NONE`.
    row_of: Vec<u32>,
    /// Row to its watched column, or `NONE`.
    watch: Vec<u32>,
    /// Column to the rows watching it.
    watchers: Vec<Vec<u32>>,
    /// Row to its place among its watched column's watchers.
    slot: Vec<u32>,
    /// The assigned columns, and those assigned true.
    assigned: Vec<u64>,
    ones: Vec<u64>,
    /// The basic columns.
    basic_mask: Vec<u64>,
}

impl Matrix {
    fn row(&self, r: usize) -> &[u64] {
        &self.bits[r * self.words..(r + 1) * self.words]
    }

    fn holds(&self, r: usize, c: u32) -> bool {
        bit_get(self.row(r), c as usize)
    }

    /// A column of row `r` neither basic nor assigned, other than `except`, looked for from the word holding `from`
    /// round to the one before it: a watch moving on finds its next column where it left off.
    fn free_column(&self, r: usize, from: u32, except: u32) -> Option<u32> {
        let row = self.row(r);
        let first = if from == NONE { 0 } else { from as usize / 64 };
        for w in (first..self.words).chain(0..first) {
            let mut free = row[w] & !self.assigned[w] & !self.basic_mask[w];
            while free != 0 {
                let c = (w * 64) as u32 + free.trailing_zeros();
                if c != except {
                    return Some(c);
                }
                free &= free - 1;
            }
        }
        None
    }

    /// Add row `from` into row `into`.
    fn add_row(&mut self, from: usize, into: usize) {
        let (a, b) = (from * self.words, into * self.words);
        for w in 0..self.words {
            self.bits[b + w] ^= self.bits[a + w];
        }
        self.rhs[into] ^= self.rhs[from];
    }

    fn watch_column(&mut self, r: usize, c: u32) {
        self.watch[r] = c;
        self.slot[r] = self.watchers[c as usize].len() as u32;
        self.watchers[c as usize].push(r as u32);
    }

    fn unwatch(&mut self, r: usize) {
        let list = &mut self.watchers[self.watch[r] as usize];
        let at = self.slot[r] as usize;
        list.swap_remove(at);
        if let Some(&moved) = list.get(at) {
            self.slot[moved as usize] = at as u32;
        }
        self.watch[r] = NONE;
    }
}

impl WatchedXor {
    /// The theory of `equations` over `num_vars` variables, each connected component reduced once. An equation
    /// naming a variable past `num_vars` is left out, and so is a component past [`WATCHED_CELLS`].
    pub fn new(num_vars: usize, equations: &[XorEquation]) -> WatchedXor {
        let mut inconsistent = false;
        let mut equations_kept: Vec<(Vec<u32>, bool)> = Vec::new();
        for equation in equations {
            if equation.vars.iter().any(|&v| v >= num_vars) {
                continue;
            }
            let mut sorted: Vec<u32> = equation.vars.iter().map(|&v| v as u32).collect();
            sorted.sort_unstable();
            let mut vars: Vec<u32> = Vec::with_capacity(sorted.len());
            for v in sorted {
                if vars.last() == Some(&v) {
                    vars.pop();
                } else {
                    vars.push(v);
                }
            }
            if vars.is_empty() {
                inconsistent |= equation.rhs;
            } else {
                equations_kept.push((vars, equation.rhs));
            }
        }
        let mut parent: Vec<u32> = (0..num_vars as u32).collect();
        fn root(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for (vars, _) in &equations_kept {
            for pair in vars.windows(2) {
                let (a, b) = (root(&mut parent, pair[0]), root(&mut parent, pair[1]));
                if a != b {
                    parent[a as usize] = b;
                }
            }
        }
        let mut components: std::collections::BTreeMap<u32, Vec<usize>> = std::collections::BTreeMap::new();
        for (i, (vars, _)) in equations_kept.iter().enumerate() {
            components.entry(root(&mut parent, vars[0])).or_default().push(i);
        }
        let mut place = vec![(NONE, NONE); num_vars];
        let mut matrices: Vec<Matrix> = Vec::new();
        for members in components.values() {
            let mut vars: Vec<u32> = members.iter().flat_map(|&i| equations_kept[i].0.iter().copied()).collect();
            vars.sort_unstable();
            vars.dedup();
            if vars.len() * members.len() > WATCHED_CELLS {
                continue;
            }
            let words = words_for(vars.len());
            let mut bits = vec![0u64; members.len() * words];
            let mut rhs = Vec::with_capacity(members.len());
            for (r, &i) in members.iter().enumerate() {
                for v in &equations_kept[i].0 {
                    let c = vars.binary_search(v).expect("a component's variable is one of its columns");
                    bit_set(&mut bits[r * words..(r + 1) * words], c);
                }
                rhs.push(equations_kept[i].1);
            }
            let mut matrix = Matrix {
                vars,
                words,
                bits,
                rhs,
                basic: Vec::new(),
                row_of: Vec::new(),
                watch: Vec::new(),
                watchers: Vec::new(),
                slot: Vec::new(),
                assigned: vec![0; words],
                ones: vec![0; words],
                basic_mask: vec![0; words],
            };
            let (rows, columns) = (members.len(), matrix.vars.len());
            matrix.row_of = vec![NONE; columns];
            for c in 0..columns {
                let rank = matrix.basic.len();
                let Some(p) = (rank..rows).find(|&r| bit_get(matrix.row(r), c)) else { continue };
                if p != rank {
                    for w in 0..words {
                        matrix.bits.swap(p * words + w, rank * words + w);
                    }
                    matrix.rhs.swap(p, rank);
                }
                for r in 0..rows {
                    if r != rank && bit_get(matrix.row(r), c) {
                        matrix.add_row(rank, r);
                    }
                }
                matrix.basic.push(c as u32);
                matrix.row_of[c] = rank as u32;
                bit_set(&mut matrix.basic_mask, c);
            }
            let rank = matrix.basic.len();
            // The rows past the rank are empty: `0 = 1` among them refutes the equations, `0 = 0` says nothing.
            inconsistent |= matrix.rhs[rank..].iter().any(|&b| b);
            if rank == 0 {
                continue;
            }
            matrix.bits.truncate(rank * words);
            matrix.rhs.truncate(rank);
            matrix.watch = vec![NONE; rank];
            matrix.slot = vec![0; rank];
            matrix.watchers = vec![Vec::new(); columns];
            for (c, &v) in matrix.vars.iter().enumerate() {
                place[v as usize] = (matrices.len() as u32, c as u32);
            }
            matrices.push(matrix);
        }
        let mut row_base = Vec::with_capacity(matrices.len());
        let mut rows = 0u32;
        for matrix in &matrices {
            row_base.push(rows);
            rows += matrix.basic.len() as u32;
        }
        let mut units = Vec::new();
        for (m, matrix) in matrices.iter_mut().enumerate() {
            for r in 0..matrix.basic.len() {
                match matrix.free_column(r, NONE, NONE) {
                    Some(c) => matrix.watch_column(r, c),
                    None => units.push((m as u32, r as u32)),
                }
            }
        }
        WatchedXor {
            matrices,
            place,
            value: vec![0; num_vars],
            synced: Vec::new(),
            unwatched: Vec::new(),
            units,
            inconsistent,
            row_base,
            lazy: false,
            forced: Vec::new(),
            calls: 0,
            refutations: 0,
        }
    }

    /// The calls the theory has answered.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// The calls that ended in a refutation: a row assigned throughout and violated, or the equations inconsistent.
    pub fn refutations(&self) -> u64 {
        self.refutations
    }

    /// Whether the theory holds a matrix, or knows the equations inconsistent.
    pub fn is_active(&self) -> bool {
        self.inconsistent || !self.matrices.is_empty()
    }

    /// The variables no matrix holds as a basic column, before any assignment: once they are all assigned, the
    /// theory's propagation assigns the rest.
    pub fn decision_vars(&self) -> Vec<usize> {
        let mut basic = vec![false; self.place.len()];
        for matrix in &self.matrices {
            for &c in &matrix.basic {
                basic[matrix.vars[c as usize] as usize] = true;
            }
        }
        (0..self.place.len()).filter(|&v| !basic[v]).collect()
    }

    /// Bring the matrices to `trail`, the first `keep` literals of which are the ones last given, and return what
    /// the assignments past them force or refute.
    fn sync(&mut self, trail: &[Lit], keep: usize) -> Vec<Vec<Lit>> {
        debug_assert!(self.synced[..keep] == trail[..keep], "the kept literals are the ones last given");
        while self.synced.len() > keep {
            let l = self.synced.pop().expect("a literal past the kept ones");
            self.unassign(l);
        }
        while let Some(&(at, m, r)) = self.unwatched.last() {
            if at <= keep {
                break;
            }
            self.unwatched.pop();
            let matrix = &mut self.matrices[m as usize];
            let c = matrix
                .free_column(r as usize, NONE, NONE)
                .expect("undoing the assignment that emptied a row frees one of its columns");
            matrix.watch_column(r as usize, c);
        }
        let mut out = Vec::new();
        for &l in &trail[keep..] {
            self.synced.push(l);
            self.assign(l, &mut out);
        }
        for i in 0..self.units.len() {
            let (m, r) = (self.units[i].0 as usize, self.units[i].1 as usize);
            let matrix = &self.matrices[m];
            if self.value[matrix.vars[matrix.basic[r] as usize] as usize] == 0 {
                self.check(m, r, &mut out);
            }
        }
        self.calls += 1;
        if self.inconsistent {
            self.refutations += 1;
            self.forced.clear();
            return vec![Vec::new()];
        }
        if out.iter().any(|clause| clause.iter().all(|&l| self.value[l.var() as usize] == if l.is_positive() { -1 } else { 1 })) {
            self.refutations += 1;
        }
        out
    }

    /// Apply `l`, the last literal synced.
    fn assign(&mut self, l: Lit, out: &mut Vec<Vec<Lit>>) {
        let v = l.var() as usize;
        self.value[v] = if l.is_positive() { 1 } else { -1 };
        let (m, c) = self.place[v];
        if m == NONE {
            return;
        }
        let m = m as usize;
        bit_set(&mut self.matrices[m].assigned, c as usize);
        if l.is_positive() {
            bit_set(&mut self.matrices[m].ones, c as usize);
        }
        let r = self.matrices[m].row_of[c as usize];
        if r != NONE {
            let r = r as usize;
            // Trade for a free column other than the watch, so the row keeps it; else for the watch itself.
            let watched = self.matrices[m].watch[r];
            if watched == NONE {
                self.check(m, r, out);
            } else {
                let w = self.matrices[m].free_column(r, watched, watched).unwrap_or(watched);
                self.pivot(m, r, c, w, out);
            }
            return;
        }
        for r in std::mem::take(&mut self.matrices[m].watchers[c as usize]) {
            let r = r as usize;
            match self.matrices[m].free_column(r, c, NONE) {
                Some(w) => self.matrices[m].watch_column(r, w),
                None => {
                    self.matrices[m].watch[r] = NONE;
                    self.unwatched.push((self.synced.len(), m as u32, r as u32));
                    self.check(m, r, out);
                }
            }
        }
    }

    /// Make the free column `w` of row `r` its basic column in place of the assigned `c`: row `r` is added into every
    /// other row holding `w`, and a row the addition took its watch from watches again or, with nothing left to
    /// watch, propagates.
    fn pivot(&mut self, m: usize, r: usize, c: u32, w: u32, out: &mut Vec<Vec<Lit>>) {
        let at = self.synced.len();
        for other in 0..self.matrices[m].basic.len() {
            if other == r || !self.matrices[m].holds(other, w) {
                continue;
            }
            self.matrices[m].add_row(r, other);
            let watched = self.matrices[m].watch[other];
            debug_assert!(watched != NONE, "a row holding a free column watches one");
            if self.matrices[m].holds(other, watched) {
                continue;
            }
            self.matrices[m].unwatch(other);
            match self.matrices[m].free_column(other, watched, NONE) {
                Some(x) => self.matrices[m].watch_column(other, x),
                None => {
                    self.unwatched.push((at, m as u32, other as u32));
                    self.check(m, other, out);
                }
            }
        }
        let matrix = &mut self.matrices[m];
        matrix.basic[r] = w;
        matrix.row_of[c as usize] = NONE;
        matrix.row_of[w as usize] = r as u32;
        bit_clear(&mut matrix.basic_mask, c as usize);
        bit_set(&mut matrix.basic_mask, w as usize);
        if matrix.watch[r] == w {
            matrix.unwatch(r);
            match matrix.free_column(r, w, NONE) {
                Some(x) => matrix.watch_column(r, x),
                None => {
                    self.unwatched.push((at, m as u32, r as u32));
                    self.check(m, r, out);
                }
            }
        }
    }

    fn unassign(&mut self, l: Lit) {
        let v = l.var() as usize;
        self.value[v] = 0;
        let (m, c) = self.place[v];
        if m != NONE {
            bit_clear(&mut self.matrices[m as usize].assigned, c as usize);
            bit_clear(&mut self.matrices[m as usize].ones, c as usize);
        }
    }

    /// What row `r` of matrix `m` says: read word by word, its unassigned columns and the parity of its true ones.
    fn says(&self, m: usize, r: usize) -> Says {
        let matrix = &self.matrices[m];
        let row = matrix.row(r);
        let (mut open, mut free, mut odd) = (0u32, 0usize, false);
        for w in 0..matrix.words {
            let unassigned = row[w] & !matrix.assigned[w];
            if unassigned != 0 {
                open += unassigned.count_ones();
                if open > 1 {
                    return Says::Nothing;
                }
                free = w * 64 + unassigned.trailing_zeros() as usize;
            }
            odd ^= (row[w] & matrix.ones[w]).count_ones() % 2 == 1;
        }
        let parity = matrix.rhs[r] ^ odd;
        match open {
            1 => {
                debug_assert_eq!(free as u32, matrix.basic[r], "a row forces its basic column");
                Says::Forces(Lit::new(matrix.vars[free], parity))
            }
            _ if parity => Says::Refutes,
            _ => Says::Nothing,
        }
    }

    /// Report what row `r` of matrix `m` says: a forced literal by its token when the call is lazy, else by its
    /// clause; a refutation by its clause.
    fn check(&mut self, m: usize, r: usize, out: &mut Vec<Vec<Lit>>) {
        match self.says(m, r) {
            Says::Nothing => {}
            Says::Forces(lit) if self.lazy => self.forced.push((lit, self.row_base[m] + r as u32)),
            Says::Forces(_) | Says::Refutes => {
                let mut clause = Vec::new();
                self.explain_row(m, r, &mut clause);
                out.push(clause);
            }
        }
    }

    /// Row `r`'s clause under the current values, onto `clause`: its basic column's literal as the other columns force
    /// it, then the literals of the others, each false. For a refuted row every literal is false; for a row that
    /// forced its basic column it is the forcing clause, unchanged while that literal stays assigned, since a row
    /// with no free column is never added into nor pivots.
    fn explain_row(&self, m: usize, r: usize, clause: &mut Vec<Lit>) {
        let matrix = &self.matrices[m];
        let b = matrix.basic[r] as usize;
        let mut forced = matrix.rhs[r];
        clause.push(Lit::pos(0));
        let first = clause.len() - 1;
        for (w, &word) in matrix.row(r).iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let c = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if c == b {
                    continue;
                }
                let v = matrix.vars[c];
                let value = self.value[v as usize];
                debug_assert!(value != 0, "a row explains itself only once its other columns are assigned");
                forced ^= value > 0;
                clause.push(Lit::new(v, value < 0));
            }
        }
        clause[first] = Lit::new(matrix.vars[b], forced);
    }
}

/// The watched matrix as a CDCL theory: each call syncs the matrices to the trail, undoing only past the literals kept
/// since the last call, and returns every clause the new assignments make unit or false — or, called lazily, every
/// literal they force by its row, explained when asked.
impl crate::cdcl::Theory for WatchedXor {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        let keep = self.synced.iter().zip(trail).take_while(|(a, b)| a == b).count();
        self.sync(trail, keep)
    }

    fn propagate_kept(&mut self, trail: &[Lit], kept: usize) -> Vec<Vec<Lit>> {
        self.sync(trail, kept.min(self.synced.len()))
    }

    fn propagate_lazily(&mut self, trail: &[Lit], kept: usize, forced: &mut Vec<(Lit, u32)>) -> Vec<Vec<Lit>> {
        self.lazy = true;
        let refuted = self.sync(trail, kept.min(self.synced.len()));
        self.lazy = false;
        forced.append(&mut self.forced);
        refuted
    }

    fn explain(&mut self, token: u32, clause: &mut Vec<Lit>) {
        let m = self.row_base.partition_point(|&base| base <= token) - 1;
        self.explain_row(m, (token - self.row_base[m]) as usize, clause);
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/xor_engine/tests.rs"]
mod tests;
