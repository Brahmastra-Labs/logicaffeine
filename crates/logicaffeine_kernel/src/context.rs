//! Typing context for the kernel.
//!
//! A context maps variable names to their types.
//! Used during type checking to track what variables are in scope.

use crate::term::{Term, Universe};
use std::collections::HashMap;
use std::sync::Arc;

// A context snapshot shares immutable declaration payloads even when a later
// registration forks its name map. Map membership retains Global/Const policy.
type DefinitionPayload = (Vec<String>, Term, Term);
type DefinitionMap = HashMap<String, Arc<DefinitionPayload>>;

// Removing an absent entry must not fork an immutable context snapshot (or
// dissociate its weak cache-generation observers).
fn remove_shared_entry<V: Clone>(map: &mut Arc<HashMap<String, V>>, name: &str) -> Option<V> {
    if !map.contains_key(name) {
        return None;
    }
    Arc::make_mut(map).remove(name)
}

/// One change to a context's registries, newest first: the persistent log the core's mirror reads to
/// learn what a context changed since the one it mirrors (`dag::mirror`). A clone shares the whole
/// log; the head's pointer is the lineage's identity, and it cannot alias a freed entry while anyone
/// holds an `Arc` to it.
pub(crate) struct LogEntry {
    pub(crate) name: String,
    /// The name's registration was removed (shadowed, redeclared or revoked) rather than added.
    pub(crate) removed: bool,
    pub(crate) prev: Option<Arc<LogEntry>>,
}

/// One name's registration, as [`Context::registration`] reads it.
pub(crate) enum Registration<'a> {
    /// An inductive family: its arity.
    Inductive(&'a Term),
    /// A definition: its type and body.
    Definition(&'a Term, &'a Term),
    /// A universe-polymorphic definition: its level parameters, type and body.
    UniversePoly(&'a [String], &'a Term, &'a Term),
    /// An axiom: its type.
    Axiom(&'a Term),
}

impl std::fmt::Debug for LogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LogEntry({}{})",
            if self.removed { "-" } else { "+" },
            self.name
        )
    }
}

impl Drop for LogEntry {
    // A long log would otherwise drop recursively, one frame per entry.
    fn drop(&mut self) {
        let mut next = self.prev.take();
        while let Some(entry) = next {
            match Arc::try_unwrap(entry) {
                Ok(mut owned) => next = owned.prev.take(),
                Err(_) => break,
            }
        }
    }
}

#[path = "context_order_cache.rs"]
mod order_cache;

#[cfg(test)]
std::thread_local! {
    static OPAQUE_INVALIDATION_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Identity of every immutable registry inspected by integer arithmetic attestation.
/// Weak owners prevent allocation-address reuse without keeping whole map payloads
/// alive or forcing Arc::make_mut to clone them during later declarations.
#[derive(Debug)]
struct ArithmeticGeneration {
    definitions: std::sync::Weak<DefinitionMap>,
    declarations: std::sync::Weak<HashMap<String, Term>>,
    pending: std::sync::Weak<HashMap<String, String>>,
    inductives: std::sync::Weak<HashMap<String, Term>>,
    constructors: std::sync::Weak<HashMap<String, (String, Term)>>,
    constructor_order: std::sync::Weak<HashMap<String, Vec<String>>>,
    inductive_params: std::sync::Weak<HashMap<String, usize>>,
    inductive_inferred_params: std::sync::Weak<HashMap<String, usize>>,
    poly_types: std::sync::Weak<HashMap<String, (Vec<String>, Term)>>,
    universe_polys: std::sync::Weak<DefinitionMap>,
}
impl ArithmeticGeneration {
    fn of(ctx: &Context) -> Self {
        Self {
            definitions: Arc::downgrade(&ctx.definitions),
            declarations: Arc::downgrade(&ctx.declarations),
            pending: Arc::downgrade(&ctx.pending),
            inductives: Arc::downgrade(&ctx.inductives),
            constructors: Arc::downgrade(&ctx.constructors),
            constructor_order: Arc::downgrade(&ctx.constructor_order),
            inductive_params: Arc::downgrade(&ctx.inductive_params),
            inductive_inferred_params: Arc::downgrade(&ctx.inductive_inferred_params),
            poly_types: Arc::downgrade(&ctx.poly_types),
            universe_polys: Arc::downgrade(&ctx.universe_polys),
        }
    }
    fn matches(&self, ctx: &Context) -> bool {
        self.definitions.ptr_eq(&Arc::downgrade(&ctx.definitions))
            && self.declarations.ptr_eq(&Arc::downgrade(&ctx.declarations))
            && self.pending.ptr_eq(&Arc::downgrade(&ctx.pending))
            && self.inductives.ptr_eq(&Arc::downgrade(&ctx.inductives))
            && self.constructors.ptr_eq(&Arc::downgrade(&ctx.constructors))
            && self
                .constructor_order
                .ptr_eq(&Arc::downgrade(&ctx.constructor_order))
            && self
                .inductive_params
                .ptr_eq(&Arc::downgrade(&ctx.inductive_params))
            && self
                .inductive_inferred_params
                .ptr_eq(&Arc::downgrade(&ctx.inductive_inferred_params))
            && self.poly_types.ptr_eq(&Arc::downgrade(&ctx.poly_types))
            && self
                .universe_polys
                .ptr_eq(&Arc::downgrade(&ctx.universe_polys))
    }
}
#[derive(Debug)]
struct ArithmeticAttestation {
    generation: ArithmeticGeneration,
    // Independently attest the complete arithmetic operation dependency closures.
    results: [Option<bool>; 6],
    // The defined reflection operations (`reflection::DEFINED`), by position.
    reflection: [Option<bool>; 7],
    // Whether `Nat` and `Int` are the canonical families literals are read as.
    numerals: Option<CanonicalNumerals>,
}

/// How far the context's numeral families are canonical. A literal reads as the constructor it
/// names while that constructor is its family's: `0` as `Zero` (`zero`: `Zero : Nat` is a
/// constructor of `Nat`), `n + 1` as `Succ n` (`succ`: so is `Succ : Nat → Nat`). Literal
/// arithmetic needs the families exact: `nat`, `Nat` is `Zero` and `Succ` and nothing else; `int`,
/// `Int` is `Int_ofNat` and `Int_negSucc` over that `Nat`, so `-3` reads as `Int_negSucc 2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalNumerals {
    pub(crate) zero: bool,
    pub(crate) succ: bool,
    pub(crate) nat: bool,
    pub(crate) int: bool,
}

/// The annotation and its lexical analysis have the same immutable lifetime.
/// Context extensions share both; replacing a binding creates a fresh analysis.
#[derive(Debug)]
struct LocalBinding {
    ty: Term,
    free_names: std::sync::OnceLock<std::collections::HashSet<String>>,
}

/// Typing context: maps variable names to their types.
///
/// The context is immutable-by-default: `extend` creates a new context
/// with the additional binding, preserving the original.
///
/// Also stores global definitions:
/// - Inductive types (e.g., Nat : Type 0)
/// - Constructors (e.g., Zero : Nat, Succ : Nat -> Nat)
/// - Declarations (e.g., hypotheses like h1 : P -> Q)
#[derive(Debug, Clone, Default)]
pub struct Context {
    // A certificate must be checked without using the optimization it licenses.
    pub(crate) checking_order_recurrence: bool,
    /// Every change to the registries a declaration lands in, newest first ([`LogEntry`]).
    admissions: Option<Arc<LogEntry>>,
    arithmetic_attestation: Arc<std::sync::Mutex<Option<ArithmeticAttestation>>>,
    order_attestation: Arc<std::sync::Mutex<Option<order_cache::OrderAttestation>>>,
    checked_opaque: Arc<HashMap<String, crate::prelude::CheckedOpaqueDependencies>>,
    // Shared direct dependency nodes, with reverse edges for conservative
    // invalidation. A long theorem chain stores one node per declaration.
    checked_opaque_graph: Arc<HashMap<String, Arc<std::collections::BTreeSet<String>>>>,
    checked_opaque_dependents: Arc<HashMap<String, std::collections::BTreeSet<String>>>,
    pub(crate) quotient_primitives: Arc<HashMap<String, crate::quotient::Certificate>>,
    pub(crate) prelude_quotient: Option<Arc<crate::quotient::PreludeCertificate>>,
    /// The certified observational equality (`observational`), when one is registered.
    pub(crate) observational: Option<Arc<crate::observational::Certificate>>,
    /// Local variable bindings (from λ and Π) — the only part that grows during type
    /// inference (one entry per enclosing binder). The context is `extend`ed (cloned) at
    /// every binder, so the binding TYPES are shared behind `Arc`: cloning the map copies
    /// pointers, not whole proposition types.
    bindings: HashMap<String, Arc<LocalBinding>>,

    /// The global environment below is FIXED during inference but the context is
    /// `extend`ed (cloned) at every λ/Π. Sharing it behind `Rc` makes that clone O(1)
    /// instead of deep-copying every premise type — the difference between linear and
    /// quadratic checking of a large certified proof.
    ///
    /// Inductive type definitions: name -> sort (e.g., "Nat" -> Type 0)
    inductives: Arc<HashMap<String, Term>>,

    /// Constructor definitions: name -> (inductive_name, type)
    constructors: Arc<HashMap<String, (String, Term)>>,

    /// How many LEADING arguments of an inductive's arity are uniform PARAMETERS (as
    /// opposed to INDICES that vary per constructor). `List (A:Type)` has 1 parameter and
    /// 0 indices; `Eq (A) (x) : A → Prop` has 2 parameters and 1 index; `Vector (A) : Nat
    /// → Type` has 1 parameter and 1 index. Absence means "all of the arity is
    /// parameters" (0 indices) — so every non-indexed inductive behaves exactly as before
    /// this map existed, and indexed elimination is a strict extension.
    inductive_params: Arc<HashMap<String, usize>>,

    /// The params/indices split INFERRED from the registered constructors, for an inductive that
    /// never declared one. Maintained by [`Context::add_constructor`] (registration time, so the
    /// hot readers stay O(1)) and consulted by [`Context::inductive_num_params`] only when
    /// `inductive_params` has no explicit entry.
    ///
    /// Defaulting to the FULL ARITY instead — what this map replaces — declared every indexed
    /// family's indices to be parameters. `BVec : Nat → Type 0` claimed one uniform parameter
    /// while `BVNil : BVec Zero` and `BVCons : … → BVec (Succ n)` target different lengths, so the
    /// only kind of `match` a length-indexed vector exists for was not typeable at all.
    inductive_inferred_params: Arc<HashMap<String, usize>>,

    /// Order of constructor registration per inductive.
    /// HashMap doesn't preserve insertion order, so we track it explicitly.
    constructor_order: Arc<HashMap<String, Vec<String>>>,

    /// Declaration bindings (axioms/hypotheses): name -> type
    /// Used for certifying proofs where hypotheses are assumed.
    declarations: Arc<HashMap<String, Term>>,

    /// Definition bodies: name -> (type, body)
    /// Definitions are transparent - they unfold during normalization.
    /// Distinguished from declarations (axioms) which have no body.
    definitions: Arc<DefinitionMap>,

    /// Hint database: theorem names marked as hints for auto tactic.
    /// When auto fails with decision procedures, it tries to apply these hints.
    hints: Arc<Vec<String>>,

    /// Universe-polymorphic definitions (R3): name -> (universe params, type, body). A
    /// `Term::Const { name, levels }` reference instantiates these params with `levels`.
    universe_polys: Arc<DefinitionMap>,

    /// Typeclass instance database (R4): each `(type, value)` is an instance the
    /// elaborator may resolve for an instance-implicit argument — e.g.
    /// `(Inhabited Nat, mk Nat Zero)`. Searched by unifying `type` against the required
    /// class type.
    ///
    /// PUSH-ONLY, and nothing may ever depend on an ABSOLUTE count: a declaration-time
    /// registration can appear at any moment, so a test that pins `instances().len()` has to
    /// measure RELATIVE to a baseline it captured itself.
    instances: Arc<Vec<(Term, Term)>>,

    /// Per-instance PRIORITY, index-aligned with `instances` (Lean's `instance (priority := n)`;
    /// the default is [`DEFAULT_INSTANCE_PRIORITY`]). Kept as a side-car rather than folded into
    /// the pair so `instances()` stays a true borrowed slice of `(type, value)` — the shape every
    /// reader already destructures — while the ordering key gains a second component.
    ///
    /// The pair is maintained ONLY by [`Context::add_instance_with_priority`]; every other door
    /// funnels through it, so the two vectors cannot drift.
    instance_priorities: Arc<Vec<i64>>,

    /// The SCOPE each instance belongs to, index-aligned with `instances`. `0` is the permanent
    /// (file) scope; `d > 0` is the depth of the [`Context::push_scope`] that owns it, and
    /// [`Context::pop_scope`] removes exactly those entries. See the scope section below.
    instance_scopes: Arc<Vec<usize>>,

    /// THE INSTANCE INDEX — Lean's discrimination tree, rooted at the class head symbol.
    ///
    /// `Lean/Meta/Instances.lean:67` keeps instances in a `DiscrTree InstanceEntry` and
    /// `Lean/Meta/SynthInstance.lean:210` queries it with `getUnify`, so a goal is only ever
    /// offered the entries whose key path could unify with it. A flat `Vec` scanned end to end
    /// costs a `MetaCtx` clone, a telescope freshening and a full unification attempt PER ENTRY,
    /// which at a Mathlib-sized instance set stops being a speed question and becomes a
    /// usability one.
    ///
    /// Maps a class head symbol to the `instances` indices whose conclusion is headed by it. An
    /// entry whose conclusion head is NOT a closed symbol — an instance concluding at a variable
    /// it binds, or at a δ-reducible name that could unfold into any class — lives in
    /// `instance_open` instead and is offered to EVERY goal, which is Lean's `star` bucket
    /// (`Lean/Meta/DiscrTree.lean:727`, where `getUnify` unions `getStarResult d` into every
    /// answer). Losing a `star` entry is the one way an index can silently drop an instance.
    instance_by_head: Arc<HashMap<String, Vec<usize>>>,

    /// The `star` bucket of [`Context::instance_by_head`] — entries no head symbol can rule out.
    instance_open: Arc<Vec<usize>>,

    /// The SCOPE each coercion belongs to, index-aligned with `coercions`. Same encoding.
    coercion_scopes: Arc<Vec<usize>>,

    /// The SCOPE each simp rule belongs to, index-aligned with `simp_rules`. Same encoding.
    simp_scopes: Arc<Vec<usize>>,

    /// The SCOPE each hint belongs to, index-aligned with `hints`. Same encoding.
    hint_scopes: Arc<Vec<usize>>,

    /// How many scopes are currently OPEN — Lean's `section`/`namespace` nesting depth. `0` is
    /// the permanent scope, where an unqualified registration always lands.
    scope_depth: usize,

    /// DECLARED REDUCIBILITY — Lean's `@[reducible]` / `@[irreducible]`. Only the non-default
    /// entries are stored, so the overwhelmingly common answer costs one failed hash lookup.
    reducibility: Arc<HashMap<String, Reducibility>>,

    /// The undo log that makes `local attribute [irreducible]` revert: `(owning scope, name,
    /// the value that was there before)`. A map cannot be filtered by owner the way the
    /// index-aligned vectors can, so the PREVIOUS value is recorded and replayed in reverse.
    reducibility_undo: Arc<Vec<(usize, String, Option<Reducibility>)>>,

    /// REGISTERED REWRITE RULES — the `@[simp]` set, compiled once at registration.
    ///
    /// The lemma is stored as the shape a simplifier applies, not as a name to look up: a
    /// consumer that had to re-open a Π-telescope and re-destructure an `Eq` on every rewrite
    /// attempt would pay for the parse once per candidate per subterm.
    simp_rules: Arc<Vec<SimpRule>>,

    /// `simp_rules` indexed by the LHS's head symbol — the dispatch key. Rewriting a subterm
    /// whose head is `f` consults only the rules that could possibly match it, so adding a
    /// thousand lemmas about other symbols costs a rewrite attempt nothing.
    simp_by_head: Arc<HashMap<String, Vec<usize>>>,

    /// FALLBACK instances — a second, strictly lower tier consulted only after every entry in
    /// `instances` has failed. Lean's `default_instance`s and its derived `Decidable` machinery
    /// live in exactly such a tier: a library entry must never outrank something the user
    /// declared, and it must not show up in `instances()` as if the user had declared it.
    ///
    /// This is where declaration-time synthesis (`decEq{I}`) and prelude-shipped instances go.
    fallback_instances: Arc<Vec<(Term, Term)>>,

    /// Registered COERCIONS (E1): each `(from, to, coe)` is a function `coe : from → to` the
    /// elaborator may insert when an argument of type `from` is supplied where `to` is
    /// expected — Lean's `Coe`/`↑`. Searched by unifying `from`/`to` against the mismatch.
    coercions: Arc<Vec<(Term, Term, Term)>>,

    /// PER-BINDER implicitness (E2): a global's parameter kinds in order — implicit,
    /// explicit, and instance may INTERLEAVE (Lean's `BinderInfo`). Absent means the legacy
    /// "`implicit_args` leading implicits, rest explicit" model.
    binder_kinds: Arc<HashMap<String, Vec<crate::elaborate::ParamKind>>>,

    /// PER-PARAMETER DEFAULTS: a global's parameter NAME and the value inlined when the caller
    /// omits that argument, position by position — Lean's `(m : Int := 3)`.
    ///
    /// The name travels with the default because a `Term::Pi`'s binder name is not reliable here:
    /// `Int -> (m : Int := 3) -> Int` has an ANONYMOUS first binder, and a default that mentions
    /// an earlier parameter has to be closed over the argument that parameter actually received.
    /// The declaration door knows both (it runs `TermParser::name_anonymous_binders` before it
    /// records) and a `Pi` telescope alone does not.
    param_defaults: Arc<HashMap<String, Vec<(String, Option<Term>)>>>,

    /// How many LEADING parameters of a global are implicit (declared with `{…}` in the
    /// surface). The surface elaborator inserts and infers that many arguments at each
    /// application of the global, so the user writes `id 0` for `id Int 0`.
    implicit_args: Arc<HashMap<String, usize>>,

    /// Registered STRUCTURES (Rung 0c): a structure type name → its metadata. Only
    /// these one-constructor inductives get DEFINITIONAL ETA (`p ≡ ⟨p.1, …, p.n⟩`),
    /// keyed here so the rule is local and testable, never inferred for arbitrary
    /// one-constructor inductives.
    structures: Arc<HashMap<String, StructInfo>>,

    /// Constants this context AUTO-DERIVED at declaration time (`{I}_rec`, `{I}_noConfusion`,
    /// `decEq{I}`, …), per inductive.
    ///
    /// Provenance, and it is load-bearing: `add_constructor_checked` fires once PER CONSTRUCTOR, so
    /// the auxiliaries derived after the first constructor describe an inductive that is not
    /// finished yet. Re-deriving must therefore REPLACE the previous generation — but it must never
    /// replace a constant a human wrote (the prelude's hand-written `decEqNat` is the live case).
    /// Recording what we generated is what separates those two.
    derived_aux: Arc<HashMap<String, Vec<String>>>,

    /// Postpone equation products while a prelude is still replacing auxiliaries.
    equations_suspended: bool,

    /// The REVERSE of `derived_aux`: auxiliary name → the inductive that generated it.
    ///
    /// Two jobs, both O(1). It answers [`Context::is_derived_aux`] — "is this constant ours to
    /// replace, or did a human write it?" — and it lets [`Context::displace`] DISOWN a name the
    /// moment anything else registers it, so a re-derivation can never reach in and delete a
    /// constant that is no longer the one we generated. (The prelude's hand-written `decEqNat`,
    /// registered after an intermediate generation of `Nat`'s auxiliaries, is exactly that case.)
    derived_aux_owner: Arc<HashMap<String, String>>,

    /// MUTUAL inductive blocks (K3): each member name → the full ordered list of the
    /// block's members. `Even`/`Odd`, `Tree`/`Forest` — an inductive registered
    /// alone maps to nothing (its recursor recurses only on itself). The mutual
    /// recursor derivation reads this to give each member a motive and to route a
    /// recursive occurrence of a SIBLING to the sibling's fixpoint.
    mutual_blocks: Arc<HashMap<String, Vec<String>>>,

    /// DEFERRED registration obligations: definition name → the global name its body mentions that
    /// was not in scope when it was declared. See [`Context::declare`].
    ///
    pending: Arc<HashMap<String, String>>,

    /// The INVERSE index of `pending`: blocker name → the definitions waiting on it. Unblocking is
    /// then a lookup, not a scan of every outstanding obligation — a translator that emits 20 000
    /// definitions in forward-reference order must not pay quadratically for the privilege.
    ///
    /// Stale entries are permitted and filtered on use (`pending` is the authority on who is still
    /// waiting), which is what keeps removal O(1).
    pending_waiters: Arc<HashMap<String, Vec<String>>>,

    /// EAGER declaration-time auxiliary synthesis — Lean's behaviour, and OFF by default here for
    /// one MEASURED reason. `add_constructor` has no "declaration finished" signal, so eager
    /// synthesis re-derives after every constructor, and a constructor-disjointness table is
    /// quadratic in the constructor count. Measured on this prelude (the cost is printed by
    /// `tests/S_spine_extra.rs`): registering the standard library costs 0.18 s with synthesis
    /// QUEUED against 1.11 s synthesising once per INDUCTIVE (31 inductives, 162 auxiliaries) and
    /// far more again once per CONSTRUCTOR, against a test suite that rebuilds the prelude
    /// hundreds of times. The per-inductive figure was 17 s before wave 2; the ratio is what the
    /// switch is for, and the ratio has not changed.
    ///
    /// So it is a switch, not a policy baked into the registration door: a caller that wants Lean's
    /// declaration-time constants turns it on ([`Context::set_auto_auxiliaries`]) or asks for them
    /// in one pass at the end of a load ([`Context::seal`]), and a library loader that does not
    /// want 17 s of quadratic tables for its internal AST types pays nothing for them.
    auto_aux: bool,

    /// Inductives whose auxiliaries are OWED, in first-touch order (so synthesis is deterministic
    /// and a later member of a mutual block never overtakes an earlier one).
    dirty_aux: Arc<Vec<String>>,

    /// Declarations [`Context::declare`] REFUSED, as `(name, reason)`, in the order it refused them.
    ///
    /// `add_definition` returns `()`, so a refusal it drops would otherwise be invisible — and an
    /// invisible refusal is how a definition silently stops existing and a distant `rfl` starts
    /// failing for no stated reason. Recording it makes the gateway auditable: a caller that cannot
    /// carry an error can still ASK.
    refused: Arc<Vec<(String, String)>>,

    /// UNIVERSE-POLYMORPHIC TYPE DECLARATIONS: an inductive (or structure) whose ARITY mentions
    /// level variables — `Mul.{u} : Sort u → Sort u`, `Box.{u} : Sort u → Sort u`.
    ///
    /// Deliberately NOT `universe_polys`, and the difference is not bookkeeping: that map is a
    /// params/type/BODY triple and `reduce_step`'s `Term::Const` arm δ-unfolds whatever it finds
    /// there. A type former has no body, so entering one would either need a fake body or would
    /// teach the reducer to find `None` where it expects a term. Keeping the two registries apart
    /// is what lets `Const { name: "Mul", levels }` TYPE while staying inert to reduction, which
    /// is exactly what an inductive reference must do.
    poly_types: Arc<HashMap<String, (Vec<String>, Term)>>,

    /// Each definition's OWN δ-edges: the distinct global names its body mentions, computed once
    /// when the definition is registered.
    ///
    /// The δ-graph is walked by `Context::delta_reaches` once per declaration, and re-deriving
    /// an edge list by walking a body every time it is visited makes that walk cost the SIZE of
    /// every reachable body rather than its out-degree. Caching the edges is what keeps the
    /// acyclicity gate a gate rather than a tax.
    definition_deps: Arc<HashMap<String, Arc<[String]>>>,

    /// Each registered definition's DEFINITIONAL HEIGHT, computed by this kernel when its body is
    /// registered: one more than the greatest height among the constants its body mentions, where
    /// inductives, constructors, axioms and anything without a registered body count as 0. It is
    /// Lean's reducibility height, never read from an export: lazy δ in conversion unfolds the
    /// side whose head is higher first. A height orders unfolding only; it decides no equality.
    definition_heights: Arc<HashMap<String, u32>>,

    /// Every global name that OCCURS in the body of some definition registered here — a monotone
    /// over-approximation, never pruned.
    ///
    /// It exists so [`Context::declare`]'s δ-acyclicity check can be skipped in the case that is
    /// almost every case: a name nothing mentions cannot be on a cycle, so a fresh declaration
    /// costs one hash lookup instead of a walk of the δ-graph. Over-approximating is the safe
    /// direction — a stale entry buys an unnecessary walk, while a missing one would miss a cycle
    /// — which is why `displace` does not remove from it.
    referenced: Arc<std::collections::HashSet<String>>,
    /// Meanings relied on by native nested positivity, including transitive
    /// container constructors and unfolded aliases. Immutable once relied on:
    /// changing them could turn an already admitted datatype negative.
    native_positivity_dependencies: Arc<std::collections::HashSet<String>>,
    /// Who MENTIONS whom: for each global name, every registered global whose stored type, sort
    /// or body mentions it. The reverse index the stable-meaning gate reads — see
    /// [`Context::admit_meaning`] — so asking "was anything checked against this name?" costs the
    /// size of the answer, not a walk of the environment.
    mentioned_by: Arc<HashMap<String, std::collections::BTreeSet<String>>>,
    /// The forward edges of [`Context::mentioned_by`], per registered name, so `displace` can take
    /// a name's mentions back out exactly.
    mentions: Arc<HashMap<String, Vec<String>>>,
    /// What each registered global's stored terms ELIMINATE — recorded once, at registration, so
    /// the extension gate reads it instead of walking bodies. See [`Eliminations`].
    eliminations: Arc<HashMap<String, Eliminations>>,
    /// THE SHADOW TABLE: each written absolute name that denotes a FRESH IDENTITY rather than the
    /// declaration this environment holds under that name, paired with the identity, in the order
    /// the written names were first shadowed. One entry per written name — a second shadow of the
    /// same name replaces the identity in place. See [`Context::record_shadow`].
    shadows: Arc<Vec<(String, String)>>,
    /// Every identity minted for a shadow → the written name it was minted for. Kept for the
    /// identity's whole life, so a shadow that is itself shadowed still has a name to print by.
    shadow_origins: Arc<HashMap<String, String>>,
    /// This crate's own unit tests only: the caches that must notice a change of meaning
    /// (attestations, certificates, lookup memos) are defence in depth behind the stable-meaning
    /// gate, and testing them needs the change the gate refuses. No public door reaches this.
    #[cfg(test)]
    meaning_changes_permitted: bool,
}

/// How an incoming declaration relates to what its name already means — the question
/// [`Context::admit_meaning`] is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// Exactly the current meaning.
    Identical,
    /// A body for what is now a bodyless declaration, at the identical type and levels. Every
    /// checked dependent stays valid — δ only adds conversions — so it is admitted even when
    /// something depends on the name. (An AXIOM stated over the old constant may of course be
    /// false of the body; an axiom is an assumption, and `#print axioms` still names it.)
    Refinement,
    /// Anything else.
    Different,
}

/// What a registration door may do with a name, as decided by [`Context::admit_meaning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// Nothing outside the name's declaration unit depends on it: register (and displace) as
    /// usual.
    Register,
    /// Something depends on the name, and the incoming declaration is EXACTLY what it already
    /// means: nothing changes, so nothing is done.
    Keep,
}

/// The outcome of checking a body in [`Context::declare`].
enum Verdict {
    /// The body was inferred and has the declared type.
    Checked,
    /// The body mentions this name, which is not in scope yet; the check is owed.
    Pending(String),
}

/// One member of a MUTUAL inductive block: its name, arity sort, uniform parameter
/// count, and constructors (name + full type). Constructor types may reference ANY
/// member of the block — that is the whole point of a mutual declaration.
#[derive(Clone, Debug)]
pub struct MutualInductive {
    /// The inductive's name (e.g. `Even`).
    pub name: String,
    /// Its arity sort (e.g. `Nat → Prop`).
    pub sort: Term,
    /// How many leading arity arguments are uniform parameters (the rest are indices).
    pub num_params: usize,
    /// Constructors: `(name, full type)`, types possibly mentioning sibling members.
    pub constructors: Vec<(String, Term)>,
}

/// A coercion as the context stores it: erased, like every term it holds.
fn stored_coercion(from: Term, to: Term, coe: Term) -> (Term, Term, Term) {
    use crate::annotation::stored;
    (stored(from), stored(to), stored(coe))
}

/// An inductive block with every annotation erased, or `None` when there is nothing to erase —
/// the block's terms as the context will store and check them.
fn erased_block(block: &[MutualInductive]) -> Option<Vec<MutualInductive>> {
    let annotated = !crate::annotation::within_erased_call()
        && block.iter().any(|member| {
            member.sort.has_annotations()
                || member
                    .constructors
                    .iter()
                    .any(|(_, ty)| ty.has_annotations())
        });
    annotated.then(|| {
        block
            .iter()
            .map(|member| MutualInductive {
                name: member.name.clone(),
                sort: member.sort.strip_annotations(),
                num_params: member.num_params,
                constructors: member
                    .constructors
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.strip_annotations()))
                    .collect(),
            })
            .collect()
    })
}

/// Metadata for a registered structure (record): its single constructor, how many
/// leading type PARAMETERS it takes, the projection function names in field order, and
/// the declaration shape the elaborator needs — the parameter and field telescopes, the
/// parents recovered from a flattened field prefix, and which parameters are OUTPUTS.
#[derive(Clone, Debug)]
pub struct StructInfo {
    /// The constructor name (e.g. `Prod_mk`).
    pub mk: String,
    /// Number of leading type parameters (e.g. 2 for `Prod A B`).
    pub num_params: usize,
    /// Projection definition names, in field order (e.g. `[Prod_fst, Prod_snd]`).
    pub projections: Vec<String>,
    /// The parameter telescope, `(name, type)` in order — `[(A, Type 0), (B, Type 0)]`
    /// for `Prod`. Kept so a later declaration can be compared against this one.
    pub params: Vec<(String, Term)>,
    /// The field telescope, `(name, type)` in order. A field type may mention the
    /// parameters and any EARLIER field by name.
    pub fields: Vec<(String, Term)>,
    /// Structures this one EXTENDS, recovered from its flattened field list: a previously
    /// declared structure over the identical parameter telescope whose whole field list is
    /// a strict prefix of this one's. See [`Context::add_structure`].
    pub parents: Vec<String>,
    /// Parameter positions that are OUTPUTS — determined BY a resolved instance rather than
    /// matched against (Lean's `outParam`). Inferred at declaration; overridable with
    /// [`Context::set_out_params`].
    pub out_params: Vec<usize>,
}

/// Lean's default instance priority — what `instance foo : C α := …` gets when it says no
/// `(priority := n)`. Named rather than spelled `1000` at each site so a translator carrying a
/// Lean source's explicit priorities lands them on the same scale.
pub const DEFAULT_INSTANCE_PRIORITY: i64 = 1000;

/// The mark a shadow's fresh identity carries after the name it was minted for (`add✝⟨1⟩`), and
/// the mark an inaccessible name is printed with (`add✝`). See [`Context::mint_shadow_identity`].
pub(crate) const SHADOW_MARK: char = '\u{271d}';

impl Context {
    /// One attested answer, reused for exactly the registry generation it inspected: `read` finds
    /// it in that generation's entry, `attest` decides it outside the lock, and `write` keeps it.
    fn attested<T: Copy>(
        &self,
        read: impl Fn(&ArithmeticAttestation) -> Option<T>,
        write: impl FnOnce(&mut ArithmeticAttestation, T),
        attest: impl FnOnce() -> T,
    ) -> T {
        if let Ok(cache) = self.arithmetic_attestation.lock() {
            if let Some(found) = cache
                .as_ref()
                .filter(|entry| entry.generation.matches(self))
                .and_then(&read)
            {
                return found;
            }
        }
        // Do not hold the cache mutex while inspecting definitions.
        let result = attest();
        if let Ok(mut cache) = self.arithmetic_attestation.lock() {
            if !cache
                .as_ref()
                .is_some_and(|entry| entry.generation.matches(self))
            {
                *cache = Some(ArithmeticAttestation {
                    generation: ArithmeticGeneration::of(self),
                    results: [None; 6],
                    reflection: [None; 7],
                    numerals: None,
                });
            }
            if let Some(entry) = cache.as_mut() {
                write(entry, result);
            }
        }
        result
    }

    /// Reuse a structural arithmetic attestation only for the exact global
    /// registry generation it inspected. Caller transparency/rewrite policy is
    /// intentionally checked outside this cache.
    pub(crate) fn cached_integer_arithmetic(
        &self,
        operation: &str,
        attest: impl FnOnce() -> bool,
    ) -> bool {
        let slot = match operation {
            "add" => 0,
            "mul" => 1,
            "sub" => 2,
            "le" => 3,
            "div" => 4,
            "mod" => 5,
            _ => return false,
        };
        self.attested(
            |entry| entry.results[slot],
            |entry, result| entry.results[slot] = Some(result),
            attest,
        )
    }

    /// [`Context::cached_integer_arithmetic`] for the defined reflection operations: `slot` is the
    /// operation's position in `reflection::DEFINED`, and the answer lives as long as every registry
    /// it read.
    pub(crate) fn cached_reflection_attestation(
        &self,
        slot: usize,
        attest: impl FnOnce() -> bool,
    ) -> bool {
        self.attested(
            |entry| entry.reflection[slot],
            |entry, result| entry.reflection[slot] = Some(result),
            attest,
        )
    }

    /// Whether `Nat` and `Int` are the canonical families ([`CanonicalNumerals`]), decided once per
    /// registry generation.
    pub(crate) fn canonical_numerals(&self) -> CanonicalNumerals {
        self.attested(
            |entry| entry.numerals,
            |entry, numerals| entry.numerals = Some(numerals),
            || crate::int_inductive::canonical_numerals_of(self),
        )
    }

    /// Insert `value` at `name` in one of the registries a declaration lands in, and log it: the
    /// one door into those registries, so the log cannot miss a registration
    /// (`context_admission_log_lock`).
    pub(crate) fn record<V: Clone>(
        &mut self,
        table: fn(&mut Context) -> &mut Arc<HashMap<String, V>>,
        name: String,
        value: V,
    ) {
        Arc::make_mut(table(self)).insert(name.clone(), value);
        self.log(name, false);
    }

    fn log(&mut self, name: String, removed: bool) {
        let prev = self.admissions.take();
        self.admissions = Some(Arc::new(LogEntry {
            name,
            removed,
            prev,
        }));
    }

    /// How `name` is registered, read raw (a pending registration included) and by one precedence:
    /// an inductive family, a definition, a universe-polymorphic definition, an axiom. A
    /// constructor is not a registration of its own here; its family's is
    /// ([`Context::constructor_inductive`]).
    pub(crate) fn registration(&self, name: &str) -> Option<Registration<'_>> {
        if let Some(arity) = self.inductives.get(name) {
            return Some(Registration::Inductive(arity));
        }
        if let Some(definition) = self.definitions.get(name) {
            return Some(Registration::Definition(&definition.1, &definition.2));
        }
        if let Some(poly) = self.universe_polys.get(name) {
            return Some(Registration::UniversePoly(&poly.0, &poly.1, &poly.2));
        }
        self.declarations.get(name).map(Registration::Axiom)
    }

    /// Install the standard library's quotient certificate. A primitive it certifies anew changes
    /// how the core reads that name (a certified primitive is part of the quotient, not an axiom),
    /// so each is logged.
    pub(crate) fn certify_prelude_quotient(
        &mut self,
        certificate: crate::quotient::PreludeCertificate,
    ) {
        let fresh: Vec<String> = certificate
            .names()
            .filter(|n| {
                !self
                    .prelude_quotient
                    .as_ref()
                    .is_some_and(|held| held.names().any(|m| m == *n))
            })
            .map(str::to_string)
            .collect();
        self.prelude_quotient = Some(Arc::new(certificate));
        for name in fresh {
            self.log(name, false);
        }
    }

    /// Install the certificate of an observational equality: its constants compute from now on, so
    /// each one it certifies anew is logged.
    pub(crate) fn certify_observational(&mut self, certificate: crate::observational::Certificate) {
        let fresh: Vec<String> = certificate
            .names()
            .filter(|n| {
                !self
                    .observational
                    .as_ref()
                    .is_some_and(|held| held.names().any(|m| m == *n))
            })
            .map(str::to_string)
            .collect();
        self.observational = Some(Arc::new(certificate));
        for name in fresh {
            self.log(name, false);
        }
    }

    /// The newest entry of this context's admission log.
    pub(crate) fn admissions(&self) -> Option<&Arc<LogEntry>> {
        self.admissions.as_ref()
    }

    /// Create an empty context.
    pub fn new() -> Self {
        Context {
            checking_order_recurrence: false,
            admissions: None,
            arithmetic_attestation: Arc::new(std::sync::Mutex::new(None)),
            order_attestation: Arc::new(std::sync::Mutex::new(None)),
            checked_opaque: Arc::new(HashMap::new()),
            checked_opaque_graph: Arc::new(HashMap::new()),
            checked_opaque_dependents: Arc::new(HashMap::new()),
            quotient_primitives: Arc::new(HashMap::new()),
            prelude_quotient: None,
            observational: None,
            bindings: HashMap::new(),
            inductives: Arc::new(HashMap::new()),
            constructors: Arc::new(HashMap::new()),
            inductive_params: Arc::new(HashMap::new()),
            inductive_inferred_params: Arc::new(HashMap::new()),
            constructor_order: Arc::new(HashMap::new()),
            declarations: Arc::new(HashMap::new()),
            definitions: Arc::new(HashMap::new()),
            hints: Arc::new(Vec::new()),
            universe_polys: Arc::new(HashMap::new()),
            instances: Arc::new(Vec::new()),
            instance_priorities: Arc::new(Vec::new()),
            instance_scopes: Arc::new(Vec::new()),
            instance_by_head: Arc::new(HashMap::new()),
            instance_open: Arc::new(Vec::new()),
            coercion_scopes: Arc::new(Vec::new()),
            simp_scopes: Arc::new(Vec::new()),
            hint_scopes: Arc::new(Vec::new()),
            scope_depth: 0,
            reducibility: Arc::new(HashMap::new()),
            reducibility_undo: Arc::new(Vec::new()),
            simp_rules: Arc::new(Vec::new()),
            simp_by_head: Arc::new(HashMap::new()),
            fallback_instances: Arc::new(Vec::new()),
            coercions: Arc::new(Vec::new()),
            binder_kinds: Arc::new(HashMap::new()),
            param_defaults: Arc::new(HashMap::new()),
            implicit_args: Arc::new(HashMap::new()),
            structures: Arc::new(HashMap::new()),
            derived_aux: Arc::new(HashMap::new()),
            equations_suspended: false,
            derived_aux_owner: Arc::new(HashMap::new()),
            mutual_blocks: Arc::new(HashMap::new()),
            pending: Arc::new(HashMap::new()),
            pending_waiters: Arc::new(HashMap::new()),
            auto_aux: false,
            dirty_aux: Arc::new(Vec::new()),
            refused: Arc::new(Vec::new()),
            poly_types: Arc::new(HashMap::new()),
            definition_deps: Arc::new(HashMap::new()),
            definition_heights: Arc::new(HashMap::new()),
            referenced: Arc::new(std::collections::HashSet::new()),
            native_positivity_dependencies: Arc::new(std::collections::HashSet::new()),
            mentioned_by: Arc::new(HashMap::new()),
            mentions: Arc::new(HashMap::new()),
            eliminations: Arc::new(HashMap::new()),
            shadows: Arc::new(Vec::new()),
            shadow_origins: Arc::new(HashMap::new()),
            #[cfg(test)]
            meaning_changes_permitted: false,
        }
    }

    // ==========================================================================================
    // THE STABLE-MEANING INVARIANT — a registered global never changes meaning under a dependent
    // ==========================================================================================
    //
    // Lean's environment is append-only: a name is declared once, so nothing checked against it
    // can ever find it meaning something else. LOGOS lets a name be REDECLARED — the REPL is a
    // notebook, and a fixture may re-register a shape — and that is sound exactly while nothing
    // was checked against the old meaning. Without the gate three routes led to a closed proof of
    // `False` both kernels agreed on: redeclaring a definition a theorem was proved about;
    // adding a constructor to an inductive a theorem had eliminated exhaustively; and
    // registering a constructor under a name that was still a definition, so typing read one map
    // and δ another. Locked by `tests/stable_meaning.rs`.

    /// Whether `name` is registered as a global of any kind.
    fn is_registered_global(&self, name: &str) -> bool {
        self.inductives.contains_key(name)
            || self.constructors.contains_key(name)
            || self.declarations.contains_key(name)
            || self.definitions.contains_key(name)
            || self.universe_polys.contains_key(name)
            || self.poly_types.contains_key(name)
    }

    /// Borrow every occupied/reserved global identity for private scratch namespace selection.
    /// Duplicates are deliberate: no allocation or registration change is needed for this read.
    pub(crate) fn private_specialization_names(&self) -> impl Iterator<Item = &str> {
        self.inductives
            .keys()
            .chain(self.constructors.keys())
            .chain(self.declarations.keys())
            .chain(self.definitions.keys())
            .chain(self.universe_polys.keys())
            .chain(self.poly_types.keys())
            .chain(self.pending.keys())
            .chain(self.referenced.iter())
            .chain(self.native_positivity_dependencies.iter())
            .map(String::as_str)
    }

    /// Hash-map scans visit vacant buckets too; expose their conservative aggregate capacity.
    pub(crate) fn private_specialization_name_capacity(&self) -> usize {
        [
            self.inductives.capacity(),
            self.constructors.capacity(),
            self.declarations.capacity(),
            self.definitions.capacity(),
            self.universe_polys.capacity(),
            self.poly_types.capacity(),
            self.pending.capacity(),
            self.referenced.capacity(),
            self.native_positivity_dependencies.capacity(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }

    /// The DECLARATION UNIT `name` belongs to: what one declaration introduces, and what is
    /// regenerated with it — an inductive with its mutual siblings and every member's
    /// constructors, structure projections and derived auxiliaries; or a definition with the
    /// auxiliaries derived from it. Members of one unit are never each other's dependents.
    /// The declaration `name` belongs to: itself, or — for a constructor, a derived auxiliary or a
    /// structure projection — the declaration that generated it, climbed to the top.
    fn unit_root(&self, name: &str) -> String {
        let mut root = name.to_string();
        let mut climbed = std::collections::HashSet::new();
        while climbed.insert(root.clone()) {
            let owner = self
                .constructors
                .get(&root)
                .map(|(owner, _)| owner.clone())
                .or_else(|| self.derived_aux_owner.get(&root).cloned())
                .or_else(|| {
                    self.structures
                        .iter()
                        .find(|(_, info)| info.projections.iter().any(|p| p == &root))
                        .map(|(owner, _)| owner.clone())
                });
            match owner {
                Some(owner) => root = owner,
                None => break,
            }
        }
        root
    }

    fn declaration_unit(&self, name: &str) -> std::collections::HashSet<String> {
        let root = self.unit_root(name);
        let mut todo: Vec<String> = match self.mutual_blocks.get(&root) {
            Some(block) => block
                .iter()
                .cloned()
                .chain(std::iter::once(root.clone()))
                .collect(),
            None => vec![root],
        };
        let mut unit = std::collections::HashSet::new();
        while let Some(member) = todo.pop() {
            if !unit.insert(member.clone()) {
                continue;
            }
            if let Some(constructors) = self.constructor_order.get(&member) {
                todo.extend(constructors.iter().cloned());
            }
            if let Some(auxiliaries) = self.derived_aux.get(&member) {
                todo.extend(auxiliaries.iter().cloned());
            }
            if let Some(info) = self.structures.get(&member) {
                todo.extend(info.projections.iter().cloned());
            }
        }
        unit
    }

    /// Every registered global OUTSIDE `name`'s declaration unit whose type, sort or body mentions
    /// a member of it — the declarations that were checked against its current meaning. Sorted,
    /// so a refusal names its witnesses deterministically.
    fn external_dependents(&self, name: &str) -> Vec<String> {
        self.dependents_of_unit(&self.declaration_unit(name))
    }

    fn dependents_of_unit(&self, unit: &std::collections::HashSet<String>) -> Vec<String> {
        let mut found = std::collections::BTreeSet::new();
        for member in unit {
            if let Some(users) = self.mentioned_by.get(member) {
                for user in users {
                    if !unit.contains(user) && self.is_registered_global(user) {
                        found.insert(user.clone());
                    }
                }
            }
        }
        found.into_iter().collect()
    }

    /// Every registered global that depends on `unit` TRANSITIVELY — through any chain of
    /// mentions, so a `match` written against a type alias of a member is still found.
    fn transitive_dependents_of_unit(
        &self,
        unit: &std::collections::HashSet<String>,
    ) -> std::collections::BTreeSet<String> {
        let mut closure = std::collections::BTreeSet::new();
        let mut todo: Vec<&str> = unit.iter().map(String::as_str).collect();
        while let Some(name) = todo.pop() {
            let Some(users) = self.mentioned_by.get(name) else {
                continue;
            };
            for user in users {
                if !unit.contains(user)
                    && self.is_registered_global(user)
                    && closure.insert(user.clone())
                {
                    todo.push(user);
                }
            }
        }
        closure
    }

    /// Whether `inductive`'s one constructor stores NOTHING past the parameters — the shape whose
    /// constructor set is observable without being named: unit η makes any two values of a
    /// fieldless data singleton convertible, and K reduces an eliminator on any proof of a
    /// fieldless propositional one. A derivation may have used either anywhere.
    fn is_fieldless_singleton(&self, inductive: &str) -> bool {
        let constructors = self.get_constructors(inductive);
        constructors.len() == 1
            && count_leading_pis(constructors[0].1) <= self.inductive_num_params(inductive)
    }

    /// Whether `dependent` was checked against a member's CONSTRUCTOR SET — so that a new
    /// constructor would leave it certified for a statement that is no longer true. It is observed
    /// by NAMING a member other than the inductive itself (a constructor, the recursor,
    /// `noConfusion`, `below`, a projection…), or by a `match` whose motive eliminates a member or
    /// anything defined in terms of one — a `match` whose motive does not show what it eliminates
    /// is counted as one.
    ///
    /// A type that merely MENTIONS the inductive observes nothing: `TTactics := TList (Syntax →
    /// Derivation)` stays true however many constructors `Syntax` gains, which is what lets the
    /// prelude give `Int` its constructors after the declarations stated over the opaque carrier.
    fn observes_constructor_set(
        &self,
        dependent: &str,
        inductive: &str,
        unit: &std::collections::HashSet<String>,
        observable: &std::collections::BTreeSet<String>,
    ) -> bool {
        let names_more = self.mentions.get(dependent).is_some_and(|targets| {
            targets
                .iter()
                .any(|target| target != inductive && unit.contains(target))
        });
        names_more
            || self.eliminations.get(dependent).is_some_and(|eliminated| {
                eliminated.opaque
                    || eliminated
                        .heads
                        .iter()
                        .any(|head| unit.contains(head) || observable.contains(head))
            })
    }

    /// THE GATE every door asks before it (re)declares `name`, told how the incoming
    /// declaration relates to what `name` already means.
    ///
    /// * a name that is not registered, or that nothing outside its declaration unit depends on,
    ///   is registered as usual — the notebook convenience, where it is sound;
    /// * a body for a bodyless declaration at its identical type is registered too (a
    ///   [`Change::Refinement`]);
    /// * otherwise a name something depends on keeps its meaning: an identical redeclaration is a
    ///   no-op, and any other is REFUSED, naming the dependents checked against the old one.
    fn admit_meaning(&self, name: &str, change: Change) -> crate::error::KernelResult<Admission> {
        #[cfg(test)]
        if self.meaning_changes_permitted {
            return Ok(Admission::Register);
        }
        if !self.is_registered_global(name) || change == Change::Refinement {
            return Ok(Admission::Register);
        }
        let dependents = self.external_dependents(name);
        if dependents.is_empty() {
            return Ok(Admission::Register);
        }
        if change == Change::Identical {
            return Ok(Admission::Keep);
        }
        Err(self.meaning_is_relied_on(name, crate::error::MeaningChange::Redeclared, dependents))
    }

    /// `Identical` when `same` holds, else `Different` — the two-way question most doors ask.
    fn change_if(same: bool) -> Change {
        if same {
            Change::Identical
        } else {
            Change::Different
        }
    }

    /// The gate for ADDING a constructor to `inductive`: every dependent of an inductive was
    /// checked against its constructor set — an exhaustive `match`, a recursor, structure η —
    /// so a new constructor is admitted only while nothing outside the declaration depends on it.
    fn admit_extension(&self, inductive: &str) -> crate::error::KernelResult<()> {
        // A STRUCTURE's η rule (`s ≡ mk (s.1) … (s.n)`) is sound only because it has exactly one
        // constructor; a second one would be η-equal to a value it cannot be.
        if self.structures.contains_key(inductive) {
            return Err(crate::error::KernelError::InvalidDeclaration(format!(
                "`{inductive}` is a structure: it has exactly one constructor, and its η rule \
                 depends on that, so it cannot be extended"
            )));
        }
        #[cfg(test)]
        if self.meaning_changes_permitted {
            return Ok(());
        }
        let unit = self.declaration_unit(inductive);
        let closure = self.transitive_dependents_of_unit(&unit);
        let observers: Vec<String> = if self.is_fieldless_singleton(inductive) {
            closure.into_iter().collect()
        } else {
            closure
                .iter()
                .filter(|dependent| {
                    self.observes_constructor_set(dependent, inductive, &unit, &closure)
                })
                .cloned()
                .collect()
        };
        if observers.is_empty() {
            return Ok(());
        }
        Err(self.meaning_is_relied_on(inductive, crate::error::MeaningChange::Extended, observers))
    }

    /// The gates a constructor passes before it is registered. The identical constructor already
    /// registered is kept as it is. Otherwise its inductive must exist, its NAME must not carry a
    /// meaning something relies on, and a constructor the inductive does not already list is
    /// admitted only while nothing outside the declaration depends on the inductive.
    fn admit_constructor(
        &self,
        name: &str,
        inductive: &str,
        ty: &Term,
    ) -> crate::error::KernelResult<Admission> {
        let current = self.constructors.get(name);
        if current.is_some_and(|(owner, existing)| owner == inductive && same_meaning(existing, ty))
        {
            return Ok(Admission::Keep);
        }
        if !self.is_inductive(inductive) {
            return Err(crate::error::KernelError::CertificationError(format!(
                "constructor '{name}' names '{inductive}', which is not a registered inductive"
            )));
        }
        self.admit_meaning(name, Change::Different)?;
        if !current.is_some_and(|(owner, _)| owner == inductive) {
            self.admit_extension(inductive)?;
        }
        Ok(Admission::Register)
    }

    /// The refusal, naming each dependent by the declaration it belongs to: `useA_eq_def` is
    /// `useA`'s, and the author wrote `useA`.
    fn meaning_is_relied_on(
        &self,
        name: &str,
        change: crate::error::MeaningChange,
        dependents: Vec<String>,
    ) -> crate::error::KernelError {
        let dependents: std::collections::BTreeSet<String> = dependents
            .iter()
            .map(|dependent| self.unit_root(dependent))
            .collect();
        crate::error::KernelError::StableMeaningViolation {
            name: name.to_string(),
            change,
            dependents: dependents.into_iter().collect(),
        }
    }

    /// Unit tests of the caches behind the stable-meaning gate: from here on this context accepts
    /// the changes of meaning the gate refuses, so a test can check that a cache notices one.
    #[cfg(test)]
    pub(crate) fn permit_meaning_changes_for_cache_tests(&mut self) {
        self.meaning_changes_permitted = true;
    }

    /// Unit tests: apply `change` to a copy of this context under the stable-meaning gate. When
    /// the gate refuses it — answered as `StableMeaningViolation`, or recorded by a door that
    /// cannot answer — the copy as the refusal left it; `None` when the change was admitted. The
    /// context itself is untouched.
    #[cfg(test)]
    pub(crate) fn refused_meaning_change_for_test(
        &self,
        change: impl FnOnce(&mut Context) -> crate::error::KernelResult<()>,
    ) -> Option<Context> {
        let mut copy = self.clone();
        let recorded = copy.refused.len();
        let answered = change(&mut copy);
        let refused = matches!(
            answered,
            Err(crate::error::KernelError::StableMeaningViolation { .. })
        ) || copy.refused[recorded..]
            .iter()
            .any(|(_, why)| why.starts_with("Stable meaning violation"));
        refused.then_some(copy)
    }

    // ==========================================================================================
    // SHADOWS — a written name that denotes a fresh identity
    // ==========================================================================================
    //
    // The gate above never lets a declaration change what a registered key means, and a document
    // that writes `Definition add …` has not asked it to: it has introduced ITS `add`. That
    // declaration is registered under a FRESH IDENTITY, a key no surface can spell, and the table
    // below records that the written name now denotes it. Every key keeps its one meaning; what
    // moves is which key a written name reaches, which is name resolution — the surface asks this
    // table, and nothing in the kernel reads it.

    /// The shadow table, as `(written name, identity)` pairs, in the order the written names were
    /// first shadowed.
    pub fn shadows(&self) -> &[(String, String)] {
        &self.shadows
    }

    /// The identity the written absolute name `written` denotes, when a shadow redirects it: the
    /// table's own entry, or — for a constant DERIVED from a shadowing declaration (a recursor, an
    /// equation lemma, a structure's constructor or projection) — that constant, whose key spells
    /// the identity where the written name spells the name: `B2_rec` reaches `B2✝⟨1⟩_rec` exactly
    /// when `B2` reaches `B2✝⟨1⟩`.
    pub fn shadow_of(&self, written: &str) -> Option<String> {
        if self.shadows.is_empty() {
            return None;
        }
        if let Some((_, identity)) = self.shadows.iter().find(|(w, _)| w == written) {
            return Some(identity.clone());
        }
        // A derived constant is spelled with its declaration's name inside it (`B2_rec`,
        // `decEqB2`), so it is found by putting the identity in the name's place — at each place
        // the name occurs — and asking whether that key is derived from the identity.
        self.shadows.iter().find_map(|(w, identity)| {
            written.match_indices(w.as_str()).find_map(|(at, _)| {
                let key = format!("{}{identity}{}", &written[..at], &written[at + w.len()..]);
                (self.is_registered_global(&key) && self.owners(&key).any(|o| o == *identity))
                    .then_some(key)
            })
        })
    }

    /// The declarations `name` was generated by, nearest first: a constructor's inductive, a
    /// derived constant's source, a projection's structure — climbed to the top of its unit.
    fn owners<'c>(&'c self, name: &str) -> impl Iterator<Item = String> + 'c {
        let mut climbed = std::collections::HashSet::from([name.to_string()]);
        let mut at = name.to_string();
        std::iter::from_fn(move || {
            let owner = self
                .constructors
                .get(&at)
                .map(|(owner, _)| owner.clone())
                .or_else(|| self.derived_aux_owner.get(&at).cloned())
                .or_else(|| {
                    self.structures
                        .iter()
                        .find(|(_, info)| info.projections.iter().any(|p| *p == at))
                        .map(|(owner, _)| owner.clone())
                })?;
            climbed.insert(owner.clone()).then(|| {
                at = owner.clone();
                owner
            })
        })
    }

    /// The written name a minted identity was minted for; `None` for every key no shadow minted.
    pub fn shadow_origin(&self, identity: &str) -> Option<&str> {
        self.shadow_origins.get(identity).map(String::as_str)
    }

    /// The minted identity `key` IS, or is DERIVED from (a recursor, an equation lemma, a
    /// structure's constructor or projection, spelled with the identity inside its own key).
    /// `None` for every key no shadow minted or derived from.
    pub fn shadow_root(&self, key: &str) -> Option<String> {
        // Every identity carries the mark, so a key without it was neither minted nor derived.
        if self.shadow_origins.is_empty() || !key.contains(SHADOW_MARK) {
            return None;
        }
        std::iter::once(key.to_string())
            .chain(self.owners(key))
            .find(|candidate| {
                self.shadow_origins.contains_key(candidate) && key.contains(candidate.as_str())
            })
    }

    /// Whether the minted identity `identity` is the one its written name denotes NOW — `false`
    /// once a later declaration of that name has been shadowed in over it.
    pub fn is_live_shadow(&self, identity: &str) -> bool {
        self.shadow_origins.get(identity).is_some_and(|origin| {
            self.shadows
                .iter()
                .any(|(w, id)| w == origin && id == identity)
        })
    }

    /// The written absolute name that reaches `key` through a shadow, live or not: the name an
    /// identity was minted for, or — for a constant derived from an identity — its key with that
    /// name in the identity's place. `None` for a key no shadow minted or derived from.
    pub fn spelled_by(&self, key: &str) -> Option<String> {
        let root = self.shadow_root(key)?;
        Some(key.replacen(root.as_str(), &self.shadow_origins[&root], 1))
    }

    /// How `key` is WRITTEN here, for output that must read as source. A live identity is written
    /// as the name it was minted for; an identity whose name has since been shadowed again is
    /// written as that name marked inaccessible (`add✝`), the way Lean prints an inaccessible name;
    /// a constant derived from an identity is written with the identity's spelling in its place;
    /// and a declaration a shadow hides is written with the `_root_` escape, which still reaches
    /// it. Every other key is written as itself.
    pub fn written_name(&self, key: &str) -> String {
        if self.shadows.is_empty() {
            return key.to_string();
        }
        if let Some(root) = self.shadow_root(key) {
            let origin = &self.shadow_origins[&root];
            let written = if self.is_live_shadow(&root) {
                origin.clone()
            } else {
                format!("{origin}{SHADOW_MARK}")
            };
            return key.replacen(root.as_str(), &written, 1);
        }
        if self.shadow_of(key).is_some() {
            return format!("_root_.{key}");
        }
        key.to_string()
    }

    /// A fresh identity for a declaration of the written name `written`: `written✝⟨n⟩` for the
    /// least `n ≥ 1` at which nothing is registered and nothing was minted. Deterministic, so a
    /// document read twice mints the same identities; and a key already taken — even one no shadow
    /// minted — is stepped past, never reused. No identifier contains `✝` and no `«quoted name»`
    /// may contain `⟨`/`⟩`, so no surface can spell what this returns.
    pub(crate) fn mint_shadow_identity(&self, written: &str) -> String {
        (1u64..)
            .map(|n| format!("{written}{SHADOW_MARK}\u{27e8}{n}\u{27e9}"))
            .find(|key| !self.is_registered_global(key) && !self.shadow_origins.contains_key(key))
            .expect("an unbounded counter reaches a free key")
    }

    /// Record that the written name `written` now denotes `identity`. A name already shadowed
    /// keeps its place in the table and denotes the new identity; the one it denoted before stays
    /// registered, keeps every dependent it has, and keeps its origin to be written by.
    pub(crate) fn record_shadow(&mut self, written: &str, identity: &str) {
        let table = Arc::make_mut(&mut self.shadows);
        match table.iter_mut().find(|(w, _)| w == written) {
            Some(entry) => entry.1 = identity.to_string(),
            None => table.push((written.to_string(), identity.to_string())),
        }
        Arc::make_mut(&mut self.shadow_origins).insert(identity.to_string(), written.to_string());
    }

    /// Record that the registered global `name` mentions every global in `term`, and what
    /// `term` eliminates.
    fn record_mentions(&mut self, name: &str, term: &Term) {
        let mut found: Vec<&str> = Vec::new();
        collect_global_mentions(term, &mut found);
        let found: Vec<String> = found.into_iter().map(str::to_string).collect();
        self.record_mention_edges(name, found);
        self.record_eliminations(name, term);
    }

    fn record_eliminations(&mut self, name: &str, term: &Term) {
        let mut found = Eliminations::default();
        collect_eliminations(term, &mut found);
        if found.heads.is_empty() && !found.opaque {
            return;
        }
        let entry = Arc::make_mut(&mut self.eliminations)
            .entry(name.to_string())
            .or_default();
        entry.opaque |= found.opaque;
        for head in found.heads {
            if !entry.heads.contains(&head) {
                entry.heads.push(head);
            }
        }
    }

    fn record_mention_edges(&mut self, name: &str, targets: impl IntoIterator<Item = String>) {
        let forward = Arc::make_mut(&mut self.mentions)
            .entry(name.to_string())
            .or_default();
        let mut added: Vec<String> = Vec::new();
        for target in targets {
            if target != name && !forward.contains(&target) {
                forward.push(target.clone());
                added.push(target);
            }
        }
        if added.is_empty() {
            return;
        }
        let reverse = Arc::make_mut(&mut self.mentioned_by);
        for target in added {
            reverse.entry(target).or_default().insert(name.to_string());
        }
    }

    /// Take `name`'s mentions out of the index — it is being displaced, so it no longer depends
    /// on anything. What depends on IT stays recorded: those declarations still mention it.
    fn forget_mentions(&mut self, name: &str) {
        if self.eliminations.contains_key(name) {
            remove_shared_entry(&mut self.eliminations, name);
        }
        let Some(targets) = remove_shared_entry(&mut self.mentions, name) else {
            return;
        };
        for target in targets {
            if !self
                .mentioned_by
                .get(&target)
                .is_some_and(|users| users.contains(name) || users.is_empty())
            {
                continue;
            }
            let reverse = Arc::make_mut(&mut self.mentioned_by);
            if let Some(users) = reverse.get_mut(&target) {
                users.remove(name);
                if users.is_empty() {
                    reverse.remove(&target);
                }
            }
        }
    }

    pub(crate) fn check_native_dependency_mutation(
        &self,
        name: &str,
    ) -> crate::error::KernelResult<()> {
        if self.native_positivity_dependencies.contains(name) {
            return Err(crate::error::KernelError::CertificationError(format!(
                "declaration '{name}' is sealed by a native nested positivity check"
            )));
        }
        Ok(())
    }

    fn refuse_native_dependency_mutation(&mut self, name: &str) -> bool {
        if let Err(error) = self.check_native_dependency_mutation(name) {
            self.record_refusal(name, error);
            true
        } else {
            false
        }
    }

    /// A door that cannot return an error records its refusal in the ledger
    /// ([`Context::refused_declarations`]) instead of swallowing it.
    fn record_refusal(&mut self, name: &str, error: crate::error::KernelError) {
        Arc::make_mut(&mut self.refused).push((name.to_string(), error.to_string()));
    }

    fn check_native_dependencies(
        &self,
        dependencies: &std::collections::HashSet<String>,
    ) -> crate::error::KernelResult<()> {
        for name in dependencies {
            if self.pending.contains_key(name) {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "native positivity cannot depend on pending definition '{name}'"
                )));
            }
        }
        Ok(())
    }

    fn seal_native_dependencies(&mut self, dependencies: std::collections::HashSet<String>) {
        if dependencies.is_empty() {
            return;
        }
        // Displacing a parent removes its constructors; regenerating its
        // auxiliaries removes their definitions. Preserve these owners too.
        let mut complete = dependencies;
        let mut todo: Vec<_> = complete.iter().cloned().collect();
        while let Some(name) = todo.pop() {
            let owner = self
                .constructors
                .get(&name)
                .map(|(owner, _)| owner)
                .or_else(|| self.derived_aux_owner.get(&name));
            if let Some(owner) = owner {
                if complete.insert(owner.clone()) {
                    todo.push(owner.clone());
                }
            }
        }
        Arc::make_mut(&mut self.native_positivity_dependencies).extend(complete);
    }

    /// Record structure metadata supplied by a caller. Registering [`StructInfo`] grants structure
    /// η, so this door grants it only when [`Context::register_structure_checked`]'s decision does:
    /// the named projections must be `name`'s projections, and `info`'s constructor and parameter
    /// count must be the ones that decision found. The caller's descriptive metadata (field and
    /// parameter names, parents, output parameters) is kept. A refusal registers nothing and is
    /// recorded in [`Context::refused_declarations`], as every unchecked door records one.
    pub fn register_struct_info(&mut self, name: &str, info: StructInfo) {
        let refusal = match self.structure_eta_evidence(name, &info.projections) {
            Ok(evidence) if evidence.mk == info.mk && evidence.num_params == info.num_params => None,
            Ok(evidence) => Some(format!(
                "structure metadata for `{name}` names constructor `{}` with {} parameters, but its constructor is `{}` with {}",
                info.mk, info.num_params, evidence.mk, evidence.num_params
            )),
            Err(refusal) => Some(refusal.to_string()),
        };
        match refusal {
            Some(why) => Arc::make_mut(&mut self.refused).push((name.to_string(), why)),
            None => self.insert_struct_info(name, info),
        }
    }

    /// Insert structure metadata whose η a caller has already decided: the kernel's own structure
    /// builder (which generates the constructor and its projections) and the checked door.
    pub(crate) fn insert_struct_info(&mut self, name: &str, mut info: StructInfo) {
        if self.refuse_native_dependency_mutation(name) {
            return;
        }
        for (_, ty) in info.params.iter_mut().chain(info.fields.iter_mut()) {
            *ty = crate::annotation::stored(std::mem::replace(ty, Term::Hole));
        }
        self.invalidate_checked_opaque(name);
        Arc::make_mut(&mut self.structures).insert(name.to_string(), info);
    }

    /// The structure metadata for an inductive type name, if it is a registered
    /// structure (record). `None` for ordinary inductives — eta never fires for them.
    pub fn struct_info(&self, name: &str) -> Option<&StructInfo> {
        self.structures.get(name)
    }

    /// The parameter positions of the class `name` that are OUTPUTS — Lean's `outParam`.
    /// An output takes no part in instance MATCHING: the instance determines it, and a
    /// mismatch against what the surrounding context expected is an ordinary type error at
    /// the use site rather than a different instance. Empty for anything that is not a
    /// registered structure.
    pub fn out_params(&self, name: &str) -> &[usize] {
        self.structures
            .get(name)
            .map(|i| i.out_params.as_slice())
            .unwrap_or(&[])
    }

    /// Declare the class `name`'s output parameter positions explicitly, replacing the ones
    /// [`Context::add_structure`] inferred. The inference is a default for classes declared
    /// without annotation; this is the channel a translator uses to carry Lean's `outParam`
    /// across exactly.
    pub fn set_out_params(&mut self, name: &str, positions: Vec<usize>) {
        if let Some(info) = Arc::make_mut(&mut self.structures).get_mut(name) {
            info.out_params = positions;
        }
    }

    /// If `name` is a derived parent-forget edge (`{Child}_to{Parent}`, generated by
    /// [`Context::add_structure`] from a flattened field prefix), the parent class it forgets
    /// to. Instance resolution uses this to tell a HIERARCHY route apart from an ordinary
    /// overlapping instance: two routes to one parent that disagree are an incoherent diamond,
    /// where two direct instances are merely overlap.
    pub fn parent_edge_target(&self, name: &str) -> Option<&str> {
        let (child, parent) = name.split_once("_to")?;
        let info = self.structures.get(child)?;
        info.parents
            .iter()
            .find(|p| *p == parent)
            .map(|p| p.as_str())
    }

    /// If `ctor` is the constructor of a registered structure, return `(structure
    /// name, its info)`. Used to detect an η-expandable constructor head.
    pub fn struct_of_constructor(&self, ctor: &str) -> Option<(&str, &StructInfo)> {
        let ind = self.constructor_inductive(ctor)?;
        let info = self.structures.get(ind)?;
        Some((ind, info))
    }

    /// Record that the global `name` has `count` leading implicit parameters, so the
    /// surface elaborator inserts that many inferred arguments at each application.
    pub fn set_implicit_args(&mut self, name: &str, count: usize) {
        Arc::make_mut(&mut self.implicit_args).insert(name.to_string(), count);
    }

    /// How many leading parameters of `name` are implicit (0 if none/unknown).
    pub fn implicit_args(&self, name: &str) -> usize {
        self.implicit_args.get(name).copied().unwrap_or(0)
    }

    /// Register a typeclass instance: a `value` of type `ty` (e.g. `mk Nat Zero` of type
    /// `Inhabited Nat`). The elaborator resolves an instance-implicit argument by
    /// searching these for a `ty` that unifies with the required class type.
    ///
    /// Registered at [`DEFAULT_INSTANCE_PRIORITY`] — Lean's `instance` with no
    /// `(priority := …)`. Use [`Context::add_instance_with_priority`] to say otherwise.
    pub fn add_instance(&mut self, ty: Term, value: Term) {
        self.add_instance_with_priority(ty, value, DEFAULT_INSTANCE_PRIORITY);
    }

    /// Register a typeclass instance at an explicit PRIORITY — Lean's
    /// `instance (priority := n) …`. Higher wins; ties go to the LATER declaration.
    pub fn add_instance_with_priority(&mut self, ty: Term, value: Term, priority: i64) {
        self.register_instance(ty, value, priority, 0);
    }

    /// Register a `local instance` — Lean's `local instance`, and the thing the flat push-only
    /// `Vec` could not express: it wins for as long as the enclosing scope is open and LEAVES the
    /// database at [`Context::pop_scope`].
    ///
    /// Registered OUTSIDE any open scope this is an ordinary permanent instance, because that is
    /// what `local` means at file level in Lean: there is no `end` to revert it.
    pub fn add_local_instance(&mut self, ty: Term, value: Term) {
        self.add_local_instance_with_priority(ty, value, DEFAULT_INSTANCE_PRIORITY);
    }

    /// [`Context::add_local_instance`] at an explicit priority.
    pub fn add_local_instance_with_priority(&mut self, ty: Term, value: Term, priority: i64) {
        let owner = self.scope_depth;
        self.register_instance(ty, value, priority, owner);
    }

    /// The one door every instance registration funnels through, so the three index-aligned
    /// vectors cannot drift.
    fn register_instance(&mut self, ty: Term, value: Term, priority: i64, owner: usize) {
        let (ty, value) = (
            crate::annotation::stored(ty),
            crate::annotation::stored(value),
        );
        let index = self.instances.len();
        let head = self.discrimination_head(&ty);
        Arc::make_mut(&mut self.instances).push((ty, value));
        Arc::make_mut(&mut self.instance_priorities).push(priority);
        Arc::make_mut(&mut self.instance_scopes).push(owner);
        match head {
            Some(h) => Arc::make_mut(&mut self.instance_by_head)
                .entry(h)
                .or_default()
                .push(index),
            None => Arc::make_mut(&mut self.instance_open).push(index),
        }
        debug_assert_eq!(self.instances.len(), self.instance_priorities.len());
        debug_assert_eq!(self.instances.len(), self.instance_scopes.len());
    }

    /// The DISCRIMINATION KEY of a class type — the head symbol its conclusion is built from, or
    /// `None` when no head symbol can rule anything out.
    ///
    /// The Π telescope is stripped first, exactly as `mkInstanceKey` strips it with
    /// `forallMetaTelescopeReducing` (`Lean/Meta/Instances.lean:97-101`), so
    /// `Π{A}. Shown A → Shown (TList A)` keys on `Shown` and not on the arrow.
    ///
    /// `None` — the `star` bucket — is returned for three shapes, and each is a case where a key
    /// would be a LIE rather than an index:
    ///
    /// * a conclusion headed by a bound variable (`Π(G : Type). G`), which may become any class;
    /// * a conclusion headed by a δ-REDUCIBLE name, which unfolds to something else before
    ///   anything compares heads — Lean reduces with `reduceDT` before keying
    ///   (`Lean/Meta/DiscrTree.lean:352`) and we take the conservative branch instead; and
    /// * anything that is not an application of a name at all.
    fn discrimination_head(&self, ty: &Term) -> Option<String> {
        let mut bound: Vec<&str> = Vec::new();
        let mut cur = ty;
        while let Term::Pi {
            param, body_type, ..
        } = cur
        {
            bound.push(param);
            cur = body_type;
        }
        let mut head = cur;
        while let Term::App(f, _) = head {
            head = f;
        }
        let name = match head {
            Term::Global(n) => n,
            Term::Const { name, .. } => name,
            _ => return None,
        };
        // A binder may SHADOW a global of the same name, and a name the instance itself binds is
        // not a class head at all.
        if bound.contains(&name.as_str()) {
            return None;
        }
        // A δ-reducible name is not a key: it unfolds before any head comparison happens.
        if self.get_definition_body(name).is_some() {
            return None;
        }
        Some(name.clone())
    }

    /// Rebuild [`Context::instance_by_head`] from `instances`. Called only where the index
    /// positions SHIFT — a scope closing compacts the vector — because every other door appends.
    fn reindex_instances(&mut self) {
        let mut by_head: HashMap<String, Vec<usize>> = HashMap::new();
        let mut open: Vec<usize> = Vec::new();
        let types: Vec<Term> = self.instances.iter().map(|(t, _)| t.clone()).collect();
        for (i, ty) in types.iter().enumerate() {
            match self.discrimination_head(ty) {
                Some(h) => by_head.entry(h).or_default().push(i),
                None => open.push(i),
            }
        }
        self.instance_by_head = Arc::new(by_head);
        self.instance_open = Arc::new(open);
    }

    // ==========================================================================================
    // SCOPES — `section` / `namespace` … `end`, and what `local` means
    // ==========================================================================================

    /// Open a SCOPE — Lean's `section`/`namespace`. Everything registered `local` inside it is
    /// reverted by the matching [`Context::pop_scope`]; everything else is permanent.
    ///
    /// THE INVARIANT, stated once so no future scoped thing can get it wrong: **a scope reverts
    /// the four ATTRIBUTE DATABASES — instances, coercions, simp rules, hints — and NEVER a
    /// declaration.** A `def` written inside a `section` survives its `end`, which is why
    /// snapshotting and restoring the whole context is not a scoping mechanism: it would delete
    /// the section's real content along with its attributes.
    ///
    /// Returns the new depth, so a caller can assert its own nesting.
    pub fn push_scope(&mut self) -> usize {
        self.scope_depth += 1;
        self.scope_depth
    }

    /// Close the innermost scope — Lean's `end`. Every `local` registration made at this depth
    /// leaves the database it was in; declarations made inside are untouched.
    ///
    /// A `pop_scope` with no scope open is a no-op rather than a panic: an unbalanced `end` is a
    /// surface-syntax error, and the kernel's job is to stay consistent, not to be the diagnostic.
    pub fn pop_scope(&mut self) {
        let closing = self.scope_depth;
        if closing == 0 {
            return;
        }
        self.scope_depth -= 1;

        // Retain in lock-step so the index-aligned side-cars cannot drift. Each database keeps
        // exactly the entries whose owning scope is not the one closing.
        let instances_before = self.instances.len();
        retain_scoped2(
            &mut self.instances,
            &mut self.instance_priorities,
            &mut self.instance_scopes,
            closing,
        );
        if self.instances.len() != instances_before {
            // Positions shifted, so every stored index is stale. Rebuilt rather than patched:
            // an index that is a FUNCTION of the vector cannot come to disagree with it.
            self.reindex_instances();
        }
        retain_scoped1(&mut self.coercions, &mut self.coercion_scopes, closing);
        retain_scoped1(&mut self.hints, &mut self.hint_scopes, closing);
        if self.simp_scopes.iter().any(|d| *d == closing) {
            retain_scoped1(&mut self.simp_rules, &mut self.simp_scopes, closing);
            self.reindex_simp();
        }
        self.revert_reducibility(closing);
    }

    /// How many scopes are open (`0` = the permanent scope).
    pub fn scope_depth(&self) -> usize {
        self.scope_depth
    }

    /// Register a FALLBACK instance — one consulted only after every ordinary instance has
    /// failed to resolve the goal. This is the tier for library-shipped and DERIVED instances
    /// (`decEq{I}`, `Inhabited Nat`): they must be findable, but a user instance for the same
    /// goal must always outrank them, and they must not appear in [`Context::instances`], which
    /// is the faithful record of what was explicitly declared.
    ///
    /// Better than Lean in one respect: Lean expresses this with a numeric priority that a user
    /// instance can accidentally undercut. A separate tier cannot be undercut at all.
    pub fn add_fallback_instance(&mut self, ty: Term, value: Term) {
        let (ty, value) = (
            crate::annotation::stored(ty),
            crate::annotation::stored(value),
        );
        // Idempotent: declaration-time synthesis re-runs once per constructor, and a fallback
        // tier that grew by one entry per re-derivation would turn an n-constructor enum into n
        // identical candidates.
        if self
            .fallback_instances
            .iter()
            .any(|(t, v)| t == &ty && v == &value)
        {
            return;
        }
        Arc::make_mut(&mut self.fallback_instances).push((ty, value));
    }

    /// All registered FALLBACK instances, as `(type, value)` pairs — the lower tier, searched
    /// only once [`Context::instances`] has been exhausted.
    pub fn fallback_instances(&self) -> &[(Term, Term)] {
        &self.fallback_instances
    }

    /// Keep fallback candidates that can share the goal's rigid class head, in the
    /// existing later-first order. Open/reducible heads stay visible on either side.
    /// Recompute in the current context: no cached key can outlive a definition change.
    /// This only avoids impossible unification trials, not the fallback head scan.
    pub(crate) fn fallback_instance_search_order_for(&self, goal: &Term) -> Vec<usize> {
        let head = self.discrimination_head(goal);
        (0..self.fallback_instances.len())
            .rev()
            .filter(|&i| {
                match (
                    &head,
                    self.discrimination_head(&self.fallback_instances[i].0),
                ) {
                    (Some(expected), Some(actual)) => expected == &actual,
                    _ => true,
                }
            })
            .collect()
    }

    /// The priority of the instance registered at index `i` (Lean's default, 1000, for an
    /// instance registered before priorities existed or through [`Context::add_instance`]).
    pub fn instance_priority(&self, i: usize) -> i64 {
        self.instance_priorities
            .get(i)
            .copied()
            .unwrap_or(DEFAULT_INSTANCE_PRIORITY)
    }

    /// The order instance resolution must scan [`Context::instances`] in — LEAN'S RULE, stated
    /// once, here, so no scan site can invent its own:
    ///
    /// > higher PRIORITY first; among equal priorities, the LATER declaration first.
    ///
    /// Earliest-first — what a linear `0..n` scan gives — inverts every tie-break a Lean source
    /// relies on, silently: both readings type-check and compute different answers.
    ///
    /// Returned as indices into `instances()` so a caller can report WHICH entry won and why.
    pub fn instance_search_order(&self) -> Vec<usize> {
        self.ranked((0..self.instances.len()).collect())
    }

    /// [`Context::instance_search_order`] restricted to the candidates that could answer `goal` —
    /// Lean's `getUnify` against the discrimination tree (`Lean/Meta/SynthInstance.lean:210`),
    /// in the same order the unrestricted scan would have visited them.
    ///
    /// THE INVARIANT, and it is the only one that matters: this is a SUBSET of
    /// `instance_search_order()` containing every entry that could possibly unify, in the same
    /// relative order. So it changes what a search PAYS, never what it ANSWERS — and
    /// `S_spine_extra::the_index_keeps_every_open_candidate_visible_and_the_order_unchanged`
    /// holds it to both halves.
    ///
    /// Returns `(candidates, pruned)`, where `pruned` is how many entries the index excluded
    /// without a unification attempt. Lean's tree throws that number away; reporting it is what
    /// makes "why did this instance not fire?" answerable.
    pub fn instance_search_order_for(&self, goal: &Term) -> (Vec<usize>, usize) {
        let Some(head) = self.discrimination_head(goal) else {
            // A goal no head symbol describes must see EVERYTHING — the `star` query in
            // `getUnify`, which folds over the whole root map.
            return (self.instance_search_order(), 0);
        };
        let mut chosen: Vec<usize> = self
            .instance_by_head
            .get(&head)
            .cloned()
            .unwrap_or_default();
        chosen.extend_from_slice(&self.instance_open);
        let pruned = self.instances.len() - chosen.len();
        (self.ranked(chosen), pruned)
    }

    /// Lean's ordering key applied to a candidate set: higher PRIORITY first, then the LATER
    /// declaration. One helper, so a restricted scan and an unrestricted one cannot be ranked by
    /// two different rules.
    fn ranked(&self, mut order: Vec<usize>) -> Vec<usize> {
        // Stable sort on the descending key. Reversing the index first and sorting by descending
        // priority alone would rely on stability twice over; keying both components is explicit.
        order.sort_by_key(|&i| {
            (
                std::cmp::Reverse(self.instance_priority(i)),
                std::cmp::Reverse(i),
            )
        });
        order
    }

    /// Register a coercion `coe : from → to` — the elaborator may insert it when an
    /// argument of type `from` appears where `to` is expected.
    pub fn add_coercion(&mut self, from: Term, to: Term, coe: Term) {
        let (from, to, coe) = stored_coercion(from, to, coe);
        Arc::make_mut(&mut self.coercions).push((from, to, coe));
        Arc::make_mut(&mut self.coercion_scopes).push(0);
        debug_assert_eq!(self.coercions.len(), self.coercion_scopes.len());
    }

    /// A `local` coercion — reverted at [`Context::pop_scope`]. See [`Context::push_scope`].
    pub fn add_local_coercion(&mut self, from: Term, to: Term, coe: Term) {
        let owner = self.scope_depth;
        let (from, to, coe) = stored_coercion(from, to, coe);
        Arc::make_mut(&mut self.coercions).push((from, to, coe));
        Arc::make_mut(&mut self.coercion_scopes).push(owner);
        debug_assert_eq!(self.coercions.len(), self.coercion_scopes.len());
    }

    /// All registered coercions, as `(from, to, coe)` triples.
    pub fn coercions(&self) -> &[(Term, Term, Term)] {
        &self.coercions
    }

    /// Record a global's per-parameter kinds (implicit/explicit/instance, in order), so the
    /// elaborator can insert implicit and instance arguments at their real positions.
    pub fn set_binder_kinds(&mut self, name: &str, kinds: Vec<crate::elaborate::ParamKind>) {
        Arc::make_mut(&mut self.binder_kinds).insert(name.to_string(), kinds);
    }

    /// A global's per-parameter kinds, if recorded.
    pub fn binder_kinds(&self, name: &str) -> Option<&[crate::elaborate::ParamKind]> {
        self.binder_kinds.get(name).map(|v| v.as_slice())
    }

    /// Record a global's per-parameter NAME and DEFAULT VALUE, in telescope order — the surface
    /// information a `Term::Pi` has nowhere to put.
    ///
    /// One entry per position, `None` where the parameter has no default. The declaration door
    /// must name its anonymous binders (`TermParser::name_anonymous_binders`) BEFORE it records,
    /// or a default that mentions an earlier parameter has no name to be closed over and the
    /// elaborator would inline an open term.
    pub fn set_param_defaults(&mut self, name: &str, defaults: Vec<(String, Option<Term>)>) {
        let defaults = defaults
            .into_iter()
            .map(|(param, default)| (param, default.map(crate::annotation::stored)))
            .collect();
        Arc::make_mut(&mut self.param_defaults).insert(name.to_string(), defaults);
    }

    /// A global's per-parameter names and defaults, if recorded.
    pub fn param_defaults(&self, name: &str) -> Option<&[(String, Option<Term>)]> {
        self.param_defaults.get(name).map(|v| v.as_slice())
    }

    /// All registered typeclass instances, as `(type, value)` pairs, in DECLARATION order.
    ///
    /// This is the record of what was explicitly declared, not the order resolution scans —
    /// see [`Context::instance_search_order`] for that, and [`Context::fallback_instances`]
    /// for the lower tier.
    pub fn instances(&self) -> &[(Term, Term)] {
        &self.instances
    }

    /// Register a universe-polymorphic definition `name.{params} : ty := body`. A
    /// `Term::Const { name, levels }` reference later instantiates `params` with `levels`
    /// (the `.{ℓ…}` syntax), so one definition is reused at every level.
    pub fn add_universe_poly(&mut self, name: &str, params: Vec<String>, ty: Term, body: Term) {
        if let Err(error) = self.declare_universe_poly(name, params, ty, body) {
            Arc::make_mut(&mut self.refused).push((name.to_string(), error.to_string()));
        }
    }

    /// Check and register a definition with an explicit universe parameter telescope.
    pub fn declare_universe_poly(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
        body: Term,
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_definition(
            self,
            name,
            Some(&params),
            &ty,
            &body,
        )];
        let result = self.declare_definition(name, Some(params), ty, body);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    fn check_level_telescope<'a>(
        kind: &str,
        name: &str,
        params: &[String],
        terms: impl IntoIterator<Item = &'a Term>,
    ) -> crate::error::KernelResult<()> {
        let mut bound = std::collections::HashSet::new();
        for param in params {
            if !bound.insert(param.as_str()) {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "{kind} '{name}' repeats universe parameter '{param}'"
                )));
            }
        }
        for term in terms {
            for level in crate::term::free_level_vars(term) {
                if !bound.contains(level.as_str()) {
                    return Err(crate::error::KernelError::CertificationError(format!(
                        "{kind} '{name}' has unbound universe parameter '{level}'"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Look up a universe-polymorphic definition: `(universe params, type, body)`.
    pub fn get_universe_poly(&self, name: &str) -> Option<&(Vec<String>, Term, Term)> {
        if self.pending.contains_key(name) {
            return None;
        }
        self.universe_polys.get(name).map(AsRef::as_ref)
    }

    /// Declare a UNIVERSE-POLYMORPHIC INDUCTIVE: `name.{params…} : arity`, where `arity` may
    /// mention the level variables in `params`.
    ///
    /// The ordinary inductive registry is populated too, at the same arity, so `is_inductive`,
    /// `get_constructors`, the params/indices split and every `Term::Global` reference behave
    /// exactly as they do for a monomorphic declaration. What this adds is the LEVEL TELESCOPE, so
    /// a reference that writes its levels — `Const { name, levels }` — can be resolved and
    /// instantiated instead of being reported as an unbound name.
    ///
    /// ★ ONE LINE MAKES IT VISIBLE TO TYPING, and it is the same line in both kernels:
    /// `type_checker::lookup_const` and `recheck`'s `Db::Const` arm each read
    /// [`Context::get_universe_poly`] and must read [`Context::poly_declaration`] instead — one
    /// lookup that answers for a polymorphic DEFINITION and a polymorphic TYPE alike, so the two
    /// kernels cannot come to disagree about which constants carry a level telescope. Reduction
    /// must keep reading `get_universe_poly`: a type former has no body and must stay inert.
    pub fn add_universe_poly_inductive(
        &mut self,
        name: &str,
        params: Vec<String>,
        arity: Term,
        num_params: usize,
    ) {
        if self.refuse_native_dependency_mutation(name) {
            return;
        }
        let arity = self.stored_term(arity);
        let unchanged = self
            .poly_types
            .get(name)
            .is_some_and(|(declared, declared_arity)| {
                declared == &params && same_meaning(declared_arity, &arity)
            })
            && self
                .inductives
                .get(name)
                .is_some_and(|current| same_meaning(current, &arity))
            && self.inductive_num_params(name) == num_params;
        match self.admit_meaning(name, Self::change_if(unchanged)) {
            Err(refusal) => return self.record_refusal(name, refusal),
            Ok(Admission::Keep) => return,
            Ok(Admission::Register) => {}
        }
        self.add_indexed_inductive(name, arity.clone(), num_params);
        self.record_mentions(name, &arity);
        self.record(|c| &mut c.poly_types, name.to_string(), (params, arity));
    }

    /// A name's LEVEL TELESCOPE and the type it stands at — for a polymorphic type declaration or
    /// a polymorphic definition alike, in that order.
    ///
    /// The ordering is the same one [`Context::get_global`] uses (a header outranks a body), and
    /// it matters for exactly one case: a name re-declared from a definition into an inductive
    /// must resolve as the inductive, or typing would read a level telescope the reducer no
    /// longer has a body for.
    pub fn poly_declaration(&self, name: &str) -> Option<(&[String], &Term)> {
        if self.pending.contains_key(name) {
            return None;
        }
        if let Some((params, arity)) = self.poly_types.get(name) {
            return Some((params.as_slice(), arity));
        }
        // Constructors share their parent's explicit telescope, including unused
        // parameters and declaration order. Inferring one from free variables
        // would silently drop or reorder those arguments.
        if let Some((parent, ty)) = self.constructors.get(name) {
            if let Some((params, _)) = self.poly_types.get(parent) {
                return Some((params.as_slice(), ty));
            }
        }
        let (params, ty, _body) = self.universe_polys.get(name)?.as_ref();
        Some((params.as_slice(), ty))
    }

    /// The level parameters a bare reference to `name` stands at where no argument solves them
    /// (`term`'s rule 4): its declared telescope, or the level variables of its stored type. `None`
    /// for a monomorphic or unknown name.
    pub(crate) fn own_levels(&self, name: &str) -> Option<Vec<String>> {
        if let Some((params, _)) = self.poly_declaration(name) {
            return (!params.is_empty()).then(|| params.to_vec());
        }
        let ty = self.get_global(name)?;
        crate::term::mentions_level_var(ty).then(|| crate::term::free_level_vars(ty))
    }

    /// A declaration's term as the context stores it: erased ([`crate::annotation::stored`]), each
    /// bare reference that is not an application's head written at its own levels
    /// ([`crate::term::bare_references_at_own_levels`]).
    pub(crate) fn stored_term(&self, term: Term) -> Term {
        let mut term = crate::annotation::stored(term);
        crate::term::bare_references_at_own_levels(&mut term, &|name| self.own_levels(name));
        term
    }

    /// Is `name` a universe-polymorphic TYPE declaration — one with a level telescope and no body?
    pub fn is_universe_poly_inductive(&self, name: &str) -> bool {
        self.is_inductive(name) && self.poly_types.contains_key(name)
    }

    /// Add a local binding to this context (mutates in place).
    pub fn add(&mut self, name: &str, ty: Term) {
        self.bindings.insert(
            name.to_string(),
            Arc::new(LocalBinding {
                ty: crate::annotation::stored(ty),
                free_names: std::sync::OnceLock::new(),
            }),
        );
    }

    /// Closed global programs are certified in a fresh local telescope. This
    /// policy follows Context clones onto any stack-growth worker thread.
    pub(crate) fn order_recurrence_context(&self) -> Self {
        let mut context = self.clone();
        context.bindings.clear();
        context.checking_order_recurrence = true;
        context
    }

    /// Look up a local variable's type in the context.
    pub fn get(&self, name: &str) -> Option<&Term> {
        self.bindings.get(name).map(|binding| &binding.ty)
    }

    /// Sparse table capacity is copied by Context::clone even when few locals remain.
    pub(crate) fn local_binding_capacity(&self) -> usize {
        self.bindings.capacity()
    }

    /// Borrow the local-name payloads copied by Context::clone, for bounded builders.
    pub(crate) fn local_binding_names(&self) -> impl Iterator<Item = &str> {
        self.bindings.keys().map(String::as_str)
    }

    /// Whether an independent entry must consider active local assumptions.
    pub(crate) fn has_local_bindings(&self) -> bool {
        !self.bindings.is_empty()
    }

    /// Names reserved by active local keys or free in their stored types.
    pub(crate) fn local_name_in_use(&self, name: &str) -> bool {
        self.bindings.contains_key(name) || self.local_types_mention(name)
    }

    /// Whether `name` is an active local key.
    pub(crate) fn has_local(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    /// Whether `name` occurs free in some active local's STORED TYPE — the occurrence a new
    /// binder named `name` would silently re-point at itself.
    /// Each immutable annotation is analyzed at most once across context clones.
    pub(crate) fn local_types_mention(&self, name: &str) -> bool {
        self.bindings.values().any(|binding| match &binding.ty {
            Term::Var(local) => local == name,
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {
                false
            }
            other => binding
                .free_names
                .get_or_init(|| crate::term_store::free_names_in_term(other))
                .contains(name),
        })
    }

    /// Create a new context extended with an additional local binding.
    ///
    /// Does not mutate the original context.
    pub fn extend(&self, name: &str, ty: Term) -> Context {
        let mut new_ctx = self.clone();
        new_ctx.add(name, ty);
        new_ctx
    }

    /// Register an inductive type.
    ///
    /// The `sort` is the type of the inductive (e.g., Type 0 for Nat).
    ///
    /// All of the arity is treated as uniform parameters (0 indices) unless
    /// [`set_inductive_params`](Self::set_inductive_params) records a smaller parameter
    /// count — see [`add_indexed_inductive`](Self::add_indexed_inductive).
    pub fn add_inductive(&mut self, name: &str, sort: Term) {
        if let Err(refusal) = self.declare_inductive(name, sort) {
            self.record_refusal(name, refusal);
        }
    }

    /// [`Context::add_inductive`] answering instead of recording: the header is registered, kept
    /// (the identical header something depends on is not a change), or refused with the reason.
    pub fn declare_inductive(&mut self, name: &str, sort: Term) -> crate::error::KernelResult<()> {
        self.check_native_dependency_mutation(name)?;
        let sort = self.stored_term(sort);
        // A header something depends on keeps its meaning — and, being kept, its constructors.
        let unchanged = self
            .inductives
            .get(name)
            .is_some_and(|current| same_meaning(current, &sort));
        if self.admit_meaning(name, Self::change_if(unchanged))? == Admission::Keep {
            return Ok(());
        }
        // DISPLACE any previous generation of this name before installing the new one.
        //
        // `add_constructor` PUSHES onto `constructor_order`, so re-declaring a name that already
        // has constructors used to ACCUMULATE them: the second generation's constructors were
        // appended to the first's, `get_constructors` then returned more than the inductive has,
        // and `derive_recursor`/`Match` coverage saw the wrong count. That is one root cause behind
        // a whole class of failures whenever a test declares a name the prelude also owns.
        //
        // It is also half of conformance defect 3: a name could be an inductive AND a definition at
        // once, with `get_global` typing it from the inductive map while `get_definition_body`
        // reduced it from the definitions map. Displacing makes one name mean one thing.
        self.displace(name);
        self.record_mentions(name, &sort);
        self.record(|c| &mut c.inductives, name.to_string(), sort);
        // A name has entered scope, so whatever was deferred waiting for it can be checked now.
        // Every door that BINDS a name has to do this, not just `declare`: the prelude declares
        // `TTactics := TList (Syntax → Derivation)` long before `Syntax` is an inductive, and an
        // obligation only `declare` could discharge would stand for ever.
        self.retry_pending(name);
        Ok(())
    }

    /// Borrow internally checked opaque audit evidence for the current declaration.
    pub(crate) fn checked_opaque_dependencies(
        &self,
        name: &str,
    ) -> Option<&crate::prelude::CheckedOpaqueDependencies> {
        self.checked_opaque.get(name)
    }

    fn record_checked_opaque_edges(
        &mut self,
        name: String,
        edges: std::collections::BTreeSet<String>,
    ) {
        // Mutation invalidates this node before its meaning changes. Existing
        // nodes therefore belong to the same current declaration generation.
        if self.checked_opaque_graph.contains_key(&name) {
            return;
        }
        let reverse = Arc::make_mut(&mut self.checked_opaque_dependents);
        for dependency in &edges {
            reverse
                .entry(dependency.clone())
                .or_default()
                .insert(name.clone());
        }
        Arc::make_mut(&mut self.checked_opaque_graph).insert(name, Arc::new(edges));
    }

    fn invalidate_checked_opaque(&mut self, name: &str) {
        if self.checked_opaque_graph.is_empty() {
            return;
        }
        let mut pending = vec![name.to_string()];
        let mut seen = std::collections::HashSet::new();
        while let Some(changed) = pending.pop() {
            #[cfg(test)]
            OPAQUE_INVALIDATION_VISITS.with(|count| count.set(count.get() + 1));
            if !seen.insert(changed.clone()) {
                continue;
            }
            // Capture dependents before removing this node's outgoing edges;
            // cyclic inductive type/constructor graphs are ordinary here.
            if let Some(dependents) =
                remove_shared_entry(&mut self.checked_opaque_dependents, &changed)
            {
                pending.extend(dependents);
            }
            remove_shared_entry(&mut self.checked_opaque, &changed);
            if let Some(edges) = remove_shared_entry(&mut self.checked_opaque_graph, &changed) {
                for dependency in edges.iter() {
                    if !self
                        .checked_opaque_dependents
                        .get(dependency)
                        .is_some_and(|dependents| {
                            dependents.contains(&changed) || dependents.is_empty()
                        })
                    {
                        continue;
                    }
                    let reverse = Arc::make_mut(&mut self.checked_opaque_dependents);
                    if let Some(dependents) = reverse.get_mut(dependency) {
                        dependents.remove(&changed);
                        if dependents.is_empty() {
                            reverse.remove(dependency);
                        }
                    }
                }
            }
        }
    }

    fn invalidate_certificates(&mut self, name: &str) {
        self.invalidate_checked_opaque(name);
        if self
            .prelude_quotient
            .as_ref()
            .is_some_and(|c| c.dependencies.contains(name))
        {
            let certificate = self.prelude_quotient.take().expect("present certificate");
            // Revoking the family changes every primitive identity, including
            // cached opaque footprints that used only an unaffected member.
            for primitive in certificate.names() {
                self.invalidate_checked_opaque(primitive);
                self.log(primitive.to_string(), true);
            }
        }
        if self
            .observational
            .as_ref()
            .is_some_and(|c| c.dependencies.contains(name))
        {
            let certificate = self.observational.take().expect("present certificate");
            for constant in certificate.names() {
                self.invalidate_checked_opaque(constant);
                self.log(constant.to_string(), true);
            }
        }
        let revoked: Vec<String> = self
            .quotient_primitives
            .iter()
            .filter(|(_, certificate)| {
                certificate
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == name)
            })
            .map(|(primitive, _)| primitive.clone())
            .collect();
        for primitive in revoked {
            remove_shared_entry(&mut self.quotient_primitives, &primitive);
            self.log(primitive, true);
        }
    }

    /// Remove every trace of `name` from the global environment — its inductive header and any
    /// constructors registered against it, its parameter split, its structure metadata, its mutual
    /// block membership, and any definition or declaration of the same name.
    ///
    /// One name, one declaration. Re-declaring replaces; it never accumulates and never leaves a
    /// second reading of the same name behind in another map.
    fn displace(&mut self, name: &str) {
        if self.is_registered_global(name) {
            self.log(name.to_string(), true);
        }
        // A checked auxiliary describes its owner's current body/signature.
        // Source mutation has already passed the native dependency seal above.
        // User replacement disowns its name below, so only our descendants go.
        if let Some(children) = remove_shared_entry(&mut self.derived_aux, name) {
            for child in children {
                self.displace(&child);
            }
        }
        self.invalidate_certificates(name);
        // Constructors registered against this inductive, and their order entry.
        if let Some(ctors) = remove_shared_entry(&mut self.constructor_order, name) {
            for c in ctors {
                remove_shared_entry(&mut self.constructors, &c);
            }
        }
        // If the NAME ITSELF was a constructor of something, drop it from that inductive's order.
        if let Some((owner, _)) = remove_shared_entry(&mut self.constructors, name) {
            if self
                .constructor_order
                .get(&owner)
                .is_some_and(|order| order.iter().any(|c| c == name))
            {
                if let Some(order) = Arc::make_mut(&mut self.constructor_order).get_mut(&owner) {
                    order.retain(|c| c != name);
                }
            }
        }
        remove_shared_entry(&mut self.inductives, name);
        remove_shared_entry(&mut self.poly_types, name);
        remove_shared_entry(&mut self.binder_kinds, name);
        remove_shared_entry(&mut self.implicit_args, name);
        remove_shared_entry(&mut self.param_defaults, name);
        remove_shared_entry(&mut self.universe_polys, name);
        remove_shared_entry(&mut self.pending, name);
        remove_shared_entry(&mut self.inductive_params, name);
        remove_shared_entry(&mut self.inductive_inferred_params, name);
        remove_shared_entry(&mut self.definitions, name);
        // The edge list belongs to the BODY, so it goes when the body does. (`referenced` does
        // not: it is a monotone over-approximation, and shrinking it could hide a cycle.)
        remove_shared_entry(&mut self.definition_deps, name);
        remove_shared_entry(&mut self.definition_heights, name);
        remove_shared_entry(&mut self.declarations, name);
        remove_shared_entry(&mut self.structures, name);
        remove_shared_entry(&mut self.mutual_blocks, name);
        self.forget_mentions(name);
        // DISOWN: whatever is about to be registered under `name`, it is not the auxiliary we
        // generated. Keeping the claim would let the next re-derivation delete someone else's
        // constant — and "someone else" includes the prelude's hand-written `decEqNat`.
        if let Some(owner) = remove_shared_entry(&mut self.derived_aux_owner, name) {
            if self
                .derived_aux
                .get(&owner)
                .is_some_and(|list| list.iter().any(|n| n == name))
            {
                if let Some(list) = Arc::make_mut(&mut self.derived_aux).get_mut(&owner) {
                    list.retain(|n| n != name);
                }
            }
        }
    }

    pub(crate) fn equations_suspended(&self) -> bool {
        self.equations_suspended
    }

    /// Intermediate prelude recursors are checked for use during construction,
    /// but their equations can be checked once, after their final replacement.
    /// Restore the previous mode even if a caller catches a construction panic.
    pub(crate) fn with_equations_suspended<R>(
        &mut self,
        build: impl FnOnce(&mut Context) -> R,
    ) -> R {
        struct Restore<'a> {
            context: &'a mut Context,
            previous: bool,
        }
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.context.equations_suspended = self.previous;
            }
        }
        let previous = self.equations_suspended;
        self.equations_suspended = true;
        let restore = Restore {
            context: self,
            previous,
        };
        build(&mut *restore.context)
    }

    /// Include hidden pending and universe-polymorphic declarations when reserving
    /// generated names: absence from lookup never authorizes overwriting a user.
    pub(crate) fn constant_name_reserved(&self, name: &str) -> bool {
        self.get_global(name).is_some()
            || self.pending.contains_key(name)
            || self.universe_polys.contains_key(name)
    }

    pub(crate) fn record_derived_aux(&mut self, owner: &str, name: &str) {
        Arc::make_mut(&mut self.derived_aux_owner).insert(name.into(), owner.into());
        let children = Arc::make_mut(&mut self.derived_aux)
            .entry(owner.into())
            .or_default();
        if !children.iter().any(|child| child == name) {
            children.push(name.into());
        }
    }

    /// Is `name` a constant THIS context auto-derived (`{I}_rec`, `{I}_noConfusion`, `decEq{I}`, …)
    /// rather than one a human wrote?
    ///
    /// The distinction is what lets declaration-time synthesis be idempotent: re-deriving must
    /// REPLACE its own prior generation — an inductive is incomplete until its last constructor,
    /// so the auxiliaries derived after the first one describe a different type — while never
    /// touching a hand-written constant of the same name.
    pub fn is_derived_aux(&self, name: &str) -> bool {
        self.derived_aux_owner.contains_key(name)
    }

    /// The auxiliaries this context derived for a source declaration, in generation order.
    /// Inductives own recursors and related products; definitions own equation lemmas.
    pub fn derived_aux_names(&self, ind: &str) -> Option<&[String]> {
        self.derived_aux.get(ind).map(|v| v.as_slice())
    }

    /// Register a STRUCTURE (record) `{name} (params) := {name}_mk (fields)` — a
    /// one-constructor inductive with auto-derived projections and definitional eta.
    ///
    /// `params` are the leading type parameters `(A : Type 0)`, `(B : Type 0)`, …;
    /// `fields` are `(fst : A)`, `(snd : B)`, … where a field type may reference the
    /// params and any EARLIER field (by name). Registers:
    /// - the inductive `{name} : Π(params). Type 0`,
    /// - the constructor `{name}_mk : Π(params). Π(fields). {name} params`,
    /// - a projection `{name}_{fieldᵢ} : Π(params). Π(s:{name} params). Tᵢ` for each
    ///   field (its body a `match` on `s`), and
    /// - the [`StructInfo`] that gates the eta rule.
    ///
    /// The structure lives in `Type 0` (fields over `Type 0` carriers) — the common
    /// case for the algebraic hierarchy.
    /// The LEAST sort that can store `fields` — the sort `add_structure` infers when the caller
    /// does not name one, mirroring Lean's `structure Foo where …` with the result sort omitted.
    ///
    /// CIC's rule: an inductive's sort must dominate the sort of every field it STORES. Parameters
    /// do not constrain it — `List : Type u → Type u` takes a `Type (u+1)` argument and lands in
    /// `Type u`. `Prop`-valued fields do not raise it, by impredicativity.
    fn structure_min_sort(&self, params: &[(&str, Term)], fields: &[(&str, Term)]) -> Universe {
        let mut local = self.clone();
        for (p, t) in params {
            local = local.extend(p, t.clone());
        }
        // `acc` is the least DATA sort that dominates every non-propositional field. It is
        // consulted only when there IS such a field: a record that stores nothing but proofs is
        // a proposition, and seeding the accumulator at `Type 0` is what used to make that
        // unreachable — `Conj P Q` (the shape of `And`, `Iff`, and every bundled `Prop`-valued
        // class) landed in `Type 0`, where proof irrelevance stops applying and the
        // large-elimination gate stops firing.
        let mut acc = Universe::Type(0);
        let mut stores_data = false;
        for (fname, ftype) in fields {
            match crate::type_checker::infer_sort(&local, ftype) {
                // Impredicativity: a `Prop`/`SProp` field never RAISES the sort. It also never
                // forces it up out of `Prop` — that is the half that was missing.
                Ok(Universe::Prop) | Ok(Universe::SProp) => {}
                Ok(u) => {
                    stores_data = true;
                    acc = acc.max(&u);
                }
                // A field whose sort cannot be inferred is treated as DATA, never as a proof:
                // guessing `Prop` for something we could not read would silently grant proof
                // irrelevance and restrict elimination on a record that may store anything.
                Err(_) => stores_data = true,
            }
            // Later fields may mention earlier ones (`snd : B fst`), so each binding has to be in
            // scope before the next is inferred.
            local = local.extend(fname, ftype.clone());
        }
        // A FIELDLESS record is `Type 0`, matching Lean's `structure Foo where` — it is a unit
        // type, not a proposition, and `PUnit`/`Unit` must stay eliminable into `Type`.
        //
        // An all-propositional record is `Prop`, NOT `SProp`, even when every field is `SProp`:
        // `SProp` carries DEFINITIONAL proof irrelevance, which a record with fields would then
        // have to satisfy pointwise. `Prop` is Lean's answer and the conservative one.
        if fields.is_empty() || stores_data {
            acc
        } else {
            Universe::Prop
        }
    }

    /// Register a structure AT a DECLARED sort — Lean's `structure Foo (params) : <sort> where …`.
    ///
    /// The declared sort is CHECKED, not taken on faith: the generated constructor is run through
    /// `crate::type_checker::check_constructor_universes`, the same gate `add_mutual_inductives`
    /// uses, so a record may not claim a sort too small to hold what it stores. `Prop` is exempt,
    /// because `Prop` is impredicative and may store any sort at the cost of restricted elimination
    /// — exactly Lean's and Coq's rule, and what makes `Exists` (which stores an `α : Sort u`)
    /// legal in `Prop`.
    ///
    /// TRANSACTIONAL: the structure is built on a scratch context and committed only if the check
    /// passes, so a rejected declaration leaves nothing behind.
    ///
    /// Better than Lean in one respect: the error names the FIELD that forced the larger sort,
    /// where Lean reports only the two universes.
    pub fn add_structure_at(
        &mut self,
        name: &str,
        sort: Universe,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) -> crate::error::KernelResult<()> {
        let levels = Self::written_structure_levels(Some(&sort), params, fields);
        self.add_structure_poly_at(name, levels, sort, params, fields)
    }

    /// The level telescope a structure's AUTHOR wrote: the free level variables of its
    /// parameter types, its declared sort and its field types, in first-occurrence order — the
    /// same derivation [`Context::add_declaration`] and [`Context::add_inductive`] apply to what
    /// was written, and Lean's auto-bound universe rule. A structure written over `Sort u` is
    /// therefore polymorphic in `u`, and one written at a concrete level has no telescope.
    fn written_structure_levels(
        sort: Option<&Universe>,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) -> Vec<String> {
        let sort = sort.map(|u| Term::Sort(u.clone()));
        let mut levels: Vec<String> = Vec::new();
        for term in params
            .iter()
            .map(|(_, ty)| ty)
            .chain(sort.iter())
            .chain(fields.iter().map(|(_, ty)| ty))
        {
            for level in crate::term::free_level_vars(term) {
                if !levels.contains(&level) {
                    levels.push(level);
                }
            }
        }
        levels
    }

    /// [`Context::add_structure_at`] for a UNIVERSE-POLYMORPHIC structure — Lean's
    /// `structure Foo.{u…} (params) : <sort> where …`, where `sort`, the parameter telescope and
    /// the field types may all mention the level parameters in `levels`.
    ///
    /// The level telescope is the whole content: the arity is registered through
    /// [`Context::add_universe_poly_inductive`], so `Foo.{ℓ…}` resolves and ONE declaration serves
    /// every level. Everything else — the checked sort, the transactional scratch context, the
    /// constructor, the projections, the parent edges — is the monomorphic path verbatim, which is
    /// what makes an empty `levels` list exactly the old declaration rather than a near-copy.
    pub fn add_structure_poly_at(
        &mut self,
        name: &str,
        levels: Vec<String>,
        sort: Universe,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) -> crate::error::KernelResult<()> {
        if !crate::annotation::within_erased_call()
            && params
                .iter()
                .chain(fields)
                .any(|(_, ty)| ty.has_annotations())
        {
            let params: Vec<(&str, Term)> = params
                .iter()
                .map(|(name, ty)| (*name, ty.strip_annotations()))
                .collect();
            let fields: Vec<(&str, Term)> = fields
                .iter()
                .map(|(name, ty)| (*name, ty.strip_annotations()))
                .collect();
            return self.add_structure_poly_at(name, levels, sort, &params, &fields);
        }
        self.check_native_dependency_mutation(name)?;
        self.check_native_dependency_mutation(&format!("{name}_mk"))?;
        for (field, _) in fields {
            self.check_native_dependency_mutation(&format!("{name}_{field}"))?;
        }
        let result_sort = Term::Sort(sort.clone());
        Self::check_level_telescope(
            "structure",
            name,
            &levels,
            std::iter::once(&result_sort)
                .chain(params.iter().map(|(_, ty)| ty))
                .chain(fields.iter().map(|(_, ty)| ty)),
        )?;
        let mut scratch = self.clone();
        scratch.build_structure(name, &levels, sort.clone(), params, fields);

        // A structure something already depends on keeps its meaning: rebuilt identically it is
        // left exactly as it is, and rebuilt any other way it is refused.
        let generated: Vec<String> = std::iter::once(name.to_string())
            .chain(std::iter::once(format!("{name}_mk")))
            .chain(fields.iter().map(|(field, _)| format!("{name}_{field}")))
            .collect();
        let identical = generated.iter().all(|n| {
            self.get_global(n).is_some()
                && same_meaning_opt(self.get_global(n), scratch.get_global(n))
                && same_meaning_opt(self.get_definition_body(n), scratch.get_definition_body(n))
                && match (self.poly_declaration(n), scratch.poly_declaration(n)) {
                    (Some((ours, our_ty)), Some((theirs, their_ty))) => {
                        ours == theirs && same_meaning(our_ty, their_ty)
                    }
                    (None, None) => true,
                    _ => false,
                }
        });
        let mut relied_on = false;
        for generated_name in &generated {
            relied_on |=
                self.admit_meaning(generated_name, Self::change_if(identical))? == Admission::Keep;
        }
        if relied_on {
            return Ok(());
        }

        let mk = format!("{name}_mk");
        let ctor_ty = match scratch.get_global(&mk) {
            Some(t) => t.clone(),
            None => {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "structure '{name}' generated no constructor"
                )))
            }
        };
        crate::type_checker::infer_sort(&scratch, &ctor_ty)?;
        let dependencies =
            crate::positivity::native_positivity_dependencies(&scratch, &[name], &mk, &ctor_ty)?;
        scratch.check_native_dependencies(&dependencies)?;
        if let Err(e) =
            crate::type_checker::check_constructor_universes(&scratch, name, &mk, &ctor_ty)
        {
            let culprit = {
                let min = self.structure_min_sort(params, fields);
                let mut local = self.clone();
                for (p, t) in params {
                    local = local.extend(p, t.clone());
                }
                let mut which = None;
                for (fname, ftype) in fields {
                    if let Ok(u) = crate::type_checker::infer_sort(&local, ftype) {
                        if !matches!(u, Universe::Prop | Universe::SProp) && !u.is_subtype_of(&sort)
                        {
                            which = Some(format!("field `{fname}` lives in {u}"));
                            break;
                        }
                    }
                    local = local.extend(fname, ftype.clone());
                }
                which.unwrap_or_else(|| format!("its fields need at least {min}"))
            };
            return Err(crate::error::KernelError::CertificationError(format!(
                "structure '{name}' cannot live in {sort}: {culprit} ({e:?})"
            )));
        }

        scratch.seal_native_dependencies(dependencies);
        *self = scratch;
        Ok(())
    }

    /// Register a structure, INFERRING the least sort that can hold its fields.
    ///
    /// Equivalent to `add_structure_at(name, self.structure_min_sort(params, fields), …)`.
    /// Invalid declarations are recorded in `refused_declarations` and leave the context intact. Use
    /// [`Context::add_structure_at`] to declare a sort explicitly (a `Prop` record, or one placed
    /// higher than it needs to be).
    pub fn add_structure(&mut self, name: &str, params: &[(&str, Term)], fields: &[(&str, Term)]) {
        let levels = Self::written_structure_levels(None, params, fields);
        let sort = self.structure_min_sort(params, fields);
        if let Err(error) = self.add_structure_poly_at(name, levels, sort, params, fields) {
            Arc::make_mut(&mut self.refused).push((name.to_string(), error.to_string()));
        }
    }

    /// Register a UNIVERSE-POLYMORPHIC structure, INFERRING the least sort that can hold its
    /// fields — Lean's `class Mul.{u} (α : Sort u) where mul : α → α → α`, and the shape the whole
    /// algebraic hierarchy is built from.
    ///
    /// This is the wall that decided the Mathlib question. `add_structure` computes its result
    /// sort from a telescope of CONCRETE parameter types, so a class declared with `α : Type 0`
    /// refuses a `Type 0` carrier — `Type 0 : Type 1` — and one declaration could serve exactly
    /// one level. Monomorphising at `Type 0` is the tempting wrong move and it is unsound rather
    /// than merely lossy: keys are flat `String`s, so two instantiations collapse into one
    /// declaration, and because LOGOS is CUMULATIVE the resulting terms very often still
    /// type-check.
    ///
    /// The result sort is COMPUTED over the level telescope, not assumed, exactly as Lean's
    /// `mkResultUniverse` (`Lean/Elab/Inductive.lean:484`) computes it: the `max` of the stored
    /// fields' sorts, raised to at least `Type 0` when anything is stored, and `Prop` when the
    /// record stores nothing but proofs. That inferred sort dominates every field by construction,
    /// and registration still validates the declaration's types, positivity, and universes.
    ///
    /// Invalid types, negative recursion, and inconsistent universes are refused transactionally.
    /// This returns the error directly; `add_structure` records it in the refusal ledger.
    pub fn add_structure_poly(
        &mut self,
        name: &str,
        levels: Vec<String>,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) -> crate::error::KernelResult<()> {
        let sort = self.structure_min_sort(params, fields);
        self.add_structure_poly_at(name, levels, sort, params, fields)
    }

    fn build_structure(
        &mut self,
        name: &str,
        levels: &[String],
        sort: Universe,
        params: &[(&str, Term)],
        fields: &[(&str, Term)],
    ) {
        let g = |s: &str| Term::Global(s.to_string());
        let var = |s: &str| Term::Var(s.to_string());
        // Wrap `body` in a Π / λ telescope.
        let pis = |tele: &[(&str, Term)], body: Term| {
            tele.iter().rev().fold(body, |acc, (p, t)| Term::Pi {
                param: p.to_string(),
                param_type: Box::new(t.clone()),
                body_type: Box::new(acc),
            })
        };
        let lams = |tele: &[(&str, Term)], body: Term| {
            tele.iter().rev().fold(body, |acc, (p, t)| Term::Lambda {
                param: p.to_string(),
                param_type: Box::new(t.clone()),
                body: Box::new(acc),
            })
        };
        // `{name} A B …` — the structure applied to its parameter variables.
        let structure_head = if levels.is_empty() {
            g(name)
        } else {
            Term::Const {
                name: name.to_string(),
                levels: levels
                    .iter()
                    .map(|level| Universe::Var(level.clone()))
                    .collect(),
            }
        };
        let s_applied = params.iter().fold(structure_head, |acc, (p, _)| {
            Term::App(Box::new(acc), Box::new(var(p)))
        });

        // 1. The inductive, at the sort the caller settled on — and, when the declaration named
        // level parameters, WITH its level telescope, so `{name}.{ℓ…}` resolves and the one
        // declaration serves every level. An empty telescope is the monomorphic door verbatim:
        // `add_universe_poly_inductive` would otherwise record an empty parameter list, which is
        // a DIFFERENT claim from "this declaration has no level telescope".
        let ind_type = pis(params, Term::Sort(sort));
        if levels.is_empty() {
            self.add_indexed_inductive(name, ind_type, params.len());
        } else {
            self.add_universe_poly_inductive(name, levels.to_vec(), ind_type, params.len());
        }

        // 2. The constructor.
        let mk = format!("{name}_mk");
        let ctor_type = pis(params, pis(fields, s_applied.clone()));
        self.add_constructor(&mk, name, ctor_type);

        // 3. The projections.
        let mut proj_names = Vec::new();
        for (i, (fname, ftype)) in fields.iter().enumerate() {
            let proj = format!("{name}_{fname}");
            proj_names.push(proj.clone());

            // Rewrite earlier field references `f_j` (j < i) to `proj_j params disc`,
            // for a chosen discriminant term.
            let field_of = |disc: &Term, ty: &Term| -> Term {
                let mut out = ty.clone();
                for (fj, _) in fields.iter().take(i) {
                    let proj_j_applied = params
                        .iter()
                        .fold(g(&format!("{name}_{fj}")), |acc, (p, _)| {
                            Term::App(Box::new(acc), Box::new(var(p)))
                        });
                    let proj_j_applied =
                        Term::App(Box::new(proj_j_applied), Box::new(disc.clone()));
                    out = crate::type_checker::substitute(&out, fj, &proj_j_applied);
                }
                out
            };

            // Projection type: Π(params). Π(s : {name} params). Tᵢ[fⱼ := projⱼ params s].
            let ret_ty = field_of(&var("s"), ftype);
            let proj_type = pis(
                params,
                Term::Pi {
                    param: "s".to_string(),
                    param_type: Box::new(s_applied.clone()),
                    body_type: Box::new(ret_ty),
                },
            );

            // Body: λparams. λ(s). match s return (λ(s✧). Tᵢ[fⱼ := projⱼ params s✧])
            //                       with | mk => λ(fields). fieldᵢ
            let motive = Term::Lambda {
                param: "s✧".to_string(),
                param_type: Box::new(s_applied.clone()),
                body: Box::new(field_of(&var("s✧"), ftype)),
            };
            let case = lams(fields, var(fname));
            let match_term = Term::Match {
                discriminant: Box::new(var("s")),
                motive: Box::new(motive),
                cases: vec![case],
            };
            let body = lams(
                params,
                Term::Lambda {
                    param: "s".to_string(),
                    param_type: Box::new(s_applied.clone()),
                    body: Box::new(match_term),
                },
            );
            self.add_definition(proj, proj_type, body);
        }

        let params_owned: Vec<(String, Term)> = params
            .iter()
            .map(|(p, t)| (p.to_string(), t.clone()))
            .collect();
        let fields_owned: Vec<(String, Term)> = fields
            .iter()
            .map(|(f, t)| (f.to_string(), t.clone()))
            .collect();
        let parents = self
            .immediate_parent(name, &params_owned, &fields_owned)
            .into_iter()
            .collect();
        self.insert_struct_info(
            name,
            StructInfo {
                mk,
                num_params: params.len(),
                projections: proj_names,
                params: params_owned,
                fields: fields_owned,
                parents,
                out_params: infer_out_params(params, fields),
            },
        );
        self.emit_parent_edges(name, params);
    }

    /// The structure `child` EXTENDS, recovered from its flattened field list.
    ///
    /// Lean's `structure Child extends Parent` stores the parent's fields FIRST, in parent
    /// order, then the child's own — so a flattened declaration carries its inheritance in its
    /// shape, and that is the only place it can be read from: `add_structure` takes a flat
    /// field slice with no parent list. A previously declared structure is the parent when
    ///
    /// * its parameter telescope is IDENTICAL (names and types), so one forget map can be
    ///   written over the shared parameters, and
    /// * its whole field list is a STRICT prefix of the child's (names and types).
    ///
    /// Each condition rules out a near-miss that is not inheritance: a reordered field list, a
    /// same-named field at another type, a different parameter family, and a twin with exactly
    /// the same fields (which extends nothing). A FIELDLESS parent is excluded because every
    /// structure trivially extends the empty record, so it is evidence of nothing.
    ///
    /// Only the IMMEDIATE parent — the longest matching prefix — is returned, mirroring Lean:
    /// a grandparent is reached by composing edges, not by a second direct one. Ties on length
    /// cannot occur (two different field lists of one length cannot both prefix-match), and the
    /// name order makes the choice deterministic regardless of the map's iteration order.
    fn immediate_parent(
        &self,
        child: &str,
        params: &[(String, Term)],
        fields: &[(String, Term)],
    ) -> Option<String> {
        let mut best: Option<(usize, &String)> = None;
        for (pname, info) in self.structures.iter() {
            if pname == child || info.fields.is_empty() || info.fields.len() >= fields.len() {
                continue;
            }
            if info.params != params || info.fields[..] != fields[..info.fields.len()] {
                continue;
            }
            let len = info.fields.len();
            if best.is_none_or(|(blen, bname)| (len, pname) > (blen, bname)) {
                best = Some((len, pname));
            }
        }
        best.map(|(_, n)| n.clone())
    }

    /// Emit the forget map for each parent recovered by [`Context::immediate_parent`]:
    /// `{Child}_to{Parent} : Π(params). Child params → Parent params`, whose body rebuilds the
    /// parent from the child's leading projections, plus the instance entry that lets a
    /// `Parent τ` obligation be discharged from a `Child τ` in scope.
    ///
    /// It is a DEFINITION, never an axiom, so `(c.toParent).f ≡ c.f` holds by computation —
    /// an axiomatised edge would break every `rfl` that reads a parent field through a child.
    fn emit_parent_edges(&mut self, child: &str, params: &[(&str, Term)]) {
        let Some(info) = self.structures.get(child) else {
            return;
        };
        let parents = info.parents.clone();
        for parent in parents {
            let edge = format!("{child}_to{parent}");
            let Some(pinfo) = self.structures.get(&parent) else {
                continue;
            };
            let (pmk, pfields) = (pinfo.mk.clone(), pinfo.fields.len());
            let var = |s: &str| Term::Var(s.to_string());
            let applied = |head: &str| {
                params
                    .iter()
                    .fold(Term::Global(head.to_string()), |acc, (p, _)| {
                        Term::App(Box::new(acc), Box::new(var(p)))
                    })
            };
            let pis = |body: Term| {
                params.iter().rev().fold(body, |acc, (p, t)| Term::Pi {
                    param: p.to_string(),
                    param_type: Box::new(t.clone()),
                    body_type: Box::new(acc),
                })
            };
            let lams = |body: Term| {
                params.iter().rev().fold(body, |acc, (p, t)| Term::Lambda {
                    param: p.to_string(),
                    param_type: Box::new(t.clone()),
                    body: Box::new(acc),
                })
            };
            // The codomain never mentions the receiver, so the edge's own binder is anonymous.
            let ty = pis(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(applied(child)),
                body_type: Box::new(applied(&parent)),
            });
            // `Parent_mk params (Child_f₁ params s) … (Child_fₙ params s)` — the child's
            // leading projections ARE the parent's fields, in order, by the prefix condition.
            let mut rebuilt = applied(&pmk);
            for i in 0..pfields {
                let proj = self.structures[child].projections[i].clone();
                let mut value = applied(&proj);
                value = Term::App(Box::new(value), Box::new(var("s")));
                rebuilt = Term::App(Box::new(rebuilt), Box::new(value));
            }
            let body = lams(Term::Lambda {
                param: "s".to_string(),
                param_type: Box::new(applied(child)),
                body: Box::new(rebuilt),
            });
            self.add_definition(edge.clone(), ty.clone(), body);
            self.add_instance(ty, Term::Global(edge));
        }
    }

    /// Register an INDEXED inductive: `sort` is its full arity `Π(params). Π(indices).
    /// Sort`, of which the first `num_params` leading arguments are uniform parameters and
    /// the rest are indices that vary per constructor (e.g. `Eq` with `num_params == 2`).
    pub fn add_indexed_inductive(&mut self, name: &str, sort: Term, num_params: usize) {
        if self.refuse_native_dependency_mutation(name) {
            return;
        }
        let sort = self.stored_term(sort);
        let unchanged = self
            .inductives
            .get(name)
            .is_some_and(|current| same_meaning(current, &sort))
            && self.inductive_num_params(name) == num_params;
        match self.admit_meaning(name, Self::change_if(unchanged)) {
            Err(refusal) => return self.record_refusal(name, refusal),
            Ok(Admission::Keep) => return,
            Ok(Admission::Register) => {}
        }
        self.add_inductive(name, sort);
        self.set_inductive_params(name, num_params);
    }

    /// Record how many leading arguments of `name`'s arity are uniform parameters.
    pub fn set_inductive_params(&mut self, name: &str, num_params: usize) {
        if self.refuse_native_dependency_mutation(name) {
            return;
        }
        // The split decides what a `match` binds and what ι skips, so a split something was
        // checked against is part of the inductive's meaning.
        match self.admit_meaning(
            name,
            Self::change_if(self.inductive_num_params(name) == num_params),
        ) {
            Err(refusal) => return self.record_refusal(name, refusal),
            Ok(Admission::Keep) => return,
            Ok(Admission::Register) => {}
        }
        self.invalidate_certificates(name);
        self.record(|c| &mut c.inductive_params, name.to_string(), num_params);
    }

    /// The full arity of an inductive — the number of leading `Π`s in its sort (`Nat` →
    /// 0, `TList : Type → Type` → 1, `Eq : Type → A → A → Prop` → 3). `0` for an unknown
    /// name.
    pub fn inductive_arity(&self, name: &str) -> usize {
        self.inductives
            .get(name)
            .map(count_leading_pis)
            .unwrap_or(0)
    }

    /// The EXPLICITLY declared parameter count for `name`, or `None` if the inductive was
    /// registered without one. Reduction uses this to skip exactly the parameters of an
    /// indexed constructor (`refl A x` → 2), falling back to a syntactic heuristic for the
    /// legacy inductives that never declared a split — so their ι-reduction is untouched.
    pub fn inductive_declared_params(&self, name: &str) -> Option<usize> {
        self.inductive_params.get(name).copied()
    }

    /// How many leading arguments of `name`'s arity are uniform PARAMETERS.
    ///
    /// Three sources, in strict order:
    ///
    /// 1. the count the declaration stated ([`Context::add_indexed_inductive`]) — always wins, so
    ///    a translator carrying Lean's own split across is never second-guessed;
    /// 2. otherwise the count INFERRED from the registered constructors (see
    ///    `Context::infer_num_params`);
    /// 3. otherwise the full arity, for a header with no constructors yet — a declaration in
    ///    progress behaves exactly as it always did.
    ///
    /// O(1): the inference is computed once per `add_constructor`, never per read. This is read on
    /// ι-reduction's argument-skipping path, so a scan here would be a scan inside reduction.
    pub fn inductive_num_params(&self, name: &str) -> usize {
        if let Some(n) = self.inductive_params.get(name) {
            return *n;
        }
        if let Some(n) = self.inductive_inferred_params.get(name) {
            return *n;
        }
        self.inductive_arity(name)
    }

    /// INFER the params/indices split of `name` from its registered constructors — CIC's own
    /// rule, which is the rule Lean applies to `inductive … where`:
    ///
    /// > arity position `i` is a uniform PARAMETER iff every constructor passes, at position `i`,
    /// > exactly that constructor's own `i`-th binder — unchanged — both in its RESULT type and at
    /// > every RECURSIVE occurrence of the inductive inside its argument types.
    ///
    /// The parameters are the longest such PREFIX; everything after it is an index.
    ///
    /// The recursive-occurrence half is load-bearing, not belt-and-braces. `Acc`'s introduction
    /// rule `Acc_intro : Π(A)(R)(x:A). (Π(y:A). R y x → Acc A R y) → Acc A R x` passes its own `x`
    /// at position 2 in its RESULT, so the result alone reports 3 parameters — but the recursive
    /// occurrence passes `y`, not `x`. Reading the result only would make well-founded recursion's
    /// eliminator abstract nothing, and every `Acc`-based definition would die.
    ///
    /// A constructor whose result is not headed by the inductive at all contributes 0: nothing
    /// about it is uniform, and guessing higher is how an unsound split gets in.
    fn infer_num_params(&self, name: &str) -> usize {
        let arity = self.inductive_arity(name);
        if arity == 0 {
            return 0;
        }
        let ctors = self.get_constructors(name);
        // A header with no constructors yet: keep the pre-inference answer so a declaration
        // mid-flight is never re-interpreted underneath its own caller.
        if ctors.is_empty() {
            return arity;
        }
        let mut best = arity;
        for (_, cty) in &ctors {
            best = best.min(ctor_param_prefix(name, cty, arity));
            if best == 0 {
                break;
            }
        }
        best
    }

    /// How many trailing arguments of `name`'s arity are INDICES (arity − parameters).
    pub fn inductive_num_indices(&self, name: &str) -> usize {
        self.inductive_arity(name)
            .saturating_sub(self.inductive_num_params(name))
    }

    /// Register a constructor for an inductive type.
    ///
    /// The `ty` is the full type of the constructor
    /// (e.g., `Nat` for Zero, `Nat -> Nat` for Succ).
    ///
    /// Constructors are tracked in registration order for match expressions.
    pub fn add_constructor(&mut self, name: &str, inductive: &str, ty: Term) {
        if self.refuse_native_dependency_mutation(name)
            || self.refuse_native_dependency_mutation(inductive)
        {
            return;
        }
        let ty = self.stored_term(ty);
        match self.admit_constructor(name, inductive, &ty) {
            Err(refusal) => return self.record_refusal(name, refusal),
            Ok(Admission::Keep) => return,
            Ok(Admission::Register) => {}
        }
        if let Some((old_owner, _)) = self.constructors.get(name) {
            let old_owner = old_owner.clone();
            self.invalidate_checked_opaque(&old_owner);
        }
        // ONE NAME, ONE DECLARATION. Whatever `name` meant before — a definition, an axiom, a
        // constructor of another inductive — goes, or `get_global` would type the name as this
        // constructor while δ still unfolded the old body.
        if self.is_registered_global(name) {
            self.displace(name);
        }
        self.invalidate_certificates(name);
        self.invalidate_certificates(inductive);
        self.record_mentions(name, &ty);
        self.record(
            |c| &mut c.constructors,
            name.to_string(),
            (inductive.to_string(), ty),
        );

        // Track constructor order for this inductive
        Arc::make_mut(&mut self.constructor_order)
            .entry(inductive.to_string())
            .or_default()
            .push(name.to_string());

        // Re-infer the params/indices split now that one more constructor constrains it. Done
        // HERE, at registration, so `inductive_num_params` stays an O(1) map read for the
        // ι-reduction and match-typing paths that call it per step.
        //
        // An EXPLICIT declaration is never overwritten: `add_indexed_inductive` records the
        // caller's own split and that is the authority.
        if !self.inductive_params.contains_key(inductive) {
            let inferred = self.infer_num_params(inductive);
            self.record(
                |c| &mut c.inductive_inferred_params,
                inductive.to_string(),
                inferred,
            );
        }

        // DECLARATION-TIME SYNTHESIS. Lean generates `{I}.rec`, `{I}.noConfusionType`,
        // `{I}.noConfusion`, `{C}.inj`, `{I}.below`, `{I}.brecOn` and `{I}.decEq` for EVERY
        // inductive, at the declaration. Every `injection`, every `cases h` on an equation, every
        // `simp [injEq]` in a Lean development names one of them. Requiring the caller to ask
        // meant they did not exist, and axiomatising them would have turned each of those proofs
        // into an assumption.
        //
        // Wired on the PLAIN door as well as the checked one, deliberately: `add_constructor` is
        // the door `add_mutual_inductives`, `build_structure` and most translators use, so hanging
        // it off `add_constructor_checked` alone would have synthesised for a minority of
        // declarations. This is where the whole CLASS is covered.
        //
        // It fires once per CONSTRUCTOR because there is no "declaration finished" call: the
        // auxiliaries derived after the first constructor describe an inductive that is not
        // finished yet, so each run REPLACES its own prior generation (`derived_aux` provenance)
        // and the last one is the one that stands.
        if self.auto_aux {
            self.regenerate_auxiliaries(inductive);
        } else if !self.dirty_aux.iter().any(|n| n == inductive) {
            Arc::make_mut(&mut self.dirty_aux).push(inductive.to_string());
        }
        self.retry_pending(name);
    }

    /// Turn EAGER declaration-time auxiliary synthesis on or off — Lean's behaviour, made explicit.
    ///
    /// Turning it on also [`seal`](Context::seal)s whatever is already owed, so enabling it partway
    /// through a load does not silently lose the declarations that came first.
    ///
    /// See the `auto_aux` field's own note for what it costs and why the default is off.
    pub fn set_auto_auxiliaries(&mut self, on: bool) {
        self.auto_aux = on;
        if on {
            self.seal();
        }
    }

    /// Whether eager declaration-time auxiliary synthesis is on.
    pub fn auto_auxiliaries(&self) -> bool {
        self.auto_aux
    }

    /// Synthesise the auxiliaries of every inductive declared since the last seal, in declaration
    /// order — ONE pass per inductive rather than one per constructor.
    ///
    /// This is the door a library loader uses: register everything, then seal. Idempotent, and a
    /// no-op when nothing is owed.
    pub fn seal(&mut self) {
        let owed = std::mem::take(Arc::make_mut(&mut self.dirty_aux));
        for ind in owed {
            self.regenerate_auxiliaries(&ind);
        }
    }

    /// The inductives whose auxiliaries are still OWED — declared, not yet synthesised.
    ///
    /// An honest channel rather than silence: a caller can see exactly which declarations would
    /// gain constants from a [`seal`](Context::seal). Lean does not tell you, because there
    /// synthesis is unconditional and its cost is invisible.
    pub fn unsealed_inductives(&self) -> &[String] {
        &self.dirty_aux
    }

    /// Add a declaration (typed assumption/hypothesis).
    ///
    /// Used for proof certification where hypotheses are assumed.
    /// Example: h1 : P -> Q
    ///
    /// The type must be a type; a declaration whose type is not one is refused and recorded in
    /// [`Context::refused_declarations`]. A type that names a constant declared later is owed, as a
    /// definition's is: the assumption waits as a pending obligation, invisible to lookup, and is
    /// checked when that name arrives.
    pub fn add_declaration(&mut self, name: &str, ty: Term) {
        if self.refuse_native_dependency_mutation(name) {
            return;
        }
        let ty = self.stored_term(ty);
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        let blocker = match crate::type_checker::infer_sort(self, &ty) {
            Ok(_) => None,
            Err(error) => match self.first_unresolvable_global(&ty) {
                Some(missing) => Some(missing),
                None => {
                    #[cfg(feature = "shadow")]
                    crate::dag::mirror::shadow(self, &shadow_entry, Some(&error), || {
                        vec![crate::dag::logos_env::candidate_axiom(
                            self, name, None, &ty,
                        )]
                    });
                    return self.record_refusal(name, error);
                }
            },
        };
        let unchanged = self
            .declarations
            .get(name)
            .is_some_and(|current| same_meaning(current, &ty))
            && !self.definitions.contains_key(name);
        match self.admit_meaning(name, Self::change_if(unchanged)) {
            Err(refusal) => return self.record_refusal(name, refusal),
            Ok(Admission::Keep) => return,
            Ok(Admission::Register) => {}
        }
        self.displace(name);
        self.record_mentions(name, &ty);
        self.record(|c| &mut c.declarations, name.to_string(), ty);
        match blocker {
            Some(missing) => self.defer(name, missing),
            None => self.retry_pending(name),
        }
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, None, Vec::new);
    }

    /// Register an opaque assumption with an explicit universe telescope.
    /// Its type is checked before either the declaration or its metadata is installed.
    pub fn add_universe_poly_declaration(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_axiom(
            self,
            name,
            Some(&params),
            &ty,
        )];
        let result = self.add_universe_poly_declaration_checked(name, params, ty);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    fn add_universe_poly_declaration_checked(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
    ) -> crate::error::KernelResult<()> {
        self.check_native_dependency_mutation(name)?;
        let ty = self.stored_term(ty);
        if self.get_global(name).is_some() || self.pending.contains_key(name) {
            return Err(crate::error::KernelError::CertificationError(format!(
                "checked assumption cannot replace existing declaration {name}"
            )));
        }
        Self::check_level_telescope("assumption", name, &params, [&ty])?;
        crate::type_checker::infer_sort(self, &ty)?;
        self.displace(name);
        self.record_mentions(name, &ty);
        self.record(|c| &mut c.declarations, name.to_string(), ty.clone());
        self.record(|c| &mut c.poly_types, name.to_string(), (params, ty));
        self.retry_pending(name);
        Ok(())
    }

    /// Check an opaque declaration without exposing its body to conversion.
    ///
    /// Proof dependency evidence is derived internally from the original closed
    /// type/body after both checkers agree. Raw headers remain assumptions.
    /// This gateway is atomic on failure and never creates pending obligations.
    pub fn declare_opaque_checked(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
        body: Term,
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_definition(
            self,
            name,
            Some(&params),
            &ty,
            &body,
        )];
        let result = self.declare_certified_body(name, params, ty, body, false);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    /// Check a proposition and its proof in both kernels, then retain its body
    /// for kernel delta reduction. Lean theorems use this rule; opaque values do not.
    pub fn declare_theorem_checked(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
        body: Term,
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_definition(
            self,
            name,
            Some(&params),
            &ty,
            &body,
        )];
        let result = self.declare_certified_body(name, params, ty, body, true);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    /// Whether `name` already holds exactly this checked declaration: published through the
    /// checked door at the same universe parameters and type, with — for a transparent theorem —
    /// the same body, and — for an opaque one, whose body is sealed — the same dependency evidence.
    ///
    /// The checked door refuses every existing name, identical or not: it is append-only, as
    /// Lean's environment is. A document read a second time meets its own declarations there, and
    /// asks this first, so its second reading registers nothing and refuses nothing.
    pub fn holds_checked_declaration(
        &self,
        name: &str,
        params: &[String],
        ty: &Term,
        body: &Term,
        transparent_theorem: bool,
    ) -> bool {
        let Some(evidence) = self.checked_opaque.get(name) else {
            return false;
        };
        if transparent_theorem {
            return self.universe_polys.get(name).is_some_and(|payload| {
                payload.0 == params
                    && same_meaning(&payload.1, ty)
                    && same_meaning(&payload.2, body)
            });
        }
        self.poly_types
            .get(name)
            .is_some_and(|(declared, declared_ty)| {
                declared == params && same_meaning(declared_ty, ty)
            })
            && crate::prelude::checked_opaque_dependencies(self, ty, body)
                .checked
                .dependencies
                == evidence.dependencies
    }

    /// Whether `name` already holds exactly this ASSUMPTION — a bodyless declaration at the same
    /// universe parameters and type, published through the assumption door. The door refuses every
    /// existing name; this is the question a document read again asks first (see
    /// [`Context::holds_checked_declaration`]).
    pub fn holds_assumption(&self, name: &str, params: &[String], ty: &Term) -> bool {
        !self.checked_opaque.contains_key(name)
            && !self.definitions.contains_key(name)
            && self
                .declarations
                .get(name)
                .is_some_and(|declared| same_meaning(declared, ty))
            && self
                .poly_types
                .get(name)
                .is_some_and(|(declared, declared_ty)| {
                    declared == params && same_meaning(declared_ty, ty)
                })
    }

    fn declare_certified_body(
        &mut self,
        name: &str,
        params: Vec<String>,
        ty: Term,
        body: Term,
        transparent_theorem: bool,
    ) -> crate::error::KernelResult<()> {
        use crate::error::KernelError;
        let (ty, body) = (self.stored_term(ty), self.stored_term(body));
        let kind = if transparent_theorem {
            "theorem"
        } else {
            "opaque declaration"
        };
        let refuse = |reason: &str| {
            KernelError::CertificationError(format!("checked {kind} '{name}': {reason}"))
        };
        self.check_native_dependency_mutation(name)?;
        if self.get_global(name).is_some()
            || self.poly_declaration(name).is_some()
            || self.pending.contains_key(name)
        {
            return Err(refuse("cannot replace an existing declaration"));
        }
        if !self.simp_rules().is_empty() {
            return Err(refuse(
                "active simp rules are outside opaque proof certification",
            ));
        }
        Self::check_level_telescope(kind, name, &params, [&ty, &body])?;
        if transparent_theorem && self.delta_cycle_through(name, &body).is_some() {
            return Err(refuse("transparent proof body would create a delta cycle"));
        }
        let main_type = crate::type_checker::infer_type_as_type(self, &ty)?;
        if transparent_theorem {
            let sort = crate::normalize_checked(self, &main_type)
                .map_err(|_| refuse("theorem sort normalization did not complete"))?;
            if !matches!(sort, Term::Sort(ref u) if u.equiv(&Universe::Prop)) {
                return Err(refuse("theorem type must be a proposition"));
            }
        }
        let independent_type =
            crate::recheck(self, &ty).map_err(|error| KernelError::IndependentRecheck {
                subject: format!("checked opaque declaration '{name}' independent type"),
                error,
            })?;
        let main_body = crate::infer_type(self, &body)?;
        let independent_body =
            crate::recheck(self, &body).map_err(|error| KernelError::IndependentRecheck {
                subject: format!("checked opaque declaration '{name}' independent body"),
                error,
            })?;
        let exact =
            |a: &Term, b: &Term| crate::is_subtype(self, a, b) && crate::is_subtype(self, b, a);
        if !exact(&main_type, &independent_type)
            || !exact(&main_body, &ty)
            || !exact(&independent_body, &ty)
        {
            return Err(refuse(
                "main and independent types must agree with the declared type",
            ));
        }
        // Both checked entry points enforce their input depth limits before
        // allocating this additional closedness/provenance representation.
        let mut store = crate::term_store::TermStore::new();
        for term in [&ty, &body] {
            let id = store.intern(term);
            if !store.free_names_are_empty(id)
                || store
                    .shapes(id)
                    .contains(crate::term_store::NodeShape::Hole)
            {
                return Err(refuse("type and body must be closed and contain no holes"));
            }
        }
        let trace = crate::prelude::checked_opaque_dependencies(self, &ty, &body);
        if !trace.checked.usage.unresolved.is_empty() {
            return Err(refuse("unresolved certification dependency"));
        }
        // Every fallible validation precedes publication. Both reference forms
        // use the same checked type/body and exact explicit universe telescope.
        self.displace(name);
        if transparent_theorem {
            let payload = Arc::new((params, ty, body));
            self.record_definition_payload(name.to_string(), Arc::clone(&payload));
            self.record(|c| &mut c.universe_polys, name.to_string(), payload);
        } else {
            // Opaque declarations retain only headers and dependency evidence.
            self.record_mentions(name, &ty);
            self.record(|c| &mut c.declarations, name.to_string(), ty.clone());
            self.record(|c| &mut c.poly_types, name.to_string(), (params, ty));
        }
        for (dependency, edges) in trace.graph {
            self.record_checked_opaque_edges(dependency, edges);
        }
        self.record_checked_opaque_edges(name.to_string(), trace.checked.dependencies.clone());
        Arc::make_mut(&mut self.checked_opaque).insert(name.to_string(), trace.checked);
        self.retry_pending(name);
        Ok(())
    }

    /// Register a definition: name : type := body
    ///
    /// Definitions are transparent and unfold during normalization (delta reduction).
    /// This distinguishes them from declarations (axioms) which have no body.
    ///
    /// The BODY IS CHECKED against the declared type — see [`Context::declare`], which this is the
    /// `()`-returning face of. A body that definitely does not have the declared type is REFUSED
    /// and nothing is registered, so `evil : False := Zero` can no longer put a closed term of type
    /// `False` in reach of the standard library. Use `declare` where the refusal can be reported.
    pub fn add_definition(&mut self, name: String, ty: Term, body: Term) {
        if let Err(e) = self.declare(&name, ty, body) {
            // A refusal this face cannot return is RECORDED rather than swallowed — see
            // [`Context::refused_declarations`].
            Arc::make_mut(&mut self.refused).push((name, format!("{e}")));
        }
    }

    /// Every declaration the checked gateway REFUSED, as `(name, reason)`, in refusal order.
    ///
    /// Empty is the healthy state and the thing to lock: it says every definition in this context
    /// was checked or is tracked as owed, and none was silently dropped.
    pub fn refused_declarations(&self) -> &[(String, String)] {
        &self.refused
    }

    /// THE CHECKED REGISTRATION GATEWAY — one door, and everything that registers a definition
    /// goes through it.
    ///
    /// `add_definition` used to be an `insert`. That is a hole the size of the whole kernel: the
    /// type a name is registered AT is the type `infer_type` hands back for it, so an unchecked
    /// insert lets any caller mint a constant of any type, `False` included, and every proof built
    /// on top of it certifies. The Repl surface had the check; the raw `Context` API, which is the
    /// API several hundred call sites use, did not.
    ///
    /// What it refuses, and what it deliberately does not:
    ///
    /// * a body whose inferred type is not a subtype of the declared one — REFUSED, nothing
    ///   registered. (That subsumes a nonsense declared type: nothing is a subtype of it.)
    /// * a body mentioning a GLOBAL that is not yet in scope — ACCEPTED, with a PENDING OBLIGATION
    ///   recorded against the missing name. Forward references are legitimate and common (a
    ///   δ-chain registered in ascending order; a family of mutually-referring definitions). The
    ///   obligation is retried the moment the missing name is declared, so the check is DEFERRED
    ///   rather than skipped — and [`Context::pending_obligations`] reports what is still owed,
    ///   which is strictly more than a kernel that simply accepted.
    /// * a body whose failure NOTHING can repair — REFUSED. A deferral is owed against a name a
    ///   later declaration could supply and against nothing else (see
    ///   `Context::deferrable_blocker`); an obligation keyed on anything else is an IOU with no
    ///   payer, and a definition sitting in the library behind one has a body that will never be
    ///   checked. That state is not reported here, it is made unreachable.
    /// * a body that would close a δ-CYCLE — REFUSED, before anything is inserted. See
    ///   `Context::delta_cycle_through`.
    ///
    /// Better than Lean here: Lean's `addDecl` refuses a forward reference outright, so a
    /// translator has to topologically sort before it can emit. This accepts the emission order
    /// and still reports every claim it has not been able to discharge.
    pub fn declare(&mut self, name: &str, ty: Term, body: Term) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_definition(
            self, name, None, &ty, &body,
        )];
        let result = self.declare_definition(name, None, ty, body);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    /// Run additional source-language checks and ordinary kernel body checking
    /// against one immutable context. The callback cannot authorize a body:
    /// the kernel still checks it, and all normal registration guards remain.
    /// Only kernel-earned positive conversion results are shared within this call.
    pub fn declare_with_validation<E: From<crate::error::KernelError>>(
        &mut self,
        name: &str,
        ty: Term,
        body: Term,
        validate: impl FnOnce(&Context, &Term, &Term) -> Result<(), E>,
    ) -> Result<(), E> {
        let checked = crate::conversion_session::scope(self, || {
            validate(self, &ty, &body)?;
            Ok::<_, E>(self.check_body(&ty, &body))
        })?;
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = vec![crate::dag::logos_env::candidate_definition(
            self, name, None, &ty, &body,
        )];
        let result =
            self.declare_definition_with_checked_body(name, None, ty, body, true, Some(checked));
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result.map_err(E::from)
    }

    /// Optional generated proofs use the same checked gateway, but an unknown
    /// dependency must refuse the candidate rather than leave a pending proof.
    pub(crate) fn declare_checked_auxiliary(
        &mut self,
        name: &str,
        ty: Term,
        body: Term,
    ) -> crate::error::KernelResult<()> {
        self.declare_definition_with_policy(name, None, ty, body, false)
    }

    fn declare_definition(
        &mut self,
        name: &str,
        levels: Option<Vec<String>>,
        ty: Term,
        body: Term,
    ) -> crate::error::KernelResult<()> {
        self.declare_definition_with_policy(name, levels, ty, body, true)
    }

    fn declare_definition_with_policy(
        &mut self,
        name: &str,
        levels: Option<Vec<String>>,
        ty: Term,
        body: Term,
        allow_pending: bool,
    ) -> crate::error::KernelResult<()> {
        self.declare_definition_with_checked_body(name, levels, ty, body, allow_pending, None)
    }

    fn declare_definition_with_checked_body(
        &mut self,
        name: &str,
        levels: Option<Vec<String>>,
        ty: Term,
        body: Term,
        allow_pending: bool,
        checked: Option<crate::error::KernelResult<()>>,
    ) -> crate::error::KernelResult<()> {
        self.check_native_dependency_mutation(name)?;
        let (ty, body) = (self.stored_term(ty), self.stored_term(body));
        // A definition that states no telescope binds every level its stored terms mention
        // (`term`'s rules 1 and 4): its type's, then those only its body's references carry.
        let levels = match levels {
            Some(params) => {
                Self::check_level_telescope("definition", name, &params, [&ty, &body])?;
                Some(params)
            }
            None => crate::term::definition_telescope(&ty, &body),
        };
        let bodyless_at_this_type = self
            .declarations
            .get(name)
            .is_some_and(|current| same_meaning(current, &ty))
            && !self.definitions.contains_key(name)
            && !self.universe_polys.contains_key(name)
            && match &levels {
                None => !self.poly_types.contains_key(name),
                Some(params) => self
                    .poly_types
                    .get(name)
                    .is_some_and(|(declared, declared_ty)| {
                        declared == params && same_meaning(declared_ty, &ty)
                    }),
            };
        let identical = match &levels {
            None => {
                !self.universe_polys.contains_key(name)
                    && self.definitions.get(name).is_some_and(|current| {
                        current.0.is_empty()
                            && same_meaning(&current.1, &ty)
                            && same_meaning(&current.2, &body)
                    })
            }
            Some(params) => self.universe_polys.get(name).is_some_and(|current| {
                &current.0 == params
                    && same_meaning(&current.1, &ty)
                    && same_meaning(&current.2, &body)
            }),
        };
        let change = if identical {
            Change::Identical
        } else if bodyless_at_this_type {
            Change::Refinement
        } else {
            Change::Different
        };
        if self.admit_meaning(name, change)? == Admission::Keep {
            return Ok(());
        }
        // δ-ACYCLICITY, before anything is inserted. Lean's own rule — a constant's value may only
        // reference constants declared before it — and it is what makes δ-normalisation TOTAL. See
        // `Context::delta_reaches` for why the check is affordable.
        if let Some(through) = self.delta_cycle_through(name, &body) {
            return Err(crate::error::KernelError::CertificationError(format!(
                "`{name}` cannot be defined in terms of `{through}`: `{through}` already unfolds \
                 back to `{name}`, so the pair has NO normal form and δ-reduction would never \
                 finish. A constant's value may only mention constants that do not depend on it \
                 (use a `fix`/`mutualfix` for a genuinely recursive definition, where the \
                 termination guard can see it)"
            )));
        }

        let verdict = match checked.unwrap_or_else(|| self.check_body(&ty, &body)) {
            Ok(()) => Verdict::Checked,
            Err(e) if !allow_pending => return Err(e),
            // DEFERRED, never skipped — but only when the FAILURE CAN STILL BE REPAIRED. The
            // condition is a property of the body and this context, not the spelling of one
            // error variant: a body mentioning a global that resolves nowhere may become
            // checkable the moment that global is declared, and nothing else may. Keying on
            // `KernelError::UnboundVariable` instead made two mistakes at once — a name in
            // `type_checker::UNTRUSTED_AXIOMS` is reported as a `CertificationError`, so a body
            // forward-referencing `sorryAx` was REFUSED rather than deferred; and a fixpoint's
            // own binder is reported as unbound, so a body blocked on a name NO declaration can
            // ever supply was deferred for ever, sitting in the trusted library with an IOU that
            // could not be paid.
            Err(e) => match self.deferrable_blocker(&ty, &body) {
                Some(missing) => Verdict::Pending(missing),
                None => return Err(e),
            },
        };

        // Displace first: without this a definition could sit alongside an inductive or constructor
        // of the same name, and `get_global` (inductives -> constructors -> definitions) would TYPE
        // the name from one map while `get_definition_body` REDUCED it from another. That is
        // conformance defect 3, a divergence between typing and reduction — the two
        // things a kernel most needs to agree.
        self.displace(name);
        match levels {
            Some(params) => self.record_universe_poly(name.to_string(), params, ty, body),
            None => self.record_definition(name.to_string(), ty, body),
        }

        match verdict {
            Verdict::Checked => {
                Arc::make_mut(&mut self.pending).remove(name);
            }
            Verdict::Pending(missing) => self.defer(name, missing),
        }
        // Equation synthesis is optional, checked front-end output. It does
        // nothing for pending bodies or contexts lacking the standard equality.
        crate::equations::generate(self, name);
        // Whatever was waiting on THIS name can now be retried.
        self.retry_pending(name);
        Ok(())
    }

    /// CHECK `body` against the DECLARED type `ty` — the bidirectional half of the gateway.
    ///
    /// For every body but a fixpoint this is exactly `infer_type` followed by the subtype test,
    /// unchanged. A `Term::Fix` is the one shape where CHECKING is strictly stronger than
    /// INFERRING, and the difference is not cosmetic:
    ///
    /// `infer_type`'s `Fix` arm has to GUESS the fixpoint's type from the body's shape
    /// (`type_checker::infer_fix_type_structurally`), because nothing has told it one. That guess
    /// reads a λ-telescope and then a `Match`'s motive; a body whose base case is an APPLICATION
    /// of a match (`fix rec. λn. λv1. λv2. (match v1 … ) v2`) falls through to plain inference in
    /// a context where the fixpoint's own binder is NOT bound, and reports it as unbound. A
    /// declaration does not have to guess — it was handed the type — so it binds the fixpoint's
    /// own name at the declared type and checks the body there, which is the rule a bidirectional
    /// elaborator states for `def f : T := fix f. e` and the rule Lean's recursion compiler
    /// implements.
    ///
    /// The GUARDIAN is not skipped: `termination::check_termination` runs on exactly the body
    /// `infer_type` would have run it on. Without it `fix f. f` would check at any type at all and
    /// `False` would be one line away.
    ///
    /// Two conditions keep this from ever accepting more than inference does:
    ///
    /// * the declared type must itself BE a type in this context, or the binding put in scope
    ///   would be nonsense and `is_subtype` would then compare the body against that same
    ///   nonsense;
    /// * a failure FALLS BACK to the inference path and reports ITS error, so nothing that is
    ///   accepted today stops being accepted and no diagnostic changes.
    /// A definition's body against its declared type: the declared type is a type, and the body
    /// checks against it (`type_checker::check_type`: a fixpoint at the declared type itself, with
    /// the guard; anything else by inference and cumulative subtyping).
    fn check_body(&self, ty: &Term, body: &Term) -> crate::error::KernelResult<()> {
        crate::type_checker::infer_sort(self, ty)?;
        crate::type_checker::check_type(self, body, ty)
    }

    /// Install a definition and the δ-bookkeeping that goes with it — the ONE place a definition
    /// enters `definitions`, so its edge list and the reverse "is this name mentioned anywhere"
    /// index can never fall out of step with it.
    fn record_definition(&mut self, name: String, ty: Term, body: Term) {
        self.record_definition_payload(name, Arc::new((Vec::new(), ty, body)));
    }

    /// Install a universe-polymorphic definition: the ONE place one enters `universe_polys`, with
    /// the δ-bookkeeping [`Context::record_definition`] gives a monomorphic one.
    fn record_universe_poly(&mut self, name: String, params: Vec<String>, ty: Term, body: Term) {
        self.record_definition_dependencies(&name, &body);
        self.record_definition_mentions(&name, &ty, &body);
        self.record(
            |c| &mut c.universe_polys,
            name,
            Arc::new((params, ty, body)),
        );
    }

    fn record_definition_payload(&mut self, name: String, payload: Arc<DefinitionPayload>) {
        self.invalidate_checked_opaque(&name);
        self.record_definition_dependencies(&name, &payload.2);
        self.record_definition_mentions(&name, &payload.1, &payload.2);
        self.record(|c| &mut c.definitions, name, payload);
    }

    /// A definition mentions what its TYPE mentions and what its body does — the body's edges
    /// were just recorded for the δ-graph, so they are read back rather than walked twice.
    fn record_definition_mentions(&mut self, name: &str, ty: &Term, body: &Term) {
        let edges = self
            .definition_deps
            .get(name)
            .map(|edges| edges.to_vec())
            .unwrap_or_default();
        self.record_mention_edges(name, edges);
        self.record_eliminations(name, body);
        self.record_mentions(name, ty);
    }

    fn record_definition_dependencies(&mut self, name: &str, body: &Term) {
        let mut borrowed: Vec<&str> = Vec::new();
        collect_global_mentions(body, &mut borrowed);
        borrowed.sort_unstable();
        borrowed.dedup();
        let edges: Vec<String> = borrowed.into_iter().map(str::to_string).collect();
        if !edges.is_empty() {
            let seen = Arc::make_mut(&mut self.referenced);
            for e in &edges {
                if !seen.contains(e.as_str()) {
                    seen.insert(e.clone());
                }
            }
        }
        let height = 1 + edges
            .iter()
            .map(|edge| self.definition_height(edge))
            .max()
            .unwrap_or(0);
        Arc::make_mut(&mut self.definition_heights).insert(name.to_string(), height);
        Arc::make_mut(&mut self.definition_deps)
            .insert(name.to_string(), edges.into_boxed_slice().into());
    }

    /// The definitional height of `name` (see `definition_heights`); 0 for anything without a
    /// registered body.
    pub(crate) fn definition_height(&self, name: &str) -> u32 {
        self.definition_heights.get(name).copied().unwrap_or(0)
    }

    /// The first name in a definition's declared type or `body` that RESOLVES NOWHERE and that a
    /// later declaration could still supply — the exact condition under which a deferral is owed,
    /// and the only one.
    ///
    /// "Could still supply" is the whole content of the predicate, and it is why only a
    /// CONSTANT-form name qualifies. Every door that binds a constant — `declare`,
    /// `add_declaration`, `add_inductive`, `add_constructor`, `add_universe_poly` — retries the
    /// obligations keyed on the name it bound. Nothing retries a `Term::Var`, and nothing ever
    /// could: `infer_type`'s `Var` arm reads `Context::bindings`, which belongs to the enclosing
    /// TERM's binders and not to the environment. So an obligation keyed on a variable name is an
    /// IOU with no payer — which is exactly the state three prelude definitions were in, and
    /// exactly the state `Definition boom : False := fix f. f` exploited: `infer_type` reports the
    /// fixpoint's own binder as `UnboundVariable("f")`, the gateway read the VARIANT, deferred,
    /// and registered a constant of type `False` without the termination guard ever running.
    fn deferrable_blocker(&self, ty: &Term, body: &Term) -> Option<String> {
        self.first_unresolvable_global(ty)
            .or_else(|| self.first_unresolvable_global(body))
    }

    fn first_unresolvable_global(&self, body: &Term) -> Option<String> {
        let mut names: Vec<&str> = Vec::new();
        collect_global_mentions(body, &mut names);
        names
            .into_iter()
            .find(|n| !self.resolves(n))
            .map(str::to_string)
    }

    /// Does `name` resolve as a CONSTANT here — asked exactly the way `infer_type` asks it, so
    /// this can never call reachable a name typing would call unbound.
    fn resolves(&self, name: &str) -> bool {
        self.get_global(name).is_some() || self.get_universe_poly(name).is_some()
    }

    /// The global in `body` through which defining `name` would close a δ-CYCLE, if there is one.
    ///
    /// Lean's rule, and the reason δ-normalisation is total there: a constant's value may only
    /// mention constants that do not depend on it. Without it `loop_a := loop_b` beside
    /// `loop_b := loop_a` is a well-typed pair with NO normal form, and `normalize` has nothing to
    /// hand back but an abandoned reduction that looks exactly like a finished one —
    /// conformance defect 9. Refusing the SECOND of the two is the whole fix, and it needs no change to
    /// reduction at all.
    ///
    /// AFFORDABLE, which is what makes it a gateway check rather than an audit:
    ///
    /// * a name that nothing already mentions cannot be on a cycle, so the common case (a fresh
    ///   declaration) costs one hash lookup — that is what `referenced` is for;
    /// * the search expands only DEFINITIONS and stops the moment it sees `name`, so a chain
    ///   declared in forward-reference order (`chain0 := chain1` while `chain1` has no body yet)
    ///   costs one step per insert, not one per link;
    /// * `name`'s own current body is never expanded, because the walk stops there — which is
    ///   right, since a redeclaration REPLACES it.
    fn delta_cycle_through(&self, name: &str, body: &Term) -> Option<String> {
        let mut roots: Vec<&str> = Vec::new();
        collect_global_mentions(body, &mut roots);
        // A body that names ITSELF is the one-element cycle, and it is a cycle whether or not
        // anything else has ever mentioned the name — so this question is asked before the
        // `referenced` short-circuit, not after it.
        if roots.contains(&name) {
            return Some(name.to_string());
        }
        if !self.referenced.contains(name) {
            return None;
        }
        roots.sort_unstable();
        roots.dedup();
        roots
            .into_iter()
            .find(|g| self.delta_reaches(g, name))
            .map(str::to_string)
    }

    /// Whether a term depends on a definition whose admission is unfinished.
    ///
    /// Inspect references before reduction: beta/zeta reduction may erase one,
    /// and a previously admitted definition can acquire a pending dependency
    /// through redeclaration. The graph is read afresh on every query.
    pub(crate) fn has_pending_dependency(&self, term: &Term) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let mut roots = Vec::new();
        collect_global_mentions(term, &mut roots);
        let mut stack: Vec<String> = roots.into_iter().map(str::to_owned).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(name) = stack.pop() {
            if self.pending.contains_key(&name) {
                return true;
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(edges) = self.edges_of(&name) {
                stack.extend(edges.iter().filter(|n| !seen.contains(*n)).cloned());
            }
            let ty = self
                .get_global(&name)
                .or_else(|| self.get_universe_poly(&name).map(|(_, ty, _)| ty));
            if let Some(ty) = ty {
                let mut references = Vec::new();
                collect_global_mentions(ty, &mut references);
                stack.extend(
                    references
                        .into_iter()
                        .filter(|n| !seen.contains(*n))
                        .map(str::to_owned),
                );
            }
        }
        false
    }

    /// The δ-edges of `name`: the cached list where one exists, and the body's own mentions
    /// otherwise. `None` for a name that is not a definition at all — a leaf of the δ-graph.
    fn edges_of(&self, name: &str) -> Option<Arc<[String]>> {
        if let Some(cached) = self.definition_deps.get(name) {
            return Some(cached.clone());
        }
        let body = &self
            .definitions
            .get(name)
            .or_else(|| self.universe_polys.get(name))?
            .2;
        let mut out: Vec<&str> = Vec::new();
        collect_global_mentions(body, &mut out);
        out.sort_unstable();
        out.dedup();
        let owned: Vec<String> = out.into_iter().map(str::to_string).collect();
        Some(owned.into_boxed_slice().into())
    }

    /// Does unfolding definitions from `start` ever reach `target`? `start == target` counts: a
    /// definition whose own body names it is a one-element cycle.
    fn delta_reaches(&self, start: &str, target: &str) -> bool {
        if start == target {
            return true;
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut stack: Vec<String> = vec![start.to_string()];
        while let Some(cur) = stack.pop() {
            let Some(edges) = self.edges_of(&cur) else {
                continue;
            };
            if !seen.insert(cur) {
                continue;
            }
            for n in edges.iter() {
                if n == target {
                    return true;
                }
                if !seen.contains(n.as_str())
                    && (self.definitions.contains_key(n.as_str())
                        || self.universe_polys.contains_key(n.as_str()))
                {
                    stack.push(n.clone());
                }
            }
        }
        false
    }

    /// Record that `who`'s body cannot be checked until `blocker` is in scope.
    fn defer(&mut self, who: &str, blocker: String) {
        Arc::make_mut(&mut self.pending_waiters)
            .entry(blocker.clone())
            .or_default()
            .push(who.to_string());
        Arc::make_mut(&mut self.pending).insert(who.to_string(), blocker);
    }

    /// Retry every deferred obligation that was blocked on `name`, now that `name` is in scope.
    ///
    /// Amortised O(1) per declaration: obligations are keyed by the name they wait for, so this
    /// touches only the ones that were actually unblocked. A retry that fails again is re-keyed on
    /// whatever it is missing NOW (a chain of forward references resolves one link at a time); a
    /// retry that finds a genuine type error leaves the obligation standing, where
    /// [`Context::pending_obligations`] reports it, rather than retroactively deleting a constant
    /// other declarations may already be built on.
    fn retry_pending(&mut self, name: &str) {
        let mut ready = std::collections::VecDeque::from([name.to_string()]);
        while let Some(available) = ready.pop_front() {
            let Some(waiting) = Arc::make_mut(&mut self.pending_waiters).remove(&available) else {
                continue;
            };
            for who in waiting {
                if self.pending.get(&who).map(String::as_str) != Some(available.as_str()) {
                    continue;
                }
                let definition = self
                    .definitions
                    .get(&who)
                    .or_else(|| self.universe_polys.get(&who))
                    .cloned();
                let Some(payload) = definition else {
                    // A deferred assumption: its type, checked now.
                    if let Some(ty) = self.declarations.get(&who).cloned() {
                        match crate::type_checker::infer_sort(self, &ty) {
                            Ok(_) => {
                                Arc::make_mut(&mut self.pending).remove(&who);
                                ready.push_back(who);
                            }
                            Err(_) => {
                                if let Some(next) = self.first_unresolvable_global(&ty) {
                                    self.defer(&who, next);
                                }
                            }
                        }
                        continue;
                    }
                    Arc::make_mut(&mut self.pending).remove(&who);
                    continue;
                };
                let (_, ty, body) = payload.as_ref();
                match self.check_body(ty, body) {
                    Ok(()) => {
                        Arc::make_mut(&mut self.pending).remove(&who);
                        crate::equations::generate(self, &who);
                        ready.push_back(who);
                    }
                    Err(_) => {
                        if let Some(next) = self.deferrable_blocker(ty, body) {
                            self.defer(&who, next);
                        }
                    }
                }
            }
        }
    }

    /// Definitions whose body has NOT yet been checked against its declared type, and assumptions
    /// whose type has not, each paired with the name it is still waiting for — our `#print axioms`
    /// for deferred obligations.
    ///
    /// Pending definitions remain recorded but are unavailable to constant lookup and
    /// reduction until their bodies have been checked. Resolving a dependency retries its
    /// waiting definitions and exposes only those whose checks succeed.
    pub fn pending_obligations(&self) -> Vec<(&str, &str)> {
        self.pending
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Look up a global definition (inductive, constructor, definition, or declaration).
    ///
    /// Returns the type of the global.
    pub fn get_global(&self, name: &str) -> Option<&Term> {
        if self.pending.contains_key(name) {
            return None;
        }
        // Check inductives first
        if let Some(sort) = self.inductives.get(name) {
            return Some(sort);
        }
        // Check constructors
        if let Some((_, ty)) = self.constructors.get(name) {
            return Some(ty);
        }
        // Check definitions (return type, not body)
        if let Some(payload) = self.definitions.get(name) {
            return Some(&payload.1);
        }
        // Check declarations (axioms)
        self.declarations.get(name)
    }

    /// Check if a name is a definition (has a body that can be unfolded).
    pub fn is_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    /// Get the body of a definition, if it exists.
    ///
    /// Returns None for axioms, constructors, and inductives (only definitions have bodies).
    pub fn get_definition_body(&self, name: &str) -> Option<&Term> {
        if self.pending.contains_key(name) {
            return None;
        }
        self.definitions.get(name).map(|payload| &payload.2)
    }

    /// Get the type of a definition, if it exists.
    pub fn get_definition_type(&self, name: &str) -> Option<&Term> {
        if self.pending.contains_key(name) {
            return None;
        }
        self.definitions.get(name).map(|payload| &payload.1)
    }

    /// Check if a name is a constructor.
    pub fn is_constructor(&self, name: &str) -> bool {
        self.constructors.contains_key(name)
    }

    /// Get the inductive type a constructor belongs to.
    pub fn constructor_inductive(&self, name: &str) -> Option<&str> {
        self.constructors.get(name).map(|(ind, _)| ind.as_str())
    }

    /// Check if a name is an inductive type.
    pub fn is_inductive(&self, name: &str) -> bool {
        self.inductives.contains_key(name)
    }

    /// Ordered-name inventory length before filtering stale registry entries.
    /// A borrowed allocation-admission upper bound, never a validity certificate.
    pub(crate) fn constructor_inventory_upper_bound(&self, inductive: &str) -> usize {
        self.constructor_order
            .get(inductive)
            .map_or(0, |names| names.len())
    }

    /// Get all constructors for an inductive type, in registration order.
    ///
    /// Returns a vector of (constructor_name, constructor_type) pairs.
    pub fn get_constructors(&self, inductive: &str) -> Vec<(&str, &Term)> {
        self.constructor_headers(inductive).collect()
    }

    /// Borrow constructor headers in the same registration order as get_constructors.
    pub(crate) fn constructor_headers<'a>(
        &'a self,
        inductive: &str,
    ) -> impl Iterator<Item = (&'a str, &'a Term)> {
        self.constructor_order
            .get(inductive)
            .into_iter()
            .flatten()
            .filter_map(|name| {
                self.constructors
                    .get(name)
                    .map(|(_, ty)| (name.as_str(), ty))
            })
    }

    /// Borrow the sole ordered constructor header, without an inventory allocation.
    /// This lookup is metadata only and does not certify the declaration.
    pub(crate) fn unique_constructor_header(&self, inductive: &str) -> Option<(&str, &Term)> {
        let names = self.constructor_order.get(inductive)?;
        if names.len() != 1 {
            return None;
        }
        let name = &names[0];
        let (_, ty) = self.constructors.get(name)?;
        Some((name.as_str(), ty))
    }

    /// Iterate over all declarations (hypotheses).
    ///
    /// Used by the certifier to find hypothesis by type.
    pub fn iter_declarations(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.declarations.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over all definitions.
    ///
    /// Used by the UI to display definitions.
    pub fn iter_definitions(&self) -> impl Iterator<Item = (&str, &Term, &Term)> {
        self.definitions
            .iter()
            .map(|(k, payload)| (k.as_str(), &payload.1, &payload.2))
    }

    /// Iterate over all inductive types.
    ///
    /// Used by the UI to display inductive types.
    pub fn iter_inductives(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.inductives.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Iterate over the universe-polymorphic definitions: name, level parameters, type, body.
    pub fn iter_universe_polys(&self) -> impl Iterator<Item = (&str, &[String], &Term, &Term)> {
        self.universe_polys
            .iter()
            .map(|(k, payload)| (k.as_str(), payload.0.as_slice(), &payload.1, &payload.2))
    }

    /// Add a constructor with strict positivity checking.
    ///
    /// Returns an error if the inductive type appears negatively in the
    /// constructor type. This prevents paradoxes like:
    /// ```text
    /// Inductive Bad := Cons : (Bad -> False) -> Bad
    /// ```
    /// Derive and register everything Lean synthesises at declaration time — `{I}_rec`,
    /// `{I}_noConfusionType`, `{I}_noConfusion`, `{C}_inj`, `{I}_below`, `{I}_brecOn`, `decEq{I}` —
    /// and register `decEq{I}` as a `Decidable` instance so `decide` can actually reach it.
    ///
    /// Lean generates these for EVERY inductive; we generated exactly one, and even that had to be
    /// requested by the caller. Every `injection`, every `cases h` on an equation, every
    /// `simp [injEq]` in a Lean development names one of these. They are all DERIVED and
    /// kernel-checked — axiomatising them would turn each of those proofs into an assumption.
    ///
    /// Best-effort by construction: `derive_auxiliaries` omits what it cannot state (indexed
    /// families, `Prop` carriers where disjointness is *false* by proof irrelevance, carriers above
    /// `Type 0` while `Eq` is `Type 0`-only) rather than faking it. Silence here is a scope
    /// boundary, not a failure.
    pub(crate) fn regenerate_auxiliaries(&mut self, ind: &str) {
        if self.native_positivity_dependencies.contains(ind) {
            return;
        }
        // Nothing to derive for a name that is not (yet) an inductive.
        if !self.is_inductive(ind) {
            return;
        }
        // Drop only OUR previous generation for this inductive — never a hand-written constant,
        // and never one that has since been displaced by something else (`displace` disowns).
        if let Some(previous) = Arc::make_mut(&mut self.derived_aux).remove(ind) {
            for name in previous {
                self.displace(&name);
            }
        }
        // A universe-polymorphic family's auxiliaries come stated over its telescope, its own names
        // written at the generic instance, as Lean's are (`recursor::generic_family`), and are
        // registered polymorphic in it. Any other auxiliary binds the levels its stored terms
        // mention, as a definition through the gateway does.
        let produced = crate::recursor::derive_auxiliaries(self, ind);
        let telescope = crate::recursor::generic_family(self, ind).map(|(ps, _)| ps);
        let mut names: Vec<String> = Vec::with_capacity(produced.len());
        for crate::recursor::DerivedAux { name, ty, term } in produced {
            // KERNEL-CHECKED before it is trusted, through the door every definition takes. `derive_*`
            // is UNTRUSTED front-end code: it builds a term and claims a type, and declaration-time
            // registration is the moment that claim would otherwise become an assumption of the
            // standard library. Checking costs one pass per auxiliary and buys the guarantee that
            // an auxiliary is a THEOREM, never an axiom — the same bargain `add_nested_inductive`
            // already strikes for its conversion isos.
            //
            // A derivation that does not check is DROPPED, silently but harmlessly: the constant
            // simply does not exist, exactly as before declaration-time synthesis existed. It is
            // never registered unchecked.
            if self.check_body(&ty, &term).is_err() {
                continue;
            }
            // A DERIVED decidable equality is also an INSTANCE, or nothing can resolve it — and it
            // goes in the FALLBACK tier, never the declared one: `Context::instances()` is the
            // faithful record of what the program declared, and a library-synthesised entry must
            // not outrank a user instance for the same goal (nor move a count measured against it).
            if name == format!("decEq{ind}") {
                let value = match &telescope {
                    Some(ps) => Term::Const {
                        name: name.clone(),
                        levels: ps.iter().map(|l| Universe::Var(l.clone())).collect(),
                    },
                    None => Term::Global(name.clone()),
                };
                self.add_fallback_instance(ty.clone(), value);
            }
            Arc::make_mut(&mut self.derived_aux_owner).insert(name.clone(), ind.to_string());
            names.push(name.clone());
            match telescope
                .clone()
                .or_else(|| crate::term::definition_telescope(&ty, &term))
            {
                Some(ps) => self.record_universe_poly(name.clone(), ps, ty, term),
                None => self.record_definition(name.clone(), ty, term),
            }
            crate::equations::generate(self, &name);
        }
        if !names.is_empty() {
            Arc::make_mut(&mut self.derived_aux).insert(ind.to_string(), names);
        }
    }

    /// Validate the constructor telescope before any registration. Positivity alone
    /// cannot establish that the result is the declared inductive, or even a type.
    fn check_constructor_signature(
        &self,
        name: &str,
        inductive: &str,
        ty: &Term,
        block: &[&str],
    ) -> crate::error::KernelResult<()> {
        use crate::error::KernelError;
        use crate::type_checker::{def_eq, infer_sort, substitute};
        let bad = |why: &str| {
            KernelError::InvalidDeclaration(format!("constructor '{name}' of '{inductive}': {why}"))
        };
        infer_sort(self, ty)?;
        let normalize = |ctx: &Context, term: &Term| {
            crate::reduction::normalize_checked(ctx, term)
                .map_err(|e| KernelError::CertificationError(format!(
                    "constructor '{name}' of '{inductive}': signature normalization did not finish: {e:?}"
                )))
        };
        // A failed bounded conversion attempt is not by itself a semantic
        // counterexample. Require completed normal forms before rejecting a
        // mismatch; exhaustion remains an inconclusive certification error.
        let equal =
            |ctx: &Context, left: &Term, right: &Term| -> crate::error::KernelResult<bool> {
                if def_eq(ctx, left, right) {
                    return Ok(true);
                }
                let left = normalize(ctx, left)?;
                let right = normalize(ctx, right)?;
                Ok(def_eq(ctx, &left, &right))
            };
        let mut expected = normalize(
            self,
            self.get_global(inductive)
                .ok_or_else(|| bad("parent is not declared"))?,
        )?;
        let target_universe =
            crate::type_checker::constructor_target_universe(inductive, &expected)?;
        let mut cur = ty.clone();
        let mut ext = self.clone();
        let nparams = self.inductive_num_params(inductive);
        let mut parameters = Vec::new();
        let mut i = 0;
        loop {
            if !matches!(cur, Term::Pi { .. }) {
                cur = normalize(&ext, &cur)?;
            }
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = cur
            else {
                break;
            };
            let mut fresh = format!("__constructor_local_{i}");
            while ext.get(&fresh).is_some() {
                fresh.push('_');
            }
            let variable = Term::Var(fresh.clone());
            if i < nparams {
                let Term::Pi {
                    param: p,
                    param_type: pty,
                    body_type: pbody,
                } = expected
                else {
                    return Err(bad("more parameters than the parent telescope"));
                };
                if !equal(&ext, &param_type, &pty)? {
                    return Err(bad("parameter domain differs from the parent telescope"));
                }
                expected = substitute(&pbody, &p, &variable);
                parameters.push(variable.clone());
            }
            if i >= nparams {
                // Signature validation already exposes each telescope tail and
                // freshens its binders. Check precisely that same stored field.
                crate::type_checker::check_constructor_field_universe(
                    &ext,
                    inductive,
                    name,
                    &param_type,
                    &target_universe,
                )?;
            }
            if i >= nparams && nparams != 0 {
                // Uniformity is a property of the field up to conversion. In
                // particular (lambda f. f p) I contains a partial I syntactically
                // but its checked normal form is the complete occurrence I p.
                let field = normalize(&ext, &param_type)?;
                Self::check_recursive_parameter_occurrences(
                    &ext,
                    name,
                    block,
                    &parameters,
                    field,
                    &equal,
                )?;
            }
            ext = ext.extend(&fresh, *param_type);
            cur = substitute(&body_type, &param, &variable);
            i += 1;
        }
        if parameters.len() != nparams {
            return Err(bad("constructor omits uniform parameters"));
        }
        let result = normalize(&ext, &cur)?;
        let mut head = &result;
        let mut args = Vec::new();
        while let Term::App(f, a) = head {
            args.push(a.as_ref());
            head = f;
        }
        args.reverse();
        if !matches!(head, Term::Global(n) | Term::Const { name: n, .. } if n == inductive) {
            return Err(bad("result is not the declared inductive"));
        }
        if args.len() != self.inductive_arity(inductive) {
            return Err(bad("result does not fully apply the parent telescope"));
        }
        for (arg, parameter) in args.iter().zip(&parameters) {
            if !equal(&ext, arg, parameter)? {
                return Err(bad("result changes a uniform parameter"));
            }
        }
        Ok(())
    }

    /// Check complete recursive spines inside an already type-checked, normalized
    /// constructor field. Ownership makes this walk iterative without cloning
    /// every subtree; contexts retain each nested binder's lexical identity.
    fn check_recursive_parameter_occurrences(
        ctx: &Context,
        constructor: &str,
        block: &[&str],
        parameters: &[Term],
        field: Term,
        equal: &impl Fn(&Context, &Term, &Term) -> crate::error::KernelResult<bool>,
    ) -> crate::error::KernelResult<()> {
        use crate::error::KernelError;
        use crate::type_checker::{infer_type, substitute};
        let invalid = |why: &str| {
            KernelError::InvalidDeclaration(format!(
                "constructor '{constructor}': recursive occurrence {why}"
            ))
        };
        let fresh = |ctx: &Context| {
            let mut name = "__recursive_parameter_local".to_string();
            while ctx.get(&name).is_some() {
                name.push('_');
            }
            name
        };
        let mut pending = vec![(field, ctx.clone())];
        while let Some((term, scope)) = pending.pop() {
            let mut head = term;
            let mut arguments = Vec::new();
            while let Term::App(function, argument) = head {
                arguments.push(*argument);
                head = *function;
            }
            arguments.reverse();
            if matches!(&head, Term::Global(n) | Term::Const{name:n,..} if block.contains(&n.as_str()))
            {
                if arguments.len() < parameters.len() {
                    return Err(invalid("omits a uniform parameter"));
                }
                for (argument, expected) in arguments.iter().zip(parameters) {
                    if !equal(&scope, argument, expected)? {
                        return Err(invalid("changes a uniform parameter"));
                    }
                }
                // Do not revisit the function-side partial spines. Arguments can
                // themselves contain a different complete recursive occurrence.
                pending.extend(arguments.into_iter().map(|a| (a, scope.clone())));
                continue;
            }
            pending.extend(arguments.into_iter().map(|a| (a, scope.clone())));
            match head {
                Term::Pi {
                    param,
                    param_type,
                    body_type,
                }
                | Term::Lambda {
                    param,
                    param_type,
                    body: body_type,
                } => {
                    let name = fresh(&scope);
                    let variable = Term::Var(name.clone());
                    let body = substitute(&body_type, &param, &variable);
                    let inside = scope.extend(&name, (*param_type).clone());
                    pending.push((*param_type, scope));
                    pending.push((body, inside));
                }
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => {
                    // A checked normal form ordinarily has no lets. Preserve
                    // lexical substitution if normalization retained one.
                    let body = substitute(&body, &name, &value);
                    pending.push((*ty, scope.clone()));
                    pending.push((*value, scope.clone()));
                    pending.push((body, scope));
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    pending.push((*discriminant, scope.clone()));
                    pending.push((*motive, scope.clone()));
                    pending.extend(cases.into_iter().map(|case| (case, scope.clone())));
                }
                Term::Fix { name, body } => {
                    let ty = infer_type(
                        &scope,
                        &Term::Fix {
                            name: name.clone(),
                            body: body.clone(),
                        },
                    )?;
                    let new = fresh(&scope);
                    let body = substitute(&body, &name, &Term::Var(new.clone()));
                    pending.push((body, scope.extend(&new, ty)));
                }
                Term::MutualFix { defs, index: _ } => {
                    let mut inside = scope.clone();
                    let mut substitutions = Vec::new();
                    for (index, (name, _)) in defs.iter().enumerate() {
                        let ty = infer_type(
                            &scope,
                            &Term::MutualFix {
                                defs: defs.clone(),
                                index,
                            },
                        )?;
                        let mut new = fresh(&inside);
                        while defs.iter().any(|(bound, _)| bound == &new) {
                            new.push('_');
                            while inside.get(&new).is_some() {
                                new.push('_');
                            }
                        }
                        inside = inside.extend(&new, ty);
                        substitutions.push((name.clone(), Term::Var(new)));
                    }
                    for (_, mut body) in defs {
                        for (name, value) in &substitutions {
                            body = substitute(&body, name, value);
                        }
                        pending.push((body, inside.clone()));
                    }
                }
                Term::Var(_)
                | Term::Global(_)
                | Term::Const { .. }
                | Term::Sort(_)
                | Term::Lit(_)
                | Term::Hole => {}
                Term::Annotated { term, .. } => pending.push((*term, scope)),
                Term::App(..) => unreachable!("application spine peeled above"),
            }
        }
        Ok(())
    }

    pub fn add_constructor_checked(
        &mut self,
        name: &str,
        inductive: &str,
        ty: Term,
    ) -> crate::error::KernelResult<()> {
        self.check_native_dependency_mutation(name)?;
        self.check_native_dependency_mutation(inductive)?;
        let ty = self.stored_term(ty);
        // The inductive must EXIST, and this has to be the first question asked. Every gate below
        // treats an unknown head as inert — `check_constructor_universes` returns `Ok(())` when
        // `get_global` misses, and `check_positivity` sees no occurrence to be negative — so the
        // CHECKED path used to admit a constructor for a type that does not exist. A
        // universe-polymorphic definition of the same name is the live route: `Const{BoxP, [u]}`
        // resolves, `Global("BoxP")` does not, and a `BoxP_mk` whose own type will not even infer
        // landed anyway.
        if !self.is_inductive(inductive) {
            return Err(crate::error::KernelError::CertificationError(format!(
                "constructor '{name}' names '{inductive}', which is not a registered inductive — \
                 declare the inductive first (a definition or a universe-polymorphic abbreviation \
                 of that name is not an inductive and has no constructors)"
            )));
        }
        // The stable-meaning gates, asked before any work so a refusal is RETURNED: the bulk
        // door below can only record one.
        if self.admit_constructor(name, inductive, &ty)? == Admission::Keep {
            return Ok(());
        }
        // Evaluate original typing before bounded head conversion, retaining
        // the established proven-negative diagnostic when both gates fail.
        // A positive shape never waives an ill-typed erased beta argument.
        let source_typing = crate::type_checker::infer_sort(self, &ty);
        let block: Vec<&str> = self
            .mutual_block_of(inductive)
            .map(|members| members.iter().map(String::as_str).collect())
            .unwrap_or_else(|| vec![inductive]);
        let dependencies =
            crate::positivity::native_positivity_dependencies(self, &block, name, &ty)?;
        source_typing?;
        self.check_native_dependencies(&dependencies)?;
        // Infer the final parameter/index split with the candidate included, without
        // making the candidate available to its own type checker. The first constructor
        // can establish that an arity argument is an index rather than a parameter.
        let mut validation = self.clone();
        if self.inductive_declared_params(inductive).is_none() {
            let mut prospective = self.clone();
            prospective.auto_aux = false;
            prospective.add_constructor(name, inductive, ty.clone());
            let params = prospective.inductive_num_params(inductive);
            validation.set_inductive_params(inductive, params);
            if params != self.inductive_num_params(inductive) {
                // Previously uniform parameters become stored fields when the split
                // shrinks. Recheck their universe bounds before committing the change.
                for (old_name, old_ty) in self.get_constructors(inductive) {
                    validation.check_constructor_signature(old_name, inductive, old_ty, &block)?;
                }
            }
        }
        validation.check_constructor_signature(name, inductive, &ty, &block)?;

        // If it passes, add the constructor normally
        self.add_constructor(name, inductive, ty);
        // DECLARATION-TIME SYNTHESIS on the CHECKED door — Lean's behaviour, and T03-018.
        // `{I}_noConfusion`, `{I}_noConfusionType`, `{C}_inj`, `{I}_below`, `{I}_brecOn`,
        // `decEq{I}` exist the moment the declaration does, all derived and kernel-checked, none
        // of them an assumption.
        //
        // The design tension that held this back is CLOSED: `recursor::push_aux` is
        // provenance-keyed (`Context::is_derived_aux`), so a re-derivation replaces ITS OWN prior
        // generation and still refuses to touch a hand-written constant — which is why the
        // prelude's `decEqNat` survives byte-identical while a two-constructor user inductive gets
        // a regenerated eliminator.
        //
        // It fires once per CONSTRUCTOR because there is no "declaration finished" call; the
        // auxiliaries derived after the first constructor describe an inductive that is not
        // finished yet, and the last generation is the one that stands. Only the CHECKED door is
        // eager: `add_constructor` is the bulk door a library loader and a translator use, and it
        // keeps the `auto_aux` switch and the `seal` batch so registering a standard library does
        // not pay per constructor for tables nothing will ask for.
        self.regenerate_auxiliaries(inductive);
        self.seal_native_dependencies(dependencies);
        Ok(())
    }

    /// Register a MUTUAL block of inductives whose constructors may reference one
    /// another (`Even`/`Odd`, `Tree`/`Forest`). Strict positivity is checked over the
    /// WHOLE block up front — a sibling occurrence is a recursive occurrence, a
    /// sibling in a negative position is a cross-block paradox and rejected — and the
    /// registration is TRANSACTIONAL: if any constructor violates positivity, nothing
    /// is added. On success every member's header (with its parameter split), every
    /// constructor, and the block-membership registry are populated, ready for the
    /// auto-derived mutual recursors.
    pub fn add_mutual_inductives(
        &mut self,
        block: &[MutualInductive],
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = crate::dag::logos_env::candidate_block(self, None, block);
        let result = self.add_inductive_block(block, None);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    fn add_inductive_block(
        &mut self,
        block: &[MutualInductive],
        levels: Option<&[String]>,
    ) -> crate::error::KernelResult<()> {
        if let Some(erased) = erased_block(block) {
            return self.add_inductive_block(&erased, levels);
        }
        for member in block {
            self.check_native_dependency_mutation(&member.name)?;
            for (name, _) in &member.constructors {
                self.check_native_dependency_mutation(name)?;
            }
        }
        let names: Vec<&str> = block.iter().map(|m| m.name.as_str()).collect();
        // A header must end in Sort. Merely having a type would also admit
        // `I : False`, which would register I itself as a proof of False.
        let mut header_env = self.clone();
        for member in block {
            header_env.displace(&member.name);
        }
        let mut checked_arities = Vec::with_capacity(block.len());
        for member in block {
            crate::type_checker::infer_sort(&header_env, &member.sort)?;
            let arity = exposed_arity(&header_env, &member.sort, &mut |ctx, t| {
                crate::reduction::normalize_checked(ctx, t)
            })
            .map_err(|e| {
                crate::error::KernelError::CertificationError(format!(
                    "inductive '{}' header did not normalize: {e:?}",
                    member.name
                ))
            })?;
            let mut result = &arity;
            while let Term::Pi { body_type, .. } = result {
                result = body_type;
            }
            if !matches!(result, Term::Sort(_)) {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "inductive '{}' must have a parameter telescope ending in Sort",
                    member.name
                )));
            }
            checked_arities.push(arity);
        }
        // Reconcile precisely the checked, exposed telescopes that metadata and
        // auxiliary generation will use. No constructors are published yet.
        let counts: Vec<_> = block
            .iter()
            .zip(&checked_arities)
            .map(|(member, arity)| {
                (
                    member.name.as_str(),
                    member.num_params,
                    count_leading_pis(arity),
                )
            })
            .collect();
        reconcile_parameter_counts(&counts)?;
        // A block something already depends on keeps its meaning: registered again identically it
        // is left exactly as it is, and registered any other way it is refused.
        let identical = block.iter().zip(&checked_arities).all(|(member, arity)| {
            let current = self.get_constructors(&member.name);
            self.inductives
                .get(&member.name)
                .is_some_and(|current| same_meaning(current, arity))
                && self.inductive_num_params(&member.name) == member.num_params
                && levels.map_or(true, |levels| {
                    self.poly_types
                        .get(&member.name)
                        .is_some_and(|(declared, _)| declared.as_slice() == levels)
                })
                && current.len() == member.constructors.len()
                && current.iter().zip(&member.constructors).all(
                    |(&(name, ty), (wanted, wanted_ty))| {
                        name == wanted.as_str() && same_meaning(ty, wanted_ty)
                    },
                )
        });
        let mut relied_on = false;
        for member in block {
            relied_on |=
                self.admit_meaning(&member.name, Self::change_if(identical))? == Admission::Keep;
            for (constructor, _) in &member.constructors {
                relied_on |=
                    self.admit_meaning(constructor, Self::change_if(identical))? == Admission::Keep;
            }
        }
        if relied_on {
            return Ok(());
        }
        let mut temp = self.clone();
        for (member, arity) in block.iter().zip(&checked_arities) {
            match levels {
                Some(levels) => temp.add_universe_poly_inductive(
                    &member.name,
                    levels.to_vec(),
                    arity.clone(),
                    member.num_params,
                ),
                None => temp.add_indexed_inductive(&member.name, arity.clone(), member.num_params),
            }
        }
        let mut dependencies = std::collections::HashSet::new();
        // 1. Positivity of every constructor against the whole block, BEFORE mutating.
        for member in block {
            for (cname, cty) in &member.constructors {
                let source_typing = crate::type_checker::infer_sort(&temp, cty);
                dependencies.extend(crate::positivity::native_positivity_dependencies(
                    &temp, &names, cname, cty,
                )?);
                source_typing?;
            }
        }
        self.check_native_dependencies(&dependencies)?;
        // 1b. UNIVERSE CONSISTENCY of every constructor — checked in a TEMP env carrying the
        // block's headers (so a recursive/sibling field resolves), before mutating `self`,
        // keeping the whole registration transactional.
        {
            for member in block {
                for (cname, cty) in &member.constructors {
                    if let Some(levels) = levels {
                        let normalized = crate::normalize_checked(&temp, cty).map_err(|e| {
                            crate::error::KernelError::CertificationError(format!(
                                "constructor '{cname}' normalization unfinished: {e:?}"
                            ))
                        })?;
                        Self::check_block_level_instances(&normalized, &names, levels)?;
                    }
                    temp.check_constructor_signature(cname, &member.name, cty, &names)?;
                }
            }
        }
        // 2. Publish the same exposed headers used by every preceding gate.
        for (member, arity) in block.iter().zip(checked_arities) {
            match levels {
                Some(levels) => self.add_universe_poly_inductive(
                    &member.name,
                    levels.to_vec(),
                    arity,
                    member.num_params,
                ),
                None => self.add_indexed_inductive(&member.name, arity, member.num_params),
            }
        }
        // 3. Record block membership — BEFORE the constructors, because registering a
        // constructor now synthesises the member's auxiliaries, and a member that does not yet
        // know it is in a block would get a STANDALONE recursor: one that reads a sibling
        // occurrence as an ordinary non-recursive field. Membership first means every derivation
        // sees the real block.
        if block.len() > 1 {
            let members: Vec<String> = block.iter().map(|m| m.name.clone()).collect();
            let reg = Arc::make_mut(&mut self.mutual_blocks);
            for member in block {
                reg.insert(member.name.clone(), members.clone());
            }
        }
        // 4. All constructors.
        for member in block {
            for (cname, cty) in &member.constructors {
                self.add_constructor(cname, &member.name, cty.clone());
            }
        }
        // 5. One final regeneration per member, now that EVERY member's constructors are in. A
        // mutual recursor is stated over the whole block, so the generations produced while the
        // block was half-registered describe a smaller block than the one that now exists.
        if block.len() > 1 {
            for member in block {
                let name = member.name.clone();
                self.regenerate_auxiliaries(&name);
            }
        }
        self.seal_native_dependencies(dependencies);
        Ok(())
    }

    /// Register a checked inductive block with one shared universe telescope.
    pub fn add_universe_poly_inductives(
        &mut self,
        levels: &[String],
        block: &[MutualInductive],
    ) -> crate::error::KernelResult<()> {
        #[cfg(feature = "shadow")]
        let shadow_entry = crate::dag::mirror::shadow_enter(self);
        #[cfg(feature = "shadow")]
        let candidate = crate::dag::logos_env::candidate_block(self, Some(levels), block);
        let result = self.add_universe_poly_inductives_checked(levels, block);
        #[cfg(feature = "shadow")]
        crate::dag::mirror::shadow(self, &shadow_entry, result.as_ref().err(), || candidate);
        result
    }

    fn add_universe_poly_inductives_checked(
        &mut self,
        levels: &[String],
        block: &[MutualInductive],
    ) -> crate::error::KernelResult<()> {
        if let Some(erased) = erased_block(block) {
            return self.add_universe_poly_inductives_checked(levels, &erased);
        }
        let mut names = std::collections::HashSet::new();
        for member in block {
            Self::check_level_telescope("inductive", &member.name, levels, [&member.sort])?;
            for (name, ty) in std::iter::once((&member.name, &member.sort))
                .chain(member.constructors.iter().map(|(n, t)| (n, t)))
            {
                if !names.insert(name) {
                    return Err(crate::error::KernelError::CertificationError(format!(
                        "duplicate declaration '{name}' in polymorphic inductive block"
                    )));
                }
                Self::check_level_telescope("inductive block member", name, levels, [ty])?;
            }
        }
        self.add_inductive_block(block, Some(levels))
    }

    fn check_block_level_instances(
        term: &Term,
        block: &[&str],
        parameters: &[String],
    ) -> crate::error::KernelResult<()> {
        let mut pending = vec![term];
        while let Some(term) = pending.pop() {
            match term {
                Term::Global(name) if block.contains(&name.as_str()) && !parameters.is_empty() => {
                    return Err(crate::error::KernelError::InvalidDeclaration(format!(
                        "recursive reference '{name}' omits the block's universe arguments"
                    )))
                }
                Term::Const { name, levels } if block.contains(&name.as_str()) => {
                    if levels.len() != parameters.len()
                        || levels.iter().zip(parameters).any(|(level, parameter)| {
                            !level.equiv(&Universe::Var(parameter.clone()))
                        })
                    {
                        return Err(crate::error::KernelError::InvalidDeclaration(format!(
                            "recursive reference '{name}' changes the block's universe arguments"
                        )));
                    }
                }
                Term::Pi {
                    param_type,
                    body_type,
                    ..
                } => {
                    pending.push(param_type);
                    pending.push(body_type);
                }
                Term::Lambda {
                    param_type, body, ..
                } => {
                    pending.push(param_type);
                    pending.push(body);
                }
                Term::App(f, a) => {
                    pending.push(f);
                    pending.push(a);
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    pending.push(discriminant);
                    pending.push(motive);
                    pending.extend(cases);
                }
                Term::Fix { body, .. } => pending.push(body),
                Term::MutualFix { defs, .. } => pending.extend(defs.iter().map(|(_, b)| b)),
                Term::Let {
                    ty, value, body, ..
                } => {
                    pending.push(ty);
                    pending.push(value);
                    pending.push(body);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A non-indexed, fieldless singleton at any universe. Recursive and indexed
    /// single-constructor families do not have the unit eta rule.
    pub fn unit_constructor(&self, inductive: &str) -> Option<&str> {
        if !self.is_inductive(inductive)
            || self
                .mutual_block_of(inductive)
                .is_some_and(|b| b.len() != 1)
        {
            return None;
        }
        let parameters = self.inductive_num_params(inductive);
        let mut arity = self.get_global(inductive)?;
        let mut count = 0;
        while let Term::Pi { body_type, .. } = arity {
            count += 1;
            arity = body_type;
        }
        if count != parameters || !matches!(arity, Term::Sort(_)) {
            return None;
        }
        let constructors = self.get_constructors(inductive);
        if constructors.len() != 1 {
            return None;
        }
        let mut ty = constructors[0].1;
        let mut count = 0;
        while let Term::Pi { body_type, .. } = ty {
            count += 1;
            ty = body_type;
        }
        (count == parameters).then_some(constructors[0].0)
    }

    /// The unique constructor of a K-like proposition: no stored fields beyond
    /// the uniform parameters. Its indices still need conversion at each use.
    pub fn k_constructor(&self, inductive: &str) -> Option<&str> {
        if !self.is_inductive(inductive) {
            return None;
        }
        if self
            .mutual_block_of(inductive)
            .is_some_and(|block| block.len() != 1)
        {
            return None;
        }
        let mut result = self.get_global(inductive)?;
        while let Term::Pi { body_type, .. } = result {
            result = body_type;
        }
        if !matches!(result, Term::Sort(level) if *level != Universe::SProp && level.equiv(&Universe::Prop))
        {
            return None;
        }
        let ctors = self.get_constructors(inductive);
        if ctors.len() != 1 {
            return None;
        }
        let mut ty = ctors[0].1;
        let mut count = 0;
        while let Term::Pi { body_type, .. } = ty {
            count += 1;
            ty = body_type;
        }
        (count == self.inductive_num_params(inductive)).then_some(ctors[0].0)
    }

    /// The mutual block `name` belongs to (the full ordered member list), or `None`
    /// if `name` is a standalone inductive. Used by the recursor derivation to give
    /// every block member a motive and route sibling recursion.
    pub fn mutual_block_of(&self, name: &str) -> Option<&[String]> {
        self.mutual_blocks.get(name).map(|v| v.as_slice())
    }

    /// Register a NESTED inductive (`RTree := rnode : TList RTree → RTree`) by compiling
    /// it — via the UNTRUSTED [`inductive_compile`](crate::inductive_compile) front-end —
    /// to a mutual block plus conversion isos, then registering the block and CHECKING
    /// every iso through the trusted kernel. A mis-compiled sibling is caught by mutual
    /// positivity; a mis-typed iso is caught here (its inferred type must match its
    /// declared conversion type). Soundness rests entirely on those trusted checks — the
    /// compiler adds no trusted code, exactly as Lean lowers nested inductives to mutual.
    pub fn add_nested_inductive(
        &mut self,
        decl: &crate::inductive_compile::NestedDecl,
    ) -> crate::error::KernelResult<crate::inductive_compile::NestedInfo> {
        if !crate::annotation::within_erased_call()
            && std::iter::once(&decl.sort)
                .chain(decl.constructors.iter().map(|(_, t)| t))
                .any(Term::has_annotations)
        {
            return self.add_nested_inductive(&crate::inductive_compile::NestedDecl {
                name: decl.name.clone(),
                sort: decl.sort.strip_annotations(),
                constructors: decl
                    .constructors
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.strip_annotations()))
                    .collect(),
            });
        }
        let compiled = crate::inductive_compile::compile_nested(self, decl)?;

        // TRANSACTIONAL: build the whole declaration on a SCRATCH context and commit only once
        // every part of it has checked — the same shape `add_mutual_inductives` already uses.
        //
        // Registering the block first and checking the isos afterwards meant an iso failure
        // returned an error with the block and its siblings ALREADY in the context: a caller that
        // handles the error and carries on is then reasoning in an environment holding half a
        // declaration — the inductives present, the conversions missing. `compile_nested` refuses
        // eight shapes fail-closed and each promises nothing is registered; this is the one path
        // that could break that promise.
        let mut scratch = self.clone();

        // The mutual block is checked by the trusted mutual machinery (block positivity).
        scratch.add_mutual_inductives(&compiled.block)?;
        // Each iso is KERNEL-CHECKED before it is trusted: infer its type and require it
        // be the declared conversion type. A wrong iso is rejected here.
        for (name, ty, body) in &compiled.isos {
            let inferred = crate::infer_type(&scratch, body)?;
            if !crate::is_subtype(&scratch, &inferred, ty)
                || !crate::is_subtype(&scratch, ty, &inferred)
            {
                return Err(crate::error::KernelError::CertificationError(format!(
                    "nested-compile: iso '{name}' inferred type {inferred} ≠ declared {ty}"
                )));
            }
            scratch.add_definition(name.clone(), ty.clone(), body.clone());
        }

        *self = scratch;
        Ok(crate::inductive_compile::NestedInfo {
            siblings: compiled.siblings,
            isos: compiled.iso_names,
        })
    }

    /// Register a theorem as a hint for the auto tactic.
    ///
    /// Hints are theorems that auto will try to apply when decision
    /// procedures fail. This allows auto to "learn" from proven theorems.
    pub fn add_hint(&mut self, name: &str) {
        self.register_hint(name, 0);
    }

    /// A `local` hint — reverted at [`Context::pop_scope`]. See [`Context::push_scope`].
    pub fn add_local_hint(&mut self, name: &str) {
        let owner = self.scope_depth;
        self.register_hint(name, owner);
    }

    fn register_hint(&mut self, name: &str, owner: usize) {
        if !self.hints.contains(&name.to_string()) {
            Arc::make_mut(&mut self.hints).push(name.to_string());
            Arc::make_mut(&mut self.hint_scopes).push(owner);
            debug_assert_eq!(self.hints.len(), self.hint_scopes.len());
        }
        // A hint whose statement is an EQUATION is also a rewrite rule. `hints` is the one
        // extensible lemma database the surface can reach, so the `@[simp]` effect has to be
        // reachable through it as well as through the explicit [`Context::add_simp_lemma`] door;
        // a hint that is not an equation contributes nothing here and stays an `auto` hint alone.
        let _ = self.register_simp(name, owner);
    }

    /// Get all registered hints.
    ///
    /// Returns the names of theorems registered as hints.
    pub fn get_hints(&self) -> &[String] {
        &self.hints
    }

    /// Check if a theorem is registered as a hint.
    pub fn is_hint(&self, name: &str) -> bool {
        self.hints.contains(&name.to_string())
    }

    // ==========================================================================================
    // THE REDUCIBILITY AXIS — `@[reducible]` / `@[irreducible]`, and the door δ asks
    // ==========================================================================================

    /// Declare `name`'s REDUCIBILITY — Lean's `@[reducible]` / `@[irreducible]`.
    ///
    /// The axis is a property of the DECLARATION, not of the reducer, which is why it lives here:
    /// `Transparency::Excluding` takes a predicate, and without a registry every caller had to
    /// invent its own — so two of them would eventually disagree about whether a constant unfolds,
    /// and `def_eq` would answer differently depending on which one asked.
    ///
    /// The kernel itself still unfolds everything (`normalize` is `Transparency::All`), exactly as
    /// Lean's kernel ignores `@[irreducible]`: the attribute constrains the ELABORATOR, and making
    /// it constrain the kernel would let an attribute change what is TRUE.
    pub fn set_reducibility(&mut self, name: &str, r: Reducibility) {
        let owner = self.scope_depth;
        let previous = self.reducibility.get(name).copied();
        if owner > 0 {
            Arc::make_mut(&mut self.reducibility_undo).push((owner, name.to_string(), previous));
        }
        if r == Reducibility::Semireducible {
            Arc::make_mut(&mut self.reducibility).remove(name);
        } else {
            Arc::make_mut(&mut self.reducibility).insert(name.to_string(), r);
        }
    }

    /// `name`'s declared reducibility (the default when nothing was declared).
    pub fn reducibility(&self, name: &str) -> Reducibility {
        self.reducibility.get(name).copied().unwrap_or_default()
    }

    /// Whether `name` is `@[irreducible]` — THE PREDICATE `Transparency::Excluding` wants.
    ///
    /// One lookup in one map, so a reducer may ask it once per δ-candidate without the question
    /// costing more than the unfolding would have.
    pub fn is_irreducible(&self, name: &str) -> bool {
        matches!(self.reducibility.get(name), Some(Reducibility::Irreducible))
    }

    /// Conservative eligibility for using full-transparency conversion as an
    /// elaborator observation. A large attribute registry merely skips the
    /// optimization; no declaration or normalizer policy changes.
    pub(crate) fn elaborator_all_transparent(&self) -> bool {
        self.reducibility.len() <= 4096
            && self
                .reducibility
                .values()
                .all(|r| *r != Reducibility::Irreducible)
    }

    /// Undo the reducibility edits made inside the scope now closing.
    fn revert_reducibility(&mut self, closing: usize) {
        if !self.reducibility_undo.iter().any(|(d, _, _)| *d == closing) {
            return;
        }
        let undo = Arc::make_mut(&mut self.reducibility_undo);
        let mut restore: Vec<(String, Option<Reducibility>)> = Vec::new();
        while let Some((d, _, _)) = undo.last() {
            if *d != closing {
                break;
            }
            let (_, name, previous) = undo.pop().expect("just peeked");
            restore.push((name, previous));
        }
        let map = Arc::make_mut(&mut self.reducibility);
        for (name, previous) in restore {
            match previous {
                Some(r) => {
                    map.insert(name, r);
                }
                None => {
                    map.remove(&name);
                }
            }
        }
    }

    // ==========================================================================================
    // THE SIMP-LEMMA REGISTRY — `@[simp]`, and the ONE door a simplifier consults
    // ==========================================================================================

    /// Register `name` as a `@[simp]` lemma — Lean's `@[simp] theorem`.
    ///
    /// The lemma's DECLARED STATEMENT is compiled, once, into the shape a rewriter applies:
    /// `Π(x̄). lhs = rhs` becomes the rule `lhs ⟶ rhs` under the binders `x̄`, keyed by `lhs`'s
    /// head symbol. Compiling at registration rather than at every rewrite attempt is the whole
    /// reason this is a registry and not a list of names — a name-list forces every consumer to
    /// re-open the telescope and re-destructure the `Eq` for every candidate at every subterm.
    ///
    /// REFUSED, with a reason rather than a silent no-op, because each of these is a rule that
    /// would make the simplifier wrong or non-terminating rather than merely weak:
    ///
    /// * a statement that is not an equation — there is nothing to rewrite WITH;
    /// * `x = rhs` with a bare bound variable on the left — it matches every term at every
    ///   position, so one such rule rewrites the whole universe;
    /// * `lhs = rhs` where `rhs` mentions a variable `lhs` does not bind — matching cannot
    ///   determine it, so the replacement would contain a free variable;
    /// * `lhs = lhs` — an identity rule that fires for ever without changing anything.
    ///
    /// Better than Lean here in one small way: Lean accepts a looping simp lemma and you find
    /// out when `simp` diverges. The last two shapes are refused at the door with the lemma named.
    pub fn add_simp_lemma(&mut self, name: &str) -> crate::error::KernelResult<()> {
        self.register_simp(name, 0)
    }

    /// A `local`/`scoped` `@[simp]` lemma — reverted at [`Context::pop_scope`].
    pub fn add_local_simp_lemma(&mut self, name: &str) -> crate::error::KernelResult<()> {
        let owner = self.scope_depth;
        self.register_simp(name, owner)
    }

    /// Compile `name`'s declared statement into a rewrite rule owned by scope `owner`.
    fn register_simp(&mut self, name: &str, owner: usize) -> crate::error::KernelResult<()> {
        if self.simp_rules.iter().any(|r| r.name == name) {
            return Ok(());
        }
        let stmt = self
            .get_global(name)
            .cloned()
            .ok_or_else(|| crate::error::KernelError::UnboundVariable(name.to_string()))?;
        let rule = SimpRule::compile(name, &stmt)?;
        let head = rule.head.clone();
        let index = self.simp_rules.len();
        Arc::make_mut(&mut self.simp_rules).push(rule);
        Arc::make_mut(&mut self.simp_scopes).push(owner);
        Arc::make_mut(&mut self.simp_by_head)
            .entry(head)
            .or_default()
            .push(index);
        debug_assert_eq!(self.simp_rules.len(), self.simp_scopes.len());
        Ok(())
    }

    /// Every registered rewrite rule, in registration order.
    pub fn simp_rules(&self) -> &[SimpRule] {
        &self.simp_rules
    }

    /// The rules whose left-hand side has head symbol `head` — THE DISPATCH DOOR.
    ///
    /// A rewriter walking a term asks this once per subterm and gets only the rules that can
    /// possibly match it. Never scan [`Context::simp_rules`] linearly from a hot path.
    pub fn simp_rules_for(&self, head: &str) -> impl Iterator<Item = &SimpRule> {
        self.simp_by_head
            .get(head)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(move |i| &self.simp_rules[*i])
    }

    /// Whether `name` is registered as a `@[simp]` lemma.
    pub fn is_simp_lemma(&self, name: &str) -> bool {
        self.simp_rules.iter().any(|r| r.name == name)
    }

    /// Rebuild the head index after a scope close removed rules (indices shift).
    fn reindex_simp(&mut self) {
        let mut by_head: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in self.simp_rules.iter().enumerate() {
            by_head.entry(r.head.clone()).or_default().push(i);
        }
        self.simp_by_head = Arc::new(by_head);
    }
}

/// Lean's REDUCIBILITY axis, declared per constant and read by the elaborator's δ step.
///
/// Three settings and not two, because `@[reducible]` is not merely "the default": instance
/// resolution unfolds a reducible definition and stops at a semireducible one, so collapsing the
/// two changes which instances are found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reducibility {
    /// `@[reducible]` — an abbreviation, unfolded even where the elaborator is being careful.
    Reducible,
    /// The default: unfolded at ordinary transparency.
    #[default]
    Semireducible,
    /// `@[irreducible]` — the elaborator never unfolds it, and `secret = 5` is not `rfl`. The
    /// body is KEPT: this is opacity, not an axiom, and the difference is that a later
    /// `unfold`/`decide` can still reach the definition.
    Irreducible,
}

/// A `@[simp]` lemma COMPILED to the form a rewriter applies: `∀x̄. lhs ⟶ rhs`.
///
/// The binders are the lemma's universally quantified variables in order; `lhs` and `rhs`
/// mention them as [`Term::Var`]. `head` is `lhs`'s head symbol and is the registry's dispatch
/// key — see [`Context::simp_rules_for`].
#[derive(Clone, Debug, PartialEq)]
pub struct SimpRule {
    /// The lemma's name, so a rewrite can say which lemma fired.
    pub name: String,
    /// Universally quantified variables, outermost first, with their types.
    pub binders: Vec<(String, Term)>,
    /// The pattern to match.
    pub lhs: Term,
    /// What to replace a match with.
    pub rhs: Term,
    /// `lhs`'s head symbol — the dispatch key.
    pub head: String,
}

impl SimpRule {
    /// Compile a declared statement `Π(x̄). Eq A lhs rhs` into a rule, or say why it is not one.
    pub fn compile(name: &str, stmt: &Term) -> crate::error::KernelResult<SimpRule> {
        let mut binders: Vec<(String, Term)> = Vec::new();
        let mut cur = stmt;
        while let Term::Pi {
            param,
            param_type,
            body_type,
        } = cur
        {
            // A Π whose body does not mention the binder is an IMPLICATION, not a quantifier:
            // its premise would have to be discharged before the rule could fire, which is a
            // conditional rewrite and not this registry's shape.
            binders.push((param.clone(), (**param_type).clone()));
            cur = body_type;
        }
        let refuse = |why: String| {
            Err(crate::error::KernelError::CertificationError(format!(
                "`{name}` cannot be a simp lemma: {why}"
            )))
        };
        let (lhs, rhs) = match eq_sides(cur) {
            Some(pair) => pair,
            None => return refuse(format!("its statement is not an equation, it is `{cur}`")),
        };
        let head =
            match spine_head(&lhs) {
                Some(h) => h,
                None => return refuse(
                    "its left-hand side is a bare variable, so the rule would match every term \
                     at every position"
                        .to_string(),
                ),
            };
        if lhs == rhs {
            return refuse("it rewrites a term to itself, which never terminates".to_string());
        }
        let bound: Vec<&str> = binders.iter().map(|(n, _)| n.as_str()).collect();
        let (mut lhs_vars, mut rhs_vars) = (Vec::new(), Vec::new());
        free_vars_among(&lhs, &bound, &mut lhs_vars);
        free_vars_among(&rhs, &bound, &mut rhs_vars);
        if let Some(missing) = rhs_vars.iter().find(|v| !lhs_vars.contains(v)) {
            return refuse(format!(
                "its right-hand side mentions `{missing}`, which its left-hand side does not \
                 bind — matching could never determine it"
            ));
        }
        Ok(SimpRule {
            name: name.to_string(),
            binders,
            lhs,
            rhs,
            head,
        })
    }

    /// Try to rewrite `term` with this rule AT THE TOP LEVEL — first-order matching of `lhs`
    /// against `term`, then `rhs` under the resulting substitution. `None` means the rule does
    /// not apply; the caller descends into subterms itself.
    ///
    /// This lives with the rule rather than with the rewriter for one reason: matching has to
    /// agree with what [`SimpRule::compile`] refused. A rewriter that wrote its own matcher could
    /// bind a pattern variable twice to two different terms, or substitute a variable the rule
    /// never bound — and both are silent wrong answers rather than failures.
    pub fn rewrite(&self, term: &Term) -> Option<Term> {
        let pattern_vars: Vec<&str> = self.binders.iter().map(|(n, _)| n.as_str()).collect();
        let mut subst: HashMap<String, Term> = HashMap::new();
        if !match_pattern(&self.lhs, term, &pattern_vars, &mut Vec::new(), &mut subst) {
            return None;
        }
        Some(substitute_vars(&self.rhs, &subst, &mut Vec::new()))
    }

    /// The instance of the rule's variables, in binder order, that makes `lhs` read as `term` at
    /// the top level; `None` when the rule does not match there or leaves a variable undetermined.
    pub fn matching(&self, term: &Term) -> Option<Vec<Term>> {
        let pattern_vars: Vec<&str> = self.binders.iter().map(|(n, _)| n.as_str()).collect();
        let mut subst: HashMap<String, Term> = HashMap::new();
        if !match_pattern(&self.lhs, term, &pattern_vars, &mut Vec::new(), &mut subst) {
            return None;
        }
        self.binders
            .iter()
            .map(|(n, _)| subst.get(n).cloned())
            .collect()
    }
}

/// First-order matching of `pattern` against `term`.
///
/// A [`Term::Var`] named in `vars` is a PATTERN VARIABLE and binds to whatever stands opposite
/// it — unless it is shadowed by a binder inside the pattern, in which case it is an ordinary
/// bound variable and must match literally. A variable that appears twice must bind to the SAME
/// term both times (`f x x = x` may not fire on `f a b`).
fn match_pattern(
    pattern: &Term,
    term: &Term,
    vars: &[&str],
    shadowed: &mut Vec<String>,
    subst: &mut HashMap<String, Term>,
) -> bool {
    if let Term::Var(n) = pattern {
        if vars.contains(&n.as_str()) && !shadowed.contains(n) {
            return match subst.get(n) {
                Some(bound) => bound == term,
                None => {
                    subst.insert(n.clone(), term.clone());
                    true
                }
            };
        }
    }
    match (pattern, term) {
        (Term::Var(a), Term::Var(b)) => a == b,
        (Term::Global(a), Term::Global(b)) => a == b,
        (Term::Sort(a), Term::Sort(b)) => a == b,
        (Term::Lit(a), Term::Lit(b)) => a == b,
        (Term::Hole, Term::Hole) => true,
        (
            Term::Const {
                name: a,
                levels: la,
            },
            Term::Const {
                name: b,
                levels: lb,
            },
        ) => a == b && la == lb,
        (Term::App(pf, pa), Term::App(tf, ta)) => {
            match_pattern(pf, tf, vars, shadowed, subst)
                && match_pattern(pa, ta, vars, shadowed, subst)
        }
        (
            Term::Pi {
                param: pp,
                param_type: pt,
                body_type: pb,
            },
            Term::Pi {
                param: tp,
                param_type: tt,
                body_type: tb,
            },
        ) => {
            pp == tp
                && match_pattern(pt, tt, vars, shadowed, subst)
                && match_under(pp, pb, tb, vars, shadowed, subst)
        }
        (
            Term::Lambda {
                param: pp,
                param_type: pt,
                body: pb,
            },
            Term::Lambda {
                param: tp,
                param_type: tt,
                body: tb,
            },
        ) => {
            pp == tp
                && match_pattern(pt, tt, vars, shadowed, subst)
                && match_under(pp, pb, tb, vars, shadowed, subst)
        }
        (Term::Fix { name: pn, body: pb }, Term::Fix { name: tn, body: tb }) => {
            pn == tn && match_under(pn, pb, tb, vars, shadowed, subst)
        }
        (
            Term::Match {
                discriminant: pd,
                motive: pm,
                cases: pc,
            },
            Term::Match {
                discriminant: td,
                motive: tm,
                cases: tc,
            },
        ) => {
            pc.len() == tc.len()
                && match_pattern(pd, td, vars, shadowed, subst)
                && match_pattern(pm, tm, vars, shadowed, subst)
                && pc
                    .iter()
                    .zip(tc.iter())
                    .all(|(a, b)| match_pattern(a, b, vars, shadowed, subst))
        }
        _ => false,
    }
}

/// Match two bodies under a binder that SHADOWS `name` for the length of the descent.
fn match_under(
    name: &str,
    pattern: &Term,
    term: &Term,
    vars: &[&str],
    shadowed: &mut Vec<String>,
    subst: &mut HashMap<String, Term>,
) -> bool {
    shadowed.push(name.to_string());
    let ok = match_pattern(pattern, term, vars, shadowed, subst);
    shadowed.pop();
    ok
}

/// Replace every free occurrence of a substituted variable in `t`. A binder of the same name
/// shadows the substitution for its body — without that, rewriting `f x` to `λx. x` inside a
/// term that already binds `x` would capture it.
fn substitute_vars(t: &Term, subst: &HashMap<String, Term>, shadowed: &mut Vec<String>) -> Term {
    match t {
        Term::Var(n) => match subst.get(n) {
            Some(v) if !shadowed.contains(n) => v.clone(),
            _ => t.clone(),
        },
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => {
            t.clone()
        }
        Term::App(f, a) => Term::App(
            Box::new(substitute_vars(f, subst, shadowed)),
            Box::new(substitute_vars(a, subst, shadowed)),
        ),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let param_type = Box::new(substitute_vars(param_type, subst, shadowed));
            shadowed.push(param.clone());
            let body_type = Box::new(substitute_vars(body_type, subst, shadowed));
            shadowed.pop();
            Term::Pi {
                param: param.clone(),
                param_type,
                body_type,
            }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let param_type = Box::new(substitute_vars(param_type, subst, shadowed));
            shadowed.push(param.clone());
            let body = Box::new(substitute_vars(body, subst, shadowed));
            shadowed.pop();
            Term::Lambda {
                param: param.clone(),
                param_type,
                body,
            }
        }
        Term::Fix { name, body } => {
            shadowed.push(name.clone());
            let body = Box::new(substitute_vars(body, subst, shadowed));
            shadowed.pop();
            Term::Fix {
                name: name.clone(),
                body,
            }
        }
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| {
                    shadowed.push(n.clone());
                    let b = substitute_vars(b, subst, shadowed);
                    shadowed.pop();
                    (n.clone(), b)
                })
                .collect(),
            index: *index,
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let ty = Box::new(substitute_vars(ty, subst, shadowed));
            let value = Box::new(substitute_vars(value, subst, shadowed));
            shadowed.push(name.clone());
            let body = Box::new(substitute_vars(body, subst, shadowed));
            shadowed.pop();
            Term::Let {
                name: name.clone(),
                ty,
                value,
                body,
            }
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(substitute_vars(discriminant, subst, shadowed)),
            motive: Box::new(substitute_vars(motive, subst, shadowed)),
            cases: cases
                .iter()
                .map(|c| substitute_vars(c, subst, shadowed))
                .collect(),
        },
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(substitute_vars(term, subst, shadowed)),
        },
    }
}

/// The two sides of an `Eq A lhs rhs` application, if `t` is one.
fn eq_sides(t: &Term) -> Option<(Term, Term)> {
    let Term::App(f1, rhs) = t else { return None };
    let Term::App(f2, lhs) = f1.as_ref() else {
        return None;
    };
    let Term::App(f3, _carrier) = f2.as_ref() else {
        return None;
    };
    match f3.as_ref() {
        Term::Global(n) if n == "Eq" => Some(((**lhs).clone(), (**rhs).clone())),
        _ => None,
    }
}

/// The head symbol of an application spine, if it is a global or a constant.
fn spine_head(t: &Term) -> Option<String> {
    let mut cur = t;
    while let Term::App(f, _) = cur {
        cur = f;
    }
    match cur {
        Term::Global(n) => Some(n.clone()),
        Term::Const { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Every GLOBAL-form name `t` mentions, in traversal order, duplicates included — BORROWED from
/// the term, so a body with a thousand occurrences costs a thousand pointer copies and not a
/// thousand heap allocations. Owning is the caller's decision, made once, after deduplication.
///
/// `Global` and `Const` both denote an environment constant and both make `t` depend on it, so
/// both count — the δ-graph is drawn over names, not over reference spellings. Binders are
/// irrelevant here on purpose: a binder can shadow a `Var`, never a `Global`.
/// Whether two stored terms carry the same meaning for the stable-meaning gate: identical up to
/// the names of bound variables. A redeclaration that only renames a binder changes nothing.
fn same_meaning(a: &Term, b: &Term) -> bool {
    a == b || crate::type_checker::alpha_equivalent(a, b)
}

fn same_meaning_opt(a: Option<&Term>, b: Option<&Term>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_meaning(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn collect_global_mentions<'t>(t: &'t Term, acc: &mut Vec<&'t str>) {
    // ITERATIVE, with its own worklist. This runs on EVERY definition body that enters the
    // context, and a recursive walk would make the DEPTH of a term a limit on what may be
    // declared — a 20 000-rung δ-chain's last body is shallow, but a machine-generated proof term
    // is not, and aborting the process is not a refusal a caller can handle.
    let mut stack: Vec<&'t Term> = vec![t];
    while let Some(cur) = stack.pop() {
        match cur {
            Term::Global(n) => acc.push(n.as_str()),
            Term::Const { name, .. } => acc.push(name.as_str()),
            Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
            Term::App(f, a) => {
                stack.push(f);
                stack.push(a);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                stack.push(param_type);
                stack.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                stack.push(param_type);
                stack.push(body);
            }
            Term::Let {
                ty, value, body, ..
            } => {
                stack.push(ty);
                stack.push(value);
                stack.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                stack.push(discriminant);
                stack.push(motive);
                stack.extend(cases.iter());
            }
            Term::Fix { body, .. } => stack.push(body),
            Term::MutualFix { defs, .. } => stack.extend(defs.iter().map(|(_, b)| b)),
            Term::Annotated { term, .. } => stack.push(term),
        }
    }
}

/// What a registered global's stored terms ELIMINATE: the head of the type each of its `match`es
/// discriminates — its motive's LAST λ-binder, the major premise after any indices — and whether
/// some `match` hides that behind a motive that is not a λ.
#[derive(Debug, Clone, Default)]
struct Eliminations {
    heads: Vec<String>,
    opaque: bool,
}

/// Add what `t` eliminates to `acc`. Iterative, for the same reason as
/// [`collect_global_mentions`].
fn collect_eliminations(t: &Term, acc: &mut Eliminations) {
    let mut stack: Vec<&Term> = vec![t];
    while let Some(cur) = stack.pop() {
        match cur {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::App(f, a) => {
                stack.push(f);
                stack.push(a);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                stack.push(param_type);
                stack.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                stack.push(param_type);
                stack.push(body);
            }
            Term::Let {
                ty, value, body, ..
            } => {
                stack.push(ty);
                stack.push(value);
                stack.push(body);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let mut major: Option<&Term> = None;
                let mut inner: &Term = motive;
                while let Term::Lambda {
                    param_type, body, ..
                } = inner
                {
                    major = Some(param_type);
                    inner = body;
                }
                match major {
                    None => acc.opaque = true,
                    Some(mut head) => {
                        while let Term::App(f, _) = head {
                            head = f;
                        }
                        match head {
                            Term::Global(name) | Term::Const { name, .. } => {
                                if !acc.heads.contains(name) {
                                    acc.heads.push(name.clone());
                                }
                            }
                            // A bound type variable is not an inductive a declaration can extend.
                            Term::Var(_) => {}
                            _ => acc.opaque = true,
                        }
                    }
                }
                stack.push(discriminant);
                stack.push(motive);
                stack.extend(cases.iter());
            }
            Term::Fix { body, .. } => stack.push(body),
            Term::MutualFix { defs, .. } => stack.extend(defs.iter().map(|(_, b)| b)),
            Term::Annotated { term, .. } => stack.push(term),
        }
    }
}

/// Collect the occurrences of `allowed` names appearing as free [`Term::Var`]s in `t`.
fn free_vars_among(t: &Term, allowed: &[&str], acc: &mut Vec<String>) {
    match t {
        Term::Var(n) => {
            if allowed.contains(&n.as_str()) && !acc.contains(n) {
                acc.push(n.clone());
            }
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            free_vars_among(param_type, allowed, acc);
            free_vars_among(body_type, allowed, acc);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            free_vars_among(param_type, allowed, acc);
            free_vars_among(body, allowed, acc);
        }
        Term::App(f, a) => {
            free_vars_among(f, allowed, acc);
            free_vars_among(a, allowed, acc);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            free_vars_among(discriminant, allowed, acc);
            free_vars_among(motive, allowed, acc);
            for c in cases {
                free_vars_among(c, allowed, acc);
            }
        }
        Term::Fix { body, .. } => free_vars_among(body, allowed, acc),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                free_vars_among(b, allowed, acc);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            free_vars_among(ty, allowed, acc);
            free_vars_among(value, allowed, acc);
            free_vars_among(body, allowed, acc);
        }
        _ => {}
    }
}

/// Drop from `items` (and its index-aligned `owners`) every entry owned by the scope `closing`.
fn retain_scoped1<T: Clone>(items: &mut Arc<Vec<T>>, owners: &mut Arc<Vec<usize>>, closing: usize) {
    if !owners.iter().any(|d| *d == closing) {
        return;
    }
    let keep: Vec<bool> = owners.iter().map(|d| *d != closing).collect();
    let items = Arc::make_mut(items);
    let owners = Arc::make_mut(owners);
    let mut i = 0;
    items.retain(|_| {
        let k = keep.get(i).copied().unwrap_or(true);
        i += 1;
        k
    });
    owners.retain(|d| *d != closing);
    debug_assert_eq!(items.len(), owners.len());
}

/// The two-side-car version: `items` with BOTH `aux` and `owners` index-aligned to it.
fn retain_scoped2<T: Clone, A: Clone>(
    items: &mut Arc<Vec<T>>,
    aux: &mut Arc<Vec<A>>,
    owners: &mut Arc<Vec<usize>>,
    closing: usize,
) {
    if !owners.iter().any(|d| *d == closing) {
        return;
    }
    let keep: Vec<bool> = owners.iter().map(|d| *d != closing).collect();
    let mut i = 0;
    Arc::make_mut(items).retain(|_| {
        let k = keep.get(i).copied().unwrap_or(true);
        i += 1;
        k
    });
    let mut j = 0;
    Arc::make_mut(aux).retain(|_| {
        let k = keep.get(j).copied().unwrap_or(true);
        j += 1;
        k
    });
    Arc::make_mut(owners).retain(|d| *d != closing);
    debug_assert_eq!(items.len(), owners.len());
    debug_assert_eq!(aux.len(), owners.len());
}

/// The ONE parameter-count rule for a MUTUAL block, stated where registration can enforce it.
///
/// A mutual block shares a single uniform parameter telescope: `Tree A`/`Forest A` are one
/// declaration over one `A`. Two things are refused, and the second used to be laundered rather
/// than diagnosed:
///
/// * a member whose recorded `num_params` EXCEEDS its own arity — nonsense that
///   `derive_mutual_recursor`'s `.min(arity)` silently clamped, so two members could agree on a
///   number while disagreeing about what their parameters mean; and
/// * members that disagree on the count.
///
/// Returns the block's shared parameter count. An EMPTY block has none and is vacuously fine.
///
/// Better than Lean's diagnostic in one respect: the message lists every member and its own
/// count, so the odd one out is visible without re-reading the declaration.
pub fn reconcile_block_params(block: &[MutualInductive]) -> crate::error::KernelResult<usize> {
    let counts: Vec<_> = block
        .iter()
        .map(|member| {
            (
                member.name.as_str(),
                member.num_params,
                count_leading_pis(&member.sort),
            )
        })
        .collect();
    reconcile_parameter_counts(&counts)
}

/// Shared policy for raw public metadata checks and exposed checked headers.
fn reconcile_parameter_counts(
    counts: &[(&str, usize, usize)],
) -> crate::error::KernelResult<usize> {
    for &(name, params, arity) in counts {
        if params > arity {
            return Err(crate::error::KernelError::CertificationError(format!(
                "mutual block: member '{name}' declares {params} parameter(s) but its arity is only {arity} — \
                 a parameter count above the arity has no telescope to name"
            )));
        }
    }
    let Some(&(_, first, _)) = counts.first() else {
        return Ok(0);
    };
    if counts.iter().any(|&(_, n, _)| n != first) {
        let listing = counts
            .iter()
            .map(|(n, p, a)| format!("{n}: {p} of {a}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(crate::error::KernelError::CertificationError(format!(
            "mutual block members disagree on parameter count ({listing}) — a mutual block \
             shares ONE uniform parameter telescope, so a block that disagrees has no induction \
             principle and must be refused at registration"
        )));
    }
    Ok(first)
}

/// Infer which of a class's parameters are OUTPUTS — Lean's `outParam`, recovered from the
/// shape of its methods rather than from an annotation.
///
/// The reading is functional-dependency: a parameter a method CONSUMES is something a call
/// site supplies, so resolution can match on it; a parameter that only ever appears in a
/// method's RESULT is something the instance PRODUCES, so matching on it would let the
/// surrounding context's expectation silently choose a different instance. `HAdd α β γ`'s
/// `hAdd : α → β → γ` is the canonical case: `α`/`β` are consumed, `γ` is produced.
///
/// Consumption is read off the Π-telescopes of the field types: every free name in any
/// DOMAIN is consumed, as is every name a later parameter's own type depends on. Nothing in
/// a codomain is.
///
/// The floor: if EVERY parameter would be an output, none is. A class with no inputs has
/// nothing for resolution to key on, so the first registered entry would answer every goal —
/// which is why `Inhabited α` (`default : α`, a carrier that appears only as a result) must
/// keep matching on its carrier. That rule is what separates a genuine output position from a
/// class whose methods simply return their carrier.
fn infer_out_params(params: &[(&str, Term)], fields: &[(&str, Term)]) -> Vec<usize> {
    let mut consumed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (_, ty) in params {
        collect_free_names(ty, &mut Vec::new(), &mut consumed);
    }
    for (_, ty) in fields {
        collect_domain_names(ty, &mut Vec::new(), &mut consumed);
    }
    let outputs: Vec<usize> = params
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| !consumed.contains(*p))
        .map(|(i, _)| i)
        .collect();
    if outputs.len() == params.len() {
        Vec::new()
    } else {
        outputs
    }
}

/// Collect every free variable name appearing in a `Π`-DOMAIN of `ty` (at any depth of its
/// telescope). The final codomain contributes nothing — that is the whole distinction between
/// a consumed parameter and a produced one.
fn collect_domain_names(
    ty: &Term,
    bound: &mut Vec<String>,
    acc: &mut std::collections::HashSet<String>,
) {
    if let Term::Pi {
        param,
        param_type,
        body_type,
    } = ty
    {
        collect_free_names(param_type, bound, acc);
        bound.push(param.clone());
        collect_domain_names(body_type, bound, acc);
        bound.pop();
    }
}

/// Collect every free `Term::Var` name in `t`, skipping the ones `bound` shadows.
fn collect_free_names(
    t: &Term,
    bound: &mut Vec<String>,
    acc: &mut std::collections::HashSet<String>,
) {
    match t {
        Term::Var(n) => {
            if !bound.contains(n) {
                acc.insert(n.clone());
            }
        }
        Term::App(f, a) => {
            collect_free_names(f, bound, acc);
            collect_free_names(a, bound, acc);
        }
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            collect_free_names(param_type, bound, acc);
            bound.push(param.clone());
            collect_free_names(body_type, bound, acc);
            bound.pop();
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            collect_free_names(param_type, bound, acc);
            bound.push(param.clone());
            collect_free_names(body, bound, acc);
            bound.pop();
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_free_names(discriminant, bound, acc);
            collect_free_names(motive, bound, acc);
            for c in cases {
                collect_free_names(c, bound, acc);
            }
        }
        Term::Fix { name, body } => {
            bound.push(name.clone());
            collect_free_names(body, bound, acc);
            bound.pop();
        }
        Term::MutualFix { defs, .. } => {
            for (n, b) in defs {
                bound.push(n.clone());
                collect_free_names(b, bound, acc);
                bound.pop();
            }
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            collect_free_names(ty, bound, acc);
            collect_free_names(value, bound, acc);
            bound.push(name.clone());
            collect_free_names(body, bound, acc);
            bound.pop();
        }
        Term::Global(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => {}
        Term::Annotated { term, .. } => collect_free_names(term, bound, acc),
    }
}

/// The length of the longest PREFIX of `arity` positions that constructor type `cty` treats as
/// uniform parameters of `ind` — see [`Context::infer_num_params`] for the rule.
fn ctor_param_prefix(ind: &str, cty: &Term, arity: usize) -> usize {
    // Peel the constructor's own Π telescope, remembering its binder names in order.
    let mut binders: Vec<&str> = Vec::new();
    let mut cur = cty;
    while let Term::Pi {
        param, body_type, ..
    } = cur
    {
        binders.push(param.as_str());
        cur = body_type;
    }
    let Some(result_args) = applied_to(cur, ind) else {
        return 0;
    };
    // A binder position the telescope does not even reach cannot be uniform. `refl : Π(A)(x:A).
    // Eq A x x` has two binders against `Eq`'s arity of three, so its third position is an index
    // — which is exactly right, and is why the cap is on the TELESCOPE, not only on the arity.
    let cap = arity.min(binders.len()).min(result_args.len());
    let mut p = 0;
    while p < cap && matches!(&result_args[p], Term::Var(v) if v == binders[p]) {
        p += 1;
    }
    if p == 0 {
        return 0;
    }
    // Every RECURSIVE occurrence inside an argument type must pass the same binders. Walk the
    // whole telescope's domains, not just the recursive-looking ones: an occurrence can be nested
    // arbitrarily deep (`Π(y:A). R y x → Acc A R y`).
    let mut cur = cty;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        p = p.min(uniform_prefix_of_occurrences(param_type, ind, &binders, p));
        if p == 0 {
            return 0;
        }
        cur = body_type;
    }
    p
}

/// If `t` is the application spine `Global(head) a₀ … aₙ₋₁`, its arguments in order.
fn applied_to<'a>(t: &'a Term, head: &str) -> Option<Vec<&'a Term>> {
    let mut args: Vec<&Term> = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push(a);
        cur = f;
    }
    match cur {
        Term::Global(n) if n == head => {
            args.reverse();
            Some(args)
        }
        _ => None,
    }
}

/// Shrink `limit` to the longest prefix over which EVERY occurrence of `ind` inside `t` passes
/// `binders[i]` at position `i`.
fn uniform_prefix_of_occurrences(t: &Term, ind: &str, binders: &[&str], limit: usize) -> usize {
    let mut acc = limit;
    if let Some(args) = applied_to(t, ind) {
        let mut p = 0;
        while p < acc && p < args.len() && matches!(&args[p], Term::Var(v) if v == binders[p]) {
            p += 1;
        }
        // `p` is already capped by `args.len()`, so a PARTIALLY applied occurrence — which
        // supplies nothing at the positions it omits — collapses the prefix to what it does
        // supply. That is the conservative direction, and the right one.
        acc = p;
        // Descend into the ARGUMENTS only. Walking the spine's function half as well would
        // re-read `I a₀ … aₖ₋₁` as its own shorter occurrence and shrink the prefix to zero for
        // every recursive field.
        for a in args {
            acc = uniform_prefix_of_occurrences(a, ind, binders, acc);
        }
        return acc;
    }
    // Recurse into every subterm — an occurrence can hide under any constructor of `Term`.
    let visit = |sub: &Term, acc: &mut usize| {
        *acc = uniform_prefix_of_occurrences(sub, ind, binders, *acc);
    };
    match t {
        Term::App(f, a) => {
            visit(f, &mut acc);
            visit(a, &mut acc);
        }
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            visit(param_type, &mut acc);
            visit(body_type, &mut acc);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            visit(param_type, &mut acc);
            visit(body, &mut acc);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            visit(discriminant, &mut acc);
            visit(motive, &mut acc);
            for c in cases {
                visit(c, &mut acc);
            }
        }
        Term::Fix { body, .. } => visit(body, &mut acc),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                visit(b, &mut acc);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            visit(ty, &mut acc);
            visit(value, &mut acc);
            visit(body, &mut acc);
        }
        Term::Annotated { term, .. } => visit(term, &mut acc),
        Term::Var(_)
        | Term::Global(_)
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole
        | Term::Const { .. } => {}
    }
    acc
}

/// An inductive's arity as it is declared, with its telescope exposed: every binder domain is kept
/// exactly as written (`outParam`, `semiOutParam`, `optParam`, an abbreviation), because the arity
/// is the family's statement; only a tail that is neither a product nor a sort is handed to
/// `normalize`, which is Lean's `whnf` at each binder of an arity whose telescope hides behind a
/// definition. The caller decides what an unfinished normalization means.
pub(crate) fn exposed_arity<E>(
    ctx: &Context,
    arity: &Term,
    normalize: &mut dyn FnMut(&Context, &Term) -> Result<Term, E>,
) -> Result<Term, E> {
    match arity {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let inner = ctx.extend(param, (**param_type).clone());
            Ok(Term::Pi {
                param: param.clone(),
                param_type: param_type.clone(),
                body_type: Box::new(exposed_arity(&inner, body_type, normalize)?),
            })
        }
        Term::Sort(_) => Ok(arity.clone()),
        hidden => match normalize(ctx, hidden)? {
            exposed @ (Term::Pi { .. } | Term::Sort(_)) => exposed_arity(ctx, &exposed, normalize),
            stuck => Ok(stuck),
        },
    }
}

/// Count the leading `Π`s of a term — an inductive's arity, or a constructor's parameter
/// count.
fn count_leading_pis(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/accessor_tests.rs"]
mod accessor_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/accessor_tests_more.rs"]
mod accessor_tests_more;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/arithmetic_attestation_cache_controls.rs"]
mod arithmetic_attestation_cache_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/checked_opaque_storage_controls.rs"]
mod checked_opaque_storage_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/local_name_controls.rs"]
mod local_name_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/displacement_sharing_controls.rs"]
mod displacement_sharing_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/context/admission_log_controls.rs"]
mod admission_log_controls;
