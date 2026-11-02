//! Hash-consed term storage — the representation a kernel needs to be as fast as Lean's.
//!
//! `Term` is a `Box`-recursive TREE. Every walk over it is native recursion whose depth is
//! the term's depth, every `clone` copies the whole subtree, and a subterm that occurs `n`
//! times is `n` separate trees that get checked `n` times. Lean's `Expr` is the other thing:
//! a maximally-shared DAG behind a structural hash, so constructing an identical subterm
//! twice yields the SAME pointer, `a == b` is usually a pointer compare, and a cache keyed
//! on that identity checks a shared subterm ONCE.
//!
//! This module is that representation, built and measured STANDALONE:
//!
//! * [`TermStore::intern`] turns a `Term` into a [`TermId`] — a `Copy` `u32` handle.
//!   Identical structure interns to an identical id, so `a == b` is ONE integer compare
//!   ([`TermStore::same`]) instead of a tree walk.
//! * Every node carries its [`depth`](TermStore::depth), its unfolded
//!   [`tree_size`](TermStore::tree_size), its O(1) [`structural_hash`](TermStore::structural_hash),
//!   its hash-consed FREE-VARIABLE set, and the set of binder names below it. All are
//!   computed compositionally at construction, so no query re-walks the term.
//! * [`IdCache`] is a DENSE cache keyed on node identity — `Vec<Option<V>>` indexed by the
//!   id, so a lookup is a bounds check and not a hash. [`TermStore::fold`] visits each
//!   DISTINCT node exactly once, which is what "checked once, not once per occurrence" is.
//! * [`TermStore::instantiate`] is SIMULTANEOUS capture-avoiding substitution — Lean's
//!   `instantiateRev` shape: a whole argument vector in one pass, not one substitution per
//!   binder. A node whose free variables do not meet the substitution's domain is returned
//!   UNCHANGED, detected by a 64-bit mask intersection, which is what takes a deep spine
//!   from quadratic to linear.
//! * Nothing here recurses natively. Interning, extraction, folding, substitution and
//!   normalisation are all explicit-stack loops, and the store's own `Drop` is a flat
//!   `Vec` drop — so depth is bounded by the heap, not by 2 MiB of thread stack.
//!
//! ## What this module is NOT
//!
//! [`TermStore::beta_zeta_normalize`] is a MEASUREMENT INSTRUMENT, not a kernel tier. It
//! implements β and ζ and nothing else — no δ, no ι, no η, no `Fix` unfolding, no universe
//! algebra. It is not a replacement for `reduction::normalize`.
//! `beta_delta_equivalent` now uses it behind a conservative conversion gateway:
//! checked closed monomorphic definitions are expanded, unsupported shapes are refused,
//! and only alpha-equal results prove conversion; all other answers use the main kernel.
//!
//! ## The measured case for wiring this in
//!
//! Four numbers, all measured on this box against the kernel as it stands.
//!
//! 1. **The deep-spine corpus case used to ABORT, and no longer does — CLOSED.**
//!    `lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically`
//!    died by SIGABRT — a stack overflow in `infer_type`'s own `App` arm — before it reached its
//!    assertion. `MAX_TERM_DEPTH` is 1600; BISECTED in a child process, `infer_type` died at
//!    **106** rungs of a Π telescope and at **52** rungs of the corpus's own saturated dependent
//!    telescope, so the door was fifteen to thirty-one times too permissive and ADMITTED terms
//!    the kernel then killed the process on. `double_check` died at the SAME 52, because it calls
//!    `infer_type` first — so across that whole band the second kernel, whose entire job is to
//!    disagree with the first, was never consulted. [`deepen_stack`] is now wired into
//!    `infer_type`, `is_subtype`, `normalize_checked_at`, `recheck` and `double_check`, and a
//!    whole-crate run of 1413 tests reports **zero** SIGABRT/SIGSEGV: the case now FAILS AN
//!    ASSERTION, which is a verdict instead of a corpse. (The corpus case's own prose puts the
//!    ceiling at "depth 4000", which is wrong by 77×.)
//!
//!    **One band is still open, and [`deepen_stack`] does not close it.** The door and the sizer
//!    both measure the term they were HANDED. `infer_type`'s `Var` arm is
//!    `ctx.get(name).cloned()`, and `Term`'s `Clone` is derived — so it recurses over the
//!    BINDING's type, a depth neither ever saw. MEASURED in a child process: `Var("x")`, one node
//!    deep, under `x : Nat → … → Nat` at 4000 arrows kills the process BARE and through
//!    [`deepen_stack`], and is answered by [`deepen_stack_with_depth`]. Every arm inherits it, so
//!    the input term's depth is a LOWER BOUND on what a kernel call descends, never the bound.
//! 2. **A β-step on a spine pays FOUR Θ(size) walks, and the corpus's ratio is the square of
//!    that.** ITEMISED AND COUNTED, in units a loaded box cannot move — node visits, by
//!    `W_perf_extra::the_spines_four_walks_are_quadratic_in_node_visits_and_the_owned_step_is_linear`:
//!
//!    | per β-step on a spine | visits at depth 500 | at depth 1500 | ratio |
//!    |---|---:|---:|---:|
//!    | `substitute`'s `free_vars(replacement)` | 499 500 | 4 498 500 | 9.01× |
//!    | `substitute`'s clone of the replacement | 499 500 | 4 498 500 | 9.01× |
//!    | the driver's recursive `Drop` | 501 500 | 4 504 500 | 8.98× |
//!    | Brent's `current == tortoise` (and its saves) | 513 500 | 4 550 366 | 8.86× |
//!    | **all four** | **2 014 000** | **18 051 866** | **8.96×** |
//!    | an OWNED step that moves instead of copying | 2 000 | 6 000 | **3.00×** |
//!
//!    The corpus case asks for `≤ 5.00×` at 3× the depth, and the kernel's own wall clock on this
//!    box reads 6.9× to 12.3× across repeated runs of one binary (40–94 ms at depth 500, 428 ms
//!    to 1.16 s at depth 1500). **Each of the four is quadratic ON ITS OWN**, so removing any
//!    three leaves the ratio where it was — which is the measurement that says no constant factor
//!    reaches it. Three of the four can go inside `type_checker.rs` and `reduction.rs`:
//!    `free_vars(replacement)` is only needed when a BINDER in the body could capture, and on a
//!    spine the body is `Var("x")` and has none; the clone and the `Drop` both go if the reducer
//!    OWNS its term instead of borrowing it. **The fourth cannot.** Brent's cycle detection needs
//!    an equality test on the WHOLE term, and a `Box`-recursive tree has no O(1) one — every
//!    cheap surrogate (a bounded prefix compare, a depth-limited compare, a periodic compare) is
//!    either unsound or useless on exactly this shape, because two spines of different lengths
//!    agree for every rung of the shorter. A hash-consed store has both: `same` is a `u32`
//!    compare and `structural_hash` is computed at construction. That is the measured case for
//!    stage 2 below, and this store already reduces the same spine at **2.77×** (1.64 ms →
//!    4.55 ms, interning included).
//! 3. **`Nat` has no literal fast path.** `try_primitive_reduce` matches `Literal::Int` and
//!    `Literal::BigInt` only, because `term::lit_bigint` returns `None` for `Literal::Nat`, so
//!    `natLe` peels one `Succ` per unit. The two slowest tests in the kernel crate are both
//!    this, at 61.7 s and 52.4 s of a 66 s suite against a median under 0.3 s. That fix lives
//!    in `reduction.rs`, not here; `W_perf_extra`'s `a_nat_order_fast_path_must_agree_with_the_unary_unfolding`
//!    is the specification it has to satisfy.
//! 4. **The main kernel has no `whnf` and no cache of any kind.** `is_subtype` normalises BOTH
//!    sides to a FULL normal form and then compares, with one `alpha_eq` tree walk as its only
//!    shortcut, on every conversion. Lean's `isDefEq` is whnf-driven over a hash-consed `Expr`
//!    with a `whnf` cache and an `isDefEq` cache. MEASURED: one convertible-but-not-α-equal pair
//!    costs 6.55 ms once and 6.62 ms per call a hundred times over — a FLAT per-call cost, which
//!    is what "no cache anywhere" looks like from outside. That is the largest remaining
//!    algorithmic gap after (3), and [`TermStore::alpha_id`], [`IdCache`], [`TypeCache`] and
//!    [`DefEqCache::scoped`] are the four pieces it needs — the last two carrying the lifetime
//!    and the environment key that make a kernel cache SOUND rather than merely fast.
//!
//! ## The wiring plan
//!
//! **Three routes, and only one is open.**
//!
//! * *Change `Term`'s children from `Box` to `Rc`.* Hundreds of `Box::new` Term-child sites sit
//!   in FROZEN test files. Closed by rule.
//! * *Add a `Term::Shared(Rc<Term>)` variant.* Seven frozen kernel test files match `Term`
//!   exhaustively and wildcard-free, deliberately — `lean_conformance_t00_terms.rs` says so in a doc
//!   comment ("a new variant has to answer here"). Closed by rule.
//! * *Keep `Term` as the BOUNDARY type and make the store the kernel's internal
//!   representation.* **This is the recommendation.** Every frozen test builds a `Term`, calls a
//!   kernel entry point, and compares a `Term`; none of them can observe what happens in
//!   between. The boundary is EIGHT public functions taking `&Term`:
//!   `type_checker::{infer_type, is_subtype, substitute}`,
//!   `reduction::{normalize, normalize_at, normalize_checked}` (plus `normalize_checked_at`),
//!   and `recheck::{recheck, double_check}`. Intern once on the way in, extract once on the way
//!   out. The entry points already pay an O(size) `term.clone()` before their first step —
//!   `normalize_checked_at` pays two — so interning replaces a cost rather than adding one.
//!
//! **Four stages, each independently green.**
//!
//! * **Stage 1 — conversion, `type_checker.rs` only.** Five `alpha_eq` call sites become
//!   [`alpha_same`](TermStore::alpha_same), and `is_subtype` gains the cache it has never had:
//!   `HashMap<(TermId, TermId), bool>`. The walk is replaced by a walk of the same order that
//!   MEMOISES, so the second conversion mentioning a subterm is O(1) instead of O(size). Hold
//!   the store thread-local behind the existing `INSIDE_KERNEL` door, so the outermost entry
//!   creates it and `DoorGuard` drops it — a cache with the lifetime of one kernel call cannot
//!   grow without bound or leak across a `Context` change.
//! * **Stage 2 — the reducer, `reduction.rs`.** `normalize_checked_at` interns once and runs its
//!   fixpoint and Brent cycle detection on `TermId`s: (2) above goes away outright. The 54
//!   `reduce_step_at` arms port one at a time with `recheck_reduction_differential.rs` as the
//!   oracle. Depth stops binding, because the traversal is this file's explicit stack.
//! * **Stage 3 — `infer_type`, `type_checker.rs`.** Its 33 recursive self-calls become one
//!   [`fold`](TermStore::fold) over an [`IdCache`] of types — which IS
//!   `a_shared_subterm_is_checked_once_and_not_once_per_occurrence`, by construction rather than
//!   by a special case — and the 16 KB frame that aborts the process disappears with the
//!   recursion.
//! * **Stage 4 — the second kernel, `recheck.rs`.** It gets its OWN `TermStore`. Do not share
//!   one between the two kernels: six soundness bugs this campaign were miscounted identically
//!   in both, so `double_check` agreed with the bug throughout. A shared store is a shared bug.
//!
//! ## What this file hands the kernel TODAY, with no representation change
//!
//! The four stages above all move the kernel onto `TermId`. These work against the
//! `Box`-recursive `Term` the kernel already has, and each is one line at a call site. **The
//! first three are WIRED IN** — `type_checker::{infer_type, is_subtype}`,
//! `reduction::normalize_checked_at`, `recheck::{recheck, double_check}` and
//! `reduction::reduce_step_opt`'s `Term::Let` arm all call into this file now:
//!
//! * [`deepen_stack`] — the kernel takes its own stack, sized from the term it was handed, so
//!   the door's advertised limit becomes the real one and no admitted term can abort the
//!   process. One line at the top of each public entry, with no re-indentation of its body.
//! * [`zeta_or_share_opt`] — the whole occurrence-gated ζ rule, as one call, in the
//!   `Option<Term>` shape `reduce_step_opt` has. Both kernels used to ζ-substitute
//!   UNCONDITIONALLY, so a value bound once and used n times was written and checked n times,
//!   which is `a_shared_subterm_is_checked_once_and_not_once_per_occurrence` exactly: **329
//!   nodes against a ceiling of 97, and 92 under this rule — the case is green.**
//!   [`occurrences_in_term`] is the gate underneath it and [`zeta_expand_head_term`] is what
//!   conversion owes in return. All three are iterative, so none has the ceiling
//!   `type_checker::body_stats` does.
//! * [`kernel_stack_bytes`] — `recheck.rs`'s stack budget was a hard-coded 1 MiB that assumed a
//!   2 MiB thread. Derived from this instead, the second kernel stops answering `Unsupported`
//!   on a deep term the first kernel now checks happily.
//! * [`deepen_stack_with_depth`] — **NOT yet wired, and it closes a hole the other three do
//!   not.** The door and [`deepen_stack`] both measure the term they were HANDED;
//!   `infer_type`'s `Var` arm walks the CONTEXT's type instead. MEASURED in a child process: a
//!   one-node term `Var("x")` under a binding of depth 4000 kills the process both bare AND
//!   through `deepen_stack`, and is answered when the sizer is told the context's depth.
//! * [`TypeCache`] + [`TermStore::env_for`] — `inferType` memoised on `(environment, term)`,
//!   which is `a_shared_subterm_is_checked_once_and_not_once_per_occurrence` on the CHECKING
//!   side. The environment is in the key because `Term` names its variables and
//!   `Context::extend` shadows; Lean's `Expr` is de Bruijn and does not need it, which is
//!   exactly why porting its `inferType` cache across unchanged is unsound here.
//!
//! **Two things not to do.** Do not put the store in `Context` — `Context` is cloned on every
//! `extend` and would copy the store with it. And do not key a CACHE on a term's identity alone:
//! `Term` names its variables and `Context::extend` SHADOWS, so one `TermId` has one type per
//! scope. [`TypeCache`] carries the [`EnvId`] that makes the key sound, and [`KernelCall`] gives
//! it a lifetime it cannot escape.
//!
//! **The float question is SETTLED.** `Literal`'s `PartialEq` is hand-written in `term.rs` and
//! compares `f64::to_bits()`, so it is total and reflexive and the store's interning key is the
//! kernel's own equality. [`literal_identity_eq`] is the INDEPENDENT second implementation of
//! that rule, locked against `term.rs`'s over an adversarial grid, and
//! [`eq_faithful`](TermStore::eq_faithful) is the conservative guard that was correct under the
//! old rule and stays correct under the new one.
//!
//! ## Example
//!
//! ```
//! use logicaffeine_kernel::term_store::TermStore;
//! use logicaffeine_kernel::Term;
//!
//! let mut store = TermStore::new();
//! let x = Term::Var("x".to_string());
//! let twice = Term::App(Box::new(x.clone()), Box::new(x.clone()));
//!
//! // The two occurrences of `x` are ONE node.
//! let id = store.intern(&twice);
//! assert_eq!(store.dag_size(id), 2);      // the App and the one shared Var
//! assert_eq!(store.tree_size(id), 3);     // what a tree walk would count
//!
//! // Interning identical structure yields the identical handle: equality is O(1).
//! assert_eq!(store.intern(&twice), id);
//! assert_eq!(store.extract(id), twice);
//! ```

use std::collections::HashMap;

use logicaffeine_base::{BigInt, Interner, Symbol};

use crate::term::{Literal, Term, Universe};

// ================================================================================================
// HANDLES
// ================================================================================================

/// Identity of an interned term — a `Copy` handle whose equality IS structural equality.
///
/// The one caveat is floating-point, and it is exact: see [`TermStore::float_free`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TermId(u32);

impl TermId {
    /// The dense index this handle occupies — for `Vec`-keyed side tables.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identity of an interned universe level.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LevelId(u32);

impl LevelId {
    /// The dense index this handle occupies.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identity of an interned literal.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LitId(u32);

impl LitId {
    /// The dense index this handle occupies.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identity of a hash-consed, sorted set of names.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NameSetId(u32);

/// Identity of a hash-consed LOCAL TYPING ENVIRONMENT — the fragment of a `Context` that
/// decides a term's type, and therefore half of the key an `inferType` cache must have.
///
/// A `Term` names its variables. `Var("x")` interns to ONE [`TermId`] whatever scope it is in,
/// and `Context::extend` SHADOWS, so the same id has type `Nat` under one context and `Bool`
/// under another. A cache keyed on the term alone would hand the second context the first's
/// answer — the kernel certifying a type the term does not have. That is not a hazard Lean
/// shares: its `Expr` uses de Bruijn indices and globally-unique `FVarId`s, so two occurrences
/// in different scopes are literally different `Expr`s and `inferType` may cache on the
/// expression alone. Ours may not, and this is the missing half.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EnvId(u32);

impl EnvId {
    /// The EMPTY environment — the key every CLOSED subterm gets, and the reason the cache
    /// pays off: a closed subterm is checked once for the whole call however many scopes it
    /// occurs under.
    pub const EMPTY: EnvId = EnvId(0);

    /// The dense index this handle occupies.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Identity of a hash-consed simultaneous substitution.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SubstId(u32);

impl SubstId {
    /// The identity substitution, which every operation short-circuits on.
    pub const EMPTY: SubstId = SubstId(0);
}

// ================================================================================================
// FLAT NODES
// ================================================================================================

/// One term node with its children replaced by handles — the flat shape that makes the store
/// a DAG rather than a tree.
///
/// The match arms below mirror `Term`'s thirteen kernel variants ONE FOR ONE, and every match over
/// `Node` in this file is wildcard-free on purpose: a new `Term` variant must answer here
/// rather than slip past under a `_`. `Term::Annotated` has no node: the store keys on the erased
/// term, so an annotated term interns to its term's node.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Node {
    /// `Term::Sort`.
    Sort(LevelId),
    /// `Term::Var`.
    Var(Symbol),
    /// `Term::Global`.
    Global(Symbol),
    /// `Term::Const`.
    Const {
        /// The declaration name.
        name: Symbol,
        /// The explicit universe arguments.
        levels: Box<[LevelId]>,
    },
    /// `Term::Pi`.
    Pi {
        /// The bound name.
        param: Symbol,
        /// The domain.
        param_type: TermId,
        /// The codomain, in which `param` is bound.
        body_type: TermId,
    },
    /// `Term::Lambda`.
    Lambda {
        /// The bound name.
        param: Symbol,
        /// The domain.
        param_type: TermId,
        /// The body, in which `param` is bound.
        body: TermId,
    },
    /// `Term::App`.
    App(TermId, TermId),
    /// `Term::Match`.
    Match {
        /// The scrutinee.
        discriminant: TermId,
        /// The return-type motive.
        motive: TermId,
        /// One branch per constructor, in declaration order.
        cases: Box<[TermId]>,
    },
    /// `Term::Fix`.
    Fix {
        /// The self-reference name, bound in `body`.
        name: Symbol,
        /// The fixpoint body.
        body: TermId,
    },
    /// `Term::MutualFix`.
    MutualFix {
        /// The block's definitions; every name is bound in every body.
        defs: Box<[(Symbol, TermId)]>,
        /// Which definition this occurrence denotes.
        index: usize,
    },
    /// `Term::Let`.
    Let {
        /// The bound name, bound in `body`.
        name: Symbol,
        /// The ascribed type.
        ty: TermId,
        /// The bound value.
        value: TermId,
        /// The body.
        body: TermId,
    },
    /// `Term::Lit`.
    Lit(LitId),
    /// `Term::Hole`.
    Hole,
    /// A DE BRUIJN LEVEL — a bound-variable occurrence in an α-CANONICAL form.
    ///
    /// [`TermStore::intern`] never produces this: no `Term` can spell it, which is exactly why
    /// it is a node and not a reserved NAME. A reserved name could collide with a free variable
    /// a user actually wrote, and the collision would silently conflate two terms that are not
    /// α-equivalent. A node shape cannot collide with anything.
    Bound(u32),
}

impl Node {
    /// Visit this node's term children, in their canonical order.
    ///
    /// This is the SINGLE source of truth for child order. [`Node::child_count`] and
    /// [`Node::rebuild_with`] are both defined against it, and
    /// `term_store_extra::rebuild_with_round_trips_every_variant` locks the three together.
    pub fn for_each_child(&self, f: &mut impl FnMut(TermId)) {
        match self {
            Node::Sort(_)
            | Node::Var(_)
            | Node::Global(_)
            | Node::Const { .. }
            | Node::Lit(_)
            | Node::Hole
            | Node::Bound(_) => {}
            Node::Pi {
                param_type,
                body_type,
                ..
            } => {
                f(*param_type);
                f(*body_type);
            }
            Node::Lambda {
                param_type, body, ..
            } => {
                f(*param_type);
                f(*body);
            }
            Node::App(fun, arg) => {
                f(*fun);
                f(*arg);
            }
            Node::Match {
                discriminant,
                motive,
                cases,
            } => {
                f(*discriminant);
                f(*motive);
                for c in cases.iter() {
                    f(*c);
                }
            }
            Node::Fix { body, .. } => f(*body),
            Node::MutualFix { defs, .. } => {
                for (_, b) in defs.iter() {
                    f(*b);
                }
            }
            Node::Let {
                ty, value, body, ..
            } => {
                f(*ty);
                f(*value);
                f(*body);
            }
        }
    }

    /// How many term children this node has.
    pub fn child_count(&self) -> usize {
        let mut n = 0;
        self.for_each_child(&mut |_| n += 1);
        n
    }

    /// This node with its children replaced, positionally, by `children`.
    ///
    /// # Panics
    ///
    /// Panics if `children.len() != self.child_count()` — a caller that miscounts is a bug,
    /// not a recoverable condition.
    pub fn rebuild_with(&self, children: &[TermId]) -> Node {
        assert_eq!(
            children.len(),
            self.child_count(),
            "rebuild_with needs every child"
        );
        match self {
            Node::Sort(_)
            | Node::Var(_)
            | Node::Global(_)
            | Node::Const { .. }
            | Node::Lit(_)
            | Node::Hole
            | Node::Bound(_) => self.clone(),
            Node::Pi { param, .. } => Node::Pi {
                param: *param,
                param_type: children[0],
                body_type: children[1],
            },
            Node::Lambda { param, .. } => Node::Lambda {
                param: *param,
                param_type: children[0],
                body: children[1],
            },
            Node::App(_, _) => Node::App(children[0], children[1]),
            Node::Match { .. } => Node::Match {
                discriminant: children[0],
                motive: children[1],
                cases: children[2..].to_vec().into_boxed_slice(),
            },
            Node::Fix { name, .. } => Node::Fix {
                name: *name,
                body: children[0],
            },
            Node::MutualFix { defs, index } => Node::MutualFix {
                defs: defs
                    .iter()
                    .zip(children.iter())
                    .map(|((n, _), c)| (*n, *c))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                index: *index,
            },
            Node::Let { name, .. } => Node::Let {
                name: *name,
                ty: children[0],
                value: children[1],
                body: children[2],
            },
        }
    }

    /// The names this node BINDS in its children, paired with which children they scope over.
    ///
    /// `(names, scoped)` — `names` are bound in exactly the children whose index is in
    /// `scoped`. `Lambda`'s param scopes over the body but NOT over the domain; `Let`'s name
    /// scopes over the body but not the type or value; `MutualFix`'s names scope over EVERY
    /// body. Substitution and occurrence counting both read this, so shadowing is defined in
    /// one place.
    fn binding(&self) -> (Vec<Symbol>, Vec<usize>) {
        match self {
            Node::Sort(_)
            | Node::Var(_)
            | Node::Global(_)
            | Node::Const { .. }
            | Node::Lit(_)
            | Node::Hole
            | Node::Bound(_)
            | Node::App(_, _)
            | Node::Match { .. } => (Vec::new(), Vec::new()),
            Node::Pi { param, .. } => (vec![*param], vec![1]),
            Node::Lambda { param, .. } => (vec![*param], vec![1]),
            Node::Fix { name, .. } => (vec![*name], vec![0]),
            Node::MutualFix { defs, .. } => (
                defs.iter().map(|(n, _)| *n).collect(),
                (0..defs.len()).collect(),
            ),
            Node::Let { name, .. } => (vec![*name], vec![2]),
        }
    }

    /// This node with its bound names replaced by `names`, in `binding()` order.
    fn with_binders(&self, names: &[Symbol]) -> Node {
        match self {
            Node::Sort(_)
            | Node::Var(_)
            | Node::Global(_)
            | Node::Const { .. }
            | Node::Lit(_)
            | Node::Hole
            | Node::Bound(_)
            | Node::App(_, _)
            | Node::Match { .. } => self.clone(),
            Node::Pi {
                param_type,
                body_type,
                ..
            } => Node::Pi {
                param: names[0],
                param_type: *param_type,
                body_type: *body_type,
            },
            Node::Lambda {
                param_type, body, ..
            } => Node::Lambda {
                param: names[0],
                param_type: *param_type,
                body: *body,
            },
            Node::Fix { body, .. } => Node::Fix {
                name: names[0],
                body: *body,
            },
            Node::MutualFix { defs, index } => Node::MutualFix {
                defs: defs
                    .iter()
                    .zip(names.iter())
                    .map(|((_, b), n)| (*n, *b))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                index: *index,
            },
            Node::Let {
                ty, value, body, ..
            } => Node::Let {
                name: names[0],
                ty: *ty,
                value: *value,
                body: *body,
            },
        }
    }

    /// A small integer distinguishing the variant, for the structural hash.
    fn tag(&self) -> u64 {
        match self {
            Node::Sort(_) => 1,
            Node::Var(_) => 2,
            Node::Global(_) => 3,
            Node::Const { .. } => 4,
            Node::Pi { .. } => 5,
            Node::Lambda { .. } => 6,
            Node::App(_, _) => 7,
            Node::Match { .. } => 8,
            Node::Fix { .. } => 9,
            Node::MutualFix { .. } => 10,
            Node::Let { .. } => 11,
            Node::Lit(_) => 12,
            Node::Hole => 13,
            Node::Bound(_) => 14,
        }
    }
}

/// Which `Node` kind a subterm is — one per [`Node`] variant, and wildcard-free everywhere it is
/// matched for the same reason [`Node`] is.
///
/// This exists so a caller can ask "does this subterm contain a `Match` anywhere?" in O(1)
/// instead of walking it. The bit lives in `Info` and is ORed up from the children exactly
/// once, at interning time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NodeShape {
    /// [`Node::Sort`].
    Sort,
    /// [`Node::Var`].
    Var,
    /// [`Node::Global`].
    Global,
    /// [`Node::Const`].
    Const,
    /// [`Node::Pi`].
    Pi,
    /// [`Node::Lambda`].
    Lambda,
    /// [`Node::App`].
    App,
    /// [`Node::Match`].
    Match,
    /// [`Node::Fix`].
    Fix,
    /// [`Node::MutualFix`].
    MutualFix,
    /// [`Node::Let`].
    Let,
    /// [`Node::Lit`].
    Lit,
    /// [`Node::Hole`].
    Hole,
    /// [`Node::Bound`] — a de Bruijn level, which only an α-canonical form carries.
    Bound,
}

impl NodeShape {
    /// The single bit this shape occupies in a [`ShapeMask`].
    const fn bit(self) -> u16 {
        1u16 << (self as u16)
    }

    /// The shape of one node.
    fn of(node: &Node) -> NodeShape {
        match node {
            Node::Sort(_) => NodeShape::Sort,
            Node::Var(_) => NodeShape::Var,
            Node::Global(_) => NodeShape::Global,
            Node::Const { .. } => NodeShape::Const,
            Node::Pi { .. } => NodeShape::Pi,
            Node::Lambda { .. } => NodeShape::Lambda,
            Node::App(_, _) => NodeShape::App,
            Node::Match { .. } => NodeShape::Match,
            Node::Fix { .. } => NodeShape::Fix,
            Node::MutualFix { .. } => NodeShape::MutualFix,
            Node::Let { .. } => NodeShape::Let,
            Node::Lit(_) => NodeShape::Lit,
            Node::Hole => NodeShape::Hole,
            Node::Bound(_) => NodeShape::Bound,
        }
    }
}

/// The set of [`NodeShape`]s occurring anywhere in a subterm — one `u16`, computed once at
/// interning time and answered in O(1) thereafter.
///
/// # The Lean design this copies
///
/// Lean's `Expr` carries a 64-bit `Expr.Data` word in every node, and four of its bits are
/// exactly this idea: `hasFVar`, `hasExprMVar`, `hasLevelMVar`, `hasLevelParam`
/// (`Lean/Expr.lean:121-124`). `Expr.mkAppData` propagates all four with a single OR of the two
/// children's words — `((fData ||| aData) &&& (15 <<< 40)) ||| …` (`Lean/Expr.lean:193`) — so an
/// application's bits cost one instruction rather than a traversal. What they BUY is visible at
/// `Lean/Expr.lean:1661`: `let rec visit (e : Expr) := if !e.hasFVar then false else …`, a walk
/// that stops before it starts.
///
/// Ours widens the idea from four hand-picked predicates to one bit per node kind, which is what
/// lets [`beta_zeta_normal_form`] decide in O(1) whether a term can hold a redex it does not own.
/// The cost is the same: one OR per child at interning time, in `TermStore::compute_info`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ShapeMask(u16);

impl ShapeMask {
    /// The empty mask — no shape occurs.
    pub const NONE: ShapeMask = ShapeMask(0);

    /// The shapes a term made only of `Sort`, `Var`, `Global`, `App`, `Lambda`, `Pi` and `Let`
    /// can have — the admissible surface of [`beta_zeta_normal_form`]. Its complement is
    /// `Const`, `Match`, `Fix`, `MutualFix`, `Lit`, `Hole` and `Bound`.
    pub const BETA_ZETA: ShapeMask = ShapeMask(
        NodeShape::Sort.bit()
            | NodeShape::Var.bit()
            | NodeShape::Global.bit()
            | NodeShape::App.bit()
            | NodeShape::Lambda.bit()
            | NodeShape::Pi.bit()
            | NodeShape::Let.bit(),
    );

    /// Whether `shape` occurs anywhere in the subterm.
    pub fn contains(self, shape: NodeShape) -> bool {
        self.0 & shape.bit() != 0
    }

    /// Whether every shape in this mask is also in `other`.
    pub fn subset_of(self, other: ShapeMask) -> bool {
        self.0 & !other.0 == 0
    }

    /// The union of two masks.
    pub fn union(self, other: ShapeMask) -> ShapeMask {
        ShapeMask(self.0 | other.0)
    }

    /// This mask with `shape` added.
    pub fn with(self, shape: NodeShape) -> ShapeMask {
        ShapeMask(self.0 | shape.bit())
    }

    /// The raw bits, for a caller that wants to store or compare them.
    pub fn bits(self) -> u16 {
        self.0
    }
}

/// A borrowed view of one interned universe level.
///
/// Child handles belong to this store. Reading a view does not extract, clone,
/// normalize, or recursively visit its payload.
#[derive(Clone, Copy, Debug)]
pub enum LevelView<'a> {
    /// The strict proposition universe.
    SProp,
    /// The proposition universe.
    Prop,
    /// A concrete type universe.
    Type(u32),
    /// A borrowed universe variable name.
    Var(&'a str),
    /// Successor of an interned level.
    Succ(LevelId),
    /// Maximum of two interned levels.
    Max(LevelId, LevelId),
    /// Impredicative maximum of two interned levels.
    IMax(LevelId, LevelId),
}

/// A universe level with its children replaced by handles.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum LevelNode {
    SProp,
    Prop,
    Type(u32),
    Var(Symbol),
    Succ(LevelId),
    Max(LevelId, LevelId),
    IMax(LevelId, LevelId),
}

/// A literal, keyed so that it can be hash-consed.
///
/// `Float` is keyed on its RAW BITS — the only choice that yields an equivalence relation, and
/// exactly the rule `Literal`'s hand-written `PartialEq` compares by, so the key is the kernel's
/// own literal equality.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum LitKey {
    Int(i64),
    FloatBits(u64),
    Text(Symbol),
    Duration(i64),
    Date(i32),
    Moment(i64),
    BigInt(BigInt),
    Nat(BigInt),
}

/// Whether IEEE `==` and bit equality agree about this literal — the exact condition under
/// which the store's id equality is a faithful decision procedure for `Term`'s `==`.
///
/// For every `f64` that is neither a NaN nor a zero, equal values have equal bit patterns and
/// distinct bit patterns are distinct values, so the two relations coincide. The exceptions are
/// exactly two, and they break the two halves of the contract `Term` claims:
///
/// * a **NaN** is `!=` itself, so `impl Eq for Literal {}` is a false claim — `Eq` promises
///   reflexivity — and any algorithm that recovers "nothing changed" from `a == a` never fires;
/// * **`0.0` and `-0.0`** are `==` with DIFFERENT bit patterns, so `Term` equality identifies two
///   terms the kernel can still tell apart by `Display`, by serialisation, and — the moment a
///   float primitive lands — by `1.0 / x`.
fn float_eq_is_bit_eq(key: &LitKey) -> bool {
    match key {
        LitKey::FloatBits(b) => {
            let x = f64::from_bits(*b);
            !x.is_nan() && x != 0.0
        }
        LitKey::Int(_)
        | LitKey::Text(_)
        | LitKey::Duration(_)
        | LitKey::Date(_)
        | LitKey::Moment(_)
        | LitKey::BigInt(_)
        | LitKey::Nat(_) => true,
    }
}

/// **THE ONE RULE.** Total, reflexive literal equality — bit-exact on floats — offered as a
/// FUNCTION so `term.rs` and this store cannot hold two different opinions about it.
///
/// # Why a hash-consed kernel has no second option
///
/// Hash-consing is a QUOTIENT: interning maps a term to the equivalence class of its structure,
/// and a quotient exists only for an EQUIVALENCE relation. `Literal`'s derived `PartialEq` uses
/// IEEE `==` on `Literal::Float`, which is
///
/// * not reflexive (`NaN != NaN`), and
/// * not a function of the representation (`0.0 == -0.0` holds between distinct bit patterns).
///
/// So it is not an equivalence relation, and **no hash-consing scheme can implement it** — not
/// this one, not Lean's, not any. The choice is therefore not between two tastes: either
/// `Literal`'s equality becomes bit-exact on floats, or the kernel can never key anything on
/// term identity. Lean made the same call: `Literal.ofNat`/`ofScientific` compare by value and
/// its `Expr` equality is structural, never IEEE.
///
/// # It is the rule `term.rs` now has, and this is the INDEPENDENT model it is locked against
///
/// `Literal`'s `PartialEq` is hand-written and compares `f64::to_bits()`, which makes the
/// existing `impl Eq for Literal {}` an honest claim rather than an unsound contract. This
/// function is deliberately NOT a call to it: it is a second implementation, wildcard-free on
/// the diagonal where `term.rs`'s falls through to `_ => false`, and
/// `W_perf_extra::the_two_implementations_of_the_one_literal_rule_agree_on_every_pair` runs the
/// two against each other over a grid that contains three distinct NaN bit patterns, both zeros,
/// and one value of every other variant. Two implementations that agree is evidence; one
/// implementation compared with itself is not.
pub fn literal_identity_eq(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Float(x), Literal::Float(y)) => x.to_bits() == y.to_bits(),
        (Literal::Int(x), Literal::Int(y)) => x == y,
        (Literal::Text(x), Literal::Text(y)) => x == y,
        (Literal::Duration(x), Literal::Duration(y)) => x == y,
        (Literal::Date(x), Literal::Date(y)) => x == y,
        (Literal::Moment(x), Literal::Moment(y)) => x == y,
        (Literal::BigInt(x), Literal::BigInt(y)) => x == y,
        (Literal::Nat(x), Literal::Nat(y)) => x == y,
        // Wildcard-free on the DIAGONAL, which is the half that decides anything; two different
        // variants are different literals and always were.
        (Literal::Float(_), _)
        | (Literal::Int(_), _)
        | (Literal::Text(_), _)
        | (Literal::Duration(_), _)
        | (Literal::Date(_), _)
        | (Literal::Moment(_), _)
        | (Literal::BigInt(_), _)
        | (Literal::Nat(_), _) => false,
    }
}

// ================================================================================================
// PER-NODE ANNOTATIONS
// ================================================================================================

/// Everything the store knows about a node without re-walking it.
#[derive(Clone, Copy, Debug)]
struct Info {
    /// Maximum height: 1 for a leaf, `1 + max(children)` otherwise. This is what lets a
    /// caller size a traversal, or refuse a term, WITHOUT walking it.
    depth: u32,
    /// The node count a tree walk would report — what `Term`'s shape actually costs.
    /// Saturating, because a DAG can denote a tree far larger than `u64`.
    tree_size: u64,
    /// O(1) structural hash. Equal ids have equal hashes; unequal ids almost never collide.
    hash: u64,
    /// Whether the subterm is free of `Literal::Float`. `false` is the ONLY condition under
    /// which id equality and `Term` equality can disagree.
    float_free: bool,
    /// Whether id equality is a FAITHFUL decision procedure for `Term` equality on this
    /// subterm — the precise condition [`TermStore::float_free`] over-approximates. See
    /// [`TermStore::eq_faithful`].
    eq_faithful: bool,
    /// Free variable names, hash-consed and sorted.
    free: NameSetId,
    /// Every binder name occurring anywhere below, hash-consed and sorted — the exclusion
    /// set a freshening pass needs.
    binders: NameSetId,
    /// Which [`NodeShape`]s occur anywhere below, ORed up from the children. See
    /// [`ShapeMask`] for the Lean design this is copied from.
    shapes: ShapeMask,
    /// Every `Global` and `Const` NAME occurring anywhere below, hash-consed and sorted — the
    /// δ SURFACE. It is the set a caller has to certify inert before a reducer that has no δ
    /// rule may be trusted with the term.
    consts: NameSetId,
}

/// A hash-consed simultaneous substitution `dom[i] ↦ to[i]`, sorted by `dom`.
#[derive(Clone, Debug)]
struct Subst {
    dom: Box<[Symbol]>,
    to: Box<[TermId]>,
    /// Bloom mask of `dom`, for an O(1) "this node cannot be affected" test.
    dom_mask: u64,
    /// The union of the free variables of every `to`, for the capture test.
    range_free: NameSetId,
}

/// The binder name every α-canonical binder carries. It is cosmetic: a canonical form's
/// bound OCCURRENCES are [`Node::Bound`] nodes, so the name carries no information and cannot
/// be confused with a user's.
const CANON_BINDER: &str = "⟪⟫";

/// The prefix a canonical bound occurrence prints with, for [`TermStore::extract`].
const CANON_OCCURRENCE: &str = "⟪";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn mix(h: u64, x: u64) -> u64 {
    (h ^ x).wrapping_mul(FNV_PRIME)
}

/// Hash a `BigInt` by its canonical little-endian magnitude and sign, so the hash is a
/// function of the VALUE and not of the internal limb layout.
fn bigint_hash(seed: u64, n: &BigInt) -> u64 {
    let (negative, bytes) = n.to_le_bytes();
    let mut h = mix(seed, u64::from(negative));
    for b in bytes {
        h = mix(h, u64::from(b));
    }
    h
}

/// The bloom bit of one name — `mask & bit == 0` proves the name is absent.
fn name_bit(s: Symbol) -> u64 {
    let mut h = s.index() as u64;
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    1u64 << (h & 63)
}

/// How often a name occurs — the only distinction an occurrence-gated ζ rule needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Occurrences {
    /// The name does not occur free at all.
    Zero,
    /// Exactly one free occurrence — substituting cannot duplicate anything.
    Once,
    /// Two or more free occurrences — substituting COPIES the value at each one.
    Many,
}

/// The result of [`TermStore::beta_zeta_normalize`] — honest about a budget it did not finish
/// inside, the way `reduction::normalize_checked` is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreNormalForm {
    /// A β/ζ normal form was reached.
    Reached(TermId),
    /// The step budget ran out. The term is NOT known to be normal.
    OutOfFuel {
        /// The budget that was exhausted.
        budget: u64,
    },
}

/// A DAG whose unfolding is too large to materialise as a `Term`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TooLarge {
    /// The tree node count the unfolding would have.
    pub tree_size: u64,
    /// The limit the caller set.
    pub limit: u64,
}

// ================================================================================================
// THE STORE
// ================================================================================================

/// A bounded DAG transformation stopped before admitting further work or a distinct node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoreOperationLimit {
    Work,
    Nodes,
    Allocation,
}

/// A hash-consed term store: maximal structural sharing, O(1) equality, O(1) annotations.
///
/// Every `mk_*` and [`intern`](TermStore::intern) call goes through one dedup table, so two
/// structurally identical subterms are ONE node with ONE id. Nothing in this type recurses
/// natively, and its `Drop` is a flat `Vec` drop.
pub struct TermStore {
    nodes: Vec<Node>,
    info: Vec<Info>,
    dedup: HashMap<Node, TermId>,

    levels: Vec<LevelNode>,
    level_dedup: HashMap<LevelNode, LevelId>,

    lits: Vec<LitKey>,
    lit_dedup: HashMap<LitKey, LitId>,

    names: Interner,
    /// FNV hash of each symbol's STRING, so the structural hash is a function of structure
    /// and not of interning order — the property that makes it comparable across stores.
    name_hashes: Vec<u64>,
    level_hashes: Vec<u64>,
    lit_hashes: Vec<u64>,

    sets: Vec<Box<[Symbol]>>,
    set_masks: Vec<u64>,
    set_dedup: HashMap<Box<[Symbol]>, NameSetId>,

    substs: Vec<Subst>,
    subst_dedup: HashMap<(Box<[Symbol]>, Box<[TermId]>), SubstId>,

    /// Hash-consed local typing environments, each sorted by name. [`EnvId::EMPTY`] is
    /// pre-interned at index 0.
    envs: Vec<Box<[(Symbol, TermId)]>>,
    env_dedup: HashMap<Box<[(Symbol, TermId)]>, EnvId>,

    /// `(substitution, term) ↦ result`. Pure in both arguments and both are immutable, so
    /// the cache is valid for the life of the store.
    subst_cache: HashMap<(SubstId, TermId), TermId>,
    /// `(name, term) ↦ free occurrence count`, saturating.
    occ_cache: HashMap<(Symbol, TermId), u32>,
    /// `term ↦ β/ζ normal form`. Only COMPLETED results are entered.
    nf_cache: HashMap<(TermId, bool), TermId>,
    /// `(environment, level, term) ↦ α-canonical form`. Keyed on the LEVEL as well as the
    /// environment because two paths can bind the same names at the same levels after
    /// restriction and still sit at different depths, and the depth decides the levels a
    /// binder BELOW gets. Dropping the level from the key is unsound; it was tried.
    canon_cache: HashMap<(SubstId, u32, TermId), TermId>,
    /// `term ↦ α-canonical form` for the outermost call, where the environment is empty and the
    /// level is zero — the only entry point conversion ever asks for.
    alpha_cache: HashMap<TermId, TermId>,
    head_dependencies: HashMap<TermId, NameSetId>,
    /// Fresh-name counter, so a freshening pass never rescans from 1.
    fresh_counter: u64,
    /// Whether any [`substitute`](TermStore::substitute) since the last
    /// [`clear_caches`](TermStore::clear_caches) was asked to push a value into a body that
    /// BINDS one of the value's free names. See [`TermStore::rename_risk`].
    rename_risk: bool,
}

impl Default for TermStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TermStore {
    /// An empty store, with the empty name set pre-interned at [`NameSetId`] 0 and the
    /// identity substitution at [`SubstId::EMPTY`].
    pub fn new() -> Self {
        let mut store = TermStore {
            nodes: Vec::new(),
            info: Vec::new(),
            dedup: HashMap::new(),
            levels: Vec::new(),
            level_dedup: HashMap::new(),
            lits: Vec::new(),
            lit_dedup: HashMap::new(),
            names: Interner::new(),
            name_hashes: Vec::new(),
            level_hashes: Vec::new(),
            lit_hashes: Vec::new(),
            sets: Vec::new(),
            set_masks: Vec::new(),
            set_dedup: HashMap::new(),
            substs: Vec::new(),
            subst_dedup: HashMap::new(),
            envs: Vec::new(),
            env_dedup: HashMap::new(),
            subst_cache: HashMap::new(),
            occ_cache: HashMap::new(),
            nf_cache: HashMap::new(),
            canon_cache: HashMap::new(),
            alpha_cache: HashMap::new(),
            head_dependencies: HashMap::new(),
            fresh_counter: 0,
            rename_risk: false,
        };
        let empty: Box<[Symbol]> = Vec::new().into_boxed_slice();
        store.sets.push(empty.clone());
        store.set_masks.push(0);
        store.set_dedup.insert(empty, NameSetId(0));
        store.substs.push(Subst {
            dom: Vec::new().into_boxed_slice(),
            to: Vec::new().into_boxed_slice(),
            dom_mask: 0,
            range_free: NameSetId(0),
        });
        store.subst_dedup.insert(
            (Vec::new().into_boxed_slice(), Vec::new().into_boxed_slice()),
            SubstId::EMPTY,
        );
        let empty_env: Box<[(Symbol, TermId)]> = Vec::new().into_boxed_slice();
        store.envs.push(empty_env.clone());
        store.env_dedup.insert(empty_env, EnvId::EMPTY);
        store
    }

    /// How many DISTINCT nodes the store holds.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// How many distinct universe levels have been interned (without allocating).
    pub(crate) fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Whether the store holds no nodes at all.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Drop every memo table. Ids stay valid; only cached derived results are discarded.
    pub fn clear_caches(&mut self) {
        self.subst_cache.clear();
        self.occ_cache.clear();
        self.nf_cache.clear();
        self.canon_cache.clear();
        self.alpha_cache.clear();
        self.head_dependencies.clear();
        self.rename_risk = false;
    }

    /// Whether any substitution this store has performed could have RENAMED a binder.
    ///
    /// It is set when [`substitute`](TermStore::substitute) is asked to push a value into a
    /// body that BINDS one of the value's free names — the exact condition under which a
    /// capture-avoiding pass has to invent a name — and it is a hard `false` otherwise.
    ///
    /// # Why a caller comparing two substitutions needs it
    ///
    /// `type_checker::substitute_avoiding` renames a binder when `replacement_fvs.contains(param)`
    /// and this store renames on the same criterion, so on a term where the condition never
    /// arises the two are the SAME FUNCTION and their answers are equal, not merely
    /// α-equivalent. Where it does arise they can still differ, because the two passes visit
    /// different binders: this store descends only where the substitution is live, while
    /// `substitute_avoiding` descends the whole body and renames a binder whose name is a free
    /// variable of the replacement even in a subtree the substituted variable never reaches.
    /// So the flag is the precondition [`beta_zeta_normal_form`] needs to promise an EQUAL
    /// answer rather than an α-equivalent one.
    pub fn rename_risk(&self) -> bool {
        self.rename_risk
    }

    // -- names ----------------------------------------------------------------------------

    /// Intern a name, returning its symbol.
    pub fn symbol(&mut self, name: &str) -> Symbol {
        self.names.intern(name)
    }

    /// An already-interned name's symbol, without creating one.
    pub fn lookup_symbol(&self, name: &str) -> Option<Symbol> {
        self.names.lookup(name)
    }

    /// The string a symbol denotes.
    pub fn name(&self, sym: Symbol) -> &str {
        self.names.resolve(sym)
    }

    /// FNV hash of a symbol's STRING. Memoised in a dense table keyed on the symbol's index,
    /// so it is computed once per distinct name however many nodes mention it.
    fn sym_hash(&mut self, sym: Symbol) -> u64 {
        if self.name_hashes.len() <= sym.index() {
            self.name_hashes.resize(sym.index() + 1, 0);
        }
        if self.name_hashes[sym.index()] == 0 {
            let mut h = FNV_OFFSET;
            for b in self.names.resolve(sym).as_bytes() {
                h = mix(h, u64::from(*b));
            }
            // 0 is the "not yet computed" sentinel, so never store it.
            self.name_hashes[sym.index()] = if h == 0 { 1 } else { h };
        }
        self.name_hashes[sym.index()]
    }

    // -- construction ---------------------------------------------------------------------

    /// The node behind an id.
    pub fn node(&self, id: TermId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Borrow one universe node without materializing its recursive tree.
    ///
    /// As with node(), the handle must have come from this store.
    pub fn level_view(&self, id: LevelId) -> LevelView<'_> {
        match self.levels[id.index()] {
            LevelNode::SProp => LevelView::SProp,
            LevelNode::Prop => LevelView::Prop,
            LevelNode::Type(n) => LevelView::Type(n),
            LevelNode::Var(name) => LevelView::Var(self.name(name)),
            LevelNode::Succ(a) => LevelView::Succ(a),
            LevelNode::Max(a, b) => LevelView::Max(a, b),
            LevelNode::IMax(a, b) => LevelView::IMax(a, b),
        }
    }

    /// Add a node, returning the existing id if an identical node is already stored.
    ///
    /// This is the ONE place a `TermId` is minted, which is what makes the store maximally
    /// shared by construction rather than by a later pass.
    pub fn add(&mut self, node: Node) -> TermId {
        if let Some(&id) = self.dedup.get(&node) {
            return id;
        }
        let info = self.compute_info(&node);
        let id = TermId(self.nodes.len() as u32);
        self.dedup.insert(node.clone(), id);
        self.nodes.push(node);
        self.info.push(info);
        id
    }

    /// Add one already-owned node with admission before dedup hashing, metadata construction,
    /// node cloning or store growth. The caller admits any Node payload vector before creating
    /// it. Charges use cached child name-set lengths, never expanded term-tree size.
    pub(crate) fn add_bounded(
        &mut self,
        node: Node,
        work: &mut u64,
        node_limit: usize,
        allocation: &mut u64,
    ) -> Result<TermId, StoreOperationLimit> {
        let children = node.child_count() as u64;
        let mut hash_work = children.saturating_add(8);
        match &node {
            Node::Const { levels, .. } => hash_work = hash_work.saturating_add(levels.len() as u64),
            Node::MutualFix { defs, .. } => hash_work = hash_work.saturating_add(defs.len() as u64),
            _ => {}
        }
        Self::spend_operation_work(work, hash_work)?;
        // A dedup hit needs no metadata allocation and is valid at the distinct-node ceiling.
        if let Some(&id) = self.dedup.get(&node) {
            return Ok(id);
        }
        if node_limit > u32::MAX as usize || self.nodes.len() >= node_limit {
            return Err(StoreOperationLimit::Nodes);
        }
        self.admit_node_metadata(&node, work, allocation)?;
        Ok(self.add(node))
    }

    /// Upper bounds the actual compute_info operations without allocating or expanding terms.
    /// Retains a known set id through empty/identical unions, so repeated shared children with
    /// the same sets cost linear work, even if their expanded terms have exponential size.
    fn admit_node_metadata(
        &self,
        node: &Node,
        work: &mut u64,
        allocation: &mut u64,
    ) -> Result<(), StoreOperationLimit> {
        #[derive(Clone, Copy)]
        enum SetBound {
            Known(NameSetId),
            Size(u64),
        }
        let length = |value: SetBound| match value {
            SetBound::Known(id) => self.set(id).len() as u64,
            SetBound::Size(n) => n,
        };
        let mut operations = 0u64;
        let mut set_allocations = 0u64;
        let mut units = 32u64;
        let mut union = |left: SetBound, right: SetBound| {
            operations = operations.saturating_add(1);
            if let (SetBound::Known(a), SetBound::Known(b)) = (left, right) {
                if a == b {
                    return left;
                }
            }
            let (a, b) = (length(left), length(right));
            if a == 0 {
                return right;
            }
            if b == 0 {
                return left;
            }
            let size = a.saturating_add(b);
            // Merge, temporary vector, boxed key, retained copy, hashing and mask pass.
            operations = operations.saturating_add(size.saturating_mul(8));
            units = units
                .saturating_add(size.saturating_mul(4))
                .saturating_add(16);
            set_allocations = set_allocations.saturating_add(1);
            SetBound::Size(size)
        };
        let mut binders = SetBound::Known(NameSetId(0));
        let mut constants = SetBound::Known(NameSetId(0));
        let mut free = SetBound::Known(NameSetId(0));
        let bound_count = match node {
            Node::MutualFix { defs, .. } => defs.len(),
            Node::Pi { .. } | Node::Lambda { .. } | Node::Let { .. } | Node::Fix { .. } => 1,
            _ => 0,
        } as u64;
        let mut child_index = 0usize;
        let mut scan_failure = None;
        let mut removal_work = 0u64;
        let mut removal_units = 0u64;
        let mut removal_sets = 0u64;
        node.for_each_child(&mut |child| {
            if scan_failure.is_some() {
                return;
            }
            let info = self.info[child.index()];
            binders = union(binders, SetBound::Known(info.binders));
            constants = union(constants, SetBound::Known(info.consts));
            let scoped = match node {
                Node::Pi { .. } | Node::Lambda { .. } => child_index == 1,
                Node::Let { .. } => child_index == 2,
                Node::Fix { .. } | Node::MutualFix { .. } => true,
                _ => false,
            };
            let original = self.set(info.free);
            let mut contribution = SetBound::Known(info.free);
            if scoped && !original.is_empty() {
                let scan = (original.len() as u64).saturating_mul(bound_count.saturating_add(1));
                if let Err(error) = Self::spend_operation_work(work, scan) {
                    scan_failure = Some(error);
                    return;
                }
                let mut removed = 0u64;
                for symbol in original {
                    let hidden = match node {
                        Node::Pi { param, .. }
                        | Node::Lambda { param, .. }
                        | Node::Let { name: param, .. }
                        | Node::Fix { name: param, .. } => symbol == param,
                        Node::MutualFix { defs, .. } => defs.iter().any(|(name, _)| name == symbol),
                        _ => false,
                    };
                    removed += u64::from(hidden);
                }
                // compute_info calls set_remove for EVERY binder, including misses.
                removal_work = removal_work.saturating_add(
                    (original.len() as u64)
                        .saturating_add(1)
                        .saturating_mul(bound_count)
                        .saturating_mul(8),
                );
                removal_units = removal_units.saturating_add(
                    (original.len() as u64)
                        .saturating_add(1)
                        .saturating_mul(removed)
                        .saturating_mul(4),
                );
                removal_sets = removal_sets.saturating_add(removed);
                if removed > 0 {
                    contribution = if removed == original.len() as u64 {
                        SetBound::Known(NameSetId(0))
                    } else {
                        SetBound::Size(original.len() as u64 - removed)
                    };
                }
            }
            free = union(free, contribution);
            child_index += 1;
        });
        // End the closure's exclusive borrows before accumulating other costs.
        drop(union);
        if let Some(error) = scan_failure {
            return Err(error);
        }
        Self::spend_operation_work(work, child_index as u64)?;
        let binder_size = length(binders).saturating_add(bound_count);
        // Insertion of this node's binders and own variable/global symbol.
        let own = usize::from(matches!(
            node,
            Node::Var(_) | Node::Global(_) | Node::Const { .. }
        )) as u64;
        let insertion_units = binder_size
            .saturating_add(length(constants))
            .saturating_add(1)
            .saturating_mul(bound_count.saturating_add(own))
            .saturating_mul(8);
        let mut payload = child_index as u64;
        let mut name_bytes = 0u64;
        let mut hash_slots = 0usize;
        let mut note_name = |name: Symbol| {
            name_bytes = name_bytes.saturating_add(self.name(name).len() as u64);
            hash_slots = hash_slots.max(
                name.index()
                    .saturating_add(1)
                    .saturating_sub(self.name_hashes.len()),
            );
        };
        match node {
            Node::Var(n)
            | Node::Global(n)
            | Node::Fix { name: n, .. }
            | Node::Pi { param: n, .. }
            | Node::Lambda { param: n, .. }
            | Node::Let { name: n, .. } => note_name(*n),
            Node::Const { name, levels } => {
                note_name(*name);
                payload = payload.saturating_add(levels.len() as u64);
            }
            Node::MutualFix { defs, .. } => {
                payload = payload.saturating_add(defs.len() as u64);
                for (name, _) in defs.iter() {
                    note_name(*name);
                }
            }
            _ => {}
        }
        drop(note_name);
        let set_upper = set_allocations
            .saturating_add(removal_sets)
            .saturating_add(bound_count)
            .saturating_add(own);
        Self::admit_name_set_inventory(self.sets.len(), set_upper)?;
        // Node payload copies, child_free/child_sets, binding/scoped vectors, table growth,
        // and name-hash memo growth. Units are conservative 64-byte allocation credits.
        units = units
            .saturating_add(removal_units)
            .saturating_add(insertion_units)
            .saturating_add(payload.saturating_mul(16))
            .saturating_add(bound_count.saturating_mul(8))
            .saturating_add(name_bytes / 64 + 1)
            .saturating_add((hash_slots as u64) / 4 + 1)
            .saturating_add(64);
        operations = operations
            .saturating_add(removal_work)
            .saturating_add(insertion_units)
            .saturating_add(payload.saturating_mul(8))
            .saturating_add(name_bytes)
            .saturating_add(hash_slots as u64)
            .saturating_add(32);
        Self::spend_operation_work(work, operations)?;
        Self::admit_operation_allocation(allocation, units)
    }

    fn compute_info(&mut self, node: &Node) -> Info {
        let mut depth = 0u32;
        let mut tree_size = 1u64;
        let mut hash = mix(FNV_OFFSET, node.tag());
        let mut float_free = true;
        let mut eq_faithful = true;
        let mut binders: NameSetId = NameSetId(0);
        // One bit per node kind, ORed up from the children exactly as Lean's `mkAppData` ORs
        // its four `has*` bits (`Lean/Expr.lean:193`).
        let mut shapes = ShapeMask::NONE.with(NodeShape::of(node));
        let mut consts: NameSetId = NameSetId(0);

        // The hash mixes NAMES and LEVELS by their own structural hashes, never by their
        // dense indices: an index depends on interning order, so a hash built from one would
        // differ between two stores holding the same term.
        match node {
            Node::Sort(l) => hash = mix(hash, self.level_hashes[l.index()]),
            Node::Var(s) | Node::Global(s) | Node::Fix { name: s, .. } => {
                let h = self.sym_hash(*s);
                hash = mix(hash, h);
            }
            Node::Const { name, levels } => {
                let h = self.sym_hash(*name);
                hash = mix(hash, h);
                for l in levels.iter() {
                    hash = mix(hash, self.level_hashes[l.index()]);
                }
            }
            Node::Pi { param, .. } | Node::Lambda { param, .. } | Node::Let { name: param, .. } => {
                let h = self.sym_hash(*param);
                hash = mix(hash, h);
            }
            Node::MutualFix { defs, index } => {
                hash = mix(hash, *index as u64);
                for (n, _) in defs.iter() {
                    let h = self.sym_hash(*n);
                    hash = mix(hash, h);
                }
            }
            Node::Lit(l) => {
                hash = mix(hash, self.lit_hashes[l.index()]);
                float_free = !matches!(self.lits[l.index()], LitKey::FloatBits(_));
                eq_faithful = float_eq_is_bit_eq(&self.lits[l.index()]);
            }
            Node::Bound(level) => hash = mix(hash, u64::from(*level)),
            Node::App(_, _) | Node::Match { .. } | Node::Hole => {}
        }

        node.for_each_child(&mut |c| {
            let ci = self.info[c.index()];
            depth = depth.max(ci.depth);
            tree_size = tree_size.saturating_add(ci.tree_size);
            hash = mix(hash, ci.hash);
            float_free &= ci.float_free;
            eq_faithful &= ci.eq_faithful;
        });
        // Child metadata is Copy; release each borrow before interning its unions.
        // Walk the borrowed node directly instead of allocating per-node scratch vectors.
        node.for_each_child(&mut |c| {
            let ci = self.info[c.index()];
            binders = self.set_union(binders, ci.binders);
            consts = self.set_union(consts, ci.consts);
            shapes = shapes.union(ci.shapes);
        });
        // This node's own contribution to the δ surface.
        match node {
            Node::Global(n) => consts = self.set_insert(consts, *n),
            Node::Const { name, .. } => consts = self.set_insert(consts, *name),
            Node::Sort(_)
            | Node::Var(_)
            | Node::Pi { .. }
            | Node::Lambda { .. }
            | Node::App(_, _)
            | Node::Match { .. }
            | Node::Fix { .. }
            | Node::MutualFix { .. }
            | Node::Let { .. }
            | Node::Lit(_)
            | Node::Hole
            | Node::Bound(_) => {}
        }

        // Free variables: a child in a binder's scope loses the bound names.
        let (bound, scoped) = node.binding();
        let mut free = NameSetId(0);
        let mut i = 0;
        node.for_each_child(&mut |c| {
            let cf = self.info[c.index()].free;
            let contribution = if scoped.contains(&i) {
                let mut s = cf;
                for &b in &bound {
                    s = self.set_remove(s, b);
                }
                s
            } else {
                cf
            };
            free = self.set_union(free, contribution);
            i += 1;
        });
        if let Node::Var(s) = node {
            free = self.set_singleton(*s);
        }
        for &b in &bound {
            binders = self.set_insert(binders, b);
        }

        Info {
            depth: depth + 1,
            tree_size,
            hash,
            float_free,
            eq_faithful,
            free,
            binders,
            shapes,
            consts,
        }
    }

    /// `Term::Sort(u)`.
    pub fn mk_sort(&mut self, u: &Universe) -> TermId {
        let l = self.intern_level(u);
        self.add(Node::Sort(l))
    }

    /// `Term::Var(name)`.
    pub fn mk_var(&mut self, name: &str) -> TermId {
        let s = self.symbol(name);
        self.add(Node::Var(s))
    }

    /// `Term::Var` from an already-interned name.
    pub fn mk_var_sym(&mut self, s: Symbol) -> TermId {
        self.add(Node::Var(s))
    }

    /// `Term::Global(name)`.
    pub fn mk_global(&mut self, name: &str) -> TermId {
        let s = self.symbol(name);
        self.add(Node::Global(s))
    }

    /// `Term::App(f, a)`.
    pub fn mk_app(&mut self, f: TermId, a: TermId) -> TermId {
        self.add(Node::App(f, a))
    }

    /// `Term::Lambda`.
    pub fn mk_lambda(&mut self, param: &str, param_type: TermId, body: TermId) -> TermId {
        let p = self.symbol(param);
        self.add(Node::Lambda {
            param: p,
            param_type,
            body,
        })
    }

    /// `Term::Pi`.
    pub fn mk_pi(&mut self, param: &str, param_type: TermId, body_type: TermId) -> TermId {
        let p = self.symbol(param);
        self.add(Node::Pi {
            param: p,
            param_type,
            body_type,
        })
    }

    /// `Term::Let`.
    pub fn mk_let(&mut self, name: &str, ty: TermId, value: TermId, body: TermId) -> TermId {
        let n = self.symbol(name);
        self.add(Node::Let {
            name: n,
            ty,
            value,
            body,
        })
    }

    /// `Term::Fix`.
    pub fn mk_fix(&mut self, name: &str, body: TermId) -> TermId {
        let n = self.symbol(name);
        self.add(Node::Fix { name: n, body })
    }

    /// `Term::Lit`.
    pub fn mk_lit(&mut self, lit: &Literal) -> TermId {
        let l = self.intern_lit(lit);
        self.add(Node::Lit(l))
    }

    /// `Term::Hole`.
    pub fn mk_hole(&mut self) -> TermId {
        self.add(Node::Hole)
    }

    /// `Term::Const` — a universe-polymorphic global at explicit levels.
    pub fn mk_const(&mut self, name: &str, levels: &[Universe]) -> TermId {
        let n = self.symbol(name);
        let ls: Vec<LevelId> = levels.iter().map(|u| self.intern_level(u)).collect();
        self.add(Node::Const {
            name: n,
            levels: ls.into_boxed_slice(),
        })
    }

    /// `Term::Match`.
    pub fn mk_match(&mut self, discriminant: TermId, motive: TermId, cases: &[TermId]) -> TermId {
        self.add(Node::Match {
            discriminant,
            motive,
            cases: cases.to_vec().into_boxed_slice(),
        })
    }

    /// `Term::MutualFix` — every name in `defs` is bound in EVERY body.
    pub fn mk_mutual_fix(&mut self, defs: &[(&str, TermId)], index: usize) -> TermId {
        let ds: Vec<(Symbol, TermId)> = defs.iter().map(|(n, b)| (self.symbol(n), *b)).collect();
        self.add(Node::MutualFix {
            defs: ds.into_boxed_slice(),
            index,
        })
    }

    /// Intern a universe level, for a caller building a [`Node::Const`] by hand.
    pub fn level_of(&mut self, u: &Universe) -> LevelId {
        self.intern_level(u)
    }

    fn intern_level(&mut self, u: &Universe) -> LevelId {
        // Universe nesting is bounded by the level algebra, not by user terms, so this one
        // walk stays recursive by deliberate choice — see the module note on native
        // recursion, which is about TERMS.
        let node = match u {
            Universe::SProp => LevelNode::SProp,
            Universe::Prop => LevelNode::Prop,
            Universe::Type(n) => LevelNode::Type(*n),
            Universe::Var(v) => LevelNode::Var(self.names.intern(v)),
            Universe::Succ(a) => LevelNode::Succ(self.intern_level(a)),
            Universe::Max(a, b) => {
                let (x, y) = (self.intern_level(a), self.intern_level(b));
                LevelNode::Max(x, y)
            }
            Universe::IMax(a, b) => {
                let (x, y) = (self.intern_level(a), self.intern_level(b));
                LevelNode::IMax(x, y)
            }
        };
        if let Some(&id) = self.level_dedup.get(&node) {
            return id;
        }
        let hash = match node {
            LevelNode::SProp => mix(FNV_OFFSET, 101),
            LevelNode::Prop => mix(FNV_OFFSET, 102),
            LevelNode::Type(n) => mix(mix(FNV_OFFSET, 103), u64::from(n)),
            LevelNode::Var(s) => {
                let h = self.sym_hash(s);
                mix(mix(FNV_OFFSET, 104), h)
            }
            LevelNode::Succ(a) => mix(mix(FNV_OFFSET, 105), self.level_hashes[a.index()]),
            LevelNode::Max(a, b) => mix(
                mix(mix(FNV_OFFSET, 106), self.level_hashes[a.index()]),
                self.level_hashes[b.index()],
            ),
            LevelNode::IMax(a, b) => mix(
                mix(mix(FNV_OFFSET, 107), self.level_hashes[a.index()]),
                self.level_hashes[b.index()],
            ),
        };
        let id = LevelId(self.levels.len() as u32);
        self.levels.push(node);
        self.level_hashes.push(hash);
        self.level_dedup.insert(node, id);
        id
    }

    fn extract_level(&self, id: LevelId) -> Universe {
        match self.levels[id.index()] {
            LevelNode::SProp => Universe::SProp,
            LevelNode::Prop => Universe::Prop,
            LevelNode::Type(n) => Universe::Type(n),
            LevelNode::Var(s) => Universe::Var(self.names.resolve(s).to_string()),
            LevelNode::Succ(a) => Universe::Succ(Box::new(self.extract_level(a))),
            LevelNode::Max(a, b) => Universe::Max(
                Box::new(self.extract_level(a)),
                Box::new(self.extract_level(b)),
            ),
            LevelNode::IMax(a, b) => Universe::IMax(
                Box::new(self.extract_level(a)),
                Box::new(self.extract_level(b)),
            ),
        }
    }

    fn intern_lit(&mut self, lit: &Literal) -> LitId {
        let key = match lit {
            Literal::Int(x) => LitKey::Int(*x),
            Literal::Float(x) => LitKey::FloatBits(x.to_bits()),
            Literal::Text(s) => LitKey::Text(self.names.intern(s)),
            Literal::Duration(x) => LitKey::Duration(*x),
            Literal::Date(x) => LitKey::Date(*x),
            Literal::Moment(x) => LitKey::Moment(*x),
            Literal::BigInt(n) => LitKey::BigInt(n.clone()),
            Literal::Nat(n) => LitKey::Nat(n.clone()),
        };
        if let Some(&id) = self.lit_dedup.get(&key) {
            return id;
        }
        let hash = match &key {
            LitKey::Int(x) => mix(mix(FNV_OFFSET, 201), *x as u64),
            LitKey::FloatBits(b) => mix(mix(FNV_OFFSET, 202), *b),
            LitKey::Text(s) => {
                let h = self.sym_hash(*s);
                mix(mix(FNV_OFFSET, 203), h)
            }
            LitKey::Duration(x) => mix(mix(FNV_OFFSET, 204), *x as u64),
            LitKey::Date(x) => mix(mix(FNV_OFFSET, 205), i64::from(*x) as u64),
            LitKey::Moment(x) => mix(mix(FNV_OFFSET, 206), *x as u64),
            LitKey::BigInt(n) => bigint_hash(mix(FNV_OFFSET, 207), n),
            LitKey::Nat(n) => bigint_hash(mix(FNV_OFFSET, 208), n),
        };
        let id = LitId(self.lits.len() as u32);
        self.lits.push(key.clone());
        self.lit_hashes.push(hash);
        self.lit_dedup.insert(key, id);
        id
    }

    /// Borrow the heap payload size needed when extracting a literal.
    /// This excludes the Term/enum headers and allocator overhead. The handle
    /// must belong to this store, as with node() and level_view().
    pub fn literal_payload_bytes(&self, id: LitId) -> usize {
        match &self.lits[id.index()] {
            LitKey::Text(name) => self.name(*name).len(),
            LitKey::BigInt(n) | LitKey::Nat(n) => n.clone_payload_bytes(),
            LitKey::Int(_)
            | LitKey::FloatBits(_)
            | LitKey::Duration(_)
            | LitKey::Date(_)
            | LitKey::Moment(_) => 0,
        }
    }

    fn extract_lit(&self, id: LitId) -> Literal {
        match &self.lits[id.index()] {
            LitKey::Int(x) => Literal::Int(*x),
            LitKey::FloatBits(b) => Literal::Float(f64::from_bits(*b)),
            LitKey::Text(s) => Literal::Text(self.names.resolve(*s).to_string()),
            LitKey::Duration(x) => Literal::Duration(*x),
            LitKey::Date(x) => Literal::Date(*x),
            LitKey::Moment(x) => Literal::Moment(*x),
            LitKey::BigInt(n) => Literal::BigInt(n.clone()),
            LitKey::Nat(n) => Literal::Nat(n.clone()),
        }
    }

    // -- name sets ------------------------------------------------------------------------

    fn intern_set(&mut self, sorted: Vec<Symbol>) -> NameSetId {
        let boxed: Box<[Symbol]> = sorted.into_boxed_slice();
        if let Some(&id) = self.set_dedup.get(&boxed) {
            return id;
        }
        let mask = boxed.iter().fold(0u64, |m, &s| m | name_bit(s));
        let id = NameSetId(self.sets.len() as u32);
        self.sets.push(boxed.clone());
        self.set_masks.push(mask);
        self.set_dedup.insert(boxed, id);
        id
    }

    fn set(&self, id: NameSetId) -> &[Symbol] {
        &self.sets[id.0 as usize]
    }

    fn set_mask(&self, id: NameSetId) -> u64 {
        self.set_masks[id.0 as usize]
    }

    fn set_singleton(&mut self, s: Symbol) -> NameSetId {
        self.intern_set(vec![s])
    }

    fn set_union(&mut self, a: NameSetId, b: NameSetId) -> NameSetId {
        if a == b {
            return a;
        }
        if self.set(a).is_empty() {
            return b;
        }
        if self.set(b).is_empty() {
            return a;
        }
        let mut merged = Vec::with_capacity(self.set(a).len() + self.set(b).len());
        let (xs, ys) = (self.set(a), self.set(b));
        let (mut i, mut j) = (0, 0);
        while i < xs.len() || j < ys.len() {
            let take = match (xs.get(i), ys.get(j)) {
                (Some(x), Some(y)) if x < y => {
                    i += 1;
                    *x
                }
                (Some(x), Some(y)) if y < x => {
                    j += 1;
                    *y
                }
                (Some(x), Some(_)) => {
                    i += 1;
                    j += 1;
                    *x
                }
                (Some(x), None) => {
                    i += 1;
                    *x
                }
                (None, Some(y)) => {
                    j += 1;
                    *y
                }
                (None, None) => unreachable!(),
            };
            merged.push(take);
        }
        self.intern_set(merged)
    }

    fn set_remove(&mut self, a: NameSetId, s: Symbol) -> NameSetId {
        if self.set_mask(a) & name_bit(s) == 0 {
            return a;
        }
        if !self.set(a).contains(&s) {
            return a;
        }
        let kept: Vec<Symbol> = self.set(a).iter().copied().filter(|&x| x != s).collect();
        self.intern_set(kept)
    }

    fn set_insert(&mut self, a: NameSetId, s: Symbol) -> NameSetId {
        if self.set_mask(a) & name_bit(s) != 0 && self.set(a).contains(&s) {
            return a;
        }
        let mut v: Vec<Symbol> = self.set(a).to_vec();
        match v.binary_search(&s) {
            Ok(_) => return a,
            Err(pos) => v.insert(pos, s),
        }
        self.intern_set(v)
    }

    fn set_contains(&self, a: NameSetId, s: Symbol) -> bool {
        self.set_mask(a) & name_bit(s) != 0 && self.set(a).binary_search(&s).is_ok()
    }

    // -- annotations ----------------------------------------------------------------------

    /// Maximum height — 1 for a leaf. O(1): computed when the node was built.
    pub fn depth(&self, id: TermId) -> u32 {
        self.info[id.index()].depth
    }

    /// The node count an unfolding tree walk would report, saturating at `u64::MAX`. O(1).
    pub fn tree_size(&self, id: TermId) -> u64 {
        self.info[id.index()].tree_size
    }

    /// O(1) structural hash. Equal ids have equal hashes.
    pub fn structural_hash(&self, id: TermId) -> u64 {
        self.info[id.index()].hash
    }

    /// Whether the subterm contains no `Literal::Float`.
    ///
    /// The most conservative GUARD on using id equality as `Term` equality. `Literal` equality
    /// compares floats by bit identity, so id equality is faithful everywhere; this guard was
    /// correct under the old IEEE rule, when `NaN` and `±0.0` made the two part company, and
    /// stays correct under the current one. On a `float_free` term, `same(a, b)` and
    /// `extract(a) == extract(b)` agree EXACTLY.
    pub fn float_free(&self, id: TermId) -> bool {
        self.info[id.index()].float_free
    }

    /// Whether id equality is a FAITHFUL decision procedure for `Term` equality on this
    /// subterm: for any two `eq_faithful` ids, `same(a, b)` and `extract(a) == extract(b)`
    /// agree in BOTH directions.
    ///
    /// This is the precise condition; [`float_free`](TermStore::float_free) is the conservative
    /// one. A float that is neither a NaN nor a zero compares the same way under IEEE `==` and
    /// under bit equality, so a term may hold as many `1.5`s and infinities as it likes and
    /// still be safe to compare by identity under either rule; `NaN` and `±0.0` are the literals
    /// on which the two rules differ.
    ///
    /// It is SUFFICIENT rather than necessary: `Literal`'s equality is bit identity (the rule
    /// [`literal_identity_eq`] models independently), so id equality is faithful everywhere and
    /// this predicate is conservative. `store.same(a, b) && store.eq_faithful(a)` is correct under
    /// both rules.
    pub fn eq_faithful(&self, id: TermId) -> bool {
        self.info[id.index()].eq_faithful
    }

    /// Which [`NodeShape`]s occur anywhere in a subterm. O(1).
    ///
    /// This is Lean's `Expr.Data` `has*` bits widened to one per node kind — see [`ShapeMask`]
    /// for the citation and the trade-off. A reducer that does not implement ι can ask
    /// `!store.shapes(id).contains(NodeShape::Match)` without walking anything.
    pub fn shapes(&self, id: TermId) -> ShapeMask {
        self.info[id.index()].shapes
    }

    /// Every `Global` and `Const` NAME occurring anywhere in a subterm, sorted and without
    /// duplicates — the δ SURFACE. O(1) to reach, O(k) to hand back.
    ///
    /// It is the question a caller with no `Context` cannot answer for itself: whether a term
    /// still holds a δ redex depends on what those names MEAN here, and only the caller knows.
    pub fn constants(&self, id: TermId) -> Vec<&str> {
        // Sorted by the NAME and not by the symbol, because a symbol's index is interning
        // order: the same term in two stores would otherwise hand back two different orders,
        // and a set that crosses a boundary has to have one spelling.
        let mut names: Vec<&str> = self
            .set(self.info[id.index()].consts)
            .iter()
            .map(|&s| self.names.resolve(s))
            .collect();
        names.sort_unstable();
        names
    }

    /// Every BINDER name occurring anywhere in a subterm, sorted. O(1) to reach.
    ///
    /// The use is a guard rather than a query: a substitution pass that has to FRESHEN a binder
    /// mints a name this store chose, and a caller comparing its answer against another
    /// implementation's needs to know that no such name leaked into the result.
    pub fn binder_names(&self, id: TermId) -> Vec<&str> {
        self.set(self.info[id.index()].binders)
            .iter()
            .map(|&s| self.names.resolve(s))
            .collect()
    }

    /// Whether every binder name in `inner` also occurs as a binder in `outer` — the test that
    /// says a reduction invented no name of its own. O(k), over two sorted hash-consed sets.
    pub fn binders_within(&self, inner: TermId, outer: TermId) -> bool {
        let a = self.info[inner.index()].binders;
        let b = self.info[outer.index()].binders;
        if a == b {
            return true;
        }
        self.set(a).iter().all(|&s| self.set_contains(b, s))
    }

    /// Whether the only redexes this subterm can hold are β and ζ — so a reducer that
    /// implements exactly those two rules computes the same normal form as the full kernel.
    ///
    /// Two conditions, and the split between them is the point. The SHAPE condition is decided
    /// here, in O(1), from [`shapes`](TermStore::shapes): `Match` carries ι, `Fix` and
    /// `MutualFix` carry unrolling, `Const` carries δ plus a level well-formedness test, `Lit`
    /// is what the arithmetic primitives key their argument on, `Hole` is not a term, and
    /// `Bound` cannot be reached from a `Term` at all. The NAME condition belongs to the
    /// caller: whether a `Global` still has a δ body, or is one of the names a rule is keyed on
    /// rather than a shape, is a fact about the `Context` this store has never seen.
    pub fn is_beta_zeta_pure(&self, id: TermId, inert: &dyn Fn(&str) -> bool) -> bool {
        self.shapes(id).subset_of(ShapeMask::BETA_ZETA)
            && self
                .set(self.info[id.index()].consts)
                .iter()
                .all(|&c| inert(self.names.resolve(c)))
    }

    /// The free variable names of a subterm, sorted. Resolves and collects O(k)
    /// names, where k is the number of free names; the underlying symbol set is cached.
    pub fn free_vars(&self, id: TermId) -> Vec<&str> {
        #[cfg(test)]
        FREE_NAME_COLLECTIONS.with(|count| count.set(count.get() + 1));
        self.set(self.info[id.index()].free)
            .iter()
            .map(|&s| {
                #[cfg(test)]
                FREE_NAME_RESOLUTIONS.with(|count| count.set(count.get() + 1));
                self.names.resolve(s)
            })
            .collect()
    }

    /// Whether the exact free-name set is empty, including annotations and all
    /// binder scopes. O(1): the full free-symbol set was cached at construction.
    /// This says nothing about internal de Bruijn Bound nodes.
    pub fn free_names_are_empty(&self, id: TermId) -> bool {
        self.set(self.info[id.index()].free).is_empty()
    }

    /// Borrow the cached free-name symbols without resolving or allocating names.
    /// Lexical dependencies observed by the lazy constructor machine.
    ///
    /// That machine never inspects lambda domains, let annotations, or match
    /// motives. This is NOT a typing or definitional-equality cache key: two
    /// closures sharing it may still have different types. It is valid only for
    /// evaluation to the same atomic constructor after the full terms have been
    /// checked. Constructor arguments, including proofs, are all retained.
    pub(crate) fn head_reduction_symbols(&mut self, root: TermId) -> &[Symbol] {
        if let Some(&set) = self.head_dependencies.get(&root) {
            return self.set(set);
        }
        let mut work = vec![(root, false)];
        while let Some((id, ready)) = work.pop() {
            if self.head_dependencies.contains_key(&id) {
                continue;
            }
            if !ready {
                work.push((id, true));
                match self.node(id) {
                    Node::Lambda { body, .. } | Node::Fix { body, .. } => work.push((*body, false)),
                    Node::App(f, a) => {
                        work.push((*f, false));
                        work.push((*a, false));
                    }
                    Node::Let { value, body, .. } => {
                        work.push((*value, false));
                        work.push((*body, false));
                    }
                    Node::Match {
                        discriminant,
                        cases,
                        ..
                    } => {
                        work.push((*discriminant, false));
                        work.extend(cases.iter().map(|c| (*c, false)));
                    }
                    _ => {}
                }
                continue;
            }
            let free = match self.node(id).clone() {
                Node::Lambda { param, body, .. } => {
                    self.set_remove(self.head_dependencies[&body], param)
                }
                Node::Fix { name, body } => self.set_remove(self.head_dependencies[&body], name),
                Node::App(f, a) => {
                    self.set_union(self.head_dependencies[&f], self.head_dependencies[&a])
                }
                Node::Let {
                    name, value, body, ..
                } => {
                    let body = self.set_remove(self.head_dependencies[&body], name);
                    self.set_union(self.head_dependencies[&value], body)
                }
                Node::Match {
                    discriminant,
                    cases,
                    ..
                } => {
                    let mut free = self.head_dependencies[&discriminant];
                    for case in cases {
                        free = self.set_union(free, self.head_dependencies[&case]);
                    }
                    free
                }
                _ => self.info[id.index()].free,
            };
            self.head_dependencies.insert(id, free);
        }
        self.set(self.head_dependencies[&root])
    }

    /// Whether `name` occurs free. O(1) mask test, then a binary search.
    pub fn occurs_free(&self, id: TermId, name: &str) -> bool {
        match self.names.lookup(name) {
            Some(s) => self.set_contains(self.info[id.index()].free, s),
            None => false,
        }
    }

    /// Whether `name` occurs free, by symbol.
    pub fn occurs_free_sym(&self, id: TermId, s: Symbol) -> bool {
        self.set_contains(self.info[id.index()].free, s)
    }

    /// How many DISTINCT nodes `id` reaches — the DAG size, which is what a checker keyed on
    /// identity actually pays.
    pub fn dag_size(&self, id: TermId) -> usize {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![id];
        let mut count = 0;
        while let Some(t) = stack.pop() {
            if seen[t.index()] {
                continue;
            }
            seen[t.index()] = true;
            count += 1;
            self.nodes[t.index()].for_each_child(&mut |c| stack.push(c));
        }
        count
    }

    /// Whether two ids denote the same term — ONE integer compare.
    pub fn same(&self, a: TermId, b: TermId) -> bool {
        a == b
    }

    // -- local typing environments -----------------------------------------------------------

    /// Intern a local typing environment — `(name, type)` pairs, sorted by name.
    ///
    /// Two `Context`s that assign the same types to the same names get the same [`EnvId`],
    /// whatever order they were built in and whatever else they contain.
    ///
    /// # Panics
    ///
    /// Panics if a name appears twice. An environment is a FUNCTION from names to types; two
    /// entries for one name is a caller that has not decided which scope it is in, and silently
    /// keeping one of them is how a cache starts answering for the wrong context.
    pub fn env(&mut self, entries: &[(Symbol, TermId)]) -> EnvId {
        let mut sorted = entries.to_vec();
        sorted.sort_by_key(|(s, _)| *s);
        for w in sorted.windows(2) {
            assert_ne!(
                w[0].0,
                w[1].0,
                "a local typing environment assigns one type per name; `{}` was given two",
                self.names.resolve(w[0].0)
            );
        }
        let boxed: Box<[(Symbol, TermId)]> = sorted.into_boxed_slice();
        if let Some(&id) = self.env_dedup.get(&boxed) {
            return id;
        }
        let id = EnvId(self.envs.len() as u32);
        self.envs.push(boxed.clone());
        self.env_dedup.insert(boxed, id);
        id
    }

    /// The environment fragment that DECIDES `id`'s type: the types `lookup` assigns to exactly
    /// the free variables of `id`, and nothing else.
    ///
    /// This is the key half that makes an `inferType` cache sound over a NAMED term
    /// representation, and it is O(|free variables|) rather than O(|context|) because the free
    /// set was computed at construction and is itself hash-consed.
    ///
    /// Two properties make it the RIGHT key rather than merely a safe one:
    ///
    /// * A **closed** subterm gets [`EnvId::EMPTY`] under every context, so it is checked ONCE
    ///   for the whole call however many scopes it occurs under. That is where the win is.
    /// * A variable the context does not bind is OMITTED, not defaulted, so "`x` is unbound" and
    ///   "`x : Nat`" are different keys and the cached refusal cannot leak into a scope that
    ///   binds it.
    pub fn env_for(
        &mut self,
        id: TermId,
        lookup: &mut impl FnMut(&str) -> Option<TermId>,
    ) -> EnvId {
        let free = self.info[id.index()].free;
        let names: Vec<Symbol> = self.set(free).to_vec();
        let mut entries: Vec<(Symbol, TermId)> = Vec::with_capacity(names.len());
        for &sym in &names {
            let resolved = self.names.resolve(sym);
            if let Some(ty) = lookup(resolved) {
                entries.push((sym, ty));
            }
        }
        self.env(&entries)
    }

    /// The entries of an interned environment, sorted by name.
    pub fn env_entries(&self, e: EnvId) -> &[(Symbol, TermId)] {
        &self.envs[e.index()]
    }

    /// How many environments have been interned — the measure that says a descent is reusing
    /// its scopes rather than minting a new one per binder.
    pub fn env_count(&self) -> usize {
        self.envs.len()
    }

    // -- interning a `Term` ---------------------------------------------------------------

    /// Intern a `Term`. Iterative: a term the native stack could not walk still interns.
    pub fn intern(&mut self, term: &Term) -> TermId {
        enum Task<'a> {
            Enter(&'a Term),
            Exit(&'a Term, usize),
        }
        let mut stack: Vec<Task> = vec![Task::Enter(term)];
        let mut out: Vec<TermId> = Vec::new();
        let mut kids: Vec<TermId> = Vec::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Enter(t) => {
                    #[cfg(test)]
                    INTERN_VISITS.with(|count| count.set(count.get().saturating_add(1)));
                    let mut n = 0usize;
                    term_for_each_child(t, &mut |_| n += 1);
                    stack.push(Task::Exit(t, n));
                    term_for_each_child(t, &mut |c| stack.push(Task::Enter(c)));
                }
                Task::Exit(t, n) => {
                    let base = out.len() - n;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    let node = self.node_from(t, &kids);
                    let id = self.add(node);
                    out.push(id);
                }
            }
        }
        debug_assert_eq!(out.len(), 1, "interning leaves exactly the root");
        out[0]
    }

    /// Build the flat node for `t` given its already-interned children, in `for_each_child`
    /// order. Wildcard-free: a new `Term` variant must answer here.
    fn node_from(&mut self, t: &Term, kids: &[TermId]) -> Node {
        match t {
            Term::Sort(u) => {
                let l = self.intern_level(u);
                Node::Sort(l)
            }
            Term::Var(n) => Node::Var(self.names.intern(n)),
            Term::Global(n) => Node::Global(self.names.intern(n)),
            Term::Const { name, levels } => {
                let nm = self.names.intern(name);
                let ls: Vec<LevelId> = levels.iter().map(|u| self.intern_level(u)).collect();
                Node::Const {
                    name: nm,
                    levels: ls.into_boxed_slice(),
                }
            }
            Term::Pi { param, .. } => {
                let p = self.names.intern(param);
                Node::Pi {
                    param: p,
                    param_type: kids[0],
                    body_type: kids[1],
                }
            }
            Term::Lambda { param, .. } => {
                let p = self.names.intern(param);
                Node::Lambda {
                    param: p,
                    param_type: kids[0],
                    body: kids[1],
                }
            }
            Term::App(_, _) => Node::App(kids[0], kids[1]),
            Term::Match { .. } => Node::Match {
                discriminant: kids[0],
                motive: kids[1],
                cases: kids[2..].to_vec().into_boxed_slice(),
            },
            Term::Fix { name, .. } => {
                let n = self.names.intern(name);
                Node::Fix {
                    name: n,
                    body: kids[0],
                }
            }
            Term::MutualFix { defs, index } => {
                let ds: Vec<(Symbol, TermId)> = defs
                    .iter()
                    .zip(kids.iter())
                    .map(|((n, _), c)| (self.names.intern(n), *c))
                    .collect();
                Node::MutualFix {
                    defs: ds.into_boxed_slice(),
                    index: *index,
                }
            }
            Term::Let { name, .. } => {
                let n = self.names.intern(name);
                Node::Let {
                    name: n,
                    ty: kids[0],
                    value: kids[1],
                    body: kids[2],
                }
            }
            Term::Lit(l) => {
                let lid = self.intern_lit(l);
                Node::Lit(lid)
            }
            Term::Hole => Node::Hole,
            // The store keys on the ERASED term: an annotated term IS its term's node, so an
            // annotation never splits sharing and `intern(annotated) == intern(plain)`.
            Term::Annotated { .. } => self.node(kids[0]).clone(),
        }
    }

    /// Materialise a `Term` from an id. Iterative — but note that the `Term` it returns is a
    /// TREE, so DROPPING it is native recursion again, and unfolding a DAG can be
    /// exponentially larger than the DAG. [`extract_bounded`](TermStore::extract_bounded) is
    /// the guarded form.
    pub fn extract(&self, id: TermId) -> Term {
        enum Task {
            Enter(TermId),
            Exit(TermId, usize),
        }
        let mut stack = vec![Task::Enter(id)];
        let mut out: Vec<Term> = Vec::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Enter(t) => {
                    let node = &self.nodes[t.index()];
                    let n = node.child_count();
                    stack.push(Task::Exit(t, n));
                    node.for_each_child(&mut |c| stack.push(Task::Enter(c)));
                }
                Task::Exit(t, n) => {
                    let base = out.len() - n;
                    let mut kids: Vec<Term> = out.split_off(base);
                    kids.reverse();
                    out.push(self.term_from(&self.nodes[t.index()], kids));
                }
            }
        }
        debug_assert_eq!(out.len(), 1, "extraction leaves exactly the root");
        out.pop().expect("extraction leaves exactly the root")
    }

    /// [`extract`](TermStore::extract), refusing an unfolding bigger than `limit` nodes
    /// instead of trying to allocate it.
    pub fn extract_bounded(&self, id: TermId, limit: u64) -> Result<Term, TooLarge> {
        let size = self.tree_size(id);
        if size > limit {
            return Err(TooLarge {
                tree_size: size,
                limit,
            });
        }
        Ok(self.extract(id))
    }

    fn term_from(&self, node: &Node, kids: Vec<Term>) -> Term {
        let mut kids = kids.into_iter();
        match node {
            Node::Sort(l) => Term::Sort(self.extract_level(*l)),
            Node::Var(s) => Term::Var(self.names.resolve(*s).to_string()),
            Node::Global(s) => Term::Global(self.names.resolve(*s).to_string()),
            Node::Const { name, levels } => Term::Const {
                name: self.names.resolve(*name).to_string(),
                levels: levels.iter().map(|&l| self.extract_level(l)).collect(),
            },
            Node::Pi { param, .. } => Term::Pi {
                param: self.names.resolve(*param).to_string(),
                param_type: Box::new(kids.next().expect("Pi has a domain")),
                body_type: Box::new(kids.next().expect("Pi has a codomain")),
            },
            Node::Lambda { param, .. } => Term::Lambda {
                param: self.names.resolve(*param).to_string(),
                param_type: Box::new(kids.next().expect("λ has a domain")),
                body: Box::new(kids.next().expect("λ has a body")),
            },
            Node::App(_, _) => Term::App(
                Box::new(kids.next().expect("App has a function")),
                Box::new(kids.next().expect("App has an argument")),
            ),
            Node::Match { .. } => Term::Match {
                discriminant: Box::new(kids.next().expect("match has a discriminant")),
                motive: Box::new(kids.next().expect("match has a motive")),
                cases: kids.collect(),
            },
            Node::Fix { name, .. } => Term::Fix {
                name: self.names.resolve(*name).to_string(),
                body: Box::new(kids.next().expect("fix has a body")),
            },
            Node::MutualFix { defs, index } => Term::MutualFix {
                defs: defs
                    .iter()
                    .map(|(n, _)| self.names.resolve(*n).to_string())
                    .zip(kids)
                    .collect(),
                index: *index,
            },
            Node::Let { name, .. } => Term::Let {
                name: self.names.resolve(*name).to_string(),
                ty: Box::new(kids.next().expect("let has a type")),
                value: Box::new(kids.next().expect("let has a value")),
                body: Box::new(kids.next().expect("let has a body")),
            },
            Node::Lit(l) => Term::Lit(self.extract_lit(*l)),
            Node::Hole => Term::Hole,
            // Only an α-CANONICAL form holds a `Bound`, and a canonical form is a KEY, not a
            // term: this spelling exists so extraction stays total and printable, and
            // `intern(extract(canonical))` is deliberately NOT the identity.
            Node::Bound(level) => Term::Var(format!("{CANON_OCCURRENCE}{level}⟫")),
        }
    }

    // -- identity-keyed traversal ---------------------------------------------------------

    /// Fold over the DAG, children before parents, visiting every DISTINCT node EXACTLY
    /// ONCE and remembering the answer in `cache`.
    ///
    /// This is the shape `infer_type` has to take for a shared subterm to be checked once
    /// instead of once per occurrence. Iterative, so depth costs heap and not stack.
    pub fn fold<V: Clone>(
        &self,
        root: TermId,
        cache: &mut IdCache<V>,
        f: &mut dyn FnMut(&TermStore, TermId, &Node, &[V]) -> V,
    ) -> V {
        enum Task {
            Enter(TermId),
            Exit(TermId, usize),
        }
        let mut stack = vec![Task::Enter(root)];
        let mut out: Vec<V> = Vec::new();
        let mut kids: Vec<V> = Vec::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Enter(t) => {
                    if let Some(v) = cache.get(t) {
                        out.push(v.clone());
                        continue;
                    }
                    let node = &self.nodes[t.index()];
                    stack.push(Task::Exit(t, node.child_count()));
                    node.for_each_child(&mut |c| stack.push(Task::Enter(c)));
                }
                Task::Exit(t, n) => {
                    let base = out.len() - n;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    let v = f(self, t, &self.nodes[t.index()], &kids);
                    cache.insert(t, v.clone());
                    out.push(v);
                }
            }
        }
        out.pop().expect("a fold leaves exactly the root")
    }

    // -- occurrence counting --------------------------------------------------------------

    /// How many FREE occurrences of `name` a subterm has, saturating at `u32::MAX`.
    ///
    /// Memoised on node identity, so a shared subterm is counted once. This is the measure
    /// an occurrence-gated ζ rule needs: a `let` whose name occurs at most once can be
    /// expanded for free, and one that occurs more must KEEP the `let` as its sharing node.
    pub fn occurrence_count(&mut self, id: TermId, name: Symbol) -> u32 {
        enum Task {
            Enter(TermId),
            Exit(TermId, usize),
        }
        if let Some(&n) = self.occ_cache.get(&(name, id)) {
            return n;
        }
        let mut stack = vec![Task::Enter(id)];
        let mut out: Vec<u32> = Vec::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Enter(t) => {
                    if let Some(&n) = self.occ_cache.get(&(name, t)) {
                        out.push(n);
                        continue;
                    }
                    if !self.occurs_free_sym(t, name) {
                        self.occ_cache.insert((name, t), 0);
                        out.push(0);
                        continue;
                    }
                    let node = &self.nodes[t.index()];
                    stack.push(Task::Exit(t, node.child_count()));
                    node.for_each_child(&mut |c| stack.push(Task::Enter(c)));
                }
                Task::Exit(t, n) => {
                    let base = out.len() - n;
                    let mut kids: Vec<u32> = out.split_off(base);
                    kids.reverse();
                    let node = &self.nodes[t.index()];
                    let total = if let Node::Var(_) = node {
                        1
                    } else {
                        let (bound, scoped) = node.binding();
                        let shadows = bound.contains(&name);
                        kids.iter()
                            .enumerate()
                            .filter(|(i, _)| !(shadows && scoped.contains(i)))
                            .fold(0u32, |acc, (_, &k)| acc.saturating_add(k))
                    };
                    self.occ_cache.insert((name, t), total);
                    out.push(total);
                }
            }
        }
        out.pop()
            .expect("occurrence counting leaves exactly the root")
    }

    /// [`occurrence_count`](TermStore::occurrence_count), classified — all an occurrence gate
    /// needs to decide.
    pub fn occurrences(&mut self, id: TermId, name: Symbol) -> Occurrences {
        match self.occurrence_count(id, name) {
            0 => Occurrences::Zero,
            1 => Occurrences::Once,
            _ => Occurrences::Many,
        }
    }

    // -- substitution ---------------------------------------------------------------------

    fn subst_of(&mut self, mut pairs: Vec<(Symbol, TermId)>) -> SubstId {
        pairs.sort_by_key(|(s, _)| *s);
        pairs.dedup_by_key(|(s, _)| *s);
        let dom: Box<[Symbol]> = pairs.iter().map(|(s, _)| *s).collect();
        let to: Box<[TermId]> = pairs.iter().map(|(_, t)| *t).collect();
        if dom.is_empty() {
            return SubstId::EMPTY;
        }
        if let Some(&id) = self.subst_dedup.get(&(dom.clone(), to.clone())) {
            return id;
        }
        let dom_mask = dom.iter().fold(0u64, |m, &s| m | name_bit(s));
        let mut range_free = NameSetId(0);
        for &t in to.iter() {
            let f = self.info[t.index()].free;
            range_free = self.set_union(range_free, f);
        }
        let id = SubstId(self.substs.len() as u32);
        self.substs.push(Subst {
            dom: dom.clone(),
            to: to.clone(),
            dom_mask,
            range_free,
        });
        self.subst_dedup.insert((dom, to), id);
        id
    }

    fn subst_without(&mut self, sid: SubstId, names: &[Symbol]) -> SubstId {
        let s = &self.substs[sid.0 as usize];
        if names.iter().all(|n| !s.dom.contains(n)) {
            return sid;
        }
        let pairs: Vec<(Symbol, TermId)> = s
            .dom
            .iter()
            .zip(s.to.iter())
            .filter(|(d, _)| !names.contains(d))
            .map(|(d, t)| (*d, *t))
            .collect();
        self.subst_of(pairs)
    }

    fn subst_with(&mut self, sid: SubstId, name: Symbol, to: TermId) -> SubstId {
        let s = &self.substs[sid.0 as usize];
        let mut pairs: Vec<(Symbol, TermId)> = s
            .dom
            .iter()
            .zip(s.to.iter())
            .map(|(d, t)| (*d, *t))
            .collect();
        pairs.retain(|(d, _)| *d != name);
        pairs.push((name, to));
        self.subst_of(pairs)
    }

    /// A name not free in `body`, not bound anywhere in `body`, and not touched by `sid`.
    fn fresh_like(&mut self, base: Symbol, body: TermId, sid: SubstId) -> Symbol {
        let info = self.info[body.index()];
        loop {
            self.fresh_counter += 1;
            let candidate = format!("{}✦{}", self.names.resolve(base), self.fresh_counter);
            let s = self.names.intern(&candidate);
            let sub = &self.substs[sid.0 as usize];
            let clashes = self.set_contains(info.free, s)
                || self.set_contains(info.binders, s)
                || self.set_contains(sub.range_free, s)
                || sub.dom.contains(&s);
            if !clashes {
                return s;
            }
        }
    }

    /// Capture-avoiding substitution of ONE name.
    ///
    /// Freshening is STORE-SCOPED: a binder that has to be renamed takes the next name from
    /// this store's counter, so the same substitution asked for twice returns the identical id
    /// (the memo answers the second time), while the same substitution in a fresh store — or
    /// after [`clear_caches`](TermStore::clear_caches) — can pick a different fresh name and
    /// return an α-equivalent answer with a different id. Compare such answers with
    /// [`alpha_same`](TermStore::alpha_same), never with `==`.
    pub fn substitute(&mut self, body: TermId, name: Symbol, value: TermId) -> TermId {
        self.note_rename_risk(body, value);
        let sid = self.subst_of(vec![(name, value)]);
        self.apply(sid, body)
    }

    /// Record whether this substitution could make a capture-avoiding pass invent a name —
    /// `binders(body) ∩ free(value) ≠ ∅`. O(1) when the bloom masks are disjoint, which is the
    /// common case; an exact walk of the smaller sorted set otherwise.
    fn note_rename_risk(&mut self, body: TermId, value: TermId) {
        if self.rename_risk {
            return;
        }
        let binders = self.info[body.index()].binders;
        let free = self.info[value.index()].free;
        if self.set_mask(binders) & self.set_mask(free) == 0 {
            return;
        }
        let hit = self
            .set(free)
            .iter()
            .any(|&s| self.set_contains(binders, s));
        self.rename_risk = hit;
    }

    /// SIMULTANEOUS capture-avoiding substitution — Lean's `instantiateRev` shape.
    ///
    /// A whole telescope's arguments go in together, in ONE pass, so instantiating an
    /// `n`-binder telescope costs one walk and not `n`. `pairs` is treated as a simultaneous
    /// map: no `to` term is ever re-scanned for another pair's name.
    pub fn instantiate(&mut self, body: TermId, pairs: &[(Symbol, TermId)]) -> TermId {
        for &(_, value) in pairs {
            self.note_rename_risk(body, value);
        }
        let sid = self.subst_of(pairs.to_vec());
        self.apply(sid, body)
    }

    /// Apply an interned substitution. Iterative, memoised on `(substitution, node)`.
    ///
    /// The line that makes a deep spine linear is the mask test: a node whose free variables
    /// cannot meet the substitution's domain is returned UNCHANGED in O(1), so a beta step
    /// near the head of a 200,000-deep spine does not re-walk the 199,999 nodes below it.
    fn apply(&mut self, sid: SubstId, root: TermId) -> TermId {
        enum Op {
            Visit(SubstId, TermId),
            Build {
                sid: SubstId,
                id: TermId,
                n: usize,
                binders: Option<Vec<Symbol>>,
            },
        }
        if sid == SubstId::EMPTY {
            return root;
        }
        let mut stack = vec![Op::Visit(sid, root)];
        let mut out: Vec<TermId> = Vec::new();
        let mut kids: Vec<TermId> = Vec::new();
        while let Some(op) = stack.pop() {
            match op {
                Op::Visit(s, t) => {
                    if s == SubstId::EMPTY {
                        out.push(t);
                        continue;
                    }
                    let info = self.info[t.index()];
                    let sub_mask = self.substs[s.0 as usize].dom_mask;
                    if self.set_mask(info.free) & sub_mask == 0 {
                        out.push(t);
                        continue;
                    }
                    if let Some(&r) = self.subst_cache.get(&(s, t)) {
                        out.push(r);
                        continue;
                    }
                    let free = self.set(info.free).to_vec();
                    let dom = self.substs[s.0 as usize].dom.clone();
                    if !free.iter().any(|v| dom.contains(v)) {
                        out.push(t);
                        continue;
                    }
                    // A variable in the domain resolves immediately.
                    if let Node::Var(v) = self.nodes[t.index()] {
                        let sub = &self.substs[s.0 as usize];
                        if let Some(pos) = sub.dom.iter().position(|d| *d == v) {
                            let r = sub.to[pos];
                            self.subst_cache.insert((s, t), r);
                            out.push(r);
                            continue;
                        }
                        out.push(t);
                        continue;
                    }
                    let node = self.nodes[t.index()].clone();
                    let (bound, scoped) = node.binding();
                    // Restrict under the binder, then rename it if the restricted
                    // substitution would carry the binder's own name into its scope.
                    let mut inner = self.subst_without(s, &bound);
                    let mut renamed: Option<Vec<Symbol>> = None;
                    if !bound.is_empty() && inner != SubstId::EMPTY {
                        // Freshness is judged against the WHOLE node, not one scoped child:
                        // a `MutualFix` binds every name in every body, so a name that is
                        // safe in body 0 and captured in body 1 would be a silent capture.
                        // The node's own free/binder sets are a superset of every child's,
                        // which is conservative in the only safe direction.
                        let mut new_names = Vec::with_capacity(bound.len());
                        let mut any = false;
                        for &b in &bound {
                            let carries =
                                self.set_contains(self.substs[inner.0 as usize].range_free, b);
                            if carries {
                                let fresh = self.fresh_like(b, t, inner);
                                let v = self.mk_var_sym(fresh);
                                inner = self.subst_with(inner, b, v);
                                new_names.push(fresh);
                                any = true;
                            } else {
                                new_names.push(b);
                            }
                        }
                        if any {
                            renamed = Some(new_names);
                        }
                    }
                    let n = node.child_count();
                    stack.push(Op::Build {
                        sid: s,
                        id: t,
                        n,
                        binders: renamed,
                    });
                    let mut i = 0usize;
                    let mut child_ops = Vec::with_capacity(n);
                    node.for_each_child(&mut |c| {
                        let cs = if scoped.contains(&i) { inner } else { s };
                        child_ops.push(Op::Visit(cs, c));
                        i += 1;
                    });
                    for op in child_ops {
                        stack.push(op);
                    }
                }
                Op::Build {
                    sid: s,
                    id: t,
                    n,
                    binders,
                } => {
                    let base = out.len() - n;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    let node = self.nodes[t.index()].clone();
                    let rebuilt = match &binders {
                        Some(names) => node.with_binders(names).rebuild_with(&kids),
                        None => node.rebuild_with(&kids),
                    };
                    let r = self.add(rebuilt);
                    self.subst_cache.insert((s, t), r);
                    out.push(r);
                }
            }
        }
        out.pop().expect("substitution leaves exactly the root")
    }

    fn spend_operation_work(work: &mut u64, amount: u64) -> Result<(), StoreOperationLimit> {
        *work = work.checked_sub(amount).ok_or(StoreOperationLimit::Work)?;
        Ok(())
    }

    /// Conservative cumulative auxiliary-allocation credits, in 64-byte units. The caller
    /// must share one finite allowance across all bounded operations in its private store.
    /// Credits cover payload copies and amortized collection growth, not exact RSS bytes.
    fn admit_operation_allocation(
        allocation: &mut u64,
        units: u64,
    ) -> Result<(), StoreOperationLimit> {
        *allocation = allocation
            .checked_sub(units)
            .ok_or(StoreOperationLimit::Allocation)?;
        Ok(())
    }

    /// Admit a conservative number of new metadata sets before a separately bounded raw
    /// operation. This only reads the current dense inventory and allocates nothing.
    pub(crate) fn admit_name_set_growth(&self, additional: u64) -> Result<(), StoreOperationLimit> {
        Self::admit_name_set_inventory(self.sets.len(), additional)
    }

    /// Check the dense NameSetId inventory before an operation may intern new sets.
    /// Kept separate from allocation credits: a caller can supply an enormous credit budget.
    fn admit_name_set_inventory(
        current: usize,
        additional: u64,
    ) -> Result<(), StoreOperationLimit> {
        if (current as u64)
            .checked_add(additional)
            .is_none_or(|n| n > u32::MAX as u64)
        {
            return Err(StoreOperationLimit::Nodes);
        }
        Ok(())
    }

    /// Upper bound for subst_of's repeated range unions, including domain copies/sorting.
    fn admit_substitution_ranges(
        &self,
        sid: Option<SubstId>,
        pairs: &[(Symbol, TermId)],
        extra: usize,
        work: &mut u64,
        allocation: &mut u64,
    ) -> Result<(), StoreOperationLimit> {
        let (mut count, mut names) = (extra as u64, 0u64);
        if let Some(sid) = sid {
            let sub = &self.substs[sid.0 as usize];
            count = count.saturating_add(sub.to.len() as u64);
            for &child in sub.to.iter() {
                Self::spend_operation_work(work, 1)?;
                names = names.saturating_add(self.set(self.info[child.index()].free).len() as u64);
            }
        }
        for &(_, child) in pairs {
            Self::spend_operation_work(work, 1)?;
            count = count.saturating_add(1);
            names = names.saturating_add(self.set(self.info[child.index()].free).len() as u64);
        }
        // Each range union can intern at most one new set, including a restricted or
        // extended substitution. Admit before subst_of creates any intermediate union.
        Self::admit_name_set_inventory(self.sets.len(), count)?;
        // Each extra fresh variable contributes at most one free symbol.
        names = names.saturating_add(extra as u64);
        let units = count
            .saturating_add(1)
            .saturating_mul(names.saturating_add(count).saturating_add(1))
            .saturating_mul(8)
            .saturating_add(32);
        Self::spend_operation_work(work, units)?;
        Self::admit_operation_allocation(allocation, units)
    }

    /// Capture-avoiding simultaneous substitution with admission before traversal/metadata
    /// allocation and before each fresh/rebuilt node. On failure the store retains bounded
    /// partial memo entries; no partial result is returned. This leaves `instantiate` unchanged.
    pub(crate) fn instantiate_bounded(
        &mut self,
        body: TermId,
        pairs: &[(Symbol, TermId)],
        work: &mut u64,
        node_limit: usize,
        allocation: &mut u64,
    ) -> Result<TermId, StoreOperationLimit> {
        if self.nodes.len() > node_limit
            || node_limit > u32::MAX as usize
            || self.substs.len() >= node_limit
            || self.subst_cache.len() >= node_limit
        {
            return Err(StoreOperationLimit::Nodes);
        }
        if pairs.len() > node_limit {
            return Err(StoreOperationLimit::Nodes);
        }
        self.admit_substitution_ranges(None, pairs, 0, work, allocation)?;
        // Substitution construction unions the ranges' free-name sets and copies the domain.
        Self::spend_operation_work(
            work,
            ((pairs.len() as u64) + 1).saturating_mul((self.nodes.len() as u64).saturating_add(1)),
        )?;
        for &(_, value) in pairs {
            self.note_rename_risk(body, value);
        }
        let sid = self.subst_of(pairs.to_vec());
        self.apply_bounded(sid, body, work, node_limit, allocation)
    }

    fn fresh_like_bounded(
        &mut self,
        base: Symbol,
        body: TermId,
        sid: SubstId,
        work: &mut u64,
        allocation: &mut u64,
    ) -> Result<Symbol, StoreOperationLimit> {
        let info = self.info[body.index()];
        loop {
            Self::spend_operation_work(work, (self.substs[sid.0 as usize].dom.len() as u64) + 1)?;
            self.fresh_counter = self
                .fresh_counter
                .checked_add(1)
                .ok_or(StoreOperationLimit::Work)?;
            let bytes = (self.names.resolve(base).len() as u64).saturating_add(32);
            Self::spend_operation_work(work, bytes)?;
            Self::admit_operation_allocation(allocation, bytes.saturating_add(64))?;
            let candidate = format!("{}✦{}", self.names.resolve(base), self.fresh_counter);
            let symbol = self.names.intern(&candidate);
            let sub = &self.substs[sid.0 as usize];
            if !self.set_contains(info.free, symbol)
                && !self.set_contains(info.binders, symbol)
                && !self.set_contains(sub.range_free, symbol)
                && !sub.dom.contains(&symbol)
            {
                return Ok(symbol);
            }
        }
    }

    fn apply_bounded(
        &mut self,
        sid: SubstId,
        root: TermId,
        work: &mut u64,
        node_limit: usize,
        allocation: &mut u64,
    ) -> Result<TermId, StoreOperationLimit> {
        enum Op {
            Visit(SubstId, TermId),
            Build {
                sid: SubstId,
                id: TermId,
                n: usize,
                binders: Option<Vec<Symbol>>,
            },
        }
        if sid == SubstId::EMPTY {
            return Ok(root);
        }
        Self::admit_operation_allocation(allocation, 16)?;
        let mut stack = vec![Op::Visit(sid, root)];
        let mut out: Vec<TermId> = Vec::new();
        let mut kids: Vec<TermId> = Vec::new();
        while let Some(op) = stack.pop() {
            Self::spend_operation_work(work, 1)?;
            if self.substs.len() >= node_limit || self.subst_cache.len() >= node_limit {
                return Err(StoreOperationLimit::Nodes);
            }
            // Every output/cache insertion below is preceded by capacity and cumulative
            // admission. Charge even cache hits conservatively, before Vec/HashMap growth.
            if out.len() >= node_limit || stack.len() > node_limit {
                return Err(StoreOperationLimit::Nodes);
            }
            Self::admit_operation_allocation(allocation, 8)?;
            match op {
                Op::Visit(s, t) => {
                    if s == SubstId::EMPTY {
                        out.push(t);
                        continue;
                    }
                    let info = self.info[t.index()];
                    let sub_mask = self.substs[s.0 as usize].dom_mask;
                    if self.set_mask(info.free) & sub_mask == 0 {
                        out.push(t);
                        continue;
                    }
                    if let Some(&r) = self.subst_cache.get(&(s, t)) {
                        out.push(r);
                        continue;
                    }
                    let free_len = self.set(info.free).len() as u64;
                    let dom_len = self.substs[s.0 as usize].dom.len() as u64;
                    Self::spend_operation_work(
                        work,
                        free_len
                            .saturating_add(1)
                            .saturating_mul(dom_len.saturating_add(1)),
                    )?;
                    Self::admit_operation_allocation(
                        allocation,
                        free_len.saturating_add(dom_len).saturating_add(8),
                    )?;
                    let free = self.set(info.free).to_vec();
                    let dom = self.substs[s.0 as usize].dom.clone();
                    if !free.iter().any(|v| dom.contains(v)) {
                        out.push(t);
                        continue;
                    }
                    // A variable in the domain resolves immediately.
                    if let Node::Var(v) = self.nodes[t.index()] {
                        let sub = &self.substs[s.0 as usize];
                        if let Some(pos) = sub.dom.iter().position(|d| *d == v) {
                            let r = sub.to[pos];
                            self.subst_cache.insert((s, t), r);
                            out.push(r);
                            continue;
                        }
                        out.push(t);
                        continue;
                    }
                    let n = self.nodes[t.index()].child_count();
                    if stack
                        .len()
                        .checked_add(n)
                        .and_then(|v| v.checked_add(1))
                        .is_none_or(|v| v > node_limit)
                        || n > node_limit.saturating_sub(out.len())
                    {
                        return Err(StoreOperationLimit::Nodes);
                    }
                    // Covers original-node clone, binding vectors, new_names and child ops.
                    Self::spend_operation_work(
                        work,
                        (n as u64).saturating_mul(8).saturating_add(8),
                    )?;
                    Self::admit_operation_allocation(
                        allocation,
                        (n as u64).saturating_mul(16).saturating_add(16),
                    )?;
                    let node = self.nodes[t.index()].clone();
                    let (bound, scoped) = node.binding();
                    // Restrict under the binder, then rename it if the restricted
                    // substitution would carry the binder's own name into its scope.
                    Self::spend_operation_work(
                        work,
                        (dom_len + 1)
                            .saturating_mul((bound.len() as u64).saturating_add(1))
                            .saturating_mul((self.nodes.len() as u64).saturating_add(1)),
                    )?;
                    self.admit_substitution_ranges(Some(s), &[], 0, work, allocation)?;
                    let mut inner = self.subst_without(s, &bound);
                    let mut renamed: Option<Vec<Symbol>> = None;
                    if !bound.is_empty() && inner != SubstId::EMPTY {
                        // Freshness is judged against the WHOLE node, not one scoped child:
                        // a `MutualFix` binds every name in every body, so a name that is
                        // safe in body 0 and captured in body 1 would be a silent capture.
                        // The node's own free/binder sets are a superset of every child's,
                        // which is conservative in the only safe direction.
                        let mut new_names = Vec::with_capacity(bound.len());
                        let mut any = false;
                        for &b in &bound {
                            let carries =
                                self.set_contains(self.substs[inner.0 as usize].range_free, b);
                            if carries {
                                let fresh =
                                    self.fresh_like_bounded(b, t, inner, work, allocation)?;
                                if self.nodes.len() >= node_limit {
                                    return Err(StoreOperationLimit::Nodes);
                                }
                                Self::spend_operation_work(work, 64)?;
                                Self::admit_operation_allocation(allocation, 64)?;
                                let v = self.add_bounded(
                                    Node::Var(fresh),
                                    work,
                                    node_limit,
                                    allocation,
                                )?;
                                Self::spend_operation_work(
                                    work,
                                    ((self.substs[inner.0 as usize].dom.len() as u64) + 2)
                                        .saturating_mul(
                                            (self.nodes.len() as u64).saturating_add(1),
                                        ),
                                )?;
                                if self.substs.len() >= node_limit {
                                    return Err(StoreOperationLimit::Nodes);
                                }
                                if self.substs[inner.0 as usize].dom.len() >= node_limit {
                                    return Err(StoreOperationLimit::Nodes);
                                }
                                self.admit_substitution_ranges(
                                    Some(inner),
                                    &[],
                                    1,
                                    work,
                                    allocation,
                                )?;
                                inner = self.subst_with(inner, b, v);
                                new_names.push(fresh);
                                any = true;
                            } else {
                                new_names.push(b);
                            }
                        }
                        if any {
                            renamed = Some(new_names);
                        }
                    }
                    let n = node.child_count();
                    stack.push(Op::Build {
                        sid: s,
                        id: t,
                        n,
                        binders: renamed,
                    });
                    let mut i = 0usize;
                    let mut child_ops = Vec::with_capacity(n);
                    node.for_each_child(&mut |c| {
                        let cs = if scoped.contains(&i) { inner } else { s };
                        child_ops.push(Op::Visit(cs, c));
                        i += 1;
                    });
                    for op in child_ops {
                        stack.push(op);
                    }
                }
                Op::Build {
                    sid: s,
                    id: t,
                    n,
                    binders,
                } => {
                    let base = out.len() - n;
                    Self::admit_operation_allocation(
                        allocation,
                        (n as u64).saturating_mul(8).saturating_add(16),
                    )?;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    Self::spend_operation_work(
                        work,
                        (n as u64).saturating_mul(8).saturating_add(8),
                    )?;
                    let node = self.nodes[t.index()].clone();
                    let rebuilt = match &binders {
                        Some(names) => node.with_binders(names).rebuild_with(&kids),
                        None => node.rebuild_with(&kids),
                    };
                    // Admission precedes compute_info, dedup insertion and all node allocation.
                    if self.nodes.len() >= node_limit {
                        return Err(StoreOperationLimit::Nodes);
                    }
                    let r = self.add_bounded(rebuilt, work, node_limit, allocation)?;
                    self.subst_cache.insert((s, t), r);
                    out.push(r);
                }
            }
        }
        Ok(out.pop().expect("substitution leaves exactly the root"))
    }

    // -- α-canonical identity -------------------------------------------------------------

    /// The α-CANONICAL form of a term: every binder name replaced by one reserved name and
    /// every bound occurrence by its de Bruijn LEVEL, so two terms are α-equivalent exactly
    /// when their canonical forms are the SAME id.
    ///
    /// This is what makes the store useful to a NAMED kernel. `same(a, b)` is structural
    /// equality, and the kernel's conversion checker does not want structural equality — it
    /// wants α. `type_checker::is_subtype` opens with an `alpha_eq(a, b)` fast path that walks
    /// both trees on EVERY conversion, which is the hottest path in the kernel; against a
    /// canonical id it is one integer compare plus a memo hit.
    ///
    /// Free variables keep their names, so `λx. y` and `λx. z` stay distinct — α renames BOUND
    /// names only.
    ///
    /// Levels rather than indices: a level-canonical subterm depends on how deep it sits, so
    /// `λx. x` canonicalises differently under one binder than under two, and the memo holds
    /// one entry per (environment, level, node). De Bruijn INDICES would make a closed subterm's
    /// canonical form position-independent and therefore shared everywhere it occurs — strictly
    /// more sharing, at the price of an abstraction pass that shifts. Recorded rather than done.
    pub fn alpha_id(&mut self, root: TermId) -> TermId {
        if let Some(&c) = self.alpha_cache.get(&root) {
            return c;
        }
        let c = self.canonicalise(SubstId::EMPTY, 0, root);
        self.alpha_cache.insert(root, c);
        c
    }

    /// Whether two terms are α-equivalent — ONE integer compare, after each side's canonical
    /// form has been computed once.
    pub fn alpha_same(&mut self, a: TermId, b: TermId) -> bool {
        if a == b {
            return true;
        }
        // Alpha-renaming preserves shape and global references. These interned
        // annotations prove inequality without canonicalizing either DAG.
        // Saturated sizes are only a negative filter: equality proves nothing.
        let (a_info, b_info) = (self.info[a.index()], self.info[b.index()]);
        if a_info.depth != b_info.depth
            || a_info.tree_size != b_info.tree_size
            || a_info.consts != b_info.consts
        {
            return false;
        }
        self.alpha_id(a) == self.alpha_id(b)
    }

    /// Whether a term is already its own α-canonical form — that is, whether canonicalising
    /// changes nothing. True for anything that binds nothing, and also for a term whose binders
    /// already carry the reserved name and bind no occurrence.
    pub fn is_alpha_canonical(&mut self, id: TermId) -> bool {
        self.alpha_id(id) == id
    }

    fn canonicalise(&mut self, env: SubstId, level: u32, root: TermId) -> TermId {
        enum Op {
            Visit(SubstId, u32, TermId),
            Build {
                key: (SubstId, u32, TermId),
                n: usize,
                binders: usize,
            },
        }
        let canon_binder = self.symbol(CANON_BINDER);
        let mut stack = vec![Op::Visit(env, level, root)];
        let mut out: Vec<TermId> = Vec::new();
        let mut kids: Vec<TermId> = Vec::new();
        while let Some(op) = stack.pop() {
            match op {
                Op::Visit(e, l, t) => {
                    let info = self.info[t.index()];
                    // Already canonical: nothing below binds, and nothing below refers to a
                    // binder above. O(1), and it is the common case for a leaf or an atom.
                    let binds_nothing = self.set(info.binders).is_empty();
                    let untouched = e == SubstId::EMPTY
                        || self.set_mask(info.free) & self.substs[e.0 as usize].dom_mask == 0;
                    if binds_nothing && untouched {
                        out.push(t);
                        continue;
                    }
                    // Outer renaming cannot affect a subtree whose free names
                    // miss its domain. Inner binders install their own mappings.
                    // Preserve l: a closed subtree's new Bound levels still
                    // depend on its position, even with an empty environment.
                    let e = if untouched { SubstId::EMPTY } else { e };
                    if let Some(&c) = self.canon_cache.get(&(e, l, t)) {
                        out.push(c);
                        continue;
                    }
                    if let Node::Var(v) = self.nodes[t.index()] {
                        let sub = &self.substs[e.0 as usize];
                        let r = match sub.dom.iter().position(|d| *d == v) {
                            Some(pos) => sub.to[pos],
                            None => t,
                        };
                        self.canon_cache.insert((e, l, t), r);
                        out.push(r);
                        continue;
                    }
                    let node = self.nodes[t.index()].clone();
                    let (bound, scoped) = node.binding();
                    // Each bound name takes the NEXT level, in `binding()` order — so a mutual
                    // block's names occupy consecutive levels and a shadowing name overwrites
                    // the outer entry, which is exactly what makes the innermost binder win.
                    let mut inner = e;
                    for (i, &b) in bound.iter().enumerate() {
                        let marker = self.add(Node::Bound(l + i as u32));
                        inner = self.subst_with(inner, b, marker);
                    }
                    let inner_level = l + bound.len() as u32;
                    let n = node.child_count();
                    stack.push(Op::Build {
                        key: (e, l, t),
                        n,
                        binders: bound.len(),
                    });
                    let mut i = 0usize;
                    let mut child_ops = Vec::with_capacity(n);
                    node.for_each_child(&mut |c| {
                        if scoped.contains(&i) {
                            child_ops.push(Op::Visit(inner, inner_level, c));
                        } else {
                            child_ops.push(Op::Visit(e, l, c));
                        }
                        i += 1;
                    });
                    for op in child_ops {
                        stack.push(op);
                    }
                }
                Op::Build { key, n, binders } => {
                    let base = out.len() - n;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    let node = self.nodes[key.2.index()].clone();
                    let renamed = if binders == 0 {
                        node.rebuild_with(&kids)
                    } else {
                        let names = vec![canon_binder; binders];
                        node.with_binders(&names).rebuild_with(&kids)
                    };
                    let r = self.add(renamed);
                    self.canon_cache.insert(key, r);
                    out.push(r);
                }
            }
        }
        out.pop().expect("canonicalisation leaves exactly the root")
    }

    // -- the measurement instrument -------------------------------------------------------

    /// ζ-expand a `Let` at the head, whatever its occurrence count — what a conversion check
    /// must be able to do on demand once a `Let` is allowed to survive into a normal form.
    pub fn zeta_expand_head(&mut self, id: TermId) -> Option<TermId> {
        if let Node::Let {
            name, value, body, ..
        } = self.nodes[id.index()]
        {
            Some(self.substitute(body, name, value))
        } else {
            None
        }
    }

    /// β/ζ normal form, with ζ OCCURRENCE-GATED: a `let` whose name occurs at most once in
    /// the normalised body is expanded, and one that occurs more KEEPS the `let` as its
    /// sharing node.
    ///
    /// The gate is re-evaluated AFTER the body is normalised, because reduction can erase
    /// occurrences — a body with three uses of `n` before reduction and one after must still
    /// expand.
    ///
    /// # The rule is `zeta_or_share_opt`'s, all three cases of it
    ///
    /// The occurrence count is only two of the three. The third is the SPECULATION: nothing
    /// inside a held binding stepping is not the same as the binding having no redex, because
    /// ζ is always available and firing it can unblock a rule that could not see past the
    /// binder. `let x := λz. y in x x` is stuck as long as the binding is held and reduces to
    /// `y` the moment it is expanded. So a held binding whose parts are all normal is EXPANDED
    /// SPECULATIVELY and kept only when its expansion is itself a normal form — which is the
    /// corpus's `quad b b b b`, whose head is a declaration with no body.
    ///
    /// The speculation is not re-entered while one is running, exactly as
    /// `zeta_or_share_opt`'s `Speculating` guard does it: a nest of `k` shared bindings would
    /// otherwise be `2^k` probes rather than a walk.
    ///
    /// # Why the rebuild is re-examined
    ///
    /// Normalising a node's children can put a redex AT the node: `(let b := λu. u in e) arg`
    /// holds its binding while `e` mentions `b` twice, and if normalising `e` drops that to one
    /// the binding expands to a λ — and the application above it, already past its own weak-head
    /// pass, is a β-redex nobody would look at again. So a rebuild whose head can move is sent
    /// back through the loop rather than accepted.
    ///
    /// NOT A KERNEL TIER: β and ζ only. No δ, no ι, no η, no `Fix`, no universe algebra.
    pub fn beta_zeta_normalize(&mut self, root: TermId, budget: u64) -> StoreNormalForm {
        enum Op {
            Norm(TermId),
            Build {
                orig: TermId,
                head: TermId,
                n: usize,
            },
            Seal(TermId),
            /// The held-binding speculation's verdict point: `expanded`'s normal form is on
            /// top of the output stack, and it decides whether `held` stands.
            Decide {
                orig: TermId,
                head: TermId,
                held: TermId,
                expanded: TermId,
            },
        }
        let mut fuel = budget;
        // `zeta_or_share_opt`'s `Speculating` guard, as a local rather than a thread-local:
        // this loop is the only thing that can re-enter it.
        let mut probing = false;
        let mut stack = vec![Op::Norm(root)];
        let mut out: Vec<TermId> = Vec::new();
        let mut kids: Vec<TermId> = Vec::new();
        while let Some(op) = stack.pop() {
            match op {
                Op::Norm(id) => {
                    // Keyed on the speculation flag too: a normal form computed while the
                    // speculation was suppressing nested probes is NOT the normal form the
                    // unsuppressed rule gives, and handing one back for the other is how a
                    // memo quietly changes an answer.
                    if let Some(&v) = self.nf_cache.get(&(id, probing)) {
                        out.push(v);
                        continue;
                    }
                    let head = match self.whnf(id, &mut fuel) {
                        Some(h) => h,
                        None => return StoreNormalForm::OutOfFuel { budget },
                    };
                    if let Some(&v) = self.nf_cache.get(&(head, probing)) {
                        self.nf_cache.insert((id, probing), v);
                        out.push(v);
                        continue;
                    }
                    let node = self.nodes[head.index()].clone();
                    stack.push(Op::Build {
                        orig: id,
                        head,
                        n: node.child_count(),
                    });
                    let mut child_ops = Vec::with_capacity(node.child_count());
                    node.for_each_child(&mut |c| child_ops.push(Op::Norm(c)));
                    for op in child_ops {
                        stack.push(op);
                    }
                }
                Op::Build { orig, head, n } => {
                    let base = out.len() - n;
                    kids.clear();
                    kids.extend_from_slice(&out[base..]);
                    kids.reverse();
                    out.truncate(base);
                    let node = self.nodes[head.index()].clone();
                    let rebuilt = self.add(node.rebuild_with(&kids));
                    // A child moved and left a redex at this node — see the doc above.
                    if rebuilt != head && self.head_has_redex(rebuilt) {
                        stack.push(Op::Seal(orig));
                        stack.push(Op::Seal(head));
                        stack.push(Op::Norm(rebuilt));
                        continue;
                    }
                    // The one shape whose gate can flip once its children are normal.
                    if let Node::Let {
                        name, value, body, ..
                    } = self.nodes[rebuilt.index()]
                    {
                        if self.occurrences(body, name) != Occurrences::Many {
                            if fuel == 0 {
                                return StoreNormalForm::OutOfFuel { budget };
                            }
                            fuel -= 1;
                            let expanded = self.substitute(body, name, value);
                            stack.push(Op::Seal(orig));
                            stack.push(Op::Seal(head));
                            stack.push(Op::Norm(expanded));
                            continue;
                        }
                        if !probing {
                            if fuel == 0 {
                                return StoreNormalForm::OutOfFuel { budget };
                            }
                            fuel -= 1;
                            let expanded = self.substitute(body, name, value);
                            probing = true;
                            stack.push(Op::Decide {
                                orig,
                                head,
                                held: rebuilt,
                                expanded,
                            });
                            stack.push(Op::Norm(expanded));
                            continue;
                        }
                    }
                    self.nf_cache.insert((orig, probing), rebuilt);
                    self.nf_cache.insert((head, probing), rebuilt);
                    out.push(rebuilt);
                }
                Op::Decide {
                    orig,
                    head,
                    held,
                    expanded,
                } => {
                    probing = false;
                    let probe = out.pop().expect("a verdict follows a normalised expansion");
                    if probe == expanded {
                        // The expansion is already a normal form, so holding the binding loses
                        // nothing and saves every copy of the value.
                        self.nf_cache.insert((orig, false), held);
                        self.nf_cache.insert((head, false), held);
                        self.nf_cache.insert((held, false), held);
                        out.push(held);
                    } else {
                        // ζ fires. The expansion is normalised AGAIN, outside the speculation,
                        // because the probe ran with nested probes suppressed and its answer is
                        // therefore only an oracle — never the result.
                        stack.push(Op::Seal(orig));
                        stack.push(Op::Seal(head));
                        stack.push(Op::Seal(held));
                        stack.push(Op::Norm(expanded));
                    }
                }
                Op::Seal(id) => {
                    let v = *out.last().expect("a seal follows a normalised result");
                    self.nf_cache.insert((id, probing), v);
                }
            }
        }
        StoreNormalForm::Reached(out.pop().expect("normalisation leaves exactly the root"))
    }

    /// Whether a weak-head pass could still move this term — the head of its application spine
    /// is a λ with an argument, or a `let` that is applied or whose gate now says expand.
    ///
    /// Used to decide whether a REBUILT node has to go back through the loop. It is exact, not
    /// conservative, which is what keeps the re-examination from being an extra pass over every
    /// node of every term.
    fn head_has_redex(&mut self, id: TermId) -> bool {
        let mut h = id;
        let mut applied = false;
        while let Node::App(f, _) = self.nodes[h.index()] {
            h = f;
            applied = true;
        }
        match self.nodes[h.index()] {
            Node::Lambda { .. } => applied,
            Node::Let { name, body, .. } => {
                applied || self.occurrences(body, name) != Occurrences::Many
            }
            _ => false,
        }
    }

    /// Weak head normal form under β and occurrence-gated ζ. `None` means the budget ran out.
    fn whnf(&mut self, id: TermId, fuel: &mut u64) -> Option<TermId> {
        let mut cur = id;
        let mut args: Vec<TermId> = Vec::new();
        loop {
            args.clear();
            let mut head = cur;
            while let Node::App(f, a) = self.nodes[head.index()] {
                args.push(a);
                head = f;
            }
            args.reverse();
            match self.nodes[head.index()] {
                Node::Lambda { param, body, .. } if !args.is_empty() => {
                    if *fuel == 0 {
                        return None;
                    }
                    *fuel -= 1;
                    let substituted = self.substitute(body, param, args[0]);
                    cur = args[1..]
                        .iter()
                        .fold(substituted, |acc, &a| self.mk_app(acc, a));
                }
                Node::Let {
                    name, value, body, ..
                } => {
                    if self.occurrences(body, name) == Occurrences::Many {
                        return Some(cur);
                    }
                    if *fuel == 0 {
                        return None;
                    }
                    *fuel -= 1;
                    let expanded = self.substitute(body, name, value);
                    cur = args.iter().fold(expanded, |acc, &a| self.mk_app(acc, a));
                }
                _ => return Some(cur),
            }
        }
    }
}

/// Visit a `Term`'s children in the canonical order — the `Term`-side twin of
/// [`Node::for_each_child`], and wildcard-free for the same reason.
fn term_for_each_child<'a>(t: &'a Term, f: &mut impl FnMut(&'a Term)) {
    match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => {}
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            f(param_type);
            f(body_type);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            f(param_type);
            f(body);
        }
        Term::App(fun, arg) => {
            f(fun);
            f(arg);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            f(discriminant);
            f(motive);
            for c in cases.iter() {
                f(c);
            }
        }
        Term::Fix { body, .. } => f(body),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs.iter() {
                f(b);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            f(ty);
            f(value);
            f(body);
        }
        Term::Annotated { term, .. } => f(term),
    }
}

// ================================================================================================
// THE `Term`-SIDE PRIMITIVES — what the TREE kernel needs BEFORE any of this is wired in
// ================================================================================================
//
// Everything above prices the store. The three functions below are the parts of it the kernel
// can use TODAY, against the `Box`-recursive `Term` it already has, with no representation
// change and no call-site rewrite. Each replaces a NATIVELY RECURSIVE walk that has a depth
// ceiling of its own, and each is the primitive one specific corpus case is blocked on.

/// Which of `t`'s children a binder on `t` hides `name` inside.
///
/// The `Term`-side twin of [`Node::binding`], written from the same specification — a binder
/// scopes over its BODY and not over its annotation or its bound value — and wildcard-free for
/// the same reason: a new `Term` variant has to answer here rather than be silently unbound.
enum Shadow {
    /// Nothing on this node binds `name`.
    None,
    /// The child at this index is inside a binder that rebinds `name`.
    Child(usize),
    /// Every child is — a mutual block binds all of its names over all of its bodies.
    Every,
}

fn term_shadow(t: &Term, name: &str) -> Shadow {
    match t {
        Term::Pi { param, .. } | Term::Lambda { param, .. } => {
            if param == name {
                Shadow::Child(1)
            } else {
                Shadow::None
            }
        }
        Term::Let { name: bound, .. } => {
            if bound == name {
                Shadow::Child(2)
            } else {
                Shadow::None
            }
        }
        Term::Fix { name: bound, .. } => {
            if bound == name {
                Shadow::Child(0)
            } else {
                Shadow::None
            }
        }
        Term::MutualFix { defs, .. } => {
            if defs.iter().any(|(n, _)| n == name) {
                Shadow::Every
            } else {
                Shadow::None
            }
        }
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole
        | Term::App(_, _)
        | Term::Match { .. }
        | Term::Annotated { .. } => Shadow::None,
    }
}

#[cfg(test)]
thread_local! {
    static FREE_NAME_NODE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_free_name_node_visits() -> usize {
    FREE_NAME_NODE_VISITS.with(|n| n.replace(0))
}

/// Collect exact free local names without recursion or materializing another term.
/// Domains and let values are outside their binder's scope; mutual definitions
/// share all binders. Counts preserve nested shadowing and repeated mutual names.
pub(crate) fn free_names_in_term(term: &Term) -> std::collections::HashSet<String> {
    let mut free = std::collections::HashSet::<&str>::new();
    visit_free_names_and_holes(term, |name| {
        if let Some(name) = name {
            free.insert(name);
        }
        true
    });
    free.into_iter().map(str::to_owned).collect()
}

/// Whole-term closedness, including annotations and unselected branches.
/// The first free local or hole refuses admission; no graph or name set is built.
pub(crate) fn is_closed_without_holes(term: &Term) -> bool {
    visit_free_names_and_holes(term, |_| false)
}

// Some(name) is a free local; None is a hole. Returning false stops the walk.
fn visit_free_names_and_holes<'a>(
    term: &'a Term,
    mut visit: impl FnMut(Option<&'a str>) -> bool,
) -> bool {
    enum Work<'a> {
        Visit(&'a Term),
        Bind(&'a str),
        Unbind(&'a str),
    }
    use Work::*;
    let mut work = vec![Visit(term)];
    let mut bound = std::collections::HashMap::<&str, usize>::new();
    while let Some(item) = work.pop() {
        let t = match item {
            Bind(name) => {
                *bound.entry(name).or_default() += 1;
                continue;
            }
            Unbind(name) => {
                let count = bound.get_mut(name).expect("balanced lexical scope");
                *count -= 1;
                if *count == 0 {
                    bound.remove(name);
                }
                continue;
            }
            Visit(t) => t,
        };
        #[cfg(test)]
        FREE_NAME_NODE_VISITS.with(|n| n.set(n.get() + 1));
        match t {
            Term::Var(name) => {
                if !bound.contains_key(name.as_str()) && !visit(Some(name)) {
                    return false;
                }
            }
            Term::Hole => {
                if !visit(None) {
                    return false;
                }
            }
            Term::Lambda {
                param,
                param_type,
                body,
            }
            | Term::Pi {
                param,
                param_type,
                body_type: body,
            } => {
                work.extend([Unbind(param), Visit(body), Bind(param), Visit(param_type)]);
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                work.extend([
                    Unbind(name),
                    Visit(body),
                    Bind(name),
                    Visit(value),
                    Visit(ty),
                ]);
            }
            Term::Fix { name, body } => {
                work.extend([Unbind(name), Visit(body), Bind(name)]);
            }
            Term::MutualFix { defs, .. } => {
                for (name, _) in defs {
                    work.push(Unbind(name));
                }
                for (_, body) in defs {
                    work.push(Visit(body));
                }
                for (name, _) in defs {
                    work.push(Bind(name));
                }
            }
            Term::App(_, _) | Term::Match { .. } | Term::Annotated { .. } => {
                term_for_each_child(t, &mut |child| work.push(Visit(child)));
            }
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) => {}
        }
    }
    true
}

/// Free occurrences of `name` in `body`, stopping once `stop_at` have been seen.
fn count_occurrences(body: &Term, name: &str, stop_at: u32) -> u32 {
    let mut seen = 0u32;
    let mut work: Vec<&Term> = vec![body];
    while let Some(t) = work.pop() {
        #[cfg(test)]
        FREE_NAME_NODE_VISITS.with(|n| n.set(n.get() + 1));
        if let Term::Var(v) = t {
            if v == name {
                seen = seen.saturating_add(1);
                if seen >= stop_at {
                    return seen;
                }
            }
            continue;
        }
        let hidden = match term_shadow(t, name) {
            Shadow::None => None,
            Shadow::Child(i) => Some(i),
            Shadow::Every => continue,
        };
        let mut i = 0usize;
        term_for_each_child(t, &mut |c| {
            if Some(i) != hidden {
                work.push(c);
            }
            i += 1;
        });
    }
    seen
}

/// How many FREE occurrences of `name` a `Term` has, saturating at `u32::MAX`.
///
/// ITERATIVE, so it answers about a term too deep for the native stack.
/// `type_checker::body_stats` computes the same number by native recursion and therefore has a
/// ceiling of its own; the unit tests at the bottom of this file pin the two against each other
/// on every shape, so they can never drift.
pub fn occurrence_count_in_term(body: &Term, name: &str) -> u32 {
    #[cfg(test)]
    OCCURRENCE_HEAP_WALKS.with(|n| n.set(n.get() + 1));
    count_occurrences(body, name, u32::MAX)
}

/// [`occurrence_count_in_term`], classified — everything an occurrence-gated ζ rule needs, and
/// strictly CHEAPER than the count, because it stops at the second occurrence.
///
/// This is the gate `reduction.rs`'s `Term::Let` arm and `type_checker.rs`'s `Term::Let` rule
/// are missing. Both ζ-substitute UNCONDITIONALLY today, so a value bound once and used n times
/// is written n times and checked n times — which is exactly what
/// `lean_conformance_t00_terms::a_shared_subterm_is_checked_once_and_not_once_per_occurrence` measures.
pub fn occurrences_in_term(body: &Term, name: &str) -> Occurrences {
    match count_occurrences(body, name, 2) {
        0 => Occurrences::Zero,
        1 => Occurrences::Once,
        _ => Occurrences::Many,
    }
}

/// Whether `name` occurs free in `body` at all — stops at the first occurrence.
pub fn occurs_free_in_term(body: &Term, name: &str) -> bool {
    count_occurrences(body, name, 1) > 0
}

thread_local! {
    /// Whether this thread is inside a SPECULATIVE ζ expansion — the probe
    /// [`zeta_or_share_opt`] runs to decide whether holding a shared binding back would stop
    /// computation.
    ///
    /// Without it the probe is EXPONENTIAL in the nesting depth of stuck shared `Let`s: each
    /// arm steps its body (one descent) and then steps its expansion (a second descent through
    /// the same nest), so `T(k) = 2·T(k−1)`. A term nesting thirty of them would hang the
    /// reducer inside a single step, which is a denial of service on adversarial input rather
    /// than a slow answer.
    ///
    /// Inside the probe a nested shared binding therefore answers with the verdict the rule
    /// itself gives it: a shared `Let` whose parts are all normal IS the normal form, so it is
    /// NOT a redex. That makes "normal form" an inductive definition — a term is a SHARED normal
    /// form when nothing in it steps and every surviving binding's one-step ζ expansion is
    /// itself one — and the probe becomes one walk per level, `T(k) = T(k−1) + O(k·size)`.
    ///
    /// The price, stated exactly: a redex that appears only when TWO nested bindings are
    /// expanded at once is not found, and `normalize` stops at the shared form instead of the
    /// fully-expanded one. That is the same bargain the whole rule makes — a surviving `Let` is
    /// a `Let` conversion must ζ-expand on demand — and it is the INCOMPLETE direction, never
    /// the wrong one. The alternative is a probe that is exponential in nesting depth, which is
    /// a hang inside a single reduction step on input a caller chooses.
    static ZETA_SPECULATING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Restores the speculation flag on the way out, including on a panic.
struct Speculating(bool);

impl Drop for Speculating {
    fn drop(&mut self) {
        ZETA_SPECULATING.with(|f| f.set(self.0));
    }
}

impl Speculating {
    fn enter() -> Self {
        Speculating(ZETA_SPECULATING.with(|f| f.replace(true)))
    }

    fn already() -> bool {
        ZETA_SPECULATING.with(|f| f.get())
    }
}

/// ONE ζ step, OCCURRENCE-GATED, against a reducer that always returns a `Term`.
///
/// **[`zeta_or_share_opt`] is the one to wire in.** `reduction.rs` no longer has a
/// `Term`-returning `reduce_step_at`; its reducer is `reduce_step_opt -> Option<Term>`, and the
/// progress-reporting form is both simpler and stricter. This form stays because a step function
/// that cannot say "no redex" is still a legitimate shape — the in-file fixpoint driver uses it
/// — and because the two must be provably the same rule, which
/// `the_two_zeta_gates_are_the_same_rule` pins on every shape.
///
/// The rule, in three cases:
///
/// * The name occurs at most ONCE in the body → ζ fires exactly as it does today,
///   `body[name := value]`, on the UNREDUCED value. Byte-identical to the current arm, which is
///   what keeps every `let_zeta.rs` case and the T00 ζ-transparency cases green: all of them are
///   single-occurrence.
/// * The name occurs MORE → the binding is KEPT as the sharing node and `step` is run on the
///   annotation, the value and the body instead. The `Let` survives into the normal form, so a
///   value bound once and used n times is written ONCE rather than n times. That is
///   `a_shared_subterm_is_checked_once_and_not_once_per_occurrence`: the corpus's term is 329
///   nodes under the current arm and 92 under this one, against a ceiling of 97.
/// * When nothing inside can step any further the gate is RE-EVALUATED on the reduced body,
///   because reduction ERASES occurrences: a body with three uses of `n` before reduction and one
///   after must still expand, or `a_dependent_let_body_types_only_because_zeta_runs_before_
///   inference` goes red.
///
/// # What the caller owes in return
///
/// A `Let` that survives into a normal form is a `Let` that conversion has to see through.
/// `def_eq`/`types_equal`/`is_subtype` must ζ-expand a `Let` HEAD on demand —
/// [`zeta_expand_head_term`] is that step — or two convertible terms, one of which happens to
/// have kept its binding, will compare unequal. That is the negative test to write first.
pub fn zeta_or_share(
    name: &str,
    ty: &Term,
    value: &Term,
    body: &Term,
    step: &dyn Fn(&Term) -> Term,
) -> Term {
    if occurrences_in_term(body, name) != Occurrences::Many {
        return crate::type_checker::substitute(body, name, value);
    }
    let reduced_ty = step(ty);
    let reduced_value = step(value);
    let reduced_body = step(body);
    let unchanged = reduced_ty == *ty && reduced_value == *value && reduced_body == *body;
    if unchanged {
        // See [`zeta_or_share_opt`]: nothing inside stepping is NOT the same as no redex, and
        // the binding may be held back only when its expansion is itself a normal form.
        if !Speculating::already() {
            let expanded = crate::type_checker::substitute(body, name, value);
            let _probe = Speculating::enter();
            if step(&expanded) != expanded {
                return expanded;
            }
        }
    }
    Term::Let {
        name: name.to_string(),
        ty: Box::new(reduced_ty),
        value: Box::new(reduced_value),
        body: Box::new(reduced_body),
    }
}

/// [`zeta_or_share`] against a reducer that reports PROGRESS: `step` answers `None` when there
/// is no redex. `reduction.rs`'s `reduce_step_opt` calls it for every `Term::Let`.
///
/// ```
/// use logicaffeine_kernel::term_store::zeta_or_share_opt;
/// use logicaffeine_kernel::Term;
///
/// let v = |n: &str| Term::Var(n.into());
/// let g = |n: &str| Term::Global(n.into());
/// let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
/// let normal = |_: &Term| None;
///
/// // Used once: ζ fires on the unreduced value.
/// let once = app(g("Succ"), v("x"));
/// assert_eq!(
///     zeta_or_share_opt("x", &g("Nat"), &g("Zero"), &once, &normal),
///     Some(app(g("Succ"), g("Zero"))),
/// );
///
/// // Used twice, nothing inside steps and neither does the expansion: the shared `Let` is the
/// // normal form.
/// let twice = app(app(g("quad"), v("x")), v("x"));
/// assert_eq!(zeta_or_share_opt("x", &g("Nat"), &g("Zero"), &twice, &normal), None);
/// ```
///
/// The rule, in four cases, and note that NONE of them needs a structural comparison:
///
/// * The name occurs at most ONCE in the body → ζ fires exactly as it did before,
///   `Some(body[name := value])` on the UNREDUCED value. Byte-identical to the old arm, which is
///   what keeps every `let_zeta.rs` case and the T00 ζ-transparency cases green: all of them are
///   single-occurrence.
/// * The name occurs MORE and something inside still steps → the binding is KEPT as the sharing
///   node and the step is reported on whichever of the annotation, value and body moved. A value
///   bound once and used n times is written ONCE instead of n times, which is
///   `a_shared_subterm_is_checked_once_and_not_once_per_occurrence`: the corpus's term is 329
///   nodes under the old arm and **92** under this one, against a ceiling of 97.
/// * The name occurs MORE, nothing inside steps, and the one-step ζ EXPANSION still has a redex
///   → `Some(expansion)`. **"Nothing inside steps" is NOT "no redex".** ζ is always available,
///   and firing it can unblock a rule that could not see past the binding:
///   `let twice := 4 in plus twice twice` is stuck only because `plus`'s structural argument is
///   a VARIABLE, and expanding it lets ι unroll to `8`. Getting this wrong took
///   `lean_conformance_t02_declarations::a_let_rec_is_a_fix_bound_by_a_let_and_a_plain_let_cannot_
///   carry_it` — recorded `Native` — from green to a stuck `Let`, along with six other tests in
///   five different files; `W_perf_extra::the_zeta_gate_expands_a_shared_binding_whose_
///   expansion_can_still_compute` is the lock.
/// * The name occurs MORE, nothing inside steps, and the expansion has no redex either → `None`.
///   The shared `Let` IS the normal form — the corpus's `quad b b b b`, whose head is a
///   declaration with no body and which cannot step however `b` is spelled — said exactly rather
///   than recovered from `reduced == current`, so the fixpoint in `normalize_checked_at`
///   terminates on the nose and never pays a Θ(size) compare to find out.
///
/// The probe is run at most ONE level deep (see `ZETA_SPECULATING`): inside it, a nested shared
/// binding answers with the verdict the rule gives it — a shared `Let` whose parts are all
/// normal is not a redex — because an unguarded probe is `T(k) = 2·T(k−1)` in the nesting depth
/// of stuck shared bindings, which is a hang inside a single reduction step on input a caller
/// chooses. `W_perf_extra::a_nest_of_shared_bindings_does_not_make_the_zeta_probe_exponential`
/// is the lock, at sixteen levels.
///
/// The gate is RE-READ for free: reduction erases occurrences, and a body that stepped comes back
/// through this same arm on the next pass, where a name now used once expands. That is what keeps
/// `a_dependent_let_body_types_only_because_zeta_runs_before_inference` green — its body has
/// three uses of `n` before reduction and one after.
///
/// # What the caller owes in return
///
/// A `Let` that survives into a normal form is a `Let` that conversion has to see through.
/// `def_eq`/`types_equal`/`is_subtype` must ζ-expand a `Let` HEAD on demand —
/// [`zeta_expand_head_term`] is that step — or two convertible terms, one of which happens to
/// have kept its binding, will compare unequal. `def_eq(let b := v in body, body[b := v])` is the
/// RED test to write BEFORE touching the arm.
pub fn zeta_or_share_opt(
    name: &str,
    ty: &Term,
    value: &Term,
    body: &Term,
    step: &dyn Fn(&Term) -> Option<Term>,
) -> Option<Term> {
    if occurrences_in_term(body, name) != Occurrences::Many {
        return Some(crate::type_checker::substitute(body, name, value));
    }
    let reduced_ty = step(ty);
    let reduced_value = step(value);
    let reduced_body = step(body);
    if reduced_ty.is_none() && reduced_value.is_none() && reduced_body.is_none() {
        // NOTHING INSIDE STEPS IS NOT THE SAME AS NO REDEX. ζ is always available, and firing it
        // can unblock a rule that could not see past the binding: `let twice := 4 in plus twice
        // twice` is stuck only because `plus`'s structural argument is a VARIABLE, and expanding
        // it lets ι unroll. So the binding is kept only when its EXPANSION is itself a normal
        // form — which is the corpus's `quad b b b b`, whose head is a declaration with no body
        // and which cannot step however `b` is spelled.
        if Speculating::already() {
            // Inside an outer probe: a shared binding whose parts are all normal IS the normal
            // form, and answering anything else collapses the whole nest. See ZETA_SPECULATING.
            return None;
        }
        let expanded = crate::type_checker::substitute(body, name, value);
        let _probe = Speculating::enter();
        return step(&expanded).map(|_| expanded);
    }
    Some(Term::Let {
        name: name.to_string(),
        ty: Box::new(reduced_ty.unwrap_or_else(|| ty.clone())),
        value: Box::new(reduced_value.unwrap_or_else(|| value.clone())),
        body: Box::new(reduced_body.unwrap_or_else(|| body.clone())),
    })
}

/// [`zeta_or_share_opt`] on a `Let` the caller owns, stepping its parts where they lie: the same
/// rule, in the same order, with the same answer, but a part that a step leaves alone is never
/// copied and nothing is rebuilt around the one that moved. `step_in_place` must be the in-place
/// twin of `step` (`reduce_step_in_place` of `reduce_step_opt`), which
/// `reduction::owned_normalize_controls` pins shape by shape. `false` for anything not `Let`.
pub fn zeta_or_share_in_place(
    term: &mut Term,
    step_in_place: &dyn Fn(&mut Term) -> bool,
    step: &dyn Fn(&Term) -> Option<Term>,
) -> bool {
    let Term::Let {
        name,
        ty,
        value,
        body,
    } = term
    else {
        return false;
    };
    if occurrences_in_term(body, name) != Occurrences::Many {
        let Term::Let {
            name, value, body, ..
        } = std::mem::replace(term, Term::Hole)
        else {
            unreachable!()
        };
        *term = crate::type_checker::substitute_owned(*body, &name, &value);
        return true;
    }
    let ty_stepped = step_in_place(ty);
    let value_stepped = step_in_place(value);
    let body_stepped = step_in_place(body);
    if ty_stepped || value_stepped || body_stepped {
        return true;
    }
    if Speculating::already() {
        return false;
    }
    let expanded = crate::type_checker::substitute(body, name, value);
    let _probe = Speculating::enter();
    if step(&expanded).is_some() {
        *term = expanded;
        return true;
    }
    false
}

/// ζ-expand a `Let` at the head, whatever its occurrence count — the step a conversion check
/// must be able to take on demand once [`zeta_or_share`] lets a `Let` survive into a normal
/// form. `None` for anything that is not `Let`-headed.
///
/// The `Term`-side twin of [`TermStore::zeta_expand_head`].
pub fn zeta_expand_head_term(t: &Term) -> Option<Term> {
    match t {
        Term::Let {
            name, value, body, ..
        } => Some(crate::type_checker::substitute(body, name, value)),
        _ => None,
    }
}

/// Whether syntax is within the kernel's existing term-depth limit.
///
/// This is a pure admission predicate, not a typing judgment or a kernel/TLS
/// entry. It does not establish that recursive work already has a sized stack.
pub fn within_kernel_depth(term: &Term) -> bool {
    !crate::type_checker::depth_exceeds(term, crate::type_checker::MAX_TERM_DEPTH)
}

/// The nesting depth of a `Term`, counted in EDGES: a leaf is 0.
///
/// This is the DOOR's measure — `type_checker::depth_exceeds(t, n)` is exactly
/// `term_depth(t) > n` — and it is one less than [`TermStore::depth`], which counts a leaf as
/// one rung. The unit tests at the bottom of this file pin both relations, so the three
/// measures cannot drift apart silently.
pub fn term_depth(t: &Term) -> usize {
    let mut best = 0usize;
    let mut work: Vec<(&Term, usize)> = vec![(t, 0)];
    while let Some((node, d)) = work.pop() {
        if d > best {
            best = d;
        }
        let deeper = d + 1;
        term_for_each_child(node, &mut |c| work.push((c, deeper)));
    }
    best
}

/// **THE PURE β/ζ DOOR.** The `Term`-level entry point a tree reducer wires in so a β-heavy
/// reduction stops being quadratic — and the only one it needs, because it answers `None` for
/// every term it is not certain about.
///
/// `Some(nf)` is a promise that `nf` is EXACTLY what a full kernel reduction would have
/// produced: the same term, not merely an α-equivalent one. `None` means "not mine" and the
/// caller falls through to the path it already has, unchanged.
///
/// # Why this is the fix and a faster loop is not
///
/// `lean_conformance_t00_terms::a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically` asks
/// for at most 5× when a spine's depth TRIPLES. A tree reducer pays Θ(size) per step — the β
/// step copies the remaining spine, and the fixpoint comparison walks it — over Θ(size) steps,
/// so it is Θ(size²) and tripling costs about 9×. A ratio is immune to constant factors, so no
/// amount of tuning reaches the bar; the representation has to change. On this store every
/// step is `u32` work against hash-consed nodes, and the same reduction is linear.
/// `W_perf_extra::w_perf_measure_the_spine_in_the_tree_and_in_the_store` prints both columns at
/// the corpus's own two depths, and
/// `W_perf_extra::the_pure_door_triples_the_corpus_spine_in_under_five` asserts the bar on this
/// path.
///
/// # What it refuses, and why each refusal is exact
///
/// The store implements β and occurrence-gated ζ. It implements nothing else, so it must never
/// be handed a term another rule could touch. Six shapes are refused in O(1) from
/// [`TermStore::shapes`]:
///
/// * `Match` — ι lives there.
/// * `Fix` and `MutualFix` — unrolling asks `termination::structural_columns` which column the
///   recursion descends on; the store cannot call it.
/// * `Const` — δ with a level substitution, plus the well-formedness test that turns a
///   mis-levelled reference into `Term::Hole` rather than into a normal form.
/// * `Lit` — the arithmetic primitives fire only when the ARGUMENT is a literal, so a term with
///   no literal anywhere cannot reach one.
/// * `Hole` — not a term of the calculus; the type checker refuses it by construction.
///
/// And every `Global` name in the term must satisfy `inert`. That predicate is the CALLER's
/// because only the caller has the `Context`: a name is inert when δ cannot unfold it
/// (`ctx.get_definition_body(name).is_none()`) **and** it is not one of the names a rule is
/// keyed on rather than a shape — the reflection builtins `syn_*`, the native-evaluation hook
/// `reduceBool`, and the quotient eliminators `Quot_lift`/`Quot_ind`. The caller must also
/// refuse the whole door when its context has any registered `@[simp]` rule, since a rewrite
/// can fire on any shape at all.
///
/// # The two remaining honesty conditions
///
/// * **Budget.** `budget` counts β and ζ steps. Exhausting it is `None`, never a term passed
///   off as normal — the same distinction `normalize_checked` draws between a normal form and a
///   spent budget.
/// * **No invented names, on EITHER side.** Capture-avoiding substitution renames binders, and
///   an answer that renamed is α-equivalent to the kernel's rather than equal to it. Both
///   implementations rename on the same criterion — the binder's name is a free variable of the
///   replacement — so where that criterion never fires they are the SAME FUNCTION. Where it
///   does fire they can still differ, because `type_checker::substitute_avoiding` descends the
///   WHOLE body and renames such a binder even in a subtree the substituted variable never
///   reaches, while this store descends only where the substitution is live. So the door
///   refuses on [`TermStore::rename_risk`] — set the moment any substitution pushes a value
///   into a body that binds one of the value's free names — and, as a second and independent
///   check, on [`TermStore::binders_within`].
pub fn beta_zeta_normal_form(
    term: &Term,
    inert: &dyn Fn(&str) -> bool,
    budget: u64,
) -> Option<Term> {
    let mut store = TermStore::new();
    let root = store.intern(term);
    if !store.is_beta_zeta_pure(root, inert) {
        return None;
    }
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(root, budget) else {
        return None;
    };
    if store.rename_risk() || !store.binders_within(nf, root) {
        return None;
    }
    store.extract_bounded(nf, EXTRACT_CEILING).ok()
}

/// [`beta_zeta_normal_form`] with every policy question answered from the `Context` — the form a
/// reducer wires in, with nothing left for the caller to get wrong.
///
/// # The inertness criterion, and why it is POSITIVE
///
/// A `Global` is admitted only when it is a CONSTRUCTOR or an INDUCTIVE and carries no
/// definition body. That is a closed, positive criterion, and it is chosen over the obvious
/// negative one — "not a definition and not one of these builtin names" — because the negative
/// one is a list that a future rule can silently fall off the end of.
///
/// It is sufficient, rule by rule, against `reduction::reduce_step_opt`:
///
/// * δ cannot fire: a constructor and an inductive have no body, and the belt-and-braces
///   `get_definition_body(name).is_none()` says so a second time.
/// * The arithmetic primitives cannot fire: `try_primitive_reduce` returns `None` unless the
///   ARGUMENT is a `Term::Lit`, and a `Lit` anywhere is refused by shape.
/// * ι cannot fire: it is the `Match` rule, refused by shape.
/// * Fixpoint unrolling cannot fire: it needs a `Fix` head, refused by shape.
/// * Every NAME-keyed rule — the reflection builtins `syn_*`, the tactic builtins `try_*` and
///   `tact_*`, `induction_*`, `concludes`, the native-evaluation hook `reduceBool`, and the
///   quotient eliminators `Quot_lift`/`Quot_ind` — is registered with `add_declaration` and is
///   therefore neither a constructor nor an inductive. `W_perf_extra::
///   the_conservative_inertness_criterion_refuses_every_name_keyed_rule_head` executes that
///   claim over the whole list rather than asserting it in prose.
/// * A registered `@[simp]` rewrite can fire on ANY shape, so the door is shut outright
///   whenever the context has one.
///
/// # Transparency does not enter
///
/// `Transparency::Excluding` only ever makes δ LESS available, and the criterion already admits
/// no term δ could touch. So the answer is the same at every transparency, which is why this
/// takes no transparency argument.
pub fn beta_zeta_normal_form_in_context(
    ctx: &crate::Context,
    term: &Term,
    budget: u64,
) -> Option<Term> {
    if !ctx.simp_rules().is_empty() {
        return None;
    }
    let inert = |name: &str| {
        (ctx.is_constructor(name) || ctx.is_inductive(name))
            && ctx.get_definition_body(name).is_none()
    };
    let mut store = TermStore::new();
    let root = store.intern(term);
    if !store.is_beta_zeta_pure(root, &inert) {
        return None;
    }
    let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(root, budget) else {
        return None;
    };
    // A compact DAG can grow a normal form deeper than the input. The tree
    // normalizer's callers still clone/drop recursively, so keep its depth door.
    if store.depth(nf) as usize > crate::type_checker::MAX_TERM_DEPTH + 1
        || store.rename_risk()
        || !store.binders_within(nf, root)
    {
        return None;
    }
    store.extract_bounded(nf, EXTRACT_CEILING).ok()
}

/// Positive beta/zeta conversion over a shared graph, leaving every global,
/// constant, match, and recursive head opaque. These need not be full normal
/// forms: a common reduct proves equality even when other rules also apply.
/// No result or cache escapes this call, and no expanded tree is materialized.
pub(crate) fn beta_zeta_equivalent(ctx: &crate::Context, a: &Term, b: &Term, budget: u64) -> bool {
    if budget == 0 || !ctx.simp_rules().is_empty() {
        return false;
    }
    let mut store = TermStore::new();
    let a = store.intern(a);
    let b = store.intern(b);
    if [a, b]
        .into_iter()
        .any(|id| store.shapes(id).contains(NodeShape::Hole) || !store.eq_faithful(id))
    {
        return false;
    }
    let StoreNormalForm::Reached(a) = store.beta_zeta_normalize(a, budget) else {
        return false;
    };
    let StoreNormalForm::Reached(b) = store.beta_zeta_normalize(b, budget) else {
        return false;
    };
    store.alpha_same(a, b)
}

/// A positive conversion shortcut for the beta/zeta/delta fragment.
///
/// Results stay in the shared store: a deep normal form is never extracted into a
/// recursively owned tree. Only equality proves anything; unsupported shapes,
/// cycles, budget exhaustion, and unequal results all defer to ordinary conversion.
/// The store and its caches live for this call and this immutable context only.
pub(crate) fn beta_delta_equivalent(ctx: &crate::Context, a: &Term, b: &Term, budget: u64) -> bool {
    if budget == 0 || !ctx.simp_rules().is_empty() {
        return false;
    }
    fn expand(
        store: &mut TermStore,
        ctx: &crate::Context,
        root: TermId,
        fuel: &mut u64,
        done: &mut HashMap<TermId, TermId>,
    ) -> Option<TermId> {
        enum Op {
            Visit(TermId),
            Build(TermId, usize),
            Seal(TermId),
        }
        let mut work = vec![Op::Visit(root)];
        let mut out = Vec::new();
        let mut active = std::collections::HashSet::new();
        while let Some(op) = work.pop() {
            match op {
                Op::Visit(id) => {
                    if let Some(&result) = done.get(&id) {
                        out.push(result);
                        continue;
                    }
                    if *fuel == 0 || !active.insert(id) {
                        return None;
                    }
                    *fuel -= 1;
                    #[cfg(test)]
                    DELTA_EXPANSION_WORK.with(|counter| {
                        if let Some((visits, matches)) = counter.get() {
                            counter.set(Some((
                                visits + 1,
                                matches
                                    + usize::from(matches!(
                                        &store.nodes[id.index()],
                                        Node::Match { .. }
                                    )),
                            )));
                        }
                    });
                    let node = store.nodes[id.index()].clone();
                    match &node {
                        Node::Global(sym) => {
                            let name = store.names.resolve(*sym).to_string();
                            // A bare global does not instantiate a universe telescope.
                            if ctx
                                .poly_declaration(&name)
                                .is_some_and(|(params, _)| !params.is_empty())
                            {
                                return None;
                            }
                            if let Some(body) = ctx.get_definition_body(&name) {
                                let body = store.intern(body);
                                // Delta unfolding beneath a binder requires a closed body.
                                if !store.free_vars(body).is_empty() {
                                    return None;
                                }
                                let shapes = store.shapes(body);
                                if shapes.contains(NodeShape::Hole) || !store.float_free(body) {
                                    return None;
                                }
                                let supported = ShapeMask::BETA_ZETA
                                    .with(NodeShape::Const)
                                    .with(NodeShape::Lit);
                                if shapes.subset_of(supported) {
                                    work.push(Op::Seal(id));
                                    work.push(Op::Visit(body));
                                    continue;
                                }
                                // Outside this fragment, keep the original named
                                // definition. Equality may need no delta step here;
                                // ordinary conversion handles required evaluation.
                            }
                            // No supported visible body: retain the exact opaque name. A
                            // shared reduct proves conversion without requiring
                            // this leaf to be a constructor or a full normal form.
                        }
                        // Explicit constants remain exact opaque leaves. Do not
                        // instantiate levels or look up a same-named Global body;
                        // independent typing still validates reference arity.
                        Node::Const { .. }
                        | Node::Sort(_)
                        | Node::Var(_)
                        | Node::Pi { .. }
                        | Node::Lambda { .. }
                        | Node::App(..)
                        | Node::Let { .. } => {}
                        // Retain exact literal tags and payloads without evaluating
                        // them. Keep floats outside expansion even when a definition
                        // introduces one after the original input admission.
                        Node::Lit(lit)
                            if !matches!(store.lits[lit.index()], LitKey::FloatBits(_)) => {}
                        _ => return None,
                    }
                    work.push(Op::Build(id, node.child_count()));
                    node.for_each_child(&mut |child| work.push(Op::Visit(child)));
                }
                Op::Build(id, count) => {
                    let mut children = out.split_off(out.len() - count);
                    children.reverse();
                    let node = store.nodes[id.index()].rebuild_with(&children);
                    let result = store.add(node);
                    done.insert(id, result);
                    active.remove(&id);
                    out.push(result);
                }
                Op::Seal(id) => {
                    let result = *out.last()?;
                    done.insert(id, result);
                    active.remove(&id);
                }
            }
        }
        out.pop()
    }
    let mut store = TermStore::new();
    let (source_a, source_b) = (a, b);
    let a = store.intern(source_a);
    let b = store.intern(source_b);
    if [a, b]
        .into_iter()
        .any(|id| store.shapes(id).contains(NodeShape::Hole) || !store.eq_faithful(id))
    {
        return false;
    }
    // Stop at a shared intermediate term when one closed delta step proves
    // equality. Expanding both remaining tails first makes every generated
    // equation in a long declaration chain pay for all subsequent definitions.
    // Reuse the bounded, capture-safe delta probe; only exact alpha equality is
    // evidence. An unavailable/oversized step leaves the original gateway below.
    for (source, target) in [(source_a, b), (source_b, a)] {
        if let Some((_, unfolded)) = crate::reduction::bounded_delta_head(ctx, source) {
            let unfolded = store.intern(&unfolded);
            if !store.shapes(unfolded).contains(NodeShape::Hole)
                && store.eq_faithful(unfolded)
                && store.alpha_same(unfolded, target)
            {
                return true;
            }
        }
    }
    let mut fuel = budget;
    // Both operands inhabit this one store and immutable context. Reuse only
    // completed expansions: active nodes stay local to each walk, and any
    // failure ends the entire call. Every memo entry required a charged visit,
    // so this call's existing expansion allowance also bounds the memo size.
    // No context-sensitive evidence survives the call or its universe policy.
    let mut expanded = HashMap::new();
    let Some(a) = expand(&mut store, ctx, a, &mut fuel, &mut expanded) else {
        return false;
    };
    let Some(b) = expand(&mut store, ctx, b, &mut fuel, &mut expanded) else {
        return false;
    };
    drop(expanded);
    let StoreNormalForm::Reached(a) = store.beta_zeta_normalize(a, fuel) else {
        return false;
    };
    let StoreNormalForm::Reached(b) = store.beta_zeta_normalize(b, fuel) else {
        return false;
    };
    store.alpha_same(a, b)
}

/// The tree size beyond which [`beta_zeta_normal_form`] refuses to materialise its answer.
///
/// A DAG can denote a tree far larger than the machine holds, and the door's whole promise is
/// that it hands back a `Term` a tree reducer could have built for itself. Sixteen million nodes
/// is four orders of magnitude past anything the corpus builds and still bounded, so the refusal
/// is a `None` rather than an allocator death.
const EXTRACT_CEILING: u64 = 1 << 24;

// ================================================================================================
// THE KERNEL'S OWN STACK
// ================================================================================================
//
// The depth door promises a REFUSAL — "the term is neither accepted nor rejected, because
// descending it would exhaust the native stack" — and delivers a dead process for every term
// between the ceiling it actually has and the one it advertises.
//
// MEASURED on this box, debug build, libtest's 2 MiB thread:
//
// * `infer_type`'s frame is 16,832 bytes. The corpus's sixty-deep dependent telescope dies at
//   125 nested frames: 125 × 16,832 = 2,104,000 B against 2,097,152 B of stack, to the byte.
//   So the real ceiling is a term depth of about 123.
// * `MAX_TERM_DEPTH` is 1600. The door is thirteen times too permissive, and every term in the
//   band 124..=1600 passes it and then ABORTS THE PROCESS — the exact failure it exists to
//   prevent. `W_perf_extra::the_kernels_real_depth_ceiling_is_far_below_the_door_it_advertises`
//   bisects the number in a child process rather than asserting a doc comment.
// * Raising `MAX_TERM_DEPTH` cannot fix it; LOWERING it to ~120 would refuse the 1500-deep
//   β-spine the corpus normalises successfully today. The constant is not the problem: the
//   kernel is walking a term of unbounded depth on a stack somebody else chose.
//
// So the kernel takes its own. [`deepen_stack`] measures the term ONCE at the outermost entry
// — the same linear pass the door already pays — and, when the term is deeper than a stack it
// was handed can be trusted for, runs the whole call on a scoped thread whose stack is SIZED
// FROM THAT DEPTH. Nested entries see a thread-local flag and cost one `Cell` read.
//
// This is better than Lean's answer, not merely different: Lean takes a fixed global thread
// stack (`--tstack`) and throws "deep recursion was detected" above it, so its ceiling is a
// launch-time guess that is simultaneously too small for one proof and too large for every
// other. Sizing per call spends the reservation only on the terms that need it, and leaves the
// door meaning what it says.

/// The deepest term the kernel will walk on a stack it did not size.
///
/// Set from the measurement above with a wide margin: the deepest walk measured, `infer_type`,
/// fits about 123 rungs in 2 MiB, and the kernel nests other recursive walks inside it
/// (`normalize` → `reduce_step` → `substitute_avoiding` → `freshen`), so the inline band is set
/// at a quarter of the shallowest ceiling any single walk has.
pub const INLINE_DEPTH: usize = 32;

/// Stack reserved per rung of nesting past [`INLINE_DEPTH`].
///
/// 48 KiB is roughly three `infer_type` frames per rung of term depth, measured against a
/// telescope that spends about one frame per rung. The reservation is virtual: Linux commits a
/// thread stack page by page, so a deep term reserves address space and touches only what it
/// uses.
pub const BYTES_PER_RUNG: usize = 48 * 1024;

/// The floor under any stack the kernel sizes for itself, whatever the term's depth — room for
/// the non-recursive part of a kernel call, and for a panic to unwind and format its message.
pub const BASE_STACK_BYTES: usize = 2 * 1024 * 1024;

/// How many bytes of stack a term of this depth is given.
pub fn stack_bytes_for_depth(depth: usize) -> usize {
    BASE_STACK_BYTES.saturating_add(depth.saturating_mul(BYTES_PER_RUNG))
}

thread_local! {
    /// How many bytes of stack THIS thread was given by [`deepen_stack`], or 0 when the kernel
    /// did not size it.
    static SIZED_STACK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Restores the sized-stack record on the way out, including on a panic.
struct SizedGuard(usize);

impl Drop for SizedGuard {
    fn drop(&mut self) {
        SIZED_STACK.with(|s| s.set(self.0));
    }
}

impl SizedGuard {
    fn enter(bytes: usize) -> Self {
        SizedGuard(SIZED_STACK.with(|s| s.replace(bytes)))
    }
}

/// How much stack the kernel call on this thread is entitled to use.
///
/// [`BASE_STACK_BYTES`] when nobody sized it — the conservative reading of "whatever the host
/// handed us", and what `recheck.rs`'s stack budget assumes today as a hard-coded constant.
/// A budget derived from this scales with the stack the call actually got, so the second kernel
/// stops answering `Unsupported` on a deep term the first kernel now checks happily.
pub fn kernel_stack_bytes() -> usize {
    match SIZED_STACK.with(|s| s.get()) {
        0 => BASE_STACK_BYTES,
        n => n,
    }
}

/// The rungs ([`BYTES_PER_RUNG`] each) of the stack this thread runs on, by the kernel's own
/// sizing rule: at least [`INLINE_DEPTH`], at most the kernel's term-depth door.
pub fn kernel_stack_rungs() -> usize {
    let sized = kernel_stack_bytes().saturating_sub(BASE_STACK_BYTES) / BYTES_PER_RUNG;
    sized
        .max(INLINE_DEPTH)
        .min(crate::type_checker::MAX_TERM_DEPTH)
}

/// Run `f` on a stack sized for `term` — or report that this call is already on one.
///
/// `Some(r)`: `f` ran, here or on a thread of the kernel's own, and `r` is its answer.
/// `None`: this call is NESTED inside one that already sized the stack, so the caller should
/// carry straight on with its ordinary body.
///
/// It is one line at the top of a kernel entry point (`infer_type` opens with it), with no
/// re-indentation of the body:
///
/// ```
/// use logicaffeine_kernel::term_store::deepen_stack;
/// use logicaffeine_kernel::Term;
///
/// fn size(term: &Term) -> usize {
///     if let Some(answer) = deepen_stack(term, || size(term)) {
///         return answer;
///     }
///     match term {
///         Term::App(f, x) => 1 + size(f) + size(x),
///         _ => 1,
///     }
/// }
///
/// let term = Term::App(Box::new(Term::Global("f".into())), Box::new(Term::Var("x".into())));
/// assert_eq!(size(&term), 3);
/// // A call already running on a sized stack is NESTED and answers `None`.
/// assert_eq!(deepen_stack(&term, || deepen_stack(&term, || ())), Some(None));
/// ```
///
/// The recursive call inside the closure re-enters the entry point, sees the flag, gets `None`,
/// and falls through to the real body — so the measurement is paid exactly ONCE per outermost
/// kernel call, which is the same budget `enter_kernel`'s own `depth_exceeds` pass already has.
///
/// A panic inside `f` is re-raised on the calling thread with its payload intact, so
/// `#[should_panic]` and `catch_unwind` behave exactly as they do without the switch.
///
/// The closure must be `Send`, which every kernel entry's `(&Context, &Term)` capture is.
/// `reduction::normalize_at` is the ONE exception: `Transparency::Excluding` holds a
/// `&dyn Fn(&str) -> bool` with no `Sync` bound, so it is not `Send`. Adding `+ Sync` to that
/// trait object is the whole fix, and `normalize`/`normalize_checked` — which build their own
/// `Transparency::All` — need nothing.
#[must_use = "`None` means this call is NESTED and the caller must fall through to its own body; \
              dropping the answer silently discards the kernel's verdict"]
pub fn deepen_stack<R: Send>(term: &Term, f: impl FnOnce() -> R + Send) -> Option<R> {
    deepen_stack_all(&[term], f)
}

/// [`deepen_stack`] for an entry point that takes more than one term — `is_subtype` and
/// `def_eq` compare two, and the stack has to hold the DEEPER of them.
#[must_use = "`None` means this call is NESTED and the caller must fall through to its own body; \
              dropping the answer silently discards the kernel's verdict"]
pub fn deepen_stack_all<R: Send>(terms: &[&Term], f: impl FnOnce() -> R + Send) -> Option<R> {
    deepen_stack_all_with_depth(terms, 0, f)
}

/// [`deepen_stack_all`] told about depth the TERMS DO NOT SHOW — and it is not a refinement, it
/// is a hole in the door that this is the only way to close.
///
/// **THE DEFECT, measured rather than argued.** The depth door and the sized stack both measure
/// the term they were HANDED. `infer_type`'s `Var` arm does not walk the term — it returns
/// `ctx.get(name).cloned()`, and `Term`'s `Clone` is derived, so it recurses over the BINDING'S
/// type. A term of depth 1 under a context that binds one variable at a type of depth 5000
/// therefore walks 5000 rungs that neither the door nor [`deepen_stack`] ever saw:
/// `W_perf_extra::a_shallow_term_under_a_deep_binding_walks_a_depth_nobody_measured` runs it in a
/// child process. Every other arm inherits it — an `App`'s domain check normalises the function's
/// type, a `Global`'s type is cloned out of the environment — so the input term's depth is a
/// LOWER BOUND on what a kernel call descends, never the bound itself.
///
/// A caller that knows the depth its walk reaches passes it as `extra_depth`: the second kernel's
/// open entry points pass their local plan's deepest binding. `extra_depth` of 0 is exactly
/// [`deepen_stack_all`].
///
/// ```
/// use logicaffeine_kernel::term_store::deepen_stack_all_with_depth;
/// use logicaffeine_kernel::Term;
///
/// let shallow = Term::Var("x".into());
/// // One node, but the walk will descend a binding 5000 levels deep: the stack is sized for that.
/// assert_eq!(deepen_stack_all_with_depth(&[&shallow], 5000, || "answered"), Some("answered"));
/// ```
#[must_use = "`None` means this call is NESTED and the caller must fall through to its own body; \
              dropping the answer silently discards the kernel's verdict"]
pub fn deepen_stack_all_with_depth<R: Send>(
    terms: &[&Term],
    extra_depth: usize,
    f: impl FnOnce() -> R + Send,
) -> Option<R> {
    if SIZED_STACK.with(|s| s.get()) != 0 {
        return None;
    }
    let term_depth_max = terms.iter().map(|t| term_depth(t)).max().unwrap_or(0);
    // PAST THE DOOR, SPEND NOTHING. A term deeper than `MAX_TERM_DEPTH` is REFUSED by
    // `enter_kernel` before the kernel descends a single rung, so sizing a stack for it would
    // reserve tens of megabytes and start a thread to be told "no". Running inline reaches the
    // same refusal — the door is what answers, not the stack — and it caps the reservation this
    // function can ever make at `stack_bytes_for_depth(MAX_TERM_DEPTH)`.
    if term_depth_max > crate::type_checker::MAX_TERM_DEPTH {
        let _sized = SizedGuard::enter(BASE_STACK_BYTES);
        return Some(f());
    }
    let depth = term_depth_max.max(extra_depth);
    if depth <= INLINE_DEPTH {
        let _sized = SizedGuard::enter(BASE_STACK_BYTES);
        return Some(f());
    }
    Some(run_on_stack(stack_bytes_for_depth(depth), f))
}

/// [`deepen_stack`] told about depth the term does not show — see
/// [`deepen_stack_all_with_depth`] for the defect this closes.
#[must_use = "`None` means this call is NESTED and the caller must fall through to its own body; \
              dropping the answer silently discards the kernel's verdict"]
pub fn deepen_stack_with_depth<R: Send>(
    term: &Term,
    extra_depth: usize,
    f: impl FnOnce() -> R + Send,
) -> Option<R> {
    deepen_stack_all_with_depth(&[term], extra_depth, f)
}

/// Continue a reduction whose intermediate term outgrew its entry stack.
/// The caller checks the existing term-depth ceiling before calling. Geometric
/// growth bounds the number of suspended parent threads; the largest reservation
/// remains the one already permitted for MAX_TERM_DEPTH.
pub(crate) fn grow_stack_for_depth<R: Send>(depth: usize, f: impl FnOnce() -> R + Send) -> R {
    assert!(depth <= crate::type_checker::MAX_TERM_DEPTH);
    let required = stack_bytes_for_depth(depth);
    let current = kernel_stack_bytes();
    if required <= current || depth <= INLINE_DEPTH {
        return f();
    }
    let ceiling = stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH);
    let bytes = required.max(current.saturating_mul(2)).min(ceiling);
    run_on_stack(bytes, f)
}

/// Run `f` on a stack sized for `bytes`, propagating its panic.
///
/// Native: a scoped thread, so `f` may borrow the `Context` and the `Term` it was called with —
/// no `'static` bound, no `Arc`, no clone of either. The thread is the kernel's own, part of the
/// calling kernel call: a resource stop recorded on it (`reduction::observe_resource_stops`) is
/// carried back, so a conversion that happened to run on a sized stack cannot pass off an
/// undecided comparison as a decided one.
///
/// wasm32 has no threads — `Builder::spawn_scoped` is `Unsupported` there — so the browser client
/// instead links ONE stack big enough for [`stack_bytes_for_depth`] of
/// `crate::type_checker::MAX_TERM_DEPTH` up front (`apps/logicaffeine_web/build.rs`, locked
/// against these two constants by `wasm_link_stack_matches_kernel_worst_case`), and `f` runs
/// inline on it; the resource stop is observed on the calling thread instead of a spawned one.
fn run_on_stack<R: Send>(bytes: usize, f: impl FnOnce() -> R + Send) -> R {
    #[cfg(target_arch = "wasm32")]
    let (result, stopped) = {
        let _sized = SizedGuard::enter(bytes);
        let result = f();
        (result, crate::reduction::resource_stop_on_this_thread())
    };
    #[cfg(not(target_arch = "wasm32"))]
    let (result, stopped) = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(bytes)
            .spawn_scoped(scope, move || {
                let _sized = SizedGuard::enter(bytes);
                let result = f();
                (result, crate::reduction::resource_stop_on_this_thread())
            })
            .expect("the kernel can start a thread of its own")
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    });
    if let Some(reason) = stopped {
        crate::reduction::record_resource_stop(reason);
    }
    result
}

// ================================================================================================
// THE IDENTITY-KEYED CACHE
// ================================================================================================

/// A cache keyed on node identity, dense: `Vec<Option<V>>` indexed by the id, so a lookup is
/// a bounds check and NOT a hash.
///
/// This is the other half of hash-consing. Sharing makes a repeated subterm one node; a
/// cache keyed on that node makes it checked ONCE.
pub struct IdCache<V> {
    slots: Vec<Option<V>>,
    filled: usize,
}

impl<V> Default for IdCache<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> IdCache<V> {
    /// An empty cache.
    pub fn new() -> Self {
        IdCache {
            slots: Vec::new(),
            filled: 0,
        }
    }

    /// The cached value for `id`, if any.
    pub fn get(&self, id: TermId) -> Option<&V> {
        self.slots.get(id.index()).and_then(|s| s.as_ref())
    }

    /// Whether `id` has a cached value.
    pub fn contains(&self, id: TermId) -> bool {
        self.get(id).is_some()
    }

    /// Cache a value for `id`, replacing any previous one.
    pub fn insert(&mut self, id: TermId, value: V) {
        if self.slots.len() <= id.index() {
            self.slots.resize_with(id.index() + 1, || None);
        }
        if self.slots[id.index()].is_none() {
            self.filled += 1;
        }
        self.slots[id.index()] = Some(value);
    }

    /// How many distinct ids are cached — the count that says "checked once, not once per
    /// occurrence".
    pub fn len(&self) -> usize {
        self.filled
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.filled == 0
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.filled = 0;
    }
}

// ================================================================================================
// THE CONVERSION CACHE — the pair-keyed half of hash-consing
// ================================================================================================

/// A cache of decided conversions, keyed on a PAIR of node identities.
///
/// [`IdCache`] answers "what is this node's type" once per node. This answers "are these two
/// nodes convertible" once per pair, which is the other thing Lean's kernel has and ours does
/// not: `is_subtype` today has exactly one shortcut (`alpha_eq`) and then normalises BOTH sides
/// to a FULL normal form before comparing, with nothing remembered afterwards.
///
/// MEASURED on this box: 100 repetitions of one convertible-but-not-α-equal pair cost 0.662 s
/// against 0.00655 s for one — 6.55, 6.57 and 6.62 ms per call at 1, 10 and 100 repeats. The
/// per-call cost is FLAT, which is the measurement that says there is no cache anywhere.
/// `tests/W_perf_extra.rs::w_perf_measure_what_the_missing_conversion_cache_costs` prints it.
///
/// Conversion is SYMMETRIC, so `(a, b)` and `(b, a)` are one entry: the key is ordered.
///
/// NEGATIVE ENTRIES ARE KEPT, deliberately. They are the expensive half — deciding that two
/// terms are NOT convertible is what costs a full normalisation of both sides — and Lean caches
/// them for the same reason.
///
/// # Two rules this cache cannot enforce for itself, and a soundness trap in the second
///
/// 1. **Identity is not `Term` equality on floats.** Settled: `Literal`'s `PartialEq` compares
///    `f64::to_bits()`, so the store's interning key IS the kernel's equality and the two agree
///    everywhere. [`eq_faithful`](TermStore::eq_faithful) remains the conservative guard, and a
///    caller that writes `store.same(a, b) && store.eq_faithful(a)` is correct under the old
///    rule as well as the new one.
/// 2. **A cache may not outlive the environment it decided under.** Adding a definition can only
///    ADD conversions, so a stale NEGATIVE is merely incomplete. A stale POSITIVE is the kernel
///    certifying a conversion that no longer holds.
///
///    **Wave 2 named the wrong mechanism, and the correction matters.** `Context::pop_scope`
///    does NOT remove definitions: it reverts the four ATTRIBUTE databases — instances,
///    coercions, simp rules, hints — and its own doc says "NEVER a declaration"; a `def` written
///    inside a `section` survives its `end`. The call that genuinely removes one is
///    `Context::displace`, which `add_inductive`/`declare` run on the name they are about to
///    bind, so **re-declaring a name deletes the previous definition of it** — and a positive
///    decided against that body is then stale.
///
///    The reason a per-call cache needs nothing else: every kernel entry takes `&Context`, and a
///    shared reference cannot be displaced, extended in place or popped. **Within one outermost
///    kernel call the global environment is FIXED.** Only the LOCAL bindings vary, by
///    `Context::extend`, which returns a new `Context` and may SHADOW — and locals are what
///    [`EnvId`] keys. So: hold the cache for exactly one outermost call, and let [`KernelCall`]
///    make that a borrow rather than a comment — see [`DefEqCache::scoped`].
pub struct DefEqCache {
    decided: HashMap<(u32, u32), bool>,
    hits: u64,
    misses: u64,
}

impl Default for DefEqCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DefEqCache {
    /// An empty cache.
    pub fn new() -> Self {
        DefEqCache {
            decided: HashMap::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// Conversion is symmetric, so the pair is ordered before it is keyed.
    fn key(a: TermId, b: TermId) -> (u32, u32) {
        if a.0 <= b.0 {
            (a.0, b.0)
        } else {
            (b.0, a.0)
        }
    }

    /// What was decided about this pair, in either order, or `None` if it has not been asked.
    pub fn get(&mut self, a: TermId, b: TermId) -> Option<bool> {
        match self.decided.get(&Self::key(a, b)) {
            Some(&v) => {
                self.hits += 1;
                Some(v)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Record a decision. Recording the same pair twice with DIFFERENT verdicts is a bug in the
    /// caller — one `Context`, one answer — and is caught here rather than propagated.
    pub fn insert(&mut self, a: TermId, b: TermId, equal: bool) {
        if let Some(previous) = self.decided.insert(Self::key(a, b), equal) {
            assert_eq!(
                previous, equal,
                "a conversion cache was told both answers for one pair under one context — the \
                 cache outlived the context it decided under"
            );
        }
    }

    /// How many pairs have been decided.
    pub fn len(&self) -> usize {
        self.decided.len()
    }

    /// Whether nothing has been decided.
    pub fn is_empty(&self) -> bool {
        self.decided.is_empty()
    }

    /// Lookups that were answered from the cache.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Lookups that had to be decided.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Forget every decision — what a `Context` change requires, and what dropping the cache at
    /// the end of a kernel call does for free.
    pub fn clear(&mut self) {
        self.decided.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// The same cache with a LIFETIME it cannot escape — the enforceable form of rule 2 above.
    ///
    /// ```
    /// use logicaffeine_kernel::term_store::{DefEqCache, KernelCall};
    ///
    /// let call = KernelCall::begin();
    /// let conv = DefEqCache::scoped(&call);
    /// assert!(conv.is_empty() && conv.hits() == 0 && conv.misses() == 0);
    /// ```
    ///
    /// Returning it past its call does not compile:
    ///
    /// ```compile_fail,E0515
    /// use logicaffeine_kernel::term_store::{DefEqCache, KernelCall, ScopedDefEqCache};
    ///
    /// fn escape() -> ScopedDefEqCache<'static> {
    ///     let call = KernelCall::begin();
    ///     DefEqCache::scoped(&call)
    /// }
    /// ```
    ///
    /// `ScopedDefEqCache<'call>` borrows the `KernelCall`, so it cannot reach a `static` (that
    /// needs `'static`) and cannot be stored in a `Context` without giving `Context` a lifetime
    /// parameter — a change every call site would see. `KernelCall` is neither `Send` nor `Sync`,
    /// so it cannot be smuggled into a global through a `Mutex` either.
    pub fn scoped(_call: &KernelCall) -> ScopedDefEqCache<'_> {
        ScopedDefEqCache {
            inner: DefEqCache::new(),
            _call: std::marker::PhantomData,
        }
    }
}

/// [`DefEqCache`] branded with the lifetime of ONE outermost kernel call. See
/// [`DefEqCache::scoped`].
pub struct ScopedDefEqCache<'call> {
    inner: DefEqCache,
    _call: std::marker::PhantomData<&'call KernelCall>,
}

impl ScopedDefEqCache<'_> {
    /// What was decided about this pair, in either order.
    pub fn get(&mut self, a: TermId, b: TermId) -> Option<bool> {
        self.inner.get(a, b)
    }

    /// Record a decision; two answers for one pair is caught here.
    pub fn insert(&mut self, a: TermId, b: TermId, equal: bool) {
        self.inner.insert(a, b, equal)
    }

    /// How many pairs have been decided.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether nothing has been decided.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Lookups answered from the cache.
    pub fn hits(&self) -> u64 {
        self.inner.hits()
    }

    /// Lookups that had to be decided.
    pub fn misses(&self) -> u64 {
        self.inner.misses()
    }
}

// ================================================================================================
// THE TYPE CACHE — and the lifetime rule that makes it sound
// ================================================================================================

/// A token that lives exactly as long as ONE outermost kernel call, and that a cache BORROWS.
///
/// The soundness rule a conversion or type cache has to obey is not a style note, so it is not
/// written as one. A cache that outlives the environment it decided under is the kernel
/// certifying a judgement that no longer holds, and the way to make that unwritable is to give
/// the cache a lifetime it cannot escape:
///
/// ```
/// use logicaffeine_kernel::term_store::{KernelCall, TypeCache};
///
/// let call = KernelCall::begin();
/// assert!(call.is_outermost());
/// let types: TypeCache<'_, u32> = TypeCache::new(&call);
/// assert_eq!((types.hits(), types.misses()), (0, 0));
/// ```
///
/// `TypeCache<'call, _>` borrows `&'call KernelCall`, so it cannot be put in a `static` (that
/// needs `'static`), and it cannot be put in a `Context` without giving `Context` a lifetime
/// parameter — a change every call site would see and a reviewer could not miss. `KernelCall`
/// is neither `Send` nor `Sync`, so it cannot be smuggled into a global through a `Mutex`
/// either. The comment becomes a compile error:
///
/// ```compile_fail,E0515
/// use logicaffeine_kernel::term_store::{KernelCall, TypeCache};
///
/// fn outlive() -> TypeCache<'static, u32> {
///     let call = KernelCall::begin();
///     TypeCache::new(&call)
/// }
/// ```
///
/// ```compile_fail,E0277
/// fn send<T: Send>() {}
/// send::<logicaffeine_kernel::term_store::KernelCall>();
/// ```
///
/// # What actually invalidates a cached judgement, corrected
///
/// Wave 2 recorded this as "`Context::pop_scope` REMOVES definitions". **That is wrong, and the
/// correction matters because it is the one that says where the danger really is.** `pop_scope`
/// reverts the four ATTRIBUTE databases — instances, coercions, simp rules, hints — and
/// explicitly never a declaration; a `def` written inside a `section` survives its `end`. The
/// call that genuinely removes one is `Context::displace`, which every `add_inductive`,
/// `add_definition` and `declare` runs on the name it is about to bind, so **re-declaring a name
/// deletes the previous definition of it** — and a conversion decided while that body was in
/// force is then a stale POSITIVE.
///
/// The good news, and the reason a per-call cache needs nothing else: every kernel entry point
/// takes `&Context`. A shared reference cannot be displaced, extended in place, or popped. So
/// **within one outermost kernel call the global environment is FIXED**, and only the LOCAL
/// bindings vary — by `Context::extend`, which returns a NEW `Context` and may SHADOW. Locals
/// are what [`EnvId`] keys; globals need nothing but this lifetime.
pub struct KernelCall {
    /// Neither `Send` nor `Sync`: a raw pointer is the standard way to say so, and it is never
    /// dereferenced.
    _not_send: std::marker::PhantomData<*const ()>,
}

thread_local! {
    /// How many [`KernelCall`] tokens are live on this thread.
    static KERNEL_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Default for KernelCall {
    fn default() -> Self {
        Self::begin()
    }
}

impl KernelCall {
    /// Open a kernel call. The token must be held on the stack for as long as any cache keyed
    /// under it.
    pub fn begin() -> Self {
        KERNEL_CALLS.with(|n| n.set(n.get() + 1));
        KernelCall {
            _not_send: std::marker::PhantomData,
        }
    }

    /// Whether this is the OUTERMOST kernel call on this thread — the only place a cache should
    /// be created, because a cache created deeper is thrown away on every ascent and buys
    /// nothing.
    pub fn is_outermost(&self) -> bool {
        KERNEL_CALLS.with(|n| n.get()) == 1
    }

    /// How many kernel calls are live on this thread.
    pub fn depth() -> usize {
        KERNEL_CALLS.with(|n| n.get())
    }
}

impl Drop for KernelCall {
    fn drop(&mut self) {
        KERNEL_CALLS.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// `inferType`, memoised on `(environment, term)` — the cache Lean has and we do not, with the
/// half Lean does not need because its `Expr` is de Bruijn.
///
/// # Why the environment is in the key, and what happens to a cache without it
///
/// `Term` names its variables. `Var("x")` interns to ONE [`TermId`] whatever scope it occurs in,
/// and `Context::extend` SHADOWS — so under `x : Nat` that id's type is `Nat` and under
/// `x : Bool` it is `Bool`. A cache keyed on the term ALONE hands the second scope the first
/// scope's answer: the kernel certifying a type the term does not have, from an optimisation.
/// `W_perf_extra::a_type_cache_keyed_on_the_term_alone_would_certify_the_wrong_type` builds
/// exactly that and watches it happen, and the same test watches this key refuse to.
///
/// Lean is not exposed to it: its `Expr` uses de Bruijn indices for bound variables and globally
/// unique `FVarId`s for free ones, so two occurrences under different binders are DIFFERENT
/// expressions, and `inferType` may cache on the expression alone. Porting that design across
/// without porting the representation is the mistake this type exists to make unavailable.
///
/// # What it costs, and why it still pays
///
/// **The invariant the key does NOT carry, and why it does not have to.** A term's type also
/// depends on the GLOBAL environment — definitions, inductives, constructors, structures. That is
/// not in the key because it cannot change: every kernel entry point takes `&Context`, and a
/// shared reference can be neither displaced, nor extended in place, nor popped. One cache must
/// therefore be threaded only through calls that share one `&Context`'s globals, which is exactly
/// one outermost kernel call — the lifetime [`KernelCall`] gives it.
///
/// [`TermStore::env_for`] restricts the environment to a term's FREE VARIABLES, which the store
/// already knows in O(1) and hash-conses, so the key costs O(|free vars|) and not O(|context|).
/// A CLOSED subterm — every type, every literal, every global's body — keys on
/// [`EnvId::EMPTY`] under every scope, so it is checked once for the whole call however many
/// scopes it appears under. That is `a_shared_subterm_is_checked_once_and_not_once_per_occurrence`
/// on the CHECKING side, by construction rather than by a special case.
pub struct TypeCache<'call, V> {
    decided: HashMap<(EnvId, TermId), V>,
    hits: u64,
    misses: u64,
    _call: std::marker::PhantomData<&'call KernelCall>,
}

impl<'call, V: Clone + std::fmt::Debug> TypeCache<'call, V> {
    /// An empty cache, borrowing the call it may not outlive.
    pub fn new(_call: &'call KernelCall) -> Self {
        TypeCache {
            decided: HashMap::new(),
            hits: 0,
            misses: 0,
            _call: std::marker::PhantomData,
        }
    }

    /// The type decided for this term under this environment, if it has been asked.
    pub fn get(&mut self, env: EnvId, id: TermId) -> Option<&V> {
        match self.decided.get(&(env, id)) {
            Some(v) => {
                self.hits += 1;
                Some(v)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Record a decision.
    ///
    /// Recording the same key twice with a DIFFERENT answer is a bug in the caller — one
    /// environment, one term, one type — and it is the empirical tripwire for a cache that has
    /// outlived the environment it decided under. It is a `debug_assert` because it compares
    /// through `Debug`, which every kernel answer has and not every one has `PartialEq`;
    /// the test suite runs in debug, which is where a stale cache would be caught.
    pub fn insert(&mut self, env: EnvId, id: TermId, ty: V) {
        if let Some(previous) = self.decided.insert((env, id), ty) {
            let now = self.decided.get(&(env, id)).expect("just inserted");
            debug_assert_eq!(
                format!("{previous:?}"),
                format!("{now:?}"),
                "a type cache was told two answers for one (environment, term) — the cache \
                 outlived the environment it decided under"
            );
        }
    }

    /// How many judgements are cached.
    pub fn len(&self) -> usize {
        self.decided.len()
    }

    /// Whether nothing has been decided.
    pub fn is_empty(&self) -> bool {
        self.decided.is_empty()
    }

    /// Lookups answered from the cache — the number that says a shared subterm was checked once.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Lookups that had to be decided.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Forget every decision.
    pub fn clear(&mut self) {
        self.decided.clear();
        self.hits = 0;
        self.misses = 0;
    }
}

// ================================================================================================
// THE CROSS-LOCKS — the measures this file shares with the kernel's own, pinned against each other
// ================================================================================================
//
// These are unit tests rather than integration tests for one reason: `type_checker::depth_exceeds`
// is `pub(crate)`, so the only place the door's own measure can be compared against this file's is
// inside the crate. Two measures of the same tree that drift apart is precisely how a door starts
// admitting what it cannot walk.

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/conversion_gateway_tests.rs"]
mod conversion_gateway_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/beta_zeta_conversion_tests.rs"]
mod beta_zeta_conversion_tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/reduction_stack_growth_controls.rs"]
mod reduction_stack_growth_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/wasm_link_stack_controls.rs"]
mod wasm_link_stack_controls;

// Counts actual input-node visits, including repeated visits to equal syntax.
// A test-only structural-work oracle; no timing thresholds or release overhead.
#[cfg(test)]
thread_local! {
    static INTERN_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(crate) fn reset_intern_visits() {
    INTERN_VISITS.with(|count| count.set(0));
}
#[cfg(test)]
pub(crate) fn intern_visits() -> usize {
    INTERN_VISITS.with(|count| count.get())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/adjacent_delta_work_tests.rs"]
mod adjacent_delta_work_tests;

// Counts actual name materialization, not calls to the emptiness accessor.
#[cfg(test)]
thread_local! {
    static FREE_NAME_COLLECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FREE_NAME_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(crate) fn reset_free_name_work() {
    FREE_NAME_COLLECTIONS.with(|count| count.set(0));
    FREE_NAME_RESOLUTIONS.with(|count| count.set(0));
}
#[cfg(test)]
pub(crate) fn free_name_work() -> (usize, usize) {
    (
        FREE_NAME_COLLECTIONS.with(|count| count.get()),
        FREE_NAME_RESOLUTIONS.with(|count| count.get()),
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/opaque_leaf_delta_controls.rs"]
mod opaque_leaf_delta_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/literal_leaf_delta_controls.rs"]
mod literal_leaf_delta_controls;

// Counts actual expansion dispatches, not metadata queries or test helper calls.
// The optional test counter neither traverses syntax nor changes fuel.
#[cfg(test)]
thread_local! {
    static DELTA_EXPANSION_WORK: std::cell::Cell<Option<(usize, usize)>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/partial_delta_boundary_controls.rs"]
mod partial_delta_boundary_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/zero_universe_header_controls.rs"]
mod zero_universe_header_controls;

/// Count lexical occurrences with bounded native-stack scratch. The child and
/// shadow rules are the same ones used by the general heap-backed traversal.
pub(crate) fn bounded_occurrence_count_in_term(
    body: &Term,
    name: &str,
    max_depth: usize,
    max_nodes: usize,
) -> Option<u32> {
    fn visit(
        term: &Term,
        name: &str,
        depth: usize,
        max_depth: usize,
        left: &mut usize,
    ) -> Option<u32> {
        if depth >= max_depth || *left == 0 {
            return None;
        }
        *left -= 1;
        if let Term::Var(v) = term {
            return Some(u32::from(v == name));
        }
        let hidden = match term_shadow(term, name) {
            Shadow::None => None,
            Shadow::Child(i) => Some(i),
            Shadow::Every => return Some(0),
        };
        let mut index = 0;
        let mut total = Some(0u32);
        term_for_each_child(term, &mut |child| {
            if Some(index) != hidden {
                total = total
                    .and_then(|n| n.checked_add(visit(child, name, depth + 1, max_depth, left)?));
            }
            index += 1;
        });
        total
    }
    let mut left = max_nodes;
    let count = visit(body, name, 0, max_depth, &mut left)?;
    Some(count)
}
#[cfg(test)]
thread_local! {static OCCURRENCE_HEAP_WALKS:std::cell::Cell<usize>=const {std::cell::Cell::new(0)};}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/bounded_occurrence_controls.rs"]
mod bounded_occurrence_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/alpha_metadata_controls.rs"]
mod alpha_metadata_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/literal_payload_controls.rs"]
mod literal_payload_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/depth_admission_controls.rs"]
mod depth_admission_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/bounded_substitution_controls.rs"]
mod bounded_substitution_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/bounded_add_controls.rs"]
mod bounded_add_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/bounded_name_set_controls.rs"]
mod bounded_name_set_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/raw_name_set_controls.rs"]
mod raw_name_set_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/shared_delta_expansion_controls.rs"]
mod shared_delta_expansion_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/term_store/borrowed_children_metadata_controls.rs"]
mod borrowed_children_metadata_controls;
