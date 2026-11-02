//! Nested-inductive compiler (K3) — the UNTRUSTED front-end for inductives that recur
//! nested inside a container, `RTree := rnode : TList RTree → RTree`.
//!
//! Such a type is not directly a strict-positivity-checkable inductive: `RTree` appears as
//! an ARGUMENT of `TList`, not to the right of an arrow. The Lean/Coq resolution is
//! SPECIALIZATION: replace the container `TList RTree` by a fresh sibling `RTree$TList`
//! defined MUTUALLY with `RTree`, mirroring `TList`'s constructors with the element type
//! fixed to `RTree`:
//!
//! ```text
//! RTree       := rnode : RTree$TList → RTree
//! RTree$TList := RTree$TList_TNil | RTree$TList_TCons : RTree → RTree$TList → RTree$TList
//! ```
//!
//! This block is an ordinary mutual inductive (K3b), so its recursor recurses through the
//! specialized list — the whole point. To relate the specialized form back to the generic
//! `TList RTree`, the compiler emits CONVERSION ISOS `RTree$TList ↔ TList RTree`.
//!
//! Crucially this module is UNTRUSTED: it only produces terms. The caller registers the
//! mutual block through the trusted mutual machinery and TYPE-CHECKS every iso through the
//! trusted kernel — a mis-compiled sibling or iso is rejected, never trusted. Zero new
//! trusted code, exactly as Lean compiles nested inductives to mutual ones.
//!
//! # The specialisable fragment
//!
//! A container `C` is specialisable when its declared arity is a telescope of `n ≥ 1` SORT
//! parameters with no indices (`TList : Type → Type`, `TPair : Type → Type → Type`), and
//! every constructor of `C` has the shape
//!
//! ```text
//! c : Π(p₁:Sort) … Π(pₙ:Sort). ft₁ → … → ftₖ → C p₁ … pₙ
//! ```
//!
//! where each value field `ftᵢ` is one of
//!
//! * a bare parameter `pⱼ` — the slot that carries parameter `j`'s value;
//! * the SPINE `C p₁ … pₙ` — the container recurring on itself; or
//! * a CARRIED field — any type CLOSED over `p₁ … pₙ` that does not mention `C` (`Text` in
//!   `lcons : Π(A). Text → A → Labelled A → Labelled A`, `Nat` in a counted container).
//!
//! Compound fields are rewritten by the same recursive specialization used for explicit
//! nested arguments. Each inner field gets checked conversion definitions, emitted before
//! the outer wrapper uses them. Negative occurrences, dependent fields and nonuniform
//! container recursion are refused before a mutual block is registered.
//!
//! Registered container applications in fields, such as List A inside an array wrapper,
//! are specialized after fixing the constructor parameters.
//!
//! Carrying a closed field is not an unchecked pass. The emitted block goes through
//! [`Context::add_mutual_inductives`], which runs `check_positivity_mutual` over the WHOLE block
//! and `check_constructor_universes` on every member — so a carried field whose universe does not
//! fit the sibling's sort is refused there, transactionally, by the trusted kernel rather than
//! by this untrusted front end. That is strictly stronger than refusing the shape, which only
//! made Lean's labelled containers unreachable.
//!
//! **Lean agrees, and it was asked rather than assumed.** Run on this box against Lean 4.14.0:
//!
//! ```text
//! inductive Labelled (α : Type) | lnil | lcons : String → α → Labelled α → Labelled α
//! inductive LTree | node : Labelled LTree → LTree          -- ACCEPTED
//! inductive Labelled2 (α : Type) | lnil | lcons : α → Nat → Labelled2 α → Labelled2 α
//! inductive LTree2 | node : Labelled2 LTree2 → LTree2      -- ACCEPTED
//! ```
//!
//! and `#check @LTree.rec` prints exactly the specialised mutual block this module emits —
//! `{motive_1 : LTree → Sort u_1} → {motive_2 : Labelled LTree → Sort u_1} → …` with the carried
//! `(a : String)` riding through `lcons`'s minor premise in its own slot. Both FIELD ORDERS are
//! accepted, which matters because `FieldKind` here is assigned by matching a field's TYPE
//! against the parameter telescope and reads no position at all — so `Π(A). Nat → A → C A → C A`
//! and `Π(A). A → Nat → C A → C A` are the same shape to this compiler and to Lean, and no fence
//! can separate them.
//! `G_recursor_extra::a_carried_field_after_the_element_mirrors_into_the_same_slot_it_came_from`
//! takes that by computation: the sibling carries the field at its own index and the isos still
//! invert.
//!
//! Nesting `C a₁ … aₙ` then specialises each `aⱼ` that mentions the inductive (recursively,
//! so `TList (TList RTree)` becomes two siblings) and mirrors `C`'s constructors with
//! parameter `j` fixed to `aⱼ`'s representative. Every parameter position is treated
//! alike — the element may sit in any of them, or in several.
//!
//! What is deliberately OUTSIDE the fragment, refused fail-closed rather than mis-compiled:
//!
//! * a field mentioning a container parameter in a DOMAIN position, where the
//!   occurrence is outright negative, and a field
//!   mentioning the container itself other than as its own spine.
//! * an indexed container, a permuted or partially-applied result type, a `Sort`-valued
//!   value field (which the kernel's ι-reduction would mistake for a parameter), and a
//!   dependent field (one mentioning an earlier field's binder), which the emitted
//!   non-dependent sibling could not express.
//!
//! Explicit universe instances are retained in generic types, constructor references and
//! generated conversion bodies. Sibling identities include their complete universe instance;
//! malformed instances are refused by ordinary kernel inference before specialization.
//!
//! # Parameters and universes of the nested family itself
//!
//! A family may take uniform parameters (`PTree α := node : α → List (PTree α) → PTree α`) and
//! universe parameters (`UTree.{u} (α : Type u) : Type u`). The parameters are split onto one
//! canonical telescope that every emitted sibling shares (`PTree$List α`), every member is
//! universe polymorphic over exactly the family's levels and lives in the family's own sort, and
//! the base leaf of specialization is the family at exactly its own parameters and levels. An
//! occurrence at anything else (`PTree (List α)` inside `PTree α`, `UTree.{0}` inside
//! `UTree.{u}`) is an invalid declaration, refused as Lean refuses it. Recursors, motives and
//! minor premises follow Lean's auxiliary-type discovery order (outermost occurrence first),
//! which is not the post-order siblings are emitted in.

use crate::context::{Context, MutualInductive};
use crate::error::{KernelError, KernelResult};
use crate::term::Term;

/// A nested inductive declaration: a name, its sort, and constructors whose argument types
/// may mention the inductive nested inside a registered container (`TList RTree`).
pub struct NestedDecl {
    pub name: String,
    pub sort: Term,
    pub constructors: Vec<(String, Term)>,
}

/// One container specialized for this inductive, with the names of its conversion isos.
#[derive(Debug, Clone)]
pub struct IsoNames {
    /// The generic container that was specialized (e.g. `TList`).
    pub container: String,
    /// The fresh mutual sibling (e.g. `RTree$TList`).
    pub sibling: String,
    /// `to_generic : sibling → container name` — specialized to generic.
    pub to_generic: String,
    /// `from_generic : container name → sibling` — generic to specialized.
    pub from_generic: String,
}

/// The compiled artifacts of a nested inductive: the mutual block to register, and the
/// iso DEFINITIONS `(name, type, body)` to kernel-check and add. Nothing here is trusted
/// until the caller checks it.
pub struct Compiled {
    pub block: Vec<MutualInductive>,
    pub isos: Vec<(String, Term, Term)>,
    pub iso_names: Vec<IsoNames>,
    pub siblings: Vec<String>,
    /// The canonical uniform parameter telescope shared by the source inductive, every
    /// specialized sibling and every conversion iso. Empty for an unparameterized family.
    pub parameters: Vec<(String, Term)>,
}

/// What a nested-inductive registration produced: the specialized sibling type names and
/// the conversion isos relating each to its generic container.
#[derive(Debug, Clone)]
pub struct NestedInfo {
    pub siblings: Vec<String>,
    pub isos: Vec<IsoNames>,
}

// --- small builders ---
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}

/// A nested declaration with its uniform parameter telescope made explicit.
///
/// Every constructor's leading parameter binders are renamed onto ONE canonical telescope, so the
/// inductive at its own parameters — the base leaf of specialization — is one syntactic term,
/// and every specialized sibling takes the same telescope as its own uniform parameters. With
/// no parameters this is exactly the unparameterized declaration.
struct Nesting<'a> {
    decl: &'a NestedDecl,
    params: Vec<(String, Term)>,
    /// The declaration's universe parameters: every member of the emitted block is universe
    /// polymorphic over exactly these, and is referenced at exactly these.
    levels: Vec<String>,
    /// The sort every member of the block lives in, after the parameters.
    sort: Term,
    /// Each constructor's field telescope and result, over the canonical parameters.
    constructors: Vec<(String, Term)>,
}

impl Nesting<'_> {
    fn name(&self) -> &str {
        &self.decl.name
    }
    fn vars(&self) -> Vec<Term> {
        self.params.iter().map(|(p, _)| v(p)).collect()
    }
    /// A member of the block (or one of its constructors or isos) at the block's own levels.
    fn reference(&self, name: &str) -> Term {
        reference_at(name, &self.levels)
    }
    /// The inductive applied to its own parameters, in order.
    fn uniform(&self) -> Term {
        apps(self.reference(self.name()), self.vars())
    }
    /// A specialized sibling applied to the shared parameters.
    fn applied(&self, sibling: &str) -> Term {
        apps(self.reference(sibling), self.vars())
    }
    fn close_pi(&self, body: Term) -> Term {
        self.params
            .iter()
            .rev()
            .fold(body, |b, (p, t)| pi(p, t.clone(), b))
    }
    fn close_lam(&self, body: Term) -> Term {
        self.params
            .iter()
            .rev()
            .fold(body, |b, (p, t)| lam(p, t.clone(), b))
    }
}

/// `name` at the universe parameters `levels`: a plain global when there are none.
fn reference_at(name: &str, levels: &[String]) -> Term {
    if levels.is_empty() {
        g(name)
    } else {
        Term::Const {
            name: name.to_string(),
            levels: levels
                .iter()
                .cloned()
                .map(crate::term::Universe::Var)
                .collect(),
        }
    }
}

/// Split `num_params` uniform parameters off the declaration's sort and every constructor.
/// A constructor whose parameter domains differ from the sort's is refused: the parameters of
/// a nested inductive are uniform, and a mismatch means the declaration is not the one its
/// sort announces.
fn nesting<'a>(
    decl: &'a NestedDecl,
    num_params: usize,
    levels: &[String],
) -> KernelResult<Nesting<'a>> {
    let refuse = |why: String| {
        KernelError::CertificationError(format!("nested-compile: '{}' {why}", decl.name))
    };
    let canonical: Vec<String> = (0..num_params)
        .map(|i| format!("nested.parameter.{i}"))
        .collect();
    let mut params = Vec::with_capacity(num_params);
    let mut sort = decl.sort.clone();
    for name in &canonical {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = sort
        else {
            return Err(refuse(format!(
                "declares {num_params} parameter(s) but its sort binds fewer"
            )));
        };
        params.push((name.clone(), *param_type));
        sort = crate::substitute(&body_type, &param, &v(name));
    }
    // Every specialized sibling is emitted in the source's own sort, and the trusted block door
    // checks each emitted constructor's universe there. A family that may live in `Prop` has
    // different elimination rules, which specialization does not reconstruct.
    match &sort {
        Term::Sort(level) if crate::term::Universe::Type(0).is_subtype_of(level) => {}
        _ => {
            return Err(refuse(format!(
                "ends in {sort} after its parameters; nested families are reconstructed only in \
             `Type` universes (never `Prop` or a possibly-`Prop` sort)"
            )))
        }
    }
    let mut constructors = Vec::with_capacity(decl.constructors.len());
    for (cname, cty) in &decl.constructors {
        let mut ty = cty.clone();
        for (name, domain) in &params {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = ty
            else {
                return Err(refuse(format!(
                    "constructor '{cname}' binds fewer than its {num_params} parameter(s)"
                )));
            };
            if *param_type != *domain {
                return Err(refuse(format!(
                    "constructor '{cname}' binds parameter {name} at {param_type}, but the \
                     sort binds it at {domain} — nested parameters are uniform"
                )));
            }
            ty = crate::substitute(&body_type, &param, &v(name));
        }
        constructors.push((cname.clone(), ty));
    }
    Ok(Nesting {
        decl,
        params,
        levels: levels.to_vec(),
        sort,
        constructors,
    })
}

/// Simultaneously instantiate the canonical `params` of an open `image` with `args`.
/// Each parameter is first renamed to a name free in every argument, so instantiating one
/// parameter can never capture a variable another argument mentions.
fn instantiate_parameters(image: &Term, params: &[String], args: &[Term]) -> Term {
    let taken = |candidate: &str| {
        args.iter().any(|a| mentions_var(a, candidate)) || mentions_var(image, candidate)
    };
    let mut term = image.clone();
    let mut fresh = Vec::with_capacity(params.len());
    for (i, p) in params.iter().enumerate() {
        let mut name = format!("native.instance.{i}");
        while taken(&name) || params.contains(&name) {
            name.push('\'');
        }
        term = crate::substitute(&term, p, &v(&name));
        fresh.push(name);
    }
    for (name, arg) in fresh.iter().zip(args) {
        term = crate::substitute(&term, name, arg);
    }
    term
}

/// The role of one VALUE field of a container constructor, relative to the container's
/// parameter telescope `p₁ … pₙ`.
enum FieldKind {
    /// The bare parameter `pⱼ` — carries a value of parameter `j`'s type.
    Param(usize),
    /// The SPINE `C p₁ … pₙ` — the container recurring on itself.
    Spine,
    /// A CARRIED field: a type CLOSED over the container's parameter telescope, mentioning
    /// neither the container nor the inductive being declared (`Text` in
    /// `lcons : Π(A). Text → A → Labelled A → Labelled A`). Because it is closed it is the SAME
    /// type in the specialised sibling and in the generic container, so the conversion isos are
    /// the identity on it — and, being closed, it is fully visible to the trusted mutual door,
    /// which re-checks its strict positivity and its universe like any other constructor field.
    Carried(Term),
    /// A compound field with its checked specialized/generic conversion pair.
    Nested(Spec),
    /// A POSITIVE function field `Π(point : domain). result` whose domain mentions neither the
    /// container, its parameters nor the inductive: the container recurring on itself under a
    /// binder, `sup : (Nat → C A) → C A`. It converts pointwise, `λx. c (f x)`.
    Pointwise {
        point: String,
        domain: Term,
        result: Box<FieldKind>,
    },
}

/// How a value moves between a specialised type and the generic type it stands for.
#[derive(Clone)]
enum Conversion {
    /// A checked conversion iso, applied at the shared parameters.
    Iso(String),
    /// A function converts POINTWISE: `λpoint:domain. result (f point)`.
    Pointwise {
        point: String,
        domain: Term,
        result: Box<Conversion>,
    },
}

/// A function type's binders and its final codomain, each binder renamed to its own
/// `nested.point.{i}` — free in the type, and distinct along the telescope, so a pointwise
/// conversion `λp₀. λp₁. c (f p₀ p₁)` never captures one point with another. A later domain may
/// depend on an earlier binder; the renaming carries it along.
fn pointwise_telescope(ty: &Term) -> (Vec<(String, Term)>, Term) {
    let mut used = 0usize;
    let mut fresh = || loop {
        let name = format!("nested.point.{used}");
        used += 1;
        if !mentions_var(ty, &name) {
            return name;
        }
    };
    let mut renamed = Vec::new();
    let mut rest = ty.clone();
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = rest
    {
        let point = fresh();
        // Keep the remaining binders scoped so substitution respects shadowing.
        rest = crate::substitute(&body_type, &param, &v(&point));
        renamed.push((point, *param_type));
    }
    (renamed, rest)
}

/// Peel a term's leading `Π`s into `(name, type)` pairs plus the residual.
fn peel_pis(t: &Term) -> (Vec<(String, Term)>, Term) {
    let mut params = Vec::new();
    let mut cur = t.clone();
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        params.push((param, *param_type));
        cur = *body_type;
    }
    (params, cur)
}

fn is_global(t: &Term, name: &str) -> bool {
    matches!(t, Term::Global(n) if n == name)
}

/// The head and argument spine of an application (`C a b` → `(C, [a, b])`).
fn spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut head = t;
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    (head, args)
}

/// The declared name of either a monomorphic or explicitly instantiated reference.
/// The complete head term is retained separately when generating specialized conversions.
fn referenced_name(t: &Term) -> Option<&str> {
    match t {
        Term::Global(n) => Some(n.as_str()),
        Term::Const { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

/// True if the global `name` occurs anywhere in `ty`. TOTAL over `Term` — a blind spot here
/// silently passes a nesting through unspecialised, so every variant is listed explicitly
/// rather than swept up by a wildcard.
fn occurs_anywhere(ty: &Term, name: &str) -> bool {
    match ty {
        Term::Global(n) => n == name,
        Term::Const { name: n, .. } => n == name,
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => false,
        Term::App(f, x) => occurs_anywhere(f, name) || occurs_anywhere(x, name),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => occurs_anywhere(param_type, name) || occurs_anywhere(body_type, name),
        Term::Lambda {
            param_type, body, ..
        } => occurs_anywhere(param_type, name) || occurs_anywhere(body, name),
        Term::Fix { body, .. } => occurs_anywhere(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| occurs_anywhere(b, name)),
        Term::Let {
            ty, value, body, ..
        } => {
            occurs_anywhere(ty, name) || occurs_anywhere(value, name) || occurs_anywhere(body, name)
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            occurs_anywhere(discriminant, name)
                || occurs_anywhere(motive, name)
                || cases.iter().any(|c| occurs_anywhere(c, name))
        }
        Term::Annotated { term, .. } => occurs_anywhere(term, name),
    }
}

/// True if the local `name` occurs anywhere in `ty`, IGNORING shadowing — a conservative
/// over-approximation, because every caller uses it to refuse fail-closed.
fn mentions_var(ty: &Term, name: &str) -> bool {
    match ty {
        Term::Var(n) => n == name,
        Term::Global(_) | Term::Const { .. } | Term::Sort(_) | Term::Lit(_) | Term::Hole => false,
        Term::App(f, x) => mentions_var(f, name) || mentions_var(x, name),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => mentions_var(param_type, name) || mentions_var(body_type, name),
        Term::Lambda {
            param_type, body, ..
        } => mentions_var(param_type, name) || mentions_var(body, name),
        Term::Fix { body, .. } => mentions_var(body, name),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| mentions_var(b, name)),
        Term::Let {
            ty, value, body, ..
        } => mentions_var(ty, name) || mentions_var(value, name) || mentions_var(body, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            mentions_var(discriminant, name)
                || mentions_var(motive, name)
                || cases.iter().any(|c| mentions_var(c, name))
        }
        Term::Annotated { term, .. } => mentions_var(term, name),
    }
}

/// True if `ty` is a constructor argument that must be SPECIALIZED: its head is a registered
/// container inductive (≠ the inductive being defined) and `name` occurs somewhere inside it
/// (`TList name`, `TPair Nat name`, `TList (TList name)`). A bare `name`, a `Nat`, a positive
/// `Nat → name`, or a container not mentioning `name` (`TList Nat`) is NOT nested — it passes
/// through unchanged.
fn is_nested_container_type(ctx: &Context, nest: &Nesting, ty: &Term) -> bool {
    let (head, _) = spine(ty);
    referenced_name(head).is_some_and(|c| {
        c != nest.name() && (ctx.is_inductive(c) || matches!(head, Term::Const { .. }))
    }) && occurs_anywhere(ty, nest.name())
}

/// The result of specializing one nested type or one container ARGUMENT: the SPECIALIZED
/// type standing in for it, a name fragment identifying that type (used to mangle sibling
/// names), the GENERIC type it stands for, and — when the two differ through a container —
/// the names of the conversion isos relating them.
struct Spec {
    repr: Term,
    tag: String,
    generic: Term,
    to: Option<Conversion>,
    from: Option<Conversion>,
}

/// One specialized sibling to emit: a mutual inductive mirroring `container` with each of
/// its parameters fixed, plus the data its conversion isos need (each parameter's generic
/// form and its own isos, which the sibling's isos delegate to).
struct SibNode {
    container: String,
    container_head: Term,
    repr: String,
    /// Per container parameter: the SPECIALIZED type it is fixed to.
    arg_repr: Vec<Term>,
    /// Per container parameter: the GENERIC type that stands for.
    arg_generic: Vec<Term>,
    /// Per container parameter: the isos converting that parameter's values, or `None`
    /// when the argument is inert (or the inductive itself) and the conversion is identity.
    arg_to: Vec<Option<Conversion>>,
    arg_from: Vec<Option<Conversion>>,
    ctors: Vec<(String, Vec<FieldKind>)>,
    to_name: String,
    from_name: String,
    /// The shared uniform parameters, as variables: every sibling, sibling constructor and
    /// conversion iso is referenced applied to them.
    vars: Vec<Term>,
    /// The block's universe parameters, at which every sibling, sibling constructor and
    /// conversion iso is referenced.
    levels: Vec<String>,
}

impl SibNode {
    /// This sibling at the shared parameters.
    fn applied(&self) -> Term {
        self.at_params(&self.repr)
    }

    /// A global of this compilation (sibling, sibling constructor or iso) at the shared
    /// universe and value parameters.
    fn at_params(&self, name: &str) -> Term {
        apps(reference_at(name, &self.levels), self.vars.clone())
    }

    /// `container generic₁ … genericₙ` — the generic type this sibling represents.
    fn generic(&self) -> Term {
        apps(self.container_head.clone(), self.arg_generic.clone())
    }

    fn constructor(&self, name: &str) -> Term {
        match &self.container_head {
            Term::Const { levels, .. } => Term::Const {
                name: name.into(),
                levels: levels.clone(),
            },
            _ => g(name),
        }
    }

    /// The type of one field in the SPECIALIZED sibling constructor.
    fn sib_field_ty(&self, f: &FieldKind) -> Term {
        match f {
            FieldKind::Param(j) => self.arg_repr[*j].clone(),
            FieldKind::Spine => self.applied(),
            FieldKind::Carried(t) => t.clone(),
            FieldKind::Nested(s) => s.repr.clone(),
            FieldKind::Pointwise {
                point,
                domain,
                result,
            } => pi(point, domain.clone(), self.sib_field_ty(result)),
        }
    }

    /// The type of the same field in the GENERIC container constructor.
    fn gen_field_ty(&self, f: &FieldKind) -> Term {
        match f {
            FieldKind::Param(j) => self.arg_generic[*j].clone(),
            FieldKind::Spine => self.generic(),
            FieldKind::Carried(t) => t.clone(),
            FieldKind::Nested(s) => s.generic.clone(),
            FieldKind::Pointwise {
                point,
                domain,
                result,
            } => pi(point, domain.clone(), self.gen_field_ty(result)),
        }
    }

    /// True if some constructor recurs on the spine, so the isos need a fixpoint.
    fn has_spine(&self) -> bool {
        fn recurs(f: &FieldKind) -> bool {
            match f {
                FieldKind::Spine => true,
                FieldKind::Pointwise { result, .. } => recurs(result),
                _ => false,
            }
        }
        self.ctors.iter().any(|(_, fs)| fs.iter().any(recurs))
    }
}

/// A name fragment identifying a specialized type, so sibling names stay deterministic and
/// distinguish the container's arguments. `TList RTree` → `RTree$TList`, `TList (TList D)` →
/// `D$TList$TList`, `TPair Nat PTree` → `Nat$PTree$TPair`. Injectivity is not relied on for
/// soundness — [`specialize`] refuses outright if two different nestings mangle alike.
fn tag_of(t: &Term) -> String {
    match t {
        Term::Global(n) => n.clone(),
        Term::Const { name, .. } => name.clone(),
        Term::Var(n) => n.clone(),
        // Arguments first, head last — the same shape a specialised name takes, so an inert
        // `TPair Nat Bool` and a specialised one read alike.
        Term::App(..) => {
            let (head, args) = spine(t);
            let mut parts: Vec<String> = args.iter().map(|a| tag_of(a)).collect();
            parts.push(tag_of(head));
            parts.join("$")
        }
        other => sanitize(&format!("{other}")),
    }
}

/// Render a term into a name fragment: alphanumerics and `_` survive, every other run of
/// characters collapses to a single `_`.
fn sanitize(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(ch);
        } else {
            gap = true;
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Analyse one container constructor against the container's arity and the actual arguments
/// of this nesting: return the KIND of each value field, or refuse if the constructor is
/// outside the specialisable fragment (see the module header).
fn analyze_container_ctor(
    ctor: &str,
    ctor_ty: &Term,
    container: &str,
    container_head: &Term,
    arity: usize,
    ctx: &Context,
    nest: &Nesting,
    arguments: &[Spec],
    nodes: &mut Vec<SibNode>,
    seen: &mut Vec<(String, Term)>,
) -> KernelResult<Vec<FieldKind>> {
    let decl_name = nest.name();
    let refuse = |why: String| -> KernelError {
        KernelError::CertificationError(format!(
            "nested-compile: container '{container}' constructor '{ctor}' is outside the \
             specialisable fragment — {why}"
        ))
    };

    let (binders, residual) = peel_pis(ctor_ty);
    if binders.len() < arity {
        return Err(refuse(format!(
            "it binds {} arguments but '{container}' has {arity} parameter(s)",
            binders.len()
        )));
    }
    let params: Vec<String> = binders[..arity].iter().map(|(n, _)| n.clone()).collect();
    // Field positions are assigned by matching a field against a parameter NAME, so two
    // parameters sharing one name would send the shadowed slot's value to the shadowing one.
    for (i, p) in params.iter().enumerate() {
        if params[..i].contains(p) {
            return Err(refuse(format!("it binds two parameters named '{p}'")));
        }
    }
    for (name, ty) in &binders[..arity] {
        if !matches!(ty, Term::Sort(_)) {
            return Err(refuse(format!(
                "its parameter '{name}' is bound at type {ty}, not a sort — the parameter \
                 telescope must be exactly the container's {arity} sort binder(s)"
            )));
        }
    }

    // The result must be the container applied to its own parameters, in order. A permuted
    // or partially-applied result means the field positions do not correspond to the
    // nesting's arguments, so mirroring them would silently produce a DIFFERENT type.
    let expected_result = apps(
        container_head.clone(),
        params.iter().map(|p| v(p)).collect(),
    );
    if residual != expected_result {
        return Err(refuse(format!(
            "it returns {residual}, not {expected_result}"
        )));
    }

    let value_binders = &binders[arity..];
    let mut kinds = Vec::new();
    for (i, (fname, fty)) in value_binders.iter().enumerate() {
        // A dependent field — one mentioning an earlier field's binder — cannot be expressed
        // by the non-dependent sibling constructor this compiler emits.
        for (earlier, _) in &value_binders[..i] {
            if earlier != "_" && mentions_var(fty, earlier) {
                return Err(refuse(format!(
                    "its field '{fname}' depends on the earlier field '{earlier}'"
                )));
            }
        }
        // A `Sort`-valued value field would be counted as a parameter by the kernel's
        // ι-reduction heuristic, so the specialised match would drop it.
        if matches!(fty, Term::Sort(_)) {
            return Err(refuse(format!("its field '{fname}' is itself a sort")));
        }

        if let Some(j) = params
            .iter()
            .position(|p| matches!(fty, Term::Var(n) if n == p))
        {
            kinds.push(FieldKind::Param(j));
            continue;
        }
        if *fty == expected_result {
            kinds.push(FieldKind::Spine);
            continue;
        }
        if let Some(kind) = pointwise_spine(fty, &expected_result, container, decl_name, &params) {
            kinds.push(kind);
            continue;
        }
        if occurs_anywhere(fty, container) {
            return Err(refuse(format!(
                "its field '{fname}' of type {fty} mentions '{container}' somewhere other than \
                 as its own spine {expected_result} — the specialised sibling has ONE fixed \
                 argument list and cannot express a differently-applied self-occurrence"
            )));
        }
        if occurs_anywhere(fty, decl_name) {
            return Err(refuse(format!(
                "its field '{fname}' of type {fty} already mentions '{decl_name}', the inductive \
                 being declared — the mirror cannot tell that occurrence apart from the one it \
                 is specialising"
            )));
        }
        if params.iter().any(|p| mentions_var(fty, p)) {
            let mut generic = fty.clone();
            // Constructor parameters are replaced by closed actual types; dependent
            // value fields were rejected above, before this substitution.
            for (parameter, argument) in params.iter().zip(arguments) {
                generic = crate::substitute(&generic, parameter, &argument.generic);
            }
            // Ask the same trusted positivity rule about the instantiated field
            // before specializing it. A negative arrow is a semantic failure,
            // while unsupported container shapes retain their own diagnostics.
            if let Err(KernelError::PositivityViolation { reason, .. }) =
                crate::positivity::check_positivity_resolved(
                    ctx,
                    decl_name,
                    ctor,
                    &arrow(generic.clone(), nest.uniform()),
                )
            {
                return Err(KernelError::PositivityViolation {
                    inductive: decl_name.into(),
                    constructor: ctor.into(),
                    reason: format!(
                        "container '{container}' field '{fname}' has a negative occurrence: {reason}"
                    ),
                });
            }
            kinds.push(FieldKind::Nested(specialize(
                ctx, nest, &generic, nodes, seen,
            )?));
        } else {
            kinds.push(FieldKind::Carried(fty.clone()));
        }
    }
    Ok(kinds)
}

/// `Π(x₁:D₁) … Π(xₖ:Dₖ). C p₁ … pₙ` — the container recurring on itself under binders whose
/// domains mention neither the container, its parameters nor the inductive — as a pointwise
/// field; `None` for any other shape. The binders are renamed apart from the conversion's own
/// variables, and a later domain may depend on an earlier binder.
fn pointwise_spine(
    ty: &Term,
    spine: &Term,
    container: &str,
    inductive: &str,
    params: &[String],
) -> Option<FieldKind> {
    let (binders, core) = pointwise_telescope(ty);
    let closed = |domain: &Term| {
        !occurs_anywhere(domain, container)
            && !occurs_anywhere(domain, inductive)
            && params.iter().all(|p| !mentions_var(domain, p))
    };
    if binders.is_empty() || core != *spine || !binders.iter().all(|(_, domain)| closed(domain)) {
        return None;
    }
    Some(
        binders
            .into_iter()
            .rev()
            .fold(FieldKind::Spine, |result, (point, domain)| {
                FieldKind::Pointwise {
                    point,
                    domain,
                    result: Box::new(result),
                }
            }),
    )
}

/// The first nested container instance inside `ty` — a registered inductive other than
/// `inductive`, applied to arguments that mention `inductive` — whose arguments also mention the
/// local `point`. Such an instance has no fixed argument list for a sibling to mirror.
fn nesting_over_local(ctx: &Context, ty: &Term, inductive: &str, point: &str) -> Option<Term> {
    let (head, args) = spine(ty);
    if referenced_name(head).is_some_and(|c| c != inductive && ctx.is_inductive(c))
        && args.iter().any(|a| occurs_anywhere(a, inductive))
        && args.iter().any(|a| mentions_var(a, point))
    {
        return Some(ty.clone());
    }
    match ty {
        Term::App(f, x) => nesting_over_local(ctx, f, inductive, point)
            .or_else(|| nesting_over_local(ctx, x, inductive, point)),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => nesting_over_local(ctx, param_type, inductive, point)
            .or_else(|| nesting_over_local(ctx, body_type, inductive, point)),
        _ => None,
    }
}

/// Whether `ty` nests the inductive inside a registered container in a positive position: the
/// container itself, or the codomain of a function type whose domain does not mention it.
fn nests_positively(ctx: &Context, nest: &Nesting, ty: &Term) -> bool {
    match ty {
        Term::Pi {
            param_type,
            body_type,
            ..
        } => !occurs_anywhere(param_type, nest.name()) && nests_positively(ctx, nest, body_type),
        _ => is_nested_container_type(ctx, nest, ty),
    }
}

/// The first occurrence of `container` inside `ty` that is NOT `expected` (the container applied
/// to its own parameter telescope, in order) — the shape that costs a container its uniform
/// parameters. TOTAL over `Term`: a blind spot here would name no culprit and the diagnostic
/// would silently go back to being unactionable.
fn non_uniform_occurrence(ty: &Term, container: &str, expected: &Term) -> Option<Term> {
    // A spine headed by the container is decided HERE, whole: `C p₁ … pₙ` at exactly its own
    // parameters is the uniform occurrence and nothing inside it needs inspecting, and anything
    // else headed by `C` — permuted, partially applied, at a different argument — is the culprit.
    // Recursing into the head instead would report the container's own bare name out of every
    // legitimate occurrence, which is the shape that made the first version of this diagnostic
    // point at the wrong constructor.
    let (head, _) = spine(ty);
    if is_global(head, container) {
        return if ty == expected {
            None
        } else {
            Some(ty.clone())
        };
    }
    match ty {
        Term::Global(_)
        | Term::Const { .. }
        | Term::Sort(_)
        | Term::Var(_)
        | Term::Lit(_)
        | Term::Hole => None,
        Term::App(f, x) => non_uniform_occurrence(f, container, expected)
            .or_else(|| non_uniform_occurrence(x, container, expected)),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => non_uniform_occurrence(param_type, container, expected)
            .or_else(|| non_uniform_occurrence(body_type, container, expected)),
        Term::Lambda {
            param_type, body, ..
        } => non_uniform_occurrence(param_type, container, expected)
            .or_else(|| non_uniform_occurrence(body, container, expected)),
        Term::Fix { body, .. } => non_uniform_occurrence(body, container, expected),
        Term::MutualFix { defs, .. } => defs
            .iter()
            .find_map(|(_, b)| non_uniform_occurrence(b, container, expected)),
        Term::Let {
            ty: t, value, body, ..
        } => non_uniform_occurrence(t, container, expected)
            .or_else(|| non_uniform_occurrence(value, container, expected))
            .or_else(|| non_uniform_occurrence(body, container, expected)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => non_uniform_occurrence(discriminant, container, expected)
            .or_else(|| non_uniform_occurrence(motive, container, expected))
            .or_else(|| {
                cases
                    .iter()
                    .find_map(|c| non_uniform_occurrence(c, container, expected))
            }),
        Term::Annotated { term, .. } => non_uniform_occurrence(term, container, expected),
    }
}

/// The constructor that denies `container` a uniform `arity`-long parameter telescope, with the
/// reason — for the INDEXED-family refusal's diagnostic. `None` when every constructor is uniform
/// (the split was declared rather than inferred, and the declaration is the authority).
fn non_uniform_constructor(
    ctx: &Context,
    container: &str,
    arity: usize,
) -> Option<(String, String)> {
    for (cname, cty) in ctx.get_constructors(container) {
        let (binders, residual) = peel_pis(cty);
        if binders.len() < arity {
            return Some((
                cname.to_string(),
                format!(
                    "it binds {} argument(s), fewer than the {arity} the arity declares",
                    binders.len()
                ),
            ));
        }
        let ps: Vec<String> = binders[..arity].iter().map(|(n, _)| n.clone()).collect();
        let expected = apps(g(container), ps.iter().map(|p| v(p)).collect());
        if residual != expected {
            return Some((
                cname.to_string(),
                format!("it returns {residual}, not {expected}"),
            ));
        }
        for (fname, fty) in &binders[arity..] {
            if let Some(bad) = non_uniform_occurrence(fty, container, &expected) {
                return Some((
                    cname.to_string(),
                    format!(
                        "its field '{fname}' contains the occurrence {bad}, which is not \
                         '{container}' at its own parameters {expected}"
                    ),
                ));
            }
        }
    }
    None
}

/// Recursively specialize a nested type or one container argument, accumulating the siblings
/// to emit (in POST-ORDER, so an inner sibling is registered before the outer sibling whose
/// isos reference it). Three cases: an argument not mentioning the inductive is INERT and
/// stands for itself; the inductive itself is the BASE leaf; and `C a₁ … aₙ` specialises each
/// argument first, then mirrors `C` with every parameter fixed.
fn specialize(
    ctx: &Context,
    nest: &Nesting,
    ty: &Term,
    nodes: &mut Vec<SibNode>,
    seen: &mut Vec<(String, Term)>,
) -> KernelResult<Spec> {
    if !occurs_anywhere(ty, nest.name()) {
        return Ok(Spec {
            repr: ty.clone(),
            tag: tag_of(ty),
            generic: ty.clone(),
            to: None,
            from: None,
        });
    }
    // The base leaf is the inductive at exactly its own parameters. Any other application of
    // it — at a different argument, partially applied — is a non-uniform occurrence no
    // specialized sibling can stand for, and is refused below.
    if *ty == nest.uniform() {
        return Ok(Spec {
            repr: nest.uniform(),
            tag: nest.name().to_string(),
            generic: nest.uniform(),
            to: None,
            from: None,
        });
    }

    // A POSITIVE function type `Π(x:D). B`: Lean's elimination reaches under the binder, so the
    // nesting in `B` is specialised there, the binder rides along unchanged, and the conversion is
    // pointwise. A domain that mentions the inductive is a negative occurrence no specialisation
    // mends; the callers' positivity checks refuse it first, and this refusal backs them.
    if matches!(ty, Term::Pi { .. }) {
        let (binders, core) = pointwise_telescope(ty);
        if let Some((_, domain)) = binders
            .iter()
            .find(|(_, domain)| occurs_anywhere(domain, nest.name()))
        {
            return Err(KernelError::CertificationError(format!(
                "nested-compile: '{}' occurs in the domain {domain} of {ty}, which is not a positive \
                 position",
                nest.name()
            )));
        }
        if let Some(instance) = binders
            .iter()
            .find_map(|(point, _)| nesting_over_local(ctx, &core, nest.name(), point))
        {
            return Err(KernelError::InvalidDeclaration(format!(
                "nested-compile: the nested occurrence {instance} in {ty} mentions a bound variable of \
                 the function type in a container argument"
            )));
        }
        let result = specialize(ctx, nest, &core, nodes, seen)?;
        let close = |body: Term| {
            binders
                .iter()
                .rev()
                .fold(body, |b, (point, domain)| pi(point, domain.clone(), b))
        };
        let pointwise = |conversion: Option<Conversion>| {
            conversion.map(|c| {
                binders
                    .iter()
                    .rev()
                    .fold(c, |result, (point, domain)| Conversion::Pointwise {
                        point: point.clone(),
                        domain: domain.clone(),
                        result: Box::new(result),
                    })
            })
        };
        let domains: Vec<String> = binders.iter().map(|(_, domain)| tag_of(domain)).collect();
        return Ok(Spec {
            repr: close(result.repr),
            tag: format!("{}_to_{}", domains.join("_"), result.tag),
            generic: close(result.generic),
            to: pointwise(result.to),
            from: pointwise(result.from),
        });
    }

    let (head, args) = spine(ty);
    // The inductive applied at anything but its own parameters breaks parameter uniformity.
    // That is not a shape this compiler lacks; it is an invalid declaration, and Lean's kernel
    // refuses it too ("inductive datatype parameter mismatch").
    if referenced_name(head) == Some(nest.name()) {
        return Err(KernelError::InvalidDeclaration(format!(
            "nested-compile: '{}' occurs as {ty}, not at its own uniform parameters {}",
            nest.name(),
            nest.uniform()
        )));
    }
    let container = match head {
        Term::Global(c) | Term::Const { name: c, .. }
            if c != nest.name() && ctx.is_inductive(c) =>
        {
            c.clone()
        }
        _ => {
            return Err(KernelError::CertificationError(format!(
                "nested-compile: '{}' occurs in argument type {ty} in a position that is not a \
                 nesting inside a registered container, nor the inductive at its own \
                 parameters {} (only `Container …` nestings are specialized)",
                nest.name(),
                nest.uniform()
            )));
        }
    };

    // Validate the universe instance before deriving specialized names.
    crate::infer_type(ctx, head)?;
    let arity = ctx.inductive_arity(&container);
    if arity == 0 || args.len() != arity {
        return Err(KernelError::CertificationError(format!(
            "nested-compile: container '{container}' has arity {arity} but is applied to {} \
             argument(s) in {ty} — only a fully-applied container is specialisable",
            args.len()
        )));
    }
    if ctx.inductive_num_params(&container) != arity {
        // Name the CONSTRUCTOR that costs the container its uniform parameter telescope. "1 of
        // its 1 arguments are indices" is a true sentence a reader cannot act on: the split is
        // INFERRED from the constructors, so the repair is always in one of them, and a
        // diagnostic that does not say which leaves the author to re-derive the inference by
        // hand. Computed only on this path, so the ordinary compile pays nothing for it.
        let culprit = match non_uniform_constructor(ctx, &container, arity) {
            Some((c, why)) => format!(" — '{c}' is why: {why}"),
            None => String::new(),
        };
        return Err(KernelError::CertificationError(format!(
            "nested-compile: container '{container}' is an INDEXED family ({} of its {arity} \
             arguments are indices) — specialising it would fix an index as if it were a \
             uniform parameter{culprit}",
            ctx.inductive_num_indices(&container)
        )));
    }

    // Specialise every argument first (post-order), inert ones standing for themselves.
    let mut arg_specs = Vec::with_capacity(arity);
    for a in &args {
        arg_specs.push(specialize(ctx, nest, a, nodes, seen)?);
    }

    let repr = {
        let tags: Vec<&str> = arg_specs.iter().map(|s| s.tag.as_str()).collect();
        let instance = match head {
            Term::Const { levels, .. } => format!("$_{}", sanitize(&format!("{levels:?}"))),
            _ => String::new(),
        };
        format!("{}${}{}", tags.join("$"), container, instance)
    };
    let generic = apps(
        head.clone(),
        arg_specs.iter().map(|s| s.generic.clone()).collect(),
    );
    let to_name = format!("{repr}_to_{container}");
    let from_name = format!("{repr}_from_{container}");

    // Soundness does not rest on the mangling being injective: a repr already claimed by a
    // DIFFERENT nesting is refused outright rather than silently reusing the wrong sibling.
    match seen.iter().find(|(r, _)| *r == repr) {
        Some((_, claimed)) if *claimed == generic => {
            return Ok(Spec {
                repr: nest.applied(&repr),
                tag: repr,
                generic,
                to: Some(Conversion::Iso(to_name)),
                from: Some(Conversion::Iso(from_name)),
            });
        }
        Some((_, claimed)) => {
            return Err(KernelError::CertificationError(format!(
                "nested-compile: the specialised name '{repr}' would stand for both {claimed} \
                 and {generic}"
            )));
        }
        None => seen.push((repr.clone(), generic.clone())),
    }

    let actual: Vec<Term> = arg_specs.iter().map(|s| s.repr.clone()).collect();
    let mut ctors = Vec::new();
    for (orig_ctor, _) in ctx.get_constructors(&container) {
        let reference = match head {
            Term::Const { levels, .. } => Term::Const {
                name: orig_ctor.into(),
                levels: levels.clone(),
            },
            _ => g(orig_ctor),
        };
        let orig_ty = crate::infer_type(ctx, &reference)?;
        let kinds = analyze_container_ctor(
            orig_ctor, &orig_ty, &container, head, arity, ctx, nest, &arg_specs, nodes, seen,
        )?;
        ctors.push((orig_ctor.to_string(), kinds));
    }

    nodes.push(SibNode {
        container: container.clone(),
        container_head: head.clone(),
        repr: repr.clone(),
        arg_repr: actual,
        arg_generic: arg_specs.iter().map(|s| s.generic.clone()).collect(),
        arg_to: arg_specs.iter().map(|s| s.to.clone()).collect(),
        arg_from: arg_specs.iter().map(|s| s.from.clone()).collect(),
        ctors,
        to_name: to_name.clone(),
        from_name: from_name.clone(),
        vars: nest.vars(),
        levels: nest.levels.clone(),
    });

    Ok(Spec {
        repr: nest.applied(&repr),
        tag: repr,
        generic,
        to: Some(Conversion::Iso(to_name)),
        from: Some(Conversion::Iso(from_name)),
    })
}

/// Compile a nested inductive into a mutual block plus conversion isos. Untrusted: every
/// artifact is checked by the caller. Handles arbitrary nesting DEPTH (`TList (TList RTree)`)
/// and arbitrary container ARITY (`TPair Nat PTree`) by recursive specialization — each
/// container level becomes its own mutual sibling.
pub fn compile_nested(ctx: &Context, decl: &NestedDecl) -> KernelResult<Compiled> {
    compile_nested_with_params(ctx, decl, 0)
}

/// [`compile_nested`] for an inductive with `num_params` uniform parameters
/// (`PTree α := node : α → List (PTree α) → PTree α`).
///
/// The parameters are split off the sort and every constructor onto one canonical telescope
/// (see `Nesting`). The base leaf of specialization is the inductive at exactly those
/// parameters; every specialized sibling takes the same telescope as its uniform parameters
/// (`PTree$List α`), and every conversion iso abstracts it. An occurrence of the inductive at
/// any other arguments is a non-uniform nesting and is refused, as are constructor results
/// that do not return the inductive at its own parameters.
pub fn compile_nested_with_params(
    ctx: &Context,
    decl: &NestedDecl,
    num_params: usize,
) -> KernelResult<Compiled> {
    compile_nested_poly(ctx, decl, num_params, &[])
}

/// [`compile_nested_with_params`] for a universe-polymorphic family (`UTree.{u} (α : Type u)`).
///
/// Every emitted member is universe polymorphic over exactly the declaration's `levels` and
/// lives in the declaration's own sort; the inductive, its siblings and their constructors are
/// referenced at exactly those levels. An occurrence of the inductive at any other universe
/// instance is non-uniform and refused, as Lean refuses it.
pub fn compile_nested_poly(
    ctx: &Context,
    decl: &NestedDecl,
    num_params: usize,
    levels: &[String],
) -> KernelResult<Compiled> {
    let nest = nesting(decl, num_params, levels)?;

    // Specialization may shrink a container instance into a smaller sibling.
    // Preserve the source declaration's universe obligations before rewriting fields.
    let mut source = ctx.clone();
    if levels.is_empty() {
        source.add_indexed_inductive(&decl.name, decl.sort.clone(), num_params);
    } else {
        source.add_universe_poly_inductive(
            &decl.name,
            levels.to_vec(),
            decl.sort.clone(),
            num_params,
        );
    }
    for (name, ty) in &decl.constructors {
        crate::type_checker::check_constructor_universes(&source, &decl.name, name, ty)?;
    }

    // 1. The inductive's own constructors, with each NESTED argument (`TList name`,
    //    `TPair Nat name`, `TList (TList name)`, …) replaced by its outermost specialized
    //    sibling — recording every sibling that must be emitted (post-order, inner-first).
    let mut nodes: Vec<SibNode> = Vec::new();
    let mut seen: Vec<(String, Term)> = Vec::new();
    let mut own_ctors = Vec::new();
    for (cname, cty) in &nest.constructors {
        let (fields_in, residual) = peel_pis(cty);
        // Every member is emitted with the shared parameters and no indices, so every
        // constructor must land on the inductive at exactly its own parameters; an indexed or
        // differently-applied result has no sibling to name.
        if residual != nest.uniform() {
            return Err(KernelError::CertificationError(format!(
                "nested-compile: constructor '{cname}' returns {residual}, not {} — a nested \
                 inductive's constructors return it at its own uniform parameters",
                nest.uniform()
            )));
        }
        // Specialise in FIELD order, so the siblings are emitted in the order the source
        // mentions them, then rebuild the telescope inside out.
        // Lean refuses a negative occurrence before it eliminates any nesting; so does this, so
        // a nesting in a domain is the positivity violation it is rather than an unspecialised
        // field. Other outcomes of the check are left to the specialisation and the block door.
        if let Err(violation @ KernelError::PositivityViolation { .. }) =
            crate::positivity::check_positivity_resolved(ctx, nest.name(), cname, cty)
        {
            return Err(violation);
        }
        let mut fields = Vec::with_capacity(fields_in.len());
        for (pname, pty) in fields_in {
            let pty = if nests_positively(ctx, &nest, &pty) {
                specialize(ctx, &nest, &pty, &mut nodes, &mut seen)?.repr
            } else {
                pty
            };
            fields.push((pname, pty));
        }
        let mut rebuilt = residual;
        for (pname, pty) in fields.into_iter().rev() {
            rebuilt = pi(&pname, pty, rebuilt);
        }
        own_ctors.push((cname.clone(), nest.close_pi(rebuilt)));
    }

    if nodes.is_empty() {
        return Err(KernelError::CertificationError(format!(
            "nested-compile: '{}' has no nested container occurrence — register it directly",
            decl.name
        )));
    }

    // 2. The mutual block: the inductive itself, then one specialized sibling per node.
    let mut block = vec![MutualInductive {
        name: decl.name.clone(),
        sort: nest.close_pi(nest.sort.clone()),
        num_params,
        constructors: own_ctors,
    }];
    let mut siblings = Vec::new();
    for node in &nodes {
        siblings.push(node.repr.clone());
        let mut sib_ctors = Vec::new();
        for (orig, kinds) in &node.ctors {
            let mut sty = node.applied();
            for kind in kinds.iter().rev() {
                sty = arrow(node.sib_field_ty(kind), sty);
            }
            sib_ctors.push((format!("{}_{}", node.repr, orig), nest.close_pi(sty)));
        }
        block.push(MutualInductive {
            name: node.repr.clone(),
            sort: nest.close_pi(nest.sort.clone()),
            num_params,
            constructors: sib_ctors,
        });
    }

    // 2b. THE PARADOX FENCE, with references RESOLVED — before the block leaves this function
    //     and long before `add_mutual_inductives` runs its own (name-comparing) check.
    //
    //     The nested door has the same δ-shaped hole every declaration door has: declare
    //     `LTree` as an inductive, define `Negate := LTree → False`, then nest through a
    //     container while carrying a `Negate` field. `Negate` does not SPELL `LTree`, so a
    //     fence that compares names sees no occurrence and the Russell constructor
    //     `(LTree → False) → LTree` is admitted. `check_positivity_mutual_resolved` resolves
    //     the reference first, so the field is judged as the `LTree → False` it is.
    //
    //     Checking here rather than trusting the registration door also keeps the refusal at
    //     the place that can name the CONSTRUCTOR — the specialised sibling's field ordering is
    //     this function's doing, and a violation reported from inside the emitted mutual block
    //     would name a constructor the source never wrote.
    {
        let names: Vec<&str> = block.iter().map(|m| m.name.as_str()).collect();
        for member in &block {
            for (cname, cty) in &member.constructors {
                crate::positivity::check_positivity_mutual_resolved(ctx, &names, cname, cty)?;
            }
        }
    }

    // 3. The conversion isos, one pair per node — emitted inner-first so the outer iso's
    //    element-field delegation to the inner iso resolves when it is kernel-checked. Each
    //    abstracts the shared parameters, and delegates to other isos at those parameters.
    let mut isos = Vec::new();
    let mut iso_names = Vec::new();
    for node in &nodes {
        let to_ty = nest.close_pi(arrow(node.applied(), node.generic()));
        let from_ty = nest.close_pi(arrow(node.generic(), node.applied()));
        isos.push((
            node.to_name.clone(),
            to_ty,
            nest.close_lam(build_to_generic(node)),
        ));
        isos.push((
            node.from_name.clone(),
            from_ty,
            nest.close_lam(build_from_generic(node)),
        ));
        iso_names.push(IsoNames {
            container: node.container.clone(),
            sibling: node.repr.clone(),
            to_generic: node.to_name.clone(),
            from_generic: node.from_name.clone(),
        });
    }

    Ok(Compiled {
        block,
        isos,
        iso_names,
        siblings,
        parameters: nest.params.clone(),
    })
}

/// Wrap a conversion body `λ x : dom. match x …` in a fixpoint only when some constructor
/// recurs on the spine — a container with no self-recursion converts without one.
fn fixpoint(node: &SibNode, dom: Term, motive_body: Term, cases: Vec<Term>) -> Term {
    let body = lam(
        "x",
        dom.clone(),
        Term::Match {
            discriminant: Box::new(v("x")),
            motive: Box::new(lam("_", dom, motive_body)),
            cases,
        },
    );
    if node.has_spine() {
        Term::Fix {
            name: "rec".to_string(),
            body: Box::new(body),
        }
    } else {
        body
    }
}

/// `to_generic : sibling → C generic₁ … genericₙ` — rebuild each sibling constructor as the
/// corresponding container constructor at the arguments' GENERIC types, recursing on spine
/// fields and delegating each parameter field to that parameter's own `to` iso (identity at
/// an inert argument or the base leaf).
fn build_to_generic(node: &SibNode) -> Term {
    let mut cases = Vec::new();
    for (orig_ctor, kinds) in &node.ctors {
        // λa₀ … a_{k-1}. C_ctor generic₁ … genericₙ (convert a₀) … (convert a_{k-1})
        let mut body = apps(node.constructor(orig_ctor), node.arg_generic.clone());
        for (i, kind) in kinds.iter().enumerate() {
            body = app(
                body,
                convert(node, kind, v(&format!("a{i}")), &node.arg_to, true),
            );
        }
        for (i, kind) in kinds.iter().enumerate().rev() {
            body = lam(&format!("a{i}"), node.sib_field_ty(kind), body);
        }
        cases.push(body);
    }
    fixpoint(node, node.applied(), node.generic(), cases)
}

/// `from_generic : C generic₁ … genericₙ → sibling` — rebuild each container constructor
/// (matched at the arguments' generic types) as the corresponding sibling constructor,
/// recursing on the spine and delegating each parameter field to that parameter's `from` iso.
fn build_from_generic(node: &SibNode) -> Term {
    let mut cases = Vec::new();
    for (orig_ctor, kinds) in &node.ctors {
        let mut body = node.at_params(&format!("{}_{}", node.repr, orig_ctor));
        for (i, kind) in kinds.iter().enumerate() {
            body = app(
                body,
                convert(node, kind, v(&format!("a{i}")), &node.arg_from, false),
            );
        }
        for (i, kind) in kinds.iter().enumerate().rev() {
            // The binder types are the CONTAINER constructor's value fields at the generic
            // arguments.
            body = lam(&format!("a{i}"), node.gen_field_ty(kind), body);
        }
        cases.push(body);
    }
    fixpoint(node, node.generic(), node.applied(), cases)
}

/// Convert one field of a container constructor: recurse (`rec a`) on a spine occurrence, and
/// delegate a parameter field to that parameter's iso (identity when the argument is inert or is
/// the inductive itself).
fn convert(
    node: &SibNode,
    kind: &FieldKind,
    a: Term,
    isos: &[Option<Conversion>],
    to_generic: bool,
) -> Term {
    match kind {
        FieldKind::Spine => app(v("rec"), a),
        FieldKind::Param(j) => apply_conversion(node, isos[*j].as_ref(), a),
        // A carried field has the SAME type on both sides — there is nothing to convert, and
        // anything other than the identity here would be a value the round trip does not return.
        FieldKind::Carried(_) => a,
        FieldKind::Nested(spec) => apply_conversion(
            node,
            if to_generic {
                spec.to.as_ref()
            } else {
                spec.from.as_ref()
            },
            a,
        ),
        FieldKind::Pointwise {
            point,
            domain,
            result,
        } => lam(
            point,
            domain.clone(),
            convert(node, result, app(a, v(point)), isos, to_generic),
        ),
    }
}

/// `conversion` applied to `a`; no conversion is the identity.
fn apply_conversion(node: &SibNode, conversion: Option<&Conversion>, a: Term) -> Term {
    match conversion {
        None => a,
        Some(Conversion::Iso(iso)) => app(node.at_params(iso), a),
        Some(Conversion::Pointwise {
            point,
            domain,
            result,
        }) => lam(
            point,
            domain.clone(),
            apply_conversion(node, Some(result), app(a, v(point))),
        ),
    }
}

/// A recursor whose motive and constructor terms retain the source container applications.
pub struct NativeRecursor {
    /// The source datatype or a fully applied original container instance, open exactly over
    /// [`NativeRecursor::parameters`].
    pub carrier: Term,
    /// The canonical uniform parameter telescope the carrier is open over — the telescope the
    /// recursor's type and body abstract first. Empty for an unparameterized nested inductive.
    pub parameters: Vec<(String, Term)>,
    /// Independent motive universe parameters, in motive order.
    pub levels: Vec<String>,
    /// The recursor telescope over original carrier and constructor terms.
    pub ty: Term,
    /// The guarded implementation, checked in the original source context.
    pub body: Term,
}

/// Reconstruct checked generic recursors from the existing mutual generator.
///
/// The source nested datatype must already be checked and registered in ctx.
/// Results, motives and minor groups follow source/container discovery order.
/// Specialization lives in a private context; each restored implementation is
/// checked again against the original generic declarations before being returned.
pub fn derive_native_recursors(
    ctx: &Context,
    decl: &NestedDecl,
) -> KernelResult<Vec<NativeRecursor>> {
    derive_native_recursors_with_params(ctx, decl, 0)
}

/// [`derive_native_recursors`] for a nested inductive with `num_params` uniform parameters.
/// The restored recursors abstract the parameters first, exactly as Lean's generated nested
/// recursors do (`PTree.rec : {α : Type} → {motive_1 …} → {motive_2 …} → …`).
pub fn derive_native_recursors_with_params(
    ctx: &Context,
    decl: &NestedDecl,
    num_params: usize,
) -> KernelResult<Vec<NativeRecursor>> {
    derive_native_recursors_poly(ctx, decl, num_params, &[])
}

/// [`derive_native_recursors_with_params`] for a universe-polymorphic family. Each restored
/// recursor's levels are its motive universes followed by the family's own `levels`, in order.
pub fn derive_native_recursors_poly(
    ctx: &Context,
    decl: &NestedDecl,
    num_params: usize,
    levels: &[String],
) -> KernelResult<Vec<NativeRecursor>> {
    derive_nested_block_recursors(
        ctx,
        &NestedBlock {
            members: std::slice::from_ref(decl),
            num_params,
            levels,
        },
    )
}

/// A block of source families, each nesting itself or a sibling inside registered containers:
/// one family, or an ordinary mutual block. The members share `num_params` uniform parameters
/// and the universe parameters `levels`, and may each carry indices of their own.
pub struct NestedBlock<'a> {
    pub members: &'a [NestedDecl],
    pub num_params: usize,
    pub levels: &'a [String],
}

/// The auxiliary mutual block Lean's kernel reduces a nested block to.
pub struct Elimination {
    /// The source members with every nested occurrence replaced, then one auxiliary family per
    /// distinct nested container instance — in Lean's discovery order.
    pub block: Vec<MutualInductive>,
    /// Each auxiliary family and auxiliary constructor, by name, and the source term it stands
    /// for, open over [`Elimination::parameters`].
    pub restoration: Vec<(String, Term)>,
    pub parameters: Vec<(String, Term)>,
}

/// Equality of rigid result universes, not one-way cumulativity or parameter unification.
fn nested_sorts_equal(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::Sort(a), Term::Sort(b)) => a.equiv(b),
        _ => false,
    }
}

/// Reduce a nested block to an ordinary mutual block exactly as Lean's kernel does
/// (`elim_nested_inductive`).
///
/// Every constructor field of every member is scanned left to right, top-down. A fully applied
/// inductive `C A₁ … Aₚ i₁ … iᵣ` that is not a member, whose PARAMETERS mention a member, is a
/// nested occurrence: it becomes an auxiliary family `Aux P i₁ … iᵣ` over the block's
/// parameters, one per distinct `C A₁ … Aₚ`, whose constructors are `C`'s constructors at
/// `A₁ … Aₚ` rewritten the same way. Auxiliary families join the end of the queue, so the block
/// order — and with it the recursors, motives and minor premises — is Lean's (all source
/// members first, then each auxiliary as it is discovered: `List (Prod α T)` before the
/// `Prod α T` inside it). Indices stay indices: an indexed container becomes an indexed
/// auxiliary family. A nested occurrence whose parameters mention a constructor's own field
/// binders is an invalid declaration, as Lean says; one whose indices mention the block is
/// declined. Nothing here is trusted: the caller registers the block through the trusted mutual
/// door, which checks positivity and universes.
pub fn eliminate_nested(ctx: &Context, block: &NestedBlock) -> KernelResult<Elimination> {
    let refuse =
        |why: String| KernelError::CertificationError(format!("nested-elimination: {why}"));
    let first = block
        .members
        .first()
        .ok_or_else(|| refuse("an empty block".into()))?;
    let nests = block
        .members
        .iter()
        .map(|member| nesting_indexed(member, block.num_params))
        .collect::<KernelResult<Vec<_>>>()?;
    let params = nests[0].params.clone();
    for (member, nest) in block.members.iter().zip(&nests) {
        if nest.params != params {
            return Err(refuse(format!(
                "'{}' does not share the block's parameters",
                member.name
            )));
        }
    }
    let sort = nests[0].sort.clone();
    if nests
        .iter()
        .any(|nest| !nested_sorts_equal(&nest.sort, &sort))
    {
        return Err(refuse("the block's members live in different sorts".into()));
    }
    let mut elimination = Eliminator {
        ctx,
        params: params.clone(),
        levels: block.levels.to_vec(),
        members: block.members.iter().map(|m| m.name.clone()).collect(),
        base: first.name.clone(),
        auxiliaries: Vec::new(),
    };
    let mut emitted = Vec::new();
    for (member, nest) in block.members.iter().zip(&nests) {
        let mut constructors = Vec::new();
        for (name, ty) in &nest.constructors {
            let rewritten = elimination.replace(ty, &mut Vec::new())?;
            constructors.push((name.clone(), close_pi(&params, rewritten)));
        }
        emitted.push(MutualInductive {
            name: member.name.clone(),
            sort: close_pi(&params, nest.indexed_sort.clone()),
            num_params: block.num_params,
            constructors,
        });
    }
    let mut restoration = Vec::new();
    let mut next = 0;
    while next < elimination.auxiliaries.len() {
        let (name, container, arguments) = {
            let aux = &elimination.auxiliaries[next];
            (
                aux.name.clone(),
                aux.container.clone(),
                aux.arguments.clone(),
            )
        };
        let container_name = referenced_name(&container)
            .expect("a container reference")
            .to_string();
        let instance = |constructor: &str| match &container {
            Term::Const { levels, .. } => Term::Const {
                name: constructor.into(),
                levels: levels.clone(),
            },
            _ => g(constructor),
        };
        // The auxiliary family's own telescope: the container's indices at these arguments.
        let mut arity = crate::infer_type(ctx, &container)?;
        for argument in &arguments {
            let Term::Pi {
                param, body_type, ..
            } = arity
            else {
                return Err(refuse(format!(
                    "container '{container_name}' has fewer parameters than applied"
                )));
            };
            arity = crate::substitute(&body_type, &param, argument);
        }
        let mut result = &arity;
        while let Term::Pi { body_type, .. } = result {
            result = body_type;
        }
        if !nested_sorts_equal(result, &sort) {
            return Err(refuse(format!(
                "the nested instance {} lives in {result}, not the block's {sort}",
                apps(container.clone(), arguments.clone())
            )));
        }
        let mut constructors = Vec::new();
        for (constructor, _) in ctx.get_constructors(&container_name) {
            let mut ty = crate::infer_type(ctx, &instance(constructor))?;
            for argument in &arguments {
                let Term::Pi {
                    param, body_type, ..
                } = ty
                else {
                    return Err(refuse(format!(
                        "constructor '{constructor}' binds fewer parameters than its family"
                    )));
                };
                ty = crate::substitute(&body_type, &param, argument);
            }
            let rewritten = elimination.replace(&ty, &mut Vec::new())?;
            let auxiliary_constructor = format!("{name}${constructor}");
            restoration.push((
                auxiliary_constructor.clone(),
                apps(instance(constructor), arguments.clone()),
            ));
            constructors.push((auxiliary_constructor, close_pi(&params, rewritten)));
        }
        restoration.push((name.clone(), apps(container.clone(), arguments.clone())));
        emitted.push(MutualInductive {
            name,
            sort: close_pi(&params, arity),
            num_params: block.num_params,
            constructors,
        });
        next += 1;
    }
    if elimination.auxiliaries.is_empty() {
        return Err(refuse(format!(
            "'{}' has no nested occurrence — register it directly",
            first.name
        )));
    }
    Ok(Elimination {
        block: emitted,
        restoration,
        parameters: params,
    })
}

fn close_pi(params: &[(String, Term)], body: Term) -> Term {
    params
        .iter()
        .rev()
        .fold(body, |b, (p, t)| pi(p, t.clone(), b))
}

/// A member with its uniform parameters split onto the canonical telescope; indices may follow.
struct IndexedNesting {
    params: Vec<(String, Term)>,
    /// The member's final sort.
    sort: Term,
    /// The member's telescope after its parameters: its indices, then its sort.
    indexed_sort: Term,
    /// Each constructor's field telescope and result, over the canonical parameters.
    constructors: Vec<(String, Term)>,
}

fn nesting_indexed(decl: &NestedDecl, num_params: usize) -> KernelResult<IndexedNesting> {
    let refuse = |why: String| {
        KernelError::CertificationError(format!("nested-elimination: '{}' {why}", decl.name))
    };
    let mut params = Vec::with_capacity(num_params);
    let mut indexed_sort = decl.sort.clone();
    for i in 0..num_params {
        let name = format!("nested.parameter.{i}");
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = indexed_sort
        else {
            return Err(refuse(format!(
                "declares {num_params} parameter(s) but its sort binds fewer"
            )));
        };
        params.push((name.clone(), *param_type));
        indexed_sort = crate::substitute(&body_type, &param, &v(&name));
    }
    let mut sort = &indexed_sort;
    while let Term::Pi { body_type, .. } = sort {
        sort = body_type;
    }
    if !matches!(sort, Term::Sort(_)) {
        return Err(refuse(format!("ends in {sort}, not a sort")));
    }
    let sort = sort.clone();
    let mut constructors = Vec::with_capacity(decl.constructors.len());
    for (cname, cty) in &decl.constructors {
        let mut ty = cty.clone();
        for (name, domain) in &params {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = ty
            else {
                return Err(refuse(format!(
                    "constructor '{cname}' binds fewer than its {num_params} parameter(s)"
                )));
            };
            if *param_type != *domain {
                return Err(refuse(format!(
                    "constructor '{cname}' binds parameter {name} at {param_type}, but the sort binds it \
                     at {domain} — nested parameters are uniform")));
            }
            ty = crate::substitute(&body_type, &param, &v(name));
        }
        constructors.push((cname.clone(), ty));
    }
    Ok(IndexedNesting {
        params,
        sort,
        indexed_sort,
        constructors,
    })
}

/// One auxiliary family: the nested container instance `container arguments…` it stands for.
struct Auxiliary {
    name: String,
    container: Term,
    arguments: Vec<Term>,
}

struct Eliminator<'a> {
    ctx: &'a Context,
    params: Vec<(String, Term)>,
    levels: Vec<String>,
    /// Source members, then auxiliary families as they are discovered.
    members: Vec<String>,
    base: String,
    auxiliaries: Vec<Auxiliary>,
}

impl Eliminator<'_> {
    fn mentions_block(&self, term: &Term) -> bool {
        self.members.iter().any(|m| occurs_anywhere(term, m))
    }

    /// Rewrite every nested occurrence in `term`, top-down. `locals` are the binders of the
    /// constructor telescope in scope; a nested container's parameters may not mention them.
    fn replace(&mut self, term: &Term, locals: &mut Vec<String>) -> KernelResult<Term> {
        let (head, args) = spine(term);
        if let Some(container) = referenced_name(head) {
            let container = container.to_string();
            if !self.members.contains(&container) && self.ctx.is_inductive(&container) {
                let p = self.ctx.inductive_num_params(&container);
                if args.len() == self.ctx.inductive_arity(&container)
                    && p <= args.len()
                    && args[..p].iter().any(|a| self.mentions_block(a))
                {
                    if let Some(local) = locals
                        .iter()
                        .find(|l| args[..p].iter().any(|a| mentions_var(a, l)))
                    {
                        return Err(KernelError::InvalidDeclaration(format!(
                            "nested-elimination: the nested occurrence {term} mentions the constructor's \
                             own binder '{local}' in a container parameter")));
                    }
                    if args[p..].iter().any(|i| self.mentions_block(i)) {
                        return Err(KernelError::CertificationError(format!(
                            "nested-elimination: the nested occurrence {term} mentions the block in an index")));
                    }
                    let arguments: Vec<Term> = args[..p].iter().map(|a| (*a).clone()).collect();
                    let name = self.auxiliary(head, &arguments)?;
                    let mut applied = apps(
                        reference_at(&name, &self.levels),
                        self.params.iter().map(|(p, _)| v(p)).collect(),
                    );
                    for index in &args[p..] {
                        applied = app(applied, (*index).clone());
                    }
                    return Ok(applied);
                }
            }
        }
        Ok(match term {
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let domain = self.replace(param_type, locals)?;
                locals.push(param.clone());
                let body = self.replace(body_type, locals);
                locals.pop();
                pi(param, domain, body?)
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let domain = self.replace(param_type, locals)?;
                locals.push(param.clone());
                let inner = self.replace(body, locals);
                locals.pop();
                lam(param, domain, inner?)
            }
            Term::App(f, a) => app(self.replace(f, locals)?, self.replace(a, locals)?),
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let ty = self.replace(ty, locals)?;
                let value = self.replace(value, locals)?;
                locals.push(name.clone());
                let inner = self.replace(body, locals);
                locals.pop();
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(ty),
                    value: Box::new(value),
                    body: Box::new(inner?),
                }
            }
            other => other.clone(),
        })
    }

    /// The auxiliary family standing for `container arguments…`, created on first discovery.
    fn auxiliary(&mut self, container: &Term, arguments: &[Term]) -> KernelResult<String> {
        if let Some(aux) = self
            .auxiliaries
            .iter()
            .find(|aux| aux.container == *container && aux.arguments == arguments)
        {
            return Ok(aux.name.clone());
        }
        let name = format!("{}$nested{}", self.base, self.auxiliaries.len() + 1);
        if self.ctx.get_global(&name).is_some() || self.members.contains(&name) {
            return Err(KernelError::CertificationError(format!(
                "nested-elimination: the auxiliary name '{name}' is already taken"
            )));
        }
        self.members.push(name.clone());
        self.auxiliaries.push(Auxiliary {
            name: name.clone(),
            container: container.clone(),
            arguments: arguments.to_vec(),
        });
        Ok(name)
    }
}

/// Reconstruct Lean's recursors for a nested block: the elimination's mutual recursors, each
/// restored to the source families and container instances and checked again in `ctx`.
///
/// The source members must already be registered in `ctx` exactly as declared. Recursors come in
/// block order: the source members, then the auxiliary families in discovery order.
pub fn derive_nested_block_recursors(
    ctx: &Context,
    block: &NestedBlock,
) -> KernelResult<Vec<NativeRecursor>> {
    for decl in block.members {
        if ctx.get_global(&decl.name) != Some(&decl.sort)
            || ctx.inductive_num_params(&decl.name) != block.num_params
            || ctx
                .get_constructors(&decl.name)
                .iter()
                .map(|(n, t)| ((*n).to_string(), (*t).clone()))
                .collect::<Vec<_>>()
                != decl.constructors
        {
            return Err(KernelError::CertificationError(
                "native nested recursor requires the exact registered source declaration".into(),
            ));
        }
    }
    let elimination = eliminate_nested(ctx, block)?;
    let elimination = native_specialization::prepare(
        ctx,
        elimination,
        block.levels,
        &mut native_specialization::Ordinary,
    )?;
    let names: Vec<String> = elimination
        .parameters
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    let restoration: std::collections::HashMap<String, Term> =
        elimination.restoration.iter().cloned().collect();
    let mut specialized = ctx.clone();
    if block.levels.is_empty() {
        specialized.add_mutual_inductives(&elimination.block)?;
    } else {
        specialized.add_universe_poly_inductives(block.levels, &elimination.block)?;
    }
    let instance: Vec<crate::term::Universe> = block
        .levels
        .iter()
        .cloned()
        .map(crate::term::Universe::Var)
        .collect();
    let mut result = Vec::new();
    for member in &elimination.block {
        let (derived, ty, body) = crate::derive_recursor_universe_poly(&specialized, &member.name)?;
        let motives = derived
            .len()
            .checked_sub(block.levels.len())
            .filter(|&m| derived[m..] == *block.levels)
            .ok_or_else(|| {
                KernelError::CertificationError(
                    "native nested recursor universe telescope does not end in the family's levels"
                        .into(),
                )
            })?;
        let levels = derived[..motives]
            .iter()
            .chain(block.levels)
            .cloned()
            .collect();
        let ty = restore_native_term(&ty, &restoration, &names, &instance)?;
        let body = restore_native_term(&body, &restoration, &names, &instance)?;
        let actual = crate::infer_type(ctx, &body)?;
        if !crate::defeq_for_test(ctx, &actual, &ty) {
            return Err(KernelError::CertificationError(
                "restored native nested recursor type differs from its checked implementation"
                    .into(),
            ));
        }
        let carrier = restoration.get(&member.name).cloned().unwrap_or_else(|| {
            apps(
                reference_at(&member.name, block.levels),
                names.iter().map(|p| v(p)).collect(),
            )
        });
        result.push(NativeRecursor {
            carrier,
            parameters: elimination.parameters.clone(),
            levels,
            ty,
            body,
        });
    }
    Ok(result)
}

/// Replace every specialized sibling and sibling constructor by its source image.
///
/// Images are open exactly over the shared `params`. A specialized reference is restored only
/// as the head of a spine carrying at least those parameters, which instantiate the image;
/// the remaining arguments are applied to it. A partially applied reference has no source
/// counterpart and is refused rather than restored to an ill-scoped image.
fn restore_native_term(
    term: &Term,
    restoration: &std::collections::HashMap<String, Term>,
    params: &[String],
    instance: &[crate::term::Universe],
) -> KernelResult<Term> {
    let go = |t: &Term| restore_native_term(t, restoration, params, instance);
    let (head, args) = spine(term);
    let specialized = match head {
        Term::Global(name) if instance.is_empty() => Some(name),
        Term::Const { name, levels } if restoration.contains_key(name) => {
            if levels.as_slice() != instance {
                return Err(KernelError::CertificationError(format!(
                    "native nested restoration met '{name}' at a universe instance other than \
                     the family's own"
                )));
            }
            Some(name)
        }
        _ => None,
    };
    if let Some(name) = specialized {
        if let Some(image) = restoration.get(name) {
            if args.len() < params.len() {
                return Err(KernelError::CertificationError(format!(
                    "native nested restoration met '{name}' applied to {} of its {} shared \
                     parameter(s)",
                    args.len(),
                    params.len()
                )));
            }
            let mut restored = args.into_iter().map(go).collect::<KernelResult<Vec<_>>>()?;
            let rest = restored.split_off(params.len());
            return Ok(apps(instantiate_parameters(image, params, &restored), rest));
        }
    }
    Ok(match term {
        Term::App(f, a) => app(go(f)?, go(a)?),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => pi(param, go(param_type)?, go(body_type)?),
        Term::Lambda {
            param,
            param_type,
            body,
        } => lam(param, go(param_type)?, go(body)?),
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: Box::new(go(ty)?),
            value: Box::new(go(value)?),
            body: Box::new(go(body)?),
        },
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(go(discriminant)?),
            motive: Box::new(go(motive)?),
            cases: cases.iter().map(go).collect::<KernelResult<Vec<_>>>()?,
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(go(body)?),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(name, body)| Ok((name.clone(), go(body)?)))
                .collect::<KernelResult<Vec<_>>>()?,
            index: *index,
        },
        Term::Global(_)
        | Term::Var(_)
        | Term::Const { .. }
        | Term::Sort(_)
        | Term::Lit(_)
        | Term::Hole => term.clone(),
        Term::Annotated { annotation, term } => Term::Annotated {
            annotation: annotation.clone(),
            term: Box::new(go(term)?),
        },
    })
}

#[path = "elimination_bounded.rs"]
mod bounded_elimination;
#[path = "bounded_native_restoration.rs"]
pub(crate) mod bounded_native_restoration;

#[path = "native_recursors_bounded.rs"]
pub(crate) mod native_recursors_bounded;

#[path = "native_specialization.rs"]
mod native_specialization;
