//! The kernel's surface, written back: the exact inverse of `term_parser` and `command_parser`.
//!
//! For every term and command the printer spells, parsing the text in a fresh context yields the
//! same term (up to α) — `vernacular_roundtrip.rs` locks that. A term with no surface spelling is a
//! [`PrintError`], never an approximation: the caller reports source export as unsupported.
//!
//! # What "the same term" means
//!
//! The text is read back exactly as the `Repl` reads it: the term door, then name resolution at
//! the root scope against the environment the term lives in. That second stage is where a
//! qualified reference `Nat.add` becomes the constant `Nat.add` rather than the field `add` of
//! `Nat` — LOGOS's own staging, the one Lean has — so the printer writes qualified names as
//! qualified names rather than escaping them.
//!
//! # Binders
//!
//! A kernel term names its binders, and a name that does not read back BARE as that binder is
//! RENAMED rather than printed as is — α-renaming changes nothing the kernel can see. A binder is
//! renamed when its name would capture a global the body mentions (a binder `x` over a body naming
//! the global `x`, or a binder `Nat` over a body naming `Nat.add`), when the plain grammar reads it
//! as something else (a keyword, a sort, `_` that is referred to), or when it could be written only
//! in guillemets (the importer's `arena.local.3`) — a quoted binder name tells a reader nothing. A
//! fresh name is fresh against EVERY name the term spells, so a renamed binder can neither capture
//! nor be captured. Every other binder keeps its name. Globals are never renamed: a constant is
//! spelled bare, qualified, or `«quoted»`, and always means itself.
//!
//! # Depth
//!
//! Imported terms are deep. Both passes here — the scan that learns which binders are referred
//! to, and the printing itself — run on an explicit work stack, so printing a term never recurses
//! once per level of it.

use super::command::{Command, InductiveMember};
use super::surface::{Assoc, Name, OpenClause, OperatorForm, SurfaceDecl};
use super::term_parser::{is_reserved_word, is_sort_word, scan_name, TermParser, MAX_LEVEL_OFFSET};
use crate::{Literal, Term, Universe};
use std::collections::{HashMap, HashSet};

/// A term or command the LOGOS surface cannot spell (yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    NoSpelling { what: String },
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrintError::NoSpelling { what } => {
                write!(f, "the LOGOS surface has no spelling for {what}")
            }
        }
    }
}

fn no_spelling(what: impl Into<String>) -> PrintError {
    PrintError::NoSpelling { what: what.into() }
}

/// Spell `term` in the kernel term surface.
pub fn print_term(term: &Term) -> Result<String, PrintError> {
    let mut printer = Printer::new(&[term]);
    printer.term(term, Ctx::Top)?;
    Ok(printer.out)
}

/// Spell `term` read in a context whose locals are `locals`: each prints as its own name. A goal and
/// its hypotheses are spelled this way for a reader; a closed term's round trip is [`print_term`]'s,
/// which refuses a free name because the surface would read it as a global.
pub fn print_term_in(term: &Term, locals: &[&str]) -> Result<String, PrintError> {
    let mut printer = Printer::new(&[term]);
    for local in locals {
        printer
            .scope
            .entry(local.to_string())
            .or_default()
            .push(local.to_string());
        printer.avoid.insert(local.to_string());
    }
    printer.term(term, Ctx::Top)?;
    Ok(printer.out)
}

/// Spell one tactic of a script, its terms in the vernacular's own syntax, so the script reads back.
fn print_tactic(step: &crate::tactics::TacticStep) -> Result<String, PrintError> {
    use crate::tactics::TacticStep;
    let pattern = |cases: &[Vec<String>]| {
        if cases.is_empty() {
            String::new()
        } else {
            let cases: Vec<String> = cases.iter().map(|c| c.join(" ")).collect();
            format!(" as [{}]", cases.join(" | "))
        }
    };
    Ok(match step {
        TacticStep::Intro(names) => format!("intro {}", names.join(" ")),
        TacticStep::Exact(t) => format!("exact {}", slot(print_term(t)?, &[";"])),
        TacticStep::Apply(t) => format!("apply {}", slot(print_term(t)?, &[";"])),
        TacticStep::Exists(t) => format!("exists {}", slot(print_term(t)?, &[";"])),
        TacticStep::Change(t) => format!("change {}", slot(print_term(t)?, &[";"])),
        TacticStep::Have(n, t) => format!("have {n} : {}", slot(print_term(t)?, &[";"])),
        TacticStep::Rewrite(t, reverse) => {
            format!(
                "rw [{}{}]",
                if *reverse { "<- " } else { "" },
                slot(print_term(t)?, &[",", "]"])
            )
        }
        TacticStep::Assumption => "assumption".to_string(),
        TacticStep::Exfalso => "exfalso".to_string(),
        TacticStep::Constructor => "constructor".to_string(),
        TacticStep::Left => "left".to_string(),
        TacticStep::Right => "right".to_string(),
        TacticStep::Rfl => "rfl".to_string(),
        TacticStep::Induction(x, cases) => format!("induction {x}{}", pattern(cases)),
        TacticStep::Cases(x, cases) => format!("cases {x}{}", pattern(cases)),
        TacticStep::Refine(t) => format!("refine {}", slot(print_term(t)?, &[";"])),
        TacticStep::Subst(h) => format!("subst {h}"),
        TacticStep::Injection(h, names) if names.is_empty() => format!("injection {h}"),
        TacticStep::Injection(h, names) => format!("injection {h} as [{}]", names.join(" ")),
        TacticStep::Calc(steps) => {
            let steps = steps
                .iter()
                .map(|(r, p)| {
                    Ok(format!(
                        "{} := {}",
                        slot(print_term(r)?, &[":=", ",", "]"]),
                        slot(print_term(p)?, &[",", "]"])
                    ))
                })
                .collect::<Result<Vec<_>, PrintError>>()?;
            format!("calc [{}]", steps.join(", "))
        }
        TacticStep::Cc => "cc".to_string(),
        TacticStep::Decide => "decide".to_string(),
        TacticStep::Simp(lemmas) if lemmas.is_empty() => "simp".to_string(),
        TacticStep::Simp(lemmas) => {
            let lemmas = lemmas
                .iter()
                .map(|l| Ok(slot(print_term(l)?, &[",", "]"])))
                .collect::<Result<Vec<_>, PrintError>>()?;
            format!("simp [{}]", lemmas.join(", "))
        }
    })
}

/// Spell `command` as one vernacular command, terminated by its period.
pub fn print_command(command: &Command) -> Result<String, PrintError> {
    match command {
        Command::Universes { params, inner } => declaration(inner, Some(params)),
        Command::Definition { .. }
        | Command::Theorem { .. }
        | Command::TheoremBy { .. }
        | Command::Opaque { .. }
        | Command::Axiom { .. }
        | Command::InductiveBlock { .. }
        | Command::Quotient { .. } => declaration(command, None),
        Command::Inductive {
            name,
            params,
            sort,
            constructors,
        } => {
            if *sort != Term::Sort(Universe::Type(0)) {
                return Err(no_spelling(format!(
                    "the undeclared-arity inductive `{name}` at `{sort}` \u{2014} that form is always at \
                     `Type`; an inductive at another sort declares its arity, as a block"
                )));
            }
            if constructors.is_empty() {
                return Err(no_spelling(format!(
                    "the undeclared-arity inductive `{name}` with no constructors \u{2014} that form \
                     needs at least one; an empty inductive declares its arity, as a block"
                )));
            }
            let mut out = format!("Inductive {}", declaration_name(name)?);
            for (param, ty) in params {
                out.push_str(&parameter(param, ty)?);
            }
            out.push_str(" := ");
            out.push_str(&constructor_list(constructors)?);
            out.push('.');
            Ok(out)
        }
        Command::Check(t) => Ok(format!("Check {}.", plain_term(t, "`Check`")?)),
        Command::Eval(t) => Ok(format!("Eval {}.", plain_term(t, "`Eval`")?)),
        Command::Comment => Ok("/- -/".to_string()),
        Command::Structure {
            inductive,
            projections,
        } => {
            let mut out = format!("Structure {} :=", global_name(inductive)?);
            for projection in projections {
                out.push(' ');
                out.push_str(&global_name(projection)?);
            }
            out.push('.');
            Ok(out)
        }
        Command::PrintAxioms(name) => Ok(format!("Print Axioms {}.", name_path(name)?)),
        Command::Protected(inner) => match inner.as_ref() {
            // The qualifier is read off the front of a Coq-style sentence, so it can only precede
            // a command that dispatch reaches after it.
            Command::Protected(_)
            | Command::Open(_)
            | Command::OpenIn { .. }
            | Command::Comment => Err(no_spelling(
                "`Protected` in front of a command the qualifier cannot precede",
            )),
            _ => Ok(format!("Protected {}", print_command(inner)?)),
        },
        Command::Export { namespace, names } => {
            let names = names.iter().map(name_path).collect::<Result<Vec<_>, _>>()?;
            if names.is_empty() {
                return Err(no_spelling("an `Export` naming nothing"));
            }
            Ok(format!(
                "Export {} ({}).",
                name_path(namespace)?,
                names.join(" ")
            ))
        }
        Command::Open(clauses) => Ok(format!("Open {}.", open_clauses(clauses)?)),
        Command::OpenIn { opens, inner } => Ok(format!(
            "Open {} in {}",
            open_clauses(opens)?,
            print_command(inner)?
        )),
        Command::Surface(decl) => surface_declaration(decl),
    }
}

/// The vernacular command that registers the inductive block `name` belongs to, EXACTLY as `ctx`
/// holds it — its universe telescope, each member's written parameters (the context's declared
/// parameter count), its arity past them, and each constructor's type past them.
///
/// ★ THE PARAMETER SPLIT IS THE SUBTLE PART, SO IT IS WRITTEN ONCE, HERE. The command surface
/// refers to a parameter, inside the arity and the constructor types, as the GLOBAL of its name —
/// the registration then binds every global of that spelling. So a parameter must be named by a
/// spelling nothing else in those terms uses: not a constant they mention (it would be captured
/// and bound), and not a binder inside them (the conversion would reach under it and be captured
/// in turn). The kernel's own binder name is kept when it is safe, which keeps Lean's `α`, and a
/// fresh one is chosen otherwise.
///
/// The command is built, not printed: pass it to [`print_command`] for the text.
pub fn inductive_command(ctx: &crate::Context, name: &str) -> Result<Command, PrintError> {
    if !ctx.is_inductive(name) {
        return Err(no_spelling(format!(
            "`{name}` as an inductive \u{2014} the context declares no such type"
        )));
    }
    let block: Vec<String> = ctx
        .mutual_block_of(name)
        .map(|b| b.to_vec())
        .unwrap_or_else(|| vec![name.to_string()]);
    let telescope = ctx
        .poly_declaration(name)
        .map(|(params, _)| params.to_vec());
    let mut members = Vec::with_capacity(block.len());
    for member in &block {
        let arity = ctx
            .get_global(member)
            .ok_or_else(|| no_spelling(format!("the header of the block member `{member}`")))?;
        let count = ctx.inductive_num_params(member);
        let constructors = ctx.get_constructors(member);
        // Every name the block member's terms spell (a fresh name avoids all of them), and the
        // names a KEPT parameter name must not collide with: a constant or the head of a qualified
        // one, and any binder that is not one of the parameter binders being peeled off.
        let mut avoid: HashSet<String> = HashSet::new();
        let mut conflicts: HashSet<String> = HashSet::new();
        for t in std::iter::once(arity).chain(constructors.iter().map(|(_, t)| *t)) {
            spelled_names(t, &mut avoid);
            let mut rest = t;
            for _ in 0..count {
                if let Term::Pi {
                    param_type,
                    body_type,
                    ..
                } = rest
                {
                    constants_and_binders(param_type, &mut conflicts);
                    rest = body_type;
                }
            }
            constants_and_binders(rest, &mut conflicts);
        }
        let mut names: Vec<String> = Vec::with_capacity(count);
        let mut cur = arity;
        for _ in 0..count {
            let Term::Pi {
                param, body_type, ..
            } = cur
            else {
                return Err(no_spelling(format!(
                    "`{member}` declaring {count} parameters with an arity of fewer binders"
                )));
            };
            let keep = plain_atom(param) && !names.contains(param) && !conflicts.contains(param);
            let chosen = if keep {
                param.clone()
            } else {
                let mut k = 0usize;
                loop {
                    let candidate = if k == 0 {
                        "A".to_string()
                    } else {
                        format!("A{k}")
                    };
                    k += 1;
                    if !avoid.contains(&candidate) && !names.contains(&candidate) {
                        break candidate;
                    }
                }
            };
            names.push(chosen);
            cur = body_type;
        }
        let (params, sort) = split_telescope(arity, &names, member)?;
        let split = constructors
            .iter()
            .map(|(ctor, ty)| split_telescope(ty, &names, ctor))
            .collect::<Result<Vec<_>, PrintError>>()?;
        // A constructor whose own parameter binders do not spell the member's parameter types
        // (Lean's `HAdd.mk` keeps `outParam (Type w)` where the normalised arity says `Type w`)
        // cannot be written in the short form, which rebuilds them from the member: then every
        // constructor of the member restates its telescope, and is kept exactly as registered.
        let full = split
            .iter()
            .any(|(restated, _)| restated.iter().zip(&params).any(|((_, t), (_, p))| t != p));
        let constructors = if full {
            constructors
                .iter()
                .map(|(ctor, ty)| (ctor.to_string(), (*ty).clone()))
                .collect()
        } else {
            constructors
                .iter()
                .zip(split)
                .map(|((ctor, _), (_, rest))| (ctor.to_string(), rest))
                .collect()
        };
        members.push(InductiveMember {
            name: member.clone(),
            params,
            sort,
            constructors,
            full_constructors: full,
        });
    }
    let block = Command::InductiveBlock { members };
    Ok(match telescope {
        Some(params) => Command::Universes {
            params,
            inner: Box::new(block),
        },
        None => block,
    })
}

/// The constants a term mentions (and the head of each qualified one) and the binders it
/// introduces — every spelling a parameter referred to by name could be captured by.
fn constants_and_binders(t: &Term, out: &mut HashSet<String>) {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(n) | Term::Const { name: n, .. } => {
                out.insert(n.clone());
                if let Some(name) = Name::parse(n) {
                    out.insert(name.parts()[0].clone());
                }
            }
            Term::Pi {
                param,
                param_type,
                body_type: body,
            }
            | Term::Lambda {
                param,
                param_type,
                body,
            } => {
                out.insert(param.clone());
                work.push(param_type);
                work.push(body);
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                out.insert(name.clone());
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Fix { name, body } => {
                out.insert(name.clone());
                work.push(body);
            }
            Term::MutualFix { defs, .. } => {
                for (n, b) in defs {
                    out.insert(n.clone());
                    work.push(b);
                }
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Annotated { term, .. } => work.push(term),
            Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
}

/// The commands that make `name` a STRUCTURE exactly as `ctx` holds it: each projection's
/// definition (under the inductive's universe telescope), then `Structure name := proj₀ … projₙ.`
/// Empty when `ctx` holds no structure information for `name`. Emit them right after the block
/// from [`inductive_command`], before anything — a recursor whose body is written through the
/// projections, say — relies on structure η.
pub fn structure_commands(ctx: &crate::Context, name: &str) -> Result<Vec<Command>, PrintError> {
    let Some(info) = ctx.struct_info(name) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(info.projections.len() + 1);
    for projection in &info.projections {
        let (params, ty, body) = match (ctx.get_universe_poly(projection), ctx.get_definition_body(projection)) {
            (Some((params, ty, body)), _) if !params.is_empty() => (params.clone(), ty.clone(), body.clone()),
            (_, Some(body)) => (
                Vec::new(),
                ctx.get_definition_type(projection)
                    .ok_or_else(|| no_spelling(format!("the type of the projection `{projection}`")))?
                    .clone(),
                body.clone(),
            ),
            (Some((params, ty, body)), None) => (params.clone(), ty.clone(), body.clone()),
            (None, None) => {
                return Err(no_spelling(format!(
                    "the projection `{projection}` of `{name}`, which the context holds no definition for"
                )))
            }
        };
        let definition = Command::Definition {
            name: projection.clone(),
            ty: Some(ty),
            body,
            is_hint: false,
            implicit_count: 0,
            proof_goal: None,
        };
        out.push(if params.is_empty() {
            definition
        } else {
            Command::Universes {
                params,
                inner: Box::new(definition),
            }
        });
    }
    out.push(Command::Structure {
        inductive: name.to_string(),
        projections: info.projections.clone(),
    });
    Ok(out)
}

/// Every name a term spells — binders, variables, globals and each component of a qualified one.
fn spelled_names(t: &Term, out: &mut HashSet<String>) {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Var(n) => {
                out.insert(n.clone());
            }
            Term::Global(n) | Term::Const { name: n, .. } => {
                out.insert(n.clone());
                if let Some(name) = Name::parse(n) {
                    out.extend(name.parts().iter().cloned());
                }
            }
            Term::Pi {
                param,
                param_type,
                body_type: body,
            }
            | Term::Lambda {
                param,
                param_type,
                body,
            } => {
                out.insert(param.clone());
                work.push(param_type);
                work.push(body);
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                out.insert(name.clone());
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Fix { name, body } => {
                out.insert(name.clone());
                work.push(body);
            }
            Term::MutualFix { defs, .. } => {
                for (n, b) in defs {
                    out.insert(n.clone());
                    work.push(b);
                }
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Annotated { term, .. } => work.push(term),
            Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
}

/// Peel `names.len()` leading `Π`s off `ty`, turning each binder's variable into the global of the
/// chosen parameter name — the command surface's convention — and return the parameters and the
/// rest.
fn split_telescope(
    ty: &Term,
    names: &[String],
    owner: &str,
) -> Result<(Vec<(String, Term)>, Term), PrintError> {
    let mut params = Vec::with_capacity(names.len());
    let mut cur = ty.clone();
    for name in names {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = cur
        else {
            return Err(no_spelling(format!(
                "`{owner}` with fewer binders than its block's parameters"
            )));
        };
        params.push((name.clone(), *param_type));
        cur = crate::substitute(&body_type, &param, &Term::Global(name.clone()));
    }
    Ok((params, cur))
}

// =================================================================================================
// Commands
// =================================================================================================

/// A declaring command, with the universe telescope written after its name when it has one.
fn declaration(cmd: &Command, universes: Option<&Vec<String>>) -> Result<String, PrintError> {
    let telescope = match universes {
        Some(params) => universe_telescope(params)?,
        None => String::new(),
    };
    match cmd {
        Command::Definition {
            name,
            ty,
            body,
            is_hint,
            implicit_count,
            proof_goal,
        } => {
            if *is_hint {
                return Err(no_spelling(format!(
                    "the hint flag on `{name}` \u{2014} no vernacular command registers a hint"
                )));
            }
            if proof_goal.is_some() {
                return Err(no_spelling(format!(
                    "the tactic goal carried by `{name}` \u{2014} only a literate `## Theorem` proof \
                     carries one, and the vernacular `Definition` cannot state it"
                )));
            }
            let declared = ty.as_ref().unwrap_or(body);
            if leading_implicits(declared) != *implicit_count {
                return Err(no_spelling(format!(
                    "`{name}` with {implicit_count} leading implicit parameter(s) its written type \
                     does not bind \u{2014} the count is read off the `{{…}}` binders, so it cannot \
                     be stated apart from them"
                )));
            }
            let head = format!("Definition {}{telescope}", declaration_name(name)?);
            match ty {
                Some(ty) => Ok(format!(
                    "{head} : {} := {}.",
                    slot(print_term(ty)?, &[":="]),
                    plain_term(body, "a definition body under a declared type")?
                )),
                None => Ok(format!("{head} := {}.", print_term(body)?)),
            }
        }
        Command::Theorem { name, ty, proof } => Ok(format!(
            "Theorem {}{telescope} : {} := {}.",
            declaration_name(name)?,
            slot(print_term(ty)?, &[":="]),
            plain_term(proof, "a theorem's proof")?
        )),
        Command::TheoremBy { name, ty, script } => {
            let steps: Vec<String> = script.iter().map(print_tactic).collect::<Result<_, _>>()?;
            Ok(format!(
                "Theorem {}{telescope} : {} := by {}.",
                declaration_name(name)?,
                slot(print_term(ty)?, &[":="]),
                steps.join("; ")
            ))
        }
        Command::Opaque { name, ty, value } => Ok(format!(
            "Opaque {}{telescope} : {} := {}.",
            declaration_name(name)?,
            slot(print_term(ty)?, &[":="]),
            plain_term(value, "an opaque declaration's value")?
        )),
        Command::Axiom { name, ty } => Ok(format!(
            "Axiom {}{telescope} : {}.",
            declaration_name(name)?,
            slot(print_term(ty)?, &[":="])
        )),
        Command::InductiveBlock { members } => {
            if members.is_empty() {
                return Err(no_spelling("an inductive block with no members"));
            }
            let members = members
                .iter()
                .enumerate()
                .map(|(i, m)| block_member(m, if i == 0 { &telescope } else { "" }))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("Inductive {}.", members.join(" with ")))
        }
        Command::Quotient { kind, equality } => {
            use crate::quotient::Kind;
            if universes.is_none() {
                return Err(no_spelling(format!(
                    "the quotient primitive `{}` without its universe parameters \u{2014} they are \
                     always written",
                    kind.name()
                )));
            }
            let word = match kind {
                Kind::Type => "Type",
                Kind::Constructor => "Constructor",
                Kind::Lift => "Lift",
                Kind::Induction => "Induction",
            };
            match (kind, equality) {
                (Kind::Lift, Some(eq)) => Ok(format!(
                    "Quotient {word}{telescope} using {}.",
                    global_name(eq)?
                )),
                (Kind::Lift, None) => Err(no_spelling("a quotient lift naming no equality")),
                (_, Some(_)) => Err(no_spelling(format!(
                    "`Quotient {word}` naming an equality \u{2014} only `Lift` states one"
                ))),
                (_, None) => Ok(format!("Quotient {word}{telescope}.")),
            }
        }
        Command::Universes { .. } => Err(no_spelling(
            "a universe telescope on a declaration that already has one",
        )),
        other => Err(no_spelling(format!(
            "a universe telescope on {} \u{2014} universe parameters bind over a declaration",
            match other {
                Command::Inductive { .. } => "the undeclared-arity `Inductive` form",
                _ => "a command that declares nothing",
            }
        ))),
    }
}

/// One member of an inductive block: `Name.{u} (p : T) : arity := C : T | …`.
fn block_member(m: &InductiveMember, telescope: &str) -> Result<String, PrintError> {
    let mut out = format!("{}{telescope}", declaration_name(&m.name)?);
    for (param, ty) in &m.params {
        out.push_str(&parameter(param, ty)?);
    }
    out.push_str(" : ");
    out.push_str(&slot(
        plain_term(&m.sort, "an inductive's arity")?,
        &[":=", "|", "with"],
    ));
    if m.full_constructors && m.params.is_empty() {
        return Err(no_spelling(format!(
            "restated constructor telescopes on `{}`, which has no parameters to restate",
            m.name
        )));
    }
    if !m.constructors.is_empty() {
        out.push_str(" := ");
        out.push_str(&if m.full_constructors {
            restated_constructor_list(&m.constructors, m.params.len())?
        } else {
            constructor_list(&m.constructors)?
        });
    }
    Ok(out)
}

/// `C (p₁ : T₁) … (pₙ : Tₙ) : rest | …` — each constructor restating its parameter telescope, read
/// back as the full type `(p₁ : T₁) -> … -> rest`.
fn restated_constructor_list(
    constructors: &[(String, Term)],
    params: usize,
) -> Result<String, PrintError> {
    let parts = constructors
        .iter()
        .map(|(name, ty)| {
            if has_kind_marker(ty) {
                return Err(no_spelling(format!(
                    "a binder-kind marker in the constructor `{name}`"
                )));
            }
            let mut printer = Printer::new(&[ty]);
            let (binders, rest) = printer.telescope(ty, params, name)?;
            Ok(format!(
                "{}{binders} : {}",
                declaration_name(name)?,
                slot(rest, &[":=", "|", "with"])
            ))
        })
        .collect::<Result<Vec<_>, PrintError>>()?;
    Ok(parts.join(" | "))
}

/// ` (p : T)` — one written parameter.
fn parameter(name: &str, ty: &Term) -> Result<String, PrintError> {
    Ok(format!(
        " ({} : {})",
        declaration_name(name)?,
        plain_term(ty, "a parameter's type")?
    ))
}

/// `C₁ : T₁ | C₂ : T₂` — each type bracketed if it holds a separator the sentence is split at.
fn constructor_list(constructors: &[(String, Term)]) -> Result<String, PrintError> {
    let parts = constructors
        .iter()
        .map(|(name, ty)| {
            Ok(format!(
                "{} : {}",
                declaration_name(name)?,
                slot(
                    plain_term(ty, "a constructor's type")?,
                    &[":=", "|", "with"]
                )
            ))
        })
        .collect::<Result<Vec<_>, PrintError>>()?;
    Ok(parts.join(" | "))
}

/// `.{u, v}` — a universe telescope.
fn universe_telescope(params: &[String]) -> Result<String, PrintError> {
    let names = params
        .iter()
        .map(|p| level_name(p))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(".{{{}}}", names.join(", ")))
}

/// A term in a slot the command door reads with binder-kind markers STRIPPED (`parse_in`). A
/// marker there would be dropped on the way back in, so a term carrying one has no spelling in
/// that slot.
fn plain_term(t: &Term, slot_name: &str) -> Result<String, PrintError> {
    if has_kind_marker(t) {
        return Err(no_spelling(format!(
            "a binder-kind marker (`{{…}}`, `⦃…⦄`, `[…]`) in {slot_name} \u{2014} that slot reads its \
             term with the kinds stripped, so the binder kind would not survive"
        )));
    }
    print_term(t)
}

/// `text`, parenthesised if any of `separators` occurs in it at TOP LEVEL — where the command
/// door would split the sentence. A parenthesised term is the same term.
fn slot(text: String, separators: &[&str]) -> String {
    let splits = separators.iter().any(|sep| {
        if sep.chars().all(char::is_alphabetic) {
            top_level_word(&text, sep)
        } else {
            TermParser::find_top_level(&text, sep).is_some()
        }
    });
    if splits {
        format!("({text})")
    } else {
        text
    }
}

/// Does the WORD `word` occur at top level of `text`?
fn top_level_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let mut from = 0;
    while let Some(rel) = TermParser::find_top_level(&text[from..], word) {
        let at = from + rel;
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        if before.is_none_or(|c| !is_ident(c)) && after.is_none_or(|c| !is_ident(c)) {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// The number of LEADING implicit binders of a declared type (or a lambda telescope) — the count
/// the command door reads off it.
fn leading_implicits(t: &Term) -> usize {
    TermParser::binder_kinds_of(t)
        .iter()
        .take_while(|k| **k == TermParser::IMPLICIT)
        .count()
}

fn open_clauses(clauses: &[OpenClause]) -> Result<String, PrintError> {
    if clauses.is_empty() {
        return Err(no_spelling("an `Open` naming no namespace"));
    }
    let mut parts = Vec::with_capacity(clauses.len());
    for clause in clauses {
        // A clause list ends at the words that scope or narrow it, so a namespace spelled like one
        // of them is quoted.
        let parts_of = clause.namespace.parts();
        let mut part = if matches!(parts_of[0].as_str(), "in" | "hiding" | "renaming") {
            let mut quoted = format!("\u{ab}{}\u{bb}", parts_of[0]);
            for p in &parts_of[1..] {
                quoted.push('.');
                quoted.push_str(&tail_component(p)?);
            }
            quoted
        } else {
            name_path(&clause.namespace)?
        };
        let forms = clause.only.is_some() as usize
            + (!clause.hiding.is_empty()) as usize
            + (!clause.renaming.is_empty()) as usize;
        if forms > 1 {
            return Err(no_spelling(format!(
                "an `Open {part}` clause that both narrows and renames \u{2014} a clause is one form"
            )));
        }
        if let Some(only) = &clause.only {
            let names = only.iter().map(name_path).collect::<Result<Vec<_>, _>>()?;
            if names.is_empty() {
                return Err(no_spelling(format!("an `Open {part} ()` naming nothing")));
            }
            part.push_str(&format!(" ({})", names.join(" ")));
        } else if !clause.hiding.is_empty() {
            let names = clause
                .hiding
                .iter()
                .map(name_path)
                .collect::<Result<Vec<_>, _>>()?;
            part.push_str(&format!(" hiding ({})", names.join(" ")));
        } else if !clause.renaming.is_empty() {
            let pairs = clause
                .renaming
                .iter()
                .map(|(member, local)| {
                    Ok(format!("{} -> {}", name_path(member)?, name_path(local)?))
                })
                .collect::<Result<Vec<_>, PrintError>>()?;
            part.push_str(&format!(" renaming {}", pairs.join(", ")));
        }
        parts.push(part);
    }
    Ok(parts.join(" "))
}

fn surface_declaration(decl: &SurfaceDecl) -> Result<String, PrintError> {
    let quoted = |pattern: &str| {
        if pattern.contains('"') {
            Err(no_spelling(format!(
                "the pattern {pattern:?} \u{2014} a notation's pattern is a quoted string with no `\"` in it"
            )))
        } else {
            Ok(format!("\"{pattern}\""))
        }
    };
    let expansion = |template: &str| {
        let text = template.trim();
        let sentence = format!("{text}.");
        if text.is_empty() || super::command_parser::sentence_end(&sentence) != sentence.len() {
            Err(no_spelling(format!(
                "the expansion {template:?} \u{2014} it would end the sentence early"
            )))
        } else {
            Ok(text.to_string())
        }
    };
    let prec = |p: &Option<u32>| p.map(|p| format!("{p} ")).unwrap_or_default();
    match decl {
        SurfaceDecl::Notation {
            pattern,
            template,
            precedence,
            assoc,
            operator_form,
        } => {
            let word = match (operator_form, assoc) {
                (None, Assoc::Right) => "Notation",
                (Some(OperatorForm::Infix), Assoc::Left) => "Infixl",
                (Some(OperatorForm::Infix), Assoc::Right) => "Infixr",
                (Some(OperatorForm::Prefix), Assoc::Right) => "Prefix",
                (Some(OperatorForm::Postfix), Assoc::Left) => "Postfix",
                _ => {
                    return Err(no_spelling(format!(
                        "the notation {pattern:?} with {assoc:?} associativity \u{2014} no command \
                         word declares that combination"
                    )))
                }
            };
            if word.starts_with("Infix") && precedence.is_none() {
                return Err(no_spelling(format!(
                    "the infix notation {pattern:?} with no precedence \u{2014} `{word}` requires one"
                )));
            }
            Ok(format!(
                "{word} {}{} := {}.",
                prec(precedence),
                quoted(pattern)?,
                expansion(template)?
            ))
        }
        SurfaceDecl::Macro { pattern, template } => Ok(format!(
            "Macro {} := {}.",
            quoted(pattern)?,
            expansion(template)?
        )),
        SurfaceDecl::OpenSection(None) => Ok("Section.".to_string()),
        SurfaceDecl::OpenSection(Some(name)) => Ok(format!("Section {}.", declaration_name(name)?)),
        SurfaceDecl::CloseSection(None) => Ok("End.".to_string()),
        SurfaceDecl::CloseSection(Some(name)) => Ok(format!("End {}.", declaration_name(name)?)),
        SurfaceDecl::OpenNamespace(name) => Ok(format!("Namespace {}.", name_path(name)?)),
    }
}

// =================================================================================================
// Names
// =================================================================================================

/// May `s` be written BARE as a term atom and read back as the name `s`?
fn plain_atom(s: &str) -> bool {
    matches!(scan_name(s), Some((_, len)) if len == s.len())
        && !s.starts_with('\u{ab}')
        && !is_reserved_word(s)
        && !is_sort_word(s)
        && s != "_"
}

/// Can `s` be written in guillemets at all?
fn quotable(s: &str) -> bool {
    !s.is_empty() && !s.contains(['\u{ab}', '\u{bb}', '\u{27e8}', '\u{27e9}'])
}

/// A single name as one token: bare when it reads back bare, `«quoted»` otherwise.
fn token(s: &str, what: &str) -> Result<String, PrintError> {
    if plain_atom(s) {
        Ok(s.to_string())
    } else if quotable(s) {
        Ok(format!("\u{ab}{s}\u{bb}"))
    } else if s.is_empty() {
        Err(no_spelling(format!("the empty name as {what}")))
    } else {
        Err(no_spelling(format!(
            "the name {s:?} as {what} \u{2014} a name holding a guillemet or an angle bracket has no \
             quoted form"
        )))
    }
}

/// A component written after a `.` — the term door's `dot_component` reads it with the name
/// grammar alone, so a reserved word is fine there and only an unscannable one is quoted.
fn tail_component(s: &str) -> Result<String, PrintError> {
    if matches!(scan_name(s), Some((_, len)) if len == s.len()) && !s.starts_with('\u{ab}') {
        Ok(s.to_string())
    } else {
        token(s, "a name component")
    }
}

/// The spelling of the constant keyed `key` in REFERENCE position: a qualified name when the key
/// is one, a bare name, or a `«quoted»` one.
///
/// A key written with the root escape (`_root_.a.b`, which an emitter uses for a reference its own
/// namespace would otherwise capture) is printed verbatim; the reader's resolution strips the
/// escape, so it reads back as the absolute `a.b` it stands for.
fn global_name(key: &str) -> Result<String, PrintError> {
    if key == "_" {
        return Err(no_spelling(
            "the global `_` \u{2014} `_` is a hole, and the kernel's own match-case placeholder; it is \
             never a name a term refers to",
        ));
    }
    match Name::parse(key) {
        Some(name) if name.parts().len() > 1 => {
            let parts = name.parts();
            let mut out = token(&parts[0], "a qualified name's head")?;
            for part in &parts[1..] {
                out.push('.');
                out.push_str(&tail_component(part)?);
            }
            Ok(out)
        }
        _ => token(key, "a global"),
    }
}

/// The spelling of a DECLARATION name — the same spelling as a reference, because the declaration
/// door keys a name only if a term can spell it back.
fn declaration_name(key: &str) -> Result<String, PrintError> {
    global_name(key)
}

/// A hierarchical `Name` as written in `Open`, `Export` and `Print Axioms`.
fn name_path(name: &Name) -> Result<String, PrintError> {
    let parts = name.parts();
    let mut out = token(&parts[0], "a name")?;
    for part in &parts[1..] {
        out.push('.');
        out.push_str(&tail_component(part)?);
    }
    Ok(out)
}

/// A universe variable's name: bare unless the level grammar reads that word as syntax.
fn level_name(name: &str) -> Result<String, PrintError> {
    if plain_atom(name) && name != "max" && name != "imax" {
        Ok(name.to_string())
    } else {
        let quoted = token(name, "a universe variable")?;
        if quoted.starts_with('\u{ab}') {
            Ok(quoted)
        } else {
            Ok(format!("\u{ab}{name}\u{bb}"))
        }
    }
}

// =================================================================================================
// Levels
// =================================================================================================

/// A level; `atom` when it stands where the level grammar reads one ATOM (after `Sort`, as an
/// operand of `max`/`imax`, as the base of `+k`).
fn level(u: &Universe, atom: bool) -> Result<String, PrintError> {
    let text = match u {
        Universe::Prop => return Ok("0".to_string()),
        Universe::Type(n) => return Ok((u64::from(*n) + 1).to_string()),
        Universe::Var(name) => return level_name(name),
        Universe::SProp => {
            return Err(no_spelling(
                "`SProp` inside a universe level \u{2014} it is a sort, not a level, and collapses out \
                 of every `max`, `imax` and successor",
            ))
        }
        Universe::Succ(_) => {
            let mut base = u;
            let mut k: u64 = 0;
            while let Universe::Succ(inner) = base {
                k += 1;
                base = inner;
            }
            let mut text = level(base, true)?;
            while k > 0 {
                let step = k.min(MAX_LEVEL_OFFSET);
                text.push_str(&format!("+{step}"));
                k -= step;
            }
            text
        }
        Universe::Max(a, b) => format!("max {} {}", level(a, true)?, level(b, true)?),
        Universe::IMax(a, b) => format!("imax {} {}", level(a, true)?, level(b, true)?),
    };
    Ok(if atom { format!("({text})") } else { text })
}

/// A sort: `Prop`, `SProp`, `Type`, `Type n`, or `Sort ℓ`.
fn sort(u: &Universe) -> Result<String, PrintError> {
    Ok(match u {
        Universe::Prop => "Prop".to_string(),
        Universe::SProp => "SProp".to_string(),
        Universe::Type(0) => "Type".to_string(),
        Universe::Type(n) => format!("Type {n}"),
        other => format!("Sort {}", level(other, true)?),
    })
}

// =================================================================================================
// Literals
// =================================================================================================

fn literal(l: &Literal) -> Result<String, PrintError> {
    Ok(match l {
        Literal::Int(n) => n.to_string(),
        Literal::BigInt(n) => {
            if n.to_i64().is_some() {
                return Err(no_spelling(format!(
                    "the BigInt literal {n}, which fits a machine integer \u{2014} an integer has ONE \
                     literal representation, and this value's is `Int`"
                )));
            }
            n.to_string()
        }
        Literal::Nat(n) => {
            if n.is_negative() {
                return Err(no_spelling(format!("the negative Nat literal {n}")));
            }
            format!("{n}n")
        }
        Literal::Float(x) if x.is_finite() => format!("{x:?}f64"),
        Literal::Float(x) => format!("0x{:016x}f64", x.to_bits()),
        Literal::Text(s) => text(s),
        Literal::Duration(n) => duration(*n),
        Literal::Date(d) => date(i64::from(*d)),
        Literal::Moment(n) => moment(*n),
    })
}

fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}') => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An annotation's surface spelling, inside `@[…]` — the term parser's `annotation` production.
fn annotation_text(annotation: &crate::Annotation) -> String {
    use crate::{Annotation, Provenance};
    match annotation {
        Annotation::Binder(kind) => format!("binder {}", kind.lean_binder_info()),
        Annotation::Source(Provenance::Position(position)) => {
            format!("source {}", text(position.as_str()))
        }
        Annotation::Source(Provenance::LeanMetadata { key, value }) => {
            format!("mdata {} {}", text(key), text(value))
        }
    }
}

/// A duration in the coarsest unit that divides it exactly.
fn duration(n: i64) -> String {
    if n == 0 {
        return "0s".to_string();
    }
    const UNITS: [(&str, i64); 6] = [
        ("h", 3_600_000_000_000),
        ("min", 60_000_000_000),
        ("s", 1_000_000_000),
        ("ms", 1_000_000),
        ("us", 1_000),
        ("ns", 1),
    ];
    let (unit, per) = UNITS
        .iter()
        .find(|(_, per)| n % per == 0)
        .expect("every count divides by 1");
    format!("{}{unit}", n / per)
}

fn calendar_date(days: i64) -> String {
    let (y, m, d) = super::term_parser::civil_from_days(days);
    let year = if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    };
    format!("{year}-{m:02}-{d:02}")
}

fn date(days: i64) -> String {
    calendar_date(days)
}

fn moment(nanos: i64) -> String {
    let secs = nanos.div_euclid(1_000_000_000);
    let frac = nanos.rem_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let mut out = format!(
        "{}T{:02}:{:02}:{:02}",
        calendar_date(days),
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    );
    if frac != 0 {
        let digits = format!("{frac:09}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
    out.push('Z');
    out
}

/// Does a literal print with a leading `-`?
fn negative_literal(l: &Literal) -> bool {
    match l {
        Literal::Int(n) | Literal::Duration(n) | Literal::Moment(n) => *n < 0,
        Literal::BigInt(n) => n.is_negative(),
        Literal::Float(x) => x.is_finite() && x.is_sign_negative(),
        Literal::Date(d) => super::term_parser::civil_from_days(i64::from(*d)).0 < 0,
        Literal::Nat(_) | Literal::Text(_) => false,
    }
}

// =================================================================================================
// Terms
// =================================================================================================

/// Where a term stands, which decides what needs parentheses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    /// Nothing follows that the term could swallow: the whole term, inside brackets, before `|`,
    /// `end`, `:=`, `;`, `,` or `⟩`.
    Top,
    /// Before `with` or `return` — a mutual `fix`'s bodies. A `fix` at the tail would read those
    /// words as its own.
    BeforeWith,
    /// An application or anything tighter: the left of `->`, a match's discriminant or motive.
    Operand,
    /// An argument or an application's head: an atom.
    Atom,
}

/// How a term prints, for the purpose of bracketing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Binds as tightly as anything.
    Atom,
    /// An application, or a sort or literal that reads as one.
    App,
    /// Closed on the right by `end`, but not an atom.
    Closed,
    /// Extends as far right as it can: `fun`, a binder, `let`.
    Open,
    /// A `fix` — open on the right, and it reads a following `with` or `return` as its own.
    Fix,
}

fn needs_parens(shape: Shape, ctx: Ctx) -> bool {
    match ctx {
        Ctx::Top => false,
        Ctx::BeforeWith => shape == Shape::Fix,
        Ctx::Operand => !matches!(shape, Shape::Atom | Shape::App),
        Ctx::Atom => shape != Shape::Atom,
    }
}

/// The identity of a binder, stable across the two passes: the address of the node that binds it,
/// and for a mutual block which member.
type BinderId = (usize, usize);

fn node_id(t: &Term) -> usize {
    t as *const Term as usize
}

/// A pending piece of printing work.
enum Job<'t> {
    Text(&'static str),
    Owned(String),
    Term(&'t Term, Ctx),
    /// Bring binders into scope: (the kernel's name, the spelling that now reads as it).
    Bind(Vec<(String, String)>),
    /// Take the last `n` binders back out of scope.
    Unbind(usize),
}

struct Printer {
    out: String,
    /// Binders some variable in their scope refers to.
    referenced: HashSet<BinderId>,
    /// Names a binder may not keep: every global the terms reference, and every head of a
    /// qualified one — keeping them would capture that global.
    avoid: HashSet<String>,
    /// Every name the terms spell anywhere; a fresh name is chosen outside it.
    taken: HashSet<String>,
    fresh: usize,
    /// The binders in scope, innermost last: kernel name → spelling.
    scope: HashMap<String, Vec<String>>,
    scope_order: Vec<String>,
}

impl Printer {
    fn new(roots: &[&Term]) -> Self {
        let mut printer = Printer {
            out: String::new(),
            referenced: HashSet::new(),
            avoid: HashSet::new(),
            taken: HashSet::new(),
            fresh: 0,
            scope: HashMap::new(),
            scope_order: Vec::new(),
        };
        for root in roots {
            printer.scan(root);
        }
        printer
    }

    /// The first pass: which binders are referred to, which names are globals, which are taken.
    fn scan(&mut self, root: &Term) {
        enum Visit<'t> {
            Enter(&'t Term),
            Bind(String, BinderId),
            Unbind(String),
        }
        let mut bound: HashMap<String, Vec<BinderId>> = HashMap::new();
        let mut work = vec![Visit::Enter(root)];
        while let Some(visit) = work.pop() {
            let t = match visit {
                Visit::Bind(name, id) => {
                    bound.entry(name).or_default().push(id);
                    continue;
                }
                Visit::Unbind(name) => {
                    if let Some(stack) = bound.get_mut(&name) {
                        stack.pop();
                    }
                    continue;
                }
                Visit::Enter(t) => t,
            };
            match t {
                Term::Var(n) => {
                    self.taken.insert(n.clone());
                    if let Some(id) = bound.get(n).and_then(|s| s.last()) {
                        self.referenced.insert(*id);
                    }
                }
                Term::Global(n) | Term::Const { name: n, .. } => self.note_global(n),
                Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
                Term::Pi {
                    param,
                    param_type,
                    body_type: body,
                }
                | Term::Lambda {
                    param,
                    param_type,
                    body,
                } => {
                    self.taken.insert(param.clone());
                    work.push(Visit::Unbind(param.clone()));
                    work.push(Visit::Enter(body));
                    work.push(Visit::Bind(param.clone(), (node_id(t), 0)));
                    work.push(Visit::Enter(param_type));
                }
                Term::Let {
                    name,
                    ty,
                    value,
                    body,
                } => {
                    self.taken.insert(name.clone());
                    work.push(Visit::Unbind(name.clone()));
                    work.push(Visit::Enter(body));
                    work.push(Visit::Bind(name.clone(), (node_id(t), 0)));
                    work.push(Visit::Enter(value));
                    work.push(Visit::Enter(ty));
                }
                Term::Fix { name, body } => {
                    self.taken.insert(name.clone());
                    work.push(Visit::Unbind(name.clone()));
                    work.push(Visit::Enter(body));
                    work.push(Visit::Bind(name.clone(), (node_id(t), 0)));
                }
                Term::MutualFix { defs, .. } => {
                    for (n, _) in defs.iter().rev() {
                        work.push(Visit::Unbind(n.clone()));
                    }
                    for (_, b) in defs.iter().rev() {
                        work.push(Visit::Enter(b));
                    }
                    for (i, (n, _)) in defs.iter().enumerate().rev() {
                        self.taken.insert(n.clone());
                        work.push(Visit::Bind(n.clone(), (node_id(t), i)));
                    }
                }
                Term::Match {
                    discriminant,
                    motive,
                    cases,
                } => {
                    for c in cases.iter().rev() {
                        work.push(Visit::Enter(c));
                    }
                    work.push(Visit::Enter(motive));
                    work.push(Visit::Enter(discriminant));
                }
                Term::App(f, a) => {
                    // A LABEL is not a reference: a named argument's parameter name, a projection's
                    // field and an auto-param's tactic are spelled as names but denote no global.
                    if let Some(label_free) = label_positions(t) {
                        for part in label_free {
                            work.push(Visit::Enter(part));
                        }
                        continue;
                    }
                    work.push(Visit::Enter(a));
                    work.push(Visit::Enter(f));
                }
                Term::Annotated { term, .. } => work.push(Visit::Enter(term)),
            }
        }
    }

    fn note_global(&mut self, key: &str) {
        self.taken.insert(key.to_string());
        self.avoid.insert(key.to_string());
        if let Some(name) = Name::parse(key) {
            for part in name.parts() {
                self.taken.insert(part.clone());
            }
            self.avoid.insert(name.parts()[0].clone());
        }
    }

    /// The spelling a binder named `name` will have: its own name when that reads back as this
    /// binder, a fresh one otherwise.
    fn choose(&mut self, name: &str, referenced: bool) -> String {
        if name == "_" && !referenced {
            return "_".to_string();
        }
        // A binder keeps its name when the name reads back BARE. One that would need guillemets
        // (`arena.local.3`, the importer's own binder names) is renamed instead: renaming a binder
        // changes nothing the kernel can see, and a quoted binder name carries nothing a reader
        // can use.
        if !self.avoid.contains(name) && plain_atom(name) {
            return name.to_string();
        }
        loop {
            let candidate = if self.fresh == 0 {
                "x".to_string()
            } else {
                format!("x{}", self.fresh)
            };
            self.fresh += 1;
            if !self.taken.contains(&candidate) && !self.avoid.contains(&candidate) {
                self.taken.insert(candidate.clone());
                return candidate;
            }
        }
    }

    fn bind(&mut self, pairs: Vec<(String, String)>) {
        for (name, spelled) in pairs {
            self.scope.entry(name.clone()).or_default().push(spelled);
            self.scope_order.push(name);
        }
    }

    fn unbind(&mut self, n: usize) {
        for _ in 0..n {
            if let Some(name) = self.scope_order.pop() {
                if let Some(stack) = self.scope.get_mut(&name) {
                    stack.pop();
                }
            }
        }
    }

    /// The first `count` binders of the `Π` telescope `t` as ` (x : A)` groups — every one NAMED,
    /// referred to or not, because a restated parameter is a binder the reader counts — and the
    /// rest of `t`, printed with those binders in scope.
    fn telescope(
        &mut self,
        t: &Term,
        count: usize,
        owner: &str,
    ) -> Result<(String, String), PrintError> {
        let mut binders = String::new();
        let mut cur = t;
        let mut bound = 0;
        for _ in 0..count {
            let Term::Pi {
                param,
                param_type,
                body_type,
            } = cur
            else {
                return Err(no_spelling(format!(
                    "`{owner}` restating {count} parameters with fewer binders"
                )));
            };
            if !matches!(split_kind(param_type).0, Kind::Explicit)
                || split_default(param_type)?.1.is_some()
            {
                return Err(no_spelling(format!(
                    "a marked parameter binder in `{owner}`"
                )));
            }
            let spelled = self.choose(param, true);
            let start = self.out.len();
            self.term(param_type, Ctx::Top)?;
            let ty = self.out.split_off(start);
            binders.push_str(&format!(" ({spelled} : {ty})"));
            self.bind(vec![(param.clone(), spelled)]);
            bound += 1;
            cur = body_type;
        }
        let start = self.out.len();
        self.term(cur, Ctx::Top)?;
        let rest = self.out.split_off(start);
        self.unbind(bound);
        Ok((binders, rest))
    }

    fn term(&mut self, root: &Term, ctx: Ctx) -> Result<(), PrintError> {
        let mut work: Vec<Job<'_>> = vec![Job::Term(root, ctx)];
        while let Some(job) = work.pop() {
            match job {
                Job::Text(s) => self.out.push_str(s),
                Job::Owned(s) => self.out.push_str(&s),
                Job::Bind(pairs) => self.bind(pairs),
                Job::Unbind(n) => self.unbind(n),
                Job::Term(t, ctx) => {
                    let shape = self.shape(t);
                    if needs_parens(shape, ctx) {
                        work.push(Job::Text(")"));
                        work.push(Job::Term(t, Ctx::Top));
                        work.push(Job::Text("("));
                        continue;
                    }
                    let mut jobs = self.expand(t, ctx)?;
                    jobs.reverse();
                    work.extend(jobs);
                }
            }
        }
        Ok(())
    }

    /// How `t` will print.
    fn shape(&self, t: &Term) -> Shape {
        match t {
            Term::Var(_) | Term::Global(_) | Term::Const { .. } | Term::Hole => Shape::Atom,
            Term::Sort(Universe::Prop | Universe::SProp | Universe::Type(0)) => Shape::Atom,
            Term::Sort(_) => Shape::App,
            Term::Lit(l) if negative_literal(l) => Shape::App,
            Term::Lit(_) => Shape::Atom,
            Term::App(..) => match spine_form(t) {
                SpineForm::Named | SpineForm::Anon | SpineForm::Projection => Shape::Atom,
                _ => Shape::App,
            },
            Term::Match { .. } => Shape::Closed,
            Term::Fix { .. } | Term::MutualFix { .. } => Shape::Fix,
            Term::Pi { .. } | Term::Lambda { .. } | Term::Let { .. } => Shape::Open,
            // `@[…] t` is a prefix on an atom: bracketed as an argument, a head or a projection's
            // receiver, where a following `.field` or argument would otherwise be read as its own.
            Term::Annotated { .. } => Shape::App,
        }
    }

    /// The jobs that print `t` in `ctx`, in order.
    fn expand<'t>(&mut self, t: &'t Term, ctx: Ctx) -> Result<Vec<Job<'t>>, PrintError> {
        // The tail of an open construct inherits where the construct stands.
        let tail = if ctx == Ctx::BeforeWith {
            Ctx::BeforeWith
        } else {
            Ctx::Top
        };
        Ok(match t {
            Term::Var(n) => match self.scope.get(n).and_then(|s| s.last()) {
                Some(spelled) => vec![Job::Owned(spelled.clone())],
                None => {
                    return Err(no_spelling(format!(
                        "the free local variable `{n}` \u{2014} the surface reads a free name as a \
                         GLOBAL, so printing it would denote something else"
                    )))
                }
            },
            Term::Global(n) => vec![Job::Owned(global_name(n)?)],
            Term::Const { name, levels } => {
                let levels = levels
                    .iter()
                    .map(|l| level(l, false))
                    .collect::<Result<Vec<_>, _>>()?;
                vec![Job::Owned(format!(
                    "{}.{{{}}}",
                    global_name(name)?,
                    levels.join(", ")
                ))]
            }
            Term::Sort(u) => vec![Job::Owned(sort(u)?)],
            Term::Lit(l) => vec![Job::Owned(literal(l)?)],
            Term::Hole => vec![Job::Text("_")],
            Term::App(..) => self.application(t)?,
            Term::Pi {
                param,
                param_type,
                body_type,
            } => self.pi(t, param, param_type, body_type, tail)?,
            Term::Lambda { .. } => self.lambda(t, tail)?,
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let spelled = self.choose(name, self.referenced.contains(&(node_id(t), 0)));
                let wrap = |x: &Term| {
                    if matches!(x, Term::Let { .. }) {
                        Ctx::Operand
                    } else {
                        Ctx::Top
                    }
                };
                vec![
                    Job::Text("let "),
                    Job::Owned(spelled.clone()),
                    Job::Text(" : "),
                    Job::Term(ty, wrap(ty)),
                    Job::Text(" := "),
                    Job::Term(value, wrap(value)),
                    Job::Text("; "),
                    Job::Bind(vec![(name.clone(), spelled)]),
                    Job::Term(body, tail),
                    Job::Unbind(1),
                ]
            }
            Term::Fix { name, body } => {
                let spelled = self.choose(name, self.referenced.contains(&(node_id(t), 0)));
                vec![
                    Job::Text("fix "),
                    Job::Owned(spelled.clone()),
                    Job::Text(" => "),
                    Job::Bind(vec![(name.clone(), spelled)]),
                    Job::Term(body, tail),
                    Job::Unbind(1),
                ]
            }
            Term::MutualFix { defs, index } => {
                let mut seen = HashSet::new();
                for (n, _) in defs {
                    if !seen.insert(n.as_str()) {
                        return Err(no_spelling(format!(
                            "a mutual fix naming `{n}` twice \u{2014} `return {n}` could not say which \
                             member the term denotes"
                        )));
                    }
                }
                if *index >= defs.len() {
                    return Err(no_spelling(format!(
                        "a mutual fix of {} member(s) selecting member {index}",
                        defs.len()
                    )));
                }
                let spelled: Vec<String> = defs
                    .iter()
                    .enumerate()
                    // A member's name is also how `return` selects it, so it is always spelled, even
                    // when no body refers to it.
                    .map(|(_, (n, _))| self.choose(n, true))
                    .collect();
                let mut jobs = vec![
                    Job::Text("fix "),
                    Job::Bind(
                        defs.iter()
                            .map(|(n, _)| n.clone())
                            .zip(spelled.iter().cloned())
                            .collect(),
                    ),
                ];
                for (i, (_, body)) in defs.iter().enumerate() {
                    if i > 0 {
                        jobs.push(Job::Text(" with "));
                    }
                    jobs.push(Job::Owned(spelled[i].clone()));
                    jobs.push(Job::Text(" => "));
                    jobs.push(Job::Term(body, Ctx::BeforeWith));
                }
                jobs.push(Job::Unbind(defs.len()));
                jobs.push(Job::Text(" return "));
                jobs.push(Job::Owned(spelled[*index].clone()));
                jobs
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => self.matching(discriminant, motive, cases)?,
            Term::Annotated { annotation, term } => vec![
                Job::Text("@["),
                Job::Owned(annotation_text(annotation)),
                Job::Text("] "),
                Job::Term(term, Ctx::Atom),
            ],
        })
    }

    /// `Π(x : A). B` — as `A -> B` when nothing refers to `x`, else as a binder, whose bracket says
    /// how the argument is supplied.
    fn pi<'t>(
        &mut self,
        node: &'t Term,
        param: &str,
        param_type: &'t Term,
        body: &'t Term,
        tail: Ctx,
    ) -> Result<Vec<Job<'t>>, PrintError> {
        let referenced = self.referenced.contains(&(node_id(node), 0));
        let (kind, declared) = split_kind(param_type);
        let (declared, default) = split_default(declared)?;
        if kind == Kind::Explicit && default.is_none() && !referenced {
            return Ok(vec![
                Job::Term(declared, Ctx::Operand),
                Job::Text(" -> "),
                Job::Term(body, tail),
            ]);
        }
        let spelled = self.choose(param, referenced);
        let (open, close) = kind.brackets();
        let mut jobs = vec![Job::Text(open)];
        let anonymous_instance = kind == Kind::Instance && spelled == "_" && default.is_none();
        if !anonymous_instance {
            jobs.push(Job::Owned(spelled.clone()));
            jobs.push(Job::Text(" : "));
        }
        jobs.push(Job::Term(declared, Ctx::Top));
        match default {
            Some(Default::Value(v)) if kind == Kind::Explicit => {
                jobs.push(Job::Text(" := "));
                jobs.push(Job::Term(v, Ctx::Top));
            }
            Some(Default::Tactic(tac)) if kind == Kind::Explicit => {
                jobs.push(Job::Owned(format!(
                    " := by {}",
                    token(tac, "an auto-param tactic")?
                )));
            }
            Some(_) => {
                return Err(no_spelling(format!(
                "a default on the {} binder `{param}` \u{2014} only an explicit binder carries one",
                kind.describe()
            )))
            }
            None => {}
        }
        jobs.push(Job::Text(close));
        jobs.push(Job::Text(" -> "));
        jobs.push(Job::Bind(vec![(param.to_string(), spelled)]));
        jobs.push(Job::Term(body, tail));
        jobs.push(Job::Unbind(1));
        Ok(jobs)
    }

    /// `fun (x : A) {y} … => body` — consecutive lambdas share one `fun`.
    fn lambda<'t>(&mut self, t: &'t Term, tail: Ctx) -> Result<Vec<Job<'t>>, PrintError> {
        let mut jobs = vec![Job::Text("fun")];
        let mut cur = t;
        let mut bound = 0;
        while let Term::Lambda {
            param,
            param_type,
            body,
        } = cur
        {
            let referenced = self.referenced.contains(&(node_id(cur), 0));
            let (kind, declared) = split_kind(param_type);
            if TermParser::as_default_param(param_type).is_some()
                || TermParser::as_auto_param(param_type).is_some()
            {
                return Err(no_spelling(format!(
                    "a default on the `fun` binder `{param}` \u{2014} a `fun` binder carries none"
                )));
            }
            let spelled = self.choose(param, referenced);
            let (open, close) = kind.brackets();
            jobs.push(Job::Text(" "));
            match (kind, declared) {
                (Kind::Explicit, Term::Hole) => jobs.push(Job::Owned(spelled.clone())),
                (Kind::Instance, _) if spelled == "_" => {
                    jobs.push(Job::Text(open));
                    jobs.push(Job::Term(declared, Ctx::Top));
                    jobs.push(Job::Text(close));
                }
                (_, Term::Hole) => {
                    jobs.push(Job::Text(open));
                    jobs.push(Job::Owned(spelled.clone()));
                    jobs.push(Job::Text(close));
                }
                _ => {
                    jobs.push(Job::Text(open));
                    jobs.push(Job::Owned(spelled.clone()));
                    jobs.push(Job::Text(" : "));
                    jobs.push(Job::Term(declared, Ctx::Top));
                    jobs.push(Job::Text(close));
                }
            }
            jobs.push(Job::Bind(vec![(param.clone(), spelled)]));
            bound += 1;
            cur = body;
        }
        jobs.push(Job::Text(" => "));
        jobs.push(Job::Term(cur, tail));
        jobs.push(Job::Unbind(bound));
        Ok(jobs)
    }

    /// An application spine, and the parser's argument-position forms that ride on one.
    fn application<'t>(&mut self, t: &'t Term) -> Result<Vec<Job<'t>>, PrintError> {
        let (head, args) = spine(t);
        // `f ..` must END its spine, and `..` followed by a space would end the SENTENCE, so an
        // application closed by an ellipsis is always bracketed: `(f ..)`.
        let ellipsis = args
            .last()
            .is_some_and(|a| is_marker(a, TermParser::ELLIPSIS_MARKER))
            && !matches!(spine_form(t), SpineForm::Anon);
        let mut jobs = if ellipsis {
            vec![Job::Text("(")]
        } else {
            Vec::new()
        };
        let mut rest: &[&'t Term] = &args;
        match spine_form(t) {
            SpineForm::Named => {
                let (Term::Global(label), value) = (args[0], args[1]) else {
                    unreachable!()
                };
                jobs.push(Job::Owned(format!(
                    "({} := ",
                    token(label, "a named argument's parameter")?
                )));
                jobs.push(Job::Term(value, Ctx::Top));
                jobs.push(Job::Text(")"));
                rest = &args[2..];
            }
            SpineForm::Anon => {
                jobs.push(Job::Text("\u{27e8}"));
                for (i, c) in args.iter().enumerate() {
                    if i > 0 {
                        jobs.push(Job::Text(", "));
                    }
                    if is_marker(c, TermParser::ELLIPSIS_MARKER) {
                        if i + 1 != args.len() {
                            return Err(no_spelling(
                                "an ellipsis before the last component of `⟨…⟩`",
                            ));
                        }
                        jobs.push(Job::Text(".."));
                    } else {
                        jobs.push(Job::Term(c, Ctx::Top));
                    }
                }
                jobs.push(Job::Text("\u{27e9}"));
                rest = &[];
            }
            SpineForm::Projection => {
                let Term::Global(field) = args[1] else {
                    unreachable!()
                };
                let field = if !field.is_empty() && field.chars().all(|c| c.is_ascii_digit()) {
                    if matches!(args[0], Term::Lit(_)) {
                        return Err(no_spelling(format!(
                            "the numeric projection `.{field}` of a literal \u{2014} `3.1` is a decimal"
                        )));
                    }
                    field.clone()
                } else {
                    tail_component(field)?
                };
                jobs.push(Job::Term(args[0], Ctx::Atom));
                jobs.push(Job::Owned(format!(".{field}")));
                rest = &args[2..];
            }
            SpineForm::Explicit => {
                let named = args[0];
                if !starts_with_name(named) {
                    return Err(no_spelling(
                        "`@` before something that is not a name \u{2014} `@` binds to a constant's name",
                    ));
                }
                jobs.push(Job::Text("@"));
                jobs.push(Job::Term(named, Ctx::Atom));
                rest = &args[1..];
            }
            SpineForm::Plain => {
                push_spine_atom(&mut jobs, head, rest.first().copied());
            }
        }
        for (i, arg) in rest.iter().enumerate() {
            if is_marker(arg, TermParser::ELLIPSIS_MARKER) {
                if i + 1 != rest.len() {
                    return Err(no_spelling(
                        "an ellipsis before an application's last argument",
                    ));
                }
                jobs.push(Job::Text(" .."));
            } else {
                jobs.push(Job::Text(" "));
                push_spine_atom(&mut jobs, arg, rest.get(i + 1).copied());
            }
        }
        if ellipsis {
            jobs.push(Job::Text(")"));
        }
        Ok(jobs)
    }

    /// A kernel match — `match d return M with c₀ | c₁ end` — or the parser's pattern form, whose
    /// clauses the context compiles later.
    fn matching<'t>(
        &mut self,
        discriminant: &'t Term,
        motive: &'t Term,
        cases: &'t [Term],
    ) -> Result<Vec<Job<'t>>, PrintError> {
        let raw: Vec<Option<(&Term, &Term)>> =
            cases.iter().map(super::raw_match::as_case).collect();
        let patterned = !cases.is_empty() && raw.iter().all(Option::is_some);
        if !patterned && raw.iter().any(Option::is_some) {
            return Err(no_spelling(
                "a match mixing pattern clauses with positional cases",
            ));
        }
        let mut jobs = vec![Job::Text("match "), Job::Term(discriminant, Ctx::Operand)];
        if !(patterned && matches!(motive, Term::Hole)) {
            jobs.push(Job::Text(" return "));
            jobs.push(Job::Term(motive, Ctx::Operand));
        }
        jobs.push(Job::Text(" with"));
        if patterned {
            for clause in raw.into_iter().flatten() {
                let (pattern, wrapped) = clause;
                let mut binders = Vec::new();
                super::raw_match::candidates(pattern, &mut binders);
                let mut body = wrapped;
                for name in &binders {
                    match body {
                        Term::Lambda { param, param_type, body: inner }
                            if param == name && matches!(param_type.as_ref(), Term::Hole) =>
                        {
                            body = inner;
                        }
                        _ => {
                            return Err(no_spelling(format!(
                                "a pattern clause whose body does not bind its pattern variable `{name}`"
                            )))
                        }
                    }
                }
                for name in &binders {
                    if mentions_global(body, name) {
                        return Err(no_spelling(format!(
                            "the pattern variable `{name}` over a body naming the global `{name}` \u{2014} \
                             a pattern variable cannot be renamed, since the same word may name a \
                             constructor"
                        )));
                    }
                }
                jobs.push(Job::Text(" | "));
                jobs.push(Job::Owned(pattern_text(pattern, false)?));
                jobs.push(Job::Text(" => "));
                let spelled = binders
                    .iter()
                    .map(|n| Ok((n.clone(), token(n, "a pattern variable")?)))
                    .collect::<Result<Vec<_>, PrintError>>()?;
                let count = spelled.len();
                jobs.push(Job::Bind(spelled));
                jobs.push(Job::Term(body, Ctx::Top));
                jobs.push(Job::Unbind(count));
            }
        } else {
            for (i, case) in cases.iter().enumerate() {
                jobs.push(Job::Text(if i == 0 { " " } else { " | " }));
                jobs.push(Job::Term(case, Ctx::Top));
            }
        }
        jobs.push(Job::Text(" end"));
        Ok(jobs)
    }
}

/// One atom of an application spine, followed by `next`.
///
/// ★ `Type` READS A LEVEL AFTER IT. `Type 3` is the sort `Type 3`, so the sort `Type` followed
/// by an argument that prints with a leading digit — `f Type 3n` — would read the argument's digits
/// as its level. That one atom is bracketed there; everywhere else `Type` stays bare.
fn push_spine_atom<'t>(jobs: &mut Vec<Job<'t>>, t: &'t Term, next: Option<&Term>) {
    if matches!(t, Term::Sort(Universe::Type(0))) && next.is_some_and(prints_with_digit) {
        jobs.push(Job::Text("(Type)"));
    } else {
        jobs.push(Job::Term(t, Ctx::Atom));
    }
}

/// Does `t`, printed as a spine atom, begin with a decimal digit?
fn prints_with_digit(t: &Term) -> bool {
    matches!(t, Term::Lit(l) if !negative_literal(l) && !matches!(l, Literal::Text(_)))
}

/// A pattern of a raw match clause, as the pattern grammar reads it.
fn pattern_text(p: &Term, atom: bool) -> Result<String, PrintError> {
    let name = |n: &str| -> Result<String, PrintError> {
        match Name::parse(n) {
            Some(parsed) if parsed.parts().len() > 1 => {
                let parts = parsed.parts();
                let mut out = token(&parts[0], "a pattern's constructor")?;
                for part in &parts[1..] {
                    out.push('.');
                    out.push_str(&tail_component(part)?);
                }
                Ok(out)
            }
            _ => token(n, "a pattern name"),
        }
    };
    match p {
        Term::Var(n) | Term::Global(n) => name(n),
        Term::Hole => Ok("_".to_string()),
        Term::Lit(Literal::Nat(n)) if n.to_string().parse::<u64>().is_ok() => Ok(n.to_string()),
        Term::App(..) => {
            let (head, args) = spine(p);
            let Term::Global(ctor) = head else {
                return Err(no_spelling(
                    "a pattern application whose head is not a constructor name",
                ));
            };
            let mut out = name(ctor)?;
            for arg in args {
                out.push(' ');
                out.push_str(&pattern_text(arg, true)?);
            }
            Ok(if atom { format!("({out})") } else { out })
        }
        other => Err(no_spelling(format!("the pattern `{other}`"))),
    }
}

/// The head and arguments of an application spine.
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

/// Which parser form an application spine is.
enum SpineForm {
    Plain,
    /// `(x := v)` — `⟨named⟩ x v`.
    Named,
    /// `⟨a, b⟩` — `⟨anon⟩ a b`.
    Anon,
    /// `b.f` — `⟨proj⟩ b f`, possibly applied further.
    Projection,
    /// `@f a b` — `⟨explicit⟩ f a b`.
    Explicit,
}

fn spine_form(t: &Term) -> SpineForm {
    let (head, args) = spine(t);
    let Term::Global(h) = head else {
        return SpineForm::Plain;
    };
    if h == TermParser::NAMED_ARG_MARKER && args.len() == 2 && matches!(args[0], Term::Global(_)) {
        SpineForm::Named
    } else if h == crate::elaborate::ANON_CTOR_MARKER {
        SpineForm::Anon
    } else if h == crate::elaborate::DOT_MARKER
        && args.len() >= 2
        && matches!(args[1], Term::Global(_))
    {
        SpineForm::Projection
    } else if h == TermParser::EXPLICIT_ALL_MARKER && !args.is_empty() {
        SpineForm::Explicit
    } else {
        SpineForm::Plain
    }
}

/// For an application carrying a LABEL (a named argument's name, a projection's field, an
/// auto-param's tactic), the sub-terms that are ordinary terms — `None` when it carries none.
fn label_positions(t: &Term) -> Option<Vec<&Term>> {
    let Term::App(f, x) = t else { return None };
    let Term::App(head, first) = f.as_ref() else {
        return None;
    };
    let Term::Global(m) = head.as_ref() else {
        return None;
    };
    if m == TermParser::NAMED_ARG_MARKER && matches!(first.as_ref(), Term::Global(_)) {
        Some(vec![x.as_ref()])
    } else if (m == crate::elaborate::DOT_MARKER || m == TermParser::AUTO_PARAM_MARKER)
        && matches!(x.as_ref(), Term::Global(_))
    {
        Some(vec![first.as_ref()])
    } else {
        None
    }
}

fn is_marker(t: &Term, marker: &str) -> bool {
    matches!(t, Term::Global(n) if n == marker)
}

/// Does `t` print starting with a name — what `@` requires right after it?
fn starts_with_name(t: &Term) -> bool {
    match t {
        Term::Var(_) | Term::Global(_) | Term::Const { .. } => true,
        Term::App(..) => {
            matches!(spine_form(t), SpineForm::Projection) && {
                let (_, args) = spine(t);
                starts_with_name(args[0])
            }
        }
        _ => false,
    }
}

/// How a binder's argument is supplied, read off its type's marker.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Explicit,
    Implicit,
    Strict,
    Instance,
}

impl Kind {
    fn brackets(self) -> (&'static str, &'static str) {
        match self {
            Kind::Explicit => ("(", ")"),
            Kind::Implicit => ("{", "}"),
            Kind::Strict => ("\u{2983}", "\u{2984}"),
            Kind::Instance => ("[", "]"),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Kind::Explicit => "explicit",
            Kind::Implicit => "implicit",
            Kind::Strict => "strict-implicit",
            Kind::Instance => "instance",
        }
    }
}

fn split_kind(param_type: &Term) -> (Kind, &Term) {
    if let Term::App(f, declared) = param_type {
        if let Term::Global(m) = f.as_ref() {
            let kind = if m == TermParser::IMPLICIT_BINDER_MARKER {
                Some(Kind::Implicit)
            } else if m == TermParser::STRICT_IMPLICIT_BINDER_MARKER {
                Some(Kind::Strict)
            } else if m == TermParser::INSTANCE_BINDER_MARKER {
                Some(Kind::Instance)
            } else {
                None
            };
            if let Some(kind) = kind {
                return (kind, declared.as_ref());
            }
        }
    }
    (Kind::Explicit, param_type)
}

enum Default<'t> {
    Value(&'t Term),
    Tactic(&'t str),
}

/// A binder type's default value or auto-param tactic, and the declared type inside it.
fn split_default(t: &Term) -> Result<(&Term, Option<Default<'_>>), PrintError> {
    if let Term::App(f, payload) = t {
        if let Term::App(m, declared) = f.as_ref() {
            if let Term::Global(m) = m.as_ref() {
                if m == TermParser::PARAM_DEFAULT_MARKER {
                    return Ok((declared.as_ref(), Some(Default::Value(payload.as_ref()))));
                }
                if m == TermParser::AUTO_PARAM_MARKER {
                    return match payload.as_ref() {
                        Term::Global(tac) => Ok((declared.as_ref(), Some(Default::Tactic(tac)))),
                        other => Err(no_spelling(format!("the auto-param tactic `{other}`"))),
                    };
                }
            }
        }
    }
    Ok((t, None))
}

/// Does `t` mention the global `name`, or a qualified global whose head is `name`?
fn mentions_global(t: &Term, name: &str) -> bool {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(k) | Term::Const { name: k, .. } => {
                if k == name || Name::parse(k).is_some_and(|n| n.parts()[0] == name) {
                    return true;
                }
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
            _ => {}
        }
    }
    false
}

/// Does `t` carry a binder-kind marker anywhere?
fn has_kind_marker(t: &Term) -> bool {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(m)
                if m == TermParser::IMPLICIT_BINDER_MARKER
                    || m == TermParser::STRICT_IMPLICIT_BINDER_MARKER
                    || m == TermParser::INSTANCE_BINDER_MARKER =>
            {
                return true
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
            _ => {}
        }
    }
    false
}
