//! Proof-carrying place-and-route.
//!
//! A dataflow compiler that only *reports* a mapping — the way a spatial-architecture toolchain validates a
//! placement with performance counters — asks you to TRUST that the placed grid computes the intended
//! datapath. We instead SHIP A CERTIFICATE: a machine-checkable refutation that the placed grid equals its
//! datapath specification for **every** bounded input. The recipient (e.g. the silicon vendor) re-verifies it
//! with [`PlacementCertificate::recheck`], trusting only the tiny PR/RUP checker in
//! [`logicaffeine_proof::pr`] — never our prover, our placer, or our word. That is the difference between
//! "we validated it" and "here is the proof; check it yourself."
//!
//! The obligation is the same `grid ≡ datapath` biconditional the SAT-certified placer already discharges;
//! here we keep the *witness* (the propagation-redundancy + RUP steps) instead of discarding it, so the
//! certificate travels with the emitted RTL.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::proof::{Perm, ProofStep, Witness};
use logicaffeine_proof::sat::prove_unsat_certified_cnf;

use super::bitblast::{
    assert_not, blast_pred, pred_size, BlastRefusal, Pred, Word, BLAST_BUDGET_NODES,
};
use super::fabric_grid::{grid_output_to_bounded, Grid};
use super::fabric_rtl::{fabric_to_bounded, FabricOp};
use super::sva_to_verify::BoundedExpr;

/// A machine-checkable certificate that a placed mapping `grid` computes its datapath spec for every bounded
/// assignment. It carries the refuted CNF (`¬(grid ≡ datapath)`) and the propagation-redundancy + RUP
/// refutation of it. It is self-contained: [`recheck`](Self::recheck) needs nothing but the certificate, and it travels
/// whole ([`Shipped`]): read back, it rechecks as the certificate it was written from.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(into = "Shipped", from = "Shipped")]
pub struct PlacementCertificate {
    num_vars: usize,
    cnf: Vec<Vec<Lit>>,
    steps: Vec<ProofStep>,
}

/// A [`PlacementCertificate`] as it travels: every literal a DIMACS integer (`v + 1` for `+v`, `-(v + 1)` for `¬v`), each
/// refutation step by its kind, and a permutation witness as the images of the positive literals in variable order.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Shipped {
    num_vars: usize,
    cnf: Vec<Vec<i64>>,
    steps: Vec<ShippedStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShippedStep {
    Rup(Vec<i64>),
    Pr { clause: Vec<i64>, witness: ShippedWitness },
    Delete(Vec<i64>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShippedWitness {
    Assignment(Vec<i64>),
    Substitution(Vec<i64>),
    PartialSymmetry(Vec<i64>),
}

fn dimacs(l: Lit) -> i64 {
    let v = i64::from(l.var()) + 1;
    if l.is_positive() {
        v
    } else {
        -v
    }
}

fn literal(d: i64) -> Lit {
    let v = (d.unsigned_abs() - 1) as u32;
    if d > 0 {
        Lit::pos(v)
    } else {
        Lit::neg(v)
    }
}

fn clause_out(c: &[Lit]) -> Vec<i64> {
    c.iter().copied().map(dimacs).collect()
}

fn clause_in(c: &[i64]) -> Vec<Lit> {
    c.iter().copied().map(literal).collect()
}

fn images(perm: &Perm) -> Vec<i64> {
    (0..perm.num_vars()).map(|v| dimacs(perm.apply(Lit::pos(v as u32)))).collect()
}

fn step_out(s: &ProofStep) -> ShippedStep {
    match s {
        ProofStep::Rup(c) => ShippedStep::Rup(clause_out(c)),
        ProofStep::Delete(c) => ShippedStep::Delete(clause_out(c)),
        ProofStep::Pr { clause, witness } => ShippedStep::Pr {
            clause: clause_out(clause),
            witness: match witness {
                Witness::Assignment(ls) => ShippedWitness::Assignment(clause_out(ls)),
                Witness::Substitution(p) => ShippedWitness::Substitution(images(p)),
                Witness::PartialSymmetry(p) => ShippedWitness::PartialSymmetry(images(p)),
            },
        },
    }
}

fn step_in(step: &ShippedStep) -> ProofStep {
    match step {
        ShippedStep::Rup(c) => ProofStep::Rup(clause_in(c)),
        ShippedStep::Delete(c) => ProofStep::Delete(clause_in(c)),
        ShippedStep::Pr { clause, witness } => ProofStep::Pr {
            clause: clause_in(clause),
            witness: match witness {
                ShippedWitness::Assignment(ls) => Witness::Assignment(clause_in(ls)),
                ShippedWitness::Substitution(im) => Witness::Substitution(Perm::from_images(clause_in(im))),
                ShippedWitness::PartialSymmetry(im) => Witness::PartialSymmetry(Perm::from_images(clause_in(im))),
            },
        },
    }
}

impl From<PlacementCertificate> for Shipped {
    fn from(c: PlacementCertificate) -> Shipped {
        Shipped { num_vars: c.num_vars, cnf: c.cnf.iter().map(|c| clause_out(c)).collect(), steps: c.steps.iter().map(step_out).collect() }
    }
}

impl From<Shipped> for PlacementCertificate {
    fn from(s: Shipped) -> PlacementCertificate {
        PlacementCertificate { num_vars: s.num_vars, cnf: s.cnf.iter().map(|c| clause_in(c)).collect(), steps: s.steps.iter().map(step_in).collect() }
    }
}

/// The SHA-256 of a formula in DIMACS (`p cnf V C`, then each clause's literals and its `0`), as 64 lowercase hex digits.
fn formula_digest(num_vars: usize, cnf: &[Vec<Lit>]) -> String {
    let mut text = format!("p cnf {num_vars} {}\n", cnf.len());
    for c in cnf {
        for l in c {
            text.push_str(&dimacs(*l).to_string());
            text.push(' ');
        }
        text.push_str("0\n");
    }
    crate::crypto_accel::sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A refutation as it travels without its formula: a reader rebuilds the formula from what the certificate certifies,
/// and the digest of the formula it refutes says, before a step is replayed, whether it is that one.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShippedProof {
    /// The SHA-256 of the refuted formula in DIMACS.
    pub formula: String,
    steps: Vec<ShippedStep>,
}

impl ShippedProof {
    /// The refutation held to `obligation`, rebuilt by the reader, the thing it certifies named `subject`: refused unless
    /// it is of the obligation's formula and refutes it.
    pub fn certifies(&self, obligation: &Obligation, subject: &str) -> Result<(), PlacementFault> {
        if self.formula != obligation.digest() {
            return Err(PlacementFault::ForeignFormula { subject: subject.to_string() });
        }
        let cert = PlacementCertificate { num_vars: obligation.num_vars, cnf: obligation.cnf.clone(), steps: self.steps.iter().map(step_in).collect() };
        if cert.recheck() {
            Ok(())
        } else {
            Err(PlacementFault::Refutation { subject: subject.to_string() })
        }
    }
}

/// Why a placement certificate does not certify the placement it is offered for, that placement named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementFault {
    /// The placement's obligation cannot be rebuilt, so nothing can be held to it.
    NoObligation { subject: String, why: String },
    /// The certificate travels with no proof to recheck.
    NoProof { subject: String },
    /// The certificate refutes a formula that is not the placement's obligation.
    ForeignFormula { subject: String },
    /// The certificate's refutation does not refute its formula.
    Refutation { subject: String },
}

impl PlacementFault {
    /// What the certificate was offered for.
    pub fn subject(&self) -> &str {
        match self {
            PlacementFault::NoObligation { subject, .. }
            | PlacementFault::NoProof { subject }
            | PlacementFault::ForeignFormula { subject }
            | PlacementFault::Refutation { subject } => subject,
        }
    }

    /// The fault with its subject named within `context` — the backend or the lane that read the certificate.
    pub fn within(mut self, context: &str) -> PlacementFault {
        let named = match &mut self {
            PlacementFault::NoObligation { subject, .. }
            | PlacementFault::NoProof { subject }
            | PlacementFault::ForeignFormula { subject }
            | PlacementFault::Refutation { subject } => subject,
        };
        *named = format!("{context}: {named}");
        self
    }
}

impl std::fmt::Display for PlacementFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlacementFault::NoObligation { subject, why } => write!(f, "{subject}: its obligation cannot be rebuilt ({why})"),
            PlacementFault::NoProof { subject } => write!(f, "{subject}: the certificate carries no proof to recheck"),
            PlacementFault::ForeignFormula { subject } => {
                write!(f, "{subject}: the certificate refutes a formula that is not the placement's obligation")
            }
            PlacementFault::Refutation { subject } => write!(f, "{subject}: the certificate's refutation does not refute its formula"),
        }
    }
}

impl PlacementCertificate {
    /// Re-verify the certificate INDEPENDENTLY. Trusts only the small PR/RUP checker — not the prover, placer,
    /// or emitter that produced it. This is the audit a third party runs to confirm our place-and-route
    /// without re-deriving anything: it replays the refutation against the original CNF and returns whether
    /// every step is redundancy-sound and the empty clause is reached.
    ///
    /// It says that the CNF the certificate carries is unsatisfiable, and nothing about which CNF that is: a
    /// certificate proves a placement only against the obligation the checker rebuilds from the grid and the
    /// datapath themselves ([`Self::refutes`]).
    pub fn recheck(&self) -> bool {
        check_pr_refutation(self.num_vars, &self.cnf, &self.steps)
    }

    /// Whether the formula the certificate carries is `obligation`, clause for clause.
    pub fn carries(&self, obligation: &Obligation) -> bool {
        self.num_vars == obligation.num_vars && self.cnf == obligation.cnf
    }

    /// The certificate held to `obligation`, the thing it certifies named `subject`: refused unless the formula it carries
    /// is the obligation and its refutation rechecks.
    pub fn certifies(&self, obligation: &Obligation, subject: &str) -> Result<(), PlacementFault> {
        if !self.carries(obligation) {
            return Err(PlacementFault::ForeignFormula { subject: subject.to_string() });
        }
        if !self.recheck() {
            return Err(PlacementFault::Refutation { subject: subject.to_string() });
        }
        Ok(())
    }

    /// The refutation as it travels without its formula ([`ShippedProof`]).
    pub fn proof(&self) -> ShippedProof {
        ShippedProof { formula: formula_digest(self.num_vars, &self.cnf), steps: self.steps.iter().map(step_out).collect() }
    }

    /// Whether the certificate refutes `obligation`: the formula it carries is the obligation, and its refutation of it
    /// rechecks.
    pub fn refutes(&self, obligation: &Obligation) -> bool {
        self.carries(obligation) && self.recheck()
    }

    /// The number of refutation steps carried — the size of the shipped proof.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The number of Boolean variables in the refuted obligation.
    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// A stable 64-bit digest of the whole certificate — the refuted CNF and every refutation step, witnesses
    /// included (FNV-1a over the literal codes) — so a summary that travels without the object is bound to
    /// it: a dropped step, a changed clause or a different obligation is a different digest.
    pub fn digest(&self) -> u64 {
        fn mix(h: &mut u64, x: u64) {
            *h ^= x;
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        fn code(l: Lit) -> u64 {
            ((l.var() as u64) << 1) | u64::from(!l.is_positive())
        }
        fn clause(h: &mut u64, c: &[Lit]) {
            mix(h, c.len() as u64);
            for l in c {
                mix(h, code(*l));
            }
        }
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        mix(&mut h, self.num_vars as u64);
        mix(&mut h, self.cnf.len() as u64);
        for c in &self.cnf {
            clause(&mut h, c);
        }
        mix(&mut h, self.steps.len() as u64);
        for step in &self.steps {
            match step {
                ProofStep::Rup(c) => {
                    mix(&mut h, 1);
                    clause(&mut h, c);
                }
                ProofStep::Pr { clause: c, witness } => {
                    mix(&mut h, 2);
                    clause(&mut h, c);
                    match witness {
                        Witness::Assignment(ls) => {
                            mix(&mut h, 3);
                            clause(&mut h, ls);
                        }
                        Witness::Substitution(perm) => {
                            mix(&mut h, 4);
                            mix(&mut h, perm.num_vars() as u64);
                            for v in 0..perm.num_vars() {
                                mix(&mut h, code(perm.apply(Lit::pos(v as u32))));
                            }
                        }
                        Witness::PartialSymmetry(perm) => {
                            mix(&mut h, 6);
                            mix(&mut h, perm.num_vars() as u64);
                            for v in 0..perm.num_vars() {
                                mix(&mut h, code(perm.apply(Lit::pos(v as u32))));
                            }
                        }
                    }
                }
                ProofStep::Delete(c) => {
                    mix(&mut h, 5);
                    clause(&mut h, c);
                }
            }
        }
        h
    }

    /// A tampered copy: the final refutation step is dropped, so the empty clause is no longer derived. Used to
    /// prove the checker is load-bearing — [`recheck`](Self::recheck) on the result must return `false`.
    #[must_use]
    pub fn with_last_step_removed(&self) -> Self {
        let mut steps = self.steps.clone();
        steps.pop();
        Self {
            num_vars: self.num_vars,
            cnf: self.cnf.clone(),
            steps,
        }
    }
}

/// Emit a proof-carrying certificate that the placed `grid` equals the `datapath` spec for every bounded
/// input, by refuting `¬(grid ≡ datapath)` and KEEPING the checkable refutation. Fail-closed: returns `None`
/// when the equivalence does not actually hold (a wrong mapping earns no certificate) or the obligation
/// leaves the propositional fragment (bit-blast the datapath first). A returned certificate is guaranteed
/// [`recheck`](PlacementCertificate::recheck)-valid by construction — the underlying prover only hands back a
/// refutation it has already PR/RUP-verified.
pub fn certify_mapping(grid: BoundedExpr, datapath: BoundedExpr) -> Option<PlacementCertificate> {
    certify_mapping_bounded(grid, datapath).ok()
}

/// Why a mapping earned no certificate. The three answers are genuinely different — "we cannot
/// express this obligation", "we will not pay to materialise it", and "it is not true" — and a
/// caller that prints `Uncertified { reason }` must not collapse them into one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertifyRefusal {
    /// The obligation never reached a formula: no propositional form, or over the bit-blast budget.
    Blast(BlastRefusal),
    /// The formula was built but could not be refuted — the mapping is NOT equivalent to the spec,
    /// or the prover declined it. Fail-closed: a wrong mapping earns no certificate.
    NotRefuted,
}

impl CertifyRefusal {
    /// The prose a certificate carries when it declines. Names the cause — and, for a budget
    /// refusal, the budget itself, so the reader can tell a limit from a failure.
    pub fn reason(&self) -> String {
        match self {
            CertifyRefusal::Blast(b) => b.reason(),
            CertifyRefusal::NotRefuted => {
                "the placement obligation could not be refuted".to_string()
            }
        }
    }
}

/// [`certify_mapping`] with the refusal NAMED. The obligation is PRICED in the shared gate DAG
/// before any `ProofExpr` is materialised, so an encoding too big to hold fails as a typed refusal
/// in milliseconds instead of walking the allocator off a cliff.
pub fn certify_mapping_bounded(
    grid: BoundedExpr,
    datapath: BoundedExpr,
) -> Result<PlacementCertificate, CertifyRefusal> {
    certify_mapping_within(grid, datapath, BLAST_BUDGET_NODES)
}

/// [`certify_mapping_bounded`] against an EXPLICIT bit-blast ceiling — the seam a lock shrinks to
/// prove the budget is live code and not a constant nobody reads.
pub fn certify_mapping_within(
    grid: BoundedExpr,
    datapath: BoundedExpr,
    budget: u128,
) -> Result<PlacementCertificate, CertifyRefusal> {
    mapping_obligation_within(grid, datapath, budget).and_then(refute)
}

/// How large a SHARED GATE DAG the certificate path will pay to refute.
///
/// This is the ceiling that stays live once the obligation is built as a circuit rather than as a
/// tree. [`BLAST_BUDGET_NODES`] prices the `ProofExpr` TREE, which is the right question only for
/// a caller that materialises one; the gate path never does, so what it must bound is the number
/// of gates it allocates and Tseitin-encodes — both linear in the DAG. The RDU demo corpus's
/// widest body (`fib.lg`'s six-block fast-doubling chain, 75 cells of chained 8-bit multiplies)
/// prices in the tens of thousands of gates once the grid and the spec are built as DAGs, so two
/// million is orders of magnitude of headroom over anything the product lowers today, and a body
/// past it is refused BY NAME in the time it takes to build the circuit — never by the allocator.
/// `certifier_strength_lock` re-measures both sides of that claim.
pub const GATE_BUDGET_NODES: u64 = 2_000_000;

/// Certify a placement whose grid and datapath are ALREADY blasted into the shared gate DAG — the
/// path that never materialises a `ProofExpr` tree, and therefore the only one that can certify a
/// reconvergent datapath (a multiply chain's tree is exponentially larger than its circuit).
/// Fail-closed exactly like [`certify_mapping`]: a grid that is not equivalent to its spec earns
/// no certificate.
pub fn certify_gates(grid: &Word, datapath: &Word) -> Option<PlacementCertificate> {
    certify_gates_bounded(grid, datapath).ok()
}

/// [`certify_gates`] with the refusal NAMED, against the production [`GATE_BUDGET_NODES`].
pub fn certify_gates_bounded(
    grid: &Word,
    datapath: &Word,
) -> Result<PlacementCertificate, CertifyRefusal> {
    certify_gates_within(grid, datapath, GATE_BUDGET_NODES)
}

/// [`certify_gates_bounded`] against an EXPLICIT gate ceiling — the seam a lock shrinks to prove
/// the budget is live code and not a constant nobody reads.
pub fn certify_gates_within(
    grid: &Word,
    datapath: &Word,
    budget: u64,
) -> Result<PlacementCertificate, CertifyRefusal> {
    gates_obligation_within(grid, datapath, budget).and_then(refute)
}

/// What a placement certificate refutes: the CNF of `¬(grid ≡ datapath)`.
///
/// The CNF is clausified STRAIGHT FROM THE GATE DAG — the same clauses, in the same order, over the same variables that
/// `Cnf::assert` produces from the materialised tree — so the obligation built from one grid and one datapath is one
/// formula, built alike by the certifier, which refutes it, and by a checker, which holds a carried formula to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    num_vars: usize,
    cnf: Vec<Vec<Lit>>,
}

impl Obligation {
    fn of(p: &Pred) -> Obligation {
        let mut cnf = Cnf::new();
        assert_not(p, &mut cnf);
        Obligation { num_vars: cnf.num_vars(), cnf: cnf.clauses().to_vec() }
    }

    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// How many clauses the formula has.
    pub fn clauses(&self) -> usize {
        self.cnf.len()
    }

    /// The SHA-256 of the formula in DIMACS.
    pub fn digest(&self) -> String {
        formula_digest(self.num_vars, &self.cnf)
    }
}

/// The obligation of a mapping whose grid and datapath are bounded expressions, priced as a tree against `budget`.
pub fn mapping_obligation_within(grid: BoundedExpr, datapath: BoundedExpr, budget: u128) -> Result<Obligation, CertifyRefusal> {
    let eq = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p = blast_pred(&eq).ok_or(CertifyRefusal::Blast(BlastRefusal::NotBounded))?;
    let size = pred_size(&p);
    if size.tree_nodes > budget {
        return Err(CertifyRefusal::Blast(BlastRefusal::Budget { size, budget }));
    }
    Ok(Obligation::of(&p))
}

/// The obligation of a mapping whose grid and datapath are already in the shared gate DAG, priced in gates against
/// `budget`.
pub fn gates_obligation_within(grid: &Word, datapath: &Word, budget: u64) -> Result<Obligation, CertifyRefusal> {
    let p = super::bitblast::words_equal(grid, datapath).ok_or(CertifyRefusal::Blast(BlastRefusal::NotBounded))?;
    let size = pred_size(&p);
    if size.dag_nodes > budget {
        return Err(CertifyRefusal::Blast(BlastRefusal::GateBudget { dag_nodes: size.dag_nodes, budget }));
    }
    Ok(Obligation::of(&p))
}

/// Refute `obligation` and KEEP the checkable refutation, the formula built once and handed to the prover.
pub fn refute(obligation: Obligation) -> Result<PlacementCertificate, CertifyRefusal> {
    let steps = prove_unsat_certified_cnf(obligation.num_vars, &obligation.cnf).ok_or(CertifyRefusal::NotRefuted)?;
    Ok(PlacementCertificate { num_vars: obligation.num_vars, cnf: obligation.cnf, steps })
}

/// **Import-and-certify** — the SDK-facing seam. Certify a placement PRODUCED BY AN EXTERNAL TOOL (e.g. the
/// SambaFlow place-and-route) against the English `source` datapath: lower the external `placement` grid and
/// the `source` spec to the same bounded fragment, then prove they agree for every input and keep the
/// re-checkable witness. Returns `None` (fail-closed) if the placement is NOT equivalent to the source, or the
/// obligation escapes the propositional fragment. We need nothing from the vendor but the placed grid — no
/// config format, no trust in their placer: the certificate stands on its own via
/// [`PlacementCertificate::recheck`]. This is how a vendor mapping earns a proof it computes the intended
/// program.
///
/// The certificate it hands back is first held to the obligation [`recheck_external_placement`] rebuilds, as every
/// reader of it holds it.
pub fn certify_external_placement(
    placement: &Grid,
    source: &[FabricOp],
    result: usize,
    width: u32,
) -> Option<PlacementCertificate> {
    let cert = external_obligation(placement, source, result, width).and_then(refute).ok()?;
    recheck_external_placement(&cert, placement, source, result, width).ok()?;
    Some(cert)
}

/// What an external placement's certificate refutes: the placed grid's output 0 against the `source` datapath's cell
/// `result`, every `width`-bit input — the formula built from the grid and the datapath alone.
pub fn external_obligation(placement: &Grid, source: &[FabricOp], result: usize, width: u32) -> Result<Obligation, CertifyRefusal> {
    let placed = grid_output_to_bounded(placement, 0, width, &|k| format!("in{k}"));
    let spec = fabric_to_bounded(source, result, width);
    mapping_obligation_within(placed, spec, BLAST_BUDGET_NODES)
}

/// The reader's side of [`certify_external_placement`]: `cert` held to the obligation rebuilt from `placement` and
/// `source` — never to the formula it carries on its word — the refusal naming the placement it was offered for.
pub fn recheck_external_placement(
    cert: &PlacementCertificate,
    placement: &Grid,
    source: &[FabricOp],
    result: usize,
    width: u32,
) -> Result<(), PlacementFault> {
    let subject = format!("the placed grid's output 0 against the datapath's cell {result} at {width} bits");
    let obligation = external_obligation(placement, source, result, width)
        .map_err(|r| PlacementFault::NoObligation { subject: subject.clone(), why: r.reason() })?;
    cert.certifies(&obligation, &subject)
}
