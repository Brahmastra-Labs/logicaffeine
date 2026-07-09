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
        XorEngine { num_vars, var_words, eq_words, orig_vars, orig_rhs }
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
    pub fn all_consequences(&self, assign: &[Option<bool>]) -> Result<Vec<(Lit, Vec<Lit>)>, Vec<Lit>> {
        // Reduce each equation by the current assignment: move assigned variables to the rhs, keeping
        // only the unassigned variables — and remember the provenance so we can rebuild E*.
        let mut rows: Vec<Row> = Vec::with_capacity(self.orig_vars.len());
        for (i, ov) in self.orig_vars.iter().enumerate() {
            let mut vars = vec![0u64; self.var_words];
            let mut rhs = self.orig_rhs[i];
            for v in set_bits(ov) {
                match assign[v] {
                    Some(val) => rhs ^= val,        // assigned: fold into the parity
                    None => bit_set(&mut vars, v),  // unassigned: stays a live coefficient
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
                let pivot_col = set_bits(&row.vars).into_iter().find(|&p| pivot_at[p].is_some());
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
            staged.push(IncRow { bits, rhs: e.rhs, prov });
        }

        // Gauss-Jordan to RREF.
        let mut rows: Vec<IncRow> = Vec::new();
        let mut pivot_row: Vec<Option<usize>> = vec![None; num_vars];
        for mut row in staged {
            loop {
                let pc = set_bits(&row.bits).into_iter().find(|&p| pivot_row[p].is_some());
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
    fn xor_row(rows: &mut [IncRow], occ: &mut [Vec<usize>], weight: &mut [u32], j: usize, pivot: &IncRow) {
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
                _ => clause.push(Lit::new(u as u32, !self.value[u].expect("gadget var must be assigned"))),
            }
        }
        clause
    }

    /// Matrix density after the initial reduction: (rows, average row weight, max row weight). A
    /// dense reduced matrix (high average weight) is why eager Gauss-Jordan is slow and why the
    /// watch-based engine must avoid full densification.
    pub fn density(&self) -> (usize, f64, usize) {
        let weights: Vec<usize> = self.rows.iter().map(|r| popcount(&r.bits) as usize).collect();
        let n = weights.len().max(1);
        let sum: usize = weights.iter().sum();
        (weights.len(), sum as f64 / n as f64, weights.iter().copied().max().unwrap_or(0))
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
        (0..self.num_vars).filter(|&v| self.pivot_row[v].is_none()).collect()
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
                    out.push((0..k).map(|i| Lit::new(vars[i] as u32, (mask >> i) & 1 == 0)).collect());
                }
            }
        }
        out
    }

    /// The number of pivots (= rank of the system under the current assignment).
    pub fn rank(&self) -> usize {
        (0..self.num_vars).filter(|&v| self.value[v].is_none() && self.pivot_row[v].is_some()).count()
    }

    /// Free/kernel variables that occur in the matrix — the dimension of the choice space (each is a
    /// candidate break point).
    pub fn kernel_dim(&self) -> usize {
        (0..self.num_vars)
            .filter(|&v| self.value[v].is_none() && self.pivot_row[v].is_none() && self.rows.iter().any(|r| bit_get(&r.bits, v)))
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
                assert!(bit_get(&self.rows[i].bits, v), "occ[{v}] lists row {i} which lacks {v}");
            }
            for (i, row) in self.rows.iter().enumerate() {
                if bit_get(&row.bits, v) {
                    assert!(seen.contains(&i), "row {i} contains {v} but occ[{v}] omits it");
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
            assert_eq!(self.in_low[r], in_low_set.contains(&r), "in_low[{r}] disagrees with low");
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
                    forced.push((Lit::new(v as u32, rhs), self.gadget(&self.derived(&self.rows[r].prov), Some((v, rhs)))));
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
        while cp < self.applied.len() && cp < trail.len() && self.applied[cp] == trail[cp].var() as usize {
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
mod tests {
    use super::*;
    use crate::xorsat::XorEquation;

    /// Brute-force oracle: enumerate every assignment of the unassigned variables consistent with
    /// `assign` that satisfies all equations. Returns `None` if there are none (UNSAT under `assign`),
    /// else `Some(per-variable forced value)` where a variable is forced iff it takes the same value
    /// across every consistent completion.
    fn oracle(num_vars: usize, eqs: &[XorEquation], assign: &[Option<bool>]) -> Option<Vec<Option<bool>>> {
        let free: Vec<usize> = (0..num_vars).filter(|&v| assign[v].is_none()).collect();
        let mut seen: Vec<Option<bool>> = vec![None; num_vars];
        let mut first = true;
        let mut any = false;
        assert!(free.len() <= 16, "oracle only for small systems");
        for mask in 0u32..(1u32 << free.len()) {
            let mut full = vec![false; num_vars];
            for v in 0..num_vars {
                full[v] = assign[v].unwrap_or(false);
            }
            for (i, &v) in free.iter().enumerate() {
                full[v] = (mask >> i) & 1 == 1;
            }
            let ok = eqs.iter().all(|e| e.vars.iter().fold(false, |a, &v| a ^ full[v]) == e.rhs);
            if !ok {
                continue;
            }
            any = true;
            if first {
                for v in 0..num_vars {
                    seen[v] = Some(full[v]);
                }
                first = false;
            } else {
                for v in 0..num_vars {
                    if seen[v] != Some(full[v]) {
                        seen[v] = None; // varies ⇒ not forced
                    }
                }
            }
        }
        if any {
            // Only the unassigned variables count as "forced" discoveries.
            for v in 0..num_vars {
                if assign[v].is_some() {
                    seen[v] = None;
                }
            }
            Some(seen)
        } else {
            None
        }
    }

    fn lit_sat(clause: &[Lit], full: &[bool]) -> bool {
        clause.iter().any(|l| full[l.var() as usize] == l.is_positive())
    }

    #[test]
    fn forces_a_variable_by_combining_two_equations() {
        // x0⊕x1=1, x0⊕x2=1 ⇒ x1⊕x2=0 (cross-equation). Assign x1=true ⇒ x2 forced true; x0 false too.
        let eqs = vec![XorEquation::new(vec![0, 1], true), XorEquation::new(vec![0, 2], true)];
        let eng = XorEngine::new(3, &eqs);
        let assign = vec![None, Some(true), None];
        let forced = eng.all_consequences(&assign).expect("system is consistent");
        let (x2, reason) = forced.iter().find(|(l, _)| l.var() == 2).expect("x2 must be forced");
        assert!(x2.is_positive(), "x2 must be forced true");
        // the reason must be unit on x2 (its only unassigned literal).
        let unassigned: Vec<_> = reason.iter().filter(|l| assign[l.var() as usize].is_none()).collect();
        assert_eq!(unassigned.len(), 1);
        assert_eq!(unassigned[0].var(), 2);
    }

    #[test]
    fn dpll_xor_decides_a_small_mixed_instance_via_the_theory() {
        use crate::cdcl::{SolveResult, Solver};
        // CNF = the 4 gadget clauses of x0⊕x1⊕x2=0, plus units x0 and x1 ⇒ SAT with x2=false.
        let gadget = vec![
            vec![Lit::new(0, true), Lit::new(1, true), Lit::new(2, false)],
            vec![Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, true), Lit::new(2, true)],
            vec![Lit::new(0, false), Lit::new(1, false), Lit::new(2, false)],
            vec![Lit::new(0, true)],
            vec![Lit::new(1, true)],
        ];
        let mut solver = Solver::new(3);
        for c in &gadget {
            solver.add_clause(c.clone());
        }
        let engine = XorEngine::new(3, &[XorEquation::new(vec![0, 1, 2], false)]);
        let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(engine)];
        match solver.solve_with(&mut theories) {
            SolveResult::Sat(m) => {
                for c in &gadget {
                    assert!(c.iter().any(|l| m[l.var() as usize] == l.is_positive()), "model fails {c:?}");
                }
                assert!(m[0] && m[1] && !m[2], "expected x0,x1 true and x2 false");
            }
            SolveResult::Unsat => panic!("instance is SAT"),
        }
    }

    #[test]
    fn live_dpll_xor_matches_plain_cdcl_on_random_xor_formulas() {
        use crate::cdcl::{SolveResult, Solver};
        // The decisive integration gate: running the live IncXor as a CDCL theory must give the SAME
        // verdict as plain CDCL on the identical formula — for hundreds of random instances built
        // from real XOR structure plus residual clauses. The theory may only *carry* globally-valid
        // no-goods (every XOR consequence holds in every model), so it can never change the answer,
        // only the path. On SAT the returned model must satisfy every clause.
        let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        // Expand a GF(2) equation to its CNF gadget clauses (each globally valid).
        let gadget_of = |e: &XorEquation, _nv: usize| -> Vec<Vec<Lit>> {
            let vs = &e.vars;
            let k = vs.len();
            let mut out = Vec::new();
            for mask in 0u32..(1u32 << k) {
                if ((mask.count_ones() % 2) == 1) != e.rhs {
                    out.push((0..k).map(|i| Lit::new(vs[i] as u32, (mask >> i) & 1 == 0)).collect());
                }
            }
            out
        };
        for _ in 0..400 {
            let num_vars = 4 + (next() % 7) as usize; // 4..=10
            let n_eqs = 1 + (next() % 4) as usize;
            let mut eqs = Vec::new();
            let mut clauses: Vec<Vec<Lit>> = Vec::new();
            for _ in 0..n_eqs {
                let mut vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
                vars.truncate(4); // keep gadgets small
                if vars.is_empty() {
                    continue;
                }
                let e = XorEquation::new(vars, next() % 2 == 0);
                clauses.extend(gadget_of(&e, num_vars));
                eqs.push(e);
            }
            if eqs.is_empty() {
                continue;
            }
            // A few residual clauses (units/binaries) to bias toward the hard SAT/UNSAT boundary.
            for _ in 0..(2 + next() % 4) {
                let w = 1 + (next() % 2) as usize;
                let c: Vec<Lit> = (0..w)
                    .map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0))
                    .collect();
                clauses.push(c);
            }

            let mut plain = Solver::new(num_vars);
            for c in &clauses {
                plain.add_clause(c.clone());
            }
            let truth = plain.solve();

            let mut live = Solver::new(num_vars);
            for c in &clauses {
                live.add_clause(c.clone());
            }
            let engine = IncXor::new(num_vars, &eqs);
            let mut theories: Vec<Box<dyn crate::cdcl::Theory>> = vec![Box::new(engine)];
            let got = live.solve_with(&mut theories);

            match (&truth, &got) {
                (SolveResult::Sat(_), SolveResult::Sat(m)) => {
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
                        "live model fails a clause (eqs={eqs:?})"
                    );
                }
                (SolveResult::Unsat, SolveResult::Unsat) => {}
                _ => panic!("verdict mismatch: plain={truth:?} live={got:?} (eqs={eqs:?} clauses={clauses:?})"),
            }
        }
    }

    #[test]
    #[ignore = "diagnostic: reads par32 from disk and prints the GF(2) search dimensions"]
    fn par32_search_dimensions() {
        for name in ["par16-1-c", "par32-1-c", "par32-1"] {
            let path = format!("../../benchmarks/arena/instances/sat/crafted_parity/{name}.cnf");
            let Ok(text) = std::fs::read_to_string(&path) else {
                eprintln!("{name}: missing");
                continue;
            };
            let cnf = crate::dimacs::parse(&text).expect("valid dimacs");
            let eqs = crate::lyapunov::extract_xor(cnf.num_vars, &cnf.clauses);
            let engine = IncXor::new(cnf.num_vars, &eqs);
            let (rows, avgw, maxw) = engine.density();
            eprintln!(
                "{name}: vars={} clauses={} xor_eqs={} | rank={} kernel_dim={} decision_vars={} | rows={} avg_w={:.1} max_w={}",
                cnf.num_vars,
                cnf.clauses.len(),
                eqs.len(),
                engine.rank(),
                engine.kernel_dim(),
                engine.decision_vars().len(),
                rows,
                avgw,
                maxw,
            );
        }
    }

    #[test]
    #[ignore = "diagnostic: measures how far probing collapses par32's GF(2) kernel"]
    fn par32_probing_collapses_kernel() {
        use crate::xorsat::XorEquation;
        // Full propagation under a single assumed literal: residual-clause BCP interleaved with live
        // Gaussian, to a fixpoint. Returns the forced assignment, or None on conflict (a failed literal).
        fn probe(num_vars: usize, clauses: &[Vec<Lit>], eqs: &[XorEquation], start: Lit) -> Option<Vec<Option<bool>>> {
            let mut val: Vec<Option<bool>> = vec![None; num_vars];
            let mut in_engine = vec![false; num_vars];
            let mut engine = IncXor::new(num_vars, eqs);
            val[start.var() as usize] = Some(start.is_positive());
            let mut progress = true;
            while progress {
                progress = false;
                for v in 0..num_vars {
                    if val[v].is_some() && !in_engine[v] {
                        engine.assign(v, val[v].unwrap());
                        in_engine[v] = true;
                    }
                }
                match engine.state() {
                    Err(_) => return None,
                    Ok(forced) => {
                        for (lit, _) in forced {
                            let v = lit.var() as usize;
                            match val[v] {
                                None => {
                                    val[v] = Some(lit.is_positive());
                                    progress = true;
                                }
                                Some(b) => {
                                    if b != lit.is_positive() {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                }
                for c in clauses {
                    let mut sat = false;
                    let mut unset: Option<Lit> = None;
                    let mut n_unset = 0;
                    for &l in c {
                        match val[l.var() as usize] {
                            Some(b) if b == l.is_positive() => {
                                sat = true;
                                break;
                            }
                            Some(_) => {}
                            None => {
                                n_unset += 1;
                                unset = Some(l);
                            }
                        }
                    }
                    if sat {
                        continue;
                    }
                    if n_unset == 0 {
                        return None;
                    }
                    if n_unset == 1 {
                        let l = unset.unwrap();
                        if val[l.var() as usize].is_none() {
                            val[l.var() as usize] = Some(l.is_positive());
                            progress = true;
                        }
                    }
                }
            }
            Some(val)
        }

        for name in ["par16-1-c", "par32-1-c"] {
            let path = format!("../../benchmarks/arena/instances/sat/crafted_parity/{name}.cnf");
            let Ok(text) = std::fs::read_to_string(&path) else {
                eprintln!("{name}: missing");
                continue;
            };
            let cnf = crate::dimacs::parse(&text).expect("valid dimacs");
            let eqs = crate::lyapunov::extract_xor(cnf.num_vars, &cnf.clauses);
            let base = IncXor::new(cnf.num_vars, &eqs);
            let kernel = base.decision_vars();
            let kernel0 = base.kernel_dim();

            let mut folded = eqs.clone();
            let (mut failed, mut backbone, mut equiv) = (0usize, 0usize, 0usize);
            for &x in &kernel {
                let f1 = probe(cnf.num_vars, &cnf.clauses, &eqs, Lit::new(x as u32, true));
                let f0 = probe(cnf.num_vars, &cnf.clauses, &eqs, Lit::new(x as u32, false));
                match (&f1, &f0) {
                    (None, None) => {} // both polarities conflict ⇒ the whole instance is UNSAT-under-XOR
                    (None, Some(_)) => {
                        failed += 1;
                        folded.push(XorEquation::new(vec![x], false));
                    }
                    (Some(_), None) => {
                        failed += 1;
                        folded.push(XorEquation::new(vec![x], true));
                    }
                    (Some(a1), Some(a0)) => {
                        for y in 0..cnf.num_vars {
                            if y == x {
                                continue;
                            }
                            if let (Some(v1), Some(v0)) = (a1[y], a0[y]) {
                                if v1 == v0 {
                                    backbone += 1;
                                    folded.push(XorEquation::new(vec![y], v1));
                                } else {
                                    equiv += 1;
                                    folded.push(XorEquation::new(vec![x, y], v0)); // x=F⇒y=v0 ⇒ x⊕y=v0
                                }
                            }
                        }
                    }
                }
            }
            let after = IncXor::new(cnf.num_vars, &folded);
            eprintln!(
                "{name}: kernel {} → {} | probes found failed_lits={} backbone_hits={} equiv_pairs={} (raw counts, pre-dedup)",
                kernel0,
                after.kernel_dim(),
                failed,
                backbone,
                equiv,
            );
        }
    }

    #[test]
    fn detects_a_linear_contradiction() {
        // x0⊕x1=0, x1⊕x2=0, x0⊕x2=1 sum to 0=1: UNSAT with no assignment. The certificate is the
        // empty clause (an unconditional contradiction), which is correct.
        let eqs = vec![
            XorEquation::new(vec![0, 1], false),
            XorEquation::new(vec![1, 2], false),
            XorEquation::new(vec![0, 2], true),
        ];
        let eng = XorEngine::new(3, &eqs);
        match eng.analyze(&vec![None, None, None]) {
            XorStep::Conflict(_) => {}
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[test]
    fn next_branch_picks_a_free_kernel_var_and_breaking_it_collapses_the_rest() {
        // x0⊕x1=0, x2⊕x3=0: pivots x0,x2; free (kernel) x1,x3. The engine must offer a free var as
        // the break point, and breaking it forces its pivot partner (the collapse).
        let eqs = vec![XorEquation::new(vec![0, 1], false), XorEquation::new(vec![2, 3], false)];
        let mut inc = IncXor::new(4, &eqs);
        assert_eq!(inc.rank(), 2);
        assert_eq!(inc.kernel_dim(), 2);
        let b = inc.next_branch().expect("a free kernel var to break on");
        assert!(b == 1 || b == 3, "break point must be a non-pivot free var, got {b}");
        inc.assign(b, true);
        let forced = inc.state().expect("consistent");
        assert!(!forced.is_empty(), "breaking a kernel var must force its pivot partner");
    }

    #[test]
    fn incremental_engine_matches_the_recompute_oracle_under_random_trails() {
        // The fast IncXor, driven by random assign/unassign (push/pop) sequences, must report the
        // SAME forced-literal set and conflict status as the recompute XorEngine at EVERY step.
        let mut rng: u64 = 0xD1B5_4A32_D192_ED03;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let forced_set = |r: &Result<Vec<(Lit, Vec<Lit>)>, Vec<Lit>>| -> Option<std::collections::BTreeSet<(u32, bool)>> {
            match r {
                Err(_) => None, // conflict
                Ok(f) => Some(f.iter().map(|(l, _)| (l.var(), l.is_positive())).collect()),
            }
        };
        for _ in 0..2000 {
            let num_vars = 3 + (next() % 6) as usize; // 3..=8
            let n_eqs = 1 + (next() % 6) as usize;
            let mut eqs = Vec::new();
            for _ in 0..n_eqs {
                let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
                if !vars.is_empty() {
                    eqs.push(XorEquation::new(vars, next() % 2 == 0));
                }
            }
            if eqs.is_empty() {
                continue;
            }
            let oracle = XorEngine::new(num_vars, &eqs);
            let mut inc = IncXor::new(num_vars, &eqs);
            let mut assign: Vec<Option<bool>> = vec![None; num_vars];
            let mut stack: Vec<usize> = Vec::new();

            for _step in 0..40 {
                let free: Vec<usize> = (0..num_vars).filter(|&v| assign[v].is_none()).collect();
                let push = !free.is_empty() && (stack.is_empty() || next() % 2 == 0);
                if push {
                    let v = free[(next() % free.len() as u64) as usize];
                    let val = next() % 2 == 0;
                    inc.assign(v, val);
                    assign[v] = Some(val);
                    stack.push(v);
                } else if let Some(v) = stack.pop() {
                    inc.unassign();
                    assign[v] = None;
                }
                inc.check_occ();

                let got = inc.state();
                let truth = oracle.all_consequences(&assign);
                // conflict status must match
                assert_eq!(got.is_err(), truth.is_err(),
                    "conflict mismatch: inc={:?} oracle_err={} (eqs={eqs:?} assign={assign:?})",
                    got.is_err(), truth.is_err());
                if got.is_ok() {
                    assert_eq!(forced_set(&got), forced_set(&truth),
                        "forced-set mismatch (eqs={eqs:?} assign={assign:?})");
                    // every incremental reason must be unit on its literal (implied shape).
                    if let Ok(f) = &got {
                        for (lit, reason) in f {
                            let un: Vec<_> = reason.iter().filter(|l| assign[l.var() as usize].is_none()).collect();
                            assert_eq!(un.len(), 1, "reason must be unit");
                            assert_eq!(un[0].var(), lit.var());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn matches_the_brute_force_oracle_exhaustively() {
        // Robustness to absurdity: random small GF(2) systems × random partial assignments, the
        // engine's forced-set and consistency must match the brute-force oracle EXACTLY, and every
        // emitted clause must be implied (true in all solutions) and correctly unit/falsified.
        let mut rng: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for _ in 0..3000 {
            let num_vars = 3 + (next() % 5) as usize; // 3..=7
            let n_eqs = 1 + (next() % 6) as usize;
            let mut eqs = Vec::new();
            for _ in 0..n_eqs {
                let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
                if vars.is_empty() {
                    continue;
                }
                eqs.push(XorEquation::new(vars, next() % 2 == 0));
            }
            if eqs.is_empty() {
                continue;
            }
            // random partial assignment
            let assign: Vec<Option<bool>> =
                (0..num_vars).map(|_| match next() % 3 { 0 => Some(true), 1 => Some(false), _ => None }).collect();

            let eng = XorEngine::new(num_vars, &eqs);
            let got = eng.all_consequences(&assign);
            let truth = oracle(num_vars, &eqs, &assign);

            match (got, truth) {
                (Err(conflict), None) => {
                    // A genuine conflict: every literal is false under the current assignment...
                    for l in &conflict {
                        assert_eq!(assign[l.var() as usize], Some(!l.is_positive()),
                            "conflict literal must be false under the assignment");
                    }
                    // ...and the clause is IMPLIED — true in every full solution of the system.
                    for mask in 0u32..(1u32 << num_vars) {
                        let full: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
                        if eqs.iter().all(|e| e.vars.iter().fold(false, |a, &x| a ^ full[x]) == e.rhs) {
                            assert!(lit_sat(&conflict, &full), "conflict clause must be implied");
                        }
                    }
                }
                (Ok(forced), Some(truth_forced)) => {
                    // Every engine-forced var must agree with the oracle.
                    for (lit, reason) in &forced {
                        let v = lit.var() as usize;
                        assert_eq!(truth_forced[v], Some(lit.is_positive()),
                            "engine forced x{v}={} but oracle disagrees (eqs={eqs:?}, assign={assign:?})", lit.is_positive());
                        // reason must be unit: exactly one unassigned literal, and it is `lit`.
                        let un: Vec<_> = reason.iter().filter(|l| assign[l.var() as usize].is_none()).collect();
                        assert_eq!(un.len(), 1, "reason must be unit");
                        assert_eq!(un[0].var(), lit.var());
                        // reason must be IMPLIED: true in every consistent completion.
                        for mask in 0u32..(1u32 << num_vars) {
                            let full: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
                            let sat_all = eqs.iter().all(|e| e.vars.iter().fold(false, |a, &x| a ^ full[x]) == e.rhs);
                            if sat_all {
                                assert!(lit_sat(reason, &full), "reason clause must hold in every solution");
                            }
                        }
                    }
                    // Completeness: every oracle-forced unassigned var must be found by the engine.
                    let engine_vars: std::collections::HashSet<usize> =
                        forced.iter().map(|(l, _)| l.var() as usize).collect();
                    for v in 0..num_vars {
                        if assign[v].is_none() && truth_forced[v].is_some() {
                            assert!(engine_vars.contains(&v),
                                "engine missed forced x{v} (eqs={eqs:?}, assign={assign:?})");
                        }
                    }
                }
                (Ok(forced), None) => panic!("engine missed a contradiction: forced={forced:?} eqs={eqs:?} assign={assign:?}"),
                (Err(c), Some(_)) => panic!("engine reported a false conflict {c:?}: eqs={eqs:?} assign={assign:?}"),
            }
        }
    }
}
