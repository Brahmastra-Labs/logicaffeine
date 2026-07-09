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
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}

/// The role of one value argument of a container constructor, relative to the container's
/// element parameter `A`.
enum ArgKind {
    /// The bare element `A` — carries a value of the (nested) element type.
    Elem,
    /// A RECURSIVE container occurrence `Container A` — the spine.
    Recursive,
    /// Anything else (with `A` substituted by the element) — carried through unchanged.
    Other(Term),
}

/// Peel a term's leading `Π`s into `(name, type)` pairs plus the residual.
fn peel_pis(t: &Term) -> (Vec<(String, Term)>, Term) {
    let mut params = Vec::new();
    let mut cur = t.clone();
    while let Term::Pi { param, param_type, body_type } = cur {
        params.push((param, *param_type));
        cur = *body_type;
    }
    (params, cur)
}

fn is_global(t: &Term, name: &str) -> bool {
    matches!(t, Term::Global(n) if n == name)
}

/// Analyse a container constructor `Π(A). vt₁ → … → vtₖ → Container A`: return the element
/// parameter name `A` and the KIND of each value argument.
fn analyze_container_ctor(ctor_ty: &Term, container: &str) -> KernelResult<(String, Vec<ArgKind>)> {
    let (params, _residual) = peel_pis(ctor_ty);
    if params.is_empty() {
        return Err(KernelError::CertificationError(format!(
            "nested-compile: container '{container}' constructor has no element parameter"
        )));
    }
    let a = params[0].0.clone();
    let mut kinds = Vec::new();
    for (_, ty) in &params[1..] {
        let kind = match ty {
            Term::Var(n) if *n == a => ArgKind::Elem,
            Term::App(f, x) if is_global(f, container) && matches!(x.as_ref(), Term::Var(n) if *n == a) => {
                ArgKind::Recursive
            }
            _ => ArgKind::Other(ty.clone()),
        };
        kinds.push(kind);
    }
    Ok((a, kinds))
}

/// True if `name` occurs anywhere in `ty`.
fn occurs_anywhere(ty: &Term, name: &str) -> bool {
    match ty {
        Term::Global(n) => n == name,
        Term::App(f, x) => occurs_anywhere(f, name) || occurs_anywhere(x, name),
        Term::Pi { param_type, body_type, .. } => {
            occurs_anywhere(param_type, name) || occurs_anywhere(body_type, name)
        }
        Term::Lambda { param_type, body, .. } => {
            occurs_anywhere(param_type, name) || occurs_anywhere(body, name)
        }
        _ => false,
    }
}

/// True if `ty` is a constructor argument that must be SPECIALIZED: its head is a registered
/// container inductive (≠ the inductive being defined) and `name` occurs somewhere inside it
/// (`TList name`, `TList (TList name)`). A bare `name`, a `Nat`, a positive `Nat → name`, or a
/// container not mentioning `name` (`TList Nat`) is NOT nested — it passes through unchanged.
fn is_nested_container_type(ctx: &Context, decl: &NestedDecl, ty: &Term) -> bool {
    let mut head = ty;
    while let Term::App(f, _) = head {
        head = f;
    }
    matches!(head, Term::Global(c) if c != &decl.name && ctx.is_inductive(c))
        && occurs_anywhere(ty, &decl.name)
}

/// The result of specializing one nested type: its specialized REPRESENTATIVE (either the
/// inductive itself for the base leaf, or a fresh sibling), the GENERIC type it stands for,
/// and — for a non-base node — the names of its conversion isos (applied on element fields of
/// any type that nests it).
struct Spec {
    repr: String,
    generic: Term,
    to: Option<String>,
    from: Option<String>,
}

/// One specialized sibling to emit: a mutual inductive mirroring `container` with its element
/// fixed to `elem_repr`, plus the data its conversion isos need (the element's generic form and
/// the element's own isos, which the sibling's isos delegate to).
struct SibNode {
    container: String,
    repr: String,
    elem_repr: String,
    elem_generic: Term,
    inner_to: Option<String>,
    inner_from: Option<String>,
    ctors: Vec<(String, Vec<ArgKind>)>,
    to_name: String,
    from_name: String,
}

/// Recursively specialize a nested type, accumulating the siblings to emit (in POST-ORDER, so
/// an inner sibling is registered before the outer sibling whose isos reference it). The base
/// case is the inductive itself; `Container inner` specializes `inner` first, then mirrors
/// `Container` with element `= inner`'s representative.
fn specialize(
    ctx: &Context,
    decl: &NestedDecl,
    ty: &Term,
    nodes: &mut Vec<SibNode>,
    seen: &mut std::collections::HashSet<String>,
) -> KernelResult<Spec> {
    if is_global(ty, &decl.name) {
        return Ok(Spec { repr: decl.name.clone(), generic: g(&decl.name), to: None, from: None });
    }
    if let Term::App(f, inner_ty) = ty {
        if let Term::Global(c) = f.as_ref() {
            if c != &decl.name && ctx.is_inductive(c) {
                let inner = specialize(ctx, decl, inner_ty, nodes, seen)?;
                let repr = format!("{}${}", inner.repr, c);
                let generic = app(g(c), inner.generic.clone());
                let to_name = format!("{repr}_to_{c}");
                let from_name = format!("{repr}_from_{c}");
                if seen.insert(repr.clone()) {
                    // Only PURE unary containers (every field the element or a recursive
                    // occurrence — `List`/`TList`) specialize soundly; anything else is refused
                    // fail-closed rather than emitting an inconsistent block.
                    let mut ctors = Vec::new();
                    for (orig_ctor, orig_ty) in ctx.get_constructors(c) {
                        let (_a, kinds) = analyze_container_ctor(orig_ty, c)?;
                        if kinds.iter().any(|k| matches!(k, ArgKind::Other(_))) {
                            return Err(KernelError::CertificationError(format!(
                                "nested-compile: container '{c}' constructor '{orig_ctor}' has an \
                                 argument that is neither the element nor a recursive occurrence \
                                 — specializing it is not supported (only pure containers like \
                                 `List`/`TList`)"
                            )));
                        }
                        ctors.push((orig_ctor.to_string(), kinds));
                    }
                    nodes.push(SibNode {
                        container: c.clone(),
                        repr: repr.clone(),
                        elem_repr: inner.repr.clone(),
                        elem_generic: inner.generic.clone(),
                        inner_to: inner.to.clone(),
                        inner_from: inner.from.clone(),
                        ctors,
                        to_name: to_name.clone(),
                        from_name: from_name.clone(),
                    });
                }
                return Ok(Spec { repr, generic, to: Some(to_name), from: Some(from_name) });
            }
        }
    }
    Err(KernelError::CertificationError(format!(
        "nested-compile: '{}' occurs in argument type {ty} in a position that is not a nesting \
         inside a registered unary container (only `Container …` nestings are specialized)",
        decl.name
    )))
}

/// Compile a nested inductive into a mutual block plus conversion isos. Untrusted: every
/// artifact is checked by the caller. Handles arbitrary nesting DEPTH (`TList (TList RTree)`)
/// by recursive specialization — each container level becomes its own mutual sibling.
pub fn compile_nested(ctx: &Context, decl: &NestedDecl) -> KernelResult<Compiled> {
    // Universe safety: the specialized siblings are emitted at `Type 0` (see below), so the
    // nested inductive must itself be `Type 0`. Refusing anything else keeps the emitted
    // block universe-consistent rather than silently registering a higher-universe field in
    // a `Type 0` inductive.
    if decl.sort != Term::Sort(crate::term::Universe::Type(0)) {
        return Err(KernelError::CertificationError(format!(
            "nested-compile: '{}' must be a `Type 0` inductive (specialized siblings are \
             `Type 0`); higher-universe nesting is not supported",
            decl.name
        )));
    }

    // 1. The inductive's own constructors, with each NESTED argument (`TList name`,
    //    `TList (TList name)`, …) replaced by its outermost specialized sibling — recording
    //    every sibling that must be emitted (in post-order, inner-first).
    let mut nodes: Vec<SibNode> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut own_ctors = Vec::new();
    for (cname, cty) in &decl.constructors {
        let (params, residual) = peel_pis(cty);
        let mut rebuilt = residual;
        for (pname, pty) in params.into_iter().rev() {
            let pty2 = if is_nested_container_type(ctx, decl, &pty) {
                g(&specialize(ctx, decl, &pty, &mut nodes, &mut seen)?.repr)
            } else {
                pty
            };
            rebuilt = pi(&pname, pty2, rebuilt);
        }
        own_ctors.push((cname.clone(), rebuilt));
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
        sort: decl.sort.clone(),
        num_params: 0,
        constructors: own_ctors,
    }];
    let mut siblings = Vec::new();
    for node in &nodes {
        siblings.push(node.repr.clone());
        let mut sib_ctors = Vec::new();
        for (orig, kinds) in &node.ctors {
            let mut sty = g(&node.repr);
            for kind in kinds.iter().rev() {
                let arg_ty = match kind {
                    ArgKind::Elem => g(&node.elem_repr),
                    ArgKind::Recursive => g(&node.repr),
                    ArgKind::Other(t) => t.clone(),
                };
                sty = arrow(arg_ty, sty);
            }
            sib_ctors.push((format!("{}_{}", node.repr, orig), sty));
        }
        block.push(MutualInductive {
            name: node.repr.clone(),
            sort: Term::Sort(crate::term::Universe::Type(0)),
            num_params: 0,
            constructors: sib_ctors,
        });
    }

    // 3. The conversion isos, one pair per node — emitted inner-first so the outer iso's
    //    element-field delegation to the inner iso resolves when it is kernel-checked.
    let mut isos = Vec::new();
    let mut iso_names = Vec::new();
    for node in &nodes {
        let to_ty = arrow(g(&node.repr), app(g(&node.container), node.elem_generic.clone()));
        let from_ty = arrow(app(g(&node.container), node.elem_generic.clone()), g(&node.repr));
        isos.push((node.to_name.clone(), to_ty, build_to_generic(node)));
        isos.push((node.from_name.clone(), from_ty, build_from_generic(node)));
        iso_names.push(IsoNames {
            container: node.container.clone(),
            sibling: node.repr.clone(),
            to_generic: node.to_name.clone(),
            from_generic: node.from_name.clone(),
        });
    }

    Ok(Compiled { block, isos, iso_names, siblings })
}

/// `to_generic : sibling → Container elem_generic` — rebuild each sibling constructor as the
/// corresponding container constructor at the element's GENERIC type, recursing on spine args
/// and delegating element fields to the INNER iso (identity at the base leaf).
fn build_to_generic(node: &SibNode) -> Term {
    let mut cases = Vec::new();
    for (orig_ctor, kinds) in &node.ctors {
        // λa₀ … a_{k-1}. Container_ctor elem_generic (convert a₀) … (convert a_{k-1})
        let mut body = apps(g(orig_ctor), vec![node.elem_generic.clone()]);
        for (i, kind) in kinds.iter().enumerate() {
            body = app(body, convert_to(kind, v(&format!("a{i}")), "rec", &node.inner_to));
        }
        for (i, kind) in kinds.iter().enumerate().rev() {
            let ty = match kind {
                ArgKind::Elem => g(&node.elem_repr),
                ArgKind::Recursive => g(&node.repr),
                ArgKind::Other(t) => t.clone(),
            };
            body = lam(&format!("a{i}"), ty, body);
        }
        cases.push(body);
    }
    Term::Fix {
        name: "rec".to_string(),
        body: Box::new(lam(
            "x",
            g(&node.repr),
            Term::Match {
                discriminant: Box::new(v("x")),
                motive: Box::new(lam("_", g(&node.repr), app(g(&node.container), node.elem_generic.clone()))),
                cases,
            },
        )),
    }
}

/// `from_generic : Container elem_generic → sibling` — rebuild each container constructor
/// (matched at the element's generic type) as the corresponding sibling constructor, recursing
/// on spine and delegating element fields to the inner iso's `from` (identity at the base leaf).
fn build_from_generic(node: &SibNode) -> Term {
    let mut cases = Vec::new();
    for (orig_ctor, kinds) in &node.ctors {
        let mut body = g(&format!("{}_{}", node.repr, orig_ctor));
        for (i, kind) in kinds.iter().enumerate() {
            body = app(body, convert_from(kind, v(&format!("a{i}")), "rec", &node.inner_from));
        }
        for (i, kind) in kinds.iter().enumerate().rev() {
            // The binder types are the CONTAINER constructor's value args at the generic element.
            let ty = match kind {
                ArgKind::Elem => node.elem_generic.clone(),
                ArgKind::Recursive => app(g(&node.container), node.elem_generic.clone()),
                ArgKind::Other(t) => t.clone(),
            };
            body = lam(&format!("a{i}"), ty, body);
        }
        cases.push(body);
    }
    Term::Fix {
        name: "rec".to_string(),
        body: Box::new(lam(
            "x",
            app(g(&node.container), node.elem_generic.clone()),
            Term::Match {
                discriminant: Box::new(v("x")),
                motive: Box::new(lam("_", app(g(&node.container), node.elem_generic.clone()), g(&node.repr))),
                cases,
            },
        )),
    }
}

/// Convert one argument in `to_generic`: recurse (`rec a`) on a spine occurrence, delegate an
/// element field to the inner `to` iso (identity at the base leaf), pass others unchanged.
fn convert_to(kind: &ArgKind, a: Term, rec: &str, inner_to: &Option<String>) -> Term {
    match kind {
        ArgKind::Recursive => app(v(rec), a),
        ArgKind::Elem => match inner_to {
            Some(iso) => app(g(iso), a),
            None => a,
        },
        ArgKind::Other(_) => a,
    }
}

/// Convert one argument in `from_generic`: recurse on a spine occurrence, delegate an element
/// field to the inner `from` iso (identity at the base leaf), pass others unchanged.
fn convert_from(kind: &ArgKind, a: Term, rec: &str, inner_from: &Option<String>) -> Term {
    match kind {
        ArgKind::Recursive => app(v(rec), a),
        ArgKind::Elem => match inner_from {
            Some(iso) => app(g(iso), a),
            None => a,
        },
        ArgKind::Other(_) => a,
    }
}
