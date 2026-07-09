#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod context;
#[cfg(feature = "serde")]
pub mod certificate;
mod error;
pub mod inductive_compile;
pub mod interface;
pub mod positivity;
pub mod prelude;
mod reduction;
pub mod reify;
pub mod ring;
pub mod lia;
pub mod field_algebra;
pub mod word_ring;
pub mod bitvector;
pub mod cc;
pub mod eval;
pub mod simp;
pub mod elaborate;
pub mod omega;
pub mod recheck;
pub mod recursor;
pub mod termination;
mod term;
mod type_checker;

pub use context::{Context, MutualInductive, StructInfo};
pub use inductive_compile::{NestedDecl, NestedInfo};
pub use error::{KernelError, KernelResult};
pub use eval::{
    eval_bool, eval_bool_tree, native_compile_bool, native_compile_decide, native_decide,
};
pub use reduction::normalize;
pub use elaborate::{
    auto_bind_implicits, bind_self_recursion, elaborate, elaborate_app, elaborate_app_against,
    elaborate_anon_ctor, elaborate_dot, elaborate_in, fill_match_motives, instantiate, resolve_coercion,
    resolve_instance, surface_elaborate, surface_elaborate_against, unify, unify_in, MetaCtx,
    ParamKind, ANON_CTOR_MARKER, DOT_MARKER,
};
pub use recheck::{double_check, recheck, DoubleCheck, ReCheckError};
pub use recursor::derive_recursor;
pub use reify::VarInterner;
pub use logicaffeine_base::BigInt;
pub use term::{instantiate_universes, int_lit, lit_bigint, Literal, Term, Universe};
pub use type_checker::{infer_type, is_subtype};

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
