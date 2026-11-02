//! Pattern metadata for the raw term surface, consumed before kernel checking.
//!
//! Names that might be binders stay scoped in the clause body until the environment
//! distinguishes constructors from pattern variables. Kernel Match terms remain positional.

use super::match_compile::{compile_clauses, pattern_binder_types, Clause, Pattern};
use crate::type_checker::{occurs_free, substitute};
use crate::{infer_type, Context, KernelError, KernelResult, Literal, Term};
use std::collections::HashSet;

pub(crate) const CASE: &str = "⟨match-case⟩";
const NAME: &str = "⟨pattern-name⟩";

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
pub(crate) fn case(pattern: Term, body: Term) -> Term {
    app(app(Term::Global(CASE.into()), pattern), body)
}
pub(crate) fn as_case(term: &Term) -> Option<(&Term, &Term)> {
    let Term::App(f, body) = term else {
        return None;
    };
    let Term::App(head, pattern) = f.as_ref() else {
        return None;
    };
    matches!(head.as_ref(), Term::Global(n) if n == CASE).then_some((pattern, body))
}

/// The parser's possible pattern variables, unique and in source order.
pub(crate) fn candidates(term: &Term, out: &mut Vec<String>) {
    match term {
        Term::Var(n) => {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
        Term::App(f, a) => {
            candidates(f, out);
            candidates(a, out);
        }
        Term::Annotated { term, .. } => candidates(term, out),
        _ => {}
    }
}

fn as_name(term: &Term) -> Option<(&str, &Term)> {
    let Term::App(f, resolved) = term else {
        return None;
    };
    let Term::App(head, text) = f.as_ref() else {
        return None;
    };
    let Term::Lit(Literal::Text(written)) = text.as_ref() else {
        return None;
    };
    matches!(head.as_ref(), Term::Global(n) if n == NAME).then_some((written, resolved))
}

/// Resolve possible constructor names outside the clause's local binder scope.
/// Retain the spelling too: a non-constructor is a local pattern variable even
/// when an unrelated declaration has the same short name.
pub(crate) fn resolve_pattern<E>(
    term: &Term,
    resolve: &mut impl FnMut(&Term) -> Result<Term, E>,
) -> Result<Term, E> {
    if as_name(term).is_some() {
        return Ok(term.clone());
    }
    Ok(match term {
        Term::Var(written) => {
            let global = resolve(&Term::Global(written.clone()))?;
            app(
                app(
                    Term::Global(NAME.into()),
                    Term::Lit(Literal::Text(written.clone())),
                ),
                global,
            )
        }
        Term::Global(_) => resolve(term)?,
        Term::App(f, a) => app(resolve_pattern(f, resolve)?, resolve_pattern(a, resolve)?),
        _ => term.clone(),
    })
}

fn invalid(message: impl Into<String>) -> KernelError {
    KernelError::CertificationError(message.into())
}

fn decode_pattern(
    ctx: &Context,
    term: &Term,
    constructors: &mut Vec<(String, Term)>,
) -> KernelResult<Pattern> {
    let ambiguous = if let Some((written, resolved)) = as_name(term) {
        Some((written, resolved.clone()))
    } else if let Term::Var(written) = term {
        Some((written.as_str(), Term::Global(written.clone())))
    } else {
        None
    };
    if let Some((written, resolved)) = ambiguous {
        if let Term::Global(n) = &resolved {
            if ctx.is_constructor(n) {
                constructors.push((written.into(), resolved.clone()));
                return Ok(Pattern::Ctor {
                    name: n.clone(),
                    args: Vec::new(),
                });
            }
        }
        return Ok(Pattern::Var(written.into()));
    }
    match term {
        Term::Hole => Ok(Pattern::Wildcard),
        Term::Lit(Literal::Nat(n)) => {
            Ok(Pattern::Numeral(n.to_string().parse::<u64>().map_err(
                |_| invalid("natural-number pattern is too large"),
            )?))
        }
        Term::Global(n) => Ok(Pattern::Ctor {
            name: n.clone(),
            args: Vec::new(),
        }),
        Term::App(..) => {
            let mut args = Vec::new();
            let mut head = term;
            while let Term::App(f, a) = head {
                args.push(a.as_ref());
                head = f;
            }
            let Term::Global(n) = head else {
                return Err(invalid("a pattern application needs a constructor name"));
            };
            args.reverse();
            Ok(Pattern::Ctor {
                name: n.clone(),
                args: args
                    .into_iter()
                    .map(|a| decode_pattern(ctx, a, constructors))
                    .collect::<KernelResult<_>>()?,
            })
        }
        _ => Err(invalid("invalid match pattern")),
    }
}

fn decode_clause(ctx: &Context, term: &Term) -> KernelResult<Clause> {
    let (raw, wrapped) =
        as_case(term).ok_or_else(|| invalid("mixed named and positional match cases"))?;
    let mut constructors = Vec::new();
    let pattern = decode_pattern(ctx, raw, &mut constructors)?;
    let mut seen = HashSet::new();
    for name in pattern.binders() {
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("a pattern binds '{name}' more than once")));
        }
    }
    // Exactly the parser's candidate telescope is removed, never the body's own lambdas.
    let mut names = pattern.binders();
    names.extend(constructors.iter().map(|(n, _)| n.clone()));
    names.sort();
    names.dedup();
    let mut body = wrapped;
    for _ in &names {
        let Term::Lambda { body: inner, .. } = body else {
            return Err(invalid("match pattern binder metadata is incomplete"));
        };
        body = inner;
    }
    let mut body = body.clone();
    for (written, global) in constructors {
        body = substitute(&body, &written, &global);
    }
    Ok(Clause {
        patterns: vec![pattern],
        body,
    })
}

/// Name ignored fields before computing later dependent field types. These names
/// cannot be captured because they avoid every name spelled by the entire match.
fn name_ignored_fields(pattern: &mut Pattern, taken: &mut HashSet<String>, next: &mut usize) {
    if let Pattern::Ctor { args, .. } = pattern {
        for arg in args {
            if matches!(arg, Pattern::Wildcard) {
                loop {
                    let candidate = format!("#rawfield{}", *next);
                    *next += 1;
                    if taken.insert(candidate.clone()) {
                        *arg = Pattern::Var(candidate);
                        break;
                    }
                }
            } else {
                name_ignored_fields(arg, taken, next);
            }
        }
    }
}

pub(crate) fn compile(
    ctx: &Context,
    disc: Term,
    motive: &Term,
    cases: &[Term],
    expected: Option<&Term>,
) -> KernelResult<Term> {
    let mut clauses = cases
        .iter()
        .map(|c| decode_clause(ctx, c))
        .collect::<KernelResult<Vec<_>>>()?;
    let mut taken = HashSet::new();
    super::spelled_names(&disc, &mut taken);
    super::spelled_names(motive, &mut taken);
    if let Some(expected) = expected {
        super::spelled_names(expected, &mut taken);
    }
    for clause in &clauses {
        super::spelled_names(&clause.body, &mut taken);
        taken.extend(clause.patterns[0].binders());
    }
    let mut next = 0;
    for clause in &mut clauses {
        name_ignored_fields(&mut clause.patterns[0], &mut taken, &mut next);
    }
    let disc_ty = crate::reduction::normalize_elab(ctx, &infer_type(ctx, &disc)?);
    // Put each clause's real field types in scope before descending into nested matches.
    let clauses = clauses
        .into_iter()
        .map(|clause| {
            let mut scoped = ctx.clone();
            for (n, ty) in pattern_binder_types(ctx, &disc_ty, &clause.patterns[0])? {
                scoped = scoped.extend(&n, ty);
            }
            let body = crate::elaborate::fill_match_motives(&scoped, &clause.body, expected)?;
            Ok(Clause {
                patterns: clause.patterns,
                body,
            })
        })
        .collect::<KernelResult<Vec<_>>>()?;
    let motive = if matches!(motive, Term::Hole) {
        let result = if let Some(expected) = expected {
            expected.clone()
        } else {
            let first = clauses
                .first()
                .ok_or_else(|| invalid("a pattern match needs at least one clause"))?;
            let mut scoped = ctx.clone();
            for (n, ty) in pattern_binder_types(ctx, &disc_ty, &first.patterns[0])? {
                scoped = scoped.extend(&n, ty);
            }
            infer_type(&scoped, &crate::surface_elaborate(&scoped, &first.body)?)?
        };
        crate::elaborate::infer_match_motive(ctx, &disc, &[], Some(&result))?
    } else {
        crate::elaborate::fill_match_motives(ctx, motive, None)?
    };
    let norm_motive = crate::reduction::normalize_elab(ctx, &motive);
    let result = match &norm_motive {
        Term::Lambda { param, body, .. } if occurs_free(body, param) => {
            return compile_dependent_flat(ctx, disc, motive, &disc_ty, &clauses);
        }
        Term::Lambda { body, .. } => body.as_ref().clone(),
        // A declared FAMILY over the discriminant's type (`return Fam` with
        // `Fam : Nat → Type`) is a dependent motive written without its λ — Lean's
        // `motive := Fam`. Reading it as a constant result type would make every branch
        // claim `Fam` itself.
        other if is_family_over(ctx, other, &disc_ty) => {
            return compile_dependent_flat(ctx, disc, motive, &disc_ty, &clauses);
        }
        other => other.clone(),
    };
    compile_clauses(ctx, &[disc], &result, &clauses)
}

/// Whether `motive` is a function from `disc_ty` into a sort — a family of result types
/// indexed by the scrutinee.
fn is_family_over(ctx: &Context, motive: &Term, disc_ty: &Term) -> bool {
    let Ok(ty) = infer_type(ctx, motive) else {
        return false;
    };
    match crate::reduction::normalize_elab(ctx, &ty) {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let domain_matches = crate::defeq_for_test(ctx, &param_type, disc_ty);
            let inner = ctx.extend(&param, *param_type);
            domain_matches
                && matches!(
                    crate::reduction::normalize_elab(&inner, &body_type),
                    Term::Sort(_)
                )
        }
        _ => false,
    }
}

/// Preserve explicit dependent motives on flat constructor alternatives. The
/// kernel still checks each branch against its constructor-specific motive.
fn compile_dependent_flat(
    ctx: &Context,
    disc: Term,
    motive: Term,
    disc_ty: &Term,
    clauses: &[Clause],
) -> KernelResult<Term> {
    let mut head = disc_ty;
    while let Term::App(f, _) = head {
        head = f;
    }
    let ind = match head {
        Term::Global(n) | Term::Const { name: n, .. } => n,
        _ => return Err(KernelError::NotAnInductive(format!("{disc_ty}"))),
    };
    let constructors = ctx.get_constructors(ind);
    let mut ordered = Vec::new();
    for (ctor, _) in constructors {
        let clause = clauses
            .iter()
            .find(|c| matches!(&c.patterns[0], Pattern::Ctor { name, .. } if name == ctor))
            .ok_or_else(|| {
                invalid(format!(
                    "match is not exhaustive: missing constructor '{ctor}'"
                ))
            })?;
        let Pattern::Ctor { args, .. } = &clause.patterns[0] else {
            unreachable!()
        };
        if !args.iter().all(|p| matches!(p, Pattern::Var(_))) {
            return Err(invalid(
                "a dependent match currently requires flat constructor patterns with named fields",
            ));
        }
        let mut body = clause.body.clone();
        for (name, ty) in pattern_binder_types(ctx, disc_ty, &clause.patterns[0])?
            .into_iter()
            .rev()
        {
            body = Term::Lambda {
                param: name,
                param_type: Box::new(ty),
                body: Box::new(body),
            };
        }
        ordered.push(body);
    }
    if clauses.len() != ordered.len() {
        return Err(invalid(
            "a dependent match requires one alternative per constructor",
        ));
    }
    Ok(Term::Match {
        discriminant: Box::new(disc),
        motive: Box::new(motive),
        cases: ordered,
    })
}
