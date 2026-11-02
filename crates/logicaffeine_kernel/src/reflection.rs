//! The deep embedding's operations that are definitions: `syn_lift`, `syn_subst`, `syn_beta`,
//! `syn_size`, `syn_max_var`, `syn_quote` and `syn_diag`, each a structural recursion over `Syntax`
//! that the kernel checks like any definition, so their meaning is the definition's and none of
//! them is an assumption.
//!
//! The native implementations in `reduction.rs` stay as accelerators of the canonical definitions:
//! the main kernel runs one only when the context's definition of that name, its `Syntax` and `Univ`
//! families and the integer operations it computes with are exactly the ones registered here
//! ([`attested`]); otherwise δ unfolds the definition. The independent checker always computes by
//! the definition, so every use of an accelerator is checked against the definition it stands for.

use crate::context::Context;
use crate::interface::TermParser;
use crate::term::Term;
use std::sync::OnceLock;

/// The defined operations, each after every one its body names.
pub(crate) const DEFINED: [&str; 7] = [
    "syn_lift",
    "syn_subst",
    "syn_beta",
    "syn_size",
    "syn_max_var",
    "syn_quote",
    "syn_diag",
];

/// The cases of a `match` on `Syntax` in constructor order: `SVar k`, `SGlobal k`, `SSort u`,
/// `SApp a b`, `SLam a b`, `SPi a b`, `SLit n`, `SName s`.
const DEFINITIONS: [(&str, &str, &str); 7] = [
    (
        "syn_lift",
        "Int -> Int -> Syntax -> Syntax",
        "fix go => fun (amount : Int) (cutoff : Int) (t : Syntax) => \
         match t return (fun (t : Syntax) => Syntax) with \
           fun (k : Int) => match le cutoff k return (fun (b : Bool) => Syntax) with SVar (add k amount) | SVar k end \
         | fun (k : Int) => SGlobal k \
         | fun (u : Univ) => SSort u \
         | fun (a : Syntax) (b : Syntax) => SApp (go amount cutoff a) (go amount cutoff b) \
         | fun (a : Syntax) (b : Syntax) => SLam (go amount cutoff a) (go amount (add cutoff 1) b) \
         | fun (a : Syntax) (b : Syntax) => SPi (go amount cutoff a) (go amount (add cutoff 1) b) \
         | fun (n : Int) => SLit n \
         | fun (s : Text) => SName s \
         end",
    ),
    (
        "syn_subst",
        "Syntax -> Int -> Syntax -> Syntax",
        "fix go => fun (r : Syntax) (i : Int) (t : Syntax) => \
         match t return (fun (t : Syntax) => Syntax) with \
           fun (k : Int) => match le k i return (fun (b : Bool) => Syntax) with \
               match le i k return (fun (b : Bool) => Syntax) with r | SVar k end \
             | SVar k end \
         | fun (k : Int) => SGlobal k \
         | fun (u : Univ) => SSort u \
         | fun (a : Syntax) (b : Syntax) => SApp (go r i a) (go r i b) \
         | fun (a : Syntax) (b : Syntax) => SLam (go r i a) (go (syn_lift 1 0 r) (add i 1) b) \
         | fun (a : Syntax) (b : Syntax) => SPi (go r i a) (go (syn_lift 1 0 r) (add i 1) b) \
         | fun (n : Int) => SLit n \
         | fun (s : Text) => SName s \
         end",
    ),
    (
        "syn_beta",
        "Syntax -> Syntax -> Syntax",
        "fun (body : Syntax) (arg : Syntax) => \
         (fix go => fun (r : Syntax) (i : Int) (t : Syntax) => \
          match t return (fun (t : Syntax) => Syntax) with \
            fun (k : Int) => match le k i return (fun (b : Bool) => Syntax) with \
                match le i k return (fun (b : Bool) => Syntax) with r | SVar k end \
              | SVar (sub k 1) end \
          | fun (k : Int) => SGlobal k \
          | fun (u : Univ) => SSort u \
          | fun (a : Syntax) (b : Syntax) => SApp (go r i a) (go r i b) \
          | fun (a : Syntax) (b : Syntax) => SLam (go r i a) (go (syn_lift 1 0 r) (add i 1) b) \
          | fun (a : Syntax) (b : Syntax) => SPi (go r i a) (go (syn_lift 1 0 r) (add i 1) b) \
          | fun (n : Int) => SLit n \
          | fun (s : Text) => SName s \
          end) arg 0 body",
    ),
    (
        "syn_size",
        "Syntax -> Int",
        "fix go => fun (t : Syntax) => \
         match t return (fun (t : Syntax) => Int) with \
           fun (k : Int) => 1 \
         | fun (k : Int) => 1 \
         | fun (u : Univ) => 1 \
         | fun (a : Syntax) (b : Syntax) => add 1 (add (go a) (go b)) \
         | fun (a : Syntax) (b : Syntax) => add 1 (add (go a) (go b)) \
         | fun (a : Syntax) (b : Syntax) => add 1 (add (go a) (go b)) \
         | fun (n : Int) => 1 \
         | fun (s : Text) => 1 \
         end",
    ),
    (
        "syn_max_var",
        "Syntax -> Int",
        "fun (t : Syntax) => \
         (fix go => fun (depth : Int) (t : Syntax) => \
          match t return (fun (t : Syntax) => Int) with \
            fun (k : Int) => match le depth k return (fun (b : Bool) => Int) with sub k depth | -1 end \
          | fun (k : Int) => -1 \
          | fun (u : Univ) => -1 \
          | fun (a : Syntax) (b : Syntax) => \
              (fun (x : Int) (y : Int) => match le x y return (fun (c : Bool) => Int) with y | x end) (go depth a) (go depth b) \
          | fun (a : Syntax) (b : Syntax) => \
              (fun (x : Int) (y : Int) => match le x y return (fun (c : Bool) => Int) with y | x end) (go depth a) (go (add depth 1) b) \
          | fun (a : Syntax) (b : Syntax) => \
              (fun (x : Int) (y : Int) => match le x y return (fun (c : Bool) => Int) with y | x end) (go depth a) (go (add depth 1) b) \
          | fun (n : Int) => -1 \
          | fun (s : Text) => -1 \
          end) 0 t",
    ),
    (
        "syn_quote",
        "Syntax -> Syntax",
        "fix go => fun (t : Syntax) => \
         match t return (fun (t : Syntax) => Syntax) with \
           fun (k : Int) => SApp (SName \"SVar\") (SLit k) \
         | fun (k : Int) => SApp (SName \"SGlobal\") (SLit k) \
         | fun (u : Univ) => SApp (SName \"SSort\") \
             (match u return (fun (u : Univ) => Syntax) with SName \"UProp\" | fun (n : Int) => SApp (SName \"UType\") (SLit n) end) \
         | fun (a : Syntax) (b : Syntax) => SApp (SApp (SName \"SApp\") (go a)) (go b) \
         | fun (a : Syntax) (b : Syntax) => SApp (SApp (SName \"SLam\") (go a)) (go b) \
         | fun (a : Syntax) (b : Syntax) => SApp (SApp (SName \"SPi\") (go a)) (go b) \
         | fun (n : Int) => SApp (SName \"SLit\") (SLit n) \
         | fun (s : Text) => SName s \
         end",
    ),
    ("syn_diag", "Syntax -> Syntax", "fun (x : Syntax) => syn_subst (syn_quote x) 0 x"),
];

/// Each defined operation's `(name, type, body)`, parsed once.
pub(crate) fn definitions() -> &'static [(String, Term, Term)] {
    static PARSED: OnceLock<Vec<(String, Term, Term)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        DEFINITIONS
            .iter()
            .map(|(name, ty, body)| {
                let parse = |text: &str| {
                    TermParser::parse(text)
                        .unwrap_or_else(|e| panic!("`{name}`'s canonical form parses: {e:?}"))
                };
                (name.to_string(), parse(ty), parse(body))
            })
            .collect()
    })
}

/// Registers every defined operation, checked. The `Syntax`, `Univ` and `Int` families and the
/// integer operations are registered before.
pub(crate) fn register(ctx: &mut Context) {
    for (name, ty, body) in definitions() {
        ctx.add_definition(name.clone(), ty.clone(), body.clone());
    }
}

/// Whether `name`'s native implementation may stand in for its definition in `ctx`: the context
/// holds exactly the canonical definition of it and of every defined operation it names, the
/// canonical `Syntax` and `Univ`, and the attested integer operations the definitions compute with.
pub(crate) fn attested(ctx: &Context, name: &str) -> bool {
    let Some(slot) = DEFINED.iter().position(|n| *n == name) else {
        return false;
    };
    ctx.cached_reflection_attestation(slot, || {
        let canonical = |n: &str| {
            definitions()
                .iter()
                .find(|(m, ..)| m == n)
                .is_some_and(|(_, ty, body)| {
                    ctx.poly_declaration(n).is_none()
                        && !ctx.is_constructor(n)
                        && !ctx.is_inductive(n)
                        && ctx.get_definition_type(n) == Some(ty)
                        && ctx.get_definition_body(n) == Some(body)
                })
        };
        let named: &[&str] = match name {
            "syn_subst" | "syn_beta" => &["syn_lift"],
            "syn_diag" => &["syn_lift", "syn_subst", "syn_quote"],
            _ => &[],
        };
        canonical(name)
            && named.iter().all(|n| canonical(n))
            && canonical_embedding(ctx)
            && ["add", "sub", "le"]
                .iter()
                .all(|op| crate::int_inductive::accelerates_integer_alias(ctx, op))
    })
}

/// `Syntax` and `Univ` with exactly their registered constructors, in order, at their registered
/// types: the shapes the native implementations read.
fn canonical_embedding(ctx: &Context) -> bool {
    let g = |n: &str| Term::Global(n.to_string());
    let arrow = |a: Term, b: Term| Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    let syntax = || g("Syntax");
    let expected_syntax = [
        ("SVar", arrow(g("Int"), syntax())),
        ("SGlobal", arrow(g("Int"), syntax())),
        ("SSort", arrow(g("Univ"), syntax())),
        ("SApp", arrow(syntax(), arrow(syntax(), syntax()))),
        ("SLam", arrow(syntax(), arrow(syntax(), syntax()))),
        ("SPi", arrow(syntax(), arrow(syntax(), syntax()))),
        ("SLit", arrow(g("Int"), syntax())),
        ("SName", arrow(g("Text"), syntax())),
    ];
    let expected_univ = [("UProp", g("Univ")), ("UType", arrow(g("Int"), g("Univ")))];
    let family = |name: &str, expected: &[(&str, Term)]| {
        ctx.get_global(name) == Some(&Term::Sort(crate::term::Universe::Type(0)))
            && ctx.poly_declaration(name).is_none()
            && ctx.inductive_num_params(name) == 0
            && {
                let constructors = ctx.get_constructors(name);
                constructors.len() == expected.len()
                    && constructors
                        .iter()
                        .zip(expected)
                        .all(|((c, ty), (e, t))| c == e && *ty == t)
            }
    };
    family("Syntax", &expected_syntax) && family("Univ", &expected_univ)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/reflection/tests.rs"]
mod tests;
