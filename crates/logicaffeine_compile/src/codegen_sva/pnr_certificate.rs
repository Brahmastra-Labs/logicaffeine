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
use logicaffeine_proof::proof::{ProofStep, Witness};
use logicaffeine_proof::sat::prove_unsat_certified_cnf;

use super::bitblast::{
    assert_not, blast_pred, pred_size, BlastRefusal, Pred, Word, BLAST_BUDGET_NODES,
};
use super::fabric_grid::{grid_output_to_bounded, Grid};
use super::fabric_rtl::{fabric_to_bounded, FabricOp};
use super::sva_to_verify::BoundedExpr;

/// A machine-checkable certificate that a placed mapping `grid` computes its datapath spec for every bounded
/// assignment. It carries the refuted CNF (`¬(grid ≡ datapath)`) and the propagation-redundancy + RUP
/// refutation of it. It is self-contained: [`recheck`](Self::recheck) needs nothing but the certificate.
#[derive(Clone, Debug)]
pub struct PlacementCertificate {
    num_vars: usize,
    cnf: Vec<Vec<Lit>>,
    steps: Vec<ProofStep>,
}

impl PlacementCertificate {
    /// Re-verify the certificate INDEPENDENTLY. Trusts only the small PR/RUP checker — not the prover, placer,
    /// or emitter that produced it. This is the audit a third party runs to confirm our place-and-route
    /// without re-deriving anything: it replays the refutation against the original CNF and returns whether
    /// every step is redundancy-sound and the empty clause is reached.
    pub fn recheck(&self) -> bool {
        check_pr_refutation(self.num_vars, &self.cnf, &self.steps)
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
    let eq = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p = blast_pred(&eq).ok_or(CertifyRefusal::Blast(BlastRefusal::NotBounded))?;
    let size = pred_size(&p);
    if size.tree_nodes > budget {
        return Err(CertifyRefusal::Blast(BlastRefusal::Budget { size, budget }));
    }
    refute_miter(&p)
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
    let p = super::bitblast::words_equal(grid, datapath)
        .ok_or(CertifyRefusal::Blast(BlastRefusal::NotBounded))?;
    let size = pred_size(&p);
    if size.dag_nodes > budget {
        return Err(CertifyRefusal::Blast(BlastRefusal::GateBudget {
            dag_nodes: size.dag_nodes,
            budget,
        }));
    }
    refute_miter(&p)
}

/// Refute `¬(grid ≡ datapath)` and KEEP the checkable refutation.
///
/// The CNF is clausified STRAIGHT FROM THE GATE DAG — the same clauses, in the same order, over
/// the same variables that `Cnf::assert` produces from the materialised tree, and it is built
/// ONCE and handed to the prover, rather than built here and built again inside it.
fn refute_miter(p: &Pred) -> Result<PlacementCertificate, CertifyRefusal> {
    let mut cnf = Cnf::new();
    assert_not(p, &mut cnf);
    let steps = prove_unsat_certified_cnf(cnf.num_vars(), cnf.clauses())
        .ok_or(CertifyRefusal::NotRefuted)?;
    Ok(PlacementCertificate {
        num_vars: cnf.num_vars(),
        cnf: cnf.clauses().to_vec(),
        steps,
    })
}

/// **Import-and-certify** — the SDK-facing seam. Certify a placement PRODUCED BY AN EXTERNAL TOOL (e.g. the
/// SambaFlow place-and-route) against the English `source` datapath: lower the external `placement` grid and
/// the `source` spec to the same bounded fragment, then prove they agree for every input and keep the
/// re-checkable witness. Returns `None` (fail-closed) if the placement is NOT equivalent to the source, or the
/// obligation escapes the propositional fragment. We need nothing from the vendor but the placed grid — no
/// config format, no trust in their placer: the certificate stands on its own via
/// [`PlacementCertificate::recheck`]. This is how a vendor mapping earns a proof it computes the intended
/// program.
pub fn certify_external_placement(
    placement: &Grid,
    source: &[FabricOp],
    result: usize,
    width: u32,
) -> Option<PlacementCertificate> {
    let placed = grid_output_to_bounded(placement, 0, width, &|k| format!("in{k}"));
    let spec = fabric_to_bounded(source, result, width);
    certify_mapping(placed, spec)
}
