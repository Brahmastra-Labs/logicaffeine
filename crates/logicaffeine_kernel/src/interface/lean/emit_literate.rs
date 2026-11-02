//! The literate readable view: LOGOS's English forms, written only where they state a declaration
//! EXACTLY.
//!
//! Two forms can carry an imported declaration: `A Color is either red or green or blue.` for an
//! inductive, and `## To name (x: T) -> R: Yield e.` for a first-order definition. Each function
//! here either spells a candidate or says why the form cannot express the declaration; a candidate
//! is only a proposal — the emitter keeps it solely when reparsing it registers the identical
//! kernel declaration, and otherwise writes the vernacular and records the omission.

use super::terms::{is_plain_identifier, mentions_var};
use crate::{Term, Universe};

/// An inductive as the emitter writes it: names as the document spells them, parameter names
/// shared by the arity and every constructor.
pub(super) struct WrittenInductive<'a> {
    /// The absolute key.
    pub name: &'a str,
    pub params: &'a [(String, Term)],
    /// The arity after the parameters.
    pub sort: &'a Term,
    /// Constructor names relative to the inductive's namespace, and types after the parameters.
    pub constructors: &'a [(String, Term)],
    /// The inductive's own key as a constructor's result head is written.
    pub result_heads: &'a [String],
}

/// Words the literate reader treats as syntax wherever a name could stand.
const LITERATE_SYNTAX: &[&str] = &[
    "of", "and", "or", "is", "either", "with", "a", "an", "A", "An", "the", "Name", "Variable",
    "Apply", "there", "When", "Yield", "Consider", "given", "yields", "equals", "implies", "be",
    "forall", "Prop", "Type", "Let", "To",
];

fn literate_word(word: &str) -> bool {
    is_plain_identifier(word) && !LITERATE_SYNTAX.contains(&word)
}

/// A type the literate type grammar reads back as exactly `ty`: a name, a parameter, `Prop`,
/// `Type`, or `F of X` for a one-argument application.
fn literate_type(ty: &Term, params: &[String]) -> Option<String> {
    match ty {
        Term::Sort(Universe::Prop) => Some("Prop".into()),
        Term::Sort(Universe::Type(0)) => Some("Type".into()),
        Term::Var(v) if params.contains(v) && literate_word(v) => Some(v.clone()),
        Term::Global(g) if literate_word(g) => Some(g.clone()),
        Term::App(f, arg) => match &**f {
            Term::Global(g) if literate_word(g) => {
                Some(format!("{g} of {}", literate_type(arg, params)?))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `A Color is either red or green or blue.` — or why the literate inductive form cannot state it.
pub(super) fn inductive(ind: &WrittenInductive<'_>) -> Result<String, String> {
    if !matches!(ind.sort, Term::Sort(Universe::Type(0))) {
        return Err(
            "the literate inductive form declares a type in `Type`, and this one is not \
                    exactly `Type` after its parameters"
                .into(),
        );
    }
    let param_names: Vec<String> = ind.params.iter().map(|(n, _)| n.clone()).collect();
    let mut params = Vec::with_capacity(ind.params.len());
    for (name, ty) in ind.params {
        if !literate_word(name) {
            return Err(format!("the parameter name `{name}` is literate syntax"));
        }
        let spelled = literate_type(ty, &param_names)
            .ok_or_else(|| format!("the parameter type of `{name}` has no literate spelling"))?;
        params.push(format!("{name}: {spelled}"));
    }
    if ind.constructors.is_empty() {
        return Err("the literate inductive form requires at least one constructor".into());
    }
    let mut variants = Vec::with_capacity(ind.constructors.len());
    for (ctor, ty) in ind.constructors {
        let first = ctor.split('.').next().unwrap_or(ctor);
        if matches!(first, "a" | "an" | "A" | "An") || !ctor.split('.').all(literate_word) {
            return Err(format!(
                "the constructor name `{ctor}` reads as literate syntax"
            ));
        }
        let mut fields = Vec::new();
        let mut rest = ty;
        while let Term::Pi {
            param,
            param_type,
            body_type,
        } = rest
        {
            if mentions_var(body_type, param) {
                return Err(format!("the constructor `{ctor}` has a dependent field"));
            }
            let spelled = literate_type(param_type, &param_names)
                .ok_or_else(|| format!("a field type of `{ctor}` has no literate spelling"))?;
            fields.push(spelled);
            rest = body_type;
        }
        if !is_applied_to_params(rest, ind.result_heads, &param_names) {
            return Err(format!(
                "the constructor `{ctor}` does not build the type at its own parameters"
            ));
        }
        if fields.is_empty() {
            variants.push(ctor.clone());
        } else {
            let named: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, t)| format!("{}: {t}", field_name(i, fields.len())))
                .collect();
            variants.push(format!("a {ctor} with {}", named.join(" and ")));
        }
    }
    let article = if ind.name.starts_with(['A', 'E', 'I', 'O', 'U']) {
        "An"
    } else {
        "A"
    };
    let of = if params.is_empty() {
        String::new()
    } else {
        format!(" of ({})", params.join(", "))
    };
    Ok(format!(
        "{article} {}{of} is either {}.",
        ind.name,
        variants.join(" or ")
    ))
}

/// A field's label. The literate reader discards it, so it only has to read as a name.
fn field_name(i: usize, total: usize) -> String {
    if total == 1 {
        "value".into()
    } else {
        format!("field{}", i + 1)
    }
}

fn is_applied_to_params(ty: &Term, heads: &[String], params: &[String]) -> bool {
    let mut args = Vec::new();
    let mut head = ty;
    while let Term::App(f, a) = head {
        args.push(&**a);
        head = f;
    }
    args.reverse();
    matches!(head, Term::Global(g) if heads.contains(g))
        && args.len() == params.len()
        && args
            .iter()
            .zip(params)
            .all(|(a, p)| matches!(a, Term::Var(v) if v == p))
}

/// `## To name (x: T) and (y: U) -> R: Yield e.` — or why the literate definition form cannot
/// state it.
pub(super) fn definition(name: &str, ty: &Term, body: &Term) -> Result<String, String> {
    if name == "be" || !name.split('.').all(literate_word) {
        return Err(format!("the name `{name}` reads as literate syntax"));
    }
    let mut params: Vec<(String, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let (mut t, mut b) = (ty, body);
    loop {
        match (t, b) {
            (
                Term::Pi {
                    param: tp,
                    param_type: tt,
                    body_type: tb,
                },
                Term::Lambda {
                    param: bp,
                    param_type: bt,
                    body: bb,
                },
            ) => {
                if mentions_var(tb, tp) {
                    return Err(
                        "the type is dependent, and `## To` states a non-dependent one".into(),
                    );
                }
                if !super::terms::alpha_eq(tt, bt) {
                    return Err("a parameter's type differs between the type and the body".into());
                }
                if !literate_word(bp) || names.contains(bp) {
                    return Err(format!(
                        "the parameter name `{bp}` cannot be a literate parameter"
                    ));
                }
                let spelled = literate_type(bt, &[])
                    .ok_or_else(|| format!("the type of `{bp}` has no literate spelling"))?;
                params.push((bp.clone(), spelled));
                names.push(bp.clone());
                t = tb;
                b = bb;
            }
            (Term::Pi { .. }, _) => {
                return Err("the body is not a λ over every parameter the type names".into())
            }
            _ => break,
        }
    }
    let ret = literate_type(t, &[])
        .ok_or_else(|| "the result type has no literate spelling".to_string())?;
    let value = literate_term(b, &names, name)
        .ok_or_else(|| "the body has no literate spelling".to_string())?;
    let spelled_params: Vec<String> = params.iter().map(|(n, t)| format!("({n}: {t})")).collect();
    let head = if spelled_params.is_empty() {
        format!("## To {name} -> {ret}:")
    } else {
        format!("## To {name} {} -> {ret}:", spelled_params.join(" and "))
    };
    Ok(format!("{head}\n    Yield {value}."))
}

/// A term the literate term grammar reads back as exactly `term`: parameters, names and
/// applications of them.
fn literate_term(term: &Term, bound: &[String], own: &str) -> Option<String> {
    match term {
        Term::Var(v) if bound.contains(v) => Some(v.clone()),
        Term::Global(g) if literate_word(g) && g != own && !bound.contains(g) => Some(g.clone()),
        Term::Sort(Universe::Prop) => Some("Prop".into()),
        Term::Sort(Universe::Type(0)) => Some("Type".into()),
        Term::App(f, a) => {
            let f = literate_term(f, bound, own)?;
            let a = literate_term(a, bound, own)?;
            let a = if a.contains(' ') { format!("({a})") } else { a };
            Some(format!("{f} {a}"))
        }
        _ => None,
    }
}
