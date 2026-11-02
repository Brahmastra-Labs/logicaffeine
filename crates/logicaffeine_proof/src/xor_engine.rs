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
//! clause is implied and correctly unit/falsified). The incremental watched-matrix that makes it
//! cheap per call is layered on top of this oracle.

use crate::cdcl::Lit;
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
    /// variable out of range are dropped (defensive; callers pass in-range systems).
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
                bit_set(&mut vars, v);
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
    orig_rhs: Vec<bool>,
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
    /// Build from recovered equations, reducing the matrix to RREF once.
    pub fn new(num_vars: usize, eqs: &[XorEquation]) -> Self {
        let var_words = words_for(num_vars);
        let eq_words = words_for(eqs.len());
        let mut orig_vars = Vec::new();
        let mut orig_rhs = Vec::new();
        let mut staged: Vec<IncRow> = Vec::new();
        for e in eqs {
            let mut bits = vec![0u64; var_words];
            let mut ok = true;
            for &v in &e.vars {
                if v >= num_vars {
                    ok = false;
                    break;
                }
                bit_set(&mut bits, v);
            }
            if !ok {
                continue;
            }
            let idx = orig_vars.len();
            let mut prov = vec![0u64; eq_words];
            bit_set(&mut prov, idx);
            orig_vars.push(bits.clone());
            orig_rhs.push(e.rhs);
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
            orig_rhs,
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
        // longest common prefix of what we've applied and the current trail.
        let mut cp = 0;
        while cp < self.applied.len()
            && cp < trail.len()
            && self.applied[cp] == trail[cp].var() as usize
        {
            cp += 1;
        }
        while self.applied.len() > cp {
            self.unassign();
        }
        let start = self.applied.len();
        for &l in &trail[start..] {
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

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/xor_engine/tests.rs"]
mod tests;
