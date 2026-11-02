#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod annotation;
pub mod bitvector;
pub mod cc;
#[cfg(feature = "serde")]
pub mod certificate;
mod context;
mod conversion_session;
pub mod dag;
pub mod elaborate;
mod error;
pub mod eval;
pub mod field_algebra;
pub mod inductive_compile;
pub mod interface;
pub mod lia;
pub mod omega;
pub mod positivity;
pub mod prelude;
pub mod recheck;
pub mod recursor;
mod reduction;
mod reflection;
pub mod reify;
pub mod ring;
pub mod simp;
mod structure_eta;
pub mod tactic_cert;
pub mod tactics;
mod term;
pub mod termination;
mod type_checker;
pub mod word_ring;

pub use context::{reconcile_block_params, DEFAULT_INSTANCE_PRIORITY};
pub use context::{Context, MutualInductive, StructInfo};
pub use error::{KernelError, KernelResult, MeaningChange};
pub use eval::{
    eval_bool, eval_bool_tree, native_compile_bool, native_compile_decide, native_decide,
};
pub use inductive_compile::{NestedDecl, NestedInfo};
pub use structure_eta::StructureRefusal;
// `normalize` is the BEST-EFFORT face: it returns a `Term` and so cannot report that a reduction
// never finished. `normalize_checked` is the honest one — it distinguishes a real normal form from
// a δ-CYCLE (no normal form exists), an exhausted step budget, and a term too deep to walk. Any
// caller that can carry an error should prefer it.
/// The transparent carrier's payload: a binder's kind, or where a subterm came from.
pub use annotation::{Annotation, Provenance, SourcePosition};
pub use elaborate::{
    auto_bind_implicits, bind_self_recursion, elaborate, elaborate_anon_ctor, elaborate_app,
    elaborate_app_against, elaborate_dot, elaborate_in, fill_match_motives, instantiate,
    resolve_coercion, resolve_instance, surface_elaborate, surface_elaborate_against, unify,
    unify_in, MetaCtx, ParamKind, ANON_CTOR_MARKER, DOT_MARKER,
};
pub use logicaffeine_base::BigInt;
pub use recheck::{double_check, recheck, DoubleCheck, Incomplete, IncompleteKind, ReCheckError};
pub use recursor::derive_recursor;
/// Whether a verdict depended on kernel work that stopped for want of a resource: a conversion
/// that says "not equal" decided nothing when one of its reductions ran out of budget.
pub use reduction::{discard_resource_stops, observe_resource_stops};
pub use reduction::{normalize, normalize_checked, Unreduced};
pub use reify::VarInterner;
pub use term::{
    instantiate_universes, int_lit, lit_bigint, resolve_const_levels, ConstError, Literal, Term,
    Universe,
};
pub use type_checker::{infer_type, instantiate_codomain, is_subtype, substitute};

/// Definitional equality of two terms — exposed for tests that pin conversion
/// behaviour (structure eta, proof irrelevance) directly.
pub fn defeq_for_test(ctx: &Context, a: &Term, b: &Term) -> bool {
    type_checker::def_eq(ctx, a, b)
}

/// Strict-positivity check of a constructor type — exposed so tests can pin the
/// paradox fence (negative occurrences rejected, positive functional fields
/// accepted) directly.
pub fn check_positivity_for_test(
    inductive: &str,
    constructor: &str,
    ty: &Term,
) -> KernelResult<()> {
    positivity::check_positivity(inductive, constructor, ty)
}

/// The δ-transparency axis (Lean's `TransparencyMode`): `normalize_at`/`normalize_checked_at` keep
/// an `@[irreducible]` constant folded with its body intact, while `normalize` stays the kernel's
/// own always-unfolding setting.
pub use reduction::{normalize_at, normalize_checked_at, Transparency};

pub mod term_store;

/// The `@[simp]` registry's compiled rule — `Context::simp_rules`/`simp_rules_for` hand these to
/// whatever does the rewriting, already destructured into `∀x̄. lhs ⟶ rhs`.
pub use context::SimpRule;

/// Lean's reducibility axis, declared per constant — `Context::is_irreducible` is the predicate
/// `reduction::Transparency::Excluding` is looking for.
pub use context::Reducibility;

/// Lean's `#print axioms`, as a function — what a proof transitively rests on, and whether any of
/// it is an admission rather than an assumption. Lives in `prelude` (beside the trusted base it is
/// the complement of) and is surfaced here because it is not optional: the REPL, the certificate
/// re-checker and `logicaffeine_proof`'s verdicts must all read the same closure.
pub use prelude::{
    axiom_dependencies, axiom_dependencies_given, axiom_dependencies_of, foreign_axiom_note,
    is_admitted_axiom, AxiomUsage, ADMITTED_AXIOMS, FOREIGN_AXIOMS,
};

/// The elaborator's reduction face: `@[irreducible]` constants stay folded, body intact, with
/// the opacity predicate built once from `Context::is_irreducible` rather than by each caller.
/// `normalize` remains the kernel's face and unfolds everything, exactly as Lean's does.
pub use reduction::{normalize_checked_elab, normalize_elab};

pub mod equations;

/// `Int` as a real inductive — Lean's `ofNat | negSucc` with its arithmetic defined rather than
/// assumed (`Init/Data/Int/Basic.lean:40-45`, `:78-166`). The construction, packaged so the
/// prelude adopts it in one call; the module header carries how Lean keeps it fast anyway.
pub mod int_inductive;
mod integer_laws;

/// The universe-polymorphic eliminator — Lean's `Nat.rec.{u}`, whose motive lands in `Sort u`
/// rather than in the inductive's own sort (`Init/Prelude.lean` states every core recursor this
/// way). `derive_recursor` remains the monomorphic face; this one returns the level telescope
/// beside the type and term, so a caller can register it either as `add_universe_poly` (referenced
/// as `Term::Const { name, levels }`) or as a plain definition whose free level variable the
/// application rule solves from the motive actually supplied.
pub use recursor::derive_recursor_universe_poly;

/// Certified universe-polymorphic quotient primitives.
pub mod observational;
pub mod quotient;
