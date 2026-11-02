//! # Algorithmic information theory — certified description-length objects
//!
//! The provers already *measure* algorithmic information (`hypercube::symmetry_entropy_bits` =
//! log₂|Aut(F)|; the clause-orbit quotient as "the computable shadow of Kolmogorov complexity").
//! This module promotes those measurements into **certified objects**: a description length that
//! carries a re-checkable witness, so "this object compresses to N bytes" is something a checker
//! confirms rather than trusts.
//!
//! Kolmogorov complexity `K(x)` is uncomputable, so we work with the two honest sides of it:
//!
//! - **Upper bounds** `K̄(x)` are computable — the shortest program in a *fixed* description language
//!   that reproduces `x`. [`DescriptionBound`] carries such a program together with the bytes that,
//!   when decoded, reproduce `x` (the witness). The description language is layered: [`Descriptor::IntSeq`]
//!   is the closed-form generator menu of [`logicaffeine_base::describe`] (affine / geometric /
//!   polynomial / periodic / sparse / …), a computable upper bound with a lossless decode witness.
//!
//! Lower bounds (the incompressibility side) and the two-sided structural bound live alongside this,
//! gated by a budget so we never claim a bound past what the prover can itself re-check (the
//! operational Chaitin ceiling).

use logicaffeine_base::describe;

use crate::cdcl::{Lit, Var};
use crate::proof::Perm;
use std::collections::BTreeSet;

/// The description language. Each variant is a *program* in a fixed language whose length is the
/// description bound and whose execution (decode) reproduces the object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Descriptor {
    /// Layer 1: the closed-form generator menu of [`logicaffeine_base::describe`] applied to a raw
    /// integer sequence. `encoded` is the shortest menu encoding; decoding it reproduces the sequence.
    IntSeq { encoded: Vec<u8> },
    /// Layer 3: a Boolean function described by its recursive symmetry decomposition ([`structure_tree`]).
    /// Decoding replays the tree to reproduce the `2ⁿ` truth table.
    BooleanFunction {
        num_vars: usize,
        tree: StructureTree,
    },
}

/// A **computable upper bound** `K̄(x)` on the Kolmogorov complexity of an object `x`, over the fixed
/// description language, carrying a re-checkable decode witness.
///
/// `bytes` is the program length (the bound). `object_hash` is a stable content hash of `x`.
/// [`DescriptionBound::verify`] decodes the descriptor and confirms it reproduces `x` — an upper
/// bound is only trustworthy because *decode(program) = x*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptionBound {
    /// A stable content hash of the described object (order- and value-sensitive).
    pub object_hash: u64,
    /// The description length in bytes — the certified upper bound `K̄(x)`.
    pub bytes: usize,
    /// The program (in the description language) that reproduces the object.
    pub descriptor: Descriptor,
}

impl DescriptionBound {
    /// Describe an integer sequence by its shortest generator-menu encoding (Layer 1). The result's
    /// `bytes` is never larger than the plain-varint length of the sequence.
    pub fn of_int_seq(v: &[i64]) -> DescriptionBound {
        let encoded = describe::describe_int_seq(v);
        DescriptionBound {
            object_hash: hash_ints(v),
            bytes: encoded.len(),
            descriptor: Descriptor::IntSeq { encoded },
        }
    }

    /// Describe a Boolean function by its recursive symmetry decomposition (Layer 3), the same certified
    /// upper bound as [`kolmogorov_bound`] but inside the unified `DescriptionBound` framework. `bytes` is
    /// the tree's total description size rounded up to bytes. `None` if `truth.len()` is not a power of two.
    pub fn of_boolean(truth: &[bool]) -> Option<DescriptionBound> {
        let num_vars = truth.len().trailing_zeros() as usize;
        let tree = structure_tree(truth)?;
        let bytes = tree.total_description_bits().div_ceil(8).max(1);
        Some(DescriptionBound {
            object_hash: hash_bools(truth),
            bytes,
            descriptor: Descriptor::BooleanFunction { num_vars, tree },
        })
    }

    /// Re-check the decode witness: decode the descriptor and confirm it reproduces the object whose
    /// hash we recorded. This is the whole point of an upper bound — it is trusted only because the
    /// program actually reconstructs `x`. A tampered/corrupt descriptor fails here.
    pub fn verify(&self) -> bool {
        match &self.descriptor {
            Descriptor::IntSeq { encoded } => {
                encoded.len() == self.bytes
                    && describe::decode_int_seq(encoded)
                        .map_or(false, |v| hash_ints(&v) == self.object_hash)
            }
            Descriptor::BooleanFunction { num_vars, tree } => {
                tree.total_description_bits().div_ceil(8).max(1) == self.bytes
                    && tree.reconstruct().map_or(false, |t| {
                        t.len() == (1usize << num_vars) && hash_bools(&t) == self.object_hash
                    })
            }
        }
    }
}

/// A stable content hash of a Boolean truth table (FNV-1a over the bits, salted by length).
fn hash_bools(v: &[bool]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut mix = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    };
    for b in (v.len() as u64).to_le_bytes() {
        mix(b);
    }
    for &x in v {
        mix(x as u8);
    }
    h
}

/// A stable, portable content hash (FNV-1a over the little-endian value bytes, salted by the count so
/// a trailing zero can't alias a shorter sequence). Deterministic across runs and machines.
fn hash_ints(v: &[i64]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut mix = |bytes: [u8; 8]| {
        for b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
    };
    mix((v.len() as u64).to_le_bytes());
    for &x in v {
        mix(x.to_le_bytes());
    }
    h
}

// ---- The two-sided structural bound: K̄(F) ≤ K̄(rep) + K̄(gens) + O(1) -----------------------
//
// A formula with a large automorphism group is *compressible*: one representative clause per orbit,
// plus the generators of the group, reconstructs the whole formula. This is the "symmetry = compression"
// thesis made into a re-checkable certificate — `symmetry_entropy_bits` (log₂|Aut|) is exactly the bits
// the group saves, and here that saving is realized as bytes and independently verified.

/// The fixed byte cost of the group-expansion decoder ("take `rep`, apply the group generated by
/// `gens`, output the union"). A documented constant so the inequality `K̄(F) ≤ K̄(rep) + K̄(gens) + O(1)`
/// is concrete.
pub const GROUP_DECODER_OVERHEAD: usize = 8;

/// A certified statement that `F` is described by an orbit-representative set plus automorphism
/// generators — with all three description lengths, the group-entropy, and a self-contained decode
/// witness. [`StructuralBound::verify`] re-derives `F`, `rep`, and the generators from the three
/// descriptors and confirms the generators are automorphisms and that expanding `rep` by the group
/// they generate reproduces `F` exactly.
#[derive(Clone, Debug)]
pub struct StructuralBound {
    /// The number of Boolean variables `F` is defined over.
    pub num_vars: usize,
    /// K̄(F): the flat description of the whole formula.
    pub whole: DescriptionBound,
    /// K̄(rep): the description of one clause per orbit.
    pub rep: DescriptionBound,
    /// K̄(gens): the description of the automorphism generators.
    pub gens: DescriptionBound,
    /// log₂|Aut(F)| — the bits of symmetry the group compresses out.
    pub group_entropy_bits: f64,
}

impl StructuralBound {
    /// The group description length: `K̄(rep) + K̄(gens) + O(1)`.
    pub fn group_bytes(&self) -> usize {
        self.rep.bytes + self.gens.bytes + GROUP_DECODER_OVERHEAD
    }

    /// Whether the group description is strictly shorter than the flat one — a *certified compression*
    /// by symmetry. (When it is not, we honestly report the flat encoding as the better bound.)
    pub fn is_compression(&self) -> bool {
        self.group_bytes() < self.whole.bytes
    }

    /// The best certified upper bound on `K(F)`: the smaller of the flat and group descriptions.
    pub fn best_bytes(&self) -> usize {
        self.whole.bytes.min(self.group_bytes())
    }

    /// Re-check the whole certificate from scratch, trusting nothing the producer computed:
    /// (1) all three decode witnesses round-trip; (2) every recovered generator is a genuine
    /// automorphism of the recovered `F`; (3) expanding `rep` by the group the generators generate
    /// reproduces `F` exactly (so `rep` + `gens` is a *complete* description).
    pub fn verify(&self) -> bool {
        // (1) all three decode witnesses round-trip.
        if !(self.whole.verify() && self.rep.verify() && self.gens.verify()) {
            return false;
        }
        // Recover F, rep, and the generators from the descriptors alone — trust nothing precomputed.
        let (nv_f, f_clauses) = match decode_cnf(&self.whole.descriptor) {
            Some(x) => x,
            None => return false,
        };
        let (_, rep_clauses) = match decode_cnf(&self.rep.descriptor) {
            Some(x) => x,
            None => return false,
        };
        let (nv_g, gens) = match decode_gens(&self.gens.descriptor) {
            Some(x) => x,
            None => return false,
        };
        if nv_f != self.num_vars || nv_g != self.num_vars {
            return false;
        }
        // (2) every recovered generator is a genuine automorphism of the recovered F.
        if !gens
            .iter()
            .all(|p| crate::symmetry_detect::perm_is_automorphism(&f_clauses, p))
        {
            return false;
        }
        // (3) expanding rep by the group the generators generate reproduces F exactly.
        reconstructs(&f_clauses, &rep_clauses, &gens)
    }
}

/// Decode a CNF descriptor back to `(num_vars, clauses)` — `None` on a corrupt/tampered witness.
fn decode_cnf(d: &Descriptor) -> Option<(usize, Vec<Vec<Lit>>)> {
    let Descriptor::IntSeq { encoded } = d else {
        return None;
    };
    unflatten_cnf(&describe::decode_int_seq(encoded)?)
}

/// Decode a generator descriptor back to `(num_vars, generators)` — `None` on a corrupt/tampered witness.
fn decode_gens(d: &Descriptor) -> Option<(usize, Vec<Perm>)> {
    let Descriptor::IntSeq { encoded } = d else {
        return None;
    };
    unflatten_gens(&describe::decode_int_seq(encoded)?)
}

/// Build the structural bound for `F` under a set of candidate `generators`. Returns `None` if any
/// generator is not an automorphism, or if `rep` + generators does not reconstruct `F` (so we never
/// issue a certificate we could not re-check).
pub fn structural_bound(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    generators: &[Perm],
) -> Option<StructuralBound> {
    if !generators
        .iter()
        .all(|g| crate::symmetry_detect::perm_is_automorphism(clauses, g))
    {
        return None;
    }
    let orbits = crate::hypercube::clause_orbits(clauses, generators);
    let rep_clauses: Vec<Vec<Lit>> = orbits
        .iter()
        .filter_map(|o| o.first().map(|&i| clauses[i].clone()))
        .collect();
    // The representative set plus the group must reconstruct the whole formula.
    if !reconstructs(clauses, &rep_clauses, generators) {
        return None;
    }
    Some(StructuralBound {
        num_vars,
        whole: DescriptionBound::of_int_seq(&flatten_cnf(num_vars, clauses)),
        rep: DescriptionBound::of_int_seq(&flatten_cnf(num_vars, &rep_clauses)),
        gens: DescriptionBound::of_int_seq(&flatten_gens(num_vars, generators)),
        group_entropy_bits: crate::hypercube::symmetry_entropy_bits(num_vars, clauses),
    })
}

/// Whether `rep_clauses` plus the group generated by `gens` reconstructs `f_clauses` exactly — checked
/// WITHOUT enumerating the (astronomically large) group. The caller has verified `f_clauses` is closed
/// under each generator (`perm_is_automorphism`), so `group·rep ⊆ F` follows from `rep ⊆ F`, and
/// `F ⊆ group·rep` holds iff every orbit of F under the generators contains a rep clause (any F clause
/// shares an orbit — hence a group element — with the rep of that orbit).
fn reconstructs(f_clauses: &[Vec<Lit>], rep_clauses: &[Vec<Lit>], gens: &[Perm]) -> bool {
    let f_keys = clause_set(f_clauses);
    let rep_keys = clause_set(rep_clauses);
    if !rep_keys.is_subset(&f_keys) {
        return false; // rep must be clauses of F
    }
    let orbits = crate::hypercube::clause_orbits(f_clauses, gens);
    orbits.iter().all(|orbit| {
        orbit
            .iter()
            .any(|&i| rep_keys.contains(&crate::symmetry_detect::clause_key(&f_clauses[i])))
    })
}

/// The set of canonical clause keys — order- and literal-order-independent, so two formulas compare
/// equal iff they are the same clause set.
fn clause_set(clauses: &[Vec<Lit>]) -> BTreeSet<Vec<u32>> {
    clauses
        .iter()
        .map(|c| crate::symmetry_detect::clause_key(c))
        .collect()
}

/// Encode a literal as `2·var + sign` (0 = positive), the dense non-negative code.
fn lit_code(l: Lit) -> i64 {
    (l.var() as i64) * 2 + if l.is_positive() { 0 } else { 1 }
}

/// The inverse of [`lit_code`]. `None` on a negative/garbage code (a tampered descriptor).
fn lit_from_code(code: i64) -> Option<Lit> {
    if code < 0 {
        return None;
    }
    Some(Lit::new((code / 2) as Var, code % 2 == 0))
}

/// Flatten a CNF to `[num_vars, num_clauses, len₀, lit₀₀, …, len₁, …]` for description.
fn flatten_cnf(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<i64> {
    let mut out = vec![num_vars as i64, clauses.len() as i64];
    for c in clauses {
        out.push(c.len() as i64);
        for &l in c {
            out.push(lit_code(l));
        }
    }
    out
}

/// The inverse of [`flatten_cnf`]. `None` on a malformed/tampered sequence.
fn unflatten_cnf(flat: &[i64]) -> Option<(usize, Vec<Vec<Lit>>)> {
    let mut it = flat.iter().copied();
    let num_vars = usize::try_from(it.next()?).ok()?;
    let num_clauses = usize::try_from(it.next()?).ok()?;
    let mut clauses = Vec::with_capacity(num_clauses.min(4096));
    for _ in 0..num_clauses {
        let len = usize::try_from(it.next()?).ok()?;
        let mut c = Vec::with_capacity(len.min(4096));
        for _ in 0..len {
            c.push(lit_from_code(it.next()?)?);
        }
        clauses.push(c);
    }
    if it.next().is_some() {
        return None; // trailing garbage
    }
    Some((num_vars, clauses))
}

/// Flatten generators to `[num_gens, num_vars, σ₀(+0), σ₀(+1), …, σ₁(+0), …]` for description.
fn flatten_gens(num_vars: usize, gens: &[Perm]) -> Vec<i64> {
    let mut out = vec![gens.len() as i64, num_vars as i64];
    for g in gens {
        for v in 0..num_vars {
            out.push(lit_code(g.apply(Lit::pos(v as Var))));
        }
    }
    out
}

/// The inverse of [`flatten_gens`]. `None` on a malformed/tampered sequence.
fn unflatten_gens(flat: &[i64]) -> Option<(usize, Vec<Perm>)> {
    let mut it = flat.iter().copied();
    let num_gens = usize::try_from(it.next()?).ok()?;
    let num_vars = usize::try_from(it.next()?).ok()?;
    let mut gens = Vec::with_capacity(num_gens.min(4096));
    for _ in 0..num_gens {
        let mut images = Vec::with_capacity(num_vars.min(4096));
        for _ in 0..num_vars {
            images.push(lit_from_code(it.next()?)?);
        }
        gens.push(Perm::from_images(images));
    }
    if it.next().is_some() {
        return None;
    }
    Some((num_vars, gens))
}

// ---- Class-relative incompressibility: "no linear (GF(2)) symmetry shortcut" -----------------
//
// A formula's parity (GF(2)) structure is *exactly* characterized by the rank and null space of the
// XOR system latent in its clauses. When that structure is fully exposed — every parity constraint
// recovered, the solution space pinned to exactly 2^k over the k-dimensional kernel — there is no
// further LINEAR collapse to find: any residual hardness is non-linear. This turns the par32-style
// "linearly-rigid kernel" *measurement* into a re-checkable *certificate*, class-relative to the
// linear/parity class 𝒟 (an exact claim, no appeal to uncomputable universal K).

/// The maximum variable count for which we ship an explicit GF(2) kernel basis (the strongest
/// witness): `gf2::solve_gf2` packs each row into a `u64`. Larger systems (par32-scale) are a
/// documented follow-on requiring incremental kernel-basis extraction from [`crate::xor_engine`].
const RIGIDITY_MAX_VARS: usize = 64;

/// The prover's own description budget — **the operational Chaitin ceiling.** Kolmogorov complexity
/// lower bounds are only certifiable up to what the prover can itself re-check; beyond a resource
/// bound we decline rather than over-claim. Every *lower-bound* (incompressibility) path is gated by
/// a `Budget` and returns a documented [`Refusal`] instead of a certificate when a bound is exceeded.
/// (Upper bounds — [`DescriptionBound`] — are always safe to compute: you can only be pleasantly
/// surprised by a shorter description.)
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// The largest GF(2) system (in variables) whose kernel basis we will materialize and re-check.
    pub max_gaussian_dim: usize,
}

impl Budget {
    /// The standard budget: caps the GF(2) kernel-basis materialization at `RIGIDITY_MAX_VARS`.
    pub fn standard() -> Budget {
        Budget {
            max_gaussian_dim: RIGIDITY_MAX_VARS,
        }
    }
}

/// Why a lower-bound certificate was **not** issued — the documented refusal that makes the Chaitin
/// ceiling operational. A refusal is never a claim that no bound exists; it is an honest "this is
/// beyond what I can re-check within budget."
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The GF(2) system exceeds the budget's variable cap (par32-scale needs the incremental engine).
    OverBudgetGaussian { dim: usize, cap: usize },
    /// There is no linear (parity) structure in the formula to certify rigid.
    NoLinearStructure,
}

/// A re-checkable certificate that `F`'s parity structure admits no linear symmetry shortcut beyond
/// its exposed kernel: the GF(2) coefficient system has rank `rank`, its solution space is exactly
/// `2^solution_count_log2` (spanned by `kernel_basis`), and this is the complete linear structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinearRigidityCert {
    /// The number of Boolean variables `F` is over.
    pub num_vars: usize,
    /// The GF(2) coefficient rows recovered from `F` — bit `v` set iff variable `v` occurs in that
    /// parity equation (the right-hand side is irrelevant to the linear structure).
    pub rows: Vec<u64>,
    /// The rank of the coefficient system — the number of independent parity constraints.
    pub rank: usize,
    /// A basis for the null space (the linear symmetry directions). Exactly `num_vars − rank` vectors.
    pub kernel_basis: Vec<Vec<bool>>,
    /// `log₂` of the number of GF(2) solutions — `= kernel_basis.len()`.
    pub solution_count_log2: u32,
}

/// Certify the linear (GF(2)) rigidity of `F` under the standard budget — `None` on any refusal
/// (no parity structure, or over budget). See [`certify_linear_rigidity_within`] for the documented
/// refusal.
pub fn certify_linear_rigidity(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<LinearRigidityCert> {
    certify_linear_rigidity_within(num_vars, clauses, &Budget::standard()).ok()
}

/// Certify the linear (GF(2)) rigidity of `F`, failing **closed** to `budget`: a system that exceeds
/// the budget's Gaussian cap returns [`Refusal::OverBudgetGaussian`] — the operational Chaitin
/// ceiling — rather than a certificate we could not re-check.
pub fn certify_linear_rigidity_within(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    budget: &Budget,
) -> Result<LinearRigidityCert, Refusal> {
    if num_vars > budget.max_gaussian_dim {
        return Err(Refusal::OverBudgetGaussian {
            dim: num_vars,
            cap: budget.max_gaussian_dim,
        });
    }
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return Err(Refusal::NoLinearStructure);
    }
    let rows = eqs_to_rows(&eqs, num_vars);
    let sol = crate::gf2::solve_gf2(num_vars, &rows, &vec![false; rows.len()])
        .ok_or(Refusal::NoLinearStructure)?;
    let rank = num_vars - sol.kernel_basis.len();
    Ok(LinearRigidityCert {
        num_vars,
        rows,
        rank,
        solution_count_log2: sol.kernel_basis.len() as u32,
        kernel_basis: sol.kernel_basis,
    })
}

/// Re-check a [`LinearRigidityCert`] against `F`, trusting nothing the producer computed: re-extract
/// the parity system, independently recompute its kernel, and confirm the certificate's basis is a
/// genuine, independent, complete null-space of the recovered rows.
pub fn check_linear_rigidity(cert: &LinearRigidityCert, clauses: &[Vec<Lit>]) -> bool {
    if cert.num_vars > RIGIDITY_MAX_VARS {
        return false;
    }
    // Re-extract the parity system straight from the clauses.
    let eqs = crate::lyapunov::extract_xor(cert.num_vars, clauses);
    if eqs.is_empty() {
        return false;
    }
    let rows = eqs_to_rows(&eqs, cert.num_vars);
    // The recovered coefficient rows must match the certificate's.
    let recovered: BTreeSet<u64> = rows.iter().copied().collect();
    let claimed: BTreeSet<u64> = cert.rows.iter().copied().collect();
    if recovered != claimed {
        return false;
    }
    // Independently recompute the true rank / kernel dimension from the recovered rows.
    let sol = match crate::gf2::solve_gf2(cert.num_vars, &rows, &vec![false; rows.len()]) {
        Some(s) => s,
        None => return false,
    };
    let true_kernel_dim = sol.kernel_basis.len();
    if cert.rank != cert.num_vars - true_kernel_dim
        || cert.solution_count_log2 as usize != true_kernel_dim
        || cert.kernel_basis.len() != true_kernel_dim
    {
        return false;
    }
    // Every certificate basis vector must be a genuine homogeneous solution (rows·v = 0)…
    for v in &cert.kernel_basis {
        if v.len() != cert.num_vars || rows.iter().any(|&row| gf2_dot(row, v)) {
            return false;
        }
    }
    // …and the basis must be linearly independent (hence a complete null-space by its dimension).
    independent_gf2(&cert.kernel_basis)
}

/// The par32-scale linear-structure certificate: the GF(2) rank and kernel dimension of `F`'s parity
/// system, valid for **any** number of variables (beyond the `u64` cap of [`LinearRigidityCert`]).
/// It carries no explicit kernel basis — at this scale the witness is *recomputation*: re-extract the
/// parity system, re-reduce it, and confirm the same rank and kernel dimension. This is the object
/// par32's "157-dim linearly-rigid kernel" measurement becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinearStructureCert {
    /// The number of Boolean variables `F` is over.
    pub num_vars: usize,
    /// The number of XOR (parity) equations recovered from `F`.
    pub num_xor_eqs: usize,
    /// The rank of the GF(2) system (independent parity constraints), from RREF reduction.
    pub rank: usize,
    /// The dimension of the linear choice space — the free variables occurring in the reduced matrix.
    pub kernel_dim: usize,
}

/// Certify the GF(2) linear structure of `F` at any scale via the incremental engine, or `None` if
/// there is no parity structure to characterize.
pub fn certify_linear_structure(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<LinearStructureCert> {
    let eqs = crate::lyapunov::extract_xor(num_vars, clauses);
    if eqs.is_empty() {
        return None;
    }
    let inc = crate::xor_engine::IncXor::new(num_vars, &eqs);
    Some(LinearStructureCert {
        num_vars,
        num_xor_eqs: eqs.len(),
        rank: inc.rank(),
        kernel_dim: inc.kernel_dim(),
    })
}

/// Re-check a [`LinearStructureCert`] by re-extracting `F`'s parity system, re-reducing it, and
/// confirming the certificate's rank, kernel dimension, and equation count are exactly reproduced.
pub fn check_linear_structure(cert: &LinearStructureCert, clauses: &[Vec<Lit>]) -> bool {
    let eqs = crate::lyapunov::extract_xor(cert.num_vars, clauses);
    if eqs.len() != cert.num_xor_eqs {
        return false;
    }
    let inc = crate::xor_engine::IncXor::new(cert.num_vars, &eqs);
    inc.rank() == cert.rank && inc.kernel_dim() == cert.kernel_dim
}

/// The honest verdict on whether `F` admits a **linear (GF(2)) symmetry shortcut** — the certified,
/// re-checkable "no shortcut of this class" answer the dispatcher and diagnostics can report instead
/// of silently spinning a symmetry search. Class-relative (linear/parity), with the Chaitin ceiling
/// as the documented frame: we certify the linear structure exactly, never claim an absolute bound.
#[derive(Clone, Debug)]
pub enum LinearShortcut {
    /// `F`'s parity structure is fully exposed and rigid — no linear shortcut beyond the certified
    /// `2^kernel_dim` solution space. The strongest available witness is attached: an explicit kernel
    /// basis ([`LinearRigidityCert`]) when the system fits the `u64` budget, always the incremental
    /// dimensions ([`LinearStructureCert`]).
    None {
        rigidity: Option<LinearRigidityCert>,
        structure: LinearStructureCert,
    },
    /// `F` carries no parity structure at all — the linear class offers no leverage (and nothing to
    /// break), so a linear-symmetry search would find nothing.
    NoLinearStructure,
}

/// Decide the linear-shortcut verdict for `F` (fail-closed via [`certify_linear_structure`]).
pub fn linear_shortcut_verdict(num_vars: usize, clauses: &[Vec<Lit>]) -> LinearShortcut {
    match certify_linear_structure(num_vars, clauses) {
        None => LinearShortcut::NoLinearStructure,
        Some(structure) => {
            let rigidity = certify_linear_rigidity(num_vars, clauses);
            LinearShortcut::None {
                rigidity,
                structure,
            }
        }
    }
}

/// The variable cap for the exact rigidity check in [`incompressibility_gate`]: the automorphism search
/// is superpolynomial, so past this size the gate declines (conservative) rather than pay for it.
const GATE_SYMMETRY_MAX_VARS: usize = 48;

/// The SAT-dispatcher gate: `Some(cert)` iff `F`'s parity structure is fully exposed AND `F` is provably
/// rigid (`|Aut| = 1`), so the symmetry arsenal is **provably useless** and the solver may go straight to
/// CDCL with an honest "no shortcut of this class" verdict. The exact rigidity check
/// (`symmetry_entropy_bits == 0`) is size-gated — past `GATE_SYMMETRY_MAX_VARS` the gate declines rather
/// than run the superpolynomial automorphism search (there is no cheap sound global-asymmetry test; it is
/// graph-isomorphism-hard). Fail-closed throughout: any doubt returns `None` and the arsenal runs as usual.
pub fn incompressibility_gate(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<LinearStructureCert> {
    let structure = certify_linear_structure(num_vars, clauses)?;
    if num_vars > GATE_SYMMETRY_MAX_VARS {
        return None; // conservative: the exact rigidity check would be too expensive at this size
    }
    if crate::hypercube::symmetry_entropy_bits(num_vars, clauses) == 0.0 {
        Some(structure) // linear structure + |Aut| = 1 ⇒ genuinely no symmetry shortcut
    } else {
        None
    }
}

/// Pack each XOR equation's variable set into a `u64` coefficient row.
fn eqs_to_rows(eqs: &[crate::xorsat::XorEquation], num_vars: usize) -> Vec<u64> {
    eqs.iter()
        .map(|e| {
            e.vars
                .iter()
                .filter(|&&v| v < num_vars.min(64))
                .fold(0u64, |m, &v| m | (1u64 << v))
        })
        .collect()
}

/// GF(2) dot product of a coefficient row (bitmask) with a solution vector: the parity of the
/// selected entries.
fn gf2_dot(row: u64, v: &[bool]) -> bool {
    let mut acc = false;
    let mut r = row;
    while r != 0 {
        let i = r.trailing_zeros() as usize;
        if i < v.len() {
            acc ^= v[i];
        }
        r &= r - 1;
    }
    acc
}

// ---- The incompressibility lemma (the lower-bound / "prove" direction) ----------------------
//
// The counting core of algorithmic information theory: over a binary alphabet there are `2ⁿ` strings
// of length `n` but only `Σ_{i<n} 2ⁱ = 2ⁿ − 1` programs *shorter* than `n`. Fewer descriptions than
// objects ⇒ by pigeonhole at least one length-`n` string has no shorter program: it is incompressible,
// `K(x) ≥ n`. This is a *lower* bound proved by a shortage of descriptions — the incompressibility
// method — and it is certified by the very same `O(1)` pigeonhole engine the SAT side already trusts.

/// The **incompressibility lemma** for length `n`, as a re-checkable counting certificate: `2ⁿ`
/// strings (pigeons) against `2ⁿ − 1` shorter programs (holes) ⇒ an incompressible string exists.
/// `None` only when `n` is out of the exact-`u128` range (`1 ≤ n ≤ 127`). Re-check with
/// [`crate::pigeonhole::check_counting_cert`].
pub fn incompressible_string_exists(n: u32) -> Option<crate::pigeonhole::CountingCert> {
    if n == 0 || n > 127 {
        return None; // n=0 is the trivial K(ε) ≥ 0; beyond 127, 2ⁿ overflows u128
    }
    let strings: u128 = 1u128 << n;
    let shorter_programs: u128 = strings - 1; // 2⁰ + … + 2ⁿ⁻¹ = 2ⁿ − 1
    crate::pigeonhole::certify_pigeonhole_unsat(strings, shorter_programs)
}

/// A kernel-re-checkable counting certificate that an incompressible Boolean function on `n` variables MUST
/// exist. A function is its `2ⁿ`-bit truth table, and there are only `2^{2ⁿ} − 1` programs shorter than
/// `2ⁿ` bits — too few to name all `2^{2ⁿ}` functions — so at least one has no shorter description. This is
/// the certified REASON the census residue is nonempty: [`boolean_function_census`] measures it, counting
/// guarantees it. Any such function necessarily sits in the residue (no arsenal compresses the truly
/// incompressible). `None` for `n = 0` or `n > 6` (a `2⁷`-bit truth table overflows the exact-`u128`
/// counting range). Re-check with [`crate::pigeonhole::check_counting_cert`].
pub fn certified_incompressible_function_exists(n: u32) -> Option<crate::pigeonhole::CountingCert> {
    if n == 0 || n > 6 {
        return None;
    }
    incompressible_string_exists(1u32 << n)
}

// ---- Incompressibility as a cryptographic diagnostic ----------------------------------------
//
// A key or ciphertext must be *incompressible*: if the description engine finds a shorter program
// for it, THAT program is the attack — a short description means the data is predictable (low
// Kolmogorov complexity). This is the algorithmic-information view of "looks random". It is honest
// and one-sided: by the Chaitin ceiling we can certify WEAKNESS (exhibit a compression witness that
// re-decodes to the data) but never absolute strength — incompressibility here is NECESSARY for a
// secure key/ciphertext, not sufficient, and only relative to this fixed generator class. (Finding
// the shortest *linear* recurrence is exactly the classic LFSR / Berlekamp–Massey attack.)

/// The algorithmic-information verdict on key or ciphertext material (a byte string).
#[derive(Clone, Debug)]
pub enum CryptoStrength {
    /// A description shorter than the raw bytes exists — a **certified structural weakness**. `witness`
    /// decodes back to the data (the concrete attack), and `ratio` = K̄/n < 1 quantifies predictability.
    Weak {
        witness: DescriptionBound,
        ratio: f64,
    },
    /// No description beats storing the raw bytes — incompressible relative to the engine's generator
    /// class. A necessary (not sufficient) condition for cryptographic randomness; no weakness of this
    /// class. (`ratio ≥ 1`.)
    IncompressibleInClass { ratio: f64 },
}

/// The incompressibility ratio `K̄(x)/n` for an `n`-byte string — the shortest menu description
/// measured against storing the bytes raw (the incompressible size for byte material). ≈ 1.0 ⇒
/// incompressible (no exploitable structure of this class); well below 1.0 ⇒ compressible (a short,
/// predictable description exists).
pub fn incompressibility_ratio(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 1.0;
    }
    let ints: Vec<i64> = data.iter().map(|&b| b as i64).collect();
    describe::describe_int_seq(&ints).len() as f64 / data.len() as f64
}

/// The **GF(2⁸) word linear complexity** of a byte string — the length of the shortest *byte-oriented*
/// (GF(256)) LFSR that generates it, via Berlekamp–Massey over the AES field. The word analogue of the
/// bit linear complexity: low relative to `n/2` ⇒ a word-LFSR keystream, a certified weakness. (Runs in
/// `O(n²)` byte-ops — 64× fewer than the bit-level BM on the same data.)
pub fn gf256_word_complexity(data: &[u8]) -> usize {
    let elems: Vec<describe::Gf256> = data.iter().map(|&b| describe::Gf256(b)).collect();
    describe::berlekamp_massey_field(&elems).0
}

/// The **2-adic complexity** of a byte string (its LSB-first bit expansion) — the size of the shortest
/// FCSR (feedback-with-carry / add-with-carry) generating it. Low relative to the bit count ⇒ a
/// carry-based keystream that **fools every linear-complexity test** (Berlekamp–Massey over any field
/// sees high complexity; the carry is nonlinear over GF(2)). The certified weakness the linear tools miss.
pub fn two_adic_complexity_of_bytes(data: &[u8]) -> usize {
    let bits: Vec<bool> = data
        .iter()
        .flat_map(|&b| (0..8).map(move |j| (b >> j) & 1 == 1))
        .collect();
    describe::two_adic_complexity(&bits)
}

/// The **maximal order complexity** of a byte string (its LSB-first bit expansion) — the length of the
/// shortest feedback register, LINEAR OR NONLINEAR, generating it. The TOP of the FSR hierarchy: it
/// catches nonlinear generators (NFSRs, algebraic combiners) that fool every linear-complexity measure.
/// Low relative to the bit count ⇒ a short-register generator. This is the last cheap rung — a general
/// nonlinear feedback function is a full truth table (as large as the data), so a low MOC is a certified
/// STRUCTURAL weakness (a short register exists) even though recovering its *sparse* form is the (hard)
/// algebraic attack. For a real cipher MOC `≈ n/2`: the incompressible residue, the Chaitin ceiling.
pub fn maximal_order_complexity_of_bytes(data: &[u8]) -> usize {
    let bits: Vec<bool> = data
        .iter()
        .flat_map(|&b| (0..8).map(move |j| (b >> j) & 1 == 1))
        .collect();
    describe::maximal_order_complexity(&bits)
}

/// The certified result of an **algebraic-recurrence attack**: a low-degree nonlinear feedback register
/// that regenerates a byte string, recovered by [`describe::detect_algebraic_recurrence`]. This is the
/// OPEN rung — where maximal order complexity can only *measure* a nonlinear register (its `2^order`
/// truth table), the algebraic attack *recovers* it as a sparse ANF and thereby *compresses* it, when
/// the feedback has low degree. The `anf` is a re-checkable witness (replay it with the seed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlgebraicAttack {
    /// The register length `L` — how many past bits the feedback reads.
    pub order: usize,
    /// The ANF degree bound `d` the recovery used.
    pub degree: usize,
    /// The number of active ANF coefficients — the description size, `O(Lᵈ)`.
    pub anf_terms: usize,
    /// The recovered ANF over `monomials(order, degree)` — the witness (`algebraic_generate` replays it).
    pub anf: Vec<bool>,
    /// `2^order` — the truth-table size maximal order complexity would need for the same register.
    pub truth_table: usize,
}

/// Run the algebraic attack on `data` (its LSB-first bit expansion) at maximal ANF degree `max_degree`,
/// searching register orders `1..=max_order`: return the shortest low-degree nonlinear feedback that
/// regenerates the whole bit stream, with its sparse ANF. `None` when no degree-≤`max_degree` register
/// up to `max_order` fits — the genuine high-degree / incompressible residue (a real cipher's keystream,
/// the Chaitin ceiling). `max_order` bounds the cost: the solve is `O(rows · M²/64)` with `M = O(Lᵈ)`.
pub fn algebraic_attack_on_bytes(
    data: &[u8],
    max_degree: usize,
    max_order: usize,
) -> Option<AlgebraicAttack> {
    let bits: Vec<bool> = data
        .iter()
        .flat_map(|&b| (0..8).map(move |j| (b >> j) & 1 == 1))
        .collect();
    let cap = max_order.min(bits.len() / 2);
    for l in 1..=cap {
        if let Some(anf) = describe::detect_algebraic_recurrence(&bits, l, max_degree) {
            return Some(AlgebraicAttack {
                order: l,
                degree: max_degree,
                anf_terms: anf.iter().filter(|&&c| c).count().max(1),
                anf,
                truth_table: 1usize.checked_shl(l as u32).unwrap_or(usize::MAX),
            });
        }
    }
    None
}

/// A certified **combiner leak**: one candidate LFSR that a keystream measurably correlates with,
/// recovered by [`describe::correlation_attack`]. The `attack.init_state` is the re-checkable witness.
#[derive(Clone, Debug, PartialEq)]
pub struct CombinerLeak {
    /// Index into the candidate menu of the leaking register.
    pub candidate_index: usize,
    /// The recovered register (initial state + correlation edge).
    pub attack: describe::CorrelationAttack,
    /// The spurious-bias floor for this register length and sample count.
    pub floor: f64,
    /// `bias / floor` — how far above the noise the correlation sits (the significance).
    pub margin: f64,
}

/// Scan a byte keystream against a menu of candidate LFSR feedback tap-sets: return every register the
/// keystream correlates with beyond `significance ×` the spurious floor. Each hit is a certified break of
/// a nonlinear **combiner** generator — a HIDDEN constituent register recovered *independently*
/// (Siegenthaler divide-and-conquer), collapsing a `2^(Σ Lⱼ)` search to `Σ 2^Lⱼ`. This reaches the
/// combiners the algebraic-recurrence rung structurally cannot (their output is a function of the hidden
/// register outputs, not of the keystream's own past). Empty ⇒ no first-order correlation with any
/// candidate: correlation-immune, or not this combiner — the ceiling, where higher-order and
/// fast-correlation attacks take over.
pub fn scan_for_combiner_leaks(
    keystream: &[u8],
    candidates: &[Vec<bool>],
    significance: f64,
) -> Vec<CombinerLeak> {
    let bits: Vec<bool> = keystream
        .iter()
        .flat_map(|&b| (0..8).map(move |j| (b >> j) & 1 == 1))
        .collect();
    let mut leaks = Vec::new();
    for (i, taps) in candidates.iter().enumerate() {
        let Some(attack) = describe::correlation_attack(&bits, taps) else {
            continue;
        };
        let floor = describe::spurious_bias_floor(taps.len(), attack.samples);
        if floor > 0.0 && attack.bias > significance * floor {
            leaks.push(CombinerLeak {
                candidate_index: i,
                margin: attack.bias / floor,
                floor,
                attack,
            });
        }
    }
    leaks
}

/// Fast correlation attack (Meier–Staffelbach): recover a leaking LFSR's initial state from a noisy
/// keystream by DECODING the register-as-linear-code, in time polynomial in `L` — where the exhaustive
/// correlation attack ([`scan_for_combiner_leaks`], Rung E) needs `O(2^L)`. This is what scales the
/// correlation break to the register lengths real ciphers use. See [`describe::fast_correlation_attack`].
pub fn fast_correlation_attack(
    keystream: &[bool],
    taps: &[bool],
    max_iters: usize,
) -> Option<Vec<bool>> {
    describe::fast_correlation_attack(keystream, taps, max_iters)
}

/// Break a shrinking (clock-controlled) generator: recover both register states from the output alone by
/// guessing the clock register and linear-solving the data register. Reaches a generator whose output is
/// a data-dependent decimation — not a fixed function of any register's own past — that no feedback,
/// correlation, algebraic, or linear rung can touch. See [`describe::attack_shrinking_generator`].
pub fn attack_shrinking_generator(
    output: &[bool],
    a_taps: &[bool],
    s_taps: &[bool],
) -> Option<(Vec<bool>, Vec<bool>)> {
    describe::attack_shrinking_generator(output, a_taps, s_taps)
}

// ---- The auto-lens-finder: a portfolio meta-dispatcher that forces every lens to declare itself -------
//
// The whole campaign is a menu of LENSES, each compressing (and thereby breaking) a different structured
// family. The covering question is: run them ALL against an object and see which one — if any — fires.
// The union of the lenses covers the union of their families; whatever no lens compresses is the
// incompressible residue relative to this arsenal — the operational Chaitin ceiling. No arrangement can
// cover the WHOLE space (counting says most sequences are incompressible, and Chaitin forbids certifying
// which), but we can cover the structured families and MEASURE the hole. Lenses are tried cheapest-first.

/// One lens's verdict on a sequence: the number of bits it needs to DESCRIBE the sequence (`usize::MAX`
/// if the lens finds no exploitable structure). Lower ⇒ the lens compresses it more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensCoverage {
    pub lens: &'static str,
    pub description_bits: usize,
}

/// The result of running the whole lens portfolio against a bit sequence.
#[derive(Clone, Debug)]
pub struct LensReport {
    pub length_bits: usize,
    /// Every lens's description-length verdict (the covering menu, laid out).
    pub coverage: Vec<LensCoverage>,
    /// The lens achieving the shortest description — the one that "covers" the sequence.
    pub best_lens: &'static str,
    pub best_description_bits: usize,
    /// Whether the best lens compresses by at least 2× — i.e. the sequence is genuinely covered, not
    /// in the incompressible residue.
    pub covered: bool,
}

/// The **auto-lens-finder**: run every sequence lens in the arsenal against `bits` and report which one
/// compresses it (covers it) and by how much — or that none do, placing it in the incompressible residue.
/// This makes the covering explicit: each structured family is caught by its own lens, and a
/// cryptographically-random sequence falls through all of them (the ceiling). Lenses are ordered
/// cheapest-first so a covered sequence is recognized quickly.
pub fn lens_report(bits: &[bool]) -> LensReport {
    let n = bits.len();
    let mut coverage = Vec::new();

    // Linear (LFSR): a length-L register describes it in ~2L bits (taps + seed).
    let lc = describe::berlekamp_massey_gf2(bits).0;
    coverage.push(LensCoverage {
        lens: "linear (LFSR / Berlekamp–Massey)",
        description_bits: lc.saturating_mul(2),
    });

    // 2-adic (FCSR): a carry register of complexity c describes it in ~2c bits.
    let tc = describe::two_adic_complexity(bits);
    coverage.push(LensCoverage {
        lens: "2-adic (FCSR)",
        description_bits: tc.saturating_mul(2),
    });

    // Maximal-order: a nonlinear register of order L, but its feedback is a 2^L truth table — it only
    // COMPRESSES when the register is tiny (otherwise it merely measures).
    let moc = describe::maximal_order_complexity(bits);
    let moc_bits = if moc < 20 {
        (1usize << moc).saturating_add(moc)
    } else {
        usize::MAX
    };
    coverage.push(LensCoverage {
        lens: "maximal-order (nonlinear FSR)",
        description_bits: moc_bits,
    });

    // Algebraic recurrence (degree ≤ 2): a low-degree nonlinear feedback, sparse ANF (bounded order search).
    let alg = (1..=12).find_map(|l| {
        describe::detect_algebraic_recurrence(bits, l, 2)
            .map(|c| l + c.iter().filter(|&&b| b).count())
    });
    coverage.push(LensCoverage {
        lens: "algebraic-recurrence (deg-2 ANF)",
        description_bits: alg.unwrap_or(usize::MAX),
    });

    // MDL codec menu: affine / geometric / polynomial / periodic / sparse / RLE / … over the byte packing.
    let bytes = describe::bits_to_bytes(bits);
    let enc = describe::describe_int_seq(&bytes).len().saturating_mul(8);
    coverage.push(LensCoverage {
        lens: "MDL codec menu",
        description_bits: enc,
    });

    let best = coverage
        .iter()
        .min_by_key(|c| c.description_bits)
        .expect("nonempty");
    let covered = best.description_bits.saturating_mul(2) < n;
    LensReport {
        length_bits: n,
        best_lens: best.lens,
        best_description_bits: best.description_bits,
        covered,
        coverage,
    }
}

/// A coverage census over a corpus of sequences: how many the lens arsenal covers, how many fall in the
/// uncovered residue, and which lens covered how many. This is the covering problem made countable.
#[derive(Clone, Debug)]
pub struct CoverageCensus {
    pub total: usize,
    /// Covered by some lens (compressed ≥ 2×).
    pub covered: usize,
    /// The incompressible residue — covered by nothing in the arsenal.
    pub uncovered: usize,
    /// How many sequences each lens was the best cover for.
    pub by_lens: Vec<(&'static str, usize)>,
}

/// Census a corpus of bit sequences through the [`lens_report`] portfolio: tally what each lens covers
/// and what lands in the uncovered residue.
pub fn census(corpus: &[Vec<bool>]) -> CoverageCensus {
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    let mut covered = 0;
    for seq in corpus {
        let r = lens_report(seq);
        if r.covered {
            covered += 1;
            *counts.entry(r.best_lens).or_insert(0) += 1;
        }
    }
    CoverageCensus {
        total: corpus.len(),
        covered,
        uncovered: corpus.len() - covered,
        by_lens: counts.into_iter().collect(),
    }
}

/// Exhaustively census EVERY length-`len` bit sequence (`2^len` of them): what fraction of the WHOLE
/// space does the lens arsenal cover? The answer is a small sliver — most sequences are incompressible,
/// the residue — which is the concrete, countable face of the Chaitin ceiling. The structured families
/// our lenses catch are real and important, but they are a vanishing fraction of the space; you cannot
/// arrange lenses to cover it all (counting forbids it), and you cannot even certify which points are
/// uncovered (Chaitin forbids that). `len ≤ ~18` to stay tractable.
pub fn exhaustive_coverage(len: usize) -> CoverageCensus {
    let corpus: Vec<Vec<bool>> = (0u64..(1u64 << len))
        .map(|code| (0..len).map(|i| (code >> i) & 1 == 1).collect())
        .collect();
    census(&corpus)
}

/// The trace of recursively symmetry-breaking an object down to its incompressible core.
#[derive(Clone, Debug)]
pub struct RecursiveReduction {
    /// The size (bytes) at each recursion level — level 0 is the input, the last is the fixed point.
    pub sizes: Vec<usize>,
    /// How many symmetry breaks (compressions) before the fixed point.
    pub depth: usize,
    /// The size of the irreducible core: what the arsenal cannot reduce further.
    pub irreducible_bytes: usize,
    /// Whether any reduction happened at all.
    pub compressed: bool,
}

/// **Recursively symmetry-break** an object until nothing reduces it further. Each level applies the
/// compression lens (the MDL description menu) and recurses on the DESCRIPTION — each compression is a
/// symmetry break, and its output becomes the next object. The recursion terminates at the FIXED POINT:
/// the point where no lens shrinks it, the incompressible core.
///
/// The terminus is `no lens fires` — which is *"irreducible by this arsenal"*, and is NOT, and can never
/// be, *"structureless."* Proving no structure exists would be certifying a Kolmogorov lower bound
/// `K(x) > c`, which the kernel-certified Chaitin theorem in this module says a bounded system can do for
/// no object past its budget. So this function computes the fixed point; the thing it can never output is
/// a proof that the fixed point has no structure.
pub fn recursive_reduce(bytes: &[u8]) -> RecursiveReduction {
    let mut cur: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
    let mut sizes = vec![cur.len()];
    loop {
        let enc = describe::describe_int_seq(&cur);
        if enc.len() >= cur.len() {
            break; // no lens shrinks it — the incompressible fixed point (relative to the arsenal)
        }
        cur = enc.iter().map(|&b| b as i64).collect();
        sizes.push(cur.len());
    }
    RecursiveReduction {
        depth: sizes.len() - 1,
        irreducible_bytes: *sizes.last().expect("nonempty"),
        compressed: sizes.len() > 1,
        sizes,
    }
}

/// The linear-cryptanalytic profile of a Boolean combining/filter function, from its Walsh spectrum: the
/// best linear approximation (the distinguisher), its nonlinearity, and its correlation-immunity order.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearDistinguisher {
    /// The linear mask `w` of the best approximation `z ≈ ⟨w, inputs⟩`.
    pub mask: usize,
    /// The bias `|Pr[C=⟨w,x⟩] − ½|` of that approximation — the distinguishing advantage.
    pub bias: f64,
    /// The Hamming weight of `mask`: how many registers the approximation combines (weight 1 = Rung E).
    pub mask_weight: u32,
    /// Distance to the nearest affine function; maximal ⇒ bent ⇒ no exploitable linear approximation.
    pub nonlinearity: u64,
    /// The correlation-immunity order — how many registers the first-order correlation attack must miss.
    pub immunity_order: usize,
}

/// The linear-cryptanalysis profile of a combining/filter function given as its `2ⁿ` truth table: the
/// whole Walsh spectrum distilled into the best linear approximation, its nonlinearity, and its
/// correlation-immunity order. Where [`scan_for_combiner_leaks`] (Rung E) reads only weight-1 masks, this
/// reads them ALL — surfacing the multi-register approximation (`mask_weight ≥ 2`) that E is blind to,
/// even on a first-order correlation-immune function. `None` for a malformed table.
pub fn linear_cryptanalysis(truth: &[bool]) -> Option<LinearDistinguisher> {
    let (mask, bias) = describe::best_linear_approximation(truth, true)?;
    Some(LinearDistinguisher {
        mask,
        bias,
        mask_weight: mask.count_ones(),
        nonlinearity: describe::nonlinearity(truth)?,
        immunity_order: describe::correlation_immunity_order(truth)?,
    })
}

/// The algebraic-immunity profile of a filter/combining function: the minimum degree of an annihilator
/// (the algebraic attack's leverage), the maximum possible `⌈n/2⌉`, and whether it sits at that ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlgebraicImmunityReport {
    /// `AI(C)` — the minimum degree of a nonzero annihilator of `C` or `C ⊕ 1`.
    pub immunity: usize,
    /// `⌈n/2⌉` — the maximum algebraic immunity any `n`-variable function can have.
    pub max_possible: usize,
    /// A re-checkable minimum-degree annihilator (the attack's leverage).
    pub witness: describe::AnnihilatorWitness,
    /// `AI(C) == ⌈n/2⌉` — maximal algebraic immunity, the algebraic-incompressible residue (the ceiling).
    pub is_maximal: bool,
}

/// The algebraic-immunity profile of a Boolean function given as its `2ⁿ` truth table. Low immunity is a
/// certified structural weakness: a degree-`AI` annihilator turns each keystream bit of a filter
/// generator using `C` into a degree-`AI` equation in the secret state ([`algebraic_filter_attack`]).
/// Maximal immunity (`AI = ⌈n/2⌉`) is the ceiling — no low-degree relation to exploit. `None` for a
/// malformed table.
pub fn algebraic_immunity_of(truth: &[bool]) -> Option<AlgebraicImmunityReport> {
    let (immunity, witness) = describe::algebraic_immunity(truth)?;
    let n = truth.len().trailing_zeros() as usize;
    let max_possible = n.div_ceil(2);
    Some(AlgebraicImmunityReport {
        immunity,
        max_possible,
        witness,
        is_maximal: immunity == max_possible,
    })
}

/// Recover the initial state of a filter generator (a length-`L` LFSR with feedback `taps` filtered by
/// `filter_truth`) via the algebraic attack — the certified break of a target the correlation and Walsh
/// rungs only glimpse statistically. See [`describe::algebraic_filter_attack`]. Returns the secret
/// initial state (verified by regeneration) or `None`.
pub fn algebraic_filter_attack(
    keystream: &[bool],
    taps: &[bool],
    filter_truth: &[bool],
) -> Option<Vec<bool>> {
    describe::algebraic_filter_attack(keystream, taps, filter_truth)
}

// ---- The hypercube structure finder: classify a Boolean function by walking its cube -----------------
//
// A length-`2ⁿ` truth table IS a labeling of the corners of the `n`-cube. The structure finder walks that
// cube through each structural lens — each lens is a genuine traversal of the cube along one axis of
// structure — and reports the TIGHTEST class that describes the whole function in far fewer than `2ⁿ`
// bits, with a re-checkable decode witness:
//
//   • Constant       — every corner equal (1 bit).                      walk: read the corners.
//   • Junta(k)       — only k of n variables ever change the output.    walk: probe each coordinate edge.
//   • Affine         — degree ≤ 1: f(x) = ⊕ aᵢxᵢ ⊕ c (n+1 bits).        walk: the Walsh–Hadamard butterfly.
//   • Symmetric      — depends only on Hamming weight (n+1 bits).        walk: the weight shells.
//   • LowDegree(d)   — a sparse ANF: few monomials describe it.         walk: the Möbius butterfly.
//   • ResistedArsenal— dense high-degree ANF, all vars, high nonlin.    the residue: no lens fires.
//
// The residue is `irreducible by this arsenal`, NOT `structureless` — the Chaitin ceiling (this module)
// forbids certifying the latter. It is exactly the class that indistinguishability obfuscation must live
// in: a function whose structure NO adversary lens reads off. The 2001 impossibility of *ideal*
// obfuscation is the cryptographic cousin of Chaitin — the code always reveals at least itself — which is
// why iO settles for *indistinguishability*, hiding which structured program you hold inside the residue.

/// The compressibility class of a Boolean function on the hypercube, tightest description first. Each
/// variant carries a re-checkable witness ([`CubeStructure::reconstruct`]) except the residue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CubeStructure {
    /// Every corner carries the same value — the whole cube in one bit.
    Constant(bool),
    /// A junta: the output depends only on `relevant` (a strict subset of the variables); `subtable` is
    /// its `2^{|relevant|}` truth table over those variables (indexed LSB-first in `relevant` order).
    Junta {
        relevant: Vec<usize>,
        subtable: Vec<bool>,
    },
    /// Affine (degree ≤ 1): `f(x) = ⊕ᵢ coeffs[i]·xᵢ ⊕ constant`.
    Affine { coeffs: Vec<bool>, constant: bool },
    /// Symmetric: the output depends only on the Hamming weight of the input; `by_weight[w]` is its value
    /// on every corner of weight `w` (length `n+1`).
    Symmetric { by_weight: Vec<bool> },
    /// Low algebraic degree with a sparse ANF: `anf` is the `2ⁿ` Möbius-transform witness, `degree` its top.
    LowDegree { degree: usize, anf: Vec<bool> },
    /// No lens compresses it: dense high-degree ANF, all variables relevant, high nonlinearity. The
    /// incompressible residue relative to this arsenal — NOT a proof of structurelessness (Chaitin).
    ResistedArsenal { nonlinearity: u64, degree: usize },
}

impl CubeStructure {
    /// Rebuild the `2ⁿ` truth table from this description — the re-checkable decode witness. `None` for the
    /// residue (which carries no compressed description) or a malformed witness.
    pub fn reconstruct(&self, n: usize) -> Option<Vec<bool>> {
        let size = 1usize << n;
        match self {
            CubeStructure::Constant(b) => Some(vec![*b; size]),
            CubeStructure::Junta { relevant, subtable } => {
                if subtable.len() != 1usize << relevant.len() {
                    return None;
                }
                Some(
                    (0..size)
                        .map(|x| {
                            let c = relevant.iter().enumerate().fold(0usize, |acc, (bit, &v)| {
                                if x & (1 << v) != 0 {
                                    acc | (1 << bit)
                                } else {
                                    acc
                                }
                            });
                            subtable[c]
                        })
                        .collect(),
                )
            }
            CubeStructure::Affine { coeffs, constant } => {
                if coeffs.len() != n {
                    return None;
                }
                Some(
                    (0..size)
                        .map(|x| {
                            coeffs
                                .iter()
                                .enumerate()
                                .fold(*constant, |acc, (i, &a)| acc ^ (a && (x & (1 << i) != 0)))
                        })
                        .collect(),
                )
            }
            CubeStructure::Symmetric { by_weight } => {
                if by_weight.len() != n + 1 {
                    return None;
                }
                Some(
                    (0..size)
                        .map(|x| by_weight[(x as u64).count_ones() as usize])
                        .collect(),
                )
            }
            // The ANF↔truth Möbius transform is its own inverse over GF(2).
            CubeStructure::LowDegree { anf, .. } => {
                if anf.len() != size {
                    return None;
                }
                describe::anf(anf)
            }
            CubeStructure::ResistedArsenal { .. } => None,
        }
    }
}

/// The verdict of the hypercube structure finder: the tightest structural class, its description size in
/// bits, and how that compares to the raw `2ⁿ`-bit truth table.
#[derive(Clone, Debug)]
pub struct StructureReport {
    pub num_vars: usize,
    pub class: CubeStructure,
    /// `2ⁿ` — the bits to store the truth table corner-by-corner.
    pub raw_bits: usize,
    /// The bits of the tightest structural description found.
    pub description_bits: usize,
    /// Whether any lens beat storing the raw truth table.
    pub compressed: bool,
}

/// Bits needed to name one of `n` variables.
fn var_index_bits(n: usize) -> usize {
    if n <= 1 {
        1
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize
    }
}

/// `Σ_{k=0}^{d} C(n,k)` — the number of ANF coefficient bits for all monomials of degree `≤ d`, i.e. the
/// DENSE encoding size of a degree-`d` function (vs the sparse `weight · n` monomial list).
fn partial_binomial_sum(n: usize, d: usize) -> usize {
    let mut sum = 0usize;
    let mut c = 1usize;
    for k in 0..=d.min(n) {
        sum = sum.saturating_add(c);
        c = c.saturating_mul(n - k) / (k + 1);
    }
    sum
}

/// If the function depends only on Hamming weight, its value on each weight shell `0..=n`; else `None`.
fn symmetric_profile(truth: &[bool], n: usize) -> Option<Vec<bool>> {
    let mut by_weight: Vec<Option<bool>> = vec![None; n + 1];
    for (x, &val) in truth.iter().enumerate() {
        let w = (x as u64).count_ones() as usize;
        match by_weight[w] {
            None => by_weight[w] = Some(val),
            Some(v) if v != val => return None,
            _ => {}
        }
    }
    Some(by_weight.into_iter().map(|o| o.unwrap_or(false)).collect())
}

/// **Walk the hypercube** of a Boolean function (its `2ⁿ` truth table) and return the tightest structural
/// class that describes it, with the achieved description size and a re-checkable witness. Every lens is a
/// traversal of the cube along one axis of structure (coordinate edges, Walsh butterfly, weight shells,
/// Möbius butterfly); the finder reports whichever yields the shortest description, or the residue
/// ([`CubeStructure::ResistedArsenal`]) when none beats storing the table raw. `None` if `truth.len()` is
/// not a power of two.
pub fn find_structure(truth: &[bool]) -> Option<StructureReport> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let raw = truth.len();
    let mut cands: Vec<(CubeStructure, usize)> = Vec::new();

    // Constant — the tightest lens: the whole cube in one bit.
    if truth.iter().all(|&b| b == truth[0]) {
        cands.push((CubeStructure::Constant(truth[0]), 1));
    }

    // Junta — probe each coordinate edge: a variable is relevant iff flipping it ever changes the output.
    let relevant: Vec<usize> = (0..n)
        .filter(|&i| (0..truth.len()).any(|x| truth[x] != truth[x ^ (1 << i)]))
        .collect();
    if !relevant.is_empty() && relevant.len() < n {
        let k = relevant.len();
        let subtable: Vec<bool> = (0..1usize << k)
            .map(|c| {
                let x = relevant.iter().enumerate().fold(0usize, |acc, (bit, &v)| {
                    if c & (1 << bit) != 0 {
                        acc | (1 << v)
                    } else {
                        acc
                    }
                });
                truth[x]
            })
            .collect();
        let bits = k * var_index_bits(n) + (1usize << k);
        cands.push((CubeStructure::Junta { relevant, subtable }, bits));
    }

    // Affine — the Walsh butterfly: nonlinearity 0 ⇔ degree ≤ 1. Read coeffs by walking the n axes.
    if describe::nonlinearity(truth) == Some(0) {
        let constant = truth[0];
        let coeffs: Vec<bool> = (0..n).map(|i| truth[1 << i] ^ constant).collect();
        cands.push((CubeStructure::Affine { coeffs, constant }, n + 1));
    }

    // Symmetric — the weight shells: the output depends only on Hamming weight.
    if let Some(by_weight) = symmetric_profile(truth, n) {
        cands.push((CubeStructure::Symmetric { by_weight }, n + 1));
    }

    // Low degree — the Möbius butterfly. Account the TIGHTER of the sparse monomial list (`weight · n`) and
    // the dense coefficient vector (`Σ_{k≤deg} C(n,k)`), so a DENSE bounded-degree function compresses too.
    if let Some(anf) = describe::anf(truth) {
        let weight = anf.iter().filter(|&&c| c).count();
        let degree = anf
            .iter()
            .enumerate()
            .filter(|(_, &c)| c)
            .map(|(m, _)| (m as u64).count_ones() as usize)
            .max()
            .unwrap_or(0);
        // The dense coefficient vector only counts for genuinely bounded degree (`≤ n−2`); at degree `n−1`
        // it would save a single trivial bit and swallow the honest residue.
        let dense = if degree + 2 <= n {
            partial_binomial_sum(n, degree)
        } else {
            usize::MAX
        };
        let bits = weight.saturating_mul(n).min(dense).max(1);
        cands.push((CubeStructure::LowDegree { degree, anf }, bits));
    }

    let (class, description_bits) = cands
        .into_iter()
        .min_by_key(|(_, b)| *b)
        .filter(|(_, b)| *b < raw)
        .unwrap_or_else(|| {
            let degree = describe::algebraic_degree(truth).unwrap_or(n);
            let nonlinearity = describe::nonlinearity(truth).unwrap_or(0);
            (
                CubeStructure::ResistedArsenal {
                    nonlinearity,
                    degree,
                },
                raw,
            )
        });

    Some(StructureReport {
        num_vars: n,
        compressed: description_bits < raw,
        description_bits,
        raw_bits: raw,
        class,
    })
}

/// A **cover of the cube by structural class**: the ANF degree stratification of a Boolean function. Every
/// function is a XOR of monomials `∏_{i∈S} xᵢ`, and each monomial is a structural class — its degree `|S|`
/// is the order of variable interaction it encodes. Peeling low-degree slices (constant, linear,
/// quadratic, …) covers most of the `2ⁿ` corners with a handful of terms; what remains is the residue: the
/// high-degree core no lower-order slice explains. This is why the peel is more efficient than walking the
/// cube corner by corner — each class-slice accounts for many corners at once, and you reason about the
/// small residue directly instead of labeling all `2ⁿ` of them.
#[derive(Clone, Debug)]
pub struct StructureCover {
    pub num_vars: usize,
    /// `monomials_by_degree[d]` = ANF monomials of degree exactly `d` — the size of the degree-`d` slice.
    pub monomials_by_degree: Vec<usize>,
    /// The total ANF monomials across all slices — the whole description in monomials.
    pub total_monomials: usize,
    /// The highest degree present: the residue's interaction order (0 if the function is a constant).
    pub residue_degree: usize,
    /// The number of monomials at the residue degree — the irreducible high-degree core.
    pub residue_monomials: usize,
    /// Bits to store the ANF as monomial indices (`total · n`), against `2ⁿ` raw.
    pub description_bits: usize,
    pub raw_bits: usize,
    /// Whether the peel is a genuine compression — a sparse ANF beats storing the truth table.
    pub compressed: bool,
}

/// **Peel the cube apart by structural class** and examine the residue: return the ANF degree
/// stratification of a Boolean function (its `2ⁿ` truth table). Each degree is a slice of structure; the
/// top nonempty degree is the residue — the interaction order that survives every lower-order peel, and
/// the honest answer to *what remains and why*. `None` if `truth.len()` is not a power of two.
pub fn structure_cover(truth: &[bool]) -> Option<StructureCover> {
    let anf = describe::anf(truth)?;
    let n = truth.len().trailing_zeros() as usize;
    let mut by_degree = vec![0usize; n + 1];
    for (m, &c) in anf.iter().enumerate() {
        if c {
            by_degree[(m as u64).count_ones() as usize] += 1;
        }
    }
    let total: usize = by_degree.iter().sum();
    let residue_degree = by_degree.iter().rposition(|&c| c > 0).unwrap_or(0);
    let residue_monomials = by_degree[residue_degree];
    let raw = truth.len();
    let description_bits = if total == 0 { 1 } else { total * n };
    Some(StructureCover {
        num_vars: n,
        compressed: description_bits < raw,
        monomials_by_degree: by_degree,
        total_monomials: total,
        residue_degree,
        residue_monomials,
        description_bits,
        raw_bits: raw,
    })
}

// ---- Linear structures: the derivative symmetry the coordinate lenses miss ---------------------------
//
// The lenses above read structure in the GIVEN coordinates. But a function can be simple in a ROTATED
// basis and look dense here: a rotated junta is invariant under some direction `a` that is not an axis, so
// the coordinate walk sees every variable as relevant and the finder calls it residue. The autocorrelation
// exposes exactly this: `a` is a LINEAR STRUCTURE when `f(x⊕a) ⊕ f(x)` is constant (`|r_f(a)| = 2ⁿ`). The
// set of linear structures forms a GF(2) subspace `V(f)`, and `dim V(f) = k > 0` means `f` collapses to an
// `(n−k)`-variable function after a linear change of basis. Peeling `V(f)` is the affine-group symmetry
// break the whole arsenal was missing — and a genuinely random function still has `V(f) = {0}`, the residue
// that survives even this.

/// The linear space `V(f)` of a Boolean function: the directions along which the derivative is constant.
#[derive(Clone, Debug)]
pub struct LinearStructureReport {
    pub num_vars: usize,
    /// A GF(2) echelon basis of `V(f)`; each element is a nonzero variable bitmask.
    pub basis: Vec<usize>,
    /// For each basis vector `a`, the constant value of `f(x⊕a) ⊕ f(x)`: `false` = an invariance (period),
    /// `true` = a complement (always flips the output).
    pub derivative: Vec<bool>,
}

impl LinearStructureReport {
    /// `dim V(f)` — the number of dimensions a linear change of basis can peel off.
    pub fn dim(&self) -> usize {
        self.basis.len()
    }
    /// Whether the function carries any linear structure the coordinate lenses miss.
    pub fn is_reducible(&self) -> bool {
        !self.basis.is_empty()
    }
    /// Re-check the witness against the truth table: each basis vector is nonzero and independent, and its
    /// recorded derivative is genuinely constant across the whole cube.
    pub fn verify(&self, truth: &[bool]) -> bool {
        if self.basis.len() != self.derivative.len() {
            return false;
        }
        if gf2_echelon_basis(&self.basis).len() != self.basis.len() {
            return false; // the recorded basis is not independent
        }
        for (&a, &c) in self.basis.iter().zip(&self.derivative) {
            if a == 0 || a >= truth.len() {
                return false;
            }
            if !(0..truth.len()).all(|x| (truth[x] ^ truth[x ^ a]) == c) {
                return false;
            }
        }
        true
    }
}

/// A xor-basis (echelon over GF(2)) of the span of `vectors` — each returned mask has a distinct highest
/// set bit, and the count is the GF(2) rank.
fn gf2_echelon_basis(vectors: &[usize]) -> Vec<usize> {
    let mut basis: Vec<usize> = Vec::new();
    for &v in vectors {
        let mut x = v;
        for &b in &basis {
            x = x.min(x ^ b);
        }
        if x != 0 {
            basis.push(x);
            basis.sort_unstable_by(|a, b| b.cmp(a));
        }
    }
    basis
}

/// Reduce `x` modulo an echelon basis, clearing every pivot bit (leaving the coset representative that is
/// zero on all pivot positions).
fn reduce_by_basis(mut x: usize, basis: &[usize]) -> usize {
    for &b in basis {
        let hb = 1usize << (usize::BITS - 1 - b.leading_zeros());
        if x & hb != 0 {
            x ^= b;
        }
    }
    x
}

/// Detect the linear space `V(f)` of a Boolean function from its autocorrelation. `None` if `truth.len()`
/// is not a power of two.
pub fn linear_structures(truth: &[bool]) -> Option<LinearStructureReport> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let r = describe::autocorrelation(truth)?;
    let full = truth.len() as i64;
    let structs: Vec<usize> = (1..truth.len()).filter(|&a| r[a].abs() == full).collect();
    let basis = gf2_echelon_basis(&structs);
    let derivative = basis.iter().map(|&a| truth[a] ^ truth[0]).collect();
    Some(LinearStructureReport {
        num_vars: n,
        basis,
        derivative,
    })
}

/// The genuine domain compression from the INVARIANCE subspace `V₀(f) = {a : f(x⊕a) = f(x) ∀x}`: an
/// `(n − dim V₀)`-variable truth table carrying all the information, over the surviving `free_positions`. A
/// rotated junta collapses here even though the coordinate junta lens saw every variable as relevant.
#[derive(Clone, Debug)]
pub struct InvarianceReduction {
    pub original_vars: usize,
    pub reduced_vars: usize,
    /// The surviving variable positions (a complement of the invariance pivots).
    pub free_positions: Vec<usize>,
    /// The reduced function over `free_positions`, in the same LSB-first index order.
    pub reduced_truth: Vec<bool>,
}

impl InvarianceReduction {
    /// Re-check that the reduction reproduces the original: every corner maps to the reduced value of its
    /// coset representative.
    pub fn verify(&self, truth: &[bool], invariance_basis: &[usize]) -> bool {
        if self.reduced_truth.len() != 1 << self.reduced_vars {
            return false;
        }
        (0..truth.len()).all(|x| {
            let rep = reduce_by_basis(x, invariance_basis);
            let y = self
                .free_positions
                .iter()
                .enumerate()
                .fold(0usize, |acc, (i, &p)| {
                    if rep & (1 << p) != 0 {
                        acc | (1 << i)
                    } else {
                        acc
                    }
                });
            self.reduced_truth[y] == truth[x]
        })
    }
}

/// Reduce an echelon xor-basis to reduced row echelon: each vector keeps its pivot bit, cleared from all
/// others.
fn gf2_rref(basis: &[usize]) -> Vec<usize> {
    let mut b = basis.to_vec();
    let pivots: Vec<usize> = b
        .iter()
        .map(|&v| 1usize << (usize::BITS - 1 - v.leading_zeros()))
        .collect();
    for i in 0..b.len() {
        for j in 0..b.len() {
            if i != j && b[j] & pivots[i] != 0 {
                b[j] ^= b[i];
            }
        }
    }
    b
}

/// The affine reduction of a Boolean function: `f(x) = h(reduced) ⊕ ℓ(x)`, where `ℓ` is a linear form and
/// `h` collapses onto the free coordinates. This peels the COMPLEMENT linear structures (`D_a f = 1`) that
/// [`reduce_by_invariance`] cannot — a residue XOR a linear form is invisible to every other axis but folds
/// here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffineReduction {
    pub num_vars: usize,
    /// The linear form `ℓ(x) = ⟨c, x⟩` peeled off (as a coefficient bitmask).
    pub linear_form: usize,
    pub invariance_basis: Vec<usize>,
    pub free_positions: Vec<usize>,
    /// The reduced function `h` over `free_positions`.
    pub reduced_truth: Vec<bool>,
}

impl AffineReduction {
    /// Rebuild the truth table: `f(x) = h(coset-rep of x) ⊕ ⟨c, x⟩`.
    pub fn reconstruct(&self) -> Option<Vec<bool>> {
        if self.reduced_truth.len() != 1 << self.free_positions.len() {
            return None;
        }
        Some(
            (0..1usize << self.num_vars)
                .map(|x| {
                    let rep = reduce_by_basis(x, &self.invariance_basis);
                    let y = self
                        .free_positions
                        .iter()
                        .enumerate()
                        .fold(0usize, |acc, (j, &p)| {
                            if rep & (1 << p) != 0 {
                                acc | (1 << j)
                            } else {
                                acc
                            }
                        });
                    self.reduced_truth[y] ^ ((self.linear_form & x).count_ones() % 2 == 1)
                })
                .collect(),
        )
    }
}

/// Peel a linear form off a Boolean function so its complement linear structures become invariances, then
/// reduce. Catches `f = h ⊕ ℓ` where `ℓ` is linear and `h` collapses — the residue-plus-a-linear-form class
/// every other axis misses. Fail-closed: returns `Some` only if the reconstruction re-checks exactly.
/// `None` when there are no complement structures (pure invariance is [`reduce_by_invariance`]'s job).
pub fn affine_reduce(truth: &[bool]) -> Option<AffineReduction> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let ls = linear_structures(truth)?;
    if !ls.derivative.iter().any(|&d| d) {
        return None; // no complement structure — nothing beyond invariance
    }
    // The linear form: 1 on the pivots whose RREF basis vector has a complement derivative.
    let rref = gf2_rref(&ls.basis);
    let mut c = 0usize;
    for &b in &rref {
        if truth[b] ^ truth[0] {
            c |= 1usize << (usize::BITS - 1 - b.leading_zeros());
        }
    }
    let h: Vec<bool> = (0..truth.len())
        .map(|x| truth[x] ^ ((c & x).count_ones() % 2 == 1))
        .collect();
    let (red, inv_basis) = reduce_by_invariance(&h)?;
    let out = AffineReduction {
        num_vars: n,
        linear_form: c,
        invariance_basis: inv_basis,
        free_positions: red.free_positions,
        reduced_truth: red.reduced_truth,
    };
    (out.reconstruct().as_deref() == Some(truth)).then_some(out)
}

/// Peel the invariance subspace off a Boolean function: quotient out every direction `a` with `f(x⊕a) =
/// f(x)` and return the smaller function on the surviving coordinates, together with the invariance basis
/// that certifies it. `None` when the invariance subspace is trivial (nothing to reduce).
pub fn reduce_by_invariance(truth: &[bool]) -> Option<(InvarianceReduction, Vec<usize>)> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let r = describe::autocorrelation(truth)?;
    let full = truth.len() as i64;
    let invariances: Vec<usize> = (1..truth.len()).filter(|&a| r[a] == full).collect();
    let basis = gf2_echelon_basis(&invariances);
    if basis.is_empty() {
        return None;
    }
    let pivots: Vec<usize> = basis
        .iter()
        .map(|&b| (usize::BITS - 1 - b.leading_zeros()) as usize)
        .collect();
    let free_positions: Vec<usize> = (0..n).filter(|p| !pivots.contains(p)).collect();
    let reduced_vars = free_positions.len();
    let reduced_truth: Vec<bool> = (0..1usize << reduced_vars)
        .map(|y| {
            let x = free_positions
                .iter()
                .enumerate()
                .fold(0usize, |acc, (i, &p)| {
                    if y & (1 << i) != 0 {
                        acc | (1 << p)
                    } else {
                        acc
                    }
                });
            truth[x]
        })
        .collect();
    Some((
        InvarianceReduction {
            original_vars: n,
            reduced_vars,
            free_positions,
            reduced_truth,
        },
        basis,
    ))
}

// ---- Affine equivalence: the AGL(n,2)-invariant signature, the rung above linear structures ----------
//
// Linear structures peel ONE rotation; the rung above asks whether two functions are the SAME up to any
// invertible linear change of basis (the affine group AGL(n,2)). The Walsh-spectrum MULTISET is the
// invariant: applying a linear map `x → Ax` only PERMUTES the Walsh coefficients, so the multiset of
// amplitudes is unchanged — affine-equivalent functions share it. A single nonzero amplitude is the
// PLATEAUED class (bent when that amplitude is `2^{n/2}`), and the amplitude `2^{(n+k)/2}` reads off the
// linear-space dimension `k` — unifying this rung with the one below it. A generic function has a spread
// of amplitudes and belongs to no small affine class: the residue.

/// The AGL(n,2)-invariant signature of a Boolean function: its Walsh amplitude distribution and the
/// plateaued/bent classification that distribution induces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffineSignature {
    pub num_vars: usize,
    /// The multiset `|Ŵ(w)| → count`, sorted by amplitude — invariant under any affine change of variables.
    pub walsh_abs_distribution: Vec<(u64, usize)>,
    /// Exactly one nonzero amplitude ⇒ PLATEAUED (maximal nonlinearity for its linear-space dimension).
    pub is_plateaued: bool,
    /// The plateau amplitude `2^{(n+k)/2}` when plateaued, else `None`.
    pub amplitude: Option<u64>,
    /// Plateaued with amplitude `2^{n/2}` ⇒ BENT: perfectly nonlinear, trivial linear space (`k = 0`).
    pub is_bent: bool,
}

impl AffineSignature {
    /// The linear-space dimension `k` implied by the plateau amplitude `2^{(n+k)/2}` (so `k = 2·log₂amp −
    /// n`), matching [`linear_structures`]`.dim()`. `None` when not plateaued.
    pub fn implied_linear_dim(&self) -> Option<usize> {
        let amp = self.amplitude?;
        if !amp.is_power_of_two() {
            return None;
        }
        let log = amp.trailing_zeros() as usize;
        (2 * log).checked_sub(self.num_vars)
    }
}

/// Compute the affine-equivalence signature of a Boolean function from its Walsh spectrum. `None` if
/// `truth.len()` is not a power of two.
pub fn affine_signature(truth: &[bool]) -> Option<AffineSignature> {
    let n = truth.len().trailing_zeros() as usize;
    let spec = describe::walsh_spectrum(truth)?;
    let mut counts: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for &c in &spec {
        *counts.entry(c.unsigned_abs()).or_default() += 1;
    }
    let nonzero: Vec<u64> = counts.keys().copied().filter(|&v| v != 0).collect();
    let is_plateaued = nonzero.len() == 1;
    let amplitude = is_plateaued.then(|| nonzero[0]);
    let bent_amp = (n % 2 == 0).then(|| 1u64 << (n / 2));
    let is_bent = amplitude.is_some() && amplitude == bent_amp;
    Some(AffineSignature {
        num_vars: n,
        walsh_abs_distribution: counts.into_iter().collect(),
        is_plateaued,
        amplitude,
        is_bent,
    })
}

// ---- Separability: the direct-sum symmetry — independent blocks solved apart ------------------------
//
// A different axis than the linear-group rungs: does the function SPLIT into independent pieces? `f(x) =
// g(x_A) ⊕ h(x_B)` on disjoint variable sets `A, B` is the cube analogue of a SAT instance whose clause
// graph has two components — you solve the pieces apart instead of the whole. The blocks are exactly the
// connected components of the ANF variable-interaction graph (two variables are linked when a monomial
// contains both), because every monomial lives inside one component. Splitting turns one `2ⁿ` table into
// `Σ 2^{|Bᵢ|}` — an exponential collapse — and surfaces the true independent subsystems. A function whose
// interaction graph is connected is a single irreducible block: no direct-sum symmetry to break.

/// A direct-sum decomposition `f = constant ⊕ ⊕ᵢ fᵢ(x_{Bᵢ})` over independent variable blocks.
#[derive(Clone, Debug)]
pub struct SeparableDecomposition {
    pub num_vars: usize,
    /// The XOR of the empty monomial — the standalone constant term.
    pub constant: bool,
    /// A partition of the RELEVANT variables into independent blocks (each sorted; irrelevant vars omitted).
    pub blocks: Vec<Vec<usize>>,
    /// Each block's function as a `2^{|Bᵢ|}` truth table, aligned with `blocks` (LSB-first in block order).
    pub block_truths: Vec<Vec<bool>>,
}

impl SeparableDecomposition {
    /// Whether the function splits into two or more independent blocks.
    pub fn is_separable(&self) -> bool {
        self.blocks.len() >= 2
    }
    /// The total bits of the block tables — `Σ 2^{|Bᵢ|}`, against `2ⁿ` for the whole truth table.
    pub fn table_bits(&self) -> usize {
        self.block_truths.iter().map(|t| t.len()).sum()
    }
    /// Rebuild the `2ⁿ` truth table — the re-checkable witness. `None` for a malformed decomposition.
    pub fn reconstruct(&self, n: usize) -> Option<Vec<bool>> {
        let size = 1usize << n;
        let mut out = vec![self.constant; size];
        for (b, t) in self.blocks.iter().zip(&self.block_truths) {
            if t.len() != 1 << b.len() {
                return None;
            }
            for (x, slot) in out.iter_mut().enumerate() {
                let y = b.iter().enumerate().fold(0usize, |acc, (j, &i)| {
                    if x & (1 << i) != 0 {
                        acc | (1 << j)
                    } else {
                        acc
                    }
                });
                *slot ^= t[y];
            }
        }
        Some(out)
    }
}

fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Decompose a Boolean function into its independent direct-sum blocks (the connected components of its ANF
/// interaction graph). `None` if `truth.len()` is not a power of two.
pub fn separable_decomposition(truth: &[bool]) -> Option<SeparableDecomposition> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let anf = describe::anf(truth)?;
    let constant = anf[0];
    let mut parent: Vec<usize> = (0..n).collect();
    let mut relevant = vec![false; n];
    for (m, &c) in anf.iter().enumerate() {
        if !c || m == 0 {
            continue;
        }
        let bits: Vec<usize> = (0..n).filter(|&i| m & (1 << i) != 0).collect();
        for &i in &bits {
            relevant[i] = true;
        }
        for w in bits.windows(2) {
            let (ra, rb) = (uf_find(&mut parent, w[0]), uf_find(&mut parent, w[1]));
            parent[ra] = rb;
        }
    }
    let mut comp: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for i in 0..n {
        if relevant[i] {
            let r = uf_find(&mut parent, i);
            comp.entry(r).or_default().push(i);
        }
    }
    let blocks: Vec<Vec<usize>> = comp.into_values().collect();
    let mut block_truths = Vec::with_capacity(blocks.len());
    for b in &blocks {
        let bmask: usize = b.iter().fold(0, |acc, &i| acc | (1 << i));
        let mut sub = vec![false; 1usize << b.len()];
        for (m, &c) in anf.iter().enumerate() {
            if !c || m == 0 || m & !bmask != 0 {
                continue;
            }
            let sub_m = b.iter().enumerate().fold(0usize, |acc, (j, &i)| {
                if m & (1 << i) != 0 {
                    acc | (1 << j)
                } else {
                    acc
                }
            });
            sub[sub_m] = true;
        }
        block_truths.push(describe::anf(&sub)?); // Möbius is its own inverse: sub-ANF → sub-truth
    }
    Some(SeparableDecomposition {
        num_vars: n,
        constant,
        blocks,
        block_truths,
    })
}

// ---- Variable automorphisms: the permutation-symmetry group Aut(f) ----------------------------------
//
// The `symmetric` lens is the special case `Aut(f) = Sₙ` — invariance under EVERY variable permutation.
// This rung finds the whole permutation group `Aut(f) = {σ : f(σ·x) = f(x)}` and certifies it with a
// Schreier–Sims BSGS. It catches partial symmetry the full-symmetric lens declares residue: a
// ROTATION-symmetric function (invariant under a cyclic shift of the variables — an entire class in stream
// ciphers) is a permutation symmetry, NOT a linear one, so every rung of the linear ladder misses it. The
// group compresses the `2ⁿ` table to one value per input-orbit — exactly `n+1` weight classes when the
// group is `Sₙ`, recovering the symmetric lens as the top of this ladder.

/// The variable-permutation symmetry of a Boolean function: a certified subgroup `G ≤ Aut(f)` and the
/// input-orbit compression it induces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariableSymmetry {
    pub num_vars: usize,
    /// Generators of `G` (each a permutation of the `n` variables) — every one a verified automorphism.
    pub generators: Vec<Vec<usize>>,
    /// `|G|` — the certified group order from Schreier–Sims.
    pub order: u128,
    /// The number of orbits of inputs `{0,1}ⁿ` under `G`: the compressed table size (one value per orbit).
    pub orbit_count: usize,
    /// One value per orbit, indexed by the orbit's minimal representative in ascending order.
    pub orbit_values: Vec<bool>,
}

impl VariableSymmetry {
    /// Whether the function has any nontrivial variable-permutation symmetry.
    pub fn is_nontrivial(&self) -> bool {
        self.order > 1
    }
    /// Rebuild the `2ⁿ` truth table by replaying the group action and painting each orbit its value — the
    /// re-checkable witness. `None` for a malformed report.
    pub fn reconstruct(&self, n: usize) -> Option<Vec<bool>> {
        let size = 1usize << n;
        if self.orbit_values.len() != self.orbit_count {
            return None;
        }
        let mut orbs = self.input_orbits(size)?;
        orbs.sort_by_key(|o| *o.iter().min().expect("nonempty orbit"));
        if orbs.len() != self.orbit_count {
            return None;
        }
        let mut out = vec![false; size];
        for (k, o) in orbs.iter().enumerate() {
            for &x in o {
                out[x] = self.orbit_values[k];
            }
        }
        Some(out)
    }
    /// Re-check: every generator is a genuine automorphism and the orbit painting reproduces `f`.
    pub fn verify(&self, truth: &[bool]) -> bool {
        for s in &self.generators {
            if s.len() != self.num_vars || !is_variable_automorphism(truth, s) {
                return false;
            }
        }
        self.reconstruct(self.num_vars).as_deref() == Some(truth)
    }
    fn input_orbits(&self, size: usize) -> Option<Vec<Vec<usize>>> {
        let lifted: Vec<Vec<usize>> = self
            .generators
            .iter()
            .map(|s| (0..size).map(|x| permute_input_bits(x, s)).collect())
            .collect();
        Some(if lifted.is_empty() {
            (0..size).map(|x| vec![x]).collect()
        } else {
            crate::permgroup::orbits(size, &lifted)
        })
    }
}

/// Permute the input bits of `x` by the variable permutation `sigma`: bit `i` moves to bit `sigma[i]`.
fn permute_input_bits(x: usize, sigma: &[usize]) -> usize {
    sigma.iter().enumerate().fold(0usize, |acc, (i, &si)| {
        if x & (1 << i) != 0 {
            acc | (1 << si)
        } else {
            acc
        }
    })
}

/// Whether `f(σ·x) = f(x)` for all `x` — i.e. `sigma` is a variable automorphism of the function.
fn is_variable_automorphism(truth: &[bool], sigma: &[usize]) -> bool {
    (0..truth.len()).all(|x| truth[x] == truth[permute_input_bits(x, sigma)])
}

/// Find the variable-permutation symmetry group of a Boolean function: test every transposition and the
/// cyclic shift, certify the subgroup they generate with Schreier–Sims, and compress by input-orbits.
/// `None` if `truth.len()` is not a power of two.
pub fn variable_symmetry(truth: &[bool]) -> Option<VariableSymmetry> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let mut generators: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let mut s: Vec<usize> = (0..n).collect();
            s.swap(i, j);
            if is_variable_automorphism(truth, &s) {
                generators.push(s);
            }
        }
    }
    // Products of two disjoint transpositions — the symmetries no single transposition realizes (e.g. a
    // reflection, or an even permutation fixing f that its transposition factors do not).
    for a in 0..n {
        for b in a + 1..n {
            for c in 0..n {
                if c == a || c == b {
                    continue;
                }
                for d in c + 1..n {
                    if d == a || d == b {
                        continue;
                    }
                    let mut s: Vec<usize> = (0..n).collect();
                    s.swap(a, b);
                    s.swap(c, d);
                    if is_variable_automorphism(truth, &s) {
                        generators.push(s);
                    }
                }
            }
        }
    }
    if n > 1 {
        let rho: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
        if is_variable_automorphism(truth, &rho) {
            generators.push(rho);
        }
    }
    let order = crate::permgroup::schreier_sims(n, &generators).order();
    let size = 1usize << n;
    let sym = VariableSymmetry {
        num_vars: n,
        generators,
        order,
        orbit_count: 0,
        orbit_values: Vec::new(),
    };
    let mut orbs = sym.input_orbits(size)?;
    orbs.sort_by_key(|o| *o.iter().min().expect("nonempty orbit"));
    let orbit_values: Vec<bool> = orbs
        .iter()
        .map(|o| truth[*o.iter().min().expect("nonempty")])
        .collect();
    Some(VariableSymmetry {
        orbit_count: orbs.len(),
        orbit_values,
        ..sym
    })
}

// ---- The unified deep finder: the tightest description across every axis ----------------------------
//
// Each rung is a different symmetry axis, and a function may be simple on one while dense on all the
// others: separable-but-high-degree, rotation-symmetric-but-not-linear, a rotated junta the coordinate
// walk calls residue. The deep finder runs EVERY axis — coordinate lenses, direct-sum separability, the
// Aut(f) permutation group, and linear-invariance reduction — and returns whichever gives the shortest
// description. Only when all of them fail to beat the raw truth table is the verdict `residue`: the honest
// incompressible core relative to the whole arsenal (never a proof of structurelessness — Chaitin).

/// The winning axis and description size of the unified deep structure finder.
#[derive(Clone, Debug)]
pub struct DeepStructureReport {
    pub num_vars: usize,
    pub raw_bits: usize,
    pub description_bits: usize,
    /// Which axis gave the tightest description (`coordinate:*`, `separable`, `permutation`,
    /// `linear-invariance`, or `residue`).
    pub winner: &'static str,
    pub compressed: bool,
}

/// Run every structural axis and return the globally tightest description of a Boolean function. `None` if
/// `truth.len()` is not a power of two.
pub fn find_structure_deep(truth: &[bool]) -> Option<DeepStructureReport> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let raw = truth.len();
    let mut cands: Vec<(&'static str, usize)> = Vec::new();

    if let Some(r) = find_structure(truth) {
        if r.compressed {
            let name = match r.class {
                CubeStructure::Constant(_) => "coordinate:constant",
                CubeStructure::Junta { .. } => "coordinate:junta",
                CubeStructure::Affine { .. } => "coordinate:affine",
                CubeStructure::Symmetric { .. } => "coordinate:symmetric",
                CubeStructure::LowDegree { .. } => "coordinate:low-degree",
                CubeStructure::ResistedArsenal { .. } => "coordinate:residue",
            };
            cands.push((name, r.description_bits));
        }
    }
    if let Some(d) = separable_decomposition(truth) {
        if d.is_separable() {
            cands.push(("separable", d.table_bits()));
        }
    }
    if let Some(s) = variable_symmetry(truth) {
        if s.is_nontrivial() {
            cands.push(("permutation", s.orbit_count));
        }
    }
    if let Some((red, _)) = reduce_by_invariance(truth) {
        cands.push(("linear-invariance", 1usize << red.reduced_vars));
    }
    if let Some(ar) = affine_reduce(truth) {
        cands.push(("affine", ar.reduced_truth.len() + n));
    }

    let (winner, description_bits) = cands
        .into_iter()
        .filter(|(_, b)| *b < raw)
        .min_by_key(|(_, b)| *b)
        .unwrap_or(("residue", raw));
    Some(DeepStructureReport {
        num_vars: n,
        raw_bits: raw,
        description_bits,
        winner,
        compressed: description_bits < raw,
    })
}

// ---- The recursive peel: structure all the way down to the fixed point -----------------------------
//
// The deep finder picks the tightest axis for the WHOLE function. The recursive peel goes further: it
// peels that axis and re-runs itself on the smaller function(s) it exposes — each block of a separable
// split, the reduced function of a linear-invariance peel — until it bottoms out in a coordinate leaf or
// the residue. A function like `((x0⊕x1)∧x2) ⊕ (x3∧x4∧x5)` splits into two blocks, and the first block
// then peels its rotated-junta invariance: a two-level tree the single-pass finder cannot express. The
// whole tree's `reconstruct` is one re-checkable witness for the entire decomposition. Recursion
// terminates because every peel strictly reduces the variable count.

/// A recursive structural decomposition of a Boolean function: the fixed point of the deep finder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructureTree {
    /// A coordinate-lens leaf (constant/junta/affine/symmetric/low-degree), reconstructable from `class`.
    Coordinate {
        class: CubeStructure,
        num_vars: usize,
        bits: usize,
    },
    /// A permutation-symmetry leaf: the orbit painting reconstructs it.
    Permutation {
        sym: VariableSymmetry,
        num_vars: usize,
    },
    /// The incompressible residue — stored raw (no axis compressed it).
    Residue { truth: Vec<bool> },
    /// Split into independent blocks, each recursively decomposed.
    Separable {
        num_vars: usize,
        constant: bool,
        blocks: Vec<(Vec<usize>, StructureTree)>,
    },
    /// Reduced by an invariance subspace; `inner` is the decomposition of the smaller function.
    LinearReduced {
        num_vars: usize,
        invariance_basis: Vec<usize>,
        free_positions: Vec<usize>,
        inner: Box<StructureTree>,
    },
    /// Peeled a linear form `⟨c,x⟩`, then reduced; `inner` decomposes the residual `h`.
    AffineReduced {
        num_vars: usize,
        linear_form: usize,
        invariance_basis: Vec<usize>,
        free_positions: Vec<usize>,
        inner: Box<StructureTree>,
    },
}

impl StructureTree {
    /// The nesting depth of the decomposition (0 for a leaf).
    pub fn depth(&self) -> usize {
        match self {
            StructureTree::Coordinate { .. }
            | StructureTree::Permutation { .. }
            | StructureTree::Residue { .. } => 0,
            StructureTree::Separable { blocks, .. } => {
                1 + blocks.iter().map(|(_, t)| t.depth()).max().unwrap_or(0)
            }
            StructureTree::LinearReduced { inner, .. }
            | StructureTree::AffineReduced { inner, .. } => 1 + inner.depth(),
        }
    }
    /// The total description size in bits: the sum over the leaves, with a small charge per structural node.
    pub fn total_description_bits(&self) -> usize {
        match self {
            StructureTree::Coordinate { bits, .. } => *bits,
            StructureTree::Permutation { sym, .. } => sym.orbit_count,
            StructureTree::Residue { truth } => truth.len(),
            StructureTree::Separable { blocks, .. } => {
                1 + blocks
                    .iter()
                    .map(|(v, t)| v.len() + t.total_description_bits())
                    .sum::<usize>()
            }
            StructureTree::LinearReduced {
                invariance_basis,
                inner,
                ..
            } => invariance_basis.len() + inner.total_description_bits(),
            StructureTree::AffineReduced {
                num_vars, inner, ..
            } => *num_vars + inner.total_description_bits(),
        }
    }
    /// Rebuild the full `2ⁿ` truth table by composing the recursive descriptions — the whole-tree witness.
    pub fn reconstruct(&self) -> Option<Vec<bool>> {
        match self {
            StructureTree::Coordinate {
                class, num_vars, ..
            } => class.reconstruct(*num_vars),
            StructureTree::Permutation { sym, num_vars } => sym.reconstruct(*num_vars),
            StructureTree::Residue { truth } => Some(truth.clone()),
            StructureTree::Separable {
                num_vars,
                constant,
                blocks,
            } => {
                let size = 1usize << num_vars;
                let mut out = vec![*constant; size];
                for (vars, sub) in blocks {
                    let bt = sub.reconstruct()?;
                    for (x, slot) in out.iter_mut().enumerate() {
                        let y = vars.iter().enumerate().fold(0usize, |acc, (j, &i)| {
                            if x & (1 << i) != 0 {
                                acc | (1 << j)
                            } else {
                                acc
                            }
                        });
                        *slot ^= bt[y];
                    }
                }
                Some(out)
            }
            StructureTree::LinearReduced {
                num_vars,
                invariance_basis,
                free_positions,
                inner,
            } => {
                let inner_truth = inner.reconstruct()?;
                Some(
                    (0..1usize << num_vars)
                        .map(|x| {
                            let rep = reduce_by_basis(x, invariance_basis);
                            let y =
                                free_positions
                                    .iter()
                                    .enumerate()
                                    .fold(0usize, |acc, (j, &p)| {
                                        if rep & (1 << p) != 0 {
                                            acc | (1 << j)
                                        } else {
                                            acc
                                        }
                                    });
                            inner_truth[y]
                        })
                        .collect(),
                )
            }
            StructureTree::AffineReduced {
                num_vars,
                linear_form,
                invariance_basis,
                free_positions,
                inner,
            } => {
                let inner_truth = inner.reconstruct()?;
                Some(
                    (0..1usize << num_vars)
                        .map(|x| {
                            let rep = reduce_by_basis(x, invariance_basis);
                            let y =
                                free_positions
                                    .iter()
                                    .enumerate()
                                    .fold(0usize, |acc, (j, &p)| {
                                        if rep & (1 << p) != 0 {
                                            acc | (1 << j)
                                        } else {
                                            acc
                                        }
                                    });
                            inner_truth[y] ^ ((linear_form & x).count_ones() % 2 == 1)
                        })
                        .collect(),
                )
            }
        }
    }
}

/// Recursively decompose a Boolean function to the fixed point of the deep finder: peel the tightest axis,
/// then re-run on the smaller function(s) it exposes, until a coordinate leaf or the residue. `None` if
/// `truth.len()` is not a power of two.
pub fn structure_tree(truth: &[bool]) -> Option<StructureTree> {
    if truth.is_empty() || !truth.len().is_power_of_two() {
        return None;
    }
    let n = truth.len().trailing_zeros() as usize;
    let deep = find_structure_deep(truth)?;
    let tree = match deep.winner {
        "separable" => {
            let d = separable_decomposition(truth)?;
            let mut blocks = Vec::with_capacity(d.blocks.len());
            for (vars, bt) in d.blocks.iter().zip(&d.block_truths) {
                blocks.push((vars.clone(), structure_tree(bt)?));
            }
            StructureTree::Separable {
                num_vars: n,
                constant: d.constant,
                blocks,
            }
        }
        "linear-invariance" => {
            let (red, basis) = reduce_by_invariance(truth)?;
            StructureTree::LinearReduced {
                num_vars: n,
                invariance_basis: basis,
                free_positions: red.free_positions.clone(),
                inner: Box::new(structure_tree(&red.reduced_truth)?),
            }
        }
        "affine" => {
            let ar = affine_reduce(truth)?;
            StructureTree::AffineReduced {
                num_vars: n,
                linear_form: ar.linear_form,
                invariance_basis: ar.invariance_basis,
                free_positions: ar.free_positions,
                inner: Box::new(structure_tree(&ar.reduced_truth)?),
            }
        }
        "permutation" => StructureTree::Permutation {
            sym: variable_symmetry(truth)?,
            num_vars: n,
        },
        "residue" => StructureTree::Residue {
            truth: truth.to_vec(),
        },
        _ => StructureTree::Coordinate {
            class: find_structure(truth)?.class,
            num_vars: n,
            bits: deep.description_bits,
        },
    };
    Some(tree)
}

/// An exhaustive census of the whole Boolean-function space on `n` variables: which structural axis the
/// deep finder wins on, for every one of the `2^{2ⁿ}` functions.
#[derive(Clone, Debug)]
pub struct BooleanCensus {
    pub num_vars: usize,
    /// `2^{2ⁿ}` — the number of Boolean functions on `n` variables.
    pub total: usize,
    /// Each winning axis paired with how many functions it is the tightest description for.
    pub by_winner: Vec<(&'static str, usize)>,
    /// Functions some axis compressed (`total − residue`).
    pub compressed: usize,
    /// Functions no axis compressed — the incompressible residue relative to the whole arsenal.
    pub residue: usize,
}

/// Exhaustively classify **every** Boolean function on `n` variables by the deep finder's winning axis.
/// `None` for `n = 0` or `n > 4` (beyond `2^{2⁴} = 65536` functions the space is astronomically large).
/// The residue count is the concrete, countable face of the Chaitin ceiling: it is a growing fraction of
/// the space as `n` rises — structured functions are `2^{poly}`, the space is `2^{2ⁿ}`.
pub fn boolean_function_census(n: usize) -> Option<BooleanCensus> {
    if n == 0 || n > 4 {
        return None;
    }
    let dim = 1usize << n;
    let total = 1u64 << dim;
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for code in 0..total {
        let truth: Vec<bool> = (0..dim).map(|i| (code >> i) & 1 == 1).collect();
        *counts
            .entry(find_structure_deep(&truth)?.winner)
            .or_default() += 1;
    }
    let residue = counts.get("residue").copied().unwrap_or(0);
    Some(BooleanCensus {
        num_vars: n,
        total: total as usize,
        by_winner: counts.into_iter().collect(),
        compressed: total as usize - residue,
        residue,
    })
}

/// A sampled census of the Boolean-function space on `n` variables, for `n` too large to enumerate.
#[derive(Clone, Debug)]
pub struct SampledCensus {
    pub num_vars: usize,
    pub samples: usize,
    /// How many sampled functions no axis compressed.
    pub residue: usize,
    pub by_winner: Vec<(&'static str, usize)>,
}

impl SampledCensus {
    /// The estimated fraction of the space that is the incompressible residue.
    pub fn residue_fraction(&self) -> f64 {
        self.residue as f64 / self.samples.max(1) as f64
    }
}

/// Estimate the coverage map at `n` variables (where `2^{2ⁿ}` is unenumerable) from a uniform random sample
/// of `samples` functions, classified by the deep finder's winning axis. `None` for `n = 0` or `n > 12`.
/// The residue fraction climbs toward `1` as `n` grows — the asymptotic form of the exhaustive census, and
/// the whole thesis: structured functions vanish and almost every function is incompressible.
pub fn sampled_boolean_census(n: usize, samples: usize, seed: u64) -> Option<SampledCensus> {
    if n == 0 || n > 12 {
        return None;
    }
    let dim = 1usize << n;
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for s in 0..samples {
        let truth: Vec<bool> = (0..dim)
            .map(|i| {
                let mut z = seed
                    .wrapping_add((s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                    .wrapping_add(i as u64 + 1);
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                (z ^ (z >> 31)) & 1 == 1
            })
            .collect();
        *counts
            .entry(find_structure_deep(&truth)?.winner)
            .or_default() += 1;
    }
    let residue = counts.get("residue").copied().unwrap_or(0);
    Some(SampledCensus {
        num_vars: n,
        samples,
        residue,
        by_winner: counts.into_iter().collect(),
    })
}

/// A **certified Kolmogorov upper bound** on a Boolean function: the recursive structure decomposition and
/// its total description size, with a re-checkable decode witness. This is a `K̄` — an UPPER bound only.
/// The kernel-certified Chaitin theorem in this module forbids certifying `K(f) > c` for any function past
/// budget, so a bound that equals the raw `2ⁿ` means "irreducible by this arsenal," never "incompressible."
#[derive(Clone, Debug)]
pub struct KolmogorovBound {
    pub num_vars: usize,
    /// `K̄(f)` — the total description size of the recursive decomposition (a computable upper bound).
    pub bits: usize,
    /// `2ⁿ` — the cost of storing the truth table outright.
    pub raw_bits: usize,
    /// The recursive decomposition itself — the decode witness.
    pub tree: StructureTree,
}

impl KolmogorovBound {
    /// `K̄(f) / 2ⁿ` — → 0 for the algorithmically simplest functions, 1 for the residue.
    pub fn ratio(&self) -> f64 {
        self.bits as f64 / self.raw_bits.max(1) as f64
    }
    /// Whether the decomposition beats storing the truth table.
    pub fn is_compressed(&self) -> bool {
        self.bits < self.raw_bits
    }
    /// Re-check the bound: the decomposition must decode back to the exact function.
    pub fn verify(&self, truth: &[bool]) -> bool {
        self.tree.reconstruct().as_deref() == Some(truth)
    }
}

/// Compute a certified Kolmogorov upper bound for a Boolean function via its recursive structure
/// decomposition. `None` if `truth.len()` is not a power of two.
pub fn kolmogorov_bound(truth: &[bool]) -> Option<KolmogorovBound> {
    let n = truth.len().trailing_zeros() as usize;
    let tree = structure_tree(truth)?;
    Some(KolmogorovBound {
        num_vars: n,
        bits: tree.total_description_bits(),
        raw_bits: truth.len(),
        tree,
    })
}

// ---- Vectorial Boolean functions (S-boxes): the ladder lifted to n→m bit maps ------------------------
//
// A block cipher's S-box is a vectorial Boolean function `S : {0,1}ⁿ → {0,1}ᵐ`, and its cryptographic
// STRENGTH is the absence of the very structure this whole module hunts. Every nonzero output mask `b`
// gives a component function `S_b(x) = ⟨b, S(x)⟩` — a single-output function we feed to the lenses above.
// The S-box is weak exactly when those components carry structure: low LINEARITY (a good linear
// approximation → linear cryptanalysis), low DIFFERENTIAL UNIFORMITY breached (a biased difference →
// differential cryptanalysis), low DEGREE (an algebraic relation), or outright affinity. A strong S-box —
// AES's — is the residue: high nonlinearity, differential uniformity 4, degree 7. "We broke AES" is not a
// claim these numbers support; they are exactly what certifies its S-box is sound.

/// The cryptographic structural profile of an S-box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SboxProfile {
    pub in_bits: usize,
    pub out_bits: usize,
    /// The maximum count `#{x : S(x⊕a) ⊕ S(x) = b}` over `a≠0, b` — differential-cryptanalysis resistance
    /// (lower is stronger; `2` is the optimal APN bound).
    pub differential_uniformity: u32,
    /// `maxₐ,_{b≠0} |Σₓ (−1)^{⟨b,S(x)⟩ ⊕ ⟨a,x⟩}|` — linear-cryptanalysis resistance (lower is stronger).
    pub linearity: u64,
    /// The minimum algebraic degree over the nonzero component functions.
    pub min_degree: usize,
    /// Every component is affine — the trivially-broken S-box.
    pub is_affine: bool,
    /// A permutation (`n = m` and injective).
    pub is_bijective: bool,
    /// Almost perfect nonlinear: differential uniformity `2`, the optimal differential resistance.
    pub is_apn: bool,
}

/// Profile an S-box `S : {0,1}ⁿ → {0,1}ᵐ` given as its output table (`sbox[x] = S(x)`, `out_bits = m`):
/// differential uniformity, linearity, minimum component degree, and the affine/bijective/APN flags.
/// `None` if the table length is not a power of two.
pub fn sbox_profile(sbox: &[u32], out_bits: usize) -> Option<SboxProfile> {
    if sbox.is_empty() || !sbox.len().is_power_of_two() {
        return None;
    }
    let n = sbox.len().trailing_zeros() as usize;
    let m = out_bits;
    let size = sbox.len();

    let mut differential_uniformity = 0u32;
    for a in 1..size {
        let mut count = vec![0u32; 1usize << m];
        for x in 0..size {
            count[(sbox[x] ^ sbox[x ^ a]) as usize] += 1;
        }
        differential_uniformity = differential_uniformity.max(*count.iter().max().unwrap_or(&0));
    }

    let mut linearity = 0u64;
    let mut min_degree = usize::MAX;
    let mut is_affine = true;
    for b in 1..(1usize << m) {
        let comp: Vec<bool> = sbox
            .iter()
            .map(|&y| (b as u32 & y).count_ones() % 2 == 1)
            .collect();
        let spec = describe::walsh_spectrum(&comp)?;
        linearity = linearity.max(spec.iter().map(|&c| c.unsigned_abs()).max().unwrap_or(0));
        let deg = describe::algebraic_degree(&comp)?;
        min_degree = min_degree.min(deg);
        if deg > 1 {
            is_affine = false;
        }
    }

    let mut seen = vec![false; 1usize << m];
    let is_bijective = n == m
        && sbox.iter().all(|&y| {
            let u = y as usize;
            u < seen.len() && !std::mem::replace(&mut seen[u], true)
        });

    Some(SboxProfile {
        in_bits: n,
        out_bits: m,
        differential_uniformity,
        linearity,
        min_degree: if min_degree == usize::MAX {
            0
        } else {
            min_degree
        },
        is_affine,
        is_bijective,
        is_apn: n == m && differential_uniformity == 2,
    })
}

/// The affine-invariant spectra of an S-box: the fingerprints for classification up to affine equivalence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SboxSpectra {
    pub in_bits: usize,
    pub out_bits: usize,
    /// The differential spectrum: the multiset of DDT entries `#{x : S(x⊕a)⊕S(x)=b}` over `a≠0`, as
    /// `(value → count)` sorted. Invariant under any affine change of input or output coordinates.
    pub differential_spectrum: Vec<(u32, usize)>,
    /// The linear (Walsh) spectrum: the multiset of amplitudes `|Σₓ(−1)^{⟨b,S(x)⟩⊕⟨a,x⟩}|` over `a, b≠0`.
    pub walsh_spectrum: Vec<(u64, usize)>,
}

/// The affine-equivalence fingerprint of an S-box: its differential and linear spectra. Two S-boxes that
/// are affine-equivalent (`S′(x) = B·S(A·x⊕a)⊕c` for invertible `A,B`) share both spectra, so a difference
/// in either certifies inequivalence — the necessary test at the heart of S-box classification. `None` if
/// the table length is not a power of two.
pub fn sbox_spectra(sbox: &[u32], out_bits: usize) -> Option<SboxSpectra> {
    if sbox.is_empty() || !sbox.len().is_power_of_two() {
        return None;
    }
    let n = sbox.len().trailing_zeros() as usize;
    let m = out_bits;
    let size = sbox.len();

    let mut ddt_hist: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for a in 1..size {
        let mut count = vec![0u32; 1usize << m];
        for x in 0..size {
            count[(sbox[x] ^ sbox[x ^ a]) as usize] += 1;
        }
        for &c in &count {
            *ddt_hist.entry(c).or_default() += 1;
        }
    }

    let mut walsh_hist: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for b in 1..(1usize << m) {
        let comp: Vec<bool> = sbox
            .iter()
            .map(|&y| (b as u32 & y).count_ones() % 2 == 1)
            .collect();
        for &w in &describe::walsh_spectrum(&comp)? {
            *walsh_hist.entry(w.unsigned_abs()).or_default() += 1;
        }
    }

    Some(SboxSpectra {
        in_bits: n,
        out_bits: m,
        differential_spectrum: ddt_hist.into_iter().collect(),
        walsh_spectrum: walsh_hist.into_iter().collect(),
    })
}

/// The **boomerang uniformity** of an S-box permutation — the third cryptanalytic pillar after
/// differential and linear. `BCT[a][b] = #{x : S⁻¹(S(x)⊕b) ⊕ S⁻¹(S(x⊕a)⊕b) = a}`; the uniformity is the
/// maximum over `a≠0, b≠0` and measures resistance to the boomerang attack (lower is stronger; `2` is
/// optimal, attained exactly by APN permutations). `None` if `sbox` is not a permutation.
pub fn boomerang_uniformity(sbox: &[u32]) -> Option<u32> {
    if sbox.is_empty() || !sbox.len().is_power_of_two() {
        return None;
    }
    let size = sbox.len();
    let mut inv = vec![u32::MAX; size];
    for (x, &y) in sbox.iter().enumerate() {
        let u = y as usize;
        if u >= size || inv[u] != u32::MAX {
            return None; // not a permutation
        }
        inv[u] = x as u32;
    }
    let mut bu = 0u32;
    for a in 1..size {
        for b in 1..size {
            let mut count = 0u32;
            for x in 0..size {
                let lhs = inv[(sbox[x] as usize) ^ b] ^ inv[(sbox[x ^ a] as usize) ^ b];
                if lhs as usize == a {
                    count += 1;
                }
            }
            bu = bu.max(count);
        }
    }
    Some(bu)
}

/// The structural-security verdict on an S-box — the vectorial analogue of [`rsa_full_audit`]. It flags
/// only PROVABLE weaknesses (an exact linear relation, a deterministic difference, a quadratic system); a
/// clean profile is the honest ceiling, "no structural weakness of these classes," never a proof of
/// security (the Chaitin frame in the symmetric world).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SboxVerdict {
    /// Every component is affine — trivially broken; linear cryptanalysis is exact.
    Affine,
    /// Some component `⟨b, S(x)⟩` is affine (`linearity = 2ⁿ`): a linear relation among the outputs holds
    /// with probability 1.
    LinearComponent,
    /// Some input difference forces a single output difference (`differential uniformity = 2ⁿ`).
    DeterministicDifference,
    /// Algebraic degree 2 — the S-box is a system of quadratic equations, the classic algebraic-attack target.
    Quadratic,
    /// No structural weakness of the above classes. The profile is the honest ceiling, not a security proof.
    NoStructuralWeaknessFound {
        differential_uniformity: u32,
        linearity: u64,
        min_degree: usize,
        boomerang_uniformity: Option<u32>,
    },
}

/// Audit an S-box with the full structural arsenal — differential, linear, algebraic, and boomerang — and
/// return the first provable weakness, else the honest ceiling with the measured profile. `None` if the
/// table length is not a power of two. Threshold-free: it never fabricates a "weak" verdict from an
/// arbitrary cutoff, only from a structure that is exact.
pub fn sbox_full_audit(sbox: &[u32], out_bits: usize) -> Option<SboxVerdict> {
    let p = sbox_profile(sbox, out_bits)?;
    let full = 1u64 << p.in_bits;
    if p.is_affine {
        return Some(SboxVerdict::Affine);
    }
    if p.linearity == full {
        return Some(SboxVerdict::LinearComponent);
    }
    if p.differential_uniformity as u64 == full {
        return Some(SboxVerdict::DeterministicDifference);
    }
    if p.min_degree == 2 {
        return Some(SboxVerdict::Quadratic);
    }
    Some(SboxVerdict::NoStructuralWeaknessFound {
        differential_uniformity: p.differential_uniformity,
        linearity: p.linearity,
        min_degree: p.min_degree,
        boomerang_uniformity: if p.is_bijective {
            boomerang_uniformity(sbox)
        } else {
            None
        },
    })
}

// ---- RSA: the thesis in the public-key world (structural weakness vs the number-theoretic ceiling) ---
//
// The compressibility ladder attacks SYMMETRIC keystreams through sequence structure; RSA rests on
// integer factorization instead, so none of those rungs touch it. But the same principle governs a
// modulus: a WEAK RSA key carries exploitable structure (a small factor, primes too close, a smooth
// `p−1`, a shared prime, a small private exponent), and each such structure is a compression — a short
// description of the secret — recovered as a certified factorization by `crate::factor`. A
// SOUND modulus has none: it is the number-theoretic incompressible residue, the Chaitin ceiling in the
// public-key world. Crush every structured form; the sound form stands, and that standing is the proof.

/// The structural-security verdict on an RSA modulus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RsaStrength {
    /// A certified structural break: `p·q = N`, found by the named attack (a re-checkable witness).
    Factored {
        p: logicaffeine_base::BigInt,
        q: logicaffeine_base::BigInt,
        method: &'static str,
    },
    /// The whole structural arsenal declined within budget — no structural shortcut exists. Only the
    /// general sub-exponential algorithms remain, which real key sizes push out of reach: the ceiling.
    SoundAgainstStructuralAttacks,
}

/// Audit an RSA modulus with the full structural factoring arsenal (see [`crate::factor`]):
/// return a certified factorization if any structural weakness exists, else the soundness verdict — the
/// number-theoretic incompressible residue. Uses the default triage budget.
pub fn rsa_structural_audit(n: &logicaffeine_base::BigInt) -> RsaStrength {
    match crate::factor::structural_factor(n, Default::default()) {
        Some(w) => RsaStrength::Factored {
            p: w.p,
            q: w.q,
            method: w.method,
        },
        None => RsaStrength::SoundAgainstStructuralAttacks,
    }
}

/// The verdict of throwing the ENTIRE arsenal at an RSA public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RsaAuditVerdict {
    /// The structural factoring suite split the modulus (the method names the structure it exploited).
    Factored {
        method: &'static str,
        p: logicaffeine_base::BigInt,
        q: logicaffeine_base::BigInt,
    },
    /// Wiener's attack found a small private exponent.
    WeakExponent,
    /// The compressibility classifier found exploitable structure in the modulus bytes.
    CompressibleModulus(CompressibilityClass),
    /// No lens in the arsenal compresses or factors it. This is NOT a proof of security — by the
    /// kernel-certified Chaitin bound (this module) no such proof exists — it is "resists everything we
    /// have built," the honest ceiling.
    ResistsFullArsenal,
}

/// Throw the **entire arsenal** at an RSA public key `(N, e)`: the structural factoring suite (small
/// factor / close primes / smooth `p−1` / Pollard rho), Wiener's small-exponent attack, and the
/// compressibility classifier on the modulus bytes. This is the pre-release safety gate — if any of our
/// own mathematics broke RSA, this is where it would surface. `ResistsFullArsenal` is the honest ceiling,
/// not a security proof: we certify weakness whenever structure exists and can never certify its absence.
pub fn rsa_full_audit(
    n: &logicaffeine_base::BigInt,
    e: &logicaffeine_base::BigInt,
) -> RsaAuditVerdict {
    use crate::factor;
    if let Some(w) = factor::structural_factor(n, Default::default()) {
        return RsaAuditVerdict::Factored {
            method: w.method,
            p: w.p,
            q: w.q,
        };
    }
    if factor::wiener(e, n).is_some() {
        return RsaAuditVerdict::WeakExponent;
    }
    let (_, bytes) = n.to_le_bytes();
    let report = classify_bytes(&bytes);
    if report.class != CompressibilityClass::Incompressible {
        return RsaAuditVerdict::CompressibleModulus(report.class);
    }
    RsaAuditVerdict::ResistsFullArsenal
}

/// Assess key/ciphertext bytes: `Weak` (with a re-checkable compression witness — the concrete attack)
/// when the engine describes the data in fewer bytes than storing it raw, else `IncompressibleInClass`.
pub fn assess_key_material(data: &[u8]) -> CryptoStrength {
    let ints: Vec<i64> = data.iter().map(|&b| b as i64).collect();
    let witness = DescriptionBound::of_int_seq(&ints);
    let ratio = if data.is_empty() {
        1.0
    } else {
        witness.bytes as f64 / data.len() as f64
    };
    // A description shorter than the raw bytes is a certified weakness — the witness IS the attack.
    if witness.bytes < data.len() {
        CryptoStrength::Weak { witness, ratio }
    } else {
        CryptoStrength::IncompressibleInClass { ratio }
    }
}

// ---- Compressibility classes: where an input sits on the ordered → random spectrum --------------
//
// The description engine tries a whole menu of generators and keeps the shortest. The SHAPE of the
// winner is the input's *compressibility class* — a computable, certified shadow of Kolmogorov
// complexity — and how far it beats storing the raw bytes is the *degree*. From most to least
// structured: Generated (a tiny closed-form program) → Periodic → LowEntropy → Smooth → Incompressible
// (algorithmically random relative to this class).

/// The compressibility class of an input, read off the winning description-menu generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressibilityClass {
    /// A closed-form program reproduces it (affine / geometric / polynomial / general generator) —
    /// algorithmically simplest: K̄ = O(1) regardless of length.
    Generated,
    /// A repeating block (cyclic / periodic).
    Periodic,
    /// Low per-symbol entropy: a dominant value, long runs, or a tiny alphabet (sparse / RLE / dict).
    LowEntropy,
    /// Smoothly varying: small successive differences or a narrow value range (delta / DoD / frame-of-reference).
    Smooth,
    /// Incompressible: nothing beats storing it raw — algorithmically random relative to this class.
    Incompressible,
}

/// A full compressibility report: the class, the degree (`K̄/n`), and the raw vs described sizes.
#[derive(Clone, Debug)]
pub struct CompressibilityReport {
    pub class: CompressibilityClass,
    /// `K̄ / n` — described bytes over raw bytes. ≈ 1 incompressible; → 0 algorithmically simple.
    pub ratio: f64,
    pub described_bytes: usize,
    pub raw_bytes: usize,
}

/// Classify a byte string by its compressibility class and degree.
pub fn classify_bytes(data: &[u8]) -> CompressibilityReport {
    let ints: Vec<i64> = data.iter().map(|&b| b as i64).collect();
    let described = describe::describe_int_seq(&ints);
    let raw = data.len();
    let ratio = if raw == 0 {
        1.0
    } else {
        described.len() as f64 / raw as f64
    };
    let class = if described.len() >= raw.max(1) {
        // Nothing beat storing the bytes raw ⇒ incompressible, whatever the nominal winning tag.
        CompressibilityClass::Incompressible
    } else {
        class_of_tag(described.first().copied().unwrap_or(describe::T_INTS))
    };
    CompressibilityReport {
        class,
        ratio,
        described_bytes: described.len(),
        raw_bytes: raw,
    }
}

/// Classify a text string by the compressibility class of its UTF-8 bytes.
pub fn classify_text(text: &str) -> CompressibilityReport {
    classify_bytes(text.as_bytes())
}

/// Classify an integer sequence (numeric data, not bytes) by its compressibility class, measured
/// against the plain varint encoding — so a linear recurrence (Fibonacci-class) is recognized as a
/// closed-form `Generated` program even though it grows past byte range.
pub fn classify_int_seq(data: &[i64]) -> CompressibilityReport {
    let described = describe::describe_int_seq(data);
    let mut baseline = vec![describe::T_INTS];
    describe::leb128_encode(&mut baseline, data.iter().copied(), data.len());
    let raw = baseline.len();
    let ratio = if raw == 0 {
        1.0
    } else {
        described.len() as f64 / raw as f64
    };
    let class = if described.len() >= raw {
        CompressibilityClass::Incompressible
    } else {
        class_of_tag(described.first().copied().unwrap_or(describe::T_INTS))
    };
    CompressibilityReport {
        class,
        ratio,
        described_bytes: described.len(),
        raw_bytes: raw,
    }
}

/// Map a winning description-menu tag to its compressibility class.
fn class_of_tag(tag: u8) -> CompressibilityClass {
    use describe::*;
    match tag {
        t if t == T_INTS_AFFINE
            || t == T_INTS_GEOMETRIC
            || t == T_INTS_POLY
            || t == T_GEN
            || t == T_INTS_LRECUR =>
        {
            CompressibilityClass::Generated
        }
        t if t == T_INTS_PERIODIC => CompressibilityClass::Periodic,
        t if t == T_INTS_SPARSE || t == T_INTS_RLE || t == T_INTS_DICT => {
            CompressibilityClass::LowEntropy
        }
        t if t == T_INTS_DELTA || t == T_INTS_DOD || t == T_INTS_FOR => {
            CompressibilityClass::Smooth
        }
        _ => CompressibilityClass::Incompressible, // T_INTS / T_BYTES: no exploitable structure of this class
    }
}

/// Whether the given GF(2) vectors (each ≤ 64-wide) are linearly independent — Gaussian elimination
/// over GF(2), full rank iff rank equals the count.
fn independent_gf2(vectors: &[Vec<bool>]) -> bool {
    let mut rows: Vec<u64> = vectors
        .iter()
        .map(|v| {
            v.iter().enumerate().fold(
                0u64,
                |m, (i, &b)| if b && i < 64 { m | (1u64 << i) } else { m },
            )
        })
        .collect();
    let mut rank = 0usize;
    for col in 0..64u32 {
        if let Some(piv) = (rank..rows.len()).find(|&r| rows[r] & (1u64 << col) != 0) {
            rows.swap(rank, piv);
            let pr = rows[rank];
            for r in 0..rows.len() {
                if r != rank && rows[r] & (1u64 << col) != 0 {
                    rows[r] ^= pr;
                }
            }
            rank += 1;
        }
    }
    rank == vectors.len()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/ait/tests.rs"]
mod tests;
