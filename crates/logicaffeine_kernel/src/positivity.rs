//! Strict positivity checking for inductive types.
//!
//! An inductive type must appear only in "strictly positive" positions in its constructors.
//! Without this check, we could define paradoxical types like:
//!
//! ```text
//! Inductive Bad := Cons : (Bad -> False) -> Bad
//! ```
//!
//! This would allow encoding Russell's paradox and proving False.
//!
//! Strict Positivity Rules (from CIC):
//!
//! I is strictly positive in T iff:
//! 1. I does not occur in T, OR
//! 2. T = Π(x:A). B where:
//!    - If A = I exactly, it's a "recursive argument" (allowed)
//!    - Otherwise, I must NOT occur in A at all
//!    - AND I must be strictly positive in B
//!
//! Examples:
//! - `I -> I` is valid: first I is a recursive argument, second is result
//! - `(I -> X) -> I` is INVALID: I occurs inside the param type of another arrow
//! - `X -> I -> I` is valid: X has no I, second param is recursive arg

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::term::Term;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// Check strict positivity in a constructor type without unfolding definitions.
///
/// Use [`check_positivity_resolved`] at declaration boundaries, where a context is
/// available. This syntactic variant supports callers whose input has already been
/// expanded and provides an independent comparison for alias-free types.
pub fn check_positivity(inductive: &str, constructor: &str, ty: &Term) -> KernelResult<()> {
    check_strictly_positive(&Delta::blind(), &[inductive], constructor, ty)
}

/// Strict positivity with every REFERENCE resolved — the paradox fence that a definitional
/// alias cannot walk around.
///
/// [`check_positivity`] reads the constructor type as written, so a name is an occurrence of
/// the inductive only when it is SPELLED as the inductive. That is not a property of the type,
/// it is a property of the spelling, and δ makes the two different things:
///
/// ```text
/// Inductive Bad : Type 0                 -- no constructors yet
/// Definition Negate : Prop := Bad -> False
/// Constructor cons : Negate -> Bad       -- `Bad` does not literally occur in `Negate`
/// ```
///
/// `Negate` is `Bad → False` up to δ, so `cons` is the Russell constructor
/// `(Bad → False) → Bad` — and `λx. match x with | cons f => f (cons f)` then inhabits
/// `Bad → False`, which applied to its own `cons` inhabits `False`. No fixpoint and no
/// recursor are needed: a bare `Term::Match` does it, so no later gate can catch what this one
/// lets past. The blind check accepts it because `occurs_in` asks whether the STRING "Bad"
/// appears.
///
/// This entry point answers the same four questions about the same four shapes, but resolves a
/// reference to a transparent definition (or a universe-polymorphic one) before deciding
/// whether the inductive occurs in it and whether an occurrence is a legitimate recursive one.
/// Both directions fail SAFE: a reference whose unfolding cannot be decided counts as an
/// occurrence (so the constructor is refused), and never as a recursive one.
pub fn check_positivity_resolved(
    ctx: &Context,
    inductive: &str,
    constructor: &str,
    ty: &Term,
) -> KernelResult<()> {
    check_strictly_positive(&Delta::over(ctx), &[inductive], constructor, ty)
}

/// [`check_positivity_resolved`] for a MUTUAL block — the same δ resolution, and a member
/// hidden behind an alias is a member of the block for every question asked here.
pub fn check_positivity_mutual_resolved(
    ctx: &Context,
    block: &[&str],
    constructor: &str,
    ty: &Term,
) -> KernelResult<()> {
    check_strictly_positive(&Delta::over(ctx), block, constructor, ty)
}

/// Dependencies whose meanings justified native container positivity. Empty for
/// checks that did not use the native rule, preserving existing mutation behavior.
pub(crate) fn native_positivity_dependencies(
    ctx: &Context,
    block: &[&str],
    constructor: &str,
    ty: &Term,
) -> KernelResult<HashSet<String>> {
    let delta = Delta::over(ctx);
    check_strictly_positive(&delta, block, constructor, ty)?;
    if !delta.used_nested.get() {
        return Ok(HashSet::new());
    }
    let mut dependencies = delta.dependencies.into_inner();
    dependencies.retain(|name| !block.contains(&name.as_str()));
    Ok(dependencies)
}

/// Conservative occurrence query using the positivity checker's memoized
/// definition graph. A false result means normalization cannot expose a
/// recursive occurrence of this block; cycles conservatively answer true.
pub(crate) fn mentions_inductive_block(ctx: &Context, block: &[&str], term: &Term) -> bool {
    occurs_in(&Delta::over(ctx), block, term)
}

/// The δ oracle the fence consults: what a REFERENCE unfolds to, and whether that unfolding
/// mentions the block at all.
///
/// `ctx: None` is the historical blind behaviour — every reference is opaque, so the answers
/// are exactly the ones [`check_positivity`] always gave. With a context, a definition is
/// transparent and a `Const` at explicit levels is transparent in the same way, because both
/// are references and the paradox does not care which spelling carried it.
///
/// `mentions` is memoised per name: definition bodies are closed, so "does `n` mention a block
/// member" does not depend on what `n` is applied to, and the same question is asked once per
/// constructor rather than once per occurrence. `path` is the UNFOLDING STACK, not a global
/// visited set: a name re-entered while it is still being unfolded is a cycle the kernel's
/// acyclic definition graph should make impossible, and the honest answer to a question that
/// does not terminate is the refusing one.
struct Delta<'a> {
    ctx: Option<&'a Context>,
    path: RefCell<Vec<String>>,
    mentions: RefCell<HashMap<String, bool>>,
    nested_path: RefCell<Vec<Term>>,
    dependencies: RefCell<HashSet<String>>,
    used_nested: Cell<bool>,
    // Diagnostic only: retain the innermost container field that failed the
    // existing positivity predicate. Never used to admit or reject a term.
    negative_container: RefCell<Option<String>>,
    // Failure to justify a container is not evidence of negative occurrence.
    inconclusive: Cell<bool>,
    /// δ-unfoldings left in the HEAD position for one constructor. The definition graph is
    /// acyclic, so this cannot be reached by a well-formed context; it is here because a fence
    /// that loops on a malformed one is a fence that hangs the compiler instead of refusing.
    /// Exhaustion answers in the refusing direction on both sides.
    fuel: RefCell<u32>,
}

/// Generous by three orders of magnitude: the deepest alias chain in the standard prelude is
/// two.
const UNFOLD_BUDGET: u32 = 512;

impl<'a> Delta<'a> {
    fn blind() -> Self {
        Delta {
            ctx: None,
            path: RefCell::new(Vec::new()),
            mentions: RefCell::new(HashMap::new()),
            nested_path: RefCell::new(Vec::new()),
            dependencies: RefCell::new(HashSet::new()),
            used_nested: Cell::new(false),
            negative_container: RefCell::new(None),
            inconclusive: Cell::new(false),
            // Local beta/zeta exposure is bounded even when global delta is disabled.
            fuel: RefCell::new(UNFOLD_BUDGET),
        }
    }

    fn over(ctx: &'a Context) -> Self {
        Delta {
            ctx: Some(ctx),
            path: RefCell::new(Vec::new()),
            mentions: RefCell::new(HashMap::new()),
            nested_path: RefCell::new(Vec::new()),
            dependencies: RefCell::new(HashSet::new()),
            used_nested: Cell::new(false),
            negative_container: RefCell::new(None),
            inconclusive: Cell::new(false),
            fuel: RefCell::new(UNFOLD_BUDGET),
        }
    }

    /// Spend one head unfolding, or report that the budget is gone.
    fn spend(&self) -> bool {
        let mut fuel = self.fuel.borrow_mut();
        if *fuel == 0 {
            return false;
        }
        *fuel -= 1;
        true
    }

    /// The body `name` unfolds to, or `None` when it is opaque — an axiom, a constructor, an
    /// inductive, an unknown name, or anything at all when this `Delta` is blind.
    fn body(&self, name: &str) -> Option<Term> {
        let ctx = self.ctx?;
        self.dependencies.borrow_mut().insert(name.to_string());
        if let Some(b) = ctx.get_definition_body(name) {
            return Some(b.clone());
        }
        ctx.get_universe_poly(name).map(|(_, _, body)| body.clone())
    }

    /// Whether `name`'s unfolding mentions a member of `block`. `false` for every opaque name,
    /// which is what makes the common case — a constructor field of some unrelated type —
    /// cost one hash lookup.
    fn mentions(&self, block: &[&str], name: &str) -> bool {
        if self.ctx.is_none() {
            return false;
        }
        if let Some(known) = self.mentions.borrow().get(name) {
            return *known;
        }
        if self.path.borrow().iter().any(|n| n == name) {
            return true;
        }
        let Some(body) = self.body(name) else {
            self.mentions.borrow_mut().insert(name.to_string(), false);
            return false;
        };
        self.path.borrow_mut().push(name.to_string());
        let answer = occurs_in(self, block, &body);
        self.path.borrow_mut().pop();
        self.mentions.borrow_mut().insert(name.to_string(), answer);
        answer
    }
}

/// β-apply a CLOSED definition body to `args`, or `None` when it runs out of λs first.
///
/// Positivity in CIC is a property of the type UP TO CONVERSION, so `GTo GList` where
/// `GTo := λX. Nat → X` has to be judged as the `Nat → GList` it is — a strictly-positive
/// functional field — and `GNegOf GBad` where `GNegOf := λX. X → False` as the `GBad → False`
/// it is. Judging the spelling instead refuses the first and admits the second.
///
/// The substitution is `type_checker::substitute`, which α-renames a binder that would capture
/// a free variable of the argument. There is deliberately no second implementation of it here:
/// a paradox fence that substitutes differently from the kernel is a fence around a different
/// language.
fn beta_apply(body: &Term, args: &[&Term]) -> Option<Term> {
    let mut cur = body.clone();
    for a in args {
        match cur {
            Term::Lambda { param, body, .. } => {
                cur = crate::type_checker::substitute(&body, &param, a);
            }
            _ => return None,
        }
    }
    Some(cur)
}

/// The name a reference carries, whether written monomorphically or at explicit levels.
fn ref_name(t: &Term) -> Option<&str> {
    match t {
        Term::Global(n) | Term::Const { name: n, .. } => Some(n.as_str()),
        _ => None,
    }
}

/// Check strict positivity of a MUTUAL BLOCK of inductives in a constructor type.
///
/// A constructor of one block member may recursively reference ANY member — `Even`'s
/// `even_succ : Π(n). Odd n → Even (Succ n)` places the sibling `Odd` in a
/// strictly-positive recursive position. The block is treated as a single "inductive"
/// for positivity: an occurrence of any member to the RIGHT of every arrow is a
/// (recursive) occurrence and allowed; any member in a DOMAIN is a negative
/// occurrence and rejected — so a cross-block paradox `(Even n → False) → Odd n` is
/// caught exactly as a self-negative one is.
pub fn check_positivity_mutual(block: &[&str], constructor: &str, ty: &Term) -> KernelResult<()> {
    check_strictly_positive(&Delta::blind(), block, constructor, ty)
}

/// Check that the inductive appears only strictly positively.
///
/// At the top level of constructor type, we allow:
/// - I as a direct parameter type (recursive argument)
/// - I in the final result type
/// - But NOT I nested inside function types within parameters
fn check_strictly_positive(
    delta: &Delta<'_>,
    block: &[&str],
    constructor: &str,
    ty: &Term,
) -> KernelResult<()> {
    let negative = |where_: &str| {
        if delta.inconclusive.get() {
            return Err(KernelError::CertificationError(format!(
                "native positivity check is inconclusive for constructor '{constructor}' ({where_})"
            )));
        }
        let mut reason = format!(
            "'{}' occurs in negative position ({where_})",
            block.join("/")
        );
        if let Some(origin) = delta.negative_container.borrow().as_ref() {
            reason.push_str(" through ");
            reason.push_str(origin);
        }
        Err(KernelError::PositivityViolation {
            inductive: block.join("/"),
            constructor: constructor.to_string(),
            reason,
        })
    };
    match ty {
        // Direct occurrence of a block member is always fine (either as recursive argument
        // or result type), whether written monomorphically or at explicit levels.
        Term::Global(name) | Term::Const { name, .. } if block.contains(&name.as_str()) => Ok(()),

        // Pi type: Π(x:A). B
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            // Check the parameter type A.
            // If A is a recursive argument (a block member `I` applied to its
            // parameters, with no block member occurring in the arguments — `I`,
            // `I a`, `List A`, `Odd n`, …), it is a strictly-positive recursive
            // occurrence (allowed). Otherwise no block member may occur in A at all.
            if !is_recursive_arg(delta, block, param_type) && occurs_in(delta, block, param_type) {
                return negative("inside parameter type");
            }

            // Recursively check the body type B
            check_strictly_positive(delta, block, constructor, body_type)
        }

        // Application: check both parts, and — when the head is a REFERENCE that unfolds —
        // the shape it unfolds to. `AliasI a` is `I a` and `Negate a` may be `a → False`;
        // reading only the spelling is how the fence is walked around.
        Term::App(func, arg) => {
            let mut head = ty;
            let mut args: Vec<&Term> = Vec::new();
            while let Term::App(f, a) = head {
                args.push(a.as_ref());
                head = f.as_ref();
            }
            args.reverse();
            if matches!(head, Term::Lambda { .. }) && !args.is_empty() {
                // Reuse the same capture-safe beta exposure as recursive-field
                // classification. The original constructor is typed separately.
                if !delta.spend() {
                    delta.inconclusive.set(true);
                    return negative("inside an exhausted beta telescope");
                }
                let Some(mut reduced) = beta_apply(head, &args[..1]) else {
                    delta.inconclusive.set(true);
                    return negative("inside an unexposed beta telescope");
                };
                for argument in &args[1..] {
                    reduced = Term::App(Box::new(reduced), Box::new((*argument).clone()));
                }
                return check_strictly_positive(delta, block, constructor, &reduced);
            }
            // A head that UNFOLDS is resolved before it is judged; one that does not — an
            // inductive, an axiom, a constructor, or any head at all when this fence is blind —
            // falls through to the structural walk below, exactly as before.
            if let Some(name) = ref_name(head).filter(|n| !block.contains(n)) {
                let could_reach =
                    delta.mentions(block, name) || args.iter().any(|a| occurs_in(delta, block, a));
                if let Some(body) = could_reach.then(|| delta.body(name)).flatten() {
                    let Some(reduced) = delta.spend().then(|| beta_apply(&body, &args)).flatten()
                    else {
                        delta.inconclusive.set(true);
                        return negative("inside a reference this fence cannot resolve");
                    };
                    return check_strictly_positive(delta, block, constructor, &reduced);
                }
            }
            check_strictly_positive(delta, block, constructor, func)?;
            check_strictly_positive(delta, block, constructor, arg)
        }

        // Lambda (unusual in types, but handle it)
        Term::Lambda {
            param_type, body, ..
        } => {
            // Same rule as Pi for param_type
            if !is_recursive_arg(delta, block, param_type) && occurs_in(delta, block, param_type) {
                return negative("inside lambda parameter");
            }
            check_strictly_positive(delta, block, constructor, body)
        }

        // Other terms: no occurrences of the inductive to worry about
        Term::Sort(_) => Ok(()),
        Term::Var(_) => Ok(()),
        // Some other declaration, not a block member — monomorphic or universe-polymorphic.
        // A TRANSPARENT one is a reference, not a leaf: if its unfolding mentions the block,
        // the shape that matters is the unfolding's, and it is checked here rather than
        // believed to be absent.
        Term::Global(_) | Term::Const { .. } => {
            let name = ref_name(ty).expect("a Global/Const carries a name");
            if !delta.mentions(block, name) {
                return Ok(());
            }
            match delta.body(name) {
                Some(body) => check_strictly_positive(delta, block, constructor, &body),
                None => {
                    delta.inconclusive.set(true);
                    negative("inside an unfoldable reference")
                }
            }
        }
        Term::Lit(_) => Ok(()), // Literals cannot contain inductives

        // Match in types (unusual but possible)
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            check_strictly_positive(delta, block, constructor, discriminant)?;
            check_strictly_positive(delta, block, constructor, motive)?;
            for case in cases {
                check_strictly_positive(delta, block, constructor, case)?;
            }
            Ok(())
        }

        // Fix in types (very unusual)
        Term::Fix { body, .. } => check_strictly_positive(delta, block, constructor, body),

        // Mutual fix in types (very unusual): check every definition's body.
        Term::MutualFix { defs, .. } => {
            for (_, body) in defs {
                check_strictly_positive(delta, block, constructor, body)?;
            }
            Ok(())
        }

        // Checked declaration paths type the ORIGINAL Let before accepting
        // positivity. Its value's occurrences acquire polarity only after zeta.
        Term::Let {
            name, value, body, ..
        } => {
            if !delta.spend() {
                delta.inconclusive.set(true);
                return negative("inside an exhausted let telescope");
            }
            let exposed = crate::type_checker::substitute(body, name, value);
            check_strictly_positive(delta, block, constructor, &exposed)
        }

        // Hole: type placeholder, no occurrences to check
        Term::Hole => Ok(()),

        // An annotation hides no occurrence: the term under it is checked in the same position.
        Term::Annotated { term, .. } => check_strictly_positive(delta, block, constructor, term),
    }
}

/// True if `term` is a strictly-positive recursive occurrence of `inductive` —
/// possibly a FUNCTIONAL one. Two shapes:
/// - a TELESCOPE `Π(z:B). rest` where `inductive` does not occur in the domain
///   `B` (so it is not in a negative position) and `rest` is itself a recursive
///   occurrence — e.g. `Acc_intro`'s field `Π(y:A). R y x → Acc A R y`, or a
///   rose tree's `Nat → Tree`; and
/// - the BASE `I e₁ … eₙ`: the inductive applied to arguments that do not mention
///   it (`I`, `I a`, `List A`, an indexed `Acc A R y`).
///
/// This is exactly CIC strict positivity: the inductive may appear only to the
/// RIGHT of every arrow. A negative occurrence (`Bad → …`, `(Bad → X) → …`) puts
/// it in a domain, so `is_recursive_arg` returns `false` and the caller's
/// `occurs_in` check then rejects the constructor.
///
/// A δ-TRANSPARENT reference is followed, in the one direction that cannot widen the fence:
/// the arguments must already be free of the block (so unfolding cannot smuggle a member in
/// through a substitution), the body is closed, and what it unfolds to has to satisfy this
/// same predicate. An alias for a recursive occurrence is one; an alias for `Bad → False` is
/// not, because its unfolding puts the block in a domain.
fn is_recursive_arg(delta: &Delta<'_>, block: &[&str], term: &Term) -> bool {
    match term {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => !occurs_in(delta, block, param_type) && is_recursive_arg(delta, block, body_type),
        _ => {
            let mut head = term;
            let mut args: Vec<&Term> = Vec::new();
            while let Term::App(func, arg) = head {
                args.push(arg.as_ref());
                head = func.as_ref();
            }
            args.reverse();
            if matches!(head, Term::Lambda { .. }) && !args.is_empty() {
                if !delta.spend() {
                    delta.inconclusive.set(true);
                    return false;
                }
                // Checked constructor entry points require original source
                // typing as well as this positive result; no ill-typed erased
                // argument is admitted. Reuse delta aliases' capture-safe beta.
                let Some(mut reduced) = beta_apply(head, &args[..1]) else {
                    delta.inconclusive.set(true);
                    return false;
                };
                for argument in &args[1..] {
                    reduced = Term::App(Box::new(reduced), Box::new((*argument).clone()));
                }
                return is_recursive_arg(delta, block, &reduced);
            }
            if is_block_member(block, head) {
                // `I e₁ … eₙ` — the BASE shape. The block may not occur in the index
                // arguments: `I (I a)` is not a recursive occurrence of `I`, it is `I` applied
                // to one.
                return args.iter().all(|a| !occurs_in(delta, block, a));
            }
            let Some(name) = ref_name(head).filter(|n| !block.contains(n)) else {
                return false;
            };
            // Unfold when the block could be in there AT ALL — either the definition's own body
            // mentions it, or an ARGUMENT does. The second half is the one `GTo GList` needs:
            // `GTo := λX. Nat → X` mentions nothing, and the recursive occurrence arrives
            // through the argument, so gating on the body alone refuses an ordinary rose tree.
            if !delta.mentions(block, name) && !args.iter().any(|a| occurs_in(delta, block, a)) {
                return false;
            }
            let Some(body) = delta.body(name) else {
                return nested_container_arg(delta, block, head, &args);
            };
            match delta.spend().then(|| beta_apply(&body, &args)).flatten() {
                Some(reduced) => is_recursive_arg(delta, block, &reduced),
                None => {
                    delta.inconclusive.set(true);
                    false
                }
            }
        }
    }
}

// A registered container is covariant only if every constructor preserves that
// property after fixing its uniform parameters. This is a constructor check,
// not a whitelist of names or a guess from the container's arity.
//
// An indexed container (`Vec α n`) is judged the same way: its parameters are fixed, its index
// arguments must not mention the block, and each constructor may return the container at any
// indices of its own — the parameters, never the indices, are what nesting instantiates.
fn nested_container_arg(delta: &Delta<'_>, block: &[&str], head: &Term, args: &[&Term]) -> bool {
    let unsupported = || {
        delta.inconclusive.set(true);
        false
    };
    let Some(ctx) = delta.ctx else {
        return unsupported();
    };
    let Some(name) = ref_name(head) else {
        return unsupported();
    };
    delta.dependencies.borrow_mut().insert(name.to_string());
    let parameters = ctx.inductive_num_params(name);
    if !ctx.is_inductive(name)
        || parameters == 0
        || parameters > ctx.inductive_arity(name)
        || args.len() != ctx.inductive_arity(name)
    {
        return unsupported();
    }
    if args[parameters..]
        .iter()
        .any(|index| occurs_in(delta, block, index))
    {
        return unsupported();
    }
    let Ok(mut arity) = crate::infer_type(ctx, head) else {
        return unsupported();
    };
    let mut domains = Vec::with_capacity(parameters);
    for (position, argument) in args.iter().enumerate() {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = arity
        else {
            return unsupported();
        };
        if position < parameters {
            // Only a type parameter can carry the block into the container; a value
            // parameter (`VBox 3 T`'s `3`) must not mention it at all.
            if !matches!(param_type.as_ref(), Term::Sort(_)) && occurs_in(delta, block, argument) {
                return unsupported();
            }
            domains.push(*param_type);
        }
        arity = crate::type_checker::substitute(&body_type, &param, argument);
    }
    if !matches!(arity, Term::Sort(_)) {
        return unsupported();
    }
    delta.used_nested.set(true);
    // The container at these parameters, whatever its indices: the unit nesting instantiates.
    let instance = args[..parameters].iter().fold(head.clone(), |f, a| {
        Term::App(Box::new(f), Box::new((*a).clone()))
    });
    // Revisiting this exact instantiated container closes only a positive
    // constructor-field cycle. A function domain is rejected by is_recursive_arg
    // before control can reach this assumption.
    if delta
        .nested_path
        .borrow()
        .iter()
        .any(|active| *active == instance)
    {
        return true;
    }
    if !delta.spend() {
        return unsupported();
    }
    delta.nested_path.borrow_mut().push(instance.clone());
    let answer = (|| {
        for (constructor, _) in ctx.get_constructors(name) {
            delta
                .dependencies
                .borrow_mut()
                .insert(constructor.to_string());
            let reference = match head {
                Term::Const { levels, .. } => Term::Const {
                    name: constructor.into(),
                    levels: levels.clone(),
                },
                _ => Term::Global(constructor.into()),
            };
            let Ok(mut ty) = crate::infer_type(ctx, &reference) else {
                return unsupported();
            };
            for (argument, domain) in args[..parameters].iter().zip(&domains) {
                let Term::Pi {
                    param,
                    param_type,
                    body_type,
                } = ty
                else {
                    return unsupported();
                };
                if *param_type != *domain {
                    return unsupported();
                }
                ty = crate::type_checker::substitute(&body_type, &param, argument);
            }
            while let Term::Pi {
                param_type,
                body_type,
                ..
            } = ty
            {
                if occurs_in(delta, block, &param_type)
                    && !is_recursive_arg(delta, block, &param_type)
                {
                    if !delta.inconclusive.get() {
                        let mut origin = delta.negative_container.borrow_mut();
                        if origin.is_none() {
                            *origin =
                                Some(format!("container '{name}', constructor '{constructor}'"));
                        }
                    }
                    return false;
                }
                ty = *body_type;
            }
            // The constructor must return this instance at indices of its own, preserving
            // every parameter. Dropping an unused parameter is not an admissible rewrite.
            let mut result_head = &ty;
            let mut result_args = Vec::new();
            while let Term::App(f, a) = result_head {
                result_args.push(a.as_ref());
                result_head = f;
            }
            result_args.reverse();
            if result_head != head
                || result_args.len() != args.len()
                || result_args[..parameters] != args[..parameters]
                || result_args[parameters..]
                    .iter()
                    .any(|index| occurs_in(delta, block, index))
            {
                return unsupported();
            }
        }
        true
    })();
    delta.nested_path.borrow_mut().pop();
    answer
}

/// True if `term` REFERS to a member of the block — the head of a recursive occurrence.
///
/// A reference is a member whether it is written monomorphically (`Term::Global`) or at
/// explicit universe levels (`Term::Const`). Treating `Const` as inert here would make a
/// universe-polymorphic recursive occurrence invisible: `Π(_:Nat). Bad.{u}` would stop
/// being recognised as a strictly-positive recursive argument and be REFUSED, while
/// `occurs_in`'s matching blind spot would let the negative `(Bad.{u} → False) → Bad.{u}`
/// through. The two halves must agree, so both read the name from the same place.
fn is_block_member(block: &[&str], term: &Term) -> bool {
    match term {
        Term::Global(name) | Term::Const { name, .. } => block.contains(&name.as_str()),
        _ => false,
    }
}

/// Check if any block member's name occurs anywhere in the term, THROUGH δ.
///
/// `Term::Const` is a REFERENCE, not an inert leaf. The moment a `Const` can name an
/// inductive — which is exactly what universe-polymorphic inductives require — skipping it
/// here would hide a negative recursive occurrence from the paradox fence: `Bad.{u}` in a
/// constructor domain would read as "the inductive does not occur", and
/// `Cons : (Bad.{u} → False) → Bad.{u}` would be accepted. Universe levels cannot mention
/// an inductive, so the `levels` carry nothing to descend into.
///
/// A TRANSPARENT definition is a reference for the same reason a `Const` is. `Negate` whose
/// body is `Bad → False` mentions `Bad`, and a fence that answers "no" there is a fence with
/// a δ-shaped hole in it — see [`check_positivity_resolved`] for the paradox it admits.
/// [`Delta::blind`] restores the name-only answer for the historical entry points.
fn occurs_in(delta: &Delta<'_>, block: &[&str], term: &Term) -> bool {
    match term {
        Term::Global(name) | Term::Const { name, .. } => {
            block.contains(&name.as_str()) || delta.mentions(block, name)
        }
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) => false,
        Term::Pi {
            param_type,
            body_type,
            ..
        } => occurs_in(delta, block, param_type) || occurs_in(delta, block, body_type),
        Term::Lambda {
            param_type, body, ..
        } => occurs_in(delta, block, param_type) || occurs_in(delta, block, body),
        Term::App(func, arg) => occurs_in(delta, block, func) || occurs_in(delta, block, arg),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            occurs_in(delta, block, discriminant)
                || occurs_in(delta, block, motive)
                || cases.iter().any(|c| occurs_in(delta, block, c))
        }
        Term::Fix { body, .. } => occurs_in(delta, block, body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| occurs_in(delta, block, b)),
        Term::Let {
            ty, value, body, ..
        } => {
            occurs_in(delta, block, ty)
                || occurs_in(delta, block, value)
                || occurs_in(delta, block, body)
        }
        Term::Hole => false, // Holes don't contain inductives
        Term::Annotated { term, .. } => occurs_in(delta, block, term),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/positivity/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/positivity/constructor_exposure_budget_controls.rs"]
mod constructor_exposure_budget_controls;
